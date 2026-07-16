//! Integration tests: NDJSON JSON-RPC over a real Unix socket against
//! `MockPaneHost`, covering the #26572 handshake, the core method set, the
//! ptygrid auth-token extension, and `context_exited` push events.

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use teams_backend::mock::MockPaneHost;
use teams_backend::server::{bind_socket, serve, ServerConfig};

static SOCKET_SEQ: AtomicU32 = AtomicU32::new(0);

fn socket_path() -> PathBuf {
    let seq = SOCKET_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ptygrid-teams-test-{}-{seq}/backend.sock",
        std::process::id()
    ))
}

struct TestServer {
    host: Arc<MockPaneHost>,
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    fn start(host: MockPaneHost, auth_token: Option<&str>) -> Self {
        let path = socket_path();
        let host = Arc::new(host);
        let listener = bind_socket(&path).expect("bind socket");
        let config = ServerConfig {
            auth_token: auth_token.map(String::from),
        };
        let serve_host = Arc::clone(&host);
        let task = tokio::spawn(async move {
            let _ = serve(listener, serve_host, config).await;
        });
        Self { host, path, task }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

struct Client {
    reader: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(server: &TestServer) -> Self {
        let stream = UnixStream::connect(&server.path).await.expect("connect");
        let (read, writer) = stream.into_split();
        Self {
            reader: BufReader::new(read).lines(),
            writer,
        }
    }

    async fn send_raw(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.write_all(b"\n").await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), self.reader.next_line())
            .await
            .expect("timed out waiting for a line")
            .expect("read line")
            .expect("connection closed");
        serde_json::from_str(&line).expect("valid json")
    }

    async fn request(&mut self, id: &str, method: &str, params: Value) -> Value {
        self.send_raw(
            &serde_json::to_string(&json!({"id": id, "method": method, "params": params})).unwrap(),
        )
        .await;
        self.recv().await
    }

    async fn initialize(&mut self, token: Option<&str>) -> Value {
        let mut params = json!({"protocol_version": "1", "capabilities": ["events"]});
        if let Some(token) = token {
            params["auth_token"] = json!(token);
        }
        self.request("init", "initialize", params).await
    }
}

#[tokio::test]
async fn handshake_then_full_method_surface() {
    let server = TestServer::start(MockPaneHost::new(), None);
    let mut client = Client::connect(&server).await;

    // initialize: version + capabilities + self context id (#26572 shape).
    let init = client.initialize(None).await;
    assert_eq!(init["result"]["protocol_version"], "1");
    assert_eq!(init["result"]["self_context_id"], "%0");
    let caps = init["result"]["capabilities"].as_array().unwrap();
    assert!(caps.contains(&json!("events")));

    // spawn_agent with the proposal's example-shaped params.
    let spawned = client
        .request(
            "2",
            "spawn_agent",
            json!({
                "command": ["claude", "--agent-id", "researcher@my-team"],
                "cwd": "/project",
                "env": {"CLAUDECODE": "1"},
                "metadata": {"name": "researcher", "color": "blue", "role": "teammate"}
            }),
        )
        .await;
    assert_eq!(spawned["result"]["context_id"], "%1");
    let ctx = server.host.context("%1").unwrap();
    assert_eq!(ctx.params.command[0], "claude");
    assert_eq!(ctx.params.cwd.as_deref(), Some("/project"));
    assert_eq!(ctx.params.metadata.role.as_deref(), Some("teammate"));

    // write: base64 payload reaches the host decoded.
    let wrote = client
        .request("3", "write", json!({"context_id": "%1", "data": "aGVsbG8="}))
        .await;
    assert!(wrote["error"].is_null());
    assert_eq!(server.host.context("%1").unwrap().written, b"hello");

    // capture returns { text }.
    server.host.set_capture_text("%1", "captured output");
    let captured = client
        .request("4", "capture", json!({"context_id": "%1", "lines": 200}))
        .await;
    assert_eq!(captured["result"]["text"], "captured output");

    // list includes self and the spawned context.
    let listed = client.request("5", "list", json!({})).await;
    assert_eq!(listed["result"]["contexts"], json!(["%0", "%1"]));

    // get_self_id mirrors initialize's self_context_id.
    let self_id = client.request("6", "get_self_id", json!({})).await;
    assert_eq!(self_id["result"]["context_id"], "%0");

    // ptygrid/focus extension.
    let focused = client
        .request("7", "ptygrid/focus", json!({"context_id": "%1"}))
        .await;
    assert!(focused["error"].is_null());
    assert!(server.host.context("%1").unwrap().focused);

    // kill.
    let killed = client.request("8", "kill", json!({"context_id": "%1"})).await;
    assert!(killed["error"].is_null());
    assert!(server.host.context("%1").unwrap().killed);
}

#[tokio::test]
async fn rejects_requests_before_initialize() {
    let server = TestServer::start(MockPaneHost::new(), None);
    let mut client = Client::connect(&server).await;
    let resp = client.request("1", "list", json!({})).await;
    assert_eq!(resp["error"]["code"], -32002);
}

#[tokio::test]
async fn rejects_wrong_or_missing_token() {
    let server = TestServer::start(MockPaneHost::new(), Some("secret"));

    let mut client = Client::connect(&server).await;
    let resp = client.initialize(Some("wrong")).await;
    assert_eq!(resp["error"]["code"], -32002);

    let mut client = Client::connect(&server).await;
    let resp = client.initialize(None).await;
    assert_eq!(resp["error"]["code"], -32002);

    let mut client = Client::connect(&server).await;
    let resp = client.initialize(Some("secret")).await;
    assert_eq!(resp["result"]["protocol_version"], "1");
}

#[tokio::test]
async fn rejects_unsupported_protocol_version() {
    let server = TestServer::start(MockPaneHost::new(), None);
    let mut client = Client::connect(&server).await;
    let resp = client
        .request("1", "initialize", json!({"protocol_version": "99"}))
        .await;
    assert_eq!(resp["error"]["code"], -32602);
}

#[tokio::test]
async fn spawn_denied_by_binary_allowlist() {
    let server = TestServer::start(MockPaneHost::with_allowlist(&["claude"]), None);
    let mut client = Client::connect(&server).await;
    client.initialize(None).await;
    let resp = client
        .request("2", "spawn_agent", json!({"command": ["rm", "-rf", "/"]}))
        .await;
    assert_eq!(resp["error"]["code"], -32001);
}

#[tokio::test]
async fn unknown_method_and_context_errors() {
    let server = TestServer::start(MockPaneHost::new(), None);
    let mut client = Client::connect(&server).await;
    client.initialize(None).await;

    let resp = client.request("2", "resize", json!({})).await;
    assert_eq!(resp["error"]["code"], -32601);

    let resp = client
        .request("3", "write", json!({"context_id": "%9", "data": "aGk="}))
        .await;
    assert_eq!(resp["error"]["code"], -32000);

    let resp = client
        .request("4", "write", json!({"context_id": "%9", "data": "@@not-base64@@"}))
        .await;
    assert_eq!(resp["error"]["code"], -32602);
}

#[tokio::test]
async fn parse_error_answers_with_null_id() {
    let server = TestServer::start(MockPaneHost::new(), None);
    let mut client = Client::connect(&server).await;
    client.send_raw("{this is not json").await;
    let resp = client.recv().await;
    assert_eq!(resp["error"]["code"], -32700);
    assert!(resp["id"].is_null());
}

#[tokio::test]
async fn context_exited_pushed_to_initialized_connections() {
    let server = TestServer::start(MockPaneHost::new(), None);
    let mut client = Client::connect(&server).await;
    client.initialize(None).await;

    server.host.emit_exit("%1", Some(0));
    let event = client.recv().await;
    assert_eq!(event["method"], "context_exited");
    assert_eq!(event["params"]["context_id"], "%1");
    assert_eq!(event["params"]["exit_code"], 0);
    assert!(event.get("id").is_none());
}
