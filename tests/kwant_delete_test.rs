//! The person deleting any bot (`DELETE /api/kwants/:id`): its direct chat, threads, memories,
//! skills, routines and memberships go with it; a channel or team it led gets another lead;
//! refused while it works, for a space's only bot, and for a bot's own token.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::NewMessage, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{create_router, turns::ActiveTurn, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
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

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    call_as(app, method, uri, None, body).await
}

/// With the person's API token (deleting a bot is person-only).
async fn call_as(app: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn dm(kwant: &str, role: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: None,
        direct_kwant_id: Some(kwant.into()),
        sender_role: role.into(),
        sender_id: if role == "user" { "user".into() } else { kwant.into() },
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    }
}

#[tokio::test]
async fn the_person_deletes_any_bot_with_its_history() {
    let (app, state, _tmp) = setup().await;
    let space = SpaceId::default_space();
    let bots = state.db.list_kwants(&space).await.unwrap();
    assert!(bots.len() >= 2, "the default space has several bots");
    let (bot, other) = (bots[0].clone(), bots[1].clone());

    // A bot with history: a direct chat, a thread, a memory, a skill, a routine, and it leads a channel
    state.db.insert_message(&dm(&bot.id, "user", "hello")).await.unwrap();
    state.db.insert_message(&dm(&bot.id, "kwant", "hi")).await.unwrap();
    let thread = state.db.create_thread(&bot.id, "Strategy", "chat", None, "user").await.unwrap();
    state.db.add_memory(&bot.id, "Alex trades in Go.", "user").await.unwrap();
    let (s, v) = call(&app, "POST", "/api/skills", Some(json!({"name": "backtest", "description": "How to backtest", "body": "1. run", "kwant_id": bot.id}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let routine = json!({"kwant_id": bot.id, "name": "Digest", "instructions": "Summarise.", "schedule": {"type": "interval", "every_minutes": 60}});
    let (s, v) = call(&app, "POST", "/api/routines", Some(routine)).await;
    assert!(s.is_success(), "{s} {v}");
    let (s, v) = call(&app, "POST", "/api/channels", Some(json!({"name": "desk", "description": null, "routing_mode": "auto", "lead_kwant_id": bot.id, "member_kwants": [bot.id, other.id]}))).await;
    assert!(s.is_success(), "{s} {v}");
    let desk = v["id"].as_str().unwrap().to_string();

    // The confirm dialog's counts
    let (s, usage) = call(&app, "GET", &format!("/api/kwants/{}/usage", bot.id), None).await;
    assert_eq!(s, StatusCode::OK, "{usage}");
    assert_eq!(usage["conversations"], 2, "{usage}");
    assert_eq!(usage["memories"], 1);
    assert_eq!(usage["skills"], 1);
    assert_eq!(usage["routines"], 1);
    assert_eq!(usage["running"], false);

    // While it works: 409 running, nothing removed
    state.turns.set_turn(ActiveTurn {
        turn_id: "t1".into(),
        kwant_id: bot.id.clone(),
        context_id: "c".into(),
        thread_id: None,
        state: "running".into(),
        since: chrono::Utc::now(),
        purpose: None,
    });
    let (s, v) = call_as(&app, "DELETE", &format!("/api/kwants/{}", bot.id), Some(state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["error"], "running");
    assert!(v["message"].as_str().unwrap().contains("working"));
    assert!(state.db.get_kwant(&bot.id).await.unwrap().is_some());
    state.turns.end_turn("t1");

    // A bot's own token can't delete bots
    let (s, _) = {
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/api/kwants/{}", bot.id))
            .header("authorization", "Bearer kwnt_turn_not_the_person");
        let resp = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        (resp.status(), ())
    };
    assert!(s == StatusCode::FORBIDDEN || s == StatusCode::UNAUTHORIZED, "{s}");

    // Deleted, with everything it owned
    let (s, v) = call_as(&app, "DELETE", &format!("/api/kwants/{}", bot.id), Some(state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], json!(bot.id));
    assert_eq!(v["status"], json!("deleted"));
    assert!(state.db.get_kwant(&bot.id).await.unwrap().is_none());
    assert!(state.db.list_messages(&bot.id, 50).await.unwrap().is_empty());
    assert!(state.db.get_thread(&thread.id).await.unwrap().is_none());
    let (_, usage) = call(&app, "GET", &format!("/api/kwants/{}/usage", bot.id), None).await;
    assert!(usage.get("memories").is_none(), "unknown bot now: {usage}");
    let (_, skills) = call(&app, "GET", "/api/skills", None).await;
    assert!(!skills.to_string().contains("backtest"), "{skills}");
    let (_, routines) = call(&app, "GET", "/api/routines", None).await;
    assert!(!routines.to_string().contains(&bot.id), "{routines}");
    // The channel it led goes to its next member; memberships are gone
    let desk = state.db.get_channel(&desk).await.unwrap().unwrap();
    assert_eq!(desk.lead_kwant_id, other.id);
    assert!(!desk.member_kwants.contains(&bot.id));
    for t in state.db.list_teams(&space).await.unwrap() {
        assert_ne!(t.lead_kwant_id, bot.id);
        assert!(!t.member_kwants.contains(&bot.id));
    }
    let (s, _) = call_as(&app, "DELETE", &format!("/api/kwants/{}", bot.id), Some(state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // A space's only bot stays
    let rest = state.db.list_kwants(&space).await.unwrap();
    for k in &rest[1..] {
        let (s, v) = call_as(&app, "DELETE", &format!("/api/kwants/{}", k.id), Some(state.api_token.reveal()), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (s, v) = call_as(&app, "DELETE", &format!("/api/kwants/{}", rest[0].id), Some(state.api_token.reveal()), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["error"], "last_bot");
}
