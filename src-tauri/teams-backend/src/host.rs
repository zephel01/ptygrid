//! `PaneHost`: the surface the socket server drives.
//!
//! The ptygrid app crate will implement this over its real session manager
//! (PTY spawn / write_pty / read_output / kill) in a later release. Tests use
//! `MockPaneHost`. The trait is synchronous on purpose: every real operation
//! is a quick in-process call, and keeping it sync avoids an async-trait
//! dependency in the hot path.

use tokio::sync::broadcast;

use crate::protocol::{SpawnAgentParams, CONTEXT_NOT_FOUND, INTERNAL_ERROR, SPAWN_DENIED};

/// Push-event payload: a context (pane process) exited.
#[derive(Debug, Clone)]
pub struct ContextExited {
    pub context_id: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostError {
    /// Unknown / already-dead context id.
    ContextNotFound(String),
    /// Spawn rejected (e.g. argv0 not in the teammate binary allowlist).
    SpawnDenied(String),
    Internal(String),
}

impl HostError {
    pub fn code(&self) -> i64 {
        match self {
            HostError::ContextNotFound(_) => CONTEXT_NOT_FOUND,
            HostError::SpawnDenied(_) => SPAWN_DENIED,
            HostError::Internal(_) => INTERNAL_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            HostError::ContextNotFound(id) => format!("context not found: {id}"),
            HostError::SpawnDenied(reason) => format!("spawn denied: {reason}"),
            HostError::Internal(reason) => format!("internal error: {reason}"),
        }
    }
}

/// Backend operations, mirroring the #26572 core set plus ptygrid extensions.
pub trait PaneHost: Send + Sync + 'static {
    /// Start a teammate process; returns the new context id. Implementations
    /// enforce the teammate binary allowlist here (`HostError::SpawnDenied`).
    fn spawn_agent(&self, params: SpawnAgentParams) -> Result<String, HostError>;

    /// Send raw bytes to the context's stdin (already base64-decoded).
    fn write(&self, context_id: &str, data: &[u8]) -> Result<(), HostError>;

    /// Read up to `lines` lines of scrollback (all available when `None`).
    fn capture(&self, context_id: &str, lines: Option<u32>) -> Result<String, HostError>;

    /// Terminate the context. Must not trigger autorestart.
    fn kill(&self, context_id: &str) -> Result<(), HostError>;

    /// Live context ids, including the self context.
    fn list(&self) -> Vec<String>;

    /// The context the connecting client itself runs in (`initialize`
    /// response `self_context_id`, and `get_self_id`).
    fn self_context_id(&self) -> String;

    /// ptygrid extension: focus/highlight a pane (tmux `select-pane`).
    fn focus(&self, context_id: &str) -> Result<(), HostError>;

    /// Subscribe to context-exit events, forwarded to initialized
    /// connections as `context_exited` notifications.
    fn subscribe_exits(&self) -> broadcast::Receiver<ContextExited>;
}
