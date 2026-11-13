//! In a team discussion, the teammate the speaker @mentions goes next, before the planned
//! reactors (Mira's UX audit #4). A 3-bot team runs a real discussion through the server on a
//! fake claude CLI: Ann leads, Bob and Cody are the planned reactors, and Ann's opening asks
//! @Cody for the data. Cody must speak second, answering Ann, then Bob, then Ann's summary.
//! Its own test binary: it changes PATH.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
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
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// - the opening: asks @Cody for the numbers;
/// - a teammate the speaker asked directly: hands over the data;
/// - any other reply: reacts;
/// - the wrap-up: a plain summary.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *"opening a team discussion"*) out='Blue-green cutover. @Cody please get the current table sizes first.' ;;
  *"Wrap up the team discussion"*) out='Summary: blue-green, sized from the table numbers.' ;;
  *"asked you directly"*) out='Here are the table sizes: orders 40 GB, users 2 GB.' ;;
  *"Team discussion in"*) out='Reacting: add a rollback window.' ;;
  *) out="answer" ;;
esac
printf '%s\n' "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"$out\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":20}}"
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        exe::write_executable(&bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

#[tokio::test]
async fn the_teammate_the_lead_mentions_speaks_before_the_planned_reactors() {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let bot = |name: &str| {
        json!({"name": name, "title": format!("{name} the engineer"), "persona_prompt": "Test persona",
            "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"})
    };
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    let cody = ok(&app, "POST", "/api/kwants", Some(bot("Cody"))).await["id"].as_str().unwrap().to_string();
    let team = ok(
        &app,
        "POST",
        "/api/teams",
        Some(json!({"name": "infra", "lead_kwant_id": ann, "member_kwants": [ann, bob, cody]})),
    )
    .await;
    let channels = ok(&app, "GET", "/api/channels", None).await;
    let channel = channels
        .as_array()
        .or_else(|| channels["channels"].as_array())
        .unwrap()
        .iter()
        .find(|c| c["name"] == "team-infra")
        .expect("the team's chat")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut events = state.event_sender.subscribe();
    let sent = ok(
        &app,
        "POST",
        "/api/chat/send",
        Some(json!({"channel_id": channel, "team_id": team["id"], "sender_id": "user",
            "prompt": "How should we design the zero-downtime database migration architecture? Weigh the tradeoff."})),
    )
    .await;
    assert_eq!(sent["status"], "dispatched", "{sent}");

    // Every turn as the server announced it, until the discussion completes
    let mut planned: Vec<String> = Vec::new();
    let mut turns: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout_at(deadline, events.recv()).await.expect("discussion completes").unwrap();
        match event {
            RuntimeEvent::DiscussionStarted { speakers, .. } => {
                let name = |id: &String| [(&ann, "Ann"), (&bob, "Bob"), (&cody, "Cody")].iter().find(|(i, _)| *i == id).unwrap().1;
                planned = speakers.iter().map(|id| name(id).to_string()).collect();
            }
            RuntimeEvent::DiscussionTurnStarted { speaker_name, turn_index, total_turns, asked_by, phase, .. } => {
                turns.push(format!(
                    "{turn_index}/{total_turns} {speaker_name} ({}){}",
                    phase.unwrap_or_default(),
                    asked_by.map(|a| format!(" asked by {a}")).unwrap_or_default()
                ));
            }
            RuntimeEvent::DiscussionCompleted { .. } => break,
            _ => {}
        }
    }
    println!("planned: {planned:?}\nturns:\n  {}", turns.join("\n  "));

    let messages = state.db.list_messages(&channel, 100).await.unwrap();
    let order: Vec<(&str, &str)> = messages
        .iter()
        .filter(|m| m.sender_role == "kwant")
        .map(|m| {
            let who = if m.sender_id == ann { "Ann" } else if m.sender_id == bob { "Bob" } else { "Cody" };
            (who, m.text.as_str())
        })
        .collect();
    println!("chat:\n  {}", order.iter().map(|(w, t)| format!("{w}: {t}")).collect::<Vec<_>>().join("\n  "));

    assert_eq!(planned, ["Ann", "Bob", "Cody"], "the router plans Ann, then Bob and Cody");
    assert_eq!(
        turns,
        [
            "1/4 Ann (open)",
            "2/4 Cody (reply) asked by Ann",
            "3/4 Bob (reply)",
            "4/4 Ann (summary)",
        ],
        "Cody, whom Ann asked, goes before Bob"
    );
    let who: Vec<&str> = order.iter().map(|(w, _)| *w).collect();
    assert_eq!(who, ["Ann", "Cody", "Bob", "Ann"], "{order:#?}");
    assert!(order[1].1.starts_with("Here are the table sizes"), "Cody answers Ann: {order:#?}");
}
