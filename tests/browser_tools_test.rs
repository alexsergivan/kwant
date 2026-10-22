//! The bot browser as kwnt's own MCP tools (`browser_open`, `browser_page`, `browser_act`).
//! Claude's permission prompt asks the person before Bash, so the old curl instructions were a
//! detour; these tools go through the self server, which needs no approval card, and drive
//! only the turn's own sandbox. The browser part runs a real windowless Chrome in Local mode and
//! is skipped when none is installed.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    drivers::{BrowserAccess, TurnInput},
    mcp::{PERMISSION_TOOL, SELF_SERVER_NAME},
    router::LayaRouter,
    sandbox::computer,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const PAGE: &str = "<!doctype html><title>Test page</title><body>\
<p>Hello from the test page</p>\
<button onclick=\"document.title='Clicked!';document.body.insertAdjacentHTML('beforeend','<p>button was pressed</p>')\">Press me</button>";

async fn serve_page() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

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

async fn post(app: &Router, uri: &str, auth: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

/// Calls one self tool; returns its text and whether it is an error.
async fn self_tool(app: &Router, token: &str, name: &str, args: Value) -> (String, bool) {
    let (status, v) = post(app, "/api/mcp/self", Some(token), rpc("tools/call", json!({"name": name, "arguments": args}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (
        v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(),
        v["result"]["isError"].as_bool().unwrap_or(false),
    )
}

async fn tool_names(app: &Router, token: &str) -> Vec<String> {
    let (_, v) = post(app, "/api/mcp/self", Some(token), rpc("tools/list", json!({}))).await;
    v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

/// What claude's `--permission-prompt-tool` asks kwnt before running a tool.
async fn claude_asks(app: &Router, token: &str, tool_name: &str) -> String {
    let (status, v) = post(
        app,
        &format!("/api/mcp/permission/{token}"),
        None,
        rpc("tools/call", json!({"name": PERMISSION_TOOL, "arguments": {"tool_name": tool_name, "input": {"url": "http://93.184.216.34/"}, "tool_use_id": "u1"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let decision: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    decision["behavior"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn claude_turn_gets_browser_tools_that_drive_its_own_panel() {
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    std::env::set_var("KWNT_COMPUTER_MODE", "local");
    let (app, state, _tmp) = setup().await;
    let kwants = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let (bot, other) = (&kwants[0], &kwants[1]);
    let scope = |sandbox: Option<&str>| SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        sandbox_target: sandbox.map(str::to_string),
        ..Default::default()
    };

    // A turn with its sandbox: the browser tools are listed next to the memory tools
    let (ctx, token) = mcp_runtime::prepare_turn_with_self(&state, bot, None, None, Some(scope(Some(&bot.id))))
        .await
        .unwrap();
    let names = tool_names(&app, &token).await;
    for tool in ["browser_open", "browser_page", "browser_act", "memory_add"] {
        assert!(names.iter().any(|n| n == tool), "{tool} missing from {names:?}");
    }
    // Without a sandbox (e.g. a turn that shows no panel) they aren't offered or callable
    let (_, bare) = mcp_runtime::prepare_turn_with_self(&state, bot, None, None, Some(scope(None))).await.unwrap();
    assert!(!tool_names(&app, &bare).await.iter().any(|n| n.starts_with("browser_")));
    let (text, is_error) = self_tool(&app, &bare, "browser_open", json!({"url": "http://127.0.0.1:1/"})).await;
    assert!(is_error && text.contains("Unknown tool"), "{text}");

    // claude asks before every tool: the browser tools (like web lookups) pass with no card
    assert_eq!(claude_asks(&app, &token, "WebFetch").await, "allow");
    assert_eq!(claude_asks(&app, &token, &format!("mcp__{SELF_SERVER_NAME}__browser_open")).await, "allow");
    assert_eq!(claude_asks(&app, &token, &format!("mcp__{SELF_SERVER_NAME}__browser_act")).await, "allow");
    assert!(state.broker.pending_ids().await.is_empty(), "no approval card for the bot's own browser");

    // The prompt tells a tools turn to use them, not curl through a shell it doesn't have
    let input = TurnInput {
        thread_id: "t".into(),
        kwant: bot.clone(),
        prompt: "open the page".into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: None,
        team_context: None,
        workspace_dir: None,
        browser_api_url: Some("http://127.0.0.1:7878/api/sandbox/x".into()),
        sandbox_id: Some(bot.id.clone()),
        teammates: vec![],
        continuity: String::new(),
        mcp: Some(ctx.clone()),
    };
    let prompt = input.format_system_prompt();
    assert!(prompt.contains("browser_open") && !prompt.contains("curl -sS"), "{prompt}");
    let shell = TurnInput { mcp: None, ..input.clone() }.format_system_prompt();
    assert!(shell.contains("curl -sS -X POST 'http://127.0.0.1:7878/api/sandbox/x/navigate'"));
    let none = input.format_system_prompt_with(BrowserAccess::Unavailable);
    assert!(!none.contains("curl -sS") && !none.contains("browser_open") && none.contains("no way to use the browser"));

    // Every turn knows it is a kwant bot, not the engine; only a turn whose tools work is sent to `routine_create`
    for (label, p) in [("tools", &prompt), ("shell", &shell), ("unavailable", &none)] {
        assert!(p.contains("[Who you are]"), "{label}");
        assert!(p.contains(&format!("You are {}, a bot in kwant", bot.name)), "{label}");
        assert!(p.contains("you are not Claude Code, Codex or Grok"), "{label}");
        assert!(p.contains("Never use the engine's own scheduler"), "{label}");
    }
    assert!(prompt.contains("`routine_create`") && !prompt.contains("Routines in kwant's sidebar"));
    for p in [&shell, &none] {
        assert!(p.contains("Routines in kwant's sidebar") && !p.contains("`routine_create`"), "{p}");
    }

    if computer::local_chrome_path().is_none() {
        eprintln!("browser part skipped: no Chrome or Chromium installed");
        mcp_runtime::finish_turn(&state, &token).await;
        return;
    }
    let url = serve_page().await;
    let (text, is_error) = self_tool(&app, &token, "browser_open", json!({"url": url})).await;
    assert!(!is_error, "{text}");
    let page: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(page["status"], "navigated", "{page}");
    assert_eq!(page["title"], "Test page");
    assert!(page["text"].as_str().unwrap().contains("Hello from the test page"));
    assert!(page["clickable"].to_string().contains("Press me"), "{page}");

    let (text, is_error) = self_tool(&app, &token, "browser_act", json!({"action": "click_text", "text": "Press me"})).await;
    assert!(!is_error, "{text}");
    let page: Value = serde_json::from_str(&text).unwrap();
    assert!(page["result"].as_str().unwrap().contains("Press me"), "{page}");
    assert!(page["text"].as_str().unwrap().contains("button was pressed"), "{page}");

    // It drove the bot's own panel: the same controller the side panel's screenshots come from
    let ctrl = state.browser_controllers.lock().await.get(&bot.id).cloned().expect("bot's browser");
    let mut title = String::new();
    for _ in 0..30 {
        title = ctrl.sync_location().await.map(|(_, t)| t).unwrap_or_default();
        if title == "Clicked!" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(title, "Clicked!");
    assert!(!ctrl.capture_screenshot().await.unwrap().is_empty());
    assert!(!state.browser_controllers.lock().await.contains_key(&other.id), "no other bot's browser touched");

    let (text, is_error) = self_tool(&app, &token, "browser_page", json!({})).await;
    assert!(!is_error && text.contains("button was pressed"), "{text}");
    let (text, is_error) = self_tool(&app, &token, "browser_act", json!({"action": "dance"})).await;
    assert!(is_error && text.contains("Unknown action"), "{text}");

    // Take Control: the bot is told to stop, and nothing is clicked
    {
        let _held = ctrl.control.clone().try_write_owned().unwrap();
        let (text, is_error) = self_tool(&app, &token, "browser_act", json!({"action": "click_text", "text": "Press me"})).await;
        assert!(is_error && text.contains("taken control"), "{text}");
    }

    // The turn's token ends with the turn
    mcp_runtime::finish_turn(&state, &token).await;
    let (status, _) = post(&app, "/api/mcp/self", Some(&token), rpc("tools/list", json!({}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    drop(ctrl);
    state.browser_service.set_mode(computer::ComputerMode::Vm).await;
    let _ = computer::wait_local_chrome_gone(&state.config.data_dir.join("local-chrome")).await;
}
