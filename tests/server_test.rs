use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kwnt::{
    config::Config,
    db::Database,
    server::{create_router, AppState},
    tools::ToolRunner,
    broker::PermissionBroker,
    router::LayaRouter,
};
use serde_json::json;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_server_health_and_kwants() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    // 1. Health check
    let req = Request::builder()
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. List Kwants
    let req = Request::builder()
        .uri("/api/kwants")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. Send chat message
    let send_body = json!({
        "channel_id": null,
        "kwant_id": null,
        "prompt": "Hello 8-bit realm!",
        "sender_id": "player1"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/send")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&send_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
