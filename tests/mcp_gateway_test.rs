//! kwnt's MCP gateway: the per-turn streamable-HTTP endpoint codex reaches the kwant's servers
//! through. Every call is approved by kwnt (the approval card) before it is forwarded.

use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{models::McpServer, Database},
    drivers::events::RuntimeEvent,
    mcp::{FixedApprover, McpClient, McpTurnContext, ToolApprover},
    router::LayaRouter,
    server::{
        create_router,
        mcp_runtime::{self, BrokerApprover, PermissionSession},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::{tempdir, TempDir};

const KWNT: &str = env!("CARGO_BIN_EXE_kwnt");

fn echo_server() -> McpServer {
    McpServer {
        id: "s1".into(),
        name: "echo".into(),
        transport: "stdio".into(),
        command: Some(KWNT.into()),
        args: vec!["mcp-echo-server".into()],
        url: None,
        env: BTreeMap::from([("ECHO_SECRET".to_string(), "hunter2-secret".to_string())]),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    }
}

async fn state() -> (AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (state, tmp)
}

async fn serve(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, create_router(state)).await.unwrap() });
    format!("http://{addr}")
}

/// Registers a turn whose gateway calls are decided by `approver`.
async fn register(state: &AppState, token: &str, approver: Arc<dyn ToolApprover>) -> McpTurnContext {
    let ctx = McpTurnContext {
        servers: vec![echo_server()],
        approver,
        config_dir: std::env::temp_dir(),
        permission_url: None,
        gateway: None,
        max_tool_iterations: 8,
        records: Default::default(),
    };
    state.mcp_sessions.lock().await.insert(
        token.into(),
        PermissionSession::new("k1".into(), "Ada".into(), "t1".into(), ctx.clone(), None),
    );
    ctx
}

/// The gateway as codex sees it: an http MCP server with a bearer token header.
fn gateway(base: &str, token: Option<&str>) -> McpServer {
    McpServer {
        id: "kwnt".into(),
        name: "kwnt".into(),
        transport: "http".into(),
        command: None,
        args: vec![],
        url: Some(format!("{base}/api/mcp/gateway")),
        env: BTreeMap::new(),
        headers: token.map(|t| BTreeMap::from([("Authorization".to_string(), format!("Bearer {t}"))])).unwrap_or_default(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    }
}

#[tokio::test]
async fn gateway_lists_the_kwants_tools_and_forwards_approved_calls() {
    let (state, _tmp) = state().await;
    let base = serve(state.clone()).await;
    let ctx = register(&state, "allow-tok", Arc::new(FixedApprover(true))).await;

    let mut client = McpClient::connect(&gateway(&base, Some("allow-tok")), None).await.unwrap();
    assert_eq!(client.server_info["name"], "kwnt");
    let tools = client.list_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"echo__echo") && names.contains(&"echo__add"), "{names:?}");
    let echo = tools.iter().find(|t| t.name == "echo__echo").unwrap();
    assert_eq!(echo.input_schema["required"], json!(["text"]));

    let out = client.call_tool("echo__echo", json!({"text": "hello gateway"})).await.unwrap();
    assert!(!out.is_error);
    assert_eq!(out.text, "hello gateway");
    // Server secrets never reach the agent
    let secret = client.call_tool("echo__env", json!({"name": "ECHO_SECRET"})).await.unwrap();
    assert!(!secret.text.contains("hunter2-secret"), "{}", secret.text);
    // Tools the kwant doesn't have are refused
    let unknown = client.call_tool("other__echo", json!({})).await.unwrap();
    assert!(unknown.is_error);
    client.shutdown().await;

    let records = ctx.take_records();
    assert_eq!(records[0]["name"], "mcp__echo__echo");
    assert_eq!(records[0]["server"], "echo");
    assert_eq!(records[0]["status"], "ok");
    assert_eq!(records[0]["result"], "hello gateway");
    mcp_runtime::finish_turn(&state, "allow-tok").await;
}

#[tokio::test]
async fn gateway_denied_call_returns_no_output() {
    let (state, _tmp) = state().await;
    let base = serve(state.clone()).await;
    let ctx = register(&state, "deny-tok", Arc::new(FixedApprover(false))).await;

    let mut client = McpClient::connect(&gateway(&base, Some("deny-tok")), None).await.unwrap();
    let out = client.call_tool("echo__echo", json!({"text": "must not run"})).await.unwrap();
    assert!(out.is_error);
    assert_eq!(out.text, "The user denied this tool call.");
    client.shutdown().await;

    let records = ctx.take_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["status"], "denied");
    assert!(records[0].get("result").is_none());
    mcp_runtime::finish_turn(&state, "deny-tok").await;
}

#[tokio::test]
async fn gateway_rejects_missing_wrong_and_finished_tokens() {
    let (state, _tmp) = state().await;
    let base = serve(state.clone()).await;
    register(&state, "live-tok", Arc::new(FixedApprover(true))).await;

    assert!(McpClient::connect(&gateway(&base, None), None).await.is_err(), "no token");
    assert!(McpClient::connect(&gateway(&base, Some("wrong-tok")), None).await.is_err(), "wrong token");
    let status = reqwest::Client::new()
        .post(format!("{base}/api/mcp/gateway"))
        .header("Authorization", "Basic live-tok")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 401, "only bearer tokens");
    // The token never works as a path segment
    assert!(McpClient::connect(&gateway(&format!("{base}/api/mcp/gateway/live-tok"), None), None).await.is_err());

    assert!(McpClient::connect(&gateway(&base, Some("live-tok")), None).await.is_ok());
    mcp_runtime::finish_turn(&state, "live-tok").await;
    assert!(McpClient::connect(&gateway(&base, Some("live-tok")), None).await.is_err(), "finished turn");
}

#[tokio::test]
async fn gateway_calls_wait_for_the_approval_card() {
    let (state, _tmp) = state().await;
    let base = serve(state.clone()).await;
    let approver = Arc::new(BrokerApprover {
        db: state.db.clone(),
        broker: state.broker.clone(),
        events: state.event_sender.clone(),
        timeout: Duration::from_secs(10),
    });
    register(&state, "card-tok", approver).await;
    let mut events = state.event_sender.subscribe();

    let client = McpClient::connect(&gateway(&base, Some("card-tok")), None).await.unwrap();
    let call = tokio::spawn(async move {
        let out = client.call_tool("echo__echo", json!({"text": "after approval"})).await;
        (client, out)
    });
    let ticket = loop {
        match tokio::time::timeout(Duration::from_secs(10), events.recv()).await.unwrap().unwrap() {
            RuntimeEvent::ApprovalRequired { id, tool_name, details } => {
                assert_eq!(tool_name, "mcp__echo__echo");
                assert_eq!(details["kwant_name"], "Ada");
                assert_eq!(details["arguments"]["text"], "after approval");
                break id;
            }
            _ => continue,
        }
    };
    assert!(!call.is_finished(), "the call must wait for the user");
    state.broker.resolve(&ticket, ApprovalDecision::AllowOnce).await;
    let (mut client, out) = call.await.unwrap();
    assert_eq!(out.unwrap().text, "after approval");
    client.shutdown().await;
    mcp_runtime::finish_turn(&state, "card-tok").await;
}
