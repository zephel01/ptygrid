//! End-to-end: run the real `ptygrid-tmux-shim` binary against an in-process
//! backend server, exactly as a tmux-driving client would invoke it.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use teams_backend::mock::MockPaneHost;
use teams_backend::server::{bind_socket, serve, ServerConfig};

static SOCKET_SEQ: AtomicU32 = AtomicU32::new(0);

fn socket_path() -> PathBuf {
    let seq = SOCKET_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ptygrid-shim-e2e-{}-{seq}/backend.sock",
        std::process::id()
    ))
}

struct TestServer {
    host: Arc<MockPaneHost>,
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    fn start(token: &str) -> Self {
        let path = socket_path();
        let host = Arc::new(MockPaneHost::with_allowlist(&["claude"]));
        let listener = bind_socket(&path).expect("bind socket");
        let config = ServerConfig {
            auth_token: Some(token.to_string()),
        };
        let serve_host = Arc::clone(&host);
        let task = tokio::spawn(async move {
            let _ = serve(listener, serve_host, config).await;
        });
        Self { host, path, task }
    }

    fn shim(&self, token: &str, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ptygrid-tmux-shim"))
            .args(args)
            .env("PTYGRID_TEAMS_SOCK", &self.path)
            .env("PTYGRID_TEAMS_TOKEN", token)
            .env("TMUX_PANE", "%0")
            .output()
            .expect("run shim")
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

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn teammate_pane_lifecycle_through_the_shim() {
    let server = TestServer::start("tok");

    // split-window: Claude Code's teammate spawn ([観測] shape).
    let out = server.shim(
        "tok",
        &[
            "split-window",
            "-d",
            "-h",
            "-P",
            "-F",
            "#{pane_id}",
            "claude --agent-id 'researcher@team'",
        ],
    );
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "%1\n");
    let ctx = server.host.context("%1").expect("context spawned");
    assert_eq!(
        ctx.params.command,
        vec!["claude", "--agent-id", "researcher@team"]
    );

    // send-keys: text + Enter reaches the pane as bytes.
    let out = server.shim("tok", &["send-keys", "-t", "%1", "hello", "Enter"]);
    assert!(out.status.success());
    assert_eq!(server.host.context("%1").unwrap().written, b"hello\r");

    // capture-pane -p prints the pane text.
    server.host.set_capture_text("%1", "teammate says hi");
    let out = server.shim("tok", &["capture-pane", "-p", "-t", "%1"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "teammate says hi\n");

    // select-pane maps to the focus extension.
    let out = server.shim("tok", &["select-pane", "-t", "%1"]);
    assert!(out.status.success());
    assert!(server.host.context("%1").unwrap().focused);

    // display-message answers from TMUX_PANE without RPC.
    let out = server.shim("tok", &["display-message", "-p", "#{pane_id}"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "%0\n");

    // kill-pane terminates the teammate.
    let out = server.shim("tok", &["kill-pane", "-t", "%1"]);
    assert!(out.status.success());
    assert!(server.host.context("%1").unwrap().killed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn allowlist_violation_surfaces_as_shim_failure() {
    let server = TestServer::start("tok");
    let out = server.shim("tok", &["split-window", "-P", "rm -rf /tmp/x"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("spawn denied"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_token_fails_and_noops_do_not_need_the_socket() {
    let server = TestServer::start("tok");

    // Wrong token: RPC-backed commands fail.
    let out = server.shim("bad", &["kill-pane", "-t", "%1"]);
    assert!(!out.status.success());

    // No-op commands succeed with no socket at all.
    let out = Command::new(env!("CARGO_BIN_EXE_ptygrid-tmux-shim"))
        .args(["set-option", "-g", "status", "off"])
        .env_remove("PTYGRID_TEAMS_SOCK")
        .output()
        .expect("run shim");
    assert!(out.status.success());
    assert_eq!(stdout(&out), "");

    // has-session style presence checks also stay socket-free.
    let out = Command::new(env!("CARGO_BIN_EXE_ptygrid-tmux-shim"))
        .args(["list-sessions"])
        .env_remove("PTYGRID_TEAMS_SOCK")
        .output()
        .expect("run shim");
    assert!(out.status.success());
    assert_eq!(stdout(&out), "ptygrid: 1 windows\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_socket_is_a_clear_failure_for_rpc_commands() {
    let out = Command::new(env!("CARGO_BIN_EXE_ptygrid-tmux-shim"))
        .args(["split-window", "-P", "claude"])
        .env_remove("PTYGRID_TEAMS_SOCK")
        .env_remove("PTYGRID_TEAMS_TOKEN")
        .output()
        .expect("run shim");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("PTYGRID_TEAMS_SOCK"));
}
