//! Blocking NDJSON JSON-RPC client over a Unix domain socket.
//!
//! Used by the `ptygrid-tmux-shim` binary and by the app crate's in-process
//! `__tmux-compat` entry point (cmux-style: the app re-executes itself as the
//! fake `tmux`). One connection per process: `connect` performs the
//! `initialize` handshake, then each `call` issues one request and reads until
//! the matching response, skipping interleaved push events.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use serde_json::Value;

use crate::shim::RpcClient;

/// Blocking client: connect, `initialize`, then one request per `call`. IDs are
/// locally unique per connection.
pub struct SocketClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl SocketClient {
    /// Connect to `path`, authenticating with `token` when the server requires
    /// one. Returns an error string on any connect/handshake failure.
    pub fn connect(path: &str, token: Option<&str>) -> Result<Self, String> {
        use crate::protocol::PROTOCOL_VERSION;

        let stream =
            UnixStream::connect(path).map_err(|e| format!("cannot connect to {path}: {e}"))?;
        let reader_half = stream
            .try_clone()
            .map_err(|e| format!("cannot clone socket: {e}"))?;
        let mut client = Self {
            reader: BufReader::new(reader_half),
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

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
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

        // Read until the response matching our id arrives; push events (no id /
        // different method) may be interleaved and are skipped.
        loop {
            let mut buf = String::new();
            let n = self
                .reader
                .read_line(&mut buf)
                .map_err(|e| format!("socket read failed: {e}"))?;
            if n == 0 {
                return Err("connection closed by backend".into());
            }
            let value: Value =
                serde_json::from_str(buf.trim_end()).map_err(|e| format!("bad response: {e}"))?;
            if value.get("id").map(|v| v == &serde_json::json!(id)) != Some(true) {
                continue;
            }
            if let Some(err) = value.get("error") {
                let msg = err["message"].as_str().unwrap_or("unknown error");
                return Err(format!("backend error: {msg}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl RpcClient for SocketClient {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.request(method, params)
    }
}
