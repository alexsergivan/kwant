//! The daily spending stop (0.7.4, next-074/spend-stop/spec.md). A fake claude CLI stands in for
//! every engine: it reports 8,710 fresh tokens a turn and $0.42, or no cost at all when the prompt
//! says NOCOST (like an engine that doesn't report `cost_usd`). The day moves on through the
//! stop's clock seam, never by waiting. Its own test binary: it changes PATH.

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
        goals::{Created, NewGoal, NewGoalCheck},
        missions::now_ms,
        models::Message,
        Database,
    },
    router::LayaRouter,
    server::{create_router, goals::runner, spend_stop, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
cost=',"total_cost_usd":0.42'
case "$last" in *NOCOST*) cost="" ;; esac
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"answer\"$cost,\"usage\":{\"input_tokens\":12,\"cache_creation_input_tokens\":8211,\"cache_read_input_tokens\":83544,\"output_tokens\":487},\"modelUsage\":{\"claude-opus-5-5\":{\"outputTokens\":487}}}"
"#;

const FRESH_PER_TURN: u64 = 12 + 8_211 + 487;

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
}

fn state_at(dir: &Path, db: Database) -> AppState {
    let config = Config {
        data_dir: dir.to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    AppState::new(config.clone(), db, ToolRunner::new(dir.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
}

/// Ann and Bob on the fake claude in channel "desk", Bob leading.
async fn setup(dir: &Path) -> Env {
    fake_cli();
    let state = state_at(dir, Database::init(dir).await.unwrap());
    let app = create_router(state.clone());
    let bot = |name: &str| {
        json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
            "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"})
    };
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "lead_kwant_id": bob, "member_kwants": [ann]}))).await;
    Env { app, state, channel: c["id"].as_str().unwrap().to_string(), ann }
}

impl Env {
    async fn say(&self, prompt: &str) {
        ok(&self.app, "POST", "/api/chat/send", Some(json!({"channel_id": self.channel, "prompt": prompt, "sender_id": "user"}))).await;
    }

    async fn messages(&self) -> Vec<Message> {
        self.state.db.list_messages(&self.channel, 200).await.unwrap()
    }

    async fn replies(&self) -> usize {
        self.messages().await.iter().filter(|m| m.sender_role == "kwant" && m.sender_id == "Ann").count()
    }

    async fn person_messages(&self) -> usize {
        self.messages().await.iter().filter(|m| m.sender_role == "user").count()
    }

    async fn notices(&self) -> Vec<Message> {
        self.messages().await.into_iter().filter(|m| meta(m)["kind"] == "spend_stopped").collect()
    }

    async fn today(&self) -> Value {
        ok(&self.app, "GET", "/api/usage/today", None).await
    }

    async fn set(&self, body: Value) {
        ok(&self.app, "POST", "/api/settings", Some(body)).await;
    }

    /// Sends `prompt` to Ann and waits for her reply.
    async fn turn(&self, prompt: &str) {
        let before = self.replies().await;
        self.say(&format!("@Ann {prompt}")).await;
        for _ in 0..200 {
            if self.replies().await > before {
                // the turn is booked right after the reply is saved
                tokio::time::sleep(Duration::from_millis(300)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no reply to {prompt}: {:#?}", self.messages().await.iter().map(|m| (&m.sender_id, &m.text)).collect::<Vec<_>>());
    }

    /// Sends `prompt` while stopped: it's saved, and no turn starts.
    async fn held(&self, prompt: &str) {
        let (people, replies) = (self.person_messages().await, self.replies().await);
        self.say(&format!("@Ann {prompt}")).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(self.person_messages().await, people + 1, "the message is saved");
        assert_eq!(self.replies().await, replies, "no turn while stopped");
    }
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// Spec: off by default, and a database from before 0.7.4 starts at 0.
#[tokio::test]
async fn off_by_default_and_upgraded_db_starts_at_zero() {
    let tmp = tempfile::tempdir().unwrap();
    // A 0.7.3 database: everything but the spending stop's tables
    drop(Database::init(tmp.path()).await.unwrap());
    let url = format!("sqlite://{}", tmp.path().join("kwnt.db").display());
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    for t in ["daily_spend", "spend_stop_notices"] {
        sqlx::query(&format!("DROP TABLE {t}")).execute(&pool).await.unwrap();
    }
    pool.close().await;

    let env = setup(tmp.path()).await;
    let settings = ok(&env.app, "GET", "/api/settings", None).await;
    assert_eq!((settings["daily_spend_stop_tokens"].as_u64(), settings["daily_spend_stop_usd"].as_f64()), (Some(0), Some(0.0)));
    let today = env.today().await;
    let keys: Vec<&str> = today.as_object().unwrap().keys().map(String::as_str).collect();
    for k in ["today_tokens", "today_usd", "stop_tokens", "stop_usd", "stopped", "day_ends_at"] {
        assert!(keys.contains(&k), "{k} in {today}");
    }
    assert_eq!(today["today_tokens"], 0);
    assert_eq!(today["today_usd"], 0.0);
    assert_eq!(today["stopped"], false);
    assert_eq!(today["timezone"], "UTC");

    // Off, turns run however much they use, and they're still counted
    env.turn("one").await;
    env.turn("two").await;
    let today = env.today().await;
    assert_eq!(today["today_tokens"].as_u64(), Some(2 * FRESH_PER_TURN));
    assert_eq!(today["stopped"], false);
    assert!(env.notices().await.is_empty());
}

/// Spec: over the token limit a person message is saved, no turn starts, and one notice per chat
/// per day says so; a second message the same day gets no second notice.
#[tokio::test]
async fn over_token_limit_saves_message_no_turn_one_notice() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.set(json!({"daily_spend_stop_tokens": FRESH_PER_TURN})).await;
    assert_eq!(ok(&env.app, "GET", "/api/settings", None).await["daily_spend_stop_tokens"].as_u64(), Some(FRESH_PER_TURN));

    env.turn("first").await; // reaches the limit; a turn already running finishes
    let today = env.today().await;
    assert_eq!((today["stopped"].as_bool(), today["over"].as_str()), (Some(true), Some("tokens")), "{today}");

    env.held("second").await;
    let notices = env.notices().await;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].text, "Today's spending stop: used 8k of 8k tokens. Continue today, or change it in Settings › All spaces › Usage.");
    let m = meta(&notices[0]);
    assert_eq!((m["over"].as_str(), m["used_tokens"].as_u64(), m["stop_tokens"].as_u64()), (Some("tokens"), Some(FRESH_PER_TURN), Some(FRESH_PER_TURN)));
    assert_eq!(m["day_ends_at"], today["day_ends_at"]);

    env.held("third").await;
    assert_eq!(env.notices().await.len(), 1, "one notice a day");

    // Needs you carries one item while it holds
    let waiting = ok(&env.app, "GET", "/api/waiting", None).await;
    let items: Vec<&Value> = waiting["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "spend_stopped").collect();
    assert_eq!(items.len(), 1, "{waiting}");
    assert_eq!(items[0]["continue_url"], "/api/usage/today/continue");

    // Raising the limit lifts it
    env.set(json!({"daily_spend_stop_tokens": 3 * FRESH_PER_TURN})).await;
    assert_eq!(env.today().await["stopped"], false);
    env.turn("fourth").await;
    let waiting = ok(&env.app, "GET", "/api/waiting", None).await;
    assert!(!waiting["items"].as_array().unwrap().iter().any(|i| i["kind"] == "spend_stopped"));
}

/// Spec: a routine and a goal check skip with `spend_stopped` in their own history, and no AutoFix
/// attempt starts.
#[tokio::test]
async fn routine_and_goal_check_skip_with_spend_stopped_no_autofix() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let since = chrono::Utc::now() - chrono::Duration::minutes(1);
    env.set(json!({"daily_spend_stop_tokens": 1000})).await;
    spend_stop::book(&env.state, 1500, None).await;
    assert_eq!(env.today().await["stopped"], true);

    // A routine run now
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": env.ann, "name": "Digest", "instructions": "Summarise yesterday.",
            "schedule": {"type": "daily", "time": "09:00", "tz": "UTC"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    let rid = r["id"].as_str().unwrap().to_string();
    for _ in 0..2 {
        let (s, _) = call(&env.app, "POST", &format!("/api/routines/{rid}/run"), None).await;
        assert_eq!(s, StatusCode::ACCEPTED);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let runs = ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await;
    let runs = runs.as_array().unwrap();
    assert_eq!(runs.len(), 2, "{runs:?}");
    for run in runs {
        assert_eq!(run["status"], "skipped", "{run}");
        assert!(run["error"].as_str().unwrap().starts_with("spend_stopped"), "{run}");
    }
    assert_eq!(env.replies().await, 0);
    assert!(env.state.db.list_messages(&env.ann, 50).await.unwrap().iter().all(|m| m.sender_role != "kwant"), "no turn in Ann's chat");

    // A due goal check
    let goal = NewGoal {
        space_id: kwnt::db::spaces::default_space_id(),
        title: "Site is up".into(),
        description: String::new(),
        kwant_id: env.ann.clone(),
        folder: String::new(),
        mode: "suggest".into(),
        interval_minutes: 60,
        confirm_failures: 1,
        budget_tokens: 0,
        max_fixes_per_day: 3,
        priority: 2,
        created_by: "person".into(),
        draft: false,
        checks: vec![NewGoalCheck {
            label: "up".into(),
            kind: "http".into(),
            spec: json!({ "url": "http://127.0.0.1:9/" }),
            metric: Some("status".into()),
            operator: Some("eq".into()),
            target: Some("200".into()),
            severity: "required".into(),
            freshness_minutes: 120,
        }],
    };
    let Created::Goal(g) = env.state.db.create_goal(&goal, now_ms() - 1000).await.unwrap() else { panic!("goal") };
    assert!(runner::tick(&env.state).await.is_empty(), "nothing leased");
    assert!(runner::tick(&env.state).await.is_empty());
    let events = env.state.db.goal_events(&g.id, 100).await.unwrap();
    let skipped: Vec<_> = events.iter().filter(|e| e.kind == "skipped").collect();
    assert_eq!(skipped.len(), 1, "once a day: {events:?}");
    assert!(skipped[0].detail.contains("\"reason\":\"spend_stopped\""), "{}", skipped[0].detail);
    let after = env.state.db.get_goal(&g.id).await.unwrap().unwrap();
    assert_eq!((after.status.as_str(), after.fail_streak, after.checking_since), ("checking", 0, None));

    // No AutoFix: no failure recorded, no attempt
    let (attempts, _, _, classes) = env.state.db.autofix_stats(&kwnt::db::spaces::default_space_id(), since).await.unwrap();
    assert_eq!((attempts, classes), (0, vec![]));

    // Continue today: the routine runs again
    ok(&env.app, "POST", "/api/usage/today/continue", None).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/routines/{rid}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    for _ in 0..200 {
        let runs = ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await;
        if runs[0]["status"] == "ok" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the routine didn't run after Continue");
}

/// Spec: Continue today lifts the stop until the day ends; the next day it starts again from 0.
#[tokio::test]
async fn continue_today_lifts_and_next_day_resets() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let noon = chrono::DateTime::parse_from_rfc3339("2026-11-06T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
    env.state.spend.set(Some(noon));
    env.set(json!({"daily_spend_stop_tokens": FRESH_PER_TURN})).await;
    env.turn("first").await;
    env.held("second").await;
    assert_eq!(env.notices().await.len(), 1);

    let today = ok(&env.app, "POST", "/api/usage/today/continue", None).await;
    assert_eq!((today["stopped"].as_bool(), today["continued"].as_bool()), (Some(false), Some(true)), "{today}");
    assert_eq!(today["day"], "2026-11-06");
    assert_eq!(today["day_ends_at"], "2026-11-07T00:00:00Z");
    env.turn("third").await;
    assert_eq!(env.today().await["today_tokens"].as_u64(), Some(2 * FRESH_PER_TURN));

    // Changing a limit takes Continue back: the new limit applies at once
    env.set(json!({"daily_spend_stop_tokens": FRESH_PER_TURN + 1})).await;
    let today = env.today().await;
    assert_eq!((today["stopped"].as_bool(), today["continued"].as_bool()), (Some(true), Some(false)), "{today}");

    // The next day: back to 0, turns run, and a new day may have its own notice
    env.state.spend.set(Some(noon + chrono::Duration::days(1)));
    let today = env.today().await;
    assert_eq!((today["day"].as_str(), today["today_tokens"].as_u64(), today["stopped"].as_bool()), (Some("2026-11-07"), Some(0), Some(false)));
    env.turn("tomorrow").await;
    env.turn("tomorrow again").await;
    env.held("tomorrow, stopped").await;
    assert_eq!(env.notices().await.len(), 2, "one notice on each day");
    // Yesterday's totals stay where they were
    assert_eq!(env.state.db.spend_day("2026-11-06").await.unwrap().tokens as u64, 2 * FRESH_PER_TURN);

    // The day follows the person's time zone
    env.set(json!({"timezone": "Asia/Tokyo"})).await;
    let today = env.today().await;
    if today["timezone"] == "Asia/Tokyo" {
        assert_eq!((today["day"].as_str(), today["day_ends_at"].as_str()), (Some("2026-11-07"), Some("2026-11-07T15:00:00Z")));
    }
}

/// Spec: the USD limit counts only engines that report `cost_usd`.
#[tokio::test]
async fn usd_limit_counts_only_engines_reporting_cost() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.set(json!({"daily_spend_stop_usd": 0.5})).await;
    env.turn("priced").await;
    env.turn("NOCOST one").await;
    env.turn("NOCOST two").await;
    let today = env.today().await;
    assert_eq!(today["today_usd"].as_f64(), Some(0.42), "{today}");
    assert_eq!(today["today_tokens"].as_u64(), Some(3 * FRESH_PER_TURN), "tokens count for every engine");
    assert_eq!(today["stopped"], false, "the uncosted turns added no dollars");

    env.set(json!({"daily_spend_stop_usd": 0.42})).await;
    let today = env.today().await;
    assert_eq!((today["stopped"].as_bool(), today["over"].as_str(), today["stop_usd"].as_f64()), (Some(true), Some("usd"), Some(0.42)));
    env.held("NOCOST stopped").await;
    assert!(env.notices().await[0].text.contains("used $0.42 of $0.42"));

    // Bad limits are refused, and nothing changes
    for bad in [json!({"daily_spend_stop_usd": -1}), json!({"daily_spend_stop_usd": 100_001}), json!({"daily_spend_stop_tokens": 10_000_000_001u64}), json!({"daily_spend_stop_tokens": -5})] {
        let (s, v) = call(&env.app, "POST", "/api/settings", Some(bad.clone())).await;
        assert!(s.is_client_error(), "{bad}: {s} {v}");
    }
    assert_eq!(env.today().await["stop_usd"].as_f64(), Some(0.42));
}

/// Spec: a restart keeps the day's total.
#[tokio::test]
async fn restart_keeps_days_total() {
    let tmp = tempfile::tempdir().unwrap();
    {
        let env = setup(tmp.path()).await;
        env.set(json!({"daily_spend_stop_tokens": FRESH_PER_TURN})).await;
        env.turn("before the restart").await;
        assert_eq!(env.today().await["stopped"], true);
    }
    let state = state_at(tmp.path(), Database::init(tmp.path()).await.unwrap());
    let app = create_router(state.clone());
    let today = ok(&app, "GET", "/api/usage/today", None).await;
    assert_eq!((today["today_tokens"].as_u64(), today["today_usd"].as_f64(), today["stopped"].as_bool()), (Some(FRESH_PER_TURN), Some(0.42), Some(true)), "{today}");
}
