//! In-memory `PaneHost` used by unit/integration tests and the shim
//! end-to-end test. Context ids use the tmux-compatible `%<n>` form that the
//! real ptygrid host is also expected to use (shims print these ids back to
//! tmux-driving clients).

use std::collections::BTreeMap;
use std::sync::Mutex;

use tokio::sync::broadcast;

use crate::host::{ContextExited, HostError, PaneHost};
use crate::protocol::SpawnAgentParams;

#[derive(Debug, Clone)]
pub struct MockContext {
    pub params: SpawnAgentParams,
    pub written: Vec<u8>,
    pub killed: bool,
    pub focused: bool,
    pub capture_text: String,
}

#[derive(Default)]
struct State {
    next_id: u32,
    contexts: BTreeMap<String, MockContext>,
}

pub struct MockPaneHost {
    state: Mutex<State>,
    exits: broadcast::Sender<ContextExited>,
    /// When non-empty, spawn_agent rejects argv0 basenames not listed here.
    allowlist: Vec<String>,
}

impl Default for MockPaneHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPaneHost {
    pub fn new() -> Self {
        let (exits, _) = broadcast::channel(16);
        Self {
            state: Mutex::new(State {
                next_id: 1,
                contexts: BTreeMap::new(),
            }),
            exits,
            allowlist: Vec::new(),
        }
    }

    pub fn with_allowlist(binaries: &[&str]) -> Self {
        let mut host = Self::new();
        host.allowlist = binaries.iter().map(|s| s.to_string()).collect();
        host
    }

    /// Snapshot a context for assertions.
    pub fn context(&self, id: &str) -> Option<MockContext> {
        self.state.lock().unwrap().contexts.get(id).cloned()
    }

    /// Set the text `capture` returns for a context.
    pub fn set_capture_text(&self, id: &str, text: &str) {
        if let Some(ctx) = self.state.lock().unwrap().contexts.get_mut(id) {
            ctx.capture_text = text.to_string();
        }
    }

    /// Simulate a process exit and push the event to subscribers.
    pub fn emit_exit(&self, id: &str, exit_code: Option<i32>) {
        let _ = self.exits.send(ContextExited {
            context_id: id.to_string(),
            exit_code,
        });
    }
}

impl PaneHost for MockPaneHost {
    fn spawn_agent(&self, params: SpawnAgentParams) -> Result<String, HostError> {
        if params.command.is_empty() {
            return Err(HostError::SpawnDenied("empty command".into()));
        }
        if !self.allowlist.is_empty() {
            let argv0 = params.command[0]
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            if !self.allowlist.contains(&argv0) {
                return Err(HostError::SpawnDenied(format!(
                    "binary not in teammate allowlist: {argv0}"
                )));
            }
        }
        let mut state = self.state.lock().unwrap();
        let id = format!("%{}", state.next_id);
        state.next_id += 1;
        let capture_text = format!("[mock capture of {id}]");
        state.contexts.insert(
            id.clone(),
            MockContext {
                params,
                written: Vec::new(),
                killed: false,
                focused: false,
                capture_text,
            },
        );
        Ok(id)
    }

    fn write(&self, context_id: &str, data: &[u8]) -> Result<(), HostError> {
        let mut state = self.state.lock().unwrap();
        let ctx = state
            .contexts
            .get_mut(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        ctx.written.extend_from_slice(data);
        Ok(())
    }

    fn capture(&self, context_id: &str, lines: Option<u32>) -> Result<String, HostError> {
        let state = self.state.lock().unwrap();
        let ctx = state
            .contexts
            .get(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        match lines {
            Some(n) => {
                let all: Vec<&str> = ctx.capture_text.lines().collect();
                let start = all.len().saturating_sub(n as usize);
                Ok(all[start..].join("\n"))
            }
            None => Ok(ctx.capture_text.clone()),
        }
    }

    fn kill(&self, context_id: &str) -> Result<(), HostError> {
        let mut state = self.state.lock().unwrap();
        let ctx = state
            .contexts
            .get_mut(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        ctx.killed = true;
        Ok(())
    }

    fn list(&self) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut ids = vec![self.self_context_id()];
        ids.extend(
            state
                .contexts
                .iter()
                .filter(|(_, c)| !c.killed)
                .map(|(id, _)| id.clone()),
        );
        ids
    }

    fn self_context_id(&self) -> String {
        "%0".into()
    }

    fn focus(&self, context_id: &str) -> Result<(), HostError> {
        let mut state = self.state.lock().unwrap();
        let ctx = state
            .contexts
            .get_mut(context_id)
            .ok_or_else(|| HostError::ContextNotFound(context_id.into()))?;
        ctx.focused = true;
        Ok(())
    }

    fn subscribe_exits(&self) -> broadcast::Receiver<ContextExited> {
        self.exits.subscribe()
    }
}
