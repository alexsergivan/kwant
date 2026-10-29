//! A bot's `[DECISIONS]` block in a team channel comes off the saved text into `meta.decisions`
//! (choice chips), as in direct chats, and a garbled block is never shown raw. A fake claude CLI
//! answers by keyword. Its own test binary: it changes PATH.

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
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// - the opening of a team discussion: a reply with a well-formed block;
/// - a reply in the discussion: the screenshot's block, two choices run together on one line and
///   a line that doesn't parse, lowercase tags, no closing tag;
/// - the wrap-up: a plain summary;
/// - anything else: "answer".
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *"opening a team discussion"*)
    out='Go with a blue-green cutover. @Bob what do you think?\n\n[DECISIONS]\nquestion: Cutover style? | options: Blue-green; Rolling | picked: Blue-green | why: instant rollback\n[/DECISIONS]' ;;
  *"Wrap up the team discussion"*) out='Summary: blue-green, 10 minute buffer.' ;;
  *"Team discussion in"*)
    out='Agreed, with a buffer.\n\n[decisions] question: Arrival buffer | options: 5 min; 10 min; 15 min | picked: 10 min | why: time to park question: Traffic on days off | options: Run Maps anyway; One-line note | picked: One-line note | why: noise\nthis line is not a decision' ;;
  *) out="answer" ;;
esac
printf '%s\n' "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"$out\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":90}}"
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
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

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_team_discussion_reply_keeps_its_choices_as_chips_not_text() {
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
    let team = ok(&app, "POST", "/api/teams", Some(json!({"name": "infra", "lead_kwant_id": ann, "member_kwants": [ann, bob]}))).await;
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

    let sent = ok(
        &app,
        "POST",
        "/api/chat/send",
        Some(json!({"channel_id": channel, "team_id": team["id"], "sender_id": "user",
            "prompt": "How should we design the zero-downtime database migration architecture? Weigh the tradeoff."})),
    )
    .await;
    assert_eq!(sent["status"], "dispatched", "{sent}");

    // Opening, Bob's reply and the wrap-up
    let mut replies = Vec::new();
    for _ in 0..300 {
        replies = state.db.list_messages(&channel, 100).await.unwrap().into_iter().filter(|m| m.sender_role == "kwant").collect();
        if replies.iter().any(|m| m.text.starts_with("Summary:")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let texts: Vec<_> = replies.iter().map(|m| (&m.sender_id, &m.text)).collect();
    assert!(replies.len() >= 3, "{texts:#?}");
    for m in &replies {
        assert!(!m.text.to_lowercase().contains("decisions]"), "raw block shown: {}", m.text);
        assert!(!m.text.contains("question:") && !m.text.contains("picked:"), "raw block shown: {}", m.text);
    }

    let opening = replies.iter().find(|m| m.text.starts_with("Go with a blue-green")).expect("the opening");
    assert_eq!(opening.text, "Go with a blue-green cutover. @Bob what do you think?");
    let d = &meta(opening)["decisions"];
    assert_eq!(d.as_array().map(Vec::len), Some(1), "{d}");
    assert_eq!((d[0]["question"].as_str(), d[0]["picked"].as_str()), (Some("Cutover style?"), Some("Blue-green")));

    // The garbled block: both run-together choices come through and nothing of it is shown
    let reply = replies.iter().find(|m| m.text.starts_with("Agreed")).expect("Bob's reply");
    assert_eq!(reply.text, "Agreed, with a buffer.");
    let d = &meta(reply)["decisions"];
    assert_eq!(d.as_array().map(Vec::len), Some(2), "{d}");
    assert_eq!((d[0]["question"].as_str(), d[0]["picked"].as_str(), d[0]["why"].as_str()), (Some("Arrival buffer"), Some("10 min"), Some("time to park")));
    assert_eq!((d[1]["question"].as_str(), d[1]["picked"].as_str()), (Some("Traffic on days off"), Some("One-line note")));

    // The chips' Change works on a channel message and names the bot that decided
    let (s, v) = call(&app, "POST", &format!("/api/messages/{}/decisions/0/change", reply.id), Some(json!({"picked": "15 min"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["prompt"].as_str().unwrap().starts_with(&format!("@{} Change of decision", reply.sender_id)), "{v}");
    let after = state.db.get_message(&reply.id).await.unwrap().unwrap();
    assert_eq!(meta(&after)["decisions"][0]["changed_to"], "15 min");
}
