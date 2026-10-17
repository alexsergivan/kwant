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
    assert!(res.unwrap_err().to_string().contains("unavailable"));

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

    for (tool, args) in [
        ("browser_screenshot", json!({})),
        ("browser_extract", json!({})),
        ("browser_click", json!({"selector": "#submit-btn"})),
        ("browser_type", json!({"selector": "#input-box", "text": "hello"})),
    ] {
        assert!(runner.execute(tool, &args).await.unwrap_err().to_string().contains("unavailable"));
    }
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
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(val["error"].is_string());
    assert!(val.get("image_base64").is_none());
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
            assert!(!is_running);
        }
        other => panic!("Expected SandboxStatusChanged, got {:?}", other),
    }
}

#[tokio::test]
async fn test_execute_tool_event_broadcasting() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let mut rx = state.event_sender.subscribe();

    for (tool, args) in [
        ("browser_navigate", json!({"url": "https://example.com"})),
        ("browser_screenshot", json!({})),
    ] {
        let result = kwnt::server::routes::execute_tool(&state, Some("team-b"), tool, &args).await;
        assert!(result.is_err());
        assert!(rx.try_recv().is_err(), "unavailable browser must not broadcast success");
    }
}

#[tokio::test]
async fn test_lazy_controller_initialization() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    state.db.set_setting("computer_mode", "vm").await.unwrap();

    // vm mode with an unknown team: refused with a clear error, never run on this computer
    let err = kwnt::server::routes::execute_tool(
        &state,
        Some("team-test"),
        "execute_shell",
        &json!({ "command": "echo 'sandboxed-fallback'" }),
    )
    .await
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("team-test") && msg.contains("not run"), "{msg}");
}

#[tokio::test]
async fn test_vm_shell_unknown_target_never_runs_on_host() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let tools = ToolRunner::new(workspace.clone());
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    state.db.set_setting("computer_mode", "vm").await.unwrap();
    let mut rx = state.event_sender.subscribe();

    let marker = tmp.path().join("ran-on-host.txt");
    let cmd = format!("touch '{}'", marker.display());
    for team in [Some("no-such-team"), Some(""), None] {
        let res = kwnt::server::routes::execute_tool(&state, team, "execute_shell", &json!({ "command": cmd })).await;
        assert!(res.is_err(), "{team:?}: must be refused, got {res:?}");
        assert!(!marker.exists(), "{team:?}: command ran on the host");
    }
    assert!(std::fs::read_dir(&workspace).unwrap().next().is_none(), "host workspace must stay untouched");
    // refused outright: no approval prompt, since vm mode never offers host execution
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_restart_sandbox_stopped_when_docker_unavailable() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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
            assert!(!is_running);
            assert!(container_name.contains("team-restarted"));
        }
        other => panic!("Expected SandboxStatusChanged, got {:?}", other),
    }
}

