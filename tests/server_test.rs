use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Team, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
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

#[tokio::test]
async fn test_teams_api_endpoints() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db.clone(), tools, broker, router);
    let app = create_router(state);

    // 1. GET /api/teams - Should return seeded team
    let req = Request::builder()
        .uri("/api/teams")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let teams: Vec<Team> = serde_json::from_slice(&bytes).unwrap();
    assert!(!teams.is_empty());
    assert_eq!(teams[0].name, "core-engineering");
    let team_id = teams[0].id.clone();

    // 2. GET /api/teams/:id
    let req = Request::builder()
        .uri(format!("/api/teams/{}", team_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. POST /api/teams
    let kwants = db.list_kwants().await.unwrap();
    let new_team_payload = json!({
        "name": "qa-squad",
        "description": "Quality assurance squad",
        "icon": "shield",
        "lead_kwant_id": kwants[0].id,
        "member_kwants": vec![kwants[0].id.clone(), kwants[1].id.clone()]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/teams")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&new_team_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let created_team: Team = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(created_team.name, "qa-squad");

    // 4. DELETE /api/teams/:id
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/teams/{}", created_team.id))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. POST /api/chat/cancel
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/cancel")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_chat_routing_with_kwant_mention() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    let send_body = json!({
        "channel_id": null,
        "kwant_id": null,
        "team_id": null,
        "prompt": "@DevOps please check system health status",
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
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["status"], "dispatched");
    assert_eq!(val["kwant"], "DevOps");
}

#[tokio::test]
async fn test_chat_routing_with_team_mention_and_cancellation() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config::default();
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state.clone());

    let mut event_rx = state.event_sender.subscribe();

    // Send complex discussion query to @core-engineering
    let send_body = json!({
        "channel_id": null,
        "kwant_id": null,
        "team_id": null,
        "prompt": "Hey @core-engineering, how should we design our zero-downtime database migration architecture tradeoff?",
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
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["status"], "dispatched");
    assert_eq!(val["team"], "core-engineering");

    // Receive discussion started event (may follow ThinkingDelta)
    let mut started_event = None;
    for _ in 0..5 {
        if let Ok(Ok(event)) = tokio::time::timeout(std::time::Duration::from_millis(500), event_rx.recv()).await {
            if matches!(event, kwnt::drivers::events::RuntimeEvent::DiscussionStarted { .. }) {
                started_event = Some(event);
                break;
            }
        }
    }
    match started_event.expect("Expected DiscussionStarted event") {
        kwnt::drivers::events::RuntimeEvent::DiscussionStarted { team_name, total_turns, .. } => {
            assert_eq!(team_name, "core-engineering");
            assert!(total_turns >= 2);
        }
        _ => unreachable!(),
    }

    // Cancel in-flight discussion
    let cancel_body = json!({ "team_id": "core-engineering" });
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/cancel")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&cancel_body).unwrap()))
        .unwrap();
    let cancel_resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(cancel_resp.status(), StatusCode::OK);
}

