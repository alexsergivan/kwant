//! Waiting on the person (F10): claude's own `AskUserQuestion` lands in the questions table, a
//! turn that ends waiting on the person gets one reminder later, the person writing or answering
//! cancels it, and `GET /api/waiting` (person-only) lists what waits. A fake claude CLI on PATH
//! answers as the bot named in its system prompt ("You are <name>, …") from `$KWNT_TEST_BEHAVE/<name>`;
//! a first line "ASK:<question>" makes it call `AskUserQuestion` (choices Yes / No) before it
//! replies with the rest. Its own test binary: it changes PATH.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{Duration as Hours, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Message, Database},
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, mcp_runtime, waiting, AppState},
    tools::ToolRunner,
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
me=$(printf '%s\n' "$@" | sed -n 's/.*You are \([A-Za-z0-9]*\), .*/\1/p' | head -n 1)
file="$KWNT_TEST_BEHAVE/$me"
if [ ! -f "$file" ]; then
  echo '{"type":"result","is_error":false,"num_turns":1,"result":"Done."}'
  exit 0
fi
first=$(head -n 1 "$file")
case "$first" in
  ASK:*)
    q="${first#ASK:}"
    tu="tu_$(date +%s%N)"
    printf '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"%s","name":"AskUserQuestion","input":{"questions":[{"question":"%s","header":"Q","multiSelect":false,"options":[{"label":"Yes","description":"go"},{"label":"No","description":"stop"}]}]}}]}}\n' "$tu" "$q"
    printf '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"%s","is_error":true,"content":"no tty"}]}}\n' "$tu"
    out=$(tail -n +2 "$file" | sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g') ;;
  *) out=$(sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g' "$file") ;;
esac
printf '{"type":"result","is_error":false,"num_turns":1,"result":"%s"}\n' "$out"
"#;

fn behave_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let (bin, behave) = (root.join("bin"), root.join("behave"));
        for d in [&bin, &behave] {
            std::fs::create_dir_all(d).unwrap();
        }
        let fake = bin.join("claude");
        exe::write_executable(&fake, FAKE_CLAUDE);
        std::env::set_var("KWNT_TEST_BEHAVE", &behave);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        behave
    })
}

/// What bot `name` replies (see the module doc); names are unique per test.
fn behave(name: &str, reply: &str) {
    std::fs::write(behave_dir().join(name), reply).unwrap();
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let resp = app
        .clone()
        .oneshot(req.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap())
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

struct Env {
    app: Router,
    state: AppState,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    behave_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
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
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

async fn add_bot(env: &Env, name: &str) -> String {
    let (s, v) = call(&env.app, "POST", "/api/kwants", Some(json!({"name": name, "title": "Specialist", "persona_prompt": "", "soul": "", "avatar_sprite": "default", "driver_type": "cli", "preferred_model": "claude"})), None).await;
    assert!(s.is_success(), "{s} {v}");
    v["id"].as_str().or(v["kwant"]["id"].as_str()).unwrap().to_string()
}

/// The person writes in `bot`'s direct chat and the bot's turn runs to its end.
async fn say_to(env: &Env, bot: &str, prompt: &str) {
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"kwant_id": bot, "prompt": prompt, "sender_id": "user"})), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(env).await;
}

async fn messages(env: &Env, bot: &str) -> Vec<Message> {
    env.state.db.list_messages(bot, 200).await.unwrap()
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

fn reminders(msgs: &[Message]) -> usize {
    msgs.iter().filter(|m| meta(m)["kind"] == "waiting_reminder").count()
}

async fn settle(env: &Env) {
    let mut quiet = 0;
    for _ in 0..400 {
        quiet = if env.state.turns.active_turns().is_empty() { quiet + 1 } else { 0 };
        if quiet >= 10 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the turn never ended");
}

async fn waiting_list(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", "/api/waiting", None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().cloned().unwrap_or_default()
}

/// One day and a bit from now: past any reminder scheduled with the default 24 h.
fn tomorrow() -> chrono::DateTime<Utc> {
    Utc::now() + Hours::hours(25)
}

#[tokio::test]
async fn claude_ask_user_question_is_a_question_and_is_reminded_once() {
    let env = setup().await;
    let bot = add_bot(&env, "Asker1").await;
    behave("Asker1", "ASK:Ship the release tonight?\nI asked whether to ship tonight.");
    say_to(&env, &bot, "get the release ready").await;

    let open = env.state.db.list_questions(Some("open"), None, None).await.unwrap();
    assert_eq!(open.len(), 1, "{open:?}");
    let q = &open[0];
    assert_eq!((q.question.as_str(), q.choice_list()), ("Ship the release tonight?", vec!["Yes".to_string(), "No".to_string()]));
    assert_eq!((q.kwant_id.as_str(), q.direct_kwant_id.as_deref()), (bot.as_str(), Some(bot.as_str())));
    let msgs = messages(&env, &bot).await;
    let card = msgs.iter().find(|m| Some(&m.id) == q.message_id.as_ref()).expect("the question has a card");
    assert_eq!((meta(card)["kind"].as_str(), meta(card)["status"].as_str()), (Some("ask_user"), Some("open")));

    // Waiting on the person, with one reminder in about a day
    let items = waiting_list(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!((items[0]["kind"].as_str(), items[0]["question_id"].as_str()), (Some("question"), Some(q.id.as_str())));
    let at: chrono::DateTime<Utc> = serde_json::from_value(items[0]["follow_up_at"].clone()).unwrap();
    assert!(at > Utc::now() + Hours::hours(23) && at < tomorrow(), "{at}");

    // Not before it's due; then once, never twice
    assert_eq!(waiting::run_due(&env.state, Utc::now()).await, 0);
    assert_eq!(waiting::run_due(&env.state, tomorrow()).await, 1);
    assert_eq!(waiting::run_due(&env.state, tomorrow() + Hours::hours(48)).await, 0);
    let msgs = messages(&env, &bot).await;
    assert_eq!(reminders(&msgs), 1);
    let r = msgs.iter().find(|m| meta(m)["kind"] == "waiting_reminder").unwrap();
    assert!(r.text.contains("Ship the release tonight?") && r.sender_role == "kwant", "{}", r.text);
    // A reminder isn't a turn and ends no turn: nothing new is scheduled from it
    settle(&env).await;
    assert_eq!(env.state.db.next_waiting_due().await.unwrap(), None);
    // Still waiting until the person answers
    let items = waiting_list(&env).await;
    assert_eq!(items.len(), 1);
    assert!(items[0]["follow_up_at"].is_null() && !items[0]["reminded_at"].is_null(), "{items:?}");
}

#[tokio::test]
async fn a_reply_that_asks_is_followed_up_and_a_plain_result_is_not() {
    let env = setup().await;
    let done = add_bot(&env, "Plain2").await;
    behave("Plain2", "Here is the summary you wanted.\nAnything else?");
    say_to(&env, &done, "summarise it").await;
    assert!(waiting_list(&env).await.is_empty(), "a finished answer with a closer isn't waiting");

    let asks = add_bot(&env, "Asks2").await;
    behave("Asks2", "I can book it.\nWhich city are you in?");
    say_to(&env, &asks, "book a table").await;
    let items = waiting_list(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!((items[0]["kind"].as_str(), items[0]["text"].as_str()), (Some("reply"), Some("Which city are you in?")));
    assert_eq!(items[0]["kwant_name"], "Asks2");

    // The person writes to that bot: no reminder, nothing waits any more
    behave("Asks2", "Booked.");
    say_to(&env, &asks, "Munich").await;
    assert!(waiting_list(&env).await.is_empty());
    assert_eq!(waiting::run_due(&env.state, tomorrow()).await, 0);
    assert_eq!(reminders(&messages(&env, &asks).await), 0);
}

#[tokio::test]
async fn answering_the_question_cancels_the_reminder() {
    let env = setup().await;
    let bot = add_bot(&env, "Asker3").await;
    behave("Asker3", "ASK:Delete the old branch?\nWaiting for your answer.");
    say_to(&env, &bot, "tidy the repo").await;
    let q = env.state.db.list_questions(Some("open"), None, None).await.unwrap().pop().expect("a question");
    behave("Asker3", "Deleted it.");
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{}/answer", q.id), Some(json!({"choice": 0})), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(&env).await;
    assert!(waiting_list(&env).await.is_empty());
    assert_eq!(waiting::run_due(&env.state, tomorrow()).await, 0);
    assert_eq!(reminders(&messages(&env, &bot).await), 0);
}

#[tokio::test]
async fn zero_hours_turns_reminders_off() {
    let env = setup().await;
    let (s, v) = call(&env.app, "GET", "/api/settings", None, None).await;
    assert_eq!((s, v["waiting_follow_up_hours"].as_i64()), (StatusCode::OK, Some(24)));
    let bad = call(&env.app, "POST", "/api/settings", Some(json!({"waiting_follow_up_hours": -1})), None).await;
    assert_eq!(bad.0, StatusCode::BAD_REQUEST);

    // Scheduled with reminders on, then switched off: it never goes out
    let bot = add_bot(&env, "Asks4").await;
    behave("Asks4", "Which week should I chart?");
    say_to(&env, &bot, "chart the signups").await;
    assert!(env.state.db.next_waiting_due().await.unwrap().is_some());
    let (s, _) = call(&env.app, "POST", "/api/settings", Some(json!({"waiting_follow_up_hours": 0})), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(waiting::run_due(&env.state, tomorrow()).await, 0);

    // Asked with reminders off: listed, never scheduled
    let other = add_bot(&env, "Asks5").await;
    behave("Asks5", "ASK:Which plan?\nWaiting.");
    say_to(&env, &other, "pick a plan").await;
    let items = waiting_list(&env).await;
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(items.iter().all(|i| i["follow_up_at"].is_null()), "{items:?}");
    assert_eq!(env.state.db.next_waiting_due().await.unwrap(), None);
    assert_eq!(waiting::run_due(&env.state, tomorrow()).await, 0);
    assert_eq!(reminders(&messages(&env, &bot).await) + reminders(&messages(&env, &other).await), 0);
}

#[tokio::test]
async fn the_waiting_list_is_for_the_person_only() {
    let env = setup().await;
    let bot = add_bot(&env, "Bot6").await;
    let kwant = env.state.db.get_kwant(&bot).await.unwrap().unwrap();
    let turn_id = uuid::Uuid::new_v4().to_string();
    let scope = SelfScope { kwant_id: bot.clone(), kwant_name: kwant.name.clone(), turn_id: Some(turn_id.clone()), ..Default::default() };
    let (_, token) = mcp_runtime::prepare_turn_with_opts(&env.state, &kwant, Some(turn_id), None, Some(scope), Default::default())
        .await
        .unwrap();
    let (s, v) = call(&env.app, "GET", "/api/waiting", None, Some(&token)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a bot's turn token: {v}");
    let (s, _) = call(&env.app, "GET", "/api/waiting", None, None).await;
    assert_eq!(s, StatusCode::OK);
    mcp_runtime::finish_turn(&env.state, &token).await;
}
