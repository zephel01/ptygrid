mod ansi;
mod app_settings;
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
mod teams_host;
mod transcript;
mod worktree;

pub use config::capture_launch_dir;
use config::ConfigManager;
use queen::QueenStatus;
use session::PtyManager;
use tauri::Manager;
use teams_hooks::TeamsHooks;
use teams_host::TeamsHostManager;

/// Phase 4.2: handle the cmux-style `__tmux-compat` re-exec. When ptygrid is
/// invoked as `ptygrid __tmux-compat <tmux args...>` (via the generated
/// `teams/bin/tmux` shim on a host lead's PATH), process the tmux subcommand
/// over the per-lead socket and return the exit code WITHOUT initializing any
/// GUI. Returns `None` for a normal launch. Call this first in `main`.
pub fn run_tmux_compat_if_requested() -> Option<i32> {
    let mut args = std::env::args();
    let _exe = args.next();
    match args.next().as_deref() {
        Some("__tmux-compat") => Some(teams_host::run_tmux_shim(&args.collect::<Vec<_>>())),
        _ => None,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(PtyManager::new())
        .manage(ConfigManager::new())
        .manage(QueenStatus::new())
        .manage(TeamsHooks::new())
        .manage(TeamsHostManager::new())
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
            commands::teams_host_status,
            commands::git_status,
            commands::git_diff,
            commands::git_stage,
            commands::git_unstage,
            commands::git_commit,
            commands::get_projects_root,
            commands::set_projects_root,
            commands::list_project_dirs
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
