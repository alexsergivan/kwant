//! claude's own tools through kwnt's permission prompt: lookups and writes inside the bot's
//! working folder run without a card; writes outside it and shell commands show the person an
//! approval card ("Always this session" is honoured); tools from servers the bot doesn't have
//! are refused.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    mcp::PERMISSION_TOOL,
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state, tmp)
}

async fn post(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// What claude's `--permission-prompt-tool` asks kwnt before running a tool.
async fn claude_asks(app: &Router, token: &str, tool_name: &str, input: Value) -> Value {
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": PERMISSION_TOOL, "arguments": {"tool_name": tool_name, "input": input, "tool_use_id": "u1"},
    }});
    let (status, v) = post(app, &format!("/api/mcp/permission/{token}"), call).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// The next approval card shown: its id and details.
async fn next_card(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> (String, Value) {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await.expect("no approval card").unwrap() {
            RuntimeEvent::ApprovalRequired { id, details, .. } => return (id, details),
            _ => continue,
        }
    }
}

#[tokio::test]
async fn built_in_tools_follow_the_folder_policy() {
    let (app, state, tmp) = setup().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let folder = tmp.path().join("ws");
    std::fs::create_dir_all(&folder).unwrap();
    let real = folder.canonicalize().unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&state, &bot, None, Some(folder.clone()), Some(scope)).await.unwrap();
    let mut events = state.event_sender.subscribe();

    // Looking things up and writing inside the folder: no card
    for (tool, input) in [
        ("WebSearch", json!({"query": "sma crossover"})),
        ("WebFetch", json!({"url": "https://example.com", "prompt": "summary"})),
        ("Write", json!({"file_path": real.join("strategy.go").display().to_string(), "content": "package x"})),
        ("Edit", json!({"file_path": "strategy.go", "old_string": "x", "new_string": "y"})),
        ("Read", json!({"file_path": "strategy.go"})),
    ] {
        let d = claude_asks(&app, &token, tool, input.clone()).await;
        assert_eq!(d, json!({"behavior": "allow", "updatedInput": input}), "{tool}");
    }
    assert!(state.broker.pending_ids().await.is_empty(), "no card for lookups or in-folder writes");

    // Writing outside the folder (or escaping it with ..) shows a card; Deny refuses it
    let outside = tmp.path().join("elsewhere.txt").display().to_string();
    for path in [outside.clone(), "../elsewhere.txt".to_string()] {
        let ask = tokio::spawn({
            let (app, token) = (app.clone(), token.clone());
            async move { claude_asks(&app, &token, "Write", json!({"file_path": path, "content": "x"})).await }
        });
        let (id, details) = next_card(&mut events).await;
        assert_eq!(details["source"], "claude_builtin");
        assert_eq!(details["tool"], "Write");
        assert_eq!(details["outside_folder"], true);
        assert_eq!(details["cwd"], real.display().to_string());
        let (status, _) = post(&app, &format!("/api/approvals/{id}/resolve"), json!({"decision": "deny"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ask.await.unwrap()["behavior"], "deny");
    }

    // A shell command shows a card with the command and folder; Allow once runs it once
    let bash = |cmd: &str| json!({"command": cmd, "description": "List files"});
    let ask = tokio::spawn({
        let (app, token, input) = (app.clone(), token.clone(), bash("ls"));
        async move { claude_asks(&app, &token, "Bash", input).await }
    });
    let (id, details) = next_card(&mut events).await;
    assert_eq!(details["source"], "claude_builtin");
    assert_eq!(details["arguments"]["command"], "ls");
    assert_eq!(details["outside_folder"], false);
    assert_eq!(details["cwd"], real.display().to_string());
    assert_eq!(details["kwant_name"], bot.name);
    post(&app, &format!("/api/approvals/{id}/resolve"), json!({"decision": "allow_once"})).await;
    assert_eq!(ask.await.unwrap(), json!({"behavior": "allow", "updatedInput": bash("ls")}));

    // Always this session: the next command runs without a card
    let ask = tokio::spawn({
        let (app, token, input) = (app.clone(), token.clone(), bash("go test ./..."));
        async move { claude_asks(&app, &token, "Bash", input).await }
    });
    let (id, _) = next_card(&mut events).await;
    post(&app, &format!("/api/approvals/{id}/resolve"), json!({"decision": "always_allow"})).await;
    assert_eq!(ask.await.unwrap()["behavior"], "allow");
    assert_eq!(claude_asks(&app, &token, "Bash", bash("ls -la")).await["behavior"], "allow");
    assert!(state.broker.pending_ids().await.is_empty(), "always-this-session skips the card");

    // A tool kwnt doesn't know asks too; servers the bot doesn't have are refused outright
    let ask = tokio::spawn({
        let (app, token) = (app.clone(), token.clone());
        async move { claude_asks(&app, &token, "SomeNewTool", json!({})).await }
    });
    let (id, details) = next_card(&mut events).await;
    assert_eq!(details["tool"], "SomeNewTool");
    post(&app, &format!("/api/approvals/{id}/resolve"), json!({"decision": "deny"})).await;
    assert_eq!(ask.await.unwrap()["behavior"], "deny");
    for tool in ["mcp__other__echo", "mcp__kwnt__approve", ""] {
        assert_eq!(claude_asks(&app, &token, tool, json!({})).await["behavior"], "deny", "{tool}");
    }
    assert!(state.broker.pending_ids().await.is_empty());
    mcp_runtime::finish_turn(&state, &token).await;
}

#[tokio::test]
async fn without_a_folder_every_write_asks() {
    let (app, state, _tmp) = setup().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&state, &bot, None, None, Some(scope)).await.unwrap();
    let mut events = state.event_sender.subscribe();
    let ask = tokio::spawn({
        let (app, token) = (app.clone(), token.clone());
        async move { claude_asks(&app, &token, "Write", json!({"file_path": "/tmp/x.txt", "content": "x"})).await }
    });
    let (id, details) = next_card(&mut events).await;
    assert_eq!(details["outside_folder"], true);
    assert!(details["cwd"].is_null());
    // The turn ending withdraws the card and denies the call
    mcp_runtime::finish_turn(&state, &token).await;
    assert_eq!(ask.await.unwrap()["behavior"], "deny");
    assert!(!state.broker.is_pending(&id).await);
}
