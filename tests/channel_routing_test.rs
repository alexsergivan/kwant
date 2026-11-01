//! A channel made by the person (`POST /api/channels`) or by a bot routes through Laya like the
//! rest: `routing_mode` defaults to `auto`, the lead is always one of the bots Laya picks from, and
//! a channel that starts with one bot gets Laya once `PATCH /api/channels/:id` adds members.
//! A fake claude CLI answers turns. Its own test binary: it changes PATH.

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
    db::{
        models::{NewChannel, NewMessage},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
    RuntimeEvent,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
echo '{"type":"result","is_error":false,"num_turns":1,"result":"answer ok"}'
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        exe::write_executable(&fake, FAKE_CLAUDE);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

async fn setup() -> (Router, AppState, tempfile::TempDir) {
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
    (create_router(state.clone()), state, tmp)
}

fn bot(name: &str) -> Value {
    json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
        "avatar_sprite": "s", "driver_type": "api", "preferred_model": "claude"})
}

fn members(c: &Value) -> Vec<String> {
    serde_json::from_str(c["member_kwants"].as_str().unwrap()).unwrap()
}

/// Sends a message to the channel; `Some(candidates)` when Laya routed it, `None` when it went
/// straight to the lead.
async fn routed(app: &Router, state: &AppState, channel: &str) -> Option<usize> {
    let mut events = state.event_sender.subscribe();
    ok(app, "POST", "/api/chat/send", Some(json!({"channel_id": channel, "prompt": "Who takes this?", "sender_id": "user"}))).await;
    // Waits for the turn to end, so the next message doesn't queue behind it
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut candidates = None;
        loop {
            match events.recv().await.map(|e| e.unscoped().clone()) {
                Ok(RuntimeEvent::RoutingDecided { candidate_count, .. }) => candidates = candidate_count,
                Ok(RuntimeEvent::TurnEnded { .. }) => return candidates,
                Err(e) => panic!("events: {e}"),
                _ => {}
            }
        }
    })
    .await
    .expect("the message was handled")
}

#[tokio::test]
async fn the_person_makes_a_channel_with_just_a_name_and_bots_and_laya_routes_it() {
    let (app, state, _tmp) = setup().await;
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();

    // No lead, no routing_mode: the first bot leads and Laya picks
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "member_kwants": [ann, bob]}))).await;
    assert_eq!(c["routing_mode"], "auto");
    assert_eq!(c["lead_kwant_id"], ann.as_str());
    assert_eq!(members(&c), vec![ann.clone(), bob.clone()]);
    assert_eq!(routed(&app, &state, c["id"].as_str().unwrap()).await, Some(2));

    // A lead plus one other member: the lead is a member too, so Laya has two to choose from
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "ops", "lead_kwant_id": ann, "member_kwants": [bob], "routing_mode": "auto"}))).await;
    assert_eq!(members(&c), vec![ann.clone(), bob.clone()], "the lead joins its own channel");
    assert_eq!(routed(&app, &state, c["id"].as_str().unwrap()).await, Some(2));

    // Refused: an unknown routing mode (it used to be stored, silently turning Laya off) and no bots
    let (s, v) = call(&app, "POST", "/api/channels", Some(json!({"name": "x", "member_kwants": [ann], "routing_mode": "whatever"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = call(&app, "POST", "/api/channels", Some(json!({"name": "y"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(state.db.list_channels(&SpaceId::default_space()).await.unwrap().iter().all(|c| c.name != "x" && c.name != "y"));
}

#[tokio::test]
async fn an_older_channel_whose_lead_isnt_listed_still_counts_the_lead() {
    let (app, state, _tmp) = setup().await;
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    // As the API used to store it: lead Ann, members [Bob]
    let old = NewChannel { name: "legacy".into(), description: None, lead_kwant_id: ann, member_kwants: vec![bob], routing_mode: "auto".into() };
    let c = state.db.create_channel(&SpaceId::default_space(), &old).await.unwrap();
    assert_eq!(routed(&app, &state, &c.id).await, Some(2));
}

#[tokio::test]
async fn a_one_bot_channel_gets_laya_once_members_are_added() {
    let (app, state, _tmp) = setup().await;
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();

    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "solo", "member_kwants": [ann]}))).await;
    let id = c["id"].as_str().unwrap().to_string();
    assert_eq!(routed(&app, &state, &id).await, None, "one bot: the lead answers, no routing");

    let c = ok(&app, "PATCH", &format!("/api/channels/{id}"), Some(json!({"member_kwants": [bob]}))).await;
    assert_eq!(members(&c), vec![ann.clone(), bob.clone()], "the lead stays in");
    assert_eq!(c["routing_mode"], "auto");
    assert_eq!(routed(&app, &state, &id).await, Some(2));

    // `lead` turns Laya off, `auto` back on; instructions in the same call are kept
    let c = ok(&app, "PATCH", &format!("/api/channels/{id}"), Some(json!({"routing_mode": "lead", "instructions": "Be brief."}))).await;
    assert_eq!(c["routing_mode"], "lead");
    assert_eq!(c["instructions"], "Be brief.");
    assert_eq!(routed(&app, &state, &id).await, None);
    ok(&app, "PATCH", &format!("/api/channels/{id}"), Some(json!({"routing_mode": "auto"}))).await;

    // Refused without changing anything
    let space = SpaceId::default_space();
    let general = state.db.general_channel(&space).await.unwrap().unwrap();
    let team = ok(&app, "POST", "/api/teams", Some(json!({"name": "core", "lead_kwant_id": ann, "member_kwants": [ann, bob]}))).await;
    let team_chat = state.db.team_channel(&space, team["name"].as_str().unwrap()).await.unwrap().expect("the team's chat");
    for (target, body) in [
        (id.clone(), json!({"routing_mode": "sometimes"})),
        (id.clone(), json!({"member_kwants": ["no-such-bot"]})),
        (general.id.clone(), json!({"member_kwants": [ann]})),
        (team_chat.clone(), json!({"member_kwants": [ann]})),
    ] {
        let (s, v) = call(&app, "PATCH", &format!("/api/channels/{target}"), Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}: {v}");
    }
    let c = state.db.get_channel(&id).await.unwrap().unwrap();
    assert_eq!((c.routing_mode.as_str(), serde_json::from_str::<Vec<String>>(&c.member_kwants).unwrap()), ("auto", vec![ann, bob]));
    let (s, _) = call(&app, "PATCH", "/api/channels/nope", Some(json!({"routing_mode": "auto"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// Sends `prompt` to the channel and returns who it was routed to and how.
async fn routed_to(app: &Router, state: &AppState, channel: &str, prompt: &str) -> (String, Option<String>) {
    let mut events = state.event_sender.subscribe();
    ok(app, "POST", "/api/chat/send", Some(json!({"channel_id": channel, "prompt": prompt, "sender_id": "user"}))).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut pick = None;
        loop {
            match events.recv().await.map(|e| e.unscoped().clone()) {
                Ok(RuntimeEvent::RoutingDecided { kwant_id, basis, .. }) => pick = Some((kwant_id, basis)),
                Ok(RuntimeEvent::TurnEnded { .. }) => return pick.expect("routed"),
                Err(e) => panic!("events: {e}"),
                _ => {}
            }
        }
    })
    .await
    .expect("the message was handled")
}

/// F3: the person's answer without an @mention goes back to the bot that asked, not to the lead
/// Laya's fallback would pick; an @mention still wins, and once the asker has replied it's over.
#[tokio::test]
async fn an_answer_goes_back_to_the_bot_that_asked_it() {
    let (app, state, _tmp) = setup().await;
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "lead_kwant_id": ann, "member_kwants": [ann, bob]}))).await;
    let cid = c["id"].as_str().unwrap().to_string();
    let say = |role: &'static str, who: String, text: &'static str| {
        let (db, cid) = (state.db.clone(), cid.clone());
        async move {
            db.insert_message(&NewMessage {
                channel_id: Some(cid),
                direct_kwant_id: None,
                sender_role: role.into(),
                sender_id: who,
                text: text.into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 0,
            })
            .await
            .unwrap();
        }
    };
    say("user", "user".into(), "Every morning send me an overview of my day").await;
    say("kwant", bob.clone(), "Happy to. **Which city are you in?**").await;
    assert_eq!(routed_to(&app, &state, &cid, "Munich").await, (bob.clone(), Some("asked".into())));

    // Bob answered ("answer ok", no question), so the next message is routed as usual
    let (to, basis) = routed_to(&app, &state, &cid, "Who takes this?").await;
    assert_ne!(basis.as_deref(), Some("asked"), "{to}");

    // An @mention wins over an open question
    say("kwant", bob.clone(), "Which day should it start?").await;
    let mut events = state.event_sender.subscribe();
    let sent = ok(&app, "POST", "/api/chat/send", Some(json!({"channel_id": cid, "prompt": "@Ann can you take over?", "sender_id": "user"}))).await;
    assert_eq!(sent["kwant"], "Ann", "{sent}");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !matches!(events.recv().await.map(|e| e.unscoped().clone()), Ok(RuntimeEvent::TurnEnded { .. })) {}
    })
    .await
    .expect("Ann's turn ended");
}

/// Rigel's #70 review: a closer like "Anything else?" isn't a question waiting for an answer, and a
/// plain "…?" gives way when the next message clearly belongs to another bot. Live HTTP path.
#[tokio::test]
async fn closers_and_clear_matches_dont_stick_to_the_asker() {
    let (app, state, _tmp) = setup().await;
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk2", "lead_kwant_id": ann, "member_kwants": [ann, bob]}))).await;
    let cid = c["id"].as_str().unwrap().to_string();
    let say = |role: &'static str, who: String, text: &'static str| {
        let (db, cid) = (state.db.clone(), cid.clone());
        async move {
            db.insert_message(&NewMessage {
                channel_id: Some(cid),
                direct_kwant_id: None,
                sender_role: role.into(),
                sender_id: who,
                text: text.into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 0,
            })
            .await
            .unwrap();
        }
    };
    say("user", "user".into(), "Summarise this week").await;
    say("kwant", bob.clone(), "Here's the summary.\n\nAnything else?").await;
    let (_, basis) = routed_to(&app, &state, &cid, "Plan my trip to Rome").await;
    assert_ne!(basis.as_deref(), Some("asked"), "a closer isn't a question");

    // A real plain question, but the next message names Ann: Ann takes it
    say("kwant", bob.clone(), "Which city are you in?").await;
    let (to, basis) = routed_to(&app, &state, &cid, "Ann, plan my trip to Rome").await;
    assert_eq!((to.as_str(), basis.as_deref()), (ann.as_str(), Some("keywords")));

    // A plain answer to it still goes back to Bob
    say("kwant", bob.clone(), "Which city are you in?").await;
    assert_eq!(routed_to(&app, &state, &cid, "Munich").await, (bob.clone(), Some("asked".into())));
}
