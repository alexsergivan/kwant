use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_embedded_assets_served() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    // Test GET / returns index.html
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body_bytes);
    assert!(html.contains("KWNTS") || html.contains("root"));
}

#[tokio::test]
async fn test_sandbox_e2e_endpoints() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    // Test GET /api/sandbox/test-team/status
    let req = Request::builder()
        .uri("/api/sandbox/test-team/status")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let status_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(status_json["is_running"], false);
    assert_eq!(status_json["team_id"], "test-team");

    // Test GET /api/sandbox/test-team/screenshot
    let req = Request::builder()
        .uri("/api/sandbox/test-team/screenshot")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let shot_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let shot_json: serde_json::Value = serde_json::from_slice(&shot_bytes).unwrap();
    assert_eq!(shot_json["team_id"], "test-team");
    assert!(shot_json["screenshot"].is_string());
}


