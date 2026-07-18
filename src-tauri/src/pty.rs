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
/// their own `comm` and never reach this path. Any `python*` basename counts
/// (`python3.13`, `pythonw`, versioned venv interpreters, …).
fn is_interpreter(base: &str) -> bool {
    base.starts_with("python") || matches!(base, "node" | "nodejs" | "bun" | "deno" | "ruby")
}

/// Canonical agent names recognizable inside an interpreter's command line.
/// Ordered by specificity; first token hit (case-insensitive, non-alphanumeric
/// boundaries — see `contains_token`) wins. Tokens appear in the agent's npm /
/// pip install path or launcher name. Tokens that would false-positive on
/// unrelated command lines are either path-anchored (`sourcegraph/amp`, since
/// bare "amp" hits "example") or excluded entirely (`q`, `cn`, `cursor`,
/// `droid` — the native-binary agents behind those never reach this path
/// anyway because their own `comm` identifies them).
const KNOWN_AGENTS: &[&str] = &[
    "cursor-agent",     // Cursor CLI (bare "cursor" is too generic)
    "continuedev",      // @continuedev/cli (its bin `cn` is too short to match)
    "sourcegraph/amp",  // @sourcegraph/amp (bare "amp" hits "example", "ramp", …)
    "opencode",         // opencode-ai npm launcher
    "openhands",        // openhands-ai pip console script
    "codebuff",
    "auggie",           // @augmentcode/auggie
    "copilot",          // @github/copilot
    "claude",
    "codex",
    "gemini",           // @google/gemini-cli
    "qwen",             // @qwen-code/qwen-code
    "grok",
    "aider",
];

/// True when `token` occurs in `haystack` with non-alphanumeric characters (or
/// string edges) on both sides, so "amp" can never hit inside "example" while
/// "claude" still hits inside "Claude-Code/cli.js". Both inputs lowercase.
fn contains_token(haystack: &str, token: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(token) {
        let i = start + pos;
        let end = i + token.len();
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = i + 1;
    }
    false
}

/// Recognize a known agent from an interpreter command line, so a grok CLI that
/// runs as `node` still shows/detects as "grok". Path-anchored tokens
/// (`sourcegraph/amp`) display as their last segment.
fn agent_from_command(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    KNOWN_AGENTS
        .iter()
        .find(|a| contains_token(&lower, a))
        .map(|a| a.rsplit('/').next().unwrap_or(a).to_string())
}

/// Extensions an interpreter runs directly; stripped for display.
const SCRIPT_EXTS: &[&str] = &[".js", ".mjs", ".cjs", ".ts", ".py", ".rb"];

/// Interpreter flags that consume the FOLLOWING argv token (`node -e <code>`,
/// `python -W <filter>`), so their value is never mistaken for a launcher path.
const INTERPRETER_VALUE_FLAGS: &[&str] = &[
    "-e",
    "-p",
    "--eval",
    "--print",
    "-r",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "-c",
    "-W",
    "-X",
];

/// Path components / file stems too generic to identify a tool on their own:
/// for `…/gemini-cli/dist/index.js` the identity is "gemini-cli", not "index".
const GENERIC_COMPONENTS: &[&str] = &[
    "index", "main", "cli", "bin", "dist", "lib", "src", "build", "out", "cjs", "esm", "run",
    "start", "launcher", "entry", "__main__", ".bin",
];

/// Directory names that mark a package-manager install tree, where walking up
/// from a generic stem to the owning package directory is safe and meaningful.
const INSTALL_TREE_MARKERS: &[&str] = &["node_modules", "site-packages", "dist-packages", ".bun"];

/// Display-name charset we trust; anything else means the token was likely a
/// fragment of quoted eval code, not a real path.
fn is_clean_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '@'))
}

/// Display name for a launcher/script path. Strips a known extension; when the
/// stem is generic (`index`, `cli`, …) and the path is inside a package
/// install tree, walks up to the owning package directory instead — confined
/// to install trees so `~/bin/cli` never resolves to a home-directory name.
fn name_from_path(path: &str) -> Option<String> {
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let base = components.last()?;
    let stem = SCRIPT_EXTS
        .iter()
        .find_map(|ext| base.strip_suffix(ext))
        .unwrap_or(base);
    if !is_clean_name(stem) {
        return None;
    }
    if !GENERIC_COMPONENTS.contains(&stem) {
        return Some(stem.to_string());
    }
    if components.iter().any(|c| INSTALL_TREE_MARKERS.contains(c)) {
        for comp in components.iter().rev().skip(1) {
            if GENERIC_COMPONENTS.contains(comp) {
                continue;
            }
            if INSTALL_TREE_MARKERS.contains(comp) {
                break;
            }
            let name = comp.strip_prefix('@').unwrap_or(comp);
            if is_clean_name(name) {
                return Some(name.to_string());
            }
            break;
        }
    }
    Some(stem.to_string())
}

/// Identity of the program an interpreter is running, from its command line.
/// Handles the launcher shapes package managers actually produce:
///
/// - `node /srv/app/server.mjs`         → "server"   (script basename)
/// - `node /opt/homebrew/bin/opencode`  → "opencode" (extensionless npm bin
///   shim — the case that used to fall through and display as "node")
/// - `node /…/node_modules/@google/gemini-cli/dist/index.js`
///   → "gemini-cli" (generic stem → package)
/// - `python /venv/bin/some-tool`       → "some-tool" (pip console script)
/// - `python -m http.server`            → "http"     (module root)
///
/// A bare-word first argument (`deno run`, `bun test`) is a subcommand, not an
/// identity, and is skipped; with nothing path-like the caller falls back to
/// the interpreter name itself.
fn launcher_basename(command: &str) -> Option<String> {
    let mut toks = command.split_whitespace();
    toks.next(); // the interpreter itself
    while let Some(tok) = toks.next() {
        if tok == "-m" {
            let root = toks.next()?.split('.').next()?.to_string();
            return is_clean_name(&root).then_some(root);
        }
        if tok.starts_with('-') && tok.len() > 1 {
            if INTERPRETER_VALUE_FLAGS.contains(&tok) {
                toks.next();
            }
            continue;
        }
        // A launcher is either a path or a bare script file; a bare word here
        // is an interpreter subcommand (`deno run`, `bun test`) — skip it.
        let has_ext = SCRIPT_EXTS.iter().any(|ext| tok.ends_with(ext));
        if !tok.contains('/') && !has_ext {
            continue;
        }
        if let Some(name) = name_from_path(tok) {
            return Some(name);
        }
    }
    None
}

/// Native agent binaries sometimes embed the platform in their name
/// (opencode's TUI child is `opencode-linux-x64` / `opencode-darwin-arm64`);
/// strip a trailing `-<os>[-<arch>]` so the display name and downstream
/// name-matching stay stable. Names without such a suffix pass through
/// unchanged (`mosh-client`, `claude-next`).
fn strip_platform_suffix(name: &str) -> String {
    const OS: &[&str] = &["darwin", "linux", "windows", "win32", "macos"];
    const ARCH: &[&str] = &["x64", "arm64", "x86_64", "aarch64", "amd64", "ia32"];
    let mut parts: Vec<&str> = name.split('-').collect();
    let n = parts.len();
    if n >= 3 && ARCH.contains(parts.last().unwrap()) && OS.contains(&parts[n - 2]) {
        parts.truncate(n - 2);
    } else if n >= 2 && OS.contains(parts.last().unwrap()) {
        parts.truncate(n - 1);
    }
    parts.join("-")
}

/// Basename of a `comm` value, stripping any path and a login-shell "-" prefix
/// (`-zsh` → `zsh`).
fn comm_basename(comm: &str) -> String {
    let base = comm.rsplit('/').next().unwrap_or(comm);
    base.strip_prefix('-').unwrap_or(base).to_string()
}

/// Foreground commands with a connection destination worth surfacing next to
/// the process name (Phase 4.4.3: pane header / status sidebar show
/// `ssh user@host` instead of just `ssh`, so a command typed into the wrong
/// pane is caught before it runs). Allowlist so the per-second sampler only
/// pays the extra argv lookup for panes actually running one of these.
fn has_destination_detail(name: &str) -> bool {
    matches!(
        name,
        "ssh" | "sftp" | "scp" | "mosh" | "mosh-client" | "telnet" | "kubectl" | "docker"
    )
}

/// Dispatch to the per-command destination parser. `name` is the resolved
/// foreground process name (already allowlisted by `has_destination_detail`).
fn destination_detail(name: &str, args: &[String]) -> Option<String> {
    match name {
        // sftp takes the same option shape as ssh (value flags overlap enough;
        // its lone extra value flag -P/-R/-s are in SSH_VALUE_FLAGS or harmless).
        "ssh" | "sftp" => ssh_destination(args),
        "scp" => scp_destination(args),
        "mosh" => first_non_option(args, &["-p", "--ssh", "--server", "--predict"]),
        "mosh-client" => mosh_client_destination(args),
        "telnet" => telnet_destination(args),
        "kubectl" => subcommand_detail(
            args,
            &["exec", "attach", "logs", "port-forward", "debug"],
            // NOTE: no "-f" here — for these subcommands -f means --follow
            // (boolean), not --filename.
            &[
                "-n",
                "--namespace",
                "-c",
                "--container",
                "--kubeconfig",
                "--context",
                "--cluster",
                "--user",
            ],
            true,
        ),
        "docker" => subcommand_detail(
            args,
            &["exec", "attach", "logs"],
            &[
                "-e",
                "--env",
                "-u",
                "--user",
                "-w",
                "--workdir",
                "--env-file",
                "--detach-keys",
                "-H",
                "--host",
                "--context",
                "-l",
                "--log-level",
            ],
            false,
        ),
        _ => None,
    }
}

/// Generic "first non-option argument" scanner: `value_flags` consume one
/// following token when they appear as a separate exact token (`-p 60001`);
/// any other `-x`/`--x[=v]` token is skipped whole.
fn first_non_option(args: &[String], value_flags: &[&str]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(tok) = iter.next() {
        if tok.starts_with('-') && tok.len() > 1 {
            if value_flags.contains(&tok.as_str()) {
                iter.next();
            }
            continue;
        }
        return Some(tok.clone());
    }
    None
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

/// scp: the shown destination is the host part of the first REMOTE operand
/// (`user@host:path` / `host:path` → `user@host`). Local-to-local copies have
/// no remote operand → None. `scp -P`/-i/-o etc. share SSH_VALUE_FLAGS.
fn scp_destination(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(tok) = iter.next() {
        if tok.starts_with('-') && tok.len() > 1 {
            if SSH_VALUE_FLAGS.contains(&tok.as_str()) {
                iter.next();
            }
            continue;
        }
        // First operand containing a ':' before any '/' is remote
        // (`host:path`, `user@host:path`; a plain `./local:odd` path is rare
        // enough to ignore).
        if let Some((host, _path)) = tok.split_once(':') {
            if !host.is_empty() && !host.contains('/') {
                return Some(host.to_string());
            }
        }
    }
    None
}

/// mosh-client: the perl `mosh` wrapper execs
/// `mosh-client -# <original argv…> <server-ip> <port>` (the `-#` block is a
/// ps-visible comment carrying what the user typed). Prefer the original
/// argv's destination; fall back to the server IP (second-to-last argument).
fn mosh_client_destination(args: &[String]) -> Option<String> {
    if let Some(pos) = args.iter().position(|tok| tok == "-#") {
        // Everything after -# except the trailing ip/port pair, tolerating a
        // "|" separator some builds insert.
        let tail = &args[pos + 1..];
        let orig: Vec<&String> = tail
            .iter()
            .take(tail.len().saturating_sub(2))
            .filter(|tok| *tok != "|")
            .collect();
        if let Some(dest) = orig
            .iter()
            .find(|tok| !tok.starts_with('-') || tok.len() == 1)
        {
            return Some((*dest).clone());
        }
    }
    // No comment block: argv ends with <server-ip> <port>.
    (args.len() >= 3).then(|| args[args.len() - 2].clone())
}

/// telnet: `telnet [options] host [port]` → `host` or `host:port`.
/// `-l user` is folded like ssh's.
fn telnet_destination(args: &[String]) -> Option<String> {
    let mut login: Option<String> = None;
    let mut iter = args.iter().skip(1).peekable();
    while let Some(tok) = iter.next() {
        if let Some(rest) = tok.strip_prefix("-l") {
            if rest.is_empty() {
                login = iter.next().cloned();
            } else {
                login = Some(rest.to_string());
            }
            continue;
        }
        if tok.starts_with('-') && tok.len() > 1 {
            if matches!(tok.as_str(), "-e" | "-n" | "-b" | "-X" | "-B") {
                iter.next();
            }
            continue;
        }
        let host = tok.clone();
        let host = match login {
            Some(user) if !host.contains('@') => format!("{user}@{host}"),
            _ => host,
        };
        // A following all-digit token is the port.
        return Some(match iter.peek() {
            Some(port) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                format!("{host}:{port}")
            }
            _ => host,
        });
    }
    None
}

/// kubectl / docker style `tool [global flags] <subcommand> [flags] <target>`:
/// detail is `"<subcommand> <target>"` (kubectl: `ns/target` when a namespace
/// flag is present). Only `allowed_subs` produce a detail — `kubectl get pods`
/// and friends stay unadorned. Parsing stops at `--` (everything after is the
/// remote command).
fn subcommand_detail(
    args: &[String],
    allowed_subs: &[&str],
    value_flags: &[&str],
    namespace_aware: bool,
) -> Option<String> {
    let mut namespace: Option<String> = None;
    let mut sub: Option<String> = None;
    let mut target: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(tok) = iter.next() {
        if tok == "--" {
            break; // everything after is the remote command
        }
        if namespace_aware {
            if let Some(v) = tok.strip_prefix("--namespace=") {
                namespace = Some(v.to_string());
                continue;
            }
            if tok == "-n" || tok == "--namespace" {
                namespace = iter.next().cloned();
                continue;
            }
        }
        if tok.starts_with('-') && tok.len() > 1 {
            if value_flags.contains(&tok.as_str()) {
                iter.next();
            }
            continue;
        }
        if sub.is_none() {
            // First bare word must be an allowed subcommand, else bail
            // (e.g. `docker build .` / `kubectl get pods` → no detail).
            if !allowed_subs.contains(&tok.as_str()) {
                return None;
            }
            sub = Some(tok.clone());
            continue;
        }
        if target.is_none() {
            target = Some(tok.clone());
            // Keep scanning: a namespace flag may come AFTER the target
            // (`kubectl exec pod -n ns`); without namespace-awareness there is
            // nothing left to learn.
            if !namespace_aware {
                break;
            }
        }
    }
    let (sub, target) = (sub?, target?);
    let target = match namespace {
        Some(ns) => format!("{ns}/{target}"),
        None => target,
    };
    Some(format!("{sub} {target}"))
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
        return Some(strip_platform_suffix(&base));
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
            if let Some(launcher) = launcher_basename(command) {
                return Some(launcher);
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
        return Some(strip_platform_suffix(&base));
    }
    // Interpreter: look at the full command line for a known agent or launcher.
    if let Some(command) = ps_field(pid, "command=") {
        if let Some(agent) = agent_from_command(&command) {
            return Some(agent);
        }
        if let Some(launcher) = launcher_basename(&command) {
            return Some(launcher);
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

/// Extra display detail for a resolved foreground process — the connection
/// destination of remote-session commands (ssh/sftp/scp/mosh/telnet) and the
/// `<subcommand> <target>` of kubectl/docker exec-style commands (Phase
/// 4.4.3). `name` is the value `process_name` returned for the same pid; the
/// allowlist check runs first so non-matching processes cost nothing. Linux
/// reads /proc/<pid>/cmdline; macOS runs `ps -o command=`.
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
    destination_detail(name, &args)
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
    destination_detail(name, &args)
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
        // Versioned interpreters (venv shebangs) count too.
        assert!(is_interpreter("python3.13"));
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
        // No known agent token → None (falls back to launcher basename).
        assert_eq!(agent_from_command("node /srv/app/server.mjs"), None);
    }

    #[test]
    fn agent_from_command_recognizes_extensionless_npm_launchers() {
        // The observed opencode case: an npm bin shim with no extension, so
        // the old script-extension rule never fired and the pane showed "node".
        assert_eq!(
            agent_from_command("node /opt/homebrew/lib/node_modules/opencode-ai/bin/opencode")
                .as_deref(),
            Some("opencode")
        );
        assert_eq!(
            agent_from_command("node /usr/lib/node_modules/@github/copilot/index.js").as_deref(),
            Some("copilot")
        );
        assert_eq!(
            agent_from_command("node /x/node_modules/@qwen-code/qwen-code/dist/index.js")
                .as_deref(),
            Some("qwen")
        );
        // Path-anchored token displays as its last segment.
        assert_eq!(
            agent_from_command("node /x/node_modules/@sourcegraph/amp/dist/main.js").as_deref(),
            Some("amp")
        );
        assert_eq!(
            agent_from_command("python3 /venv/bin/openhands").as_deref(),
            Some("openhands")
        );
    }

    #[test]
    fn agent_tokens_require_word_boundaries() {
        // "amp" must not hit inside "example"; "codex" not inside "codexchange".
        assert_eq!(agent_from_command("node /srv/example/main.js"), None);
        assert_eq!(agent_from_command("node /x/codexchange/app.js"), None);
        // Boundary characters like '-' and '/' still allow a hit.
        assert!(contains_token("/opt/claude-code/cli.js", "claude"));
        assert!(!contains_token("example", "amp"));
        assert!(contains_token("@sourcegraph/amp/dist", "sourcegraph/amp"));
    }

    #[test]
    fn launcher_basename_strips_dir_and_extension() {
        assert_eq!(
            launcher_basename("node /srv/app/server.mjs --port 3").as_deref(),
            Some("server")
        );
        assert_eq!(
            launcher_basename("python3 /x/tool.py").as_deref(),
            Some("tool")
        );
        // Nothing script- or path-like → None (caller falls back to comm).
        assert_eq!(launcher_basename("node --version"), None);
        assert_eq!(launcher_basename("node -e console.log(1)"), None);
    }

    #[test]
    fn launcher_basename_resolves_extensionless_bin_shims() {
        // npm/pip bin shims have no extension — the generic fallback that
        // keeps unknown agents from displaying as "node"/"python".
        assert_eq!(
            launcher_basename("node /opt/homebrew/bin/some-newtool").as_deref(),
            Some("some-newtool")
        );
        assert_eq!(
            launcher_basename("python /venv/bin/some-tool --serve").as_deref(),
            Some("some-tool")
        );
    }

    #[test]
    fn launcher_basename_walks_up_to_package_dir_for_generic_stems() {
        // index/cli/main stems identify nothing; inside an install tree the
        // owning package directory is the identity.
        assert_eq!(
            launcher_basename("node /usr/lib/node_modules/@foo/bar-tool/dist/index.js").as_deref(),
            Some("bar-tool")
        );
        assert_eq!(
            launcher_basename("node /x/node_modules/newagent/bin/cli.js").as_deref(),
            Some("newagent")
        );
        // Outside an install tree the walk is off: never surface a
        // home-directory name for `~/bin/cli`.
        assert_eq!(
            launcher_basename("node /Users/somebody/bin/cli").as_deref(),
            Some("cli")
        );
    }

    #[test]
    fn launcher_basename_skips_subcommands_and_reads_modules() {
        // Bare words are interpreter subcommands, not identities.
        assert_eq!(
            launcher_basename("deno run /x/app.ts").as_deref(),
            Some("app")
        );
        // `python -m pkg.module` → package root.
        assert_eq!(
            launcher_basename("python -m http.server").as_deref(),
            Some("http")
        );
    }

    #[test]
    fn platform_suffixes_are_stripped_from_native_names() {
        // opencode's native TUI child, if it takes over the foreground pgrp.
        assert_eq!(strip_platform_suffix("opencode-linux-x64"), "opencode");
        assert_eq!(strip_platform_suffix("opencode-darwin-arm64"), "opencode");
        assert_eq!(strip_platform_suffix("tool-macos"), "tool");
        // Non-platform hyphenated names pass through unchanged.
        assert_eq!(strip_platform_suffix("mosh-client"), "mosh-client");
        assert_eq!(strip_platform_suffix("claude-next"), "claude-next");
        assert_eq!(strip_platform_suffix("claude"), "claude");
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
        for name in ["ssh", "sftp", "scp", "mosh", "mosh-client", "telnet", "kubectl", "docker"] {
            assert!(has_destination_detail(name), "{name} should be allowlisted");
        }
    }

    #[test]
    fn destination_detail_covers_remote_session_commands() {
        let d = |name: &str, parts: &[&str]| destination_detail(name, &argv(parts));
        // mosh wrapper: first non-option (short value flag -p skipped).
        assert_eq!(
            d("mosh", &["mosh", "-p", "60001", "user@host"]).as_deref(),
            Some("user@host")
        );
        // mosh-client: prefer the `-#` comment block (what the user typed),
        // tolerating a "|" separator; fall back to the server IP.
        assert_eq!(
            d(
                "mosh-client",
                &["mosh-client", "-#", "evox2", "|", "192.168.4.85", "60001"]
            )
            .as_deref(),
            Some("evox2")
        );
        assert_eq!(
            d("mosh-client", &["mosh-client", "-#", "user@host", "192.168.4.85", "60001"])
                .as_deref(),
            Some("user@host")
        );
        assert_eq!(
            d("mosh-client", &["mosh-client", "192.168.4.85", "60001"]).as_deref(),
            Some("192.168.4.85")
        );
        // telnet: host[:port], -l folded like ssh.
        assert_eq!(
            d("telnet", &["telnet", "10.0.0.5", "2323"]).as_deref(),
            Some("10.0.0.5:2323")
        );
        assert_eq!(
            d("telnet", &["telnet", "-l", "admin", "sw01"]).as_deref(),
            Some("admin@sw01")
        );
        // scp: host part of the first remote operand; local-to-local → None.
        assert_eq!(
            d("scp", &["scp", "-P", "2222", "a.txt", "user@host:/tmp/"]).as_deref(),
            Some("user@host")
        );
        assert_eq!(
            d("scp", &["scp", "host:backup.tgz", "."]).as_deref(),
            Some("host")
        );
        assert_eq!(d("scp", &["scp", "a.txt", "b.txt"]), None);
        // sftp shares the ssh parser.
        assert_eq!(
            d("sftp", &["sftp", "-P", "2222", "user@host"]).as_deref(),
            Some("user@host")
        );
    }

    #[test]
    fn destination_detail_covers_kubectl_and_docker_subcommands() {
        let d = |name: &str, parts: &[&str]| destination_detail(name, &argv(parts));
        assert_eq!(
            d("kubectl", &["kubectl", "exec", "-it", "mypod", "--", "sh"]).as_deref(),
            Some("exec mypod")
        );
        // Namespace before OR after the target folds in as ns/target.
        assert_eq!(
            d(
                "kubectl",
                &["kubectl", "-n", "prod", "exec", "-it", "mypod", "--", "bash"]
            )
            .as_deref(),
            Some("exec prod/mypod")
        );
        assert_eq!(
            d("kubectl", &["kubectl", "exec", "mypod", "-n", "prod", "--", "sh"]).as_deref(),
            Some("exec prod/mypod")
        );
        // -f on these subcommands is --follow (boolean), never a value flag.
        assert_eq!(
            d("kubectl", &["kubectl", "logs", "-f", "deploy/web"]).as_deref(),
            Some("logs deploy/web")
        );
        assert_eq!(
            d("kubectl", &["kubectl", "port-forward", "svc/db", "5432:5432"]).as_deref(),
            Some("port-forward svc/db")
        );
        // Non-session subcommands stay unadorned.
        assert_eq!(d("kubectl", &["kubectl", "get", "pods"]), None);
        assert_eq!(
            d(
                "docker",
                &["docker", "exec", "-it", "-e", "FOO=1", "web", "sh"]
            )
            .as_deref(),
            Some("exec web")
        );
        assert_eq!(
            d("docker", &["docker", "logs", "-f", "web"]).as_deref(),
            Some("logs web")
        );
        assert_eq!(d("docker", &["docker", "build", "."]), None);
        // `--` before any target (nothing to show).
        assert_eq!(d("kubectl", &["kubectl", "exec", "--", "sh"]), None);
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
