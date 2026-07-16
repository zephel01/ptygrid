// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Desktop launchers on Linux/macOS do not inherit PATH additions from the
    // user's shell startup files. Restore it before any PTY, Git, or setup
    // command is started so packaged builds can find agent CLIs as expected.
    let _ = fix_path_env::fix();
    ptygrid_lib::run();
}
