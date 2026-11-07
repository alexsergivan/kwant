//! The daily spending stop (0.7.4, next-074/spend-stop/spec.md). A fake claude CLI stands in for
//! every engine: it reports 8,710 fresh tokens a turn and $0.42, or no cost at all when the prompt
//! says NOCOST (like an engine that doesn't report `cost_usd`); with FAILMID it streams 145 fresh
//! tokens and fails before a result. The day moves on through the
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
case "$last" in *FAILMID*)
  echo '{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1","usage":{"input_tokens":5,"cache_creation_input_tokens":100,"cache_read_input_tokens":9000,"output_tokens":1}}}}'
  echo '{"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":40}}}'
  echo "the engine broke mid-turn" >&2
  exit 1 ;;
esac
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

/// Spec: a routine skips with `spend_stopped` in its own history, and no AutoFix attempt starts.
/// A goal check costs no tokens, so it keeps running (Ada's review of #375: an http check against
/// a stand-in server is checked and met during the stop).
#[tokio::test]
async fn routine_skips_with_spend_stopped_goal_check_still_runs_no_autofix() {
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

    // A due goal check runs: no bot, no tokens
    let site = site().await;
    let goal = http_goal(&env, &site);
    let Created::Goal(g) = env.state.db.create_goal(&goal, now_ms() - 1000).await.unwrap() else { panic!("goal") };
    assert_eq!(runner::tick(&env.state).await, vec![g.id.clone()], "leased during the stop");
    let after = wait_goal_settled(&env, &g.id).await;
    assert_eq!((after.status.as_str(), after.fail_streak), ("met", 0), "{after:?}");
    let events = env.state.db.goal_events(&g.id, 100).await.unwrap();
    assert!(events.iter().all(|e| e.kind != "skipped"), "{events:?}");
    assert_eq!(env.today().await["today_tokens"].as_u64(), Some(1500), "the check booked nothing");

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

/// A stand-in site that answers 200 on 127.0.0.1 (a free port, never kwant's own).
async fn site() -> String {
    let app = Router::new().route("/", axum::routing::get(|| async { "ok" }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/")
}

/// A person's goal that `url` answers 200, checked by Ann.
fn http_goal(env: &Env, url: &str) -> NewGoal {
    NewGoal {
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
            spec: json!({ "url": url }),
            metric: Some("status".into()),
            operator: Some("eq".into()),
            target: Some("200".into()),
            severity: "required".into(),
            freshness_minutes: 120,
        }],
    }
}

async fn wait_goal_settled(env: &Env, id: &str) -> kwnt::db::goals::Goal {
    for _ in 0..200 {
        let g = env.state.db.get_goal(id).await.unwrap().unwrap();
        if g.checking_since.is_none() {
            return g;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the goal check never finished");
}

/// Takes every turn slot Ann has, so her next turn queues (and meets the stop's second check).
fn fill_slots(state: &AppState, kwant_id: &str) -> Vec<kwnt::server::turns::Slot> {
    let mut slots = Vec::new();
    while let Ok(s) = state.turns.limiter.try_acquire(kwant_id) {
        slots.push(s);
    }
    slots
}

async fn wait_queued(state: &AppState, kwant_id: &str) {
    for _ in 0..200 {
        if state.turns.active_turns().iter().any(|t| t.kwant_id == kwant_id && t.state == "queued") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no turn of {kwant_id} queued");
}

async fn task_events(state: &AppState, tid: &str, kind: &str) -> Vec<kwnt::db::missions::TaskEvent> {
    state.db.task_events(tid, 200).await.unwrap().into_iter().filter(|e| e.kind == kind).collect()
}

/// Ada's review of #375 (2): the first thing held each day sends one `spend_stopped`
/// notification; a second stopped turn the same day sends none, the next day may send one again.
#[tokio::test]
async fn first_trip_each_day_sends_one_notification() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let noon = chrono::DateTime::parse_from_rfc3339("2026-11-06T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
    env.state.spend.set(Some(noon));
    let mut rx = env.state.event_sender.subscribe();
    let mut sent = move || {
        let mut n = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let kwnt::RuntimeEvent::Notify { kind, title, body, .. } = ev {
                if kind == "spend_stopped" {
                    n.push((title, body));
                }
            }
        }
        n
    };
    env.set(json!({"daily_spend_stop_tokens": FRESH_PER_TURN})).await;
    env.turn("first").await;
    assert!(sent().is_empty(), "reaching the limit isn't a trip: nothing was held yet");

    env.held("second").await;
    env.held("third").await;
    let n = sent();
    assert_eq!(n.len(), 1, "two stopped turns, one notification: {n:?}");
    assert_eq!(n[0].0, "Today's spending stop is on");
    assert!(n[0].1.starts_with("Used 8k of 8k tokens."), "{}", n[0].1);

    // A routine held the same day adds none
    let r = ok(&env.app, "POST", "/api/routines", Some(json!({"kwant_id": env.ann, "name": "Digest", "instructions": "Sum up.",
        "schedule": {"type": "daily", "time": "09:00", "tz": "UTC"}}))).await;
    call(&env.app, "POST", &format!("/api/routines/{}/run", r["id"].as_str().unwrap()), None).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(sent().is_empty());

    // The next day starts at 0; once it trips, one more
    env.state.spend.set(Some(noon + chrono::Duration::days(1)));
    env.turn("tomorrow").await;
    env.held("tomorrow, stopped").await;
    env.held("tomorrow, again").await;
    assert_eq!(sent().len(), 1, "one on each day");
}

/// Ada's review of #375 (3a): an AutoFix retry due during the stop stays scheduled, its task's
/// history says `autofix_held` once a day, and it runs after Continue today.
#[tokio::test]
async fn autofix_attempt_waits_during_stop_and_runs_after_continue() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.set(json!({"autofix": "on"})).await;
    let t = ok(&env.app, "POST", "/api/tasks", Some(json!({"title": "Fix it", "acceptance": "works", "owner_kwant_id": env.ann}))).await;
    let tid = t["id"].as_str().or(t["task"]["id"].as_str()).unwrap().to_string();
    // It failed on a rate limit: AutoFix schedules a wait-and-retry
    env.state.db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'failed' WHERE id = '{tid}'")).await.unwrap();
    use kwnt::server::missions::autofix;
    let f = autofix::record(&env.state, &tid, "turn", "error", "API Error: 429 rate_limit_error: This request would exceed your rate limit", "rate limited").await;
    assert!(f.is_some());
    let attempts = env.state.db.attempts_of(&tid).await.unwrap();
    assert_eq!((attempts.len(), attempts[0].status.as_str(), attempts[0].strategy.as_str()), (1, "scheduled", "wait_and_retry"));
    let aid = attempts[0].id.clone();
    env.state.db.exec_raw_for_test(&format!("UPDATE autofix_attempts SET not_before = '2026-01-01T00:00:00Z' WHERE id = '{aid}'")).await.unwrap();

    env.set(json!({"daily_spend_stop_tokens": 1000})).await;
    spend_stop::book(&env.state, 1500, None).await;
    autofix::run_due(&env.state).await;
    autofix::run_due(&env.state).await;
    assert_eq!(env.state.db.get_attempt(&aid).await.unwrap().unwrap().status, "scheduled", "still scheduled");
    assert_eq!(env.state.db.get_task(&tid).await.unwrap().unwrap().state, "failed");
    let held = task_events(&env.state, &tid, "autofix_held").await;
    assert_eq!(held.len(), 1, "once a day: {held:?}");
    assert!(held[0].detail.contains("\"reason\":\"spend_stopped\"") && held[0].detail.contains(&aid), "{}", held[0].detail);

    ok(&env.app, "POST", "/api/usage/today/continue", None).await;
    autofix::run_due(&env.state).await;
    assert_eq!(env.state.db.get_attempt(&aid).await.unwrap().unwrap().status, "running");
    assert_eq!(env.state.db.get_task(&tid).await.unwrap().unwrap().state, "ready");
    assert_eq!(task_events(&env.state, &tid, "autofix_held").await.len(), 1);
}

/// Ada's review of #375 (3b): a mission task whose turn the stop catches at its turn (it queued,
/// and the stop came down meanwhile) goes back to `ready` with a `skipped` event: no failure, no
/// attempt used.
#[tokio::test]
async fn mission_task_caught_at_its_turn_goes_back_to_ready() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let since = chrono::Utc::now() - chrono::Duration::minutes(1);
    env.state.db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "off").await.unwrap();
    env.set(json!({"mission_board": true, "autofix": "on", "daily_spend_stop_tokens": 1000})).await;
    let slots = fill_slots(&env.state, &env.ann);
    assert!(!slots.is_empty());
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": env.ann, "budget_tokens": 900_000,
                      "channel_id": env.channel, "tasks": [{"title": "A", "owner": env.ann}]});
    let m = ok(&env.app, "POST", "/api/missions", Some(body)).await;
    let mid = m["id"].as_str().or(m["mission"]["id"].as_str()).unwrap().to_string();
    let detail = ok(&env.app, "GET", &format!("/api/missions/{mid}"), None).await;
    let tid = detail["tasks"][0]["id"].as_str().unwrap().to_string();
    let mission = env.state.db.get_mission(&mid).await.unwrap().unwrap();
    std::fs::create_dir_all(&mission.workspace_path).unwrap();
    if !kwnt::server::missions::runner::is_running(&env.state, &mid) {
        let (started, why) = kwnt::server::missions::runner::run_mission(&env.state, &mission, 1).await;
        assert_eq!(started.len(), 1, "{why:?}");
    }
    wait_queued(&env.state, &env.ann).await;
    assert_eq!(env.state.db.get_task(&tid).await.unwrap().unwrap().state, "working");

    spend_stop::book(&env.state, 1500, None).await;
    drop(slots);
    for _ in 0..200 {
        if !kwnt::server::missions::runner::is_running(&env.state, &mid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let t = env.state.db.get_task(&tid).await.unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.attempts), ("ready", 0), "back to ready, no attempt used");
    let skipped = task_events(&env.state, &tid, "skipped").await;
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].detail.contains("\"reason\":\"spend_stopped\""), "{}", skipped[0].detail);
    assert!(env.state.db.failures_of(&tid).await.unwrap().is_empty(), "not a failure");
    assert!(env.state.db.attempts_of(&tid).await.unwrap().is_empty(), "nothing for AutoFix");
    let (attempts, _, _, classes) = env.state.db.autofix_stats(&kwnt::db::spaces::default_space_id(), since).await.unwrap();
    assert_eq!((attempts, classes), (0, vec![]));
    assert_eq!(env.replies().await, 0, "no turn ran");
}

/// Ada's review of #375 (3c): a routine run whose stop comes down while its turn waits for a slot
/// is recorded `skipped` (not `error`) and starts no AutoFix.
#[tokio::test]
async fn routine_stopped_mid_start_is_skipped_not_error() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let since = chrono::Utc::now() - chrono::Duration::minutes(1);
    env.set(json!({"autofix": "on", "daily_spend_stop_tokens": 1000})).await;
    let r = ok(&env.app, "POST", "/api/routines", Some(json!({"kwant_id": env.ann, "name": "Digest", "instructions": "Sum up.",
        "schedule": {"type": "daily", "time": "09:00", "tz": "UTC"}}))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let slots = fill_slots(&env.state, &env.ann);
    let (s, _) = call(&env.app, "POST", &format!("/api/routines/{rid}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    wait_queued(&env.state, &env.ann).await;
    spend_stop::book(&env.state, 1500, None).await;
    drop(slots);
    let mut run = Value::Null;
    for _ in 0..200 {
        run = ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await[0].clone();
        if run["status"] != "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(run["status"], "skipped", "{run}");
    assert!(run["error"].as_str().unwrap().starts_with("spend_stopped"), "{run}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (attempts, _, _, classes) = env.state.db.autofix_stats(&kwnt::db::spaces::default_space_id(), since).await.unwrap();
    assert_eq!((attempts, classes), (0, vec![]), "no AutoFix");
    let runs = ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await;
    assert_eq!(runs.as_array().unwrap().len(), 1, "no retry run: {runs}");
    assert_eq!(env.replies().await, 0);
}

/// Ada's review of #375 (4): a turn that fails mid-way still counts the tokens its engine
/// reported before failing (the Claude CLI streams them).
#[tokio::test]
async fn failed_turn_counts_usage_reported_so_far() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.say("@Ann FAILMID please").await;
    let mut today = Value::Null;
    for _ in 0..100 {
        today = env.today().await;
        if today["today_tokens"].as_u64() != Some(0) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // input 5 + cache writes 100 + output 40 (cache reads aren't fresh)
    assert_eq!((today["today_tokens"].as_u64(), today["today_usd"].as_f64()), (Some(145), Some(0.0)), "{today}");
    assert!(env.messages().await.iter().all(|m| m.text != "answer"), "it failed: no answer");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(env.today().await["today_tokens"].as_u64(), Some(145), "booked once");
}
