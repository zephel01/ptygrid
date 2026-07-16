// Phase 4.2: host mode — hosting Claude Code split-pane teammates as real
// ptygrid PTY panes (方式A / cmux-style tmux shim).
//
// This module owns everything host-mode so no host logic lands in lib.rs or
// the PTY session hot path:
//
//   * pure helpers: context-id <-> session-id, the teammate binary allowlist,
//     the host pane-limit decision, and the fallback time-window correlation;
//   * the ptygrid-side `PaneHost` (`PtygridPaneHost`) that the pane-backend
//     socket server drives against the real `PtyManager`;
//   * `setup_lead`, which — only for a `teams.mode: host` lead — writes the
//     tmux shim, binds a per-lead auth-token'd Unix socket, spawns the socket
//     server + an exit/lifecycle monitor, and returns the env vars injected
//     into the lead PTY;
//   * `watch_for_fallback`, the 2s correlation that downgrades a lead to the
//     Phase 4.1 observe (read-only transcript) path when the shim is never
//     driven (the #6447-style breakage);
//   * `run_tmux_shim`, the in-process `__tmux-compat` entry point the app
//     re-executes itself as (see main.rs), so no separate shim binary ships.
//
// Opt-in gate: nothing here runs — no env injection, no socket, no shim —
// unless a lead's definition sets `teams.mode: host` (see session.rs).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::broadcast;

use teams_backend::host::{ContextExited, HostError, PaneHost};
use teams_backend::protocol::SpawnAgentParams;

use crate::config::AgentDef;
use crate::session::{PtyManager, SessionState};

/// Hard grid ceiling shared with the frontend (`MAX_PANES`).
const GRID_MAX_PANES: usize = 9;
/// PTY size for hosted teammate sessions (frontend resizes on pane attach).
const SPAWN_COLS: u16 = 120;
const SPAWN_ROWS: u16 = 30;
/// Fallback correlation window: a teammate detected via hook with no
/// split-window RPC within this long is treated as in-process (host unused).
const FALLBACK_WINDOW: Duration = Duration::from_millis(2000);
/// Monitor tick for teammate-exit broadcast + lead-exit teardown.
const MONITOR_TICK: Duration = Duration::from_millis(300);

// ---------------- pure helpers (unit-tested) ----------------

/// Context id advertised for a session: tmux-style `%<id>`. The lead itself is
/// `%0` (session ids start at 1, so `%0` never collides).
pub fn context_id_for(session_id: u32) -> String {
    format!("%{session_id}")
}

/// Inverse of [`context_id_for`]: `%<n>` -> `n`. Returns None for a malformed
/// context id.
pub fn session_id_from_context(context_id: &str) -> Option<u32> {
    context_id.strip_prefix('%')?.parse().ok()
}

/// argv0 basename (last path component), used by the teammate binary allowlist.
fn basename(argv0: &str) -> &str {
    argv0.rsplit('/').next().unwrap_or(argv0)
}

/// Whether `argv0`'s basename is in the teammate binary allowlist.
pub fn binary_allowed(allowlist: &[String], argv0: &str) -> bool {
    let base = basename(argv0);
    allowlist.iter().any(|b| b == base)
}

/// Host `split-window` pane-limit decision. Unlike observe (which refuses to
/// create the pane), host always spawns the teammate PTY; this only decides
/// whether to show a "paneless" banner because a limit is already reached.
/// Returns the banner message when over any of the three limits.
pub fn host_over_limit(
    per_lead_host: usize,
    max_panes: usize,
    total_teammates: usize,
    global_max: usize,
    total_sessions: usize,
    grid_max: usize,
) -> Option<String> {
    if per_lead_host >= max_panes {
        return Some(format!(
            "lead の teammate ペイン上限（{max_panes}）に達したため、teammate はグリッドに配置されません（バックグラウンドで動作）。"
        ));
    }
    if total_teammates >= global_max {
        return Some(format!(
            "teammate ペインの合計上限（{global_max}）に達したため、teammate はグリッドに配置されません（バックグラウンドで動作）。"
        ));
    }
    if total_sessions >= grid_max {
        return Some(format!(
            "ペイン上限（{grid_max}）に達したため、teammate はグリッドに配置されません（バックグラウンドで動作）。"
        ));
    }
    None
}

/// Host/fallback correlation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCorrelation {
    /// A split-window RPC arrived at/after the hook: the shim is driving; host.
    Hosted,
    /// Still inside the window with no RPC yet.
    Pending,
    /// The window elapsed with no RPC: shim unused, fall back to observe.
    Fallback,
}

/// Pure time-window correlation: given the hook time, the most recent
/// split-window RPC time (if any), the current time, and the window, decide
/// whether host is working, still pending, or should fall back. All times are
/// unix-epoch milliseconds.
pub fn correlate_fallback(
    hook_at_ms: u64,
    last_spawn_ms: Option<u64>,
    now_ms: u64,
    window_ms: u64,
) -> HostCorrelation {
    if last_spawn_ms.is_some_and(|s| s >= hook_at_ms) {
        return HostCorrelation::Hosted;
    }
    if now_ms.saturating_sub(hook_at_ms) >= window_ms {
        HostCorrelation::Fallback
    } else {
        HostCorrelation::Pending
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Last `n` lines of `text` (host `capture` with a `lines` argument).
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

// ---------------- tmux shim script generation ----------------

/// App-data subpaths for the host runtime artifacts.
fn bin_dir(app_data: &Path) -> PathBuf {
    app_data.join("teams").join("bin")
}
fn socket_path(app_data: &Path, lead_id: u32) -> PathBuf {
    app_data
        .join("teams")
        .join("run")
        .join(format!("lead-{lead_id}.sock"))
}

/// Write `teams/bin/tmux` as an exec shim to the ptygrid binary's
/// `__tmux-compat` entry point (cmux-style). Idempotent: only writes when the
/// content differs. Returns the shim script path. `exe` is the current ptygrid
/// executable's absolute path.
pub fn ensure_tmux_shim(app_data: &Path, exe: &Path) -> std::io::Result<PathBuf> {
    let dir = bin_dir(app_data);
    std::fs::create_dir_all(&dir)?;
    let script = dir.join("tmux");
    let content = format!("#!/bin/sh\nexec \"{}\" __tmux-compat \"$@\"\n", exe.display());
    let up_to_date = std::fs::read_to_string(&script)
        .map(|existing| existing == content)
        .unwrap_or(false);
    if !up_to_date {
        std::fs::write(&script, &content)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(script)
}

// ---------------- per-lead shared state + registry ----------------

/// State shared between a lead's `PaneHost`, its exit monitor, the fallback
/// watcher, and the status command.
struct LeadShared {
    /// Set true the first time a split-window RPC spawns a teammate.
    host_used: AtomicBool,
    /// Unix-ms of the most recent split-window RPC (0 = none yet).
    last_spawn_ms: AtomicU64,
    /// Whether this lead has been downgraded to observe (host unavailable).
    fallback: AtomicBool,
    /// agent_id -> hook time (unix ms) for teammate detections awaiting a RPC.
    pending: Mutex<HashMap<String, u64>>,
    /// context_exited push events forwarded to persistent socket clients.
    exits: broadcast::Sender<ContextExited>,
}

impl LeadShared {
    fn new() -> Arc<Self> {
        let (exits, _) = broadcast::channel(64);
        Arc::new(LeadShared {
            host_used: AtomicBool::new(false),
            last_spawn_ms: AtomicU64::new(0),
            fallback: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
            exits,
        })
    }
}

struct LeadEntry {
    shared: Arc<LeadShared>,
    #[cfg_attr(not(unix), allow(dead_code))]
    server: tauri::async_runtime::JoinHandle<()>,
}

/// Managed Tauri state: the set of active host leads.
pub struct TeamsHostManager {
    leads: Arc<Mutex<HashMap<u32, LeadEntry>>>,
}

impl Default for TeamsHostManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamsHostManager {
    pub fn new() -> Self {
        TeamsHostManager {
            leads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u32, LeadEntry>> {
        match self.leads.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// Whether `lead_id` has an active host socket server.
    pub fn is_host_lead(&self, lead_id: u32) -> bool {
        self.lock().contains_key(&lead_id)
    }

    /// Record a teammate hook detection (for the fallback correlation). No-op
    /// when the lead is not an active host lead.
    pub fn note_teammate_hook(&self, lead_id: u32, agent_id: &str) {
        if let Some(entry) = self.lock().get(&lead_id) {
            entry
                .shared
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(agent_id.to_string(), now_ms());
        }
    }

    /// Correlate a teammate detection against split-window RPCs. None when the
    /// lead is unknown or the agent was never registered.
    pub fn correlation(
        &self,
        lead_id: u32,
        agent_id: &str,
        now_ms: u64,
    ) -> Option<HostCorrelation> {
        let leads = self.lock();
        let entry = leads.get(&lead_id)?;
        let hook_at = *entry
            .shared
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(agent_id)?;
        let last = match entry.shared.last_spawn_ms.load(Ordering::SeqCst) {
            0 => None,
            v => Some(v),
        };
        Some(correlate_fallback(
            hook_at,
            last,
            now_ms,
            FALLBACK_WINDOW.as_millis() as u64,
        ))
    }

    /// Flag a lead as downgraded to observe (host unavailable).
    pub fn mark_fallback(&self, lead_id: u32) {
        if let Some(entry) = self.lock().get(&lead_id) {
            entry.shared.fallback.store(true, Ordering::SeqCst);
        }
    }

    /// Test-only: register a lead with a no-op server task so the hook /
    /// correlation / fallback bookkeeping can be exercised without a socket.
    #[cfg(test)]
    fn register_test_lead(&self, lead_id: u32) {
        let shared = LeadShared::new();
        let server = tauri::async_runtime::spawn(async {});
        self.lock().insert(lead_id, LeadEntry { shared, server });
    }

    /// `(lead_id, fallback)` for every active host lead.
    fn snapshot(&self) -> Vec<(u32, bool)> {
        let mut v: Vec<(u32, bool)> = self
            .lock()
            .iter()
            .map(|(id, e)| (*id, e.shared.fallback.load(Ordering::SeqCst)))
            .collect();
        v.sort_by_key(|t| t.0);
        v
    }
}

// ---------------- the ptygrid PaneHost ----------------

/// `PaneHost` over the real ptygrid session manager. Generic over the runtime
/// so tests can drive it with `MockRuntime`; the trait object erases `R`.
struct PtygridPaneHost<R: Runtime> {
    app: AppHandle<R>,
    lead_id: u32,
    default_cwd: Option<PathBuf>,
    base_env: Vec<(String, String)>,
    teammate_binaries: Vec<String>,
    max_panes: usize,
    global_max: usize,
    shared: Arc<LeadShared>,
}

impl<R: Runtime> PtygridPaneHost<R> {
    fn manager(&self) -> Result<tauri::State<'_, PtyManager>, HostError> {
        self.app
            .try_state::<PtyManager>()
            .ok_or_else(|| HostError::Internal("session manager unavailable".into()))
    }

    /// `%0` resolves to the lead itself; `%<n>` to that session.
    fn resolve(&self, context_id: &str) -> Option<u32> {
        if context_id == "%0" {
            return Some(self.lead_id);
        }
        session_id_from_context(context_id)
    }
}

impl<R: Runtime> PaneHost for PtygridPaneHost<R> {
    fn spawn_agent(&self, params: SpawnAgentParams) -> Result<String, HostError> {
        let argv = params.command;
        let argv0 = argv
            .first()
            .ok_or_else(|| HostError::SpawnDenied("empty command".into()))?;
        if !binary_allowed(&self.teammate_binaries, argv0) {
            return Err(HostError::SpawnDenied(format!(
                "binary not in teammate allowlist: {}",
                basename(argv0)
            )));
        }

        // Record that the shim actually drove a spawn (defeats fallback).
        self.shared.host_used.store(true, Ordering::SeqCst);
        self.shared.last_spawn_ms.store(now_ms(), Ordering::SeqCst);

        let manager = self.manager()?;

        // Host always spawns; a reached limit only means "paneless + banner".
        let (per_lead_host, total_teammates, total_sessions) =
            manager.host_limit_inputs(self.lead_id);
        if let Some(message) = host_over_limit(
            per_lead_host,
            self.max_panes,
            total_teammates,
            self.global_max,
            total_sessions,
            GRID_MAX_PANES,
        ) {
            let _ = self
                .app
                .emit("teammate-banner", serde_json::json!({ "message": message }));
        }

        let cwd = params
            .cwd
            .map(PathBuf::from)
            .or_else(|| self.default_cwd.clone());
        let mut env = self.base_env.clone();
        for (k, v) in params.env {
            env.push((k, v));
        }
        let role = params.metadata.name.or(params.metadata.role);

        let id = manager
            .spawn_teammate(
                self.app.clone(),
                argv,
                cwd,
                env,
                self.lead_id,
                role,
                SPAWN_COLS,
                SPAWN_ROWS,
            )
            .map_err(HostError::Internal)?;
        Ok(context_id_for(id))
    }

    fn write(&self, context_id: &str, data: &[u8]) -> Result<(), HostError> {
        let id = self
            .resolve(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        self.manager()?.write_pty_bytes(id, data).map_err(|e| {
            if e.contains("not found") {
                HostError::ContextNotFound(context_id.into())
            } else {
                HostError::Internal(e)
            }
        })
    }

    fn capture(&self, context_id: &str, lines: Option<u32>) -> Result<String, HostError> {
        let id = self
            .resolve(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        let (text, rows, cols) = self
            .manager()?
            .output_snapshot(id)
            .map_err(|_| HostError::ContextNotFound(context_id.into()))?;
        let rendered = crate::ansi::render_terminal(&text, rows, cols);
        Ok(match lines {
            Some(n) => tail_lines(&rendered, n as usize),
            None => rendered,
        })
    }

    fn kill(&self, context_id: &str) -> Result<(), HostError> {
        let id = self
            .resolve(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        // kill_pty sets manual_kill, which suppresses autorestart.
        self.manager()?
            .kill_pty(id)
            .map_err(|_| HostError::ContextNotFound(context_id.into()))
    }

    fn list(&self) -> Vec<String> {
        let mut ids = vec![self.self_context_id()];
        if let Ok(manager) = self.manager() {
            for id in manager.host_teammate_ids(self.lead_id) {
                ids.push(context_id_for(id));
            }
        }
        ids
    }

    fn self_context_id(&self) -> String {
        "%0".into()
    }

    fn focus(&self, context_id: &str) -> Result<(), HostError> {
        let id = self
            .resolve(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        if self.manager()?.session_state(id).is_none() {
            return Err(HostError::ContextNotFound(context_id.into()));
        }
        let _ = self
            .app
            .emit("teammate-focus", serde_json::json!({ "id": id }));
        Ok(())
    }

    fn subscribe_exits(&self) -> broadcast::Receiver<ContextExited> {
        self.shared.exits.subscribe()
    }
}

// ---------------- lead setup / env injection ----------------

/// Set up the host runtime for a `teams.mode: host` lead and return the env
/// vars to inject into its PTY. Called from session.rs *only* for host leads
/// (the opt-in gate). On any setup failure it degrades to returning no env
/// (the lead runs as an ordinary session; teammates fall back to observe).
pub fn setup_lead<R: Runtime>(
    app: &AppHandle<R>,
    lead_id: u32,
    def: &AgentDef,
    cwd: &Path,
    base_env: &[(String, String)],
) -> Vec<(String, String)> {
    let teams = match &def.teams {
        Some(t) if t.is_host() => t,
        _ => return Vec::new(),
    };
    let app_data = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("teams host: cannot resolve app-data dir: {e}");
            return Vec::new();
        }
    };
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("teams host: cannot resolve current exe: {e}");
            return Vec::new();
        }
    };
    if let Err(e) = ensure_tmux_shim(&app_data, &exe) {
        eprintln!("teams host: cannot write tmux shim: {e}");
        return Vec::new();
    }

    let token = generate_token();
    let sock = socket_path(&app_data, lead_id);
    let global_max = app
        .try_state::<crate::config::ConfigManager>()
        .and_then(|cm| cm.current())
        .and_then(|(cfg, _)| cfg.teammates)
        .unwrap_or_default()
        .effective_global_max_panes() as usize;

    let shared = LeadShared::new();
    let host = PtygridPaneHost {
        app: app.clone(),
        lead_id,
        default_cwd: Some(cwd.to_path_buf()),
        base_env: base_env.to_vec(),
        teammate_binaries: teams.effective_teammate_binaries(),
        max_panes: teams.effective_max_panes() as usize,
        global_max,
        shared: Arc::clone(&shared),
    };

    if !start_server(app, lead_id, sock.clone(), token.clone(), host, Arc::clone(&shared)) {
        return Vec::new();
    }

    // Env injected into the lead PTY. PATH gets the shim dir prepended so the
    // fake `tmux` wins; the teams vars go last so they override any def env.
    let existing_path = base_env
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let bin = bin_dir(&app_data);
    let path_value = if existing_path.is_empty() {
        bin.display().to_string()
    } else {
        format!("{}:{}", bin.display(), existing_path)
    };
    vec![
        ("PTYGRID_TEAMS_SOCK".into(), sock.display().to_string()),
        ("PTYGRID_TEAMS_TOKEN".into(), token),
        (
            "TMUX".into(),
            format!("{},{},0", sock.display(), std::process::id()),
        ),
        ("TMUX_PANE".into(), "%0".into()),
        ("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".into(), "1".into()),
        ("PATH".into(), path_value),
    ]
}

/// Bind the socket, spawn the server + monitor, and register the lead. Returns
/// false if host state is unavailable (setup is then skipped). Unix only; on
/// other platforms host mode is unsupported and this is a no-op returning
/// false.
#[cfg(unix)]
fn start_server<R: Runtime>(
    app: &AppHandle<R>,
    lead_id: u32,
    sock: PathBuf,
    token: String,
    host: PtygridPaneHost<R>,
    shared: Arc<LeadShared>,
) -> bool {
    use teams_backend::server::{bind_socket, serve, ServerConfig};

    let Some(host_mgr) = app.try_state::<TeamsHostManager>() else {
        return false;
    };
    let registry = Arc::clone(&host_mgr.leads);

    let host_arc: Arc<dyn PaneHost> = Arc::new(host);
    let server_cfg = ServerConfig {
        auth_token: Some(token),
    };
    let sock_for_server = sock.clone();
    let server = tauri::async_runtime::spawn(async move {
        match bind_socket(&sock_for_server) {
            Ok(listener) => {
                let _ = serve(listener, host_arc, server_cfg).await;
            }
            Err(e) => eprintln!("teams host: cannot bind {}: {e}", sock_for_server.display()),
        }
    });

    // Exit/lifecycle monitor: broadcast teammate exits and tear the lead down
    // when it exits. Kept off the session hot path by polling.
    let monitor_app = app.clone();
    let monitor_registry = Arc::clone(&registry);
    let monitor_shared = Arc::clone(&shared);
    let monitor_sock = sock.clone();
    tauri::async_runtime::spawn(async move {
        let mut reported: HashSet<u32> = HashSet::new();
        let mut prev: HashSet<u32> = HashSet::new();
        loop {
            tokio::time::sleep(MONITOR_TICK).await;
            let lead_gone = {
                let Some(manager) = monitor_app.try_state::<PtyManager>() else {
                    break;
                };
                let states = manager.host_teammate_states(lead_id);
                let current: HashSet<u32> = states.iter().map(|(id, _, _)| *id).collect();
                for (id, state, code) in &states {
                    if *state == SessionState::Exited && reported.insert(*id) {
                        let _ = monitor_shared.exits.send(ContextExited {
                            context_id: context_id_for(*id),
                            exit_code: *code,
                        });
                    }
                }
                for id in prev.difference(&current) {
                    if reported.insert(*id) {
                        let _ = monitor_shared.exits.send(ContextExited {
                            context_id: context_id_for(*id),
                            exit_code: None,
                        });
                    }
                }
                prev = current;
                !matches!(
                    manager.session_state(lead_id),
                    Some(SessionState::Running)
                        | Some(SessionState::Restarting)
                        | Some(SessionState::Starting)
                )
            };
            if lead_gone {
                break;
            }
        }
        // Teardown: drop the registry entry, abort the server, remove the sock.
        if let Some(entry) = monitor_registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&lead_id)
        {
            entry.server.abort();
        }
        let _ = std::fs::remove_file(&monitor_sock);
    });

    registry
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(lead_id, LeadEntry { shared, server });
    true
}

#[cfg(not(unix))]
fn start_server<R: Runtime>(
    _app: &AppHandle<R>,
    _lead_id: u32,
    _sock: PathBuf,
    _token: String,
    _host: PtygridPaneHost<R>,
    _shared: Arc<LeadShared>,
) -> bool {
    false
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("getrandom: OS entropy unavailable");
    let mut out = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in &bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------------- fallback watcher (spec 6.3) ----------------

/// Spawn the 2s correlation for a host-lead teammate detection. If no
/// split-window RPC arrives within the window, mark the lead as fallen back,
/// emit `teammate-fallback`, and (when `fallback_to_observe`) create the
/// Phase 4.1 read-only transcript pane. A hosted teammate (RPC seen) is a
/// no-op.
pub fn watch_for_fallback<R: Runtime>(
    app: AppHandle<R>,
    lead_id: u32,
    agent_id: String,
    role: Option<String>,
    transcript_path: Option<PathBuf>,
    fallback_to_observe: bool,
) {
    std::thread::spawn(move || {
        std::thread::sleep(FALLBACK_WINDOW);
        let Some(host_mgr) = app.try_state::<TeamsHostManager>() else {
            return;
        };
        if host_mgr.correlation(lead_id, &agent_id, now_ms()) != Some(HostCorrelation::Fallback) {
            return;
        }
        host_mgr.mark_fallback(lead_id);
        let _ = app.emit(
            "teammate-fallback",
            serde_json::json!({
                "leadId": lead_id,
                "agentId": agent_id.clone(),
                "reason": "no split-window RPC within window; teammate is in-process (host unavailable)",
            }),
        );
        if fallback_to_observe {
            if let Some(manager) = app.try_state::<PtyManager>() {
                manager.create_transcript_session(
                    app.clone(),
                    agent_id,
                    role,
                    lead_id,
                    transcript_path,
                );
            }
        }
    });
}

// ---------------- status command ----------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadStatus {
    pub id: u32,
    pub mode: &'static str,
    pub fallback: bool,
    /// Live host-teammate session ids owned by this lead.
    pub teammates: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamsHostStatus {
    pub leads: Vec<LeadStatus>,
}

/// Assemble `teams_host_status`.
pub fn status<R: Runtime>(app: &AppHandle<R>) -> TeamsHostStatus {
    let Some(host_mgr) = app.try_state::<TeamsHostManager>() else {
        return TeamsHostStatus { leads: Vec::new() };
    };
    let manager = app.try_state::<PtyManager>();
    let leads = host_mgr
        .snapshot()
        .into_iter()
        .map(|(id, fallback)| LeadStatus {
            id,
            mode: "host",
            fallback,
            teammates: manager
                .as_ref()
                .map(|m| m.host_teammate_ids(id))
                .unwrap_or_default(),
        })
        .collect();
    TeamsHostStatus { leads }
}

// ---------------- in-process tmux shim (`__tmux-compat`) ----------------

#[cfg(unix)]
struct NullClient;

#[cfg(unix)]
impl teams_backend::shim::RpcClient for NullClient {
    fn call(&mut self, method: &str, _params: serde_json::Value) -> Result<serde_json::Value, String> {
        Err(format!("unexpected RPC {method} from a no-op command"))
    }
}

/// Handle a `__tmux-compat` invocation: parse the tmux subcommand, forward it
/// over the per-lead socket (`PTYGRID_TEAMS_SOCK` / `PTYGRID_TEAMS_TOKEN`), and
/// print tmux-compatible output. Returns the process exit code. `args` excludes
/// argv0 and the `__tmux-compat` marker.
#[cfg(unix)]
pub fn run_tmux_shim(args: &[String]) -> i32 {
    use teams_backend::shim::{execute, parse, TmuxCommand};
    use teams_backend::shim_client::SocketClient;

    let cmd = match parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ptygrid tmux-compat: {e}");
            return 1;
        }
    };
    let self_id_env = std::env::var("TMUX_PANE").ok();

    // No-ops (presence checks, option juggling) never touch the socket.
    if matches!(cmd, TmuxCommand::NoOp(_) | TmuxCommand::ListSessions) {
        return match execute(cmd, &mut NullClient, self_id_env.as_deref()) {
            Ok(out) => {
                print!("{}", out.stdout);
                out.exit_code
            }
            Err(e) => {
                eprintln!("ptygrid tmux-compat: {e}");
                1
            }
        };
    }

    let sock = match std::env::var("PTYGRID_TEAMS_SOCK") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            eprintln!("ptygrid tmux-compat: PTYGRID_TEAMS_SOCK is not set");
            return 1;
        }
    };
    let token = std::env::var("PTYGRID_TEAMS_TOKEN").ok();
    let mut client = match SocketClient::connect(&sock, token.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ptygrid tmux-compat: {e}");
            return 1;
        }
    };
    match execute(cmd, &mut client, self_id_env.as_deref()) {
        Ok(out) => {
            print!("{}", out.stdout);
            out.exit_code
        }
        Err(e) => {
            eprintln!("ptygrid tmux-compat: {e}");
            1
        }
    }
}

#[cfg(not(unix))]
pub fn run_tmux_shim(_args: &[String]) -> i32 {
    eprintln!("ptygrid tmux-compat: unix only");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- pure helpers -----

    #[test]
    fn context_id_round_trips_with_session_id() {
        assert_eq!(context_id_for(7), "%7");
        assert_eq!(session_id_from_context("%7"), Some(7));
        assert_eq!(session_id_from_context("%0"), Some(0));
        assert_eq!(session_id_from_context("7"), None);
        assert_eq!(session_id_from_context("%x"), None);
        assert_eq!(session_id_from_context(""), None);
    }

    #[test]
    fn allowlist_checks_argv0_basename() {
        let allow = vec!["claude".to_string()];
        assert!(binary_allowed(&allow, "claude"));
        assert!(binary_allowed(&allow, "/usr/local/bin/claude"));
        assert!(!binary_allowed(&allow, "bash"));
        assert!(!binary_allowed(&allow, "/bin/sh"));
        // Not a suffix match: "claude-evil" is a different basename.
        assert!(!binary_allowed(&allow, "claude-evil"));
    }

    #[test]
    fn host_over_limit_reports_each_reached_limit() {
        // Under all limits: no banner.
        assert_eq!(host_over_limit(0, 3, 0, 6, 0, 9), None);
        // Per-lead cap reached.
        assert!(host_over_limit(3, 3, 3, 6, 3, 9).is_some());
        // Global cap reached (per-lead has room).
        assert!(host_over_limit(0, 9, 6, 6, 6, 9).is_some());
        // Grid ceiling reached (per-lead + global have room).
        assert!(host_over_limit(0, 9, 0, 99, 9, 9).is_some());
    }

    #[test]
    fn correlate_fallback_windows() {
        let window = 2000;
        // A spawn at/after the hook means host is working.
        assert_eq!(
            correlate_fallback(1000, Some(1000), 5000, window),
            HostCorrelation::Hosted
        );
        assert_eq!(
            correlate_fallback(1000, Some(1500), 5000, window),
            HostCorrelation::Hosted
        );
        // A spawn strictly before the hook does not count.
        assert_eq!(
            correlate_fallback(1000, Some(900), 1500, window),
            HostCorrelation::Pending
        );
        assert_eq!(
            correlate_fallback(1000, Some(900), 3000, window),
            HostCorrelation::Fallback
        );
        // No spawn: pending inside the window, fallback after it.
        assert_eq!(
            correlate_fallback(1000, None, 2999, window),
            HostCorrelation::Pending
        );
        assert_eq!(
            correlate_fallback(1000, None, 3000, window),
            HostCorrelation::Fallback
        );
    }

    #[test]
    fn tail_lines_takes_last_n() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail_lines("a\nb", 10), "a\nb");
        assert_eq!(tail_lines("", 5), "");
    }

    #[tokio::test]
    async fn manager_tracks_hook_correlation_and_fallback() {
        let m = TeamsHostManager::new();
        // Unknown lead: not a host lead, no correlation, no snapshot entry.
        assert!(!m.is_host_lead(1));
        assert_eq!(m.correlation(1, "a1", now_ms()), None);
        assert!(m.snapshot().is_empty());

        m.register_test_lead(1);
        assert!(m.is_host_lead(1));

        // A hook with no split-window RPC: pending now, fallback past the window.
        m.note_teammate_hook(1, "a1");
        assert_eq!(
            m.correlation(1, "a1", now_ms()),
            Some(HostCorrelation::Pending)
        );
        assert_eq!(
            m.correlation(1, "a1", now_ms() + 5000),
            Some(HostCorrelation::Fallback)
        );
        // An unregistered agent has no correlation.
        assert_eq!(m.correlation(1, "other", now_ms()), None);

        // Fallback flag surfaces in the snapshot.
        assert_eq!(m.snapshot(), vec![(1, false)]);
        m.mark_fallback(1);
        assert_eq!(m.snapshot(), vec![(1, true)]);
    }

    #[cfg(unix)]
    #[test]
    fn tmux_shim_noop_needs_no_socket() {
        // Presence-check style subcommands succeed with exit 0 and never touch
        // the socket (no PTYGRID_TEAMS_SOCK required).
        assert_eq!(run_tmux_shim(&["list-sessions".to_string()]), 0);
        assert_eq!(run_tmux_shim(&["set-option".to_string(), "-g".into(), "x".into()]), 0);
    }

    #[test]
    fn ensure_tmux_shim_is_idempotent_and_executable() {
        let dir = std::env::temp_dir().join(format!(
            "ptygrid-shim-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = PathBuf::from("/opt/ptygrid/ptygrid");

        let script = ensure_tmux_shim(&dir, &exe).unwrap();
        let content = std::fs::read_to_string(&script).unwrap();
        assert!(content.starts_with("#!/bin/sh\n"));
        assert!(content.contains("/opt/ptygrid/ptygrid"));
        assert!(content.contains("__tmux-compat"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&script).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        // Idempotent: re-running does not change the content and does not add a
        // second file to the bin dir.
        let script2 = ensure_tmux_shim(&dir, &exe).unwrap();
        assert_eq!(script, script2);
        assert_eq!(std::fs::read_to_string(&script2).unwrap(), content);
        let entries = std::fs::read_dir(bin_dir(&dir)).unwrap().count();
        assert_eq!(entries, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ----- PaneHost integration via mock_app + real PtyManager -----

    #[cfg(unix)]
    fn mock_host(binaries: &[&str]) -> (tauri::AppHandle<tauri::test::MockRuntime>, PtygridPaneHost<tauri::test::MockRuntime>) {
        // The App is dropped on return; the cloned AppHandle keeps the managed
        // state alive (same pattern as the session.rs / teams_hooks tests).
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        handle.manage(PtyManager::new());
        let host = PtygridPaneHost {
            app: handle.clone(),
            lead_id: 1,
            default_cwd: None,
            base_env: Vec::new(),
            teammate_binaries: binaries.iter().map(|s| s.to_string()).collect(),
            max_panes: 3,
            global_max: 6,
            shared: LeadShared::new(),
        };
        (handle, host)
    }

    #[cfg(unix)]
    #[test]
    fn pane_host_spawn_write_capture_kill_roundtrip() {
        use std::time::Instant;
        let (_app, host) = mock_host(&["cat"]);

        // Allowlist rejects a non-listed binary.
        let denied = host.spawn_agent(SpawnAgentParams {
            command: vec!["/bin/sh".into()],
            cwd: None,
            env: Default::default(),
            metadata: Default::default(),
        });
        assert!(matches!(denied, Err(HostError::SpawnDenied(_))));

        // Spawn a real `cat` teammate.
        let ctx = host
            .spawn_agent(SpawnAgentParams {
                command: vec!["/bin/cat".into()],
                cwd: None,
                env: Default::default(),
                metadata: Default::default(),
            })
            .expect("spawn should succeed");
        assert!(ctx.starts_with('%'));
        // list includes self (%0) + the teammate.
        let list = host.list();
        assert!(list.contains(&"%0".to_string()));
        assert!(list.contains(&ctx));

        // Write is echoed by cat; capture should reflect it.
        host.write(&ctx, b"ping-42\r").expect("write should succeed");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = String::new();
        while Instant::now() < deadline {
            seen = host.capture(&ctx, None).unwrap_or_default();
            if seen.contains("ping-42") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen.contains("ping-42"), "capture was: {seen:?}");

        // Kill removes the teammate (no autorestart).
        host.kill(&ctx).expect("kill should succeed");
        // Unknown context ids are rejected.
        assert!(matches!(
            host.write("%9999", b"x"),
            Err(HostError::ContextNotFound(_))
        ));
        assert!(matches!(
            host.kill("not-a-context"),
            Err(HostError::ContextNotFound(_))
        ));
    }

    // ----- end-to-end over a real socket (server + real PaneHost) -----

    #[cfg(unix)]
    #[tokio::test]
    async fn socket_server_drives_real_pane_host() {
        use teams_backend::server::{bind_socket, serve, ServerConfig};
        use teams_backend::shim::RpcClient;
        use teams_backend::shim_client::SocketClient;

        let (_app, host) = mock_host(&["cat"]);
        let host_arc: Arc<dyn PaneHost> = Arc::new(host);

        let sock = std::env::temp_dir().join(format!(
            "ptygrid-hostsock-{}-{}.sock",
            std::process::id(),
            now_ms()
        ));
        let listener = bind_socket(&sock).unwrap();
        let cfg = ServerConfig {
            auth_token: Some("tok".into()),
        };
        let server = tauri::async_runtime::spawn(async move {
            let _ = serve(listener, host_arc, cfg).await;
        });

        let sock_str = sock.display().to_string();
        let out = tokio::task::spawn_blocking(move || {
            let mut client = SocketClient::connect(&sock_str, Some("tok"))?;
            // spawn a cat teammate
            let res = client.call(
                "spawn_agent",
                serde_json::json!({ "command": ["/bin/cat"] }),
            )?;
            let ctx = res["context_id"].as_str().unwrap().to_string();
            // write via base64 (the server decodes it)
            use base64::Engine as _;
            let data = base64::engine::general_purpose::STANDARD.encode(b"beep-7\r");
            client.call(
                "write",
                serde_json::json!({ "context_id": ctx, "data": data }),
            )?;
            // poll capture
            let mut seen = String::new();
            for _ in 0..100 {
                let cap = client.call(
                    "capture",
                    serde_json::json!({ "context_id": ctx }),
                )?;
                seen = cap["text"].as_str().unwrap_or_default().to_string();
                if seen.contains("beep-7") {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            client.call("kill", serde_json::json!({ "context_id": ctx }))?;
            Ok::<String, String>(seen)
        })
        .await
        .unwrap()
        .unwrap();
        assert!(out.contains("beep-7"), "captured: {out:?}");

        server.abort();
        let _ = std::fs::remove_file(&sock);
    }
}
