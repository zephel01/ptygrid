//! Persisted authentication tokens (Queen `/mcp` + teammate hooks Bearer).
//!
//! Historically both tokens were minted fresh on every app launch and never
//! written to disk, so a registered `~/.claude/settings.json` hook Bearer and
//! the MCP `QUEEN_URL` token went stale on every restart — the user had to
//! re-register after each launch/rebuild. This module persists both tokens in
//! the Tauri app-data directory so they survive restarts; a leaked token can be
//! rotated on demand via [`regenerate`].
//!
//! On-disk discipline mirrors [`crate::app_settings`] / [`crate::trust`]: a
//! versioned JSON blob (`auth-tokens.json`), unknown future versions are refused
//! rather than silently opened, and writes are atomic (temp file + rename). The
//! file additionally gets `0600` (owner read/write only) on Unix because it
//! holds secrets — the same-user threat model treats app-data as user-private.
//!
//! The generation (`getrandom`, 256-bit lowercase hex) and constant-time
//! comparison stay in [`crate::teams_hooks`]; this module only changes where the
//! token comes from (non-persistent → persistent) and holds the live value in a
//! shared [`TokenHandle`] so a regeneration is visible to the already-bound
//! `/mcp` middleware and hook receiver without rebinding the server.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::queen::QueenStatus;
use crate::teams_hooks::{generate_token, TeamsHooks};

pub const AUTH_TOKENS_VERSION: u32 = 1;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

/// A live, shareable token value. Cloning shares the same `Arc`, so a
/// [`TokenHandle::set`] made through one clone (a regeneration) is immediately
/// observed by every other clone — including the copies captured by the running
/// `/mcp` auth middleware and the `/hooks/v1/*` receiver. This is what lets a
/// rotation take effect without rebinding the Axum server.
#[derive(Clone)]
pub struct TokenHandle(Arc<Mutex<String>>);

impl TokenHandle {
    pub fn new(initial: String) -> Self {
        TokenHandle(Arc::new(Mutex::new(initial)))
    }

    /// Current token value (cloned out of the lock).
    pub fn get(&self) -> String {
        match self.0.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Replace the token in place (rotation). Shared with all clones.
    pub fn set(&self, value: String) {
        match self.0.lock() {
            Ok(mut g) => *g = value,
            Err(poisoned) => *poisoned.into_inner() = value,
        }
    }
}

/// On-disk shape of `auth-tokens.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthTokensFile {
    version: u32,
    hook_token: String,
    queen_token: String,
}

/// The two live tokens returned by [`load_or_create`].
#[derive(Debug, Clone)]
pub struct AuthTokens {
    pub hook_token: String,
    pub queen_token: String,
}

/// Which token(s) a regeneration targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Hook,
    Queen,
    All,
}

impl Which {
    /// Parse the optional `which` command argument. Absent/empty defaults to
    /// `All`; anything else must be exactly `hook`, `queen`, or `all`.
    fn parse(which: Option<&str>) -> Result<Which, String> {
        match which.map(str::trim) {
            None | Some("") | Some("all") => Ok(Which::All),
            Some("hook") => Ok(Which::Hook),
            Some("queen") => Ok(Which::Queen),
            Some(other) => Err(format!(
                "unknown which: {other} (expected \"hook\", \"queen\", or \"all\")"
            )),
        }
    }

    fn touches_hook(self) -> bool {
        matches!(self, Which::Hook | Which::All)
    }

    fn touches_queen(self) -> bool {
        matches!(self, Which::Queen | Which::All)
    }
}

/// Return shape of the `regenerate_auth_tokens` command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateResult {
    /// Which tokens were regenerated: any of `"hook"` / `"queen"`.
    pub regenerated: Vec<String>,
}

fn tokens_path(app_data: &Path) -> PathBuf {
    app_data.join("auth-tokens.json")
}

/// Read the store. `Ok(None)` when the file does not exist yet; `Err` on parse
/// failure or an unknown (future) version — never silently opened.
fn read_store(app_data: &Path) -> Result<Option<AuthTokensFile>, String> {
    let path = tokens_path(app_data);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read auth tokens: {e}"))?;
    let store: AuthTokensFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid auth tokens: {e}"))?;
    if store.version != AUTH_TOKENS_VERSION {
        return Err(format!(
            "unsupported auth tokens version {} (expected {AUTH_TOKENS_VERSION})",
            store.version
        ));
    }
    Ok(Some(store))
}

/// Atomic write (temp file + rename) with `0600` perms on Unix. Permissions are
/// applied to the temp file before the rename so the visible file is never
/// world-readable, even momentarily.
fn write_store(app_data: &Path, value: &AuthTokensFile) -> Result<(), String> {
    std::fs::create_dir_all(app_data)
        .map_err(|e| format!("cannot create auth tokens directory: {e}"))?;
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp = app_data.join(format!(".auth-tokens-{}-{suffix}.tmp", std::process::id()));
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| format!("cannot encode auth tokens: {e}"))?;
    std::fs::write(&temp, bytes).map_err(|e| format!("cannot write auth tokens: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("cannot set auth tokens permissions: {e}"));
        }
    }
    std::fs::rename(&temp, tokens_path(app_data)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace auth tokens: {e}")
    })
}

// ---- app-data-relative core (testable without a Tauri AppHandle) ----

/// Load both tokens, generating and persisting a fresh pair the first time (or
/// after the file was removed). Idempotent across restarts: the same tokens are
/// returned until [`regenerate_at`] rotates them.
fn load_or_create_at(app_data: &Path) -> Result<AuthTokens, String> {
    if let Some(store) = read_store(app_data)? {
        return Ok(AuthTokens {
            hook_token: store.hook_token,
            queen_token: store.queen_token,
        });
    }
    let store = AuthTokensFile {
        version: AUTH_TOKENS_VERSION,
        hook_token: generate_token(),
        queen_token: generate_token(),
    };
    write_store(app_data, &store)?;
    Ok(AuthTokens {
        hook_token: store.hook_token,
        queen_token: store.queen_token,
    })
}

/// Rotate the requested token(s), persist, and return the full token set plus
/// the labels of what changed. A missing/corrupt store is treated as "generate
/// everything" so a rotation always yields a valid, complete file.
fn regenerate_at(app_data: &Path, which: Which) -> Result<(AuthTokens, Vec<String>), String> {
    let existing = read_store(app_data)?;
    let mut hook_token = existing
        .as_ref()
        .map(|s| s.hook_token.clone())
        .unwrap_or_else(generate_token);
    let mut queen_token = existing
        .as_ref()
        .map(|s| s.queen_token.clone())
        .unwrap_or_else(generate_token);

    let mut regenerated = Vec::new();
    if which.touches_hook() {
        hook_token = generate_token();
        regenerated.push("hook".to_string());
    }
    if which.touches_queen() {
        queen_token = generate_token();
        regenerated.push("queen".to_string());
    }

    let store = AuthTokensFile {
        version: AUTH_TOKENS_VERSION,
        hook_token,
        queen_token,
    };
    write_store(app_data, &store)?;
    Ok((
        AuthTokens {
            hook_token: store.hook_token,
            queen_token: store.queen_token,
        },
        regenerated,
    ))
}

// ---- public AppHandle-based API ----

/// Load or create the persisted token pair for `app_data`. Called once at
/// startup (before the Queen server binds) so both tokens are fixed for the run.
pub fn load_or_create(app_data: &Path) -> Result<AuthTokens, String> {
    load_or_create_at(app_data)
}

/// Rotate the token(s) selected by `which` (default all), persist the new
/// value(s), and push them into the live [`TokenHandle`]s held by
/// [`QueenStatus`] / [`TeamsHooks`] so the running `/mcp` middleware and hook
/// receiver validate against the new tokens without a rebind. Backs the
/// `regenerate_auth_tokens` command.
pub fn regenerate<R: Runtime>(
    app: &AppHandle<R>,
    which: Option<&str>,
) -> Result<RegenerateResult, String> {
    let which = Which::parse(which)?;
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot determine app data directory: {e}"))?;
    let (tokens, regenerated) = regenerate_at(&app_data, which)?;

    if which.touches_hook() {
        if let Some(hooks) = app.try_state::<TeamsHooks>() {
            hooks.set_token(tokens.hook_token.clone());
        }
    }
    if which.touches_queen() {
        if let Some(status) = app.try_state::<QueenStatus>() {
            status.set_token(tokens.queen_token.clone());
        }
    }
    Ok(RegenerateResult { regenerated })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ptygrid-tokens-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn is_hex64(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
    }

    #[test]
    fn generates_and_persists_on_first_load() {
        let app_data = temp_root("first");
        let first = load_or_create_at(&app_data).unwrap();
        assert!(is_hex64(&first.hook_token));
        assert!(is_hex64(&first.queen_token));
        assert_ne!(first.hook_token, first.queen_token);
        // A file was written and is reloadable.
        assert!(tokens_path(&app_data).is_file());
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn load_round_trip_is_stable() {
        let app_data = temp_root("roundtrip");
        let first = load_or_create_at(&app_data).unwrap();
        // A second load returns the identical persisted tokens (survives restart).
        let second = load_or_create_at(&app_data).unwrap();
        assert_eq!(first.hook_token, second.hook_token);
        assert_eq!(first.queen_token, second.queen_token);
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn rejects_unknown_version() {
        let app_data = temp_root("version");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(
            tokens_path(&app_data),
            br#"{ "version": 2, "hookToken": "aa", "queenToken": "bb" }"#,
        )
        .unwrap();
        assert!(read_store(&app_data).is_err());
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn regenerate_all_changes_both() {
        let app_data = temp_root("regen-all");
        let first = load_or_create_at(&app_data).unwrap();
        let (after, changed) = regenerate_at(&app_data, Which::All).unwrap();
        assert_eq!(changed, vec!["hook", "queen"]);
        assert_ne!(after.hook_token, first.hook_token);
        assert_ne!(after.queen_token, first.queen_token);
        // Persisted: a reload returns the rotated values.
        let reloaded = load_or_create_at(&app_data).unwrap();
        assert_eq!(reloaded.hook_token, after.hook_token);
        assert_eq!(reloaded.queen_token, after.queen_token);
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn regenerate_hook_leaves_queen() {
        let app_data = temp_root("regen-hook");
        let first = load_or_create_at(&app_data).unwrap();
        let (after, changed) = regenerate_at(&app_data, Which::Hook).unwrap();
        assert_eq!(changed, vec!["hook"]);
        assert_ne!(after.hook_token, first.hook_token);
        assert_eq!(after.queen_token, first.queen_token, "queen untouched");
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn regenerate_queen_leaves_hook() {
        let app_data = temp_root("regen-queen");
        let first = load_or_create_at(&app_data).unwrap();
        let (after, changed) = regenerate_at(&app_data, Which::Queen).unwrap();
        assert_eq!(changed, vec!["queen"]);
        assert_eq!(after.hook_token, first.hook_token, "hook untouched");
        assert_ne!(after.queen_token, first.queen_token);
        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn which_parse_accepts_known_and_rejects_unknown() {
        assert_eq!(Which::parse(None).unwrap(), Which::All);
        assert_eq!(Which::parse(Some("")).unwrap(), Which::All);
        assert_eq!(Which::parse(Some("all")).unwrap(), Which::All);
        assert_eq!(Which::parse(Some("hook")).unwrap(), Which::Hook);
        assert_eq!(Which::parse(Some("queen")).unwrap(), Which::Queen);
        assert!(Which::parse(Some("bogus")).is_err());
    }

    #[test]
    fn token_handle_shares_updates_across_clones() {
        let a = TokenHandle::new("one".to_string());
        let b = a.clone();
        assert_eq!(b.get(), "one");
        a.set("two".to_string());
        // The clone observes the rotation (shared Arc).
        assert_eq!(b.get(), "two");
    }

    #[cfg(unix)]
    #[test]
    fn persisted_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let app_data = temp_root("perms");
        load_or_create_at(&app_data).unwrap();
        let mode = std::fs::metadata(tokens_path(&app_data))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&app_data);
    }
}
