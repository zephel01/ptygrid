mod ansi;
mod config;
mod pty;
mod queen;
mod session;

use config::{ConfigInfo, ConfigManager};
use queen::{QueenStatus, QueenStatusInfo};
use session::{PtyManager, SessionInfo};
use tauri::{AppHandle, State};

/// spawn_shell: start a PTY-backed shell and return its session id.
/// cmd omitted -> $SHELL (fallback /bin/bash; powershell.exe on Windows).
/// cwd omitted -> user home dir. (Phase 1: optional cwd added.)
#[tauri::command]
fn spawn_shell(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    cols: u16,
    rows: u16,
    cmd: Option<String>,
    cwd: Option<String>,
) -> Result<u32, String> {
    manager.spawn_shell(app, cols, rows, cmd, cwd)
}

/// write_pty: forward key input to the PTY stdin.
#[tauri::command]
fn write_pty(manager: State<'_, PtyManager>, id: u32, data: String) -> Result<(), String> {
    manager.write_pty(id, data)
}

/// resize_pty: resize the PTY.
#[tauri::command]
fn resize_pty(
    manager: State<'_, PtyManager>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    manager.resize_pty(id, cols, rows)
}

/// kill_pty: terminate a session (never triggers autorestart).
#[tauri::command]
fn kill_pty(manager: State<'_, PtyManager>, id: u32) -> Result<(), String> {
    manager.kill_pty(id)
}

/// load_config: read <dir>/mterm.yml and start watching it. Applies the
/// queen block: server restarts only when the port changed; enabled=false
/// stops it.
#[tauri::command]
fn load_config(
    app: AppHandle,
    config: State<'_, ConfigManager>,
    dir: Option<String>,
) -> Result<ConfigInfo, String> {
    let info = config.load(&app, dir)?;
    let q = info.config.queen.unwrap_or_default();
    queen::apply(&app, q.effective_enabled(), q.effective_port());
    Ok(info)
}

/// queen_status: state of the built-in Queen MCP server.
#[tauri::command]
fn queen_status(status: State<'_, QueenStatus>) -> Result<QueenStatusInfo, String> {
    Ok(status.info())
}

/// spawn_agent: launch a loaded agent/process definition by name.
#[tauri::command]
fn spawn_agent(
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

/// restart_session: kill and respawn keeping the same session id.
#[tauri::command]
fn restart_session(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    id: u32,
) -> Result<(), String> {
    manager.restart_session(app, id)
}

/// list_sessions: all current sessions.
#[tauri::command]
fn list_sessions(manager: State<'_, PtyManager>) -> Result<Vec<SessionInfo>, String> {
    Ok(manager.list_sessions())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyManager::new())
        .manage(ConfigManager::new())
        .manage(QueenStatus::new())
        .setup(|app| {
            // Queen starts with defaults; load_config may adjust it later.
            queen::start_default(&app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            spawn_shell,
            write_pty,
            resize_pty,
            kill_pty,
            load_config,
            spawn_agent,
            restart_session,
            list_sessions,
            queen_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
