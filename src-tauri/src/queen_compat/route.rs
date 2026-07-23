// Phase 5.5.0: pure request/response classification for the MCP RC (2026-07-28)
// vs legacy (2025-06) dual-compat router. No I/O, no axum::extract — every
// function here takes a plain HeaderMap/Value and returns a plain value, so
// the whole routing decision tree is unit-testable without a live server
// (spec-phase5-5.md §3.1, design pin "design-5.5.0").

use axum::http::HeaderMap;
use serde_json::Value;

/// RC-route marker header (its mere presence selects the RC route).
pub const MCP_METHOD_HEADER: &str = "Mcp-Method";
/// RC-route companion header (the tool name; required alongside
/// `Mcp-Method` for `tools/call`, see `header::validate`).
pub const MCP_NAME_HEADER: &str = "Mcp-Name";
/// Legacy session marker. RC is stateless and never issues one, but per
/// spec §3.1 an incoming value is still accepted (and ignored) rather than
/// rejected, for client compatibility during the deprecation window.
pub const MCP_SESSION_ID_HEADER: &str = "Mcp-Session-Id";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    /// 2026-07-28 RC: stateless, detected by the `Mcp-Method` header.
    Rc,
    /// 2025-06 legacy: no RC marker header — session id issuance stays
    /// entirely rmcp's own business.
    Legacy,
}

/// RC iff the `Mcp-Method` header is present, regardless of its value —
/// header/body agreement is `header::validate`'s job, not detection's.
pub fn detect(headers: &HeaderMap) -> RouteKind {
    if headers.contains_key(MCP_METHOD_HEADER) {
        RouteKind::Rc
    } else {
        RouteKind::Legacy
    }
}

/// Defense-in-depth `Content-Length` check, run *before* any body read so
/// an oversized request never reaches `next` (413 without touching rmcp
/// downstream). The hard bound is still enforced by `axum::body::to_bytes`'s
/// own limit; this just short-circuits the common case where the client is
/// honest about `Content-Length`.
pub fn body_too_large(headers: &HeaderMap, max_body_bytes: usize) -> bool {
    headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|len| len > max_body_bytes)
}

/// A JSON-RPC batch is a top-level array. RC is single-request-only in
/// 5.5.0 (spec §3.1); legacy keeps whatever batching behavior rmcp already
/// has, untouched.
pub fn is_batch(body: &Value) -> bool {
    body.is_array()
}

/// The RC route never issues `Mcp-Session-Id`, but per §3.1 a client-sent
/// value is still accepted (never a 400) — just ignored. Returns it only so
/// the caller can log it once (deduped by the caller, not here).
pub fn incoming_session_id_on_rc(headers: &HeaderMap, route: RouteKind) -> Option<&str> {
    if route != RouteKind::Rc {
        return None;
    }
    headers.get(MCP_SESSION_ID_HEADER)?.to_str().ok()
}

/// Defensive: strip any `Mcp-Session-Id` an underlying rmcp response might
/// have attached before it reaches an RC client (RC must never observe one).
pub fn strip_session_id_if_rc(headers: &mut HeaderMap, route: RouteKind) {
    if route == RouteKind::Rc {
        headers.remove(MCP_SESSION_ID_HEADER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn detect_rc_when_mcp_method_header_present() {
        let h = headers(&[(MCP_METHOD_HEADER, "ping")]);
        assert_eq!(detect(&h), RouteKind::Rc);
    }

    #[test]
    fn detect_legacy_when_mcp_method_header_absent() {
        let h = headers(&[]);
        assert_eq!(detect(&h), RouteKind::Legacy);
    }

    #[test]
    fn body_too_large_boundary() {
        let at_max = headers(&[("content-length", "100")]);
        assert!(!body_too_large(&at_max, 100), "== max is not too large");
        let over_max = headers(&[("content-length", "101")]);
        assert!(body_too_large(&over_max, 100), "> max is too large");
        let missing = headers(&[]);
        assert!(
            !body_too_large(&missing, 100),
            "missing Content-Length never short-circuits here (to_bytes still enforces the bound)"
        );
    }

    #[test]
    fn is_batch_only_for_top_level_array() {
        assert!(is_batch(&serde_json::json!([{"a": 1}])));
        assert!(!is_batch(&serde_json::json!({"a": 1})));
        assert!(!is_batch(&serde_json::json!("a")));
    }

    #[test]
    fn incoming_session_id_on_rc_is_none_for_legacy_even_if_present() {
        let h = headers(&[(MCP_SESSION_ID_HEADER, "abc")]);
        assert_eq!(incoming_session_id_on_rc(&h, RouteKind::Rc), Some("abc"));
        assert_eq!(incoming_session_id_on_rc(&h, RouteKind::Legacy), None);
    }

    #[test]
    fn strip_session_id_if_rc_removes_only_on_rc() {
        let mut rc = headers(&[(MCP_SESSION_ID_HEADER, "abc")]);
        strip_session_id_if_rc(&mut rc, RouteKind::Rc);
        assert!(!rc.contains_key(MCP_SESSION_ID_HEADER));

        let mut legacy = headers(&[(MCP_SESSION_ID_HEADER, "abc")]);
        strip_session_id_if_rc(&mut legacy, RouteKind::Legacy);
        assert!(legacy.contains_key(MCP_SESSION_ID_HEADER));
    }

    #[test]
    fn header_constants_match_spec_3_1() {
        assert_eq!(MCP_METHOD_HEADER, "Mcp-Method");
        assert_eq!(MCP_NAME_HEADER, "Mcp-Name");
        assert_eq!(MCP_SESSION_ID_HEADER, "Mcp-Session-Id");
    }
}
