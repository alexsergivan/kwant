//! Work ahead: a quiet chat's lead gets one hidden run and posts one "While you were away"
//! message; `[NOTHING]` posts nothing; the daily cap, the switch, a chat that isn't quiet yet and
//! a second run before the person writes again all start nothing. A fake claude CLI answers by
//! keyword. Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration as Hours, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::{Message, NewMessage}, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// - a work-ahead run in a chat that says QUIETNOTHING: `[NOTHING]`;
/// - any other work-ahead run: a short report with a prediction;
/// - anything else: "answer".
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *"[Work ahead]"*)
    case "$last" in
      *QUIETNOTHING*) out='[NOTHING]' ;;
      *) out='While you were away I ran the pricing page link check: all 14 links work.\nNext I expect you will want the press note draft.' ;;
    esac ;;
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

struct Env {
    app: Router,
    state: AppState,
    channel: String,
    ann: String,
    bob: String,
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
    Env { app, state, channel: c["id"].as_str().unwrap().to_string(), ann, bob, _tmp: tmp }
}

/// Sets a timestamp the way kwnt writes it (sqlx's encoding), so comparisons line up.
async fn set_time(state: &AppState, id: &str, t: DateTime<Utc>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", state.config.data_dir.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?").bind(t).bind(id).execute(&pool).await.unwrap();
    pool.close().await;
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

impl Env {
    /// Puts `(role, sender, text, hours ago)` into `chat` (a channel id, or a bot's direct chat).
    async fn seed(&self, chat: &str, direct: bool, rows: &[(&str, &str, &str, i64)]) {
        for (role, sender, text, hours) in rows {
            let m = NewMessage {
                channel_id: (!direct).then(|| chat.to_string()),
                direct_kwant_id: direct.then(|| chat.to_string()),
                sender_role: role.to_string(),
                sender_id: sender.to_string(),
                text: text.to_string(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 0,
            };
            let saved = self.state.db.insert_message(&m).await.unwrap();
            set_time(&self.state, &saved.id, Utc::now() - Hours::hours(*hours)).await;
        }
    }

    async fn quiet_channel(&self, text: &str) {
        self.seed(&self.channel, false, &[("user", "user", text, 7), ("kwant", "Ann", "Plan is in place. Open items: link check not run.", 6)]).await;
    }

    async fn run_now(&self) -> Vec<Value> {
        ok(&self.app, "POST", "/api/work-ahead/run", None).await["started"].as_array().unwrap().clone()
    }

    async fn work_ahead_in(&self, chat: &str) -> Vec<Message> {
        self.state.db.list_messages(chat, 100).await.unwrap().into_iter().filter(|m| meta(m)["kind"] == "work_ahead").collect()
    }

    /// Waits until no run is going.
    async fn settle(&self) {
        for _ in 0..200 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let turns = ok(&self.app, "GET", "/api/turns", None).await;
            if turns["turns"].as_array().is_none_or(|t| t.is_empty()) {
                tokio::time::sleep(Duration::from_millis(300)).await;
                return;
            }
        }
        panic!("runs never ended");
    }
}

#[tokio::test]
async fn a_quiet_chat_gets_one_run_and_one_message_with_a_prediction() {
    let env = setup().await;
    env.quiet_channel("Plan the launch week.").await;
    let started = env.run_now().await;
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["kwant_id"], env.ann.as_str(), "the channel's lead works ahead");
    assert_eq!(started[0]["context_id"], env.channel.as_str());
    env.settle().await;
    let posted = env.work_ahead_in(&env.channel).await;
    assert_eq!(posted.len(), 1);
    let m = &posted[0];
    assert_eq!(m.sender_id, "Ann", "kwant messages carry the bot's name");
    assert!(m.text.starts_with("While you were away I ran the pricing page link check"), "{}", m.text);
    assert!(m.text.contains("Next I expect"));
    let meta = meta(m);
    assert_eq!(meta["date"], Utc::now().format("%Y-%m-%d").to_string());
    assert!(meta["run_id"].is_string());

    // Not again before the person writes, even on a later scan
    assert!(env.run_now().await.is_empty());
    assert_eq!(env.work_ahead_in(&env.channel).await.len(), 1);
}

#[tokio::test]
async fn nothing_worth_doing_posts_nothing_and_isnt_retried_today() {
    let env = setup().await;
    env.quiet_channel("Plan the launch week. QUIETNOTHING").await;
    assert_eq!(env.run_now().await.len(), 1);
    env.settle().await;
    assert!(env.work_ahead_in(&env.channel).await.is_empty());
    let all = env.state.db.list_messages(&env.channel, 100).await.unwrap();
    assert_eq!(all.len(), 2, "only the seeded messages");
    assert!(env.run_now().await.is_empty(), "one run per chat per day");
}

#[tokio::test]
async fn the_daily_cap_the_switch_and_a_chat_that_isnt_quiet_start_nothing() {
    let env = setup().await;
    // Not quiet yet: the last message is an hour old
    env.seed(&env.channel, false, &[("user", "user", "Plan the launch week.", 3), ("kwant", "Ann", "On it.", 1)]).await;
    assert!(env.run_now().await.is_empty());

    // Two quiet direct chats, a cap of 1: one run
    env.seed(&env.ann, true, &[("user", "user", "Draft the note.", 8), ("kwant", "Ann", "Outline is ready.", 7)]).await;
    env.seed(&env.bob, true, &[("user", "user", "Check the links.", 8), ("kwant", "Bob", "Started.", 7)]).await;
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"work_ahead_daily_cap": 1}))).await;
    assert!(s.is_success(), "{v}");
    assert_eq!(env.run_now().await.len(), 1);
    env.settle().await;
    assert!(env.run_now().await.is_empty(), "cap reached");

    // Off: nothing, and the route says so
    let env = setup().await;
    env.quiet_channel("Plan the launch week.").await;
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"work_ahead": false}))).await;
    assert!(s.is_success(), "{v}");
    let v = ok(&env.app, "POST", "/api/work-ahead/run", None).await;
    assert_eq!(v, json!({"enabled": false, "started": []}));
}
