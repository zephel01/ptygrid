// Phase 4.0: teammate hook receiver.
//
// Claude Code (and compatible CLIs) POST lifecycle events to a small set of
// HTTP endpoints served on the same 127.0.0.1 Axum app as the Queen MCP
// server (`/mcp`). This module owns everything about those endpoints so no
// new logic lands in lib.rs or the PTY session hot path:
//
//   * a per-app-run random bearer token (non-persistent),
//   * the `/hooks/v1/*` router (POST + Content-Type: application/json only,
//     Bearer-authenticated, no CORS),
//   * normalization of a received hook into the `teammate-lifecycle` event,
//   * `.claude/settings.json` merge used by `register_teammate_hooks`.
//
// The receiver is intentionally non-blocking: it always answers
// `200 {"decision":"allow"}` once the token checks out. When `teammates.enabled`
// is false (the default) it still validates the token but skips the event emit.

use std::path::Path;

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::config::ConfigManager;
use crate::queen::QueenStatus;

/// Managed Tauri state: the bearer token authorizing hook POSTs. Regenerated
/// on every app launch and never written to disk, so registered settings.json
/// snippets are only valid for the current run.
pub struct TeamsHooks {
    token: String,
}

impl TeamsHooks {
    pub fn new() -> Self {
        TeamsHooks {
            token: generate_token(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Default for TeamsHooks {
    fn default() -> Self {
        Self::new()
    }
}

/// 256-bit random token, lowercase hex. Uses `getrandom` (OS CSPRNG).
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("getrandom: OS entropy unavailable");
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------- lifecycle event kinds ----------

/// The five hook event kinds and their `/hooks/v1/*` path suffixes plus the
/// Claude Code settings.json event name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleKind {
    SubagentStart,
    SubagentStop,
    TeammateIdle,
    TaskCreated,
    TaskCompleted,
}

impl LifecycleKind {
    /// Normalized `kind` string used in the `teammate-lifecycle` event.
    fn as_str(self) -> &'static str {
        match self {
            LifecycleKind::SubagentStart => "subagent-start",
            LifecycleKind::SubagentStop => "subagent-stop",
            LifecycleKind::TeammateIdle => "teammate-idle",
            LifecycleKind::TaskCreated => "task-created",
            LifecycleKind::TaskCompleted => "task-completed",
        }
    }
}

/// (settings.json event name, `/hooks/v1/*` suffix) for each kind.
const HOOK_EVENTS: [(&str, &str); 5] = [
    ("SubagentStart", "subagent-start"),
    ("SubagentStop", "subagent-stop"),
    ("TeammateIdle", "teammate-idle"),
    ("TaskCreated", "task-created"),
    ("TaskCompleted", "task-completed"),
];

// ---------- request payload ----------

/// Raw hook payload (snake_case, matching Claude Code hook JSON). Unknown
/// fields are ignored; per-endpoint required fields are checked separately.
#[derive(Debug, Default, Deserialize)]
struct HookPayload {
    session_id: Option<String>,
    agent_id: Option<String>,
    agent_type: Option<String>,
    #[allow(dead_code)]
    transcript_path: Option<String>,
    cwd: Option<String>,
    task_id: Option<String>,
    task_name: Option<String>,
    status: Option<String>,
    #[allow(dead_code)]
    team_name: Option<String>,
}

/// Normalized `teammate-lifecycle` event payload sent to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LifecyclePayload {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

// ---------- router ----------

/// Router state: the app handle (for config lookups + event emit) and the
/// bearer token to compare against. Generic over the runtime so tests can
/// drive it with `tauri::test::MockRuntime`.
struct HookContext<R: Runtime> {
    app: AppHandle<R>,
    token: String,
}

// Manual Clone so we don't force a `R: Clone` bound (runtimes are not Clone,
// but `AppHandle<R>` is).
impl<R: Runtime> Clone for HookContext<R> {
    fn clone(&self) -> Self {
        HookContext {
            app: self.app.clone(),
            token: self.token.clone(),
        }
    }
}

/// Build the `/hooks/v1/*` router. Merged into the Queen Axum app in
/// `queen::run_server`; also bound standalone by the tests.
pub fn router<R: Runtime>(app: AppHandle<R>, token: String) -> Router {
    let ctx = HookContext { app, token };
    Router::new()
        .route("/hooks/v1/subagent-start", post(subagent_start::<R>))
        .route("/hooks/v1/subagent-stop", post(subagent_stop::<R>))
        .route("/hooks/v1/teammate-idle", post(teammate_idle::<R>))
        .route("/hooks/v1/task-created", post(task_created::<R>))
        .route("/hooks/v1/task-completed", post(task_completed::<R>))
        .with_state(ctx)
}

async fn subagent_start<R: Runtime>(
    State(ctx): State<HookContext<R>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    process(&ctx, LifecycleKind::SubagentStart, &headers, &body)
}

async fn subagent_stop<R: Runtime>(
    State(ctx): State<HookContext<R>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    process(&ctx, LifecycleKind::SubagentStop, &headers, &body)
}

async fn teammate_idle<R: Runtime>(
    State(ctx): State<HookContext<R>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    process(&ctx, LifecycleKind::TeammateIdle, &headers, &body)
}

async fn task_created<R: Runtime>(
    State(ctx): State<HookContext<R>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    process(&ctx, LifecycleKind::TaskCreated, &headers, &body)
}

async fn task_completed<R: Runtime>(
    State(ctx): State<HookContext<R>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    process(&ctx, LifecycleKind::TaskCompleted, &headers, &body)
}

/// Shared handler: auth -> content-type -> parse -> required fields -> emit.
/// Returns 401 (bad/missing token), 400 (bad content-type/JSON/required
/// field, logged), or 200 `{"decision":"allow"}`.
fn process<R: Runtime>(
    ctx: &HookContext<R>,
    kind: LifecycleKind,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    if !authorized(headers, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !is_json_content_type(headers) {
        eprintln!("teammate hook {}: rejecting non-JSON content-type", kind.as_str());
        return StatusCode::BAD_REQUEST.into_response();
    }
    let payload: HookPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("teammate hook {}: invalid JSON body: {e}", kind.as_str());
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    if let Err(field) = validate(kind, &payload) {
        eprintln!(
            "teammate hook {}: missing required field '{field}'",
            kind.as_str()
        );
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Token was valid; only emit when the feature is switched on.
    if teammates_enabled(&ctx.app) {
        let event = LifecyclePayload {
            kind: kind.as_str(),
            session_id: payload.session_id,
            agent_id: payload.agent_id,
            agent_type: payload.agent_type,
            task_id: payload.task_id,
            task_name: payload.task_name,
            status: payload.status,
            cwd: payload.cwd,
        };
        let _ = ctx.app.emit("teammate-lifecycle", event);
    }

    allow_response()
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|got| got == expected)
        .unwrap_or(false)
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        .unwrap_or(false)
}

/// Per-endpoint required fields: session_id always; agent_id for subagent
/// events; task_id for task events. Returns the missing field name on failure.
fn validate(kind: LifecycleKind, p: &HookPayload) -> Result<(), &'static str> {
    if is_blank(&p.session_id) {
        return Err("session_id");
    }
    match kind {
        LifecycleKind::SubagentStart | LifecycleKind::SubagentStop => {
            if is_blank(&p.agent_id) {
                return Err("agent_id");
            }
        }
        LifecycleKind::TaskCreated | LifecycleKind::TaskCompleted => {
            if is_blank(&p.task_id) {
                return Err("task_id");
            }
        }
        LifecycleKind::TeammateIdle => {}
    }
    Ok(())
}

fn is_blank(v: &Option<String>) -> bool {
    v.as_deref().unwrap_or("").is_empty()
}

fn allow_response() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"decision":"allow"}"#,
    )
        .into_response()
}

/// Whether the loaded config has `teammates.enabled: true`. Missing config or
/// missing block => false.
fn teammates_enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<ConfigManager>()
        .and_then(|cm| cm.current())
        .map(|(cfg, _)| cfg.teammates.unwrap_or_default().effective_enabled())
        .unwrap_or(false)
}

// ---------- commands support ----------

/// `teammate_hooks_info` return value.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeammateHooksInfo {
    pub enabled: bool,
    pub hook_notifications: bool,
    /// The Queen server port hooks share (bound port, else configured port).
    pub port: u16,
    /// Non-persistent bearer token for this app run.
    pub token: String,
    /// Default scope ("user" | "project") from `teammates.hooks_scope`.
    pub hooks_scope: &'static str,
}

/// Assemble `teammate_hooks_info` from config + Queen port + token.
pub fn hooks_info<R: Runtime>(app: &AppHandle<R>) -> TeammateHooksInfo {
    let teammates = app
        .try_state::<ConfigManager>()
        .and_then(|cm| cm.current())
        .and_then(|(cfg, _)| cfg.teammates)
        .unwrap_or_default();
    let port = app
        .try_state::<QueenStatus>()
        .map(|s| s.effective_port())
        .unwrap_or(crate::queen::DEFAULT_PORT);
    let token = app
        .try_state::<TeamsHooks>()
        .map(|t| t.token().to_string())
        .unwrap_or_default();
    TeammateHooksInfo {
        enabled: teammates.effective_enabled(),
        hook_notifications: teammates.effective_hook_notifications(),
        port,
        token,
        hooks_scope: teammates.effective_hooks_scope().as_str(),
    }
}

/// `register_teammate_hooks` return value.
#[derive(Debug, Clone, Serialize)]
pub struct RegisterResult {
    pub written: bool,
    pub path: String,
}

/// Resolve the settings.json path for `scope`, then merge the ptygrid hooks.
/// `user` -> `~/.claude/settings.json`; `project` -> `<config dir>/.claude/settings.json`
/// (errors if no project is loaded).
pub fn register<R: Runtime>(app: &AppHandle<R>, scope: &str) -> Result<RegisterResult, String> {
    let base_dir = match scope {
        "user" => app
            .path()
            .home_dir()
            .map_err(|e| format!("cannot determine home dir: {e}"))?,
        "project" => {
            let cm = app
                .try_state::<ConfigManager>()
                .ok_or_else(|| "config manager unavailable".to_string())?;
            let (_, dir) = cm
                .current()
                .ok_or_else(|| "no project loaded; load an mterm.yml first".to_string())?;
            dir
        }
        other => {
            return Err(format!(
                "unknown scope: {other} (expected \"user\" or \"project\")"
            ))
        }
    };
    let port = app
        .try_state::<QueenStatus>()
        .map(|s| s.effective_port())
        .unwrap_or(crate::queen::DEFAULT_PORT);
    let token = app
        .try_state::<TeamsHooks>()
        .map(|t| t.token().to_string())
        .unwrap_or_default();
    let path = base_dir.join(".claude").join("settings.json");
    write_hook_settings(&path, port, &token)
}

/// Merge the ptygrid HTTP hooks into `settings_path`, preserving existing
/// content. Old ptygrid entries (any `http://127.0.0.1:<port>/hooks/v1/` URL)
/// are replaced. Writes nothing when the result is byte-for-byte equivalent
/// to the current content; otherwise backs the file up first.
pub fn write_hook_settings(
    settings_path: &Path,
    port: u16,
    token: &str,
) -> Result<RegisterResult, String> {
    let path_str = settings_path.display().to_string();

    let existed = settings_path.is_file();
    let original: serde_json::Value = if existed {
        let text = std::fs::read_to_string(settings_path)
            .map_err(|e| format!("read failed: {e}"))?;
        if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text).map_err(|e| format!("settings.json parse failed: {e}"))?
        }
    } else {
        serde_json::json!({})
    };

    if !original.is_object() {
        return Err("settings.json root is not a JSON object".to_string());
    }

    let mut merged = original.clone();
    let root = merged
        .as_object_mut()
        .ok_or_else(|| "settings.json root is not a JSON object".to_string())?;

    let hooks_entry = root
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks = hooks_entry
        .as_object_mut()
        .ok_or_else(|| "settings.json `hooks` is not a JSON object".to_string())?;

    let authorization = format!("Bearer {token}");
    for (event_name, suffix) in HOOK_EVENTS {
        let url = format!("http://127.0.0.1:{port}/hooks/v1/{suffix}");
        let group = serde_json::json!({
            "hooks": [
                {
                    "type": "http",
                    "url": url,
                    "headers": { "Authorization": authorization },
                }
            ]
        });

        let arr = hooks
            .entry(event_name)
            .or_insert_with(|| serde_json::json!([]));
        let list = arr
            .as_array_mut()
            .ok_or_else(|| format!("settings.json `hooks.{event_name}` is not an array"))?;
        // Drop any prior ptygrid group, keep unrelated user hooks.
        list.retain(|g| !is_ptygrid_group(g));
        list.push(group);
    }

    // No semantic change -> do not touch the file (also skips the backup).
    if merged == original {
        return Ok(RegisterResult {
            written: false,
            path: path_str,
        });
    }

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }

    // Back up the existing file before overwriting it.
    if existed {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = settings_path.with_file_name(format!(
            "{}.ptygrid-backup-{unix}",
            settings_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("settings.json")
        ));
        std::fs::copy(settings_path, &backup)
            .map_err(|e| format!("backup failed: {e}"))?;
    }

    let mut text = serde_json::to_string_pretty(&merged)
        .map_err(|e| format!("serialize failed: {e}"))?;
    text.push('\n');
    std::fs::write(settings_path, text).map_err(|e| format!("write failed: {e}"))?;

    Ok(RegisterResult {
        written: true,
        path: path_str,
    })
}

/// A hook group is "ptygrid's" if any of its inner hooks targets a
/// `http://127.0.0.1:<port>/hooks/v1/` URL.
fn is_ptygrid_group(group: &serde_json::Value) -> bool {
    group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|inner| {
            inner.iter().any(|h| {
                h.get("url")
                    .and_then(|u| u.as_str())
                    .map(is_ptygrid_url)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn is_ptygrid_url(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:") && url.contains("/hooks/v1/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri::test::{mock_app, MockRuntime};
    use tauri::Listener;

    // ---------- settings.json merge ----------

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn merge_creates_all_five_events_and_is_idempotent() {
        let dir = tempdir();
        let path = dir.join(".claude").join("settings.json");

        let r1 = write_hook_settings(&path, 39237, "tok123").unwrap();
        assert!(r1.written);
        let value = read_json(&path);
        let hooks = value.get("hooks").unwrap().as_object().unwrap();
        for (event, suffix) in HOOK_EVENTS {
            let group = &hooks.get(event).unwrap().as_array().unwrap()[0];
            let hook = &group.get("hooks").unwrap().as_array().unwrap()[0];
            assert_eq!(hook.get("type").unwrap(), "http");
            assert_eq!(
                hook.get("url").unwrap(),
                &format!("http://127.0.0.1:39237/hooks/v1/{suffix}")
            );
            assert_eq!(
                hook.get("headers").unwrap().get("Authorization").unwrap(),
                "Bearer tok123"
            );
        }

        // Same inputs again -> no write, no extra backup file.
        let r2 = write_hook_settings(&path, 39237, "tok123").unwrap();
        assert!(!r2.written);
        let backups = count_backups(&path);
        assert_eq!(backups, 0, "idempotent re-run must not back up");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_preserves_existing_content_and_backs_up_on_change() {
        let dir = tempdir();
        let claude = dir.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let path = claude.join("settings.json");
        // Pre-existing user settings with an unrelated hook.
        std::fs::write(
            &path,
            r#"{
  "model": "sonnet",
  "hooks": {
    "SubagentStart": [
      { "hooks": [ { "type": "command", "command": "echo hi" } ] }
    ]
  }
}"#,
        )
        .unwrap();

        let r = write_hook_settings(&path, 40000, "abc").unwrap();
        assert!(r.written);
        assert_eq!(count_backups(&path), 1, "changed write must back up once");

        let value = read_json(&path);
        // Unrelated top-level key preserved.
        assert_eq!(value.get("model").unwrap(), "sonnet");
        // The user's command hook survives alongside the new ptygrid group.
        let start = value["hooks"]["SubagentStart"].as_array().unwrap();
        assert_eq!(start.len(), 2);
        assert!(start.iter().any(|g| g["hooks"][0]["type"] == "command"));
        assert!(start.iter().any(|g| g["hooks"][0]["type"] == "http"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_replaces_old_ptygrid_entry_on_port_change() {
        let dir = tempdir();
        let path = dir.join(".claude").join("settings.json");

        write_hook_settings(&path, 39237, "old").unwrap();
        // Re-register on a different port: the old ptygrid group is replaced,
        // not duplicated.
        let r = write_hook_settings(&path, 39240, "new").unwrap();
        assert!(r.written);

        let value = read_json(&path);
        let start = value["hooks"]["SubagentStart"].as_array().unwrap();
        assert_eq!(start.len(), 1, "old ptygrid entry must be replaced");
        assert_eq!(
            start[0]["hooks"][0]["url"],
            "http://127.0.0.1:39240/hooks/v1/subagent-start"
        );
        assert_eq!(start[0]["hooks"][0]["headers"]["Authorization"], "Bearer new");

        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ptygrid-teams-test-{}-{}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn count_backups(settings_path: &Path) -> usize {
        let dir = settings_path.parent().unwrap();
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".ptygrid-backup-")
            })
            .count()
    }

    // ---------- HTTP receiver ----------

    /// Bind the router on an ephemeral 127.0.0.1 port and serve it in the
    /// background. Returns the bound address.
    async fn serve(app: AppHandle<MockRuntime>, token: &str) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let router = router(app, token.to_string());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }

    /// Minimal blocking HTTP/1.1 client (no reqwest): returns the numeric
    /// status code. Runs on a blocking thread so it doesn't stall the runtime.
    async fn post(
        addr: std::net::SocketAddr,
        path: &str,
        auth: Option<&str>,
        content_type: Option<&str>,
        body: &str,
    ) -> u16 {
        let path = path.to_string();
        let auth = auth.map(|s| s.to_string());
        let content_type = content_type.map(|s| s.to_string());
        let body = body.to_string();
        tokio::task::spawn_blocking(move || {
            let mut req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
            if let Some(a) = auth {
                req.push_str(&format!("Authorization: {a}\r\n"));
            }
            if let Some(ct) = content_type {
                req.push_str(&format!("Content-Type: {ct}\r\n"));
            }
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
            req.push_str("Connection: close\r\n\r\n");
            req.push_str(&body);

            let mut stream = std::net::TcpStream::connect(addr).unwrap();
            stream.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            stream.read_to_string(&mut resp).unwrap();
            // "HTTP/1.1 200 OK" -> 200
            resp.split_whitespace()
                .nth(1)
                .and_then(|c| c.parse::<u16>().ok())
                .unwrap()
        })
        .await
        .unwrap()
    }

    fn enabled_app() -> AppHandle<MockRuntime> {
        let app = mock_app();
        let handle = app.handle().clone();
        handle.manage(ConfigManager::new());
        // Load a teammates.enabled: true config so emits fire.
        let cfg = crate::config::parse_config("agents: []\nteammates:\n  enabled: true").unwrap();
        set_config(&handle, cfg);
        handle
    }

    fn disabled_app() -> AppHandle<MockRuntime> {
        let app = mock_app();
        let handle = app.handle().clone();
        handle.manage(ConfigManager::new());
        // No teammates block -> disabled by default.
        let cfg = crate::config::parse_config("agents: []").unwrap();
        set_config(&handle, cfg);
        handle
    }

    /// Populate ConfigManager directly (the real `load` needs a Wry handle).
    fn set_config(app: &AppHandle<MockRuntime>, cfg: crate::config::Config) {
        app.state::<ConfigManager>()
            .set_for_test(std::env::temp_dir(), cfg);
    }

    const BODY: &str = r#"{"session_id":"s1","agent_id":"a1","agent_type":"researcher"}"#;

    #[tokio::test]
    async fn valid_request_returns_200_allow_and_emits() {
        let app = enabled_app();
        let (tx, rx) = mpsc::channel::<()>();
        app.listen("teammate-lifecycle", move |_| {
            let _ = tx.send(());
        });
        let addr = serve(app, "secret").await;

        let code = post(
            addr,
            "/hooks/v1/subagent-start",
            Some("Bearer secret"),
            Some("application/json"),
            BODY,
        )
        .await;
        assert_eq!(code, 200);
        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "enabled app must emit teammate-lifecycle"
        );
    }

    #[tokio::test]
    async fn wrong_token_returns_401_without_emit() {
        let app = enabled_app();
        let (tx, rx) = mpsc::channel::<()>();
        app.listen("teammate-lifecycle", move |_| {
            let _ = tx.send(());
        });
        let addr = serve(app, "secret").await;

        let code = post(
            addr,
            "/hooks/v1/subagent-start",
            Some("Bearer wrong"),
            Some("application/json"),
            BODY,
        )
        .await;
        assert_eq!(code, 401);
        // Missing token also 401.
        let code = post(
            addr,
            "/hooks/v1/subagent-start",
            None,
            Some("application/json"),
            BODY,
        )
        .await;
        assert_eq!(code, 401);
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "401 must not emit"
        );
    }

    #[tokio::test]
    async fn missing_required_field_returns_400() {
        let app = enabled_app();
        let addr = serve(app, "secret").await;
        // subagent-start with no agent_id.
        let code = post(
            addr,
            "/hooks/v1/subagent-start",
            Some("Bearer secret"),
            Some("application/json"),
            r#"{"session_id":"s1"}"#,
        )
        .await;
        assert_eq!(code, 400);
        // missing session_id everywhere.
        let code = post(
            addr,
            "/hooks/v1/teammate-idle",
            Some("Bearer secret"),
            Some("application/json"),
            r#"{}"#,
        )
        .await;
        assert_eq!(code, 400);
    }

    #[tokio::test]
    async fn non_json_content_type_returns_400() {
        let app = enabled_app();
        let addr = serve(app, "secret").await;
        let code = post(
            addr,
            "/hooks/v1/subagent-start",
            Some("Bearer secret"),
            Some("text/plain"),
            BODY,
        )
        .await;
        assert_eq!(code, 400);
    }

    #[tokio::test]
    async fn disabled_returns_200_but_no_emit() {
        let app = disabled_app();
        let (tx, rx) = mpsc::channel::<()>();
        app.listen("teammate-lifecycle", move |_| {
            let _ = tx.send(());
        });
        let addr = serve(app, "secret").await;

        let code = post(
            addr,
            "/hooks/v1/subagent-start",
            Some("Bearer secret"),
            Some("application/json"),
            BODY,
        )
        .await;
        assert_eq!(code, 200, "disabled still answers 200 allow");
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "disabled must not emit"
        );
    }
}
