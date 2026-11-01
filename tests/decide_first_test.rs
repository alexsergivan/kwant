//! Decide first: a reply that ends by asking the person to pick between reversible options is held,
//! the bot hears a teammate and decides, and one decided reply reaches the person, with a Change
//! that keeps a note of how the person decides. A fake claude CLI answers by keyword. Its own test
//! binary: it changes PATH.

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
    db::{models::Message, spaces::SpaceId, Database},
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

/// - the hidden round's prompt: a decided reply with a [DECISIONS] block;
/// - a teammate's view: "I'd pick B";
/// - CHOICE: the screenshot's ending ("A, B or C?"), naming Bob;
/// - SEND: an ask about sending an email;
/// - anything else: "answer".
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *"[Decide first] Your last reply"*)
    out='I checked contrast on A and C and went with B: Wax seal. Change it any time.\n\n[DECISIONS]\nquestion: Which direction? | options: A: Classic; B: Wax seal; C: Green desk | picked: B: Wax seal | why: reads best\n[/DECISIONS]' ;;
  *"[Decide first]"*) out="I'd pick B: it reads best." ;;
  *CHOICE*) out='Three directions are ready. Bob only looked at B; nobody looked at A and C.\n\nTwo decisions for you:\n1. A, B or C?\n2. FM monogram or plain?' ;;
  *SEND*) out='The draft is ready.\n\nShould I send the email to Mia now, or wait until Monday?' ;;
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

struct Env {
    app: Router,
    state: AppState,
    channel: String,
    _tmp: tempfile::TempDir,
}

/// Ann and Bob on the fake claude in one channel; Ann leads.
async fn setup() -> Env {
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
        json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
            "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"})
    };
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "lead_kwant_id": ann, "member_kwants": [bob]}))).await;
    Env { app, state, channel: c["id"].as_str().unwrap().to_string(), _tmp: tmp }
}

impl Env {
    async fn say(&self, prompt: &str) {
        ok(&self.app, "POST", "/api/chat/send", Some(json!({"channel_id": self.channel, "prompt": prompt, "sender_id": "user"}))).await;
    }

    async fn messages(&self) -> Vec<Message> {
        self.state.db.list_messages(&self.channel, 100).await.unwrap()
    }

    async fn replies(&self) -> Vec<Message> {
        self.messages().await.into_iter().filter(|m| m.sender_role == "kwant").collect()
    }

    async fn wait_for_reply(&self) -> Message {
        for _ in 0..200 {
            if let Some(m) = self.replies().await.pop() {
                // Let anything still on its way arrive
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let all = self.replies().await;
                assert_eq!(all.len(), 1, "{:#?}", all.iter().map(|m| (&m.sender_id, &m.text)).collect::<Vec<_>>());
                return all.into_iter().next().unwrap_or(m);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no reply: {:#?}", self.messages().await.iter().map(|m| (&m.sender_id, &m.text)).collect::<Vec<_>>());
    }
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_reply_asking_to_pick_is_decided_with_a_teammate_and_can_be_changed() {
    let env = setup().await;
    let mut rx = env.state.event_sender.subscribe();
    env.say("@Ann make three directions for the hero CHOICE").await;
    let reply = env.wait_for_reply().await;

    // Only the decided reply reached the chat, without the block; the choices are in meta
    assert_eq!(reply.sender_id, "Ann");
    assert_eq!(reply.text, "I checked contrast on A and C and went with B: Wax seal. Change it any time.");
    let m = meta(&reply);
    assert_eq!(m["decisions"][0]["picked"], "B: Wax seal");
    assert_eq!(m["decisions"][0]["options"], json!(["A: Classic", "B: Wax seal", "C: Green desk"]));
    assert_eq!(m["decisions"][0]["status"], "decided");
    assert_eq!(m["decide_first"]["consulted"], json!(["Bob"]), "Bob was named in the held reply");

    let (mut deciding, mut done) = (false, None);
    while let Ok(e) = rx.try_recv() {
        if let RuntimeEvent::DecideUpdated { state, outcome, consulted, .. } = e.unscoped() {
            match state.as_str() {
                "deciding" => {
                    deciding = true;
                    assert_eq!(consulted, &vec!["Bob".to_string()]);
                }
                _ => done = outcome.clone(),
            }
        }
    }
    assert!(deciding);
    assert_eq!(done.as_deref(), Some("decided"));

    // Change: bad picks are refused, a good one marks it and returns the prompt to send
    let url = format!("/api/messages/{}/decisions/0/change", reply.id);
    for bad in [json!({"picked": "D: Neon"}), json!({"picked": "B: Wax seal"}), json!({})] {
        let (s, v) = call(&env.app, "POST", &url, Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    let (s, _) = call(&env.app, "POST", &format!("/api/messages/{}/decisions/5/change", reply.id), Some(json!({"picked": "A: Classic"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "POST", "/api/messages/nope/decisions/0/change", Some(json!({"picked": "A: Classic"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let v = ok(&env.app, "POST", &url, Some(json!({"picked": "A: Classic"}))).await;
    assert_eq!(
        v["prompt"],
        "@Ann Change of decision: for “Which direction?” use “A: Classic” instead of “B: Wax seal”. Redo what depended on it."
    );
    let changed: Value = serde_json::from_str(v["message"]["meta"].as_str().unwrap()).unwrap();
    assert_eq!((changed["decisions"][0]["status"].as_str(), changed["decisions"][0]["changed_to"].as_str()), (Some("changed"), Some("A: Classic")));
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"picked": "C: Green desk"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "one change per decision");

    // Notes: the bot's pick and the person's change; the prompt block shows the change
    let space = SpaceId::default_space();
    let notes = env.state.db.list_decision_notes(&space, &[], 10).await.unwrap();
    let sources: Vec<&str> = notes.iter().map(|n| n.source.as_str()).collect();
    assert_eq!(sources, vec!["changed", "bot"]);
    assert_eq!(notes[0].bot_pick.as_deref(), Some("B: Wax seal"));
    assert_eq!(env.state.db.decision_keep_rate(&space, 20).await.unwrap(), (0, 1));
    let block = kwnt::server::decide::prompt_block(&env.state, &space).await;
    assert!(block.contains("[How the person usually decides]"), "{block}");
    assert!(block.contains("changed the bot's “B: Wax seal” to “A: Classic”"), "{block}");
    assert!(block.len() <= kwnt::server::decide::MAX_BLOCK_BYTES);
}

#[tokio::test]
async fn an_ask_about_sending_is_posted_as_is() {
    let env = setup().await;
    env.say("@Ann draft the email to Mia SEND").await;
    let reply = env.wait_for_reply().await;
    assert!(reply.text.ends_with("Should I send the email to Mia now, or wait until Monday?"), "{}", reply.text);
    assert!(meta(&reply).get("decide_first").is_none());
}

#[tokio::test]
async fn the_person_asking_to_pick_gets_the_options() {
    let env = setup().await;
    env.say("@Ann give me three directions, let me pick CHOICE").await;
    let reply = env.wait_for_reply().await;
    assert!(reply.text.ends_with("2. FM monogram or plain?"), "{}", reply.text);
    assert!(meta(&reply).get("decide_first").is_none());
}

#[tokio::test]
async fn over_the_budget_the_original_reply_is_posted() {
    let env = setup().await;
    // The first turn fits; the hidden round's reserve doesn't
    ok(&env.app, "POST", "/api/settings", Some(json!({"chain_token_budget": 1000}))).await;
    env.say("@Ann make three directions CHOICE").await;
    let reply = env.wait_for_reply().await;
    assert!(reply.text.ends_with("2. FM monogram or plain?"), "{}", reply.text);
    assert_eq!(meta(&reply)["decide_first"]["outcome"], "posted_original");
}

#[tokio::test]
async fn with_decide_first_off_nothing_is_held() {
    let env = setup().await;
    ok(&env.app, "POST", "/api/settings", Some(json!({"decide_first": false}))).await;
    env.say("@Ann make three directions CHOICE").await;
    let reply = env.wait_for_reply().await;
    assert!(reply.text.ends_with("2. FM monogram or plain?"), "{}", reply.text);
    assert!(meta(&reply).get("decide_first").is_none());
}

#[tokio::test]
async fn the_settings_round_trip_and_are_validated() {
    let env = setup().await;
    let s = ok(&env.app, "GET", "/api/settings", None).await;
    assert_eq!((s["decide_first"].as_bool(), s["work_ahead"].as_bool(), s["work_ahead_daily_cap"].as_u64()), (Some(true), Some(true), Some(3)));
    ok(&env.app, "POST", "/api/settings", Some(json!({"decide_first": false, "work_ahead": false, "work_ahead_daily_cap": 7}))).await;
    let s = ok(&env.app, "GET", "/api/settings", None).await;
    assert_eq!((s["decide_first"].as_bool(), s["work_ahead"].as_bool(), s["work_ahead_daily_cap"].as_u64()), (Some(false), Some(false), Some(7)));
    for bad in [0, 11] {
        let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"work_ahead_daily_cap": bad}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "work_ahead_daily_cap must be 1–10");
    }
}
