//! NDJSON JSON-RPC server over a Unix domain socket.
//!
//! Connection lifecycle: every connection must complete `initialize` before
//! any other request (the #26572 handshake; also where the optional ptygrid
//! auth token is checked). After a successful `initialize`, `context_exited`
//! push events are forwarded to the connection. Both persistent clients
//! (a future official CustomPaneBackend client) and one-shot clients (the
//! tmux shim, one connection per tmux subcommand) are supported.

#![cfg(unix)]

use std::io;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::host::PaneHost;
use crate::protocol::{
    CaptureParams, CaptureResult, FocusParams, GetSelfIdResult, InitializeParams,
    InitializeResult, KillParams, ListResult, Notification, Request, Response,
    SpawnAgentParams, SpawnAgentResult, WriteParams, CAPABILITIES, INVALID_PARAMS,
    INVALID_REQUEST, METHOD_NOT_FOUND, NOT_AUTHENTICATED, PARSE_ERROR, PROTOCOL_VERSION,
};

#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    /// When set, `initialize` must carry a matching `auth_token`; a mismatch
    /// is answered with `NOT_AUTHENTICATED` and the connection is closed.
    pub auth_token: Option<String>,
}

/// Create the socket's parent directory (0700), remove a stale socket file,
/// bind, and restrict the socket itself to 0600.
pub fn bind_socket(path: &Path) -> io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Accept loop. Runs until the listener errors or the future is dropped
/// (the embedding app aborts the task on shutdown).
pub async fn serve(
    listener: UnixListener,
    host: Arc<dyn PaneHost>,
    config: ServerConfig,
) -> io::Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let host = Arc::clone(&host);
        let config = config.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, host, config).await;
        });
    }
}

async fn handle_connection(
    stream: UnixStream,
    host: Arc<dyn PaneHost>,
    config: ServerConfig,
) -> io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Single writer task; request responses and push events share it.
    let (tx, mut rx) = mpsc::channel::<String>(64);
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if write_half.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            if write_half.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = write_half.flush().await;
        }
        let _ = write_half.shutdown().await;
    });

    let mut initialized = false;
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send(&tx, &Response::err(Value::Null, PARSE_ERROR, format!("parse error: {e}")))
                    .await;
                continue;
            }
        };
        let Some(id) = request.id.clone() else {
            // Client-to-server notifications are ignored in v1.
            continue;
        };

        if !initialized {
            if request.method != "initialize" {
                send(
                    &tx,
                    &Response::err(id, NOT_AUTHENTICATED, "initialize required before other requests"),
                )
                .await;
                continue;
            }
            match check_initialize(&request.params, &config) {
                Ok(()) => {
                    initialized = true;
                    let result = InitializeResult {
                        protocol_version: PROTOCOL_VERSION.into(),
                        capabilities: CAPABILITIES.iter().map(|s| s.to_string()).collect(),
                        self_context_id: host.self_context_id(),
                    };
                    send(&tx, &Response::ok(id, json!(result))).await;
                    // Forward context_exited push events from now on.
                    let mut exits = host.subscribe_exits();
                    let tx_events = tx.clone();
                    forwarder = Some(tokio::spawn(async move {
                        loop {
                            match exits.recv().await {
                                Ok(exit) => {
                                    let note = Notification::new(
                                        "context_exited",
                                        json!({
                                            "context_id": exit.context_id,
                                            "exit_code": exit.exit_code,
                                        }),
                                    );
                                    let Ok(line) = serde_json::to_string(&note) else {
                                        continue;
                                    };
                                    if tx_events.send(line).await.is_err() {
                                        break;
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    }));
                }
                Err(response_err) => {
                    send(&tx, &Response { id, ..response_err }).await;
                    break; // auth/handshake failure: close the connection
                }
            }
            continue;
        }

        let response = dispatch(&request.method, request.params, id, host.as_ref());
        send(&tx, &response).await;
    }

    if let Some(f) = forwarder {
        f.abort();
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}

/// Validate `initialize` params. Returns a template error `Response`
/// (id filled in by the caller) on failure.
fn check_initialize(params: &Value, config: &ServerConfig) -> Result<(), Response> {
    let params: InitializeParams = serde_json::from_value(params.clone()).map_err(|e| {
        Response::err(Value::Null, INVALID_PARAMS, format!("invalid initialize params: {e}"))
    })?;
    if params.protocol_version != PROTOCOL_VERSION {
        return Err(Response::err(
            Value::Null,
            INVALID_PARAMS,
            format!(
                "unsupported protocol_version {:?} (server implements {PROTOCOL_VERSION:?})",
                params.protocol_version
            ),
        ));
    }
    if let Some(expected) = &config.auth_token {
        if params.auth_token.as_deref() != Some(expected.as_str()) {
            return Err(Response::err(
                Value::Null,
                NOT_AUTHENTICATED,
                "missing or invalid auth_token",
            ));
        }
    }
    Ok(())
}

fn dispatch(method: &str, params: Value, id: Value, host: &dyn PaneHost) -> Response {
    match method {
        "initialize" => Response::err(id, INVALID_REQUEST, "connection already initialized"),
        "spawn_agent" => match parse::<SpawnAgentParams>(params) {
            Ok(p) => match host.spawn_agent(p) {
                Ok(context_id) => Response::ok(id, json!(SpawnAgentResult { context_id })),
                Err(e) => Response::err(id, e.code(), e.message()),
            },
            Err(e) => Response::err(id, INVALID_PARAMS, e),
        },
        "write" => match parse::<WriteParams>(params) {
            Ok(p) => match base64::engine::general_purpose::STANDARD.decode(&p.data) {
                Ok(bytes) => match host.write(&p.context_id, &bytes) {
                    Ok(()) => Response::ok(id, json!({})),
                    Err(e) => Response::err(id, e.code(), e.message()),
                },
                Err(e) => Response::err(id, INVALID_PARAMS, format!("data is not valid base64: {e}")),
            },
            Err(e) => Response::err(id, INVALID_PARAMS, e),
        },
        "capture" => match parse::<CaptureParams>(params) {
            Ok(p) => match host.capture(&p.context_id, p.lines) {
                Ok(text) => Response::ok(id, json!(CaptureResult { text })),
                Err(e) => Response::err(id, e.code(), e.message()),
            },
            Err(e) => Response::err(id, INVALID_PARAMS, e),
        },
        "kill" => match parse::<KillParams>(params) {
            Ok(p) => match host.kill(&p.context_id) {
                Ok(()) => Response::ok(id, json!({})),
                Err(e) => Response::err(id, e.code(), e.message()),
            },
            Err(e) => Response::err(id, INVALID_PARAMS, e),
        },
        "list" => Response::ok(id, json!(ListResult { contexts: host.list() })),
        "get_self_id" => Response::ok(
            id,
            json!(GetSelfIdResult {
                context_id: host.self_context_id()
            }),
        ),
        "ptygrid/focus" => match parse::<FocusParams>(params) {
            Ok(p) => match host.focus(&p.context_id) {
                Ok(()) => Response::ok(id, json!({})),
                Err(e) => Response::err(id, e.code(), e.message()),
            },
            Err(e) => Response::err(id, INVALID_PARAMS, e),
        },
        other => Response::err(id, METHOD_NOT_FOUND, format!("method not found: {other}")),
    }
}

fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

async fn send(tx: &mpsc::Sender<String>, response: &Response) {
    if let Ok(line) = serde_json::to_string(response) {
        let _ = tx.send(line).await;
    }
}
