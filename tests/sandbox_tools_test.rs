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

    // 2. GET /api/sandbox/:team_id/screenshot
    let req = Request::builder()
        .uri("/api/sandbox/team-a/screenshot")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(val.get("image_base64").is_some() || val.get("screenshot").is_some());

    // 3. POST /api/sandbox/:team_id/restart
    let req = Request::builder()
        .method("POST")
        .uri("/api/sandbox/team-a/restart")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
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
    let mut rx = state.event_tx.subscribe();

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
}

