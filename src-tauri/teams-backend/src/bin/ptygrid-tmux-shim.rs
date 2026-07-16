//! tmux-compatibility shim binary.
//!
//! Placed on PATH (as `tmux`) ahead of any real tmux when ptygrid hosts a
//! lead agent in teams host mode. One-shot: parses a single tmux subcommand,
//! forwards it as JSON-RPC over the Unix socket at `PTYGRID_TEAMS_SOCK`
//! (authenticating with `PTYGRID_TEAMS_TOKEN` when set), prints
//! tmux-compatible output, and exits. Unknown subcommands succeed as no-ops
//! without touching the socket.

#[cfg(unix)]
fn main() {
    std::process::exit(unix_main());
}

#[cfg(not(unix))]
fn main() {
    eprintln!("ptygrid-tmux-shim: unix only");
    std::process::exit(1);
}

#[cfg(unix)]
fn unix_main() -> i32 {
    use teams_backend::shim::{execute, parse, TmuxCommand};
    use teams_backend::shim_client::SocketClient;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse(&args) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("ptygrid-tmux-shim: {e}");
            return 1;
        }
    };

    // No-ops (presence checks, option juggling) never need the socket.
    let self_id_env = std::env::var("TMUX_PANE").ok();
    if matches!(cmd, TmuxCommand::NoOp(_) | TmuxCommand::ListSessions) {
        match execute(cmd, &mut NullClient, self_id_env.as_deref()) {
            Ok(out) => {
                print!("{}", out.stdout);
                return out.exit_code;
            }
            Err(e) => {
                eprintln!("ptygrid-tmux-shim: {e}");
                return 1;
            }
        }
    }

    let sock = match std::env::var("PTYGRID_TEAMS_SOCK") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            eprintln!("ptygrid-tmux-shim: PTYGRID_TEAMS_SOCK is not set");
            return 1;
        }
    };
    let token = std::env::var("PTYGRID_TEAMS_TOKEN").ok();

    let mut client = match SocketClient::connect(&sock, token.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ptygrid-tmux-shim: {e}");
            return 1;
        }
    };

    match execute(cmd, &mut client, self_id_env.as_deref()) {
        Ok(out) => {
            print!("{}", out.stdout);
            out.exit_code
        }
        Err(e) => {
            eprintln!("ptygrid-tmux-shim: {e}");
            1
        }
    }
}

#[cfg(unix)]
struct NullClient;

#[cfg(unix)]
impl teams_backend::shim::RpcClient for NullClient {
    fn call(&mut self, method: &str, _params: serde_json::Value) -> Result<serde_json::Value, String> {
        Err(format!("unexpected RPC {method} from a no-op command"))
    }
}
