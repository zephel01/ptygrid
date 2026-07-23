// Phase 5.5.0: RC-route JSON-RPC `_meta.traceparent` resolve/echo (design pin
// "design-5.5.0"). Pure function — no I/O. Real OTel span propagation lands
// in 5.5.1+; 5.5.0 only accepts and reflects a W3C `traceparent` string (or
// stubs one when the client sent neither). `tracestate` / `baggage` are out
// of scope for 5.5.0 and pass through untouched — this module never reads
// or writes them.

use axum::http::HeaderMap;
use serde_json::Value;

/// HTTP `traceparent` header name (fallback source, below `_meta.traceparent`).
pub const TRACEPARENT_HEADER: &str = "traceparent";

/// Attached to the compat middleware's forwarded [`axum::extract::Request`]
/// extensions so a downstream handler (5.5.1+ OTel wiring) can read the
/// resolved traceparent without re-deriving it. Unused by 5.5.0 itself
/// beyond the response echo, which keeps its own local copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Traceparent(pub String);

/// Resolve the RC-route traceparent: `params._meta.traceparent` (JSON-RPC
/// level) wins over the HTTP `traceparent` header, which wins over a stubbed
/// dummy when neither is present.
pub fn resolve_traceparent(headers: &HeaderMap, body: &Value) -> String {
    if let Some(v) = body
        .pointer("/params/_meta/traceparent")
        .and_then(Value::as_str)
    {
        return v.to_string();
    }
    if let Some(v) = headers
        .get(TRACEPARENT_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        return v.to_string();
    }
    generate_stub()
}

/// A 55-byte W3C `traceparent` dummy (`version-trace_id-parent_id-flags`,
/// all-zero ids: `2 + 1 + 32 + 1 + 16 + 1 + 2 == 55`). 5.5.0 has no real span
/// to reflect; 5.5.1 replaces the parent-id segment with the server's own
/// span id (TODO(track-b 5.5.1)).
pub fn generate_stub() -> String {
    "00-00000000000000000000000000000000-0000000000000000-00".to_string()
}

/// Embed `traceparent` into a *successful* JSON-RPC response's
/// `result._meta.traceparent`. A no-op when the response carries an `error`
/// instead of a `result`, or when `result` isn't a JSON object (nowhere to
/// hang `_meta` off of) — error responses never gain a `_meta.traceparent`.
pub fn echo_into_result(response: &mut Value, traceparent: &str) {
    if response.get("error").is_some() {
        return;
    }
    let Some(result) = response.get_mut("result") else {
        return;
    };
    let Some(result_obj) = result.as_object_mut() else {
        return;
    };
    let meta = result_obj
        .entry("_meta")
        .or_insert_with(|| Value::Object(Default::default()));
    if let Some(meta_obj) = meta.as_object_mut() {
        meta_obj.insert(
            "traceparent".to_string(),
            Value::String(traceparent.to_string()),
        );
    }
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
    fn meta_traceparent_wins_over_header_and_stub() {
        let h = headers(&[(TRACEPARENT_HEADER, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")]);
        let body = json!({
            "method": "tools/call",
            "params": {"_meta": {"traceparent": "00-11111111111111111111111111111111-2222222222222222-01"}}
        });
        assert_eq!(
            resolve_traceparent(&h, &body),
            "00-11111111111111111111111111111111-2222222222222222-01"
        );
    }

    #[test]
    fn header_wins_over_stub_when_meta_absent() {
        let h = headers(&[(TRACEPARENT_HEADER, "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")]);
        let body = json!({"method": "tools/call", "params": {}});
        assert_eq!(
            resolve_traceparent(&h, &body),
            "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"
        );
    }

    #[test]
    fn stub_used_when_neither_meta_nor_header_present() {
        let h = headers(&[]);
        let body = json!({"method": "ping"});
        let stub = resolve_traceparent(&h, &body);
        assert_eq!(stub, generate_stub());
        assert_eq!(stub.len(), 55);
    }

    #[test]
    fn tracestate_and_baggage_headers_are_ignored() {
        // Presence of tracestate/baggage must not change traceparent
        // resolution — 5.5.0 passes them through untouched (out of scope).
        let with_extra = headers(&[
            (TRACEPARENT_HEADER, "00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01"),
            ("tracestate", "vendor1=opaque"),
            ("baggage", "userId=alice"),
        ]);
        let without_extra = headers(&[(
            TRACEPARENT_HEADER,
            "00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01",
        )]);
        let body = json!({"method": "ping"});
        assert_eq!(
            resolve_traceparent(&with_extra, &body),
            resolve_traceparent(&without_extra, &body)
        );
    }

    #[test]
    fn echo_into_result_only_touches_success_not_error() {
        let mut success = json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}});
        echo_into_result(&mut success, "00-x-y-01");
        assert_eq!(success["result"]["_meta"]["traceparent"], "00-x-y-01");

        let mut error = json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32601, "message": "nope"}});
        echo_into_result(&mut error, "00-x-y-01");
        assert!(error.get("_meta").is_none());
        assert!(error["error"].get("_meta").is_none());
    }
}
