//! Wire types for the pane-backend socket protocol.
//!
//! The protocol is modeled on the CustomPaneBackend proposal
//! (anthropics/claude-code#26572): JSON-RPC 2.0 objects, newline-delimited
//! (NDJSON), over a Unix domain socket. Core methods: `initialize`,
//! `spawn_agent`, `write`, `capture`, `kill`, `list`, `get_self_id`, plus the
//! push event `context_exited`. ptygrid-specific extensions are namespaced
//! `ptygrid/*` and an optional `auth_token` field on `initialize`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol version implemented by this backend (issue #26572 draft).
pub const PROTOCOL_VERSION: &str = "1";

/// Capabilities advertised in the `initialize` response.
pub const CAPABILITIES: &[&str] = &["events", "capture"];

// ---- JSON-RPC error codes ----
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
// Implementation-defined (server) errors.
pub const CONTEXT_NOT_FOUND: i64 = -32000;
pub const SPAWN_DENIED: i64 = -32001;
pub const NOT_AUTHENTICATED: i64 = -32002;

/// A single incoming JSON-RPC object. `id: None` means notification.
/// The `jsonrpc` member is accepted but not required: the #26572 examples
/// omit it, so we tolerate both forms on input and always emit `"2.0"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonrpc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
}

/// A single outgoing JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(ErrorObject {
                code,
                message: message.into(),
            }),
        }
    }
}

/// An outgoing push event (JSON-RPC notification, no `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

// ---- method params / results ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// ptygrid extension: required when the server was configured with a
    /// token. Absent from the #26572 draft, ignored by tokenless servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: Vec<String>,
    pub self_context_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpawnMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnAgentParams {
    /// argv, never a shell string (#26572).
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub metadata: SpawnMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnAgentResult {
    pub context_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteParams {
    pub context_id: String,
    /// base64-encoded bytes (#26572).
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureParams {
    pub context_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureResult {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KillParams {
    pub context_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResult {
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSelfIdResult {
    pub context_id: String,
}

/// ptygrid extension (`ptygrid/focus`): highlight/focus a pane. tmux shims
/// map `select-pane` here; absent from the #26572 core operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusParams {
    pub context_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextExitedParams {
    pub context_id: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_request_without_jsonrpc_member() {
        // #26572 examples omit the jsonrpc member; both forms must parse.
        let r: Request =
            serde_json::from_str(r#"{"id":"1","method":"initialize","params":{"protocol_version":"1"}}"#)
                .unwrap();
        assert_eq!(r.id, Some(json!("1")));
        assert_eq!(r.method, "initialize");
        assert!(r.jsonrpc.is_none());
    }

    #[test]
    fn parses_request_with_numeric_id_and_jsonrpc() {
        let r: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"method":"list","params":{}}"#).unwrap();
        assert_eq!(r.id, Some(json!(7)));
    }

    #[test]
    fn missing_params_defaults_to_null() {
        let r: Request = serde_json::from_str(r#"{"id":"1","method":"list"}"#).unwrap();
        assert!(r.params.is_null());
    }

    #[test]
    fn spawn_agent_params_round_trip() {
        let v = json!({
            "command": ["claude", "--agent-id", "researcher@my-team"],
            "cwd": "/project",
            "env": {"CLAUDECODE": "1"},
            "metadata": {"name": "researcher", "color": "blue", "role": "teammate"}
        });
        let p: SpawnAgentParams = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(p.command[0], "claude");
        assert_eq!(p.metadata.name.as_deref(), Some("researcher"));
        assert_eq!(serde_json::to_value(&p).unwrap(), v);
    }

    #[test]
    fn spawn_agent_params_minimal() {
        let p: SpawnAgentParams =
            serde_json::from_value(json!({"command": ["claude"]})).unwrap();
        assert!(p.cwd.is_none());
        assert!(p.env.is_empty());
        assert_eq!(p.metadata, SpawnMetadata::default());
    }

    #[test]
    fn response_serializes_without_absent_members() {
        let ok = serde_json::to_string(&Response::ok(json!("1"), json!({}))).unwrap();
        assert!(!ok.contains("error"));
        let err = serde_json::to_string(&Response::err(json!("1"), METHOD_NOT_FOUND, "nope")).unwrap();
        assert!(!err.contains("result"));
        assert!(err.contains("-32601"));
    }
}
