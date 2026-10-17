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
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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

    // 4. PUT /api/teams/:id (Update team)
    let update_payload = json!({
        "name": "qa-squad",
        "description": "Updated QA and reliability team",
        "icon": "shield",
        "lead_kwant_id": kwants[1].id,
        "member_kwants": vec![kwants[1].id.clone()]
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/teams/{}", created_team.id))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let updated_team: Team = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(updated_team.description.as_deref(), Some("Updated QA and reliability team"));

    // 5. DELETE /api/teams/:id
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
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
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

#[tokio::test]
async fn test_cancel_chat_with_null_fields_only_cancels_target_kwant() {
    use tokio_util::sync::CancellationToken;

    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state.clone());

    // Insert two distinct tokens in active_cancellations
    let target_token = CancellationToken::new();
    let other_token = CancellationToken::new();
    {
        let mut tokens = state.active_cancellations.lock().await;
        tokens.insert("test-kwant".to_string(), target_token.clone());
        tokens.insert("other-session".to_string(), other_token.clone());
    }

    // Cancel with payload containing null channel_id and team_id, but specific kwant_id
    let cancel_body = json!({
        "channel_id": null,
        "kwant_id": "test-kwant",
        "team_id": null
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/cancel")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&cancel_body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Target token must be cancelled
    assert!(target_token.is_cancelled(), "Target kwant token should be cancelled");

    // Other token must NOT be cancelled (no global drain!)
    assert!(!other_token.is_cancelled(), "Other session token must NOT be cancelled by targeted cancellation");

    // active_cancellations map must still contain other_token and not test-kwant
    let tokens = state.active_cancellations.lock().await;
    assert!(!tokens.contains_key("test-kwant"));
    assert!(tokens.contains_key("other-session"));
}

#[tokio::test]
async fn test_system_directories_endpoint() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    // Create test subdirectories inside tmp
    std::fs::create_dir_all(tmp.path().join("project-alpha")).unwrap();
    std::fs::create_dir_all(tmp.path().join("project-beta")).unwrap();
    std::fs::write(tmp.path().join("some-file.txt"), "hello").unwrap();

    let uri = format!("/api/system/directories?path={}", tmp.path().display());
    let req = Request::builder()
        .uri(&uri)
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let dirs = json["directories"].as_array().expect("directories array");
    let dir_names: Vec<&str> = dirs.iter().filter_map(|d| d.as_str()).collect();

    assert!(dir_names.contains(&"project-alpha"));
    assert!(dir_names.contains(&"project-beta"));
    assert!(!dir_names.contains(&"some-file.txt"), "Files should not be listed as directories");
    assert!(json["current_path"].is_string());
    assert!(json["quick_paths"].is_array());
}

#[tokio::test]
async fn test_system_select_folder_endpoint() {
    std::env::set_var("KWNT_TEST_MODE", "1");
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/system/select-folder")
        .body(Body::empty())
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("canceled").is_some());
}

#[tokio::test]
async fn test_sandbox_password_vault_api() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());
    let kwants = db.list_kwants().await.unwrap();
    let team = db
        .create_team(&kwnt::db::models::NewTeam {
            name: "test-team-vault".into(),
            description: Some("Team for vault testing".into()),
            icon: None,
            lead_kwant_id: kwants[0].id.clone(),
            member_kwants: vec![kwants[0].id.clone()],
            workspace_path: None,
            repo_url: None,
        })
        .await
        .unwrap();
    let team_id = team.id;

    let state = AppState::new(config, db, tools, broker, router);
    let app = create_router(state);

    // 1. Initial list should be empty
    let req = Request::builder()
        .uri(format!("/api/sandbox/{}/passwords", team_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["passwords"].as_array().unwrap().len(), 0);

    // 2. Save a credential
    let save_payload = json!({
        "domain": "example.com",
        "origin": "https://example.com",
        "username": "alex",
        "password": "supersecretpassword"
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{}/passwords", team_id))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&save_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "saved");
    let cred_id = json["credential"]["id"].as_str().unwrap().to_string();
    assert_eq!(json["credential"]["username"], "alex");

    // 3. List should now contain 1 item
    let req = Request::builder()
        .uri(format!("/api/sandbox/{}/passwords", team_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let passwords = json["passwords"].as_array().unwrap();
    assert_eq!(passwords.len(), 1);
    assert_eq!(passwords[0]["domain"], "example.com");
    assert_eq!(passwords[0]["username"], "alex");
    assert_eq!(passwords[0]["password"], "supersecretpassword");

    // 4. Delete the credential
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/sandbox/{}/passwords/{}", team_id, cred_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. List should be empty again
    let req = Request::builder()
        .uri(format!("/api/sandbox/{}/passwords", team_id))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["passwords"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_sandbox_interactive_control_endpoint() {
    use futures::{SinkExt, StreamExt};
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let commands = recorded.clone();
    let ws_task = tokio::spawn(async move {
        let (stream, _) = ws_listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = ws.next().await {
            let command: serde_json::Value = serde_json::from_str(&text).unwrap();
            let result = if command["method"] == "Page.captureScreenshot" {
                json!({ "data": "fresh-screenshot" })
            } else {
                json!({})
            };
            let reply = json!({ "id": command["id"], "result": result });
            commands.lock().await.push(command);
            ws.send(tokio_tungstenite::tungstenite::Message::Text(reply.to_string())).await.unwrap();
        }
    });
    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = http_listener.local_addr().unwrap().port();
    let cdp_app = axum::Router::new().route("/json/list", axum::routing::get(move || async move {
        axum::Json(json!([{
            "type": "page",
            "webSocketDebuggerUrl": format!("ws://127.0.0.1:{ws_port}/devtools/page/test")
        }]))
    }));
    let http_task = tokio::spawn(async move { axum::serve(http_listener, cdp_app).await.unwrap(); });

    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());

    let state = AppState::new(config, db, tools, broker, router);
    let mut event_rx = state.event_sender.subscribe();
    state.browser_controllers.lock().await.insert(
        "test-team-interactive".into(), kwnt::sandbox::BrowserController::new(port),
    );
    let app = create_router(state);

    let team_id = "test-team-interactive";

    // 1. Click interaction
    let click_payload = json!({
        "action": "click",
        "x": 350,
        "y": 180
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{}/interact", team_id))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&click_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["action"], "click");
    assert_eq!(json["screenshot"], "fresh-screenshot");

    // Verify SSE event was emitted
    let ev = event_rx.recv().await.unwrap();
    match ev {
        kwnt::drivers::events::RuntimeEvent::BrowserScreenshot { team_id: tid, .. } => {
            assert_eq!(tid, team_id);
        }
        _ => panic!("Expected BrowserScreenshot event"),
    }

    // 2. Type interaction
    let type_payload = json!({
        "action": "type",
        "text": "admin@kwnt.dev"
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{}/interact", team_id))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&type_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. Key press interaction
    let key_payload = json!({
        "action": "key",
        "key": "Enter"
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{}/interact", team_id))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&key_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Scroll interaction
    let scroll_payload = json!({
        "action": "scroll",
        "x": 640,
        "y": 400,
        "delta_y": 120.0
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{}/interact", team_id))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&scroll_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let commands = recorded.lock().await;
    let mouse_events: Vec<_> = commands.iter().filter(|c| c["method"] == "Input.dispatchMouseEvent").collect();
    assert_eq!(mouse_events[0]["params"]["type"], "mousePressed");
    assert_eq!(mouse_events[1]["params"]["type"], "mouseReleased");
    for event in &mouse_events[..2] {
        assert_eq!(event["params"]["x"], 350);
        assert_eq!(event["params"]["y"], 180);
    }
    http_task.abort();
    ws_task.abort();
}

#[tokio::test]
async fn test_sandbox_interaction_errors_are_not_reported_as_success() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(), db, ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()),
    );
    let mut events = state.event_sender.subscribe();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let ctrl = kwnt::sandbox::BrowserController::new(port);
    *ctrl.last_screenshot.lock().await = Some("stale-image".into());
    state.browser_controllers.lock().await.insert("broken-browser".into(), ctrl);
    let app = create_router(state);

    for (team, payload, expected) in [
        ("missing-browser", json!({"action": "click", "x": 20, "y": 30}), StatusCode::SERVICE_UNAVAILABLE),
        ("broken-browser", json!({"action": "click", "x": 20, "y": 30}), StatusCode::BAD_GATEWAY),
        ("missing-browser", json!({"action": "unknown"}), StatusCode::BAD_REQUEST),
        ("missing-browser", json!({"action": "click"}), StatusCode::BAD_REQUEST),
        ("missing-browser", json!({"action": "click", "x": -1, "y": 30}), StatusCode::BAD_REQUEST),
        ("missing-browser", json!({"action": "key", "key": ""}), StatusCode::BAD_REQUEST),
    ] {
        let req = Request::builder().method("POST")
            .uri(format!("/api/sandbox/{team}/interact"))
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string())).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), expected);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["error"].is_string());
        assert!(body.get("screenshot").is_none());
        assert!(events.try_recv().is_err(), "failed input must not broadcast a stale screenshot");
    }
    for (team, expected) in [
        ("missing-browser", StatusCode::SERVICE_UNAVAILABLE),
        ("broken-browser", StatusCode::BAD_GATEWAY),
    ] {
        for endpoint in ["navigate", "screenshot"] {
            let (method, body) = if endpoint == "navigate" {
                ("POST", Body::from(json!({"url": "https://example.com"}).to_string()))
            } else {
                ("GET", Body::empty())
            };
            let req = Request::builder().method(method)
                .uri(format!("/api/sandbox/{team}/{endpoint}"))
                .header("content-type", "application/json").body(body).unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), expected);
            let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(body["error"].is_string());
            assert!(body.get("screenshot").is_none());
            assert!(events.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn test_url_extraction_and_relative_navigation() {
    use kwnt::server::routes::{detect_url_in_prompt, detect_url_with_context, extract_url_from_text};

    // 1. Modern TLD (.ing) extraction
    let url = extract_url_from_text("please open booktun.ing for me");
    assert_eq!(url, Some("https://booktun.ing".to_string()));

    // 2. Code extensions must not be treated as URLs
    assert_eq!(extract_url_from_text("read src/main.rs"), None);
    assert_eq!(extract_url_from_text("check file.json"), None);

    // 3. Direct URL in prompt detection
    assert_eq!(
        detect_url_in_prompt("open booktun.ing"),
        Some("https://booktun.ing".to_string())
    );

    // 4. Relative navigation with active browser context
    let rel_playlist = detect_url_with_context(
        "open some playlist page",
        Some("https://booktun.ing"),
    );
    assert_eq!(rel_playlist, Some("https://booktun.ing/playlists".to_string()));

    let rel_path = detect_url_with_context(
        "navigate to /explore",
        Some("https://booktun.ing"),
    );
    assert_eq!(rel_path, Some("https://booktun.ing/explore".to_string()));

    // 5. Explicit navigation to another site overrides active browser context
    let override_url = detect_url_with_context(
        "open youtube.com",
        Some("https://booktun.ing"),
    );
    assert_eq!(override_url, Some("https://youtube.com".to_string()));
}

#[tokio::test]
async fn test_channel_sandbox_target_resolution() {
    use kwnt::server::routes::resolve_sandbox_target_info;

    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let broker = PermissionBroker::new();
    let router = LayaRouter::new(config.laya_url.clone());
    let state = AppState::new(config, db.clone(), tools, broker, router);

    let channels = db.list_channels().await.unwrap();
    let general_channel = channels.iter().find(|c| c.name == "general").expect("Seeded general channel");

    let resolved = resolve_sandbox_target_info(&state, &general_channel.id).await;
    assert!(resolved.is_some());
    let (target_name, ws_path) = resolved.unwrap();
    assert_eq!(target_name, "chan-general");
    assert!(ws_path.to_string_lossy().ends_with("workspaces/chan-general"));
}




#[tokio::test]
async fn test_kwant_name_validation() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let state = AppState::new(config.clone(), db, tools, PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let app = create_router(state);

    let send = |method: &str, uri: &str, name: &str| {
        let body = json!({
            "name": name, "title": "t", "persona_prompt": "p",
            "avatar_sprite": "designer", "driver_type": "api", "preferred_model": "auto"
        });
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    };

    // Seeded "Architect" exists, so a case-insensitive duplicate is rejected
    let resp = app.clone().oneshot(send("POST", "/api/kwants", "architect")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let resp = app.clone().oneshot(send("POST", "/api/kwants", "Ux Designer")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let resp = app.clone().oneshot(send("POST", "/api/kwants", "  Designer ")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(created["name"], "Designer");
    let id = created["id"].as_str().unwrap();

    // Keeping its own name is fine; taking another kwant's isn't
    let resp = app.clone().oneshot(send("PUT", &format!("/api/kwants/{id}"), "Designer")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app.clone().oneshot(send("PUT", &format!("/api/kwants/{id}"), "TechLead")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
