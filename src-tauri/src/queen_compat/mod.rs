// Phase 5.5.0: MCP 2026-07-28 RC-compat router (design pin "design-5.5.0",
// spec-phase5-5.md §3.1). `middleware` is the only I/O-bearing function in
// this module tree — every decision it makes is delegated to a pure function
// in one of the submodules below, so the actual policy stays unit-testable
// without a live server (see each submodule's own tests). Wire-level behavior
// (headers/body over real HTTP) is covered by
// `src-tauri/tests/queen_compat_integration.rs`.

pub mod capabilities;
pub mod config;
pub mod deprecation;
pub mod header;
pub mod initialize;
pub mod meta;
pub mod route;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{CONTENT_LENGTH, CONTENT_TYPE},
        StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::Value;

pub use config::{McpCompatConfig, McpCompatHandle};
use route::RouteKind;

/// Cap for buffering a *response* body (RC-route traceparent echo). This is
/// our own server's outbound data, not attacker-controlled request input —
/// unlike `cfg.max_body_bytes` (a security bound on untrusted request
/// bodies), this only guards against holding a pathologically large tool
/// result fully in memory while rewriting `_meta`.
const RESPONSE_BUFFER_LIMIT: usize = 16 * 1024 * 1024;

/// The `/mcp` compat layer: classifies RC (2026-07-28) vs Legacy (2025-06)
/// traffic, enforces the two transports' respective request-shape rules, and
/// applies the response-side Deprecation trio / traceparent echo / session-id
/// hygiene. Sits between the `mcp_auth` layer (outermost) and the rmcp
/// `StreamableHttpService` (innermost) — see `queen.rs::run_server`.
pub async fn middleware(State(handle): State<McpCompatHandle>, req: Request, next: Next) -> Response {
    let cfg = handle.get();
    let route = route::detect(req.headers());

    // Defense-in-depth Content-Length check, before any body read (a
    // dishonest Content-Length is still bounded by `to_bytes`'s own limit
    // below).
    if route::body_too_large(req.headers(), cfg.max_body_bytes) {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    if let Some(resp) = transport_gate(route, &cfg) {
        return resp;
    }

    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, cfg.max_body_bytes).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };

    // Body isn't valid JSON at all: pass through unchanged and let rmcp
    // produce the JSON-RPC parse error downstream (it owns that diagnostic).
    let Ok(body_json) = serde_json::from_slice::<Value>(&bytes) else {
        let req = Request::from_parts(parts, Body::from(bytes));
        return next.run(req).await;
    };
    let id = body_json.get("id").cloned().unwrap_or(Value::Null);

    if route == RouteKind::Rc {
        if route::is_batch(&body_json) {
            return jsonrpc_error(StatusCode::BAD_REQUEST, id, -32600, "batch_not_supported");
        }
        if header::validate(&parts.headers, &body_json) != header::HeaderValidation::Ok {
            return jsonrpc_error(StatusCode::BAD_REQUEST, id, -32600, "header_body_mismatch");
        }
    }

    let method = body_json.get("method").and_then(Value::as_str).unwrap_or("");

    match capabilities::classify(method, cfg.legacy_capabilities) {
        capabilities::Classification::NoOp200 => {
            let mut resp = jsonrpc_success(id, serde_json::json!({}));
            deprecation::attach(resp.headers_mut());
            return resp;
        }
        capabilities::Classification::MethodNotFound => {
            let mut resp = jsonrpc_error(StatusCode::OK, id, -32601, "Method not found");
            deprecation::attach(resp.headers_mut());
            return resp;
        }
        capabilities::Classification::Passthrough => {}
    }

    match initialize::decide(&body_json, route) {
        // TODO(track-b 5.5.1): a real capabilities/serverInfo negotiation
        // payload; 5.5.0 only stubs the RC no-op ack (spec-phase5-5.md §3.1
        // scopes 5.5.0 to accept+short-circuit, not full negotiation).
        initialize::InitializeDecision::NoOp200 => return jsonrpc_success(id, serde_json::json!({})),
        initialize::InitializeDecision::UnsupportedVersion => {
            return jsonrpc_error(StatusCode::BAD_REQUEST, id, -32600, "unsupported_protocol_version");
        }
        initialize::InitializeDecision::Passthrough => {}
    }

    let traceparent =
        (route == RouteKind::Rc).then(|| meta::resolve_traceparent(&parts.headers, &body_json));

    let mut req = Request::from_parts(parts, Body::from(bytes));
    if let Some(tp) = &traceparent {
        req.extensions_mut().insert(meta::Traceparent(tp.clone()));
    }

    let resp = next.run(req).await;
    let mut resp = match &traceparent {
        Some(tp) => echo_traceparent_if_json(resp, tp).await,
        None => resp,
    };

    match route {
        RouteKind::Rc => route::strip_session_id_if_rc(resp.headers_mut(), route),
        RouteKind::Legacy => {
            deprecation::attach(resp.headers_mut());
            deprecation::log_deprecated_route();
        }
    }
    resp
}

/// Pure transport on/off decision (step 4 of the middleware flow) — split out
/// from `middleware` so it's unit-testable without a live server/`Next`. Body
/// unread at this point (`transport_disabled` must fire even for an unparsed
/// body), so the JSON-RPC `id` is always `Value::Null`.
fn transport_gate(route: RouteKind, cfg: &McpCompatConfig) -> Option<Response> {
    let disabled = match route {
        RouteKind::Rc => !cfg.rc_2026_07_28,
        RouteKind::Legacy => !cfg.legacy_2025_06,
    };
    disabled.then(|| jsonrpc_error(StatusCode::BAD_REQUEST, Value::Null, -32600, "transport_disabled"))
}

fn jsonrpc_success(id: Value, result: Value) -> Response {
    json_response(
        StatusCode::OK,
        serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
}

fn jsonrpc_error(status: StatusCode, id: Value, code: i64, message: &str) -> Response {
    json_response(
        status,
        serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_default(),
    )
        .into_response()
}

/// RC-route response post-processing (step 11): embed `traceparent` into a
/// successful `application/json` response's `result._meta.traceparent`. A
/// no-op for any other content type (notably `text/event-stream`, left
/// unbuffered) — `meta::echo_into_result` itself already no-ops on an
/// `error` response, so an RC error never gains a `_meta.traceparent`.
async fn echo_traceparent_if_json(resp: Response, traceparent: &str) -> Response {
    let is_json = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return resp;
    }
    let (mut parts, body) = resp.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, RESPONSE_BUFFER_LIMIT).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    meta::echo_into_result(&mut value, traceparent);
    let Ok(new_bytes) = serde_json::to_vec(&value) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    parts.headers.remove(CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(new_bytes))
}

/// Push the resolved `mcp:` block into the already-bound `/mcp` compat
/// middleware's hot-swappable handle. Called once at startup (overwriting
/// `QueenStatus::new`'s built-in-default `McpCompatConfig`) and again on
/// every config reload (`commands::load_config`) — an open TCP connection is
/// unaffected, since `McpCompatHandle::get` reads fresh per request.
pub fn apply(app: &tauri::AppHandle, cfg: &crate::config::Config) {
    use tauri::Manager;
    let handle = app.state::<crate::queen::QueenStatus>().mcp_handle();
    handle.set(McpCompatConfig::from(&cfg.mcp.unwrap_or_default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_gate_blocks_rc_when_rc_disabled() {
        let cfg = McpCompatConfig {
            rc_2026_07_28: false,
            ..McpCompatConfig::default()
        };
        let resp = transport_gate(RouteKind::Rc, &cfg).expect("RC must be gated when disabled");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        // Legacy is unaffected by the RC flag.
        assert!(transport_gate(RouteKind::Legacy, &cfg).is_none());
    }

    #[test]
    fn transport_gate_blocks_legacy_when_legacy_disabled() {
        let cfg = McpCompatConfig {
            legacy_2025_06: false,
            ..McpCompatConfig::default()
        };
        let resp = transport_gate(RouteKind::Legacy, &cfg).expect("Legacy must be gated when disabled");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        // RC is unaffected by the Legacy flag.
        assert!(transport_gate(RouteKind::Rc, &cfg).is_none());
    }
}
