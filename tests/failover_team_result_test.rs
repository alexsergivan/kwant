//! 32m: when a teammate's turn on a hand-off runs out of usage and switches account, the result
//! card in the team chat (`delegation_result`) says which account it ran on, as `meta.failover`;
//! a hand-off with no switch has no such key. A fake claude CLI on PATH: its own test binary,
//! because it changes PATH and the process env.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Message, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;

/// A profile whose home has `out-of-usage` answers with claude's usage-limit error (reset at
/// `$FAKE_RESET`); any other replies "reply from <profile home>".
const FAKE_CLAUDE: &str = r#"#!/bin/sh
if [ "$1" = --settings ]; then shift 2; fi
case "$1" in
  auth) echo '{"loggedIn":true,"email":"fake@example.com"}'; exit 0 ;;
esac
if [ -f "$CLAUDE_CONFIG_DIR/out-of-usage" ]; then
  echo '{"type":"result","is_error":true,"num_turns":1,"result":"Claude AI usage limit reached|'"$FAKE_RESET"'"}'
  exit 1
fi
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"reply from $(basename "$CLAUDE_CONFIG_DIR")\"}"
"#;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn meta_of(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// Hands `task` from `lead` to `specialist` (the lead isn't resumed) and returns the result card
/// it posts in the lead's chat, once the hand-off has completed.
async fn result_card(app: &Router, state: &AppState, lead: &str, specialist: &str, task: &str) -> Message {
    let (s, d) = call(
        app,
        "POST",
        "/api/delegations",
        Some(json!({
            "lead_kwant_id": lead,
            "specialist_kwant_id": specialist,
            "task": task,
            "title": task,
            "kwant_id": lead,
            "continue_lead": false,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    let id = d["id"].as_str().unwrap().to_string();
    // Settles on the card itself: `finish` posts it after the hand-off's row has ended
    for _ in 0..600 {
        let msgs = state.db.list_messages(lead, 50).await.unwrap();
        if let Some(card) = msgs.into_iter().find(|m| {
            let meta = meta_of(m);
            meta["kind"] == "delegation_result" && meta["delegation_id"] == id.as_str()
        }) {
            let (_, row) = call(app, "GET", &format!("/api/delegations/{id}"), None).await;
            assert_eq!(row["status"], "completed", "{row}");
            return card;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, row) = call(app, "GET", &format!("/api/delegations/{id}"), None).await;
    panic!("no result card for hand-off {id}: {row}");
}

#[tokio::test]
async fn result_card_names_the_account_a_teammate_switched_to() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    std::env::remove_var("ANTHROPIC_API_KEY");
    let reset = chrono::Utc::now().timestamp() + 2 * 3600;
    std::env::set_var("FAKE_RESET", reset.to_string());

    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // A plain hand-off: no mission board and no keep-going turns around it
    db.set_setting("mission_board", "false").await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    // Laya is unreachable: turns use the kwant's preferred model, the fake claude
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());

    // Two claude accounts; the first is out of usage
    let (_, work) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Work", "kind": "cli_home", "is_default": true, "priority": 1})),
    )
    .await;
    let (_, personal) =
        call(&app, "POST", "/api/profiles", Some(json!({"engine": "claude", "name": "Personal", "kind": "cli_home", "priority": 2}))).await;
    let (s, _) = call(&app, "PATCH", "/api/profiles/prf_system_claude", Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);
    std::fs::write(std::path::Path::new(work["home_dir"].as_str().unwrap()).join("out-of-usage"), "").unwrap();
    let personal_id = personal["id"].as_str().unwrap().to_string();

    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let mut ids = Vec::new();
    for k in kwants.as_array().unwrap().iter().take(2) {
        let id = k["id"].as_str().unwrap().to_string();
        let mut body = k.clone();
        body.as_object_mut()
            .unwrap()
            .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
        body["preferred_model"] = json!("claude");
        let (s, v) = call(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        ids.push(id);
    }
    let (lead, specialist) = (&ids[0], &ids[1]);

    // 1. The teammate's turn runs out of usage on Work and finishes on Personal: its result card
    //    names the switch
    let card = result_card(&app, &state, lead, specialist, "Research the market").await;
    assert_eq!(card.text, format!("reply from {personal_id}"), "the result text stays the teammate's reply");
    let meta = meta_of(&card);
    let failover = &meta["failover"];
    assert_eq!(failover["to_name"], "Personal", "{meta}");
    assert_eq!(failover["from_name"], "Work", "{meta}");
    assert_eq!(failover["class"], "quota", "{meta}");
    let retry_at: chrono::DateTime<chrono::Utc> = failover["retry_at"].as_str().expect("retry_at").parse().unwrap();
    assert_eq!(retry_at.timestamp(), reset);
    // Only the four fields the card needs: never the switch notice's own text
    let mut keys: Vec<&str> = failover.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["class", "from_name", "retry_at", "to_name"], "{meta}");
    // The rest of the card's meta is as before
    assert_eq!(meta["status"], "completed");
    assert_eq!(meta["specialist_id"], specialist.as_str());

    // 2. Work is now known to be out of usage, so the next hand-off runs on Personal from the
    //    start: no switch, so no `failover` key on its card
    let card = result_card(&app, &state, lead, specialist, "Write the summary").await;
    assert_eq!(card.text, format!("reply from {personal_id}"));
    let meta = meta_of(&card);
    assert!(meta.get("failover").is_none(), "no switch, no key: {meta}");
    assert_eq!(meta["kind"], "delegation_result");
}
