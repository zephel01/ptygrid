// Low-level PTY plumbing shared by the session manager.
//
// The portable-pty 0.9 API calls used here were validated by the standalone
// `pty-core-check` smoke-test crate:
//   native_pty_system(), openpty(PtySize), CommandBuilder,
//   pair.slave.spawn_command(cmd), drop(pair.slave),
//   pair.master.take_writer(), pair.master.try_clone_reader(),
//   master.resize(PtySize), child.kill(), child.wait().

use std::io::{Read, Write};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Everything produced by opening a PTY and spawning a command into it.
pub struct PtyParts {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub reader: Box<dyn Read + Send>,
}

/// Open a PTY of the given size and spawn `cmd` attached to its slave side.
/// The slave is dropped after spawn so the master reader sees EOF when the
/// child exits.
pub fn open_and_spawn(cmd: CommandBuilder, cols: u16, rows: u16) -> Result<PtyParts, String> {
    let pty_system = native_pty_system();

    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn_command failed: {e}"))?;
    drop(pair.slave);

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("take_writer failed: {e}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("try_clone_reader failed: {e}"))?;

    Ok(PtyParts {
        master: pair.master,
        writer,
        child,
        reader,
    })
}

/// Default shell when spawn_shell gets no cmd.
#[cfg(windows)]
pub fn default_shell() -> String {
    "powershell.exe".to_string()
}

#[cfg(not(windows))]
pub fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

/// Resolve a pid to a short process name (used for SessionInfo.foreground).
/// Linux: /proc/<pid>/comm (no dependency, no subprocess).
/// macOS / other unix: `ps -o comm= -p <pid>` + basename — chosen over the
/// sysinfo crate to avoid a large transitive dependency for one lookup.
/// Windows: not implemented (returns None).
#[cfg(target_os = "linux")]
pub fn process_name(pid: i32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = comm.trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn process_name(pid: i32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let comm = String::from_utf8_lossy(&out.stdout);
    let comm = comm.trim();
    // ps may print a full path (e.g. /bin/zsh) — keep the basename, and
    // strip a login-shell "-" prefix (-zsh).
    let base = comm.rsplit('/').next().unwrap_or(comm);
    let base = base.strip_prefix('-').unwrap_or(base);
    (!base.is_empty()).then(|| base.to_string())
}

#[cfg(not(unix))]
pub fn process_name(_pid: i32) -> Option<String> {
    None
}

/// User home directory (spawn_shell default working directory).
pub fn home_dir() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").ok().filter(|s| !s.is_empty())
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME").ok().filter(|s| !s.is_empty())
    }
}
