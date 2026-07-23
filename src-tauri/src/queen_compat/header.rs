// Phase 5.5.0: RC-route header/body agreement (spec-phase5-5.md §3.1, design
// pin "design-5.5.0"). Pure function — no I/O, no axum::extract.

use axum::http::HeaderMap;
use serde_json::Value;

use super::route::{MCP_METHOD_HEADER, MCP_NAME_HEADER};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderValidation {
    /// Header/body agree (or the request predates any header, i.e. Legacy —
    /// callers only invoke this on the RC route).
    Ok,
    /// `Mcp-Method` is missing, or its value doesn't case-sensitively match
    /// `body.method`.
    MethodMismatch,
    /// `body.method` is `tools/*` but `Mcp-Name` is absent.
    MissingToolName,
}

/// RC-only. `body.method` missing or non-string is folded into
/// `MethodMismatch` — the body will fail JSON-RPC handling downstream
/// regardless, so surfacing the header disagreement first is the more
/// actionable diagnostic.
pub fn validate(headers: &HeaderMap, body: &Value) -> HeaderValidation {
    let header_method = headers.get(MCP_METHOD_HEADER).and_then(|v| v.to_str().ok());
    let body_method = body.get("method").and_then(Value::as_str);
    let method = match (header_method, body_method) {
        (Some(h), Some(b)) if h == b => b,
        _ => return HeaderValidation::MethodMismatch,
    };
    if method.starts_with("tools/") && headers.get(MCP_NAME_HEADER).is_none() {
        return HeaderValidation::MissingToolName;
    }
    HeaderValidation::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::try_from(*k).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn header_and_body_method_mismatch_rejected() {
        let h = headers(&[(MCP_METHOD_HEADER, "ping")]);
        let body = json!({"method": "initialize"});
        assert_eq!(validate(&h, &body), HeaderValidation::MethodMismatch);
    }

    #[test]
    fn header_match_is_case_sensitive() {
        let h = headers(&[(MCP_METHOD_HEADER, "Initialize")]);
        let body = json!({"method": "initialize"});
        assert_eq!(validate(&h, &body), HeaderValidation::MethodMismatch);
    }

    #[test]
    fn missing_mcp_method_header_is_a_mismatch() {
        let h = headers(&[]);
        let body = json!({"method": "initialize"});
        assert_eq!(validate(&h, &body), HeaderValidation::MethodMismatch);
    }

    #[test]
    fn tools_call_requires_mcp_name() {
        let h = headers(&[(MCP_METHOD_HEADER, "tools/call")]);
        let body = json!({"method": "tools/call"});
        assert_eq!(validate(&h, &body), HeaderValidation::MissingToolName);
    }

    #[test]
    fn tools_call_with_mcp_name_is_ok() {
        let h = headers(&[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")]);
        let body = json!({"method": "tools/call"});
        assert_eq!(validate(&h, &body), HeaderValidation::Ok);
    }

    #[test]
    fn ping_and_initialize_do_not_require_mcp_name() {
        let h = headers(&[(MCP_METHOD_HEADER, "ping")]);
        let body = json!({"method": "ping"});
        assert_eq!(validate(&h, &body), HeaderValidation::Ok);

        let h = headers(&[(MCP_METHOD_HEADER, "initialize")]);
        let body = json!({"method": "initialize"});
        assert_eq!(validate(&h, &body), HeaderValidation::Ok);
    }
}
