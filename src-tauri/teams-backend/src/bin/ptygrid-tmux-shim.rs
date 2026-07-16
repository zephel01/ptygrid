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

/// Blocking NDJSON JSON-RPC client: connect, `initialize`, then one request
/// per `call`. IDs are locally unique per connection.
#[cfg(unix)]
struct SocketClient {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
    next_id: u64,
}

#[cfg(unix)]
impl SocketClient {
    fn connect(path: &str, token: Option<&str>) -> Result<Self, String> {
        use teams_backend::protocol::PROTOCOL_VERSION;

        let stream = std::os::unix::net::UnixStream::connect(path)
            .map_err(|e| format!("cannot connect to {path}: {e}"))?;
        let reader_half = stream
            .try_clone()
            .map_err(|e| format!("cannot clone socket: {e}"))?;
        let mut client = Self {
            reader: std::io::BufReader::new(reader_half),
            writer: stream,
            next_id: 1,
        };
        let mut params = serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "capabilities": [],
        });
        if let Some(token) = token {
            params["auth_token"] = serde_json::json!(token);
        }
        client.request("initialize", params)?;
        Ok(client)
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use std::io::{BufRead, Write};

        let id = self.next_id.to_string();
        self.next_id += 1;
        let line = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .map_err(|e| e.to_string())?;
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .map_err(|e| format!("socket write failed: {e}"))?;

        // Read until the response matching our id arrives; push events
        // (no id / different method) may be interleaved and are skipped.
        loop {
            let mut buf = String::new();
            let n = self
                .reader
                .read_line(&mut buf)
                .map_err(|e| format!("socket read failed: {e}"))?;
            if n == 0 {
                return Err("connection closed by backend".into());
            }
            let value: serde_json::Value =
                serde_json::from_str(buf.trim_end()).map_err(|e| format!("bad response: {e}"))?;
            if value.get("id").map(|v| v == &serde_json::json!(id)) != Some(true) {
                continue;
            }
            if let Some(err) = value.get("error") {
                let msg = err["message"].as_str().unwrap_or("unknown error");
                return Err(format!("backend error: {msg}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(serde_json::Value::Null));
        }
    }
}

#[cfg(unix)]
impl teams_backend::shim::RpcClient for SocketClient {
    fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.request(method, params)
    }
}
