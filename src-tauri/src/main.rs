// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Capture the process launch directory (the folder ptygrid was started
    // from) FIRST, before fix-path-env or any Tauri setup runs, so config
    // resolution can use it as the ② launch-folder candidate even if a later
    // step changes the process cwd.
    ptygrid_lib::capture_launch_dir();
    // Desktop launchers on Linux/macOS do not inherit PATH additions from the
    // user's shell startup files. Restore it before any PTY, Git, or setup
    // command is started so packaged builds can find agent CLIs as expected.
    let _ = fix_path_env::fix();
    ptygrid_lib::run();
}
