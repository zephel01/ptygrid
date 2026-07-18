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

/// Interpreters whose executable name (`node`, `python`, …) says nothing about
/// which agent is running — the real identity is in the command-line arguments
/// (e.g. `node …/grok-cli/index.js`). For these we look past `comm` at the full
/// command line. Agent CLIs that are native binaries (claude, codex) report
/// their own `comm` and never reach this path.
fn is_interpreter(base: &str) -> bool {
    matches!(
        base,
        "node" | "nodejs" | "bun" | "deno" | "python" | "python3" | "pythonw" | "ruby"
    )
}

/// Canonical agent names recognizable inside an interpreter's command line.
/// Ordered by specificity; first substring hit (case-insensitive) wins. These
/// tokens appear in the agent's install path or launcher name and are
/// distinctive enough that a false hit from an unrelated argument is unlikely.
const KNOWN_AGENTS: &[&str] = &["claude", "codex", "grok", "aider", "gemini"];

/// Recognize a known agent from an interpreter command line, so a grok CLI that
/// runs as `node` still shows/detects as "grok".
fn agent_from_command(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    KNOWN_AGENTS
        .iter()
        .find(|a| lower.contains(**a))
        .map(|a| (*a).to_string())
}

/// Basename (extension stripped) of the first script-like argument in a command
/// line, e.g. `node /x/y/server.mjs --port 3` → "server". Fallback for an
/// interpreter running an unrecognized script so we show more than "node".
fn script_basename(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .skip(1) // skip the interpreter path itself
        .map(|tok| tok.rsplit('/').next().unwrap_or(tok))
        .find(|base| {
            [".js", ".mjs", ".cjs", ".ts", ".py"]
                .iter()
                .any(|ext| base.ends_with(ext))
        })
        .map(|base| base.rsplit_once('.').map_or(base, |(stem, _)| stem))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Basename of a `comm` value, stripping any path and a login-shell "-" prefix
/// (`-zsh` → `zsh`).
fn comm_basename(comm: &str) -> String {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    base.strip_prefix('-').unwrap_or(base).to_string()
}

/// Foreground commands whose first non-option argument is a connection
/// destination worth surfacing next to the process name (Phase 4.4.3: pane
/// header / status sidebar show `ssh user@host` instead of just `ssh`, so a
/// command typed into the wrong pane is caught before it runs). Allowlist so
/// the per-second sampler only pays the extra argv lookup for panes actually
/// running one of these.
fn has_destination_detail(name: &str) -> bool {
    matches!(name, "ssh")
}

/// ssh options that consume a SEPARATE value argument (per `man ssh`). Any
/// other `-x` token is either a value-less flag (possibly combined, `-4A`) or
/// bundles its value (`-p22`) and is skipped whole. `-l` is handled specially
/// so the login can be folded into the shown destination.
const SSH_VALUE_FLAGS: &[&str] = &[
    "-B", "-b", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-m", "-O", "-o", "-P",
    "-p", "-Q", "-R", "-S", "-W", "-w",
];

/// Destination shown for an ssh argv: the first non-option argument
/// (`user@host`, a ssh_config alias, or a `ssh://` authority). A `-l <user>`
/// login is folded in as `user@dest` when the destination itself has no `@`.
/// None for an argv with no destination (e.g. `ssh -Q cipher`... still returns
/// the query token — acceptable: ssh exits immediately and the next 1s tick
/// clears it).
fn ssh_destination(args: &[String]) -> Option<String> {
    let mut login: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(tok) = iter.next() {
        // URI form: keep the authority (user@host:port), drop scheme and path.
        if let Some(rest) = tok.strip_prefix("ssh://") {
            let authority = rest.split('/').next().unwrap_or(rest);
            return (!authority.is_empty()).then(|| authority.to_string());
        }
        if let Some(rest) = tok.strip_prefix("-l") {
            if rest.is_empty() {
                login = iter.next().cloned(); // `-l user`
            } else {
                login = Some(rest.to_string()); // bundled `-luser`
            }
            continue;
        }
        if tok.starts_with('-') && tok.len() > 1 {
            if SSH_VALUE_FLAGS.contains(&tok.as_str()) {
                iter.next(); // skip this flag's separate value
            }
            // Bundled values (`-p22`) and combined boolean flags (`-4A`) are
            // single tokens; nothing extra to skip.
            continue;
        }
        // First non-option token = destination (anything after it is the
        // remote command and must not be consumed).
        return match login {
            Some(user) if !tok.contains('@') => Some(format!("{user}@{tok}")),
            _ => Some(tok.clone()),
        };
    }
    None
}

/// Resolve a pid to a short process name (used for SessionInfo.foreground).
/// Linux: /proc/<pid>/comm (+ /proc/<pid>/cmdline for interpreters).
/// macOS / other unix: `ps -o comm=/command= -p <pid>` — chosen over the
/// sysinfo crate to avoid a large transitive dependency for one lookup.
/// Windows: not implemented (returns None). When `comm` is a generic
/// interpreter, the command line is inspected so e.g. a node-based grok CLI
/// resolves to "grok" instead of "node".
#[cfg(target_os = "linux")]
pub fn process_name(pid: i32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let base = comm_basename(comm.trim());
    if base.is_empty() {
        return None;
    }
    if !is_interpreter(&base) {
        return Some(base);
    }
    // cmdline is NUL-separated argv; join with spaces for scanning.
    if let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        let command = raw
            .split(|b| *b == 0)
            .map(|c| String::from_utf8_lossy(c))
            .collect::<Vec<_>>()
            .join(" ");
        let command = command.trim();
        if !command.is_empty() {
            if let Some(agent) = agent_from_command(command) {
                return Some(agent);
            }
            if let Some(script) = script_basename(command) {
                return Some(script);
            }
        }
    }
    Some(base)
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn process_name(pid: i32) -> Option<String> {
    let comm = ps_field(pid, "comm=")?;
    let base = comm_basename(&comm);
    if base.is_empty() {
        return None;
    }
    if !is_interpreter(&base) {
        return Some(base);
    }
    // Interpreter: look at the full command line for a known agent or script.
    if let Some(command) = ps_field(pid, "command=") {
        if let Some(agent) = agent_from_command(&command) {
            return Some(agent);
        }
        if let Some(script) = script_basename(&command) {
            return Some(script);
        }
    }
    Some(base)
}

/// Read a single `ps -o <field>= -p <pid>` value, trimmed. `None` on failure or
/// empty output.
#[cfg(all(unix, not(target_os = "linux")))]
fn ps_field(pid: i32, field: &str) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&out.stdout);
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(not(unix))]
pub fn process_name(_pid: i32) -> Option<String> {
    None
}

/// Extra display detail for a resolved foreground process — currently the ssh
/// destination (Phase 4.4.3). `name` is the value `process_name` returned for
/// the same pid; the allowlist check runs first so non-matching processes cost
/// nothing. Linux reads /proc/<pid>/cmdline; macOS runs `ps -o command=`.
/// Windows / lookup failure: None.
#[cfg(target_os = "linux")]
pub fn process_detail(pid: i32, name: &str) -> Option<String> {
    if !has_destination_detail(name) {
        return None;
    }
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    ssh_destination(&args)
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn process_detail(pid: i32, name: &str) -> Option<String> {
    if !has_destination_detail(name) {
        return None;
    }
    // `command=` joins argv with spaces; ssh destinations/flags never contain
    // spaces themselves, so whitespace-splitting reconstructs argv well enough.
    let command = ps_field(pid, "command=")?;
    let args: Vec<String> = command.split_whitespace().map(str::to_string).collect();
    ssh_destination(&args)
}

#[cfg(not(unix))]
pub fn process_detail(_pid: i32, _name: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comm_basename_strips_path_and_login_dash() {
        assert_eq!(comm_basename("/bin/zsh"), "zsh");
        assert_eq!(comm_basename("-zsh"), "zsh");
        assert_eq!(comm_basename("claude"), "claude");
        assert_eq!(comm_basename("/usr/local/bin/node"), "node");
    }

    #[test]
    fn interpreters_are_flagged() {
        assert!(is_interpreter("node"));
        assert!(is_interpreter("bun"));
        assert!(is_interpreter("python3"));
        assert!(!is_interpreter("claude"));
        assert!(!is_interpreter("codex"));
        assert!(!is_interpreter("zsh"));
    }

    #[test]
    fn node_grok_launcher_resolves_to_grok() {
        // Exactly the command line observed on macOS (`pgrep -fl grok`): the
        // node-based grok launcher. Its comm is "node"; the identity is the path.
        let command =
            "node /Users/h.yamamoto/.local/state/fnm_multishells/82380_1784218739947/bin/grok";
        assert_eq!(agent_from_command(command).as_deref(), Some("grok"));
    }

    #[test]
    fn agent_from_command_recognizes_known_agents_case_insensitively() {
        assert_eq!(
            agent_from_command("node /opt/Claude-Code/cli.js").as_deref(),
            Some("claude")
        );
        assert_eq!(
            agent_from_command("python3 -m aider.main").as_deref(),
            Some("aider")
        );
        // No known agent token → None (falls back to script basename).
        assert_eq!(agent_from_command("node /srv/app/server.mjs"), None);
    }

    #[test]
    fn script_basename_strips_dir_and_extension() {
        assert_eq!(
            script_basename("node /srv/app/server.mjs --port 3").as_deref(),
            Some("server")
        );
        assert_eq!(
            script_basename("python3 /x/tool.py").as_deref(),
            Some("tool")
        );
        // First arg after the interpreter that is not script-like → skip; none
        // here → None.
        assert_eq!(script_basename("node --version"), None);
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ssh_destination_takes_first_non_option_argument() {
        assert_eq!(
            ssh_destination(&argv(&["ssh", "user@host"])).as_deref(),
            Some("user@host")
        );
        // Value-taking flags are skipped with their values; the remote command
        // after the destination is never consumed.
        assert_eq!(
            ssh_destination(&argv(&[
                "ssh", "-p", "2222", "-i", "~/.ssh/id", "-o", "BatchMode=yes", "host", "uptime"
            ]))
            .as_deref(),
            Some("host")
        );
        // Bundled value (-p22) and combined boolean flags (-4A) are one token.
        assert_eq!(
            ssh_destination(&argv(&["ssh", "-4A", "-p2222", "host"])).as_deref(),
            Some("host")
        );
        // Full path argv[0] (macOS `ps -o command=` shows the resolved path).
        assert_eq!(
            ssh_destination(&argv(&["/usr/bin/ssh", "alias-from-config"])).as_deref(),
            Some("alias-from-config")
        );
    }

    #[test]
    fn ssh_destination_folds_login_flag_and_uri_authority() {
        assert_eq!(
            ssh_destination(&argv(&["ssh", "-l", "root", "web01"])).as_deref(),
            Some("root@web01")
        );
        assert_eq!(
            ssh_destination(&argv(&["ssh", "-lroot", "web01"])).as_deref(),
            Some("root@web01")
        );
        // Destination already has a user part: -l does not override it.
        assert_eq!(
            ssh_destination(&argv(&["ssh", "-l", "root", "admin@web01"])).as_deref(),
            Some("admin@web01")
        );
        assert_eq!(
            ssh_destination(&argv(&["ssh", "ssh://user@host:2222/path"])).as_deref(),
            Some("user@host:2222")
        );
        // No destination at all.
        assert_eq!(ssh_destination(&argv(&["ssh", "-p", "22"])), None);
        assert_eq!(ssh_destination(&argv(&["ssh"])), None);
    }

    #[test]
    fn process_detail_is_allowlisted_by_name() {
        // Non-allowlisted names return None without any argv lookup, so the
        // per-second sampler pays nothing for ordinary shell/agent panes.
        assert_eq!(process_detail(std::process::id() as i32, "zsh"), None);
        assert_eq!(process_detail(std::process::id() as i32, "claude"), None);
        assert!(has_destination_detail("ssh"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_resolves_current_process_from_proc() {
        let name = process_name(std::process::id() as i32)
            .expect("the current test process must exist in /proc");
        assert!(!name.trim().is_empty());
        assert!(!name.contains('/'));
    }
}
