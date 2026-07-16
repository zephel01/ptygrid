// Queen: the built-in MCP server (rmcp, streamable HTTP transport) that lets
// agent CLIs running inside PTY panes read other panes, message them, spawn
// config-defined agents, and toast the UI. Phase 2 contract.
//
// The rmcp API usage here (tool_router/tool/tool_handler macros +
// StreamableHttpService served by axum) is validated by the standalone
// `mcp-server-check` crate.

use std::sync::{Mutex, MutexGuard};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, ServerHandler,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use crate::config::ConfigManager;
use crate::queen_store::{InboxWait, InboxWaitOptions, QueenStore};
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
        let Some(p) = base.checked_add(offset) else {
            break;
        };
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

    fn store(&self) -> tauri::State<'_, QueenStore> {
        self.app.state::<QueenStore>()
    }

    fn project_dir(&self) -> Result<std::path::PathBuf, ErrorData> {
        self.config().current().map(|(_, dir)| dir).ok_or_else(|| {
            ErrorData::invalid_params(
                "no project loaded; load an mterm.yml before using pins or notes",
                None,
            )
        })
    }
}

fn ok_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text.into())])
}

fn ok_json(value: &serde_json::Value) -> Result<CallToolResult, ErrorData> {
    let text =
        serde_json::to_string(value).map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
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
        description = "if true, return untouched bytes (default false = terminal screen reconstructed from ANSI cursor/erase operations and CR overwrites)"
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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetPinRequest {
    #[schemars(description = "project-scoped pin key (max 128 bytes)")]
    pub key: String,
    #[schemars(description = "small durable value (max 16 KiB)")]
    pub value: String,
    #[schemars(
        description = "required when replacing an existing pin; must equal its latest revision"
    )]
    pub expected_revision: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeletePinRequest {
    pub key: String,
    #[schemars(description = "revision returned by list_pins or set_pin")]
    pub expected_revision: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteRequest {
    #[schemars(description = "note title (max 256 bytes)")]
    pub title: String,
    #[schemars(description = "note body (max 64 KiB)")]
    pub body: String,
    #[schemars(description = "optional tags (max 32 tags, 64 bytes each)")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListNotesRequest {
    #[schemars(description = "optional case-insensitive title/body/tag substring")]
    pub query: Option<String>,
    #[schemars(description = "maximum notes to return (default 50, max 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetNoteRequest {
    pub id: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNoteRequest {
    pub id: i64,
    #[schemars(description = "revision returned by get_note, list_notes, or create_note")]
    pub expected_revision: i64,
    pub title: Option<String>,
    pub body: Option<String>,
    #[schemars(description = "when present, replaces the complete tag list")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteNoteRequest {
    pub id: i64,
    #[schemars(description = "revision returned by get_note or list_notes")]
    pub expected_revision: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendInboxRequest {
    #[schemars(description = "stable logical sender mailbox (not a session #id)")]
    pub sender: String,
    #[schemars(description = "stable logical recipient mailbox (not a session #id)")]
    pub recipient: String,
    #[schemars(description = "message subject (max 256 bytes)")]
    pub subject: String,
    #[schemars(description = "message body (max 64 KiB)")]
    pub body: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListInboxRequest {
    #[schemars(description = "stable recipient mailbox to read")]
    pub mailbox: String,
    #[schemars(description = "return only messages with ids greater than this cursor (default 0)")]
    pub after_id: Option<i64>,
    #[schemars(description = "include acknowledged messages (default false)")]
    pub include_acknowledged: Option<bool>,
    #[schemars(description = "maximum messages to return (default 50, max 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AckInboxRequest {
    pub id: i64,
    #[schemars(description = "must exactly match the message recipient")]
    pub recipient: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReplyInboxRequest {
    pub id: i64,
    #[schemars(description = "must exactly match the original message recipient")]
    pub sender: String,
    #[schemars(description = "reply body (max 64 KiB)")]
    pub body: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AwaitInboxRequest {
    #[schemars(description = "stable recipient mailbox to wait for")]
    pub mailbox: String,
    #[schemars(description = "return only messages with ids greater than this cursor (default 0)")]
    pub after_id: Option<i64>,
    #[schemars(description = "include acknowledged messages (default false)")]
    pub include_acknowledged: Option<bool>,
    #[schemars(description = "maximum messages to return (default 50, max 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "bounded wait in milliseconds (default 30000, range 1..300000)")]
    pub timeout_ms: Option<u64>,
}

fn queen_data_error(error: String) -> ErrorData {
    if error.starts_with("Queen database error") {
        ErrorData::internal_error(error, None)
    } else {
        ErrorData::invalid_params(error, None)
    }
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
        let (text, rows, cols) = manager
            .output_snapshot(id)
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        let text = if raw.unwrap_or(false) {
            text
        } else {
            crate::ansi::render_terminal(&text, rows, cols)
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

    #[tool(description = "Show a toast notification in the ptygrid UI")]
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

    #[tool(
        description = "Create or safely update a small project-scoped pin. Creating a new key omits expectedRevision. Updating an existing key requires its latest revision; stale writes fail instead of overwriting another agent's change. Returns {pin}."
    )]
    fn set_pin(
        &self,
        Parameters(SetPinRequest {
            key,
            value,
            expected_revision,
        }): Parameters<SetPinRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let pin = self
            .store()
            .set_pin(&project, key, value, expected_revision)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "pin": pin }))
    }

    #[tool(description = "List all durable pins for the loaded project, including revisions")]
    fn list_pins(&self) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let pins = self.store().list_pins(&project).map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "pins": pins }))
    }

    #[tool(
        description = "Delete a project pin only if expectedRevision still matches. A stale delete fails without removing newer content."
    )]
    fn delete_pin(
        &self,
        Parameters(DeletePinRequest {
            key,
            expected_revision,
        }): Parameters<DeletePinRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        self.store()
            .delete_pin(&project, key.clone(), expected_revision)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "deleted": true, "key": key }))
    }

    #[tool(
        description = "Create a durable project-scoped note. Returns {note} with a stable id and revision."
    )]
    fn create_note(
        &self,
        Parameters(CreateNoteRequest { title, body, tags }): Parameters<CreateNoteRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let note = self
            .store()
            .create_note(&project, title, body, tags.unwrap_or_default())
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "note": note }))
    }

    #[tool(
        description = "List project notes newest-first, optionally filtering title, body, and tags. Returns revisions required for safe updates."
    )]
    fn list_notes(
        &self,
        Parameters(ListNotesRequest { query, limit }): Parameters<ListNotesRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let notes = self
            .store()
            .list_notes(&project, query, limit.unwrap_or(50))
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "notes": notes }))
    }

    #[tool(description = "Get one project note by stable id")]
    fn get_note(
        &self,
        Parameters(GetNoteRequest { id }): Parameters<GetNoteRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let note = self
            .store()
            .get_note(&project, id)
            .map_err(queen_data_error)?
            .ok_or_else(|| ErrorData::invalid_params(format!("note {id} not found"), None))?;
        ok_json(&serde_json::json!({ "note": note }))
    }

    #[tool(
        description = "Update selected fields of a project note only if expectedRevision matches. Stale writes fail without overwriting another agent's change."
    )]
    fn update_note(
        &self,
        Parameters(UpdateNoteRequest {
            id,
            expected_revision,
            title,
            body,
            tags,
        }): Parameters<UpdateNoteRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let note = self
            .store()
            .update_note(&project, id, expected_revision, title, body, tags)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "note": note }))
    }

    #[tool(
        description = "Delete a project note only if expectedRevision still matches. A stale delete fails without removing newer content."
    )]
    fn delete_note(
        &self,
        Parameters(DeleteNoteRequest {
            id,
            expected_revision,
        }): Parameters<DeleteNoteRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        self.store()
            .delete_note(&project, id, expected_revision)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "deleted": true, "id": id }))
    }

    #[tool(
        description = "Send a durable project-scoped inbox message between stable logical mailboxes. This does not type into a live PTY. Mailbox names must not be session #ids. Returns {message} with a stable id and thread root."
    )]
    fn send_inbox(
        &self,
        Parameters(SendInboxRequest {
            sender,
            recipient,
            subject,
            body,
        }): Parameters<SendInboxRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let message = self
            .store()
            .send_inbox(&project, sender, recipient, subject, body)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "message": message }))
    }

    #[tool(
        description = "Read durable project inbox messages for one stable mailbox in ascending id order. By default only unacknowledged messages are returned. Use afterId as a stable cursor. Returns {messages, nextCursor}."
    )]
    fn list_inbox(
        &self,
        Parameters(ListInboxRequest {
            mailbox,
            after_id,
            include_acknowledged,
            limit,
        }): Parameters<ListInboxRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let after_id = after_id.unwrap_or(0);
        let messages = self
            .store()
            .list_inbox(
                &project,
                mailbox,
                after_id,
                include_acknowledged.unwrap_or(false),
                limit.unwrap_or(50),
            )
            .map_err(queen_data_error)?;
        let next_cursor = messages
            .last()
            .map(|message| message.id)
            .unwrap_or(after_id);
        ok_json(&serde_json::json!({
            "messages": messages,
            "nextCursor": next_cursor,
        }))
    }

    #[tool(
        description = "Idempotently acknowledge a durable inbox message. recipient must exactly match the stored recipient; repeated calls return the already-acknowledged message. Returns {message}."
    )]
    fn ack_inbox(
        &self,
        Parameters(AckInboxRequest { id, recipient }): Parameters<AckInboxRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let message = self
            .store()
            .ack_inbox(&project, id, recipient)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "message": message }))
    }

    #[tool(
        description = "Create a durable correlated reply. sender must exactly match the original recipient; the reply is addressed to the original sender, inherits the thread root and subject, and atomically acknowledges the original. Returns {message}."
    )]
    fn reply_inbox(
        &self,
        Parameters(ReplyInboxRequest { id, sender, body }): Parameters<ReplyInboxRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let message = self
            .store()
            .reply_inbox(&project, id, sender, body)
            .map_err(queen_data_error)?;
        ok_json(&serde_json::json!({ "message": message }))
    }

    #[tool(
        name = "await",
        description = "Wait without busy polling for durable inbox messages after a stable id cursor. Returns immediately for existing matches, or {messages: [], nextCursor: afterId, timedOut: true} at the bounded deadline. MCP request cancellation stops the wait without acknowledging or changing messages."
    )]
    async fn await_inbox(
        &self,
        cancellation: CancellationToken,
        Parameters(AwaitInboxRequest {
            mailbox,
            after_id,
            include_acknowledged,
            limit,
            timeout_ms,
        }): Parameters<AwaitInboxRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project_dir()?;
        let after_id = after_id.unwrap_or(0);
        let timeout_ms = timeout_ms.unwrap_or(30_000);
        if !(1..=300_000).contains(&timeout_ms) {
            return Err(ErrorData::invalid_params(
                "timeoutMs must be between 1 and 300000",
                None,
            ));
        }
        let outcome = self
            .store()
            .await_inbox(
                &project,
                InboxWaitOptions {
                    mailbox,
                    after_id,
                    include_acknowledged: include_acknowledged.unwrap_or(false),
                    limit: limit.unwrap_or(50),
                    timeout: std::time::Duration::from_millis(timeout_ms),
                },
                cancellation,
            )
            .await
            .map_err(queen_data_error)?;
        match outcome {
            InboxWait::Messages(messages) => {
                let next_cursor = messages
                    .last()
                    .map(|message| message.id)
                    .unwrap_or(after_id);
                ok_json(&serde_json::json!({
                    "messages": messages,
                    "nextCursor": next_cursor,
                    "timedOut": false,
                }))
            }
            InboxWait::TimedOut => ok_json(&serde_json::json!({
                "messages": [],
                "nextCursor": after_id,
                "timedOut": true,
            })),
            InboxWait::Cancelled => Err(ErrorData::internal_error(
                "await cancelled by MCP client",
                None,
            )),
        }
    }
}

#[tool_handler]
impl ServerHandler for QueenServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Queen: the ptygrid orchestrator. Use list_agents to see \
                 sessions and definitions, read_output to inspect a pane, \
                 send_message to type into a pane, spawn_agent to start a \
                 config-defined agent, notify to toast the user, and durable \
                 pins/notes and durable inbox/reply/await to coordinate \
                 project knowledge. A user phrase such as \"grok #2\", \
                 \"codex #3\", or \"#2で作業させて\" identifies an existing \
                 ptygrid pane, not a request to launch a new CLI process. \
                 First call list_agents to verify the id, then use \
                 read_output/send_message for that exact #<id>. When multiple \
                 sessions share a name, always address the exact session as #<id>.",
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

    #[test]
    fn phase_3_8_exposes_all_eighteen_tools() {
        let mut names: Vec<_> = QueenServer::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "ack_inbox",
                "await",
                "create_note",
                "delete_note",
                "delete_pin",
                "get_note",
                "list_agents",
                "list_inbox",
                "list_notes",
                "list_pins",
                "notify",
                "read_output",
                "reply_inbox",
                "send_inbox",
                "send_message",
                "set_pin",
                "spawn_agent",
                "update_note",
            ]
        );
    }
}
