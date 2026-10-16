use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::McpServer, Database},
    mcp::{McpClient, McpError, SECRET_MASK},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const KWNT: &str = env!("CARGO_BIN_EXE_kwnt");

fn echo_server(timeout_secs: i64) -> McpServer {
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
        timeout_secs,
        created_at: None,
    }
}

async fn app() -> (Router, Database, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db.clone(),
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state), db, tmp)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn stdio_client_lists_and_calls_tools() {
    let mut client = McpClient::connect(&echo_server(10), None).await.unwrap();
    assert_eq!(client.server_info["name"], "kwnt-echo");

    let tools = client.list_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["echo", "add", "sleep", "env", "crash"]);
    assert_eq!(tools[0].input_schema["required"], json!(["text"]));

    let out = client.call_tool("echo", json!({"text": "hello mcp"})).await.unwrap();
    assert_eq!(out.text, "hello mcp");
    assert!(!out.is_error);
    assert_eq!(client.call_tool("add", json!({"a": 2, "b": 3})).await.unwrap().text, "5");
    // Configured env reaches the server; kwnt's own environment does not
    assert_eq!(client.call_tool("env", json!({"name": "ECHO_SECRET"})).await.unwrap().text, "hunter2-secret");
    std::env::set_var("KWNT_PRIVATE_TEST_VAR", "leak");
    let mut fresh = McpClient::connect(&echo_server(10), None).await.unwrap();
    assert!(fresh.call_tool("env", json!({"name": "KWNT_PRIVATE_TEST_VAR"})).await.unwrap().is_error);
    fresh.shutdown().await;

    client.shutdown().await;
    // A shut-down client fails fast instead of hanging
    assert!(client.call_tool("echo", json!({"text": "x"})).await.is_err());
}

#[tokio::test]
async fn server_crash_mid_call_fails_fast() {
    let mut client = McpClient::connect(&echo_server(30), None).await.unwrap();
    let start = Instant::now();
    let err = client.call_tool("crash", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Closed), "{err:?}");
    assert!(start.elapsed() < Duration::from_secs(5), "crash should not wait for the timeout");
    assert!(client.stderr_tail().contains("crashing on request"));
    client.shutdown().await;
}

#[tokio::test]
async fn slow_tool_times_out_and_server_is_stopped() {
    let mut client = McpClient::connect(&echo_server(1), None).await.unwrap();
    let start = Instant::now();
    let err = client.call_tool("sleep", json!({"ms": 10_000})).await.unwrap_err();
    assert!(matches!(err, McpError::Timeout(1, _)), "{err:?}");
    assert!(start.elapsed() < Duration::from_secs(3));
    let start = Instant::now();
    client.shutdown().await;
    assert!(start.elapsed() < Duration::from_secs(4), "shutdown must not wait for the sleeping tool");
}

#[tokio::test]
async fn missing_command_is_a_startup_error() {
    let mut server = echo_server(5);
    server.command = Some("/nonexistent/kwnt-mcp-binary".into());
    assert!(matches!(McpClient::connect(&server, None).await, Err(McpError::Spawn(_))));
}

#[tokio::test]
async fn registry_crud_masks_secrets() {
    let (app, db, _tmp) = app().await;

    let body = json!({"name": "echo", "transport": "stdio", "command": KWNT, "args": ["mcp-echo-server"],
                      "env": {"ECHO_SECRET": "hunter2-secret"}});
    let (status, created) = call(&app, "POST", "/api/mcp/servers", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["env"]["ECHO_SECRET"], SECRET_MASK);
    assert_eq!(created["enabled"], true);
    assert_eq!(created["timeout_secs"], 30);
    let id = created["id"].as_str().unwrap().to_string();

    let (status, _) = call(&app, "POST", "/api/mcp/servers", Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, e) = call(&app, "POST", "/api/mcp/servers", Some(json!({"name": "x", "transport": "http"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(e["error"].as_str().unwrap().contains("url"));

    let (_, list) = call(&app, "GET", "/api/mcp/servers", None).await;
    assert!(!list.to_string().contains("hunter2"));
    let (_, one) = call(&app, "GET", &format!("/api/mcp/servers/{id}"), None).await;
    assert_eq!(one["env"]["ECHO_SECRET"], SECRET_MASK);

    // Sending the mask back keeps the stored secret
    let update = json!({"name": "echo2", "transport": "stdio", "command": KWNT, "args": ["mcp-echo-server"],
                        "env": {"ECHO_SECRET": SECRET_MASK, "EXTRA": "e"}, "enabled": false});
    let (status, updated) = call(&app, "PUT", &format!("/api/mcp/servers/{id}"), Some(update)).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "echo2");
    assert_eq!(updated["enabled"], false);
    let stored = db.get_mcp_server(&id).await.unwrap().unwrap();
    assert_eq!(stored.env["ECHO_SECRET"], "hunter2-secret");
    assert_eq!(stored.env["EXTRA"], "e");

    let (status, _) = call(&app, "GET", "/api/mcp/servers/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Assignment: only enabled servers are active for the kwant
    let kwant_id = db.list_kwants().await.unwrap()[0].id.clone();
    let uri = format!("/api/kwants/{kwant_id}/mcp-servers");
    let (status, _) = call(&app, "PUT", &uri, Some(json!({"server_ids": ["nope"]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, assigned) = call(&app, "PUT", &uri, Some(json!({"server_ids": [id]}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(assigned["server_ids"], json!([id]));
    assert!(db.list_active_mcp_servers_for_kwant(&kwant_id).await.unwrap().is_empty());
    let (status, _) = call(&app, "GET", "/api/kwants/nope/mcp-servers", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(&app, "DELETE", &format!("/api/mcp/servers/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = call(&app, "GET", &uri, None).await;
    assert_eq!(after["server_ids"], json!([]));
    let (status, _) = call(&app, "DELETE", &format!("/api/mcp/servers/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_endpoint_lists_tools_and_redacts_errors() {
    let (app, _db, _tmp) = app().await;

    let (_, ok_server) = call(&app, "POST", "/api/mcp/servers",
        Some(json!({"name": "echo", "transport": "stdio", "command": KWNT, "args": ["mcp-echo-server"]}))).await;
    let (status, result) = call(&app, "POST", &format!("/api/mcp/servers/{}/test", ok_server["id"].as_str().unwrap()), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["tools"].as_array().unwrap().len(), 5);
    assert_eq!(result["server_info"]["name"], "kwnt-echo");

    // A server that prints its secret and dies: the error comes back without the secret
    let (_, bad) = call(&app, "POST", "/api/mcp/servers", Some(json!({
        "name": "leaky", "transport": "stdio", "command": "sh",
        "args": ["-c", "echo \"token is $LEAKY_TOKEN\" >&2; exit 1"],
        "env": {"LEAKY_TOKEN": "tok-abcdef-123"}, "timeout_secs": 5}))).await;
    let (_, result) = call(&app, "POST", &format!("/api/mcp/servers/{}/test", bad["id"].as_str().unwrap()), None).await;
    assert_eq!(result["ok"], false, "{result}");
    let text = result.to_string();
    assert!(!text.contains("tok-abcdef-123"), "{text}");
    assert!(result["stderr_tail"].as_str().unwrap().contains(&format!("token is {SECRET_MASK}")), "{text}");

    let (status, _) = call(&app, "POST", "/api/mcp/servers/nope/test", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
