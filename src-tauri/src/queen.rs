// Queen: the built-in MCP server (rmcp, streamable HTTP transport) that lets
// agent CLIs running inside PTY panes read other panes, message them, spawn
// config-defined agents, and toast the UI. Phase 2 contract.
//
// The rmcp API usage here (tool_router/tool/tool_handler macros +
// StreamableHttpService served by axum) is validated by the standalone
// `mcp-server-check` crate.

use std::sync::{Mutex, MutexGuard};

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use crate::config::ConfigManager;
use crate::session::{PtyManager, SessionState};

/// Contract default port; fallback +1 each up to DEFAULT_PORT+9 (39246).
pub const DEFAULT_PORT: u16 = 39237;
const PORT_TRIES: u16 = 10;
/// PTY size for sessions spawned by Queen (frontend resizes on pane attach).
const QUEEN_SPAWN_COLS: u16 = 120;
const QUEEN_SPAWN_ROWS: u16 = 30;

// ---------- status (managed state + queen_status command payload) ----------

#[derive(Default)]
struct QueenStatusInner {
    enabled: bool,
    running: bool,
    /// Actually bound port (while running).
    port: Option<u16>,
    error: Option<String>,
    /// Port requested by config (base of the +1 fallback scan).
    desired_port: u16,
    /// Server instance counter; a task only updates status if it is still
    /// the current instance.
    epoch: u64,
    cancel: Option<CancellationToken>,
}

/// Managed Tauri state describing the Queen server.
pub struct QueenStatus {
    inner: Mutex<QueenStatusInner>,
}

impl QueenStatus {
    pub fn new() -> Self {
        QueenStatus {
            inner: Mutex::new(QueenStatusInner {
                desired_port: DEFAULT_PORT,
                ..Default::default()
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, QueenStatusInner> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn info(&self) -> QueenStatusInfo {
        let inner = self.lock();
        QueenStatusInfo {
            enabled: inner.enabled,
            running: inner.running,
            port: inner.port,
            url: inner
                .running
                .then(|| inner.port.map(url_for_port))
                .flatten(),
            error: inner.error.clone(),
        }
    }
}

/// `queen_status` return: { enabled, running, port?, url?, error? }.
#[derive(Debug, Clone, Serialize)]
pub struct QueenStatusInfo {
    pub enabled: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn url_for_port(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// QUEEN_URL value injected into every spawned session's env. Uses the bound
/// port when the server is up, otherwise the configured port (sessions may be
/// spawned in the brief window before the async bind completes).
/// Generic over the runtime so session.rs can be tested with MockRuntime.
pub fn current_env_url<R: tauri::Runtime>(app: &AppHandle<R>) -> Option<String> {
    let status = app.try_state::<QueenStatus>()?;
    let inner = status.lock();
    if !inner.enabled {
        return None;
    }
    Some(url_for_port(inner.port.unwrap_or(inner.desired_port)))
}

// ---------- lifecycle ----------

/// Start with defaults at app setup.
pub fn start_default(app: &AppHandle) {
    apply(app, true, DEFAULT_PORT);
}

/// Apply the effective queen config: enabled=false stops the server;
/// a changed port (or a stopped server) restarts it; otherwise no-op.
pub fn apply(app: &AppHandle, enabled: bool, port: u16) {
    let status = app.state::<QueenStatus>();
    let (do_start, epoch, cancel) = {
        let mut inner = status.lock();
        if !enabled {
            if let Some(ct) = inner.cancel.take() {
                ct.cancel();
            }
            inner.enabled = false;
            inner.running = false;
            inner.port = None;
            inner.error = None;
            (false, 0, CancellationToken::new())
        } else if inner.running && inner.desired_port == port {
            inner.enabled = true; // already serving on the right port
            (false, 0, CancellationToken::new())
        } else {
            if let Some(ct) = inner.cancel.take() {
                ct.cancel();
            }
            inner.enabled = true;
            inner.desired_port = port;
            inner.running = false;
            inner.port = None;
            inner.error = None;
            inner.epoch += 1;
            let ct = CancellationToken::new();
            inner.cancel = Some(ct.clone());
            (true, inner.epoch, ct)
        }
    };
    if do_start {
        run_server(app.clone(), port, epoch, cancel);
    }
}

/// Bind 127.0.0.1 only, trying `base`..`base+9`.
async fn bind_with_fallback(base: u16) -> Option<(tokio::net::TcpListener, u16)> {
    for offset in 0..PORT_TRIES {
        let Some(p) = base.checked_add(offset) else { break };
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", p)).await {
            return Some((listener, p));
        }
    }
    None
}

/// Spawn the HTTP server task on tauri::async_runtime.
fn run_server(app: AppHandle, base_port: u16, epoch: u64, cancel: CancellationToken) {
    tauri::async_runtime::spawn(async move {
        let status_update = |f: &dyn Fn(&mut QueenStatusInner)| {
            let status = app.state::<QueenStatus>();
            let mut inner = status.lock();
            if inner.epoch == epoch {
                f(&mut inner);
            }
        };

        let Some((listener, port)) = bind_with_fallback(base_port).await else {
            status_update(&|inner| {
                inner.running = false;
                inner.error = Some(format!(
                    "no free port in {}..={}",
                    base_port,
                    base_port.saturating_add(PORT_TRIES - 1)
                ));
            });
            return;
        };

        let service: StreamableHttpService<QueenServer, LocalSessionManager> =
            StreamableHttpService::new(
                {
                    let app = app.clone();
                    move || Ok(QueenServer::new(app.clone()))
                },
                Default::default(),
                StreamableHttpServerConfig::default(),
            );
        let router = axum::Router::new().nest_service("/mcp", service);

        status_update(&|inner| {
            inner.running = true;
            inner.port = Some(port);
            inner.error = None;
        });

        let result = axum::serve(listener, router)
            .with_graceful_shutdown({
                let cancel = cancel.clone();
                async move { cancel.cancelled_owned().await }
            })
            .await;

        let err_msg = result.err().map(|e| e.to_string());
        status_update(&|inner| {
            inner.running = false;
            if err_msg.is_some() {
                inner.error = err_msg.clone();
            }
        });
    });
}

// ---------- MCP server (the 5 contract tools) ----------

#[derive(Clone)]
pub struct QueenServer {
    app: AppHandle,
    // Read by #[tool_handler]-generated code; the lint can't see that.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl QueenServer {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
        }
    }

    fn manager(&self) -> tauri::State<'_, PtyManager> {
        self.app.state::<PtyManager>()
    }

    fn config(&self) -> tauri::State<'_, ConfigManager> {
        self.app.state::<ConfigManager>()
    }
}

fn ok_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text.into())])
}

fn ok_json(value: &serde_json::Value) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string(value)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(ok_text(text))
}

/// Take the last `n` lines of `text`.
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadOutputRequest {
    #[schemars(
        description = "definition name, foreground process name (e.g. codex running inside a shell pane), or \"#<id>\" for a session id"
    )]
    pub agent: String,
    #[schemars(description = "number of trailing lines to return (default 100, 1..1000)")]
    pub lines: Option<u32>,
    #[schemars(
        description = "if true, return untouched bytes (default false = ANSI stripped + CR overwrites folded)"
    )]
    pub raw: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendMessageRequest {
    #[schemars(
        description = "definition name, foreground process name, or \"#<id>\" for a session id"
    )]
    pub agent: String,
    #[schemars(description = "text to write to the agent's stdin")]
    pub text: String,
    #[schemars(description = "if true (default), append a carriage return to submit")]
    pub submit: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SpawnAgentRequest {
    #[schemars(description = "name of an agent/process defined in mterm.yml")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NotifyRequest {
    #[schemars(description = "notification title")]
    pub title: String,
    #[schemars(description = "notification body")]
    pub message: String,
}

#[tool_router]
impl QueenServer {
    #[tool(
        description = "List running terminal sessions and the agent/process definitions from mterm.yml that can be spawned"
    )]
    fn list_agents(&self) -> Result<CallToolResult, ErrorData> {
        let sessions = self.manager().list_sessions();
        let running_names: Vec<String> = sessions
            .iter()
            .filter(|s| s.state == SessionState::Running)
            .filter_map(|s| s.name.clone())
            .collect();

        let definitions: Vec<serde_json::Value> = match self.config().current() {
            Some((cfg, _dir)) => {
                let mut defs = Vec::new();
                for (list, kind) in [(&cfg.agents, "agent"), (&cfg.processes, "process")] {
                    for d in list {
                        defs.push(serde_json::json!({
                            "name": d.name,
                            "kind": kind,
                            "running": running_names.iter().any(|n| n == &d.name),
                        }));
                    }
                }
                defs
            }
            None => Vec::new(),
        };

        ok_json(&serde_json::json!({
            "sessions": sessions,
            "definitions": definitions,
        }))
    }

    #[tool(
        description = "Read the most recent terminal output of an agent (definition name, foreground process name, or \"#<id>\"). Returns JSON {agent, id, text} with the trailing `lines` lines (default 100, max 1000). By default the output is cleaned for reading: ANSI escapes are stripped and carriage-return overwrites (TUI spinners/progress bars) are folded to their final state; pass raw=true for the untouched bytes"
    )]
    fn read_output(
        &self,
        Parameters(ReadOutputRequest { agent, lines, raw }): Parameters<ReadOutputRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let manager = self.manager();
        let id = manager
            .resolve_agent(&agent)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let text = manager
            .output_text(id)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let text = if raw.unwrap_or(false) {
            text
        } else {
            crate::ansi::fold_cr(&crate::ansi::strip_ansi(&text))
        };
        let n = lines.unwrap_or(100).clamp(1, 1000) as usize;
        let text = tail_lines(&text, n);
        ok_json(&serde_json::json!({ "agent": agent, "id": id, "text": text }))
    }

    #[tool(
        description = "Write a message to an agent's terminal stdin (definition name, foreground process name, or \"#<id>\"). submit=true (default) appends a carriage return to submit it. CAUTION: interactive TUIs (Claude Code, Codex, ...) may already have unsent text sitting in their composer — sending more text would concatenate with it. Check the pane state with read_output first; to just press Enter and submit whatever is already typed, call this with text=\"\" and submit=true"
    )]
    fn send_message(
        &self,
        Parameters(SendMessageRequest {
            agent,
            text,
            submit,
        }): Parameters<SendMessageRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let manager = self.manager();
        let id = manager
            .resolve_agent(&agent)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let mut data = text;
        if submit.unwrap_or(true) {
            data.push('\r');
        }
        manager
            .write_pty(id, data)
            .map_err(|e| ErrorData::internal_error(e, None))?;
        Ok(ok_text("ok"))
    }

    #[tool(
        description = "Spawn an agent/process by its mterm.yml definition name (only config-defined names are allowed). Returns JSON {id}"
    )]
    fn spawn_agent(
        &self,
        Parameters(SpawnAgentRequest { name }): Parameters<SpawnAgentRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        // Allow-list: only names defined in the loaded mterm.yml.
        let (def, dir) = self
            .config()
            .resolve_def(&name)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let id = self
            .manager()
            .spawn_agent(
                self.app.clone(),
                &def,
                &dir,
                QUEEN_SPAWN_COLS,
                QUEEN_SPAWN_ROWS,
            )
            .map_err(|e| ErrorData::internal_error(e, None))?;
        ok_json(&serde_json::json!({ "id": id }))
    }

    #[tool(description = "Show a toast notification in the multi-terminal UI")]
    fn notify(
        &self,
        Parameters(NotifyRequest { title, message }): Parameters<NotifyRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        self.app
            .emit(
                "queen-notify",
                serde_json::json!({ "title": title, "message": message }),
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ok_text("ok"))
    }
}

#[tool_handler]
impl ServerHandler for QueenServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Queen: the multi-terminal orchestrator. Use list_agents to see \
                 sessions and definitions, read_output to inspect a pane, \
                 send_message to type into a pane, spawn_agent to start a \
                 config-defined agent, and notify to toast the user.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_lines_takes_last_n() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail_lines("a\nb", 10), "a\nb");
        assert_eq!(tail_lines("", 5), "");
        // trailing newline: last line is "c"
        assert_eq!(tail_lines("a\nb\nc\n", 2), "b\nc");
    }

    #[test]
    fn url_formatting() {
        assert_eq!(url_for_port(39237), "http://127.0.0.1:39237/mcp");
    }
}
