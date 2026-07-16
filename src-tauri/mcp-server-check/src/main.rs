// Standalone smoke test validating the exact rmcp 1.8 API used by
// src-tauri/src/queen.rs: streamable-HTTP MCP server (tool_router/tool/
// tool_handler macros + StreamableHttpService served by axum) plus an
// in-process MCP client (StreamableHttpClientTransport + reqwest) running
// initialize -> tools/list -> tools/call.

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolRequestParams, ClientInfo, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::{
        StreamableHttpClientTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
            session::local::LocalSessionManager,
        },
    },
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EchoRequest {
    #[schemars(description = "text to echo back")]
    text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct AddRequest {
    a: i64,
    b: i64,
}

#[derive(Clone)]
struct Dummy {
    // Read by the #[tool_handler]-generated code; dead_code lint can't see that.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl Dummy {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl Dummy {
    #[tool(description = "Echo the input text")]
    fn echo(&self, Parameters(EchoRequest { text }): Parameters<EchoRequest>) -> String {
        format!("echo: {text}")
    }

    #[tool(description = "Add two integers")]
    fn add(&self, Parameters(AddRequest { a, b }): Parameters<AddRequest>) -> String {
        (a + b).to_string()
    }
}

#[tool_handler]
impl ServerHandler for Dummy {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("dummy smoke-test server")
    }
}

/// Minimal `/mcp` token guard mirroring queen.rs (query `?token=` or Bearer).
/// Keeps this smoke honest that rmcp's client works through a token-carrying URL.
async fn mcp_auth(
    axum::extract::State(token): axum::extract::State<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ok_query = req
        .uri()
        .query()
        .map(|q| q.split('&').any(|p| p == format!("token={token}")))
        .unwrap_or(false);
    let ok_bearer = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == token)
        .unwrap_or(false);
    if ok_query || ok_bearer {
        next.run(req).await
    } else {
        axum::http::StatusCode::UNAUTHORIZED.into_response()
    }
}

fn args(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_value(v).expect("args must be an object")
}

fn first_text(result: &rmcp::model::CallToolResult) -> String {
    let v = serde_json::to_value(result).expect("serialize CallToolResult");
    v["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ---- server on an ephemeral localhost port ----
    let service: StreamableHttpService<Dummy, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(Dummy::new()),
            Default::default(),
            StreamableHttpServerConfig::default(),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    // Mirror queen.rs Finding S1: guard /mcp with a per-run token so this smoke
    // proves the token-carrying URL still completes a full MCP handshake.
    let token = "smoketoken0123456789abcdef".to_string();
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn_with_state(
            token.clone(),
            mcp_auth,
        ));
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let url = format!("http://{addr}/mcp?token={token}");
    println!("server listening at {url}");

    // ---- client: initialize ----
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url.clone()),
    );
    let client = ClientInfo::default().serve(transport).await?;
    println!(
        "initialize OK: {}",
        serde_json::to_string(&client.peer_info())?
    );

    // ---- tools/list ----
    let tools = client.list_tools(None).await?;
    let names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();
    println!("tools/list OK: {names:?}");
    assert!(names.contains(&"echo".to_string()), "echo tool missing");
    assert!(names.contains(&"add".to_string()), "add tool missing");

    // ---- tools/call echo ----
    let res = client
        .call_tool(
            CallToolRequestParams::new("echo")
                .with_arguments(args(serde_json::json!({"text": "hello-queen"}))),
        )
        .await?;
    let echo_text = first_text(&res);
    println!("tools/call echo OK: {echo_text:?} (is_error={:?})", res.is_error);
    assert_eq!(echo_text, "echo: hello-queen");
    assert_ne!(res.is_error, Some(true));

    // ---- tools/call add ----
    let res = client
        .call_tool(
            CallToolRequestParams::new("add")
                .with_arguments(args(serde_json::json!({"a": 20, "b": 22}))),
        )
        .await?;
    let add_text = first_text(&res);
    println!("tools/call add OK: {add_text:?} (is_error={:?})", res.is_error);
    assert_eq!(add_text, "42");
    assert_ne!(res.is_error, Some(true));

    client.cancel().await?;
    println!("MCP SMOKE TEST PASSED: initialize + tools/list + 2x tools/call all succeeded");
    Ok(())
}
