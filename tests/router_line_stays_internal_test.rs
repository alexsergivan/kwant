//! 32d router-line-stays-internal: every turn broadcast "🔮 Laya Router: model='…', effort='…'
//! (N% conf) — …" as a thinking line, and a team-routed message "🔮 Laya Team Router: Routed to …".
//! The live reply card showed the first as its status line until "Running on …" replaced it (Mira
//! caught it in the 0.9.35 kwant.si clips). Both are debug lines for the server log: no event, no
//! wire frame and no saved message of a direct-chat turn or a team-routed channel turn carries
//! them. 36h plain-words: "Running on Claude Code CLI · This machine · sonnet (balanced tier)" went
//! the same way, so a normal turn's stream has no "Running on" line either. A fake claude CLI
//! answers turns. Its own test binary: it changes PATH.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use kwnt::{
    broker::PermissionBroker, config::Config, db::Database, router::LayaRouter, server::create_router, server::AppState,
    tools::ToolRunner, RuntimeEvent,
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

/// What only the router's debug lines say.
const ROUTER_LINES: [&str; 3] = ["Laya Router", "Laya Team Router", "🔮"];

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
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

fn bot(name: &str) -> Value {
    json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
        "avatar_sprite": "s", "driver_type": "api", "preferred_model": "claude"})
}

/// Sends `body` to /api/chat/send and returns every event up to the turn's end, as the server
/// broadcast it (tagged with its turn, the way the live stream gets it).
async fn turn_events(app: &Router, state: &AppState, body: Value) -> Vec<RuntimeEvent> {
    let mut rx = state.event_sender.subscribe();
    ok(app, "POST", "/api/chat/send", Some(body)).await;
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let e = rx.recv().await.expect("events");
            let end = matches!(e.unscoped(), RuntimeEvent::TurnEnded { .. });
            events.push(e);
            if end {
                return;
            }
        }
    })
    .await
    .expect("the turn ended");
    events
}

/// The router lines in what reached the event bus: each event as the live stream sends it.
fn leaks(events: &[RuntimeEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| e.to_wire().to_string())
        .filter(|wire| ROUTER_LINES.iter().any(|l| wire.contains(l)))
        .collect()
}

/// The router lines in a chat's saved messages (text and reasoning, as the chat API returns them).
async fn saved_leaks(app: &Router, state: &AppState, context_id: &str) -> Vec<String> {
    let mut out: Vec<String> = state
        .db
        .list_messages(context_id, 100)
        .await
        .unwrap()
        .into_iter()
        .flat_map(|m| [Some(m.text), m.reasoning_content])
        .flatten()
        .filter(|t| ROUTER_LINES.iter().any(|l| t.contains(l)))
        .collect();
    let api = ok(app, "GET", &format!("/api/channels/{context_id}/messages"), None).await.to_string();
    if ROUTER_LINES.iter().any(|l| api.contains(l)) {
        out.push(api);
    }
    out
}

#[tokio::test]
async fn no_turn_shows_the_router_line_to_a_person() {
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
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();

    // A direct chat: the turn runs and replies, and nothing on the way says how it was routed
    let events = turn_events(&app, &state, json!({"kwant_id": ann, "prompt": "hi", "sender_id": "user"})).await;
    assert!(
        events.iter().any(|e| matches!(e.unscoped(), RuntimeEvent::TextDelta(_) | RuntimeEvent::TurnComplete { .. })),
        "the turn ran"
    );
    assert!(
        !events.iter().any(|e| matches!(e.unscoped(), RuntimeEvent::ThinkingDelta(t) if t.starts_with("Running on") || t.contains(" tier)"))),
        "36h: what the turn runs on stays in the server log, not the live status line"
    );
    assert_eq!(leaks(&events), Vec::<String>::new(), "direct chat: the router line reached the live stream");
    assert_eq!(saved_leaks(&app, &state, &ann).await, Vec::<String>::new(), "direct chat: the router line was saved");

    // A channel Laya routes between two bots: the pick is a RoutingDecided event, never a thinking line
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "member_kwants": [ann, bob]}))).await;
    let cid = c["id"].as_str().unwrap().to_string();
    let events = turn_events(&app, &state, json!({"channel_id": cid, "prompt": "Who takes this?", "sender_id": "user"})).await;
    assert!(events.iter().any(|e| matches!(e.unscoped(), RuntimeEvent::RoutingDecided { .. })), "Laya routed it");
    assert_eq!(leaks(&events), Vec::<String>::new(), "channel: a router line reached the live stream");
    assert_eq!(saved_leaks(&app, &state, &cid).await, Vec::<String>::new(), "channel: a router line was saved");
}
