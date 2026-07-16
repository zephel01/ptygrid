//! Versioned persistence of user-trusted working folders (security Finding S2).
//!
//! A `project`/`launch`-origin `ptygrid.yml` can define `cmd` / `resume` /
//! `worktree.setup` that ptygrid would otherwise autostart on load — running
//! attacker-supplied commands merely by `cd`-ing into a hostile repo, and
//! leaking host env via `${VAR}` expansion. Autostart and `worktree.setup` for
//! `project`/`launch` configs are therefore gated behind an explicit one-time
//! "trust this folder" decision made in the frontend; `~/.ptygrid` (`global`)
//! and the built-in default are the user's own config and are trusted
//! implicitly.
//!
//! Loading a config still succeeds regardless of trust (viewing settings and
//! manual, user-initiated launches are never blocked); only the *automatic*
//! command execution is gated. The loaded [`crate::config::ConfigInfo`] carries
//! an additive `trusted: bool` the frontend uses to decide whether to run the
//! autostart loop.
//!
//! Same on-disk discipline as [`crate::app_settings`]: a versioned JSON blob,
//! unknown future versions are refused rather than silently opened, and writes
//! are atomic (temp file + rename). Stored paths are canonicalized (symlinks and
//! `.`/`..` resolved) before storage and comparison so a symlink can't be used
//! to bypass the gate.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::config::ConfigOrigin;
use crate::pty::home_dir;

pub const TRUST_VERSION: u32 = 1;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

/// On-disk trust store: the set of canonicalized working-folder paths the user
/// has explicitly trusted for autostart / `worktree.setup`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrustStore {
    pub version: u32,
    #[serde(default)]
    pub folders: Vec<String>,
}

/// Return shape of the `trust_working_folder` / `is_working_folder_trusted`
/// commands.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustInfo {
    pub trusted: bool,
}

fn trust_path(app_data: &Path) -> PathBuf {
    app_data.join("trusted-folders.json")
}

/// Expand a leading `~` / `~/` to the home directory. Mirrors
/// `app_settings::expand_tilde` (kept local so the trust store stays decoupled
/// from app-settings internals).
fn expand_tilde(input: &str) -> Result<PathBuf, String> {
    if input == "~" {
        return home_dir()
            .map(PathBuf::from)
            .ok_or_else(|| "cannot determine home directory".to_string());
    }
    if let Some(rest) = input.strip_prefix("~/") {
        let home = home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
        return Ok(Path::new(&home).join(rest));
    }
    Ok(PathBuf::from(input))
}

/// Canonicalize an existing path for trust storage/compare (resolve symlinks and
/// `.`/`..`). Falls back to the path as-is when canonicalize fails (e.g. the
/// folder was removed) so a stored entry stays comparable.
fn normalize_existing(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Normalize a user/frontend-supplied working-folder string: expand a leading
/// `~`, then canonicalize.
fn normalize_input(input: &str) -> Result<PathBuf, String> {
    Ok(normalize_existing(&expand_tilde(input)?))
}

fn normalized_folders(store: &TrustStore) -> Vec<PathBuf> {
    store.folders.iter().map(PathBuf::from).collect()
}

/// Origin + membership rule. `Global` and `Default` are always trusted; a
/// `Project`/`Launch` config is trusted only when its (already-normalized)
/// folder is in the stored set. Pure so it is unit-testable.
fn is_trusted_pure(origin: ConfigOrigin, dir: &Path, folders: &[PathBuf]) -> bool {
    match origin {
        ConfigOrigin::Global | ConfigOrigin::Default => true,
        ConfigOrigin::Project | ConfigOrigin::Launch => folders.iter().any(|f| f == dir),
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "trust store path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create trust store directory: {e}"))?;
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".trust-{}-{suffix}.tmp", std::process::id()));
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| format!("cannot encode trust store: {e}"))?;
    std::fs::write(&temp, bytes).map_err(|e| format!("cannot write trust store: {e}"))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace trust store: {e}")
    })
}

fn read_store(app_data: &Path) -> Result<TrustStore, String> {
    let path = trust_path(app_data);
    if !path.is_file() {
        return Ok(TrustStore {
            version: TRUST_VERSION,
            folders: Vec::new(),
        });
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read trust store: {e}"))?;
    let store: TrustStore =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid trust store: {e}"))?;
    if store.version != TRUST_VERSION {
        return Err(format!(
            "unsupported trust store version {} (expected {TRUST_VERSION})",
            store.version
        ));
    }
    Ok(store)
}

fn app_data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("cannot determine app data directory: {e}"))
}

// ---- app-data-relative core (testable without a Tauri AppHandle) ----

fn is_trusted_at(app_data: &Path, origin: ConfigOrigin, dir: &Path) -> bool {
    // Global / Default never touch the store: they are trusted unconditionally.
    if matches!(origin, ConfigOrigin::Global | ConfigOrigin::Default) {
        return true;
    }
    // Fail closed: an unreadable/corrupt trust store means "not trusted" for a
    // project/launch config, so autostart stays gated.
    let Ok(store) = read_store(app_data) else {
        return false;
    };
    is_trusted_pure(origin, &normalize_existing(dir), &normalized_folders(&store))
}

fn add_trusted_at(app_data: &Path, dir: &str) -> Result<TrustInfo, String> {
    let normalized = normalize_input(dir)?;
    let mut store = read_store(app_data)?;
    if !normalized_folders(&store).iter().any(|f| f == &normalized) {
        store.version = TRUST_VERSION;
        store.folders.push(normalized.display().to_string());
        write_json_atomic(&trust_path(app_data), &store)?;
    }
    Ok(TrustInfo { trusted: true })
}

fn check_trusted_at(app_data: &Path, dir: &str) -> Result<TrustInfo, String> {
    let normalized = normalize_input(dir)?;
    let store = read_store(app_data)?;
    let trusted = normalized_folders(&store).iter().any(|f| f == &normalized);
    Ok(TrustInfo { trusted })
}

// ---- public AppHandle-based API ----

/// Whether the loaded config (given its `origin` and working folder `dir`) is
/// trusted for autostart / `worktree.setup`. Used by [`crate::config`] to set
/// [`crate::config::ConfigInfo::trusted`].
pub fn is_trusted<R: Runtime>(app: &AppHandle<R>, origin: ConfigOrigin, dir: &Path) -> bool {
    if matches!(origin, ConfigOrigin::Global | ConfigOrigin::Default) {
        return true;
    }
    let Ok(app_data) = app_data_dir(app) else {
        return false;
    };
    is_trusted_at(&app_data, origin, dir)
}

/// Add `dir` to the trusted set (idempotent) and report it trusted. Backs the
/// `trust_working_folder` command.
pub fn add_trusted<R: Runtime>(app: &AppHandle<R>, dir: &str) -> Result<TrustInfo, String> {
    add_trusted_at(&app_data_dir(app)?, dir)
}

/// Folder-level membership check (ignores origin). Backs the optional
/// `is_working_folder_trusted` command.
pub fn check_trusted<R: Runtime>(app: &AppHandle<R>, dir: &str) -> Result<TrustInfo, String> {
    check_trusted_at(&app_data_dir(app)?, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ptygrid-trust-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn global_and_default_are_always_trusted() {
        // No folders in the set: global/default still trusted, project/launch not.
        let dir = Path::new("/some/repo");
        assert!(is_trusted_pure(ConfigOrigin::Global, dir, &[]));
        assert!(is_trusted_pure(ConfigOrigin::Default, dir, &[]));
        assert!(!is_trusted_pure(ConfigOrigin::Project, dir, &[]));
        assert!(!is_trusted_pure(ConfigOrigin::Launch, dir, &[]));
    }

    #[test]
    fn project_and_launch_require_membership() {
        let dir = PathBuf::from("/some/repo");
        let set = vec![dir.clone()];
        assert!(is_trusted_pure(ConfigOrigin::Project, &dir, &set));
        assert!(is_trusted_pure(ConfigOrigin::Launch, &dir, &set));
        // A different folder is not trusted.
        assert!(!is_trusted_pure(
            ConfigOrigin::Project,
            Path::new("/other/repo"),
            &set
        ));
    }

    #[test]
    fn add_then_is_trusted_round_trip() {
        let root = temp_root("roundtrip");
        let app_data = root.join("app-data");
        let repo = root.join("repo");
        let other = root.join("other");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        // Untrusted before adding: project origin returns false.
        assert!(!is_trusted_at(&app_data, ConfigOrigin::Project, &repo));
        // Untrusted config still loads: the trusted field is just false. Adding
        // flips it to trusted for that folder only.
        let info = add_trusted_at(&app_data, &repo.display().to_string()).unwrap();
        assert!(info.trusted);
        assert!(is_trusted_at(&app_data, ConfigOrigin::Project, &repo));
        assert!(is_trusted_at(&app_data, ConfigOrigin::Launch, &repo));
        // A sibling folder is unaffected.
        assert!(!is_trusted_at(&app_data, ConfigOrigin::Project, &other));
        // Global/default trusted regardless of the store.
        assert!(is_trusted_at(&app_data, ConfigOrigin::Global, &other));
        assert!(is_trusted_at(&app_data, ConfigOrigin::Default, &other));

        // check_trusted mirrors membership (folder-level, origin-agnostic).
        assert!(check_trusted_at(&app_data, &repo.display().to_string())
            .unwrap()
            .trusted);
        assert!(!check_trusted_at(&app_data, &other.display().to_string())
            .unwrap()
            .trusted);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn adding_is_idempotent() {
        let root = temp_root("idem");
        let app_data = root.join("app-data");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        add_trusted_at(&app_data, &repo.display().to_string()).unwrap();
        add_trusted_at(&app_data, &repo.display().to_string()).unwrap();
        let store = read_store(&app_data).unwrap();
        assert_eq!(store.folders.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn trust_matches_via_normalized_path() {
        // A non-canonical spelling (trailing `/.`) of the same existing folder
        // must match after canonicalization, so symlink/`.`-based bypass fails.
        let root = temp_root("normalize");
        let app_data = root.join("app-data");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let noisy = format!("{}/.", repo.display());
        add_trusted_at(&app_data, &noisy).unwrap();
        // Plain path is trusted because both normalize to the same canonical dir.
        assert!(is_trusted_at(&app_data, ConfigOrigin::Project, &repo));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_store_version() {
        let root = temp_root("version");
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::write(
            trust_path(&app_data),
            br#"{ "version": 2, "folders": [] }"#,
        )
        .unwrap();
        // A future version fails read; is_trusted_at fails closed to untrusted.
        assert!(read_store(&app_data).is_err());
        assert!(!is_trusted_at(
            &app_data,
            ConfigOrigin::Project,
            &root.join("repo")
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
