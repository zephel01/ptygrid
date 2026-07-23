// Phase 5.5.0: wire-level integration coverage for the `/mcp` RC/Legacy
// compat middleware (design pin "design-5.5.0", spec-phase5-5.md §3.1). Each
// submodule under `queen_compat` already unit-tests its own pure decision
// function in isolation (see `queen_compat/mod.rs`'s own doc comment); this
// file instead drives `queen_compat::middleware` as a real axum `tower::Layer`
// in front of a small fake downstream handler standing in for rmcp's
// `StreamableHttpService`, so header/body/response-shape behavior is
// exercised the way a live HTTP client would see it, without starting a real
// rmcp server (out of scope for 5.5.0 per the design pin — `QueenServer`
// needs a live `tauri::AppHandle<Wry>`, not a mockable one; `queen.rs`'s own
// `/mcp`-auth test bypasses rmcp the same way for the same reason).
//
// `queen.rs`'s 18 MCP tools are not touched by this file at all. The
// production layer *order* (`mcp_auth` outermost, this middleware innermost,
// `/hooks/v1/*` merged after both — see `queen.rs::run_server`) is also out
// of scope here: `queen` is a private (`mod queen;`, not `pub mod`) module,
// so an external integration test binary structurally cannot reach
// `mcp_auth`/`McpAuthState` at all. That production-wiring concern is
// covered by the design pin's own "5.5.0 completion gate" manual `curl`
// step instead.

use axum::{
    body::{Body, Bytes},
    extract::Request,
    http::{
        header::{CONTENT_LENGTH, CONTENT_TYPE},
        StatusCode,
    },
    middleware::from_fn_with_state,
    response::Response,
    routing::post,
    Router,
};
use serde_json::{json, Value};
use tower::ServiceExt;

use ptygrid_lib::queen_compat::{
    meta::generate_stub,
    middleware,
    route::{MCP_METHOD_HEADER, MCP_NAME_HEADER, MCP_SESSION_ID_HEADER},
    McpCompatConfig, McpCompatHandle,
};

/// Stands in for rmcp's `StreamableHttpService`: echoes the request `id`
/// back in a JSON-RPC success (or, for `method == "force_error"`, an error),
/// and — like a real `LocalSessionManager` — always attaches
/// `Mcp-Session-Id` on its own response regardless of route. Proving the RC
/// route strips it anyway (rather than relying on downstream cooperation)
/// is the point of the session-id tests below.
async fn fake_downstream(body: Bytes) -> Response {
    let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    let payload = if method == "force_error" {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})
    } else {
        json!({"jsonrpc": "2.0", "id": id, "result": {"ok": true}})
    };
    let bytes = serde_json::to_vec(&payload).unwrap();
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(MCP_SESSION_ID_HEADER, "rmcp-issued-session-abc123")
        .body(Body::from(bytes))
        .unwrap()
}

/// Stands in for an SSE-streaming rmcp response — used only to prove the
/// compat middleware leaves a non-JSON content type completely alone (no
/// attempted `_meta` buffering/rewrite, which would corrupt a real stream).
async fn fake_sse_downstream() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .body(Body::from("data: hello\n\n"))
        .unwrap()
}

fn router_with(cfg: McpCompatConfig) -> (Router, McpCompatHandle) {
    let handle = McpCompatHandle::new(cfg);
    let router = Router::new()
        .route("/mcp", post(fake_downstream))
        .layer(from_fn_with_state(handle.clone(), middleware));
    (router, handle)
}

fn sse_router(cfg: McpCompatConfig) -> Router {
    let handle = McpCompatHandle::new(cfg);
    Router::new()
        .route("/mcp", post(fake_sse_downstream))
        .layer(from_fn_with_state(handle, middleware))
}

fn post_request(headers: &[(&str, &str)], body: Value) -> Request {
    let mut builder = Request::builder().method("POST").uri("/mcp");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn body_json(resp: Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("response body readable");
    serde_json::from_slice(&bytes).expect("response body is JSON")
}

#[tokio::test]
async fn legacy_request_keeps_session_id_and_gets_deprecation_trio() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(MCP_SESSION_ID_HEADER).unwrap(),
        "rmcp-issued-session-abc123"
    );
    assert!(resp.headers().get("deprecation").is_some());
    assert!(resp.headers().get("sunset").is_some());
    assert!(resp.headers().get("link").is_some());
}

#[tokio::test]
async fn rc_request_strips_session_id_echoes_traceparent_no_deprecation() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "list_agents"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get(MCP_SESSION_ID_HEADER).is_none());
    assert!(resp.headers().get("deprecation").is_none());
    let body = body_json(resp).await;
    assert_eq!(body["id"], 7);
    assert_eq!(body["result"]["_meta"]["traceparent"], generate_stub());
}

#[tokio::test]
async fn interleaved_legacy_and_rc_requests_never_cross_contaminate_session_id() {
    // Not a literal TCP-keep-alive test — the compat middleware has no
    // per-connection state to begin with (only a per-request ArcSwap read),
    // so the meaningful guarantee is that consecutive calls through the SAME
    // Router/handle never leak one request's route classification into the
    // next. Exercising Legacy -> RC -> Legacy back to back proves that.
    let (router, _handle) = router_with(McpCompatConfig::default());
    let legacy_req = || {
        post_request(
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
        )
    };
    let rc_req = || {
        post_request(
            &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "list_agents"}}),
        )
    };

    let r1 = router.clone().oneshot(legacy_req()).await.unwrap();
    assert!(r1.headers().get(MCP_SESSION_ID_HEADER).is_some());

    let r2 = router.clone().oneshot(rc_req()).await.unwrap();
    assert!(r2.headers().get(MCP_SESSION_ID_HEADER).is_none());

    let r3 = router.clone().oneshot(legacy_req()).await.unwrap();
    assert!(r3.headers().get(MCP_SESSION_ID_HEADER).is_some());
}

#[tokio::test]
async fn rc_header_method_mismatch_is_rejected() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "ping")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["message"], "header_body_mismatch");
}

#[tokio::test]
async fn rc_tools_call_missing_mcp_name_is_rejected() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["message"], "header_body_mismatch");
}

#[tokio::test]
async fn rc_tools_call_name_mismatch_is_rejected() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "spawn_agent"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["message"], "header_body_mismatch");
}

#[tokio::test]
async fn rc_top_level_batch_array_is_rejected() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call")],
        json!([{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}]),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["message"], "batch_not_supported");
}

#[tokio::test]
async fn rc_transport_disabled_short_circuits_before_downstream() {
    let (router, _handle) = router_with(McpCompatConfig {
        rc_2026_07_28: false,
        ..McpCompatConfig::default()
    });
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["message"], "transport_disabled");
    assert_eq!(body["id"], Value::Null);
}

#[tokio::test]
async fn legacy_transport_disabled_short_circuits_before_downstream() {
    let (router, _handle) = router_with(McpCompatConfig {
        legacy_2025_06: false,
        ..McpCompatConfig::default()
    });
    let req = post_request(&[], json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call"}));
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["message"], "transport_disabled");
}

#[tokio::test]
async fn oversized_content_length_is_rejected_before_body_read() {
    let (router, _handle) = router_with(McpCompatConfig {
        max_body_bytes: 100,
        ..McpCompatConfig::default()
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(CONTENT_LENGTH, "101")
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rc_error_response_never_gains_a_traceparent() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "force_error")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "force_error"}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.get("error").is_some());
    assert!(body["error"].get("_meta").is_none());
    assert!(body.get("_meta").is_none());
}

#[tokio::test]
async fn stale_content_length_is_replaced_after_traceparent_rewrite() {
    let (router, _handle) = router_with(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cl = resp.headers().get(CONTENT_LENGTH).cloned();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    // The fake downstream sets a Content-Length matching its *pre-echo* body;
    // the middleware strips that stale value (`mod.rs::echo_traceparent_if_json`)
    // before returning, so it can never survive verbatim. Axum's own
    // top-level routing then re-derives a fresh Content-Length from the
    // actual response body it is about to send (see
    // `axum::routing::route::set_content_length`) — so whatever value shows
    // up here must match the real (post-rewrite) byte count, never the stale
    // pre-echo one.
    if let Some(cl) = cl {
        assert_eq!(cl.to_str().unwrap(), bytes.len().to_string());
    }
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["result"]["_meta"]["traceparent"], generate_stub());
}

#[tokio::test]
async fn non_json_response_passes_through_unmodified() {
    let router = sse_router(McpCompatConfig::default());
    let req = post_request(
        &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
    );
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "text/event-stream");
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"data: hello\n\n");
}

#[tokio::test]
async fn hot_reload_disables_rc_for_the_next_request_without_rebinding() {
    let (router, handle) = router_with(McpCompatConfig::default());
    let rc_req = || {
        post_request(
            &[(MCP_METHOD_HEADER, "tools/call"), (MCP_NAME_HEADER, "list_agents")],
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_agents"}}),
        )
    };

    let before = router.clone().oneshot(rc_req()).await.unwrap();
    assert_eq!(before.status(), StatusCode::OK);

    handle.set(McpCompatConfig {
        rc_2026_07_28: false,
        ..McpCompatConfig::default()
    });

    let after = router.clone().oneshot(rc_req()).await.unwrap();
    assert_eq!(after.status(), StatusCode::BAD_REQUEST);
    let body = body_json(after).await;
    assert_eq!(body["error"]["message"], "transport_disabled");
}
