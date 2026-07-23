// Phase 5.5.0: RC-route `initialize` / `notifications/initialized` short-circuit
// (spec-phase5-5.md §3.1, design pin "design-5.5.0"). Pure function — no I/O,
// no axum::extract.

use serde_json::Value;

use super::route::RouteKind;

/// The only `protocolVersion` the RC route accepts in 5.5.0.
pub const SUPPORTED_PROTOCOL_VERSION: &str = "2026-07-28";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializeDecision {
    /// Not an RC `initialize` (any other method, or the Legacy route) —
    /// forward to rmcp unchanged. Covers `notifications/initialized`, which
    /// stays rmcp's business on both routes.
    Passthrough,
    /// RC `initialize` with a matching `protocolVersion` — short-circuit
    /// with a 200 no-op; rmcp never sees the request, so its
    /// `LocalSessionManager` never allocates state for a stateless route.
    NoOp200,
    /// RC `initialize` with a missing or mismatched `protocolVersion` — 400
    /// `unsupported_protocol_version`.
    UnsupportedVersion,
}

/// Decide RC-route handling for `initialize`. Legacy `initialize` is always
/// `Passthrough` (rmcp's `LocalSessionManager` issues the session id there).
pub fn decide(body: &Value, route: RouteKind) -> InitializeDecision {
    if route != RouteKind::Rc {
        return InitializeDecision::Passthrough;
    }
    if body.get("method").and_then(Value::as_str) != Some("initialize") {
        return InitializeDecision::Passthrough;
    }
    let version = body
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str);
    if version == Some(SUPPORTED_PROTOCOL_VERSION) {
        InitializeDecision::NoOp200
    } else {
        InitializeDecision::UnsupportedVersion
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rc_initialize_with_matching_version_is_a_noop() {
        let body = json!({"method": "initialize", "params": {"protocolVersion": "2026-07-28"}});
        assert_eq!(decide(&body, RouteKind::Rc), InitializeDecision::NoOp200);
    }

    #[test]
    fn rc_initialize_with_mismatched_version_is_rejected() {
        let body = json!({"method": "initialize", "params": {"protocolVersion": "2025-06-18"}});
        assert_eq!(
            decide(&body, RouteKind::Rc),
            InitializeDecision::UnsupportedVersion
        );
    }

    #[test]
    fn rc_initialize_with_missing_version_is_rejected() {
        let body = json!({"method": "initialize", "params": {}});
        assert_eq!(
            decide(&body, RouteKind::Rc),
            InitializeDecision::UnsupportedVersion
        );
    }

    #[test]
    fn rc_notifications_initialized_passes_through() {
        let body = json!({"method": "notifications/initialized"});
        assert_eq!(decide(&body, RouteKind::Rc), InitializeDecision::Passthrough);
    }

    #[test]
    fn legacy_initialize_passes_through() {
        let body = json!({"method": "initialize", "params": {"protocolVersion": "2026-07-28"}});
        assert_eq!(
            decide(&body, RouteKind::Legacy),
            InitializeDecision::Passthrough
        );
    }
}
