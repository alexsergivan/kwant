//! Usage visibility for CLI bots and the chain token budget. A fake claude CLI reports a turn of
//! heavy usage (mostly cache reads) and @mentions a teammate when asked to, so one person message
//! can start a chain of hand-offs. Its own test binary: it changes PATH.

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
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// Replies "answer" plus "@<name>" for a prompt with "MENTION:<name>", but only on the first turn
/// of a chain (a hand-off turn says "handed this conversation to you"), so a chain is two turns.
/// Every turn reports 12 new input, 8,211 cache-write, 83,544 cache-read and 487 output tokens,
/// $0.42: 8,710 fresh tokens.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
mention=$(printf '%s\n' "$last" | sed -n 's/.*MENTION:\([A-Za-z0-9_]*\).*/\1/p' | head -n 1)
case "$last" in *"handed this conversation to you"*) mention="" ;; esac
out="answer"
[ -n "$mention" ] && out="$out @$mention"
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"$out\",\"total_cost_usd\":0.42,\"usage\":{\"input_tokens\":12,\"cache_creation_input_tokens\":8211,\"cache_read_input_tokens\":83544,\"output_tokens\":487},\"modelUsage\":{\"claude-opus-5-5\":{\"outputTokens\":487}}}"
"#;

const FRESH_PER_TURN: u64 = 12 + 8_211 + 487;

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

/// Ann and Bob on the fake claude in one channel; Bob leads, so Ann's mention of him is a plain
/// hand-off in the channel.
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
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "lead_kwant_id": bob, "member_kwants": [ann]}))).await;
    Env { app, state, channel: c["id"].as_str().unwrap().to_string(), _tmp: tmp }
}

impl Env {
    async fn say(&self, prompt: &str) -> Value {
        ok(&self.app, "POST", "/api/chat/send", Some(json!({"channel_id": self.channel, "prompt": prompt, "sender_id": "user"}))).await
    }

    async fn messages(&self) -> Vec<Message> {
        self.state.db.list_messages(&self.channel, 100).await.unwrap()
    }

    async fn replies_from(&self, name: &str) -> usize {
        self.messages().await.iter().filter(|m| m.sender_role == "kwant" && m.sender_id == name).count()
    }

    async fn notices(&self) -> Vec<Message> {
        self.messages().await.into_iter().filter(|m| meta(m)["kind"] == "budget_paused").collect()
    }

    async fn kwant_id(&self, name: &str) -> String {
        let kwants = ok(&self.app, "GET", "/api/kwants", None).await;
        kwants.as_array().unwrap().iter().find(|k| k["name"] == name).unwrap()["id"].as_str().unwrap().to_string()
    }

    async fn wait_for<F, Fut>(&self, what: &str, mut check: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..200 {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}: {:#?}", self.messages().await.iter().map(|m| (&m.sender_id, &m.text)).collect::<Vec<_>>());
    }

    /// Lets anything still on its way arrive.
    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_cli_reply_shows_its_usage_split_and_is_counted_per_bot_and_day() {
    let env = setup().await;
    env.say("@Ann hello").await;
    env.wait_for("Ann's reply", || async { env.replies_from("Ann").await == 1 }).await;
    env.settle().await;

    let reply = env.messages().await.into_iter().find(|m| m.sender_id == "Ann").unwrap();
    // tokens_used keeps its old, cache-inclusive meaning; the split is in the meta
    assert_eq!(reply.tokens_used, 12 + 8_211 + 83_544 + 487);
    assert_eq!(
        meta(&reply)["usage"],
        json!({"fresh_input": 12, "cache_read": 83_544, "cache_write": 8_211, "output": 487, "cost_usd": 0.42})
    );

    // One model_usage row per CLI turn, on the bot's Usage tab
    let ann = env.kwant_id("Ann").await;
    let usage = ok(&env.app, "GET", &format!("/api/kwants/{ann}/model-usage"), None).await;
    let rows = usage["recent"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{usage}");
    assert_eq!(rows[0]["model"], "claude-opus-5-5");
    assert_eq!((rows[0]["fresh_input_tokens"].as_i64(), rows[0]["cache_read_tokens"].as_i64()), (Some(12), Some(83_544)));
    assert_eq!(rows[0]["cost_usd"].as_f64(), Some(0.42));

    // Settings → Usage: per bot and day
    let daily = ok(&env.app, "GET", "/api/usage/daily?days=7", None).await;
    let rows = daily["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{daily}");
    assert_eq!((rows[0]["kwant_id"].as_str(), rows[0]["kwant_name"].as_str()), (Some(ann.as_str()), Some("Ann")));
    assert_eq!(rows[0]["day"].as_str().unwrap(), chrono::Utc::now().format("%Y-%m-%d").to_string());
    assert_eq!((rows[0]["output_tokens"].as_i64(), rows[0]["cache_write_tokens"].as_i64()), (Some(487), Some(8_211)));
    assert_eq!(rows[0]["cost_usd"].as_f64(), Some(0.42));
}

#[tokio::test]
async fn a_chain_over_its_budget_pauses_hand_offs_until_the_person_says_continue() {
    let env = setup().await;
    // One turn fits, a second doesn't
    ok(&env.app, "POST", "/api/settings", Some(json!({"chain_token_budget": FRESH_PER_TURN - 1}))).await;
    assert_eq!(ok(&env.app, "GET", "/api/settings", None).await["chain_token_budget"].as_u64(), Some(FRESH_PER_TURN - 1));

    env.say("@Ann look at this MENTION:Bob").await;
    env.wait_for("the notice", || async { !env.notices().await.is_empty() }).await;
    env.settle().await;
    assert_eq!(env.replies_from("Ann").await, 1);
    assert_eq!(env.replies_from("Bob").await, 0, "no hand-off over the budget");
    let notices = env.notices().await;
    assert_eq!(notices.len(), 1, "one notice");
    assert!(notices[0].text.contains("Bob") && notices[0].text.contains("8k fresh tokens"), "{}", notices[0].text);
    assert_eq!(meta(&notices[0])["waiting"], json!(["Bob"]));

    // The person says continue: Bob takes the turn he was handed, with a fresh budget
    let sent = env.say("continue").await;
    assert_eq!(sent["status"], "continued", "{sent}");
    env.wait_for("Bob's turn", || async { env.replies_from("Bob").await == 1 }).await;
    env.settle().await;
    assert_eq!(env.replies_from("Ann").await, 1, "\"continue\" isn't a new request for Ann");
    assert_eq!(meta(&env.notices().await[0])["resumed"], true);
    // Already carried on: the button is refused
    let (s, _) = call(&env.app, "POST", &format!("/api/chains/{}/continue", notices[0].id), None).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn the_continue_button_carries_a_paused_chain_on() {
    let env = setup().await;
    ok(&env.app, "POST", "/api/settings", Some(json!({"chain_token_budget": 1}))).await;
    env.say("@Ann look at this MENTION:Bob").await;
    env.wait_for("the notice", || async { !env.notices().await.is_empty() }).await;
    let notice = env.notices().await.remove(0);
    ok(&env.app, "POST", &format!("/api/chains/{}/continue", notice.id), None).await;
    env.wait_for("Bob's turn", || async { env.replies_from("Bob").await == 1 }).await;
    assert!(env.messages().await.iter().any(|m| m.sender_role == "user" && m.text == "continue"), "the person's continue is in the chat");
    // Not a notice: refused
    let first = env.messages().await.remove(0);
    let (s, _) = call(&env.app, "POST", &format!("/api/chains/{}/continue", first.id), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_budget_of_zero_never_pauses() {
    let env = setup().await;
    ok(&env.app, "POST", "/api/settings", Some(json!({"chain_token_budget": 0}))).await;
    env.say("@Ann look at this MENTION:Bob").await;
    env.wait_for("Bob's turn", || async { env.replies_from("Bob").await == 1 }).await;
    env.settle().await;
    assert!(env.notices().await.is_empty());
}

#[tokio::test]
async fn a_fresh_install_has_no_token_limit_on_hand_offs() {
    let env = setup().await;
    assert_eq!(kwnt::server::budget::DEFAULT, 0);
    assert_eq!(ok(&env.app, "GET", "/api/settings", None).await["chain_token_budget"].as_u64(), Some(0));
    assert_eq!(kwnt::server::budget::limit(&env.state).await, 0);
    env.say("@Ann look at this MENTION:Bob").await;
    env.wait_for("Bob's turn", || async { env.replies_from("Bob").await == 1 }).await;
    env.settle().await;
    assert!(env.notices().await.is_empty(), "nothing pauses without a limit");
}

/// Runs a statement on the database file directly, as an older kwnt would have left it.
async fn raw(dir: &std::path::Path, sql: &str) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", dir.join("kwnt.db").display()))
        .await
        .unwrap();
    sqlx::query(sql).execute(&pool).await.unwrap();
    pool.close().await;
}

/// The value after an upgrade from before the limit was opt-in, when `stored` was saved.
async fn upgraded(stored: &str) -> Option<String> {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting(kwnt::server::budget::SETTING, stored).await.unwrap();
    drop(db);
    // Before the upgrade the migration had never run
    raw(tmp.path(), "DELETE FROM settings WHERE key = 'chain_token_budget_opt_in_v1'").await;
    let db = Database::init(tmp.path()).await.unwrap();
    db.get_setting(kwnt::server::budget::SETTING).await.unwrap()
}

#[tokio::test]
async fn an_upgrade_turns_off_the_old_default_and_keeps_a_limit_the_person_chose() {
    assert_eq!(upgraded("200000").await.as_deref(), Some("0"), "the old default Settings saved back is switched off");
    assert_eq!(upgraded("50000").await.as_deref(), Some("50000"), "a chosen limit stays");
    assert_eq!(upgraded("0").await.as_deref(), Some("0"));
}

#[tokio::test]
async fn a_200k_limit_set_after_the_upgrade_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting(kwnt::server::budget::SETTING, "200000").await.unwrap();
    drop(db);
    // The migration runs once: opening again doesn't clear a 200k the person picked since
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.get_setting(kwnt::server::budget::SETTING).await.unwrap().as_deref(), Some("200000"));
}
