//! Tauri IPC command boundary.
//!
//! Keep command argument/return shapes here so service modules (`session`,
//! `config`, `queen`, and Phase 3 additions) stay independent from the
//! frontend transport layer.

use tauri::{AppHandle, State};

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

/// Read `<dir>/ptygrid.yml` (legacy: mterm.yml), start watching it, and apply Queen config.
#[tauri::command]
pub fn load_config(
    app: AppHandle,
    config: State<'_, ConfigManager>,
    dir: Option<String>,
) -> Result<ConfigInfo, String> {
    let info = config.load(&app, dir)?;
    let q = info.config.queen.unwrap_or_default();
    queen::apply(&app, q.effective_enabled(), q.effective_port());
    Ok(info)
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
