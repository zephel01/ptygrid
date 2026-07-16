mod ansi;
mod commands;
mod config;
mod git_service;
mod project_state;
mod pty;
mod queen;
mod queen_store;
mod resource_monitor;
mod session;
mod teams_hooks;
mod worktree;

use config::ConfigManager;
use queen::QueenStatus;
use session::PtyManager;
use tauri::Manager;
use teams_hooks::TeamsHooks;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyManager::new())
        .manage(ConfigManager::new())
        .manage(QueenStatus::new())
        .manage(TeamsHooks::new())
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let queen_store =
                queen_store::QueenStore::open(&app_data.join("queen").join("queen.sqlite3"))
                    .map_err(std::io::Error::other)?;
            app.manage(queen_store);
            // Queen starts with defaults; load_config may adjust it later.
            queen::start_default(&app.handle().clone());
            resource_monitor::start(&app.handle().clone());
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
            commands::teammate_hooks_info,
            commands::register_teammate_hooks,
            commands::git_status,
            commands::git_diff,
            commands::git_stage,
            commands::git_unstage,
            commands::git_commit
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
