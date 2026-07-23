//! Tauri IPC command boundary.
//!
//! Keep command argument/return shapes here so service modules (`session`,
//! `config`, `queen`, and Phase 3 additions) stay independent from the
//! frontend transport layer.

use tauri::{AppHandle, Emitter, State};

use crate::app_settings::{self, ProjectDirs, ProjectsRoot};
use crate::config::{ConfigInfo, ConfigManager};
use crate::git_service::{self, GitCommitInfo, GitDiffInfo, GitStatusInfo};
use crate::project_state::{self, LogicalSession, ProjectState};
use crate::queen::{self, QueenStatus, QueenStatusInfo};
use crate::session::{PtyManager, SessionInfo};
use crate::teams_hooks::{self, RegisterResult, TeammateHooksInfo};

/// Start a PTY-backed shell and return its session id.
#[tauri::command]
pub fn spawn_shell(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    cols: u16,
    rows: u16,
    cmd: Option<String>,
    cwd: Option<String>,
) -> Result<u32, String> {
    manager.spawn_shell(app, cols, rows, cmd, cwd)
}

/// Forward key input to the PTY stdin.
#[tauri::command]
pub fn write_pty(manager: State<'_, PtyManager>, id: u32, data: String) -> Result<(), String> {
    manager.write_pty(id, data)
}

/// Resize a PTY.
#[tauri::command]
pub fn resize_pty(
    manager: State<'_, PtyManager>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    manager.resize_pty(id, cols, rows)
}

/// Terminate a session (never triggers autorestart).
#[tauri::command]
pub fn kill_pty(manager: State<'_, PtyManager>, id: u32) -> Result<(), String> {
    manager.kill_pty(id)
}

/// Load config for working folder `dir` (the project boundary; `~` expanded),
/// resolving the config file as working folder → launch folder → `~/.ptygrid`,
/// start watching the loaded file, and apply Queen config.
///
/// `allow_default` (default false) opts into the built-in default config when no
/// file is found anywhere, so a manual load succeeds (and can `cd`) without a
/// config file. The startup auto-load omits it, keeping the `not_found:` error
/// and its existing frontend fallback.
#[tauri::command]
pub fn load_config(
    app: AppHandle,
    config: State<'_, ConfigManager>,
    store: State<'_, crate::queen_store::QueenStore>,
    dir: Option<String>,
    allow_default: Option<bool>,
) -> Result<ConfigInfo, String> {
    let info = config.load(&app, dir, allow_default.unwrap_or(false))?;
    let q = info.config.queen.unwrap_or_default();
    queen::apply(&app, q.effective_enabled(), q.effective_port());
    // Phase 4.4.0: recompile agent-status rules + refresh enabled/timings from
    // the (possibly reloaded) config so pattern edits take effect immediately.
    crate::agent_status::apply(&app, &info.config);
    // Phase 4.4.2: swap in the (possibly reloaded) notifications block.
    crate::notifications::apply(&app, &info.config);
    // Phase 5.5.0: swap in the (possibly reloaded) `mcp:` block for the /mcp
    // compat middleware — an open TCP connection is unaffected, since
    // McpCompatHandle::get reads fresh per request.
    crate::queen_compat::apply(&app, &info.config);
    // Phase 5.0.1: detect workflow runs left "running" in the Queen DB from
    // before a crash/restart (the in-memory WorkflowRegistry is lost on
    // restart, the persisted row is not) and let the frontend prompt Y/N.
    // Best-effort: a lookup failure must not fail config load itself.
    if let Ok(candidates) =
        crate::orchestrator::list_resumable_workflow_runs(&store, std::path::Path::new(&info.dir))
    {
        if !candidates.is_empty() {
            let _ = app.emit("workflow-resume-pending", candidates);
        }
    }
    Ok(info)
}

/// Trust a working folder for autostart / `worktree.setup` (security Finding
/// S2). After this, a `project`/`launch`-origin config loaded from `dir` reports
/// `trusted: true` and the frontend runs its autostart loop. `~` is expanded and
/// the path canonicalized before storage. Idempotent.
#[tauri::command]
pub fn trust_working_folder(app: AppHandle, dir: String) -> Result<crate::trust::TrustInfo, String> {
    crate::trust::add_trusted(&app, &dir)
}

/// Report whether a working folder is in the trusted set (folder-level, origin
/// agnostic). Optional companion to `trust_working_folder`.
#[tauri::command]
pub fn is_working_folder_trusted(
    app: AppHandle,
    dir: String,
) -> Result<crate::trust::TrustInfo, String> {
    crate::trust::check_trusted(&app, &dir)
}

/// Return the built-in Queen MCP server status.
#[tauri::command]
pub fn queen_status(status: State<'_, QueenStatus>) -> Result<QueenStatusInfo, String> {
    Ok(status.info())
}

/// Return teammate hooks info (enabled/notifications/port/token/scope) so the
/// frontend can build a settings.json snippet.
#[tauri::command]
pub fn teammate_hooks_info(app: AppHandle) -> Result<TeammateHooksInfo, String> {
    Ok(teams_hooks::hooks_info(&app))
}

/// Merge the ptygrid HTTP hooks into `~/.claude/settings.json` (user) or
/// `<project>/.claude/settings.json` (project), preserving existing content.
#[tauri::command]
pub fn register_teammate_hooks(app: AppHandle, scope: String) -> Result<RegisterResult, String> {
    teams_hooks::register(&app, &scope)
}

/// Rotate the persisted auth token(s) and update the running auth layers so the
/// new value takes effect without restarting the Queen server. `which` is
/// `"hook"`, `"queen"`, or `"all"` (default `"all"`). Returns which tokens were
/// regenerated. After this the registered settings.json / MCP URL are stale, so
/// the frontend prompts for re-registration.
#[tauri::command]
pub fn regenerate_auth_tokens(
    app: AppHandle,
    which: Option<String>,
) -> Result<crate::token_store::RegenerateResult, String> {
    crate::token_store::regenerate(&app, which.as_deref())
}

/// Phase 4.2: report active host leads (mode, fallback state, live teammate
/// session ids) for the Teammates panel.
#[tauri::command]
pub fn teams_host_status(app: AppHandle) -> Result<crate::teams_host::TeamsHostStatus, String> {
    Ok(crate::teams_host::status(&app))
}

/// Launch a loaded agent/process definition by name.
#[tauri::command]
pub fn spawn_agent(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    config: State<'_, ConfigManager>,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    let (def, dir) = config.resolve_def(&name)?;
    manager.spawn_agent(app, &def, &dir, cols, rows)
}

/// Phase 4.3: launch a named team preset — the same backend function as the
/// Queen `spawn_team` tool (CONTRACT.md Phase 4.3). Individual member
/// failures land in the report; only "no config" / "unknown preset" reject.
#[tauri::command]
pub fn spawn_team(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    config: State<'_, ConfigManager>,
    store: State<'_, crate::queen_store::QueenStore>,
    preset: String,
    cols: u16,
    rows: u16,
) -> Result<crate::team_presets::TeamStartReport, String> {
    crate::team_presets::start_team(&app, &manager, &config, &store, &preset, cols, rows)
}

/// Phase 5.0.0.e/f: launch a named workflow declared under workflows: in
/// ptygrid.yml — the same backend function as the Queen `spawn_workflow`
/// tool. MVO supports pipeline/fan-out; supervisor/handoff reject with a
/// "not implemented in MVO" error until Phase 5.0.4.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn spawn_workflow(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    config: State<'_, ConfigManager>,
    store: State<'_, crate::queen_store::QueenStore>,
    registry: State<'_, crate::orchestrator::WorkflowRegistry>,
    name: String,
    cols: u16,
    rows: u16,
) -> Result<crate::orchestrator::WorkflowRun, String> {
    crate::orchestrator::spawn_workflow(
        &app, &manager, &config, &store, &registry, &name, cols, rows,
    )
}

/// Cancel a running workflow: kill every step's live PTY session and mark
/// the run Cancelled. Idempotent — a no-op on an already-terminal run.
#[tauri::command]
pub fn cancel_workflow(
    manager: State<'_, PtyManager>,
    config: State<'_, ConfigManager>,
    store: State<'_, crate::queen_store::QueenStore>,
    registry: State<'_, crate::orchestrator::WorkflowRegistry>,
    run_id: String,
) -> Result<crate::orchestrator::WorkflowRun, String> {
    crate::orchestrator::cancel_workflow(&manager, &config, &store, &registry, &run_id)
}

/// Phase 5.0.1: resume a workflow run left `running` in the Queen DB from
/// before a crash/restart. Steps that were `Running` when the app died are
/// reset to `Pending` (their PTY is gone) so the existing 5.0.0.c driver
/// respawns them on its next tick; terminal steps are kept as persisted.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn resume_workflow(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    config: State<'_, ConfigManager>,
    store: State<'_, crate::queen_store::QueenStore>,
    registry: State<'_, crate::orchestrator::WorkflowRegistry>,
    run_id: String,
    cols: u16,
    rows: u16,
) -> Result<crate::orchestrator::WorkflowRun, String> {
    crate::orchestrator::resume_workflow(
        &app, &manager, &config, &store, &registry, &run_id, cols, rows,
    )
}

/// Phase 5.0.1: discard a run left `running` from before a crash/restart
/// instead of resuming it — marks it `cancelled` in the Queen DB (with an
/// "abandoned after restart" note) so `load_config` never re-prompts for it.
#[tauri::command]
pub fn abandon_workflow(
    config: State<'_, ConfigManager>,
    store: State<'_, crate::queen_store::QueenStore>,
    run_id: String,
) -> Result<(), String> {
    crate::orchestrator::abandon_workflow(&config, &store, &run_id)
}

/// Return every workflow run currently held in memory (live + recently
/// terminated), same in-memory registry the Queen workflow tools use.
#[tauri::command]
pub fn list_workflow_runs(
    registry: State<'_, crate::orchestrator::WorkflowRegistry>,
) -> Result<Vec<crate::orchestrator::WorkflowRun>, String> {
    Ok(registry.list())
}

/// Kill and respawn a session while preserving its id.
#[tauri::command]
pub fn restart_session(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    id: u32,
) -> Result<(), String> {
    manager.restart_session(app, id)
}

/// Return all current sessions.
#[tauri::command]
pub fn list_sessions(manager: State<'_, PtyManager>) -> Result<Vec<SessionInfo>, String> {
    Ok(manager.list_sessions())
}

/// Persist versioned logical state under the Tauri app-data directory.
#[tauri::command]
pub fn save_project_state(app: AppHandle, state: ProjectState) -> Result<(), String> {
    project_state::save(&app, state)
}

/// Load one project's state, or the last active project when dir is omitted.
#[tauri::command]
pub fn load_project_state(
    app: AppHandle,
    dir: Option<String>,
) -> Result<Option<ProjectState>, String> {
    project_state::load(&app, dir)
}

/// Relaunch a saved logical session from the currently loaded definition.
#[tauri::command]
pub fn resume_logical_session(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    config: State<'_, ConfigManager>,
    session: LogicalSession,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    match session {
        LogicalSession::Definition { name, worktree } => {
            let (def, dir) = config.resolve_def(&name)?;
            manager.resume_agent(app, &def, &dir, cols, rows, worktree)
        }
        LogicalSession::Shell => manager.spawn_shell(app, cols, rows, None, None),
    }
}

/// Return the persisted projects root (bulk-cd helper), or `null` when unset.
#[tauri::command]
pub fn get_projects_root(app: AppHandle) -> Result<ProjectsRoot, String> {
    app_settings::get_root(&app)
}

/// Validate (`~` expanded, must be an existing directory) and persist the
/// projects root. Returns the stored (verbatim) root.
#[tauri::command]
pub fn set_projects_root(app: AppHandle, root: String) -> Result<ProjectsRoot, String> {
    app_settings::set_root(&app, root)
}

/// List non-hidden directory names directly under the saved projects root,
/// sorted, capped at 200 (`truncated` flags overflow).
#[tauri::command]
pub fn list_project_dirs(app: AppHandle) -> Result<ProjectDirs, String> {
    app_settings::list_dirs(&app)
}

fn project_dir(config: &ConfigManager, dir: Option<String>) -> Result<std::path::PathBuf, String> {
    if let Some(dir) = dir {
        return Ok(std::path::PathBuf::from(dir));
    }
    if let Some((_config, config_dir)) = config.current() {
        return Ok(config_dir);
    }
    std::env::current_dir().map_err(|e| format!("cannot determine current dir: {e}"))
}

/// Return read-only Git repository and worktree status.
#[tauri::command]
pub fn git_status(
    config: State<'_, ConfigManager>,
    dir: Option<String>,
) -> Result<GitStatusInfo, String> {
    let dir = project_dir(&config, dir)?;
    git_service::status(&dir)
}

/// Return a bounded unified diff without changing the index or worktree.
#[tauri::command]
pub fn git_diff(
    config: State<'_, ConfigManager>,
    dir: Option<String>,
    path: Option<String>,
    staged: Option<bool>,
) -> Result<GitDiffInfo, String> {
    let dir = project_dir(&config, dir)?;
    git_service::diff(&dir, path, staged.unwrap_or(false))
}

/// Stage only the explicitly supplied repository-relative paths.
#[tauri::command]
pub fn git_stage(
    config: State<'_, ConfigManager>,
    dir: Option<String>,
    paths: Vec<String>,
) -> Result<GitStatusInfo, String> {
    let dir = project_dir(&config, dir)?;
    git_service::stage(&dir, paths)
}

/// Unstage only the explicitly supplied repository-relative paths.
#[tauri::command]
pub fn git_unstage(
    config: State<'_, ConfigManager>,
    dir: Option<String>,
    paths: Vec<String>,
) -> Result<GitStatusInfo, String> {
    let dir = project_dir(&config, dir)?;
    git_service::unstage(&dir, paths)
}

/// Commit the current index. Hooks and repository signing config are honored.
#[tauri::command]
pub async fn git_commit(
    config: State<'_, ConfigManager>,
    dir: Option<String>,
    message: String,
) -> Result<GitCommitInfo, String> {
    let dir = project_dir(&config, dir)?;
    tauri::async_runtime::spawn_blocking(move || git_service::commit(&dir, message))
        .await
        .map_err(|e| format!("git commit task failed: {e}"))?
}
