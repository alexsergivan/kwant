use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::json;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_browser_tools_execution_and_event_serialization() {
    let tmp = tempdir().unwrap();
    let runner = ToolRunner::new(tmp.path().to_path_buf());

    // When no sandbox active, browser tools return helpful guidance or fallback
    let res = runner.execute("browser_navigate", &json!({ "url": "https://example.com" })).await;
    assert!(res.is_ok() || res.is_err());
    let val = res.unwrap();
    assert_eq!(val["status"], "navigated");

    // Verify event serialization
    let event = RuntimeEvent::BrowserNavigated {
        team_id: "core-engineering".into(),
        url: "https://docs.rs".into(),
        title: "Rust Documentation".into(),
        status_code: 200,
    };
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(serialized.contains("browser_navigated") || serialized.contains("BrowserNavigated"));
    assert!(serialized.contains("https://docs.rs"));
}

#[tokio::test]
async fn test_all_browser_tools_fallback() {
    let tmp = tempdir().unwrap();
    let runner = ToolRunner::new(tmp.path().to_path_buf());

    let res = runner.execute("browser_screenshot", &json!({})).await.unwrap();
    assert_eq!(res["status"], "captured");
    assert_eq!(res["url"], "about:blank");
    assert!(res["image_base64"].as_str().unwrap().len() > 10);
    assert!(res.get("timestamp").is_some());

    let res = runner.execute("browser_extract", &json!({})).await.unwrap();
    assert_eq!(res["status"], "extracted");

    let res = runner.execute("browser_click", &json!({ "selector": "#submit-btn" })).await.unwrap();
    assert_eq!(res["status"], "clicked");

    let res = runner.execute("browser_type", &json!({ "selector": "#input-box", "text": "hello" })).await.unwrap();
    assert_eq!(res["status"], "typed");
}

#[tokio::test]
async fn test_browser_and_sandbox_events_serialization() {
    let event1 = RuntimeEvent::BrowserScreenshot {
        team_id: "team-1".into(),
        url: "https://kwnt.dev".into(),
        image_base64: "dGVzdA==".into(),
        timestamp: "2026-10-15T12:00:00Z".into(),
    };
    let json1 = serde_json::to_string(&event1).unwrap();
    assert!(json1.contains("browser_screenshot"));
    assert!(json1.contains("dGVzdA=="));

    let event2 = RuntimeEvent::SandboxStatusChanged {
        team_id: "team-1".into(),
        container_name: "kwnt-sandbox-team-1".into(),
        is_running: true,
    };
    let json2 = serde_json::to_string(&event2).unwrap();
    assert!(json2.contains("sandbox_status"));
    assert!(json2.contains("kwnt-sandbox-team-1"));
}

#[tokio::test]
async fn test_sandbox_rest_endpoints() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let mut rx = state.event_sender.subscribe();
    let app = create_router(state);

    // 1. GET /api/sandbox/:team_id/status
    let req = Request::builder()
        .uri("/api/sandbox/team-a/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(val["team_id"], "team-a");
    assert_eq!(val["is_running"], false);

    // 2. GET /api/sandbox/:team_id/screenshot (Read-only GET must not broadcast SSE event)
    let req = Request::builder()
        .uri("/api/sandbox/team-a/screenshot")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(val.get("image_base64").is_some() || val.get("screenshot").is_some());
    // Verify no broadcast occurred on read-only GET
    assert!(rx.try_recv().is_err());

    // 3. POST /api/sandbox/:team_id/restart (Broadcasts SandboxStatusChanged event)
    let req = Request::builder()
        .method("POST")
        .uri("/api/sandbox/team-a/restart")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let event = rx.try_recv().expect("Expected SandboxStatusChanged on restart");
    match event {
        RuntimeEvent::SandboxStatusChanged { team_id, is_running, .. } => {
            assert_eq!(team_id, "team-a");
            assert_eq!(is_running, false);
        }
        other => panic!("Expected SandboxStatusChanged, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_tool_event_broadcasting() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let mut rx = state.event_sender.subscribe();

    let res = kwnt::server::routes::execute_tool(
        &state,
        Some("team-b"),
        "browser_navigate",
        &json!({ "url": "https://example.com" }),
    )
    .await
    .unwrap();
    assert_eq!(res["status"], "navigated");

    // Check that BrowserNavigated event was broadcast
    let event = rx.recv().await.unwrap();
    match event {
        RuntimeEvent::BrowserNavigated { team_id, url, .. } => {
            assert_eq!(team_id, "team-b");
            assert_eq!(url, "https://example.com");
        }
        other => panic!("Expected BrowserNavigated, got {:?}", other),
    }

    // Also test browser_screenshot tool execution event broadcasting
    let res_shot = kwnt::server::routes::execute_tool(
        &state,
        Some("team-b"),
        "browser_screenshot",
        &json!({}),
    )
    .await
    .unwrap();
    assert_eq!(res_shot["status"], "captured");

    let event_shot = rx.recv().await.unwrap();
    match event_shot {
        RuntimeEvent::BrowserScreenshot { team_id, url, image_base64, .. } => {
            assert_eq!(team_id, "team-b");
            assert_eq!(url, "about:blank");
            assert!(!image_base64.is_empty());
        }
        other => panic!("Expected BrowserScreenshot, got {:?}", other),
    }
}

#[tokio::test]
async fn test_lazy_controller_initialization() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);

    // Initial state: no controller
    assert!(kwnt::server::routes::get_or_create_browser_controller(&state, "team-x").await.is_none());
}

#[tokio::test]
async fn test_execute_tool_shell_fallback() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);

    let res = kwnt::server::routes::execute_tool(
        &state,
        Some("team-test"),
        "execute_shell",
        &json!({ "command": "echo 'sandboxed-fallback'" }),
    )
    .await
    .unwrap();

    assert_eq!(res["exit_code"], 0);
    assert!(res["stdout"].as_str().unwrap().contains("sandboxed-fallback"));
}

#[tokio::test]
async fn test_restart_sandbox_stopped_when_docker_unavailable() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let mut rx = state.event_sender.subscribe();

    let json_res = kwnt::server::routes::restart_sandbox(
        axum::extract::State(state),
        axum::extract::Path("team-restarted".to_string()),
    )
    .await
    .unwrap();

    assert_eq!(json_res.0["status"], "stopped");
    assert_eq!(json_res.0["is_running"], false);
    assert_eq!(json_res.0["team_id"], "team-restarted");

    let event = rx.recv().await.unwrap();
    match event {
        RuntimeEvent::SandboxStatusChanged { team_id, is_running, container_name } => {
            assert_eq!(team_id, "team-restarted");
            assert_eq!(is_running, false);
            assert!(container_name.contains("team-restarted"));
        }
        other => panic!("Expected SandboxStatusChanged, got {:?}", other),
    }
}

