mod ansi;
mod commands;
mod config;
mod git_service;
mod project_state;
mod pty;
mod queen;
mod session;
mod worktree;

use config::ConfigManager;
use queen::QueenStatus;
use session::PtyManager;

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
            commands::spawn_shell,
            commands::write_pty,
            commands::resize_pty,
            commands::kill_pty,
            commands::load_config,
            commands::spawn_agent,
            commands::restart_session,
            commands::list_sessions,
            commands::save_project_state,
            commands::load_project_state,
            commands::resume_logical_session,
            commands::queen_status,
            commands::git_status,
            commands::git_diff,
            commands::git_stage,
            commands::git_unstage,
            commands::git_commit
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
