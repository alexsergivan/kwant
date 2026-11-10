//! G2: a monthly budget per bot (release-090/g2-contract.md). The fake claude CLI from the daily
//! spending stop's tests reports 8,710 fresh tokens and $0.42 a turn (no cost with NOCOST). The
//! month moves on through the stop's clock seam, never by waiting. Its own test binary: it
//! changes PATH.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Message, Database},
    router::LayaRouter,
    server::{
        bot_budget,
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, parked, AppState,
    },
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
# Only this turn's request counts: earlier NOCOST messages are in the history part of the prompt
req="${last##*Current User Request:}"
cost=',"total_cost_usd":0.42'
case "$req" in *NOCOST*) cost="" ;; esac
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"answer\"$cost,\"usage\":{\"input_tokens\":12,\"cache_creation_input_tokens\":8211,\"cache_read_input_tokens\":83544,\"output_tokens\":487},\"modelUsage\":{\"claude-opus-5-5\":{\"outputTokens\":487}}}"
"#;

const FRESH: u64 = 12 + 8_211 + 487;

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

async fn call_as(app: &Router, method: &str, uri: &str, body: Option<Value>, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    call_as(app, method, uri, body, None).await
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

struct Env {
    app: Router,
    state: AppState,
    channel: String,
    ann: String,
    bob: String,
}

/// Ann and Bob on the fake claude in channel "desk", Bob leading.
async fn setup(dir: &Path) -> Env {
    fake_cli();
    let config = Config {
        data_dir: dir.to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let db = Database::init(dir).await.unwrap();
    let state = AppState::new(config.clone(), db, ToolRunner::new(dir.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let app = create_router(state.clone());
    let bot = |name: &str| {
        json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
            "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"})
    };
    let ann = ok(&app, "POST", "/api/kwants", Some(bot("Ann"))).await["id"].as_str().unwrap().to_string();
    let bob = ok(&app, "POST", "/api/kwants", Some(bot("Bob"))).await["id"].as_str().unwrap().to_string();
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "lead_kwant_id": bob, "member_kwants": [ann]}))).await;
    Env { app, state, channel: c["id"].as_str().unwrap().to_string(), ann, bob }
}

impl Env {
    async fn say(&self, prompt: &str) {
        ok(&self.app, "POST", "/api/chat/send", Some(json!({"channel_id": self.channel, "prompt": prompt, "sender_id": "user"}))).await;
    }

    async fn messages(&self) -> Vec<Message> {
        self.state.db.list_messages(&self.channel, 200).await.unwrap()
    }

    async fn replies(&self, who: &str) -> usize {
        self.messages().await.iter().filter(|m| m.sender_role == "kwant" && m.sender_id == who).count()
    }

    /// Notices of `kind` in the channel.
    async fn notices(&self, kind: &str) -> Vec<Message> {
        self.messages().await.into_iter().filter(|m| meta(m)["kind"] == kind).collect()
    }

    /// Notices of `kind` in Ann's 1:1.
    async fn direct(&self, kind: &str) -> Vec<Message> {
        self.state.db.list_messages(&self.ann, 200).await.unwrap().into_iter().filter(|m| meta(m)["kind"] == kind).collect()
    }

    async fn budget(&self, kwant: &str) -> Value {
        ok(&self.app, "GET", &format!("/api/kwants/{kwant}/budget"), None).await
    }

    async fn set_budget(&self, kwant: &str, body: Value) -> Value {
        ok(&self.app, "POST", &format!("/api/kwants/{kwant}/budget"), Some(body)).await
    }

    async fn wait_reply(&self, who: &str, before: usize, what: &str) {
        for _ in 0..200 {
            if self.replies(who).await > before {
                // the turn is booked right after the reply is saved
                tokio::time::sleep(Duration::from_millis(300)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no reply from {who} to {what}: {:#?}", self.messages().await.iter().map(|m| (&m.sender_id, &m.text)).collect::<Vec<_>>());
    }

    /// Sends `prompt` to `who` and waits for the reply.
    async fn turn_with(&self, who: &str, prompt: &str) {
        let before = self.replies(who).await;
        self.say(&format!("@{who} {prompt}")).await;
        self.wait_reply(who, before, prompt).await;
    }

    async fn turn(&self, prompt: &str) {
        self.turn_with("Ann", prompt).await;
    }

    /// Sends `prompt` to Ann while she's paused: saved, and no turn starts.
    async fn held(&self, prompt: &str) {
        let people = self.messages().await.iter().filter(|m| m.sender_role == "user").count();
        let replies = self.replies("Ann").await;
        self.say(&format!("@Ann {prompt}")).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(self.messages().await.iter().filter(|m| m.sender_role == "user").count(), people + 1, "the message is saved");
        assert_eq!(self.replies("Ann").await, replies, "no turn while paused");
    }

    async fn budget_parks(&self) -> Vec<kwnt::db::parked::ParkedTurn> {
        self.state.db.list_parked().await.unwrap().into_iter().filter(|p| p.reason == "bot_budget" && p.status == "pending").collect()
    }

    /// Runs the parked turns that are due now and waits for Ann's reply to the first one.
    async fn run_parked(&self) {
        let before = self.replies("Ann").await;
        assert_eq!(parked::run_due(&self.state, Utc::now() + chrono::Duration::seconds(1)).await, 1, "one held turn runs");
        self.wait_reply("Ann", before, "the held turn").await;
    }

    async fn waiting_items(&self) -> Vec<Value> {
        let w = ok(&self.app, "GET", "/api/waiting", None).await;
        w["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "bot_budget_stopped").cloned().collect()
    }
}

/// Counting: fresh tokens and reported dollars per bot, the same numbers the daily stop books.
#[tokio::test]
async fn counts_each_bots_fresh_tokens_like_the_daily_stop() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    let b = env.budget(&env.ann).await;
    assert_eq!((b["state"].as_str(), b["used_tokens"].as_u64(), b["usage_reported"].clone()), (Some("off"), Some(0), Value::Null), "{b}");

    env.turn("one").await;
    env.turn("two").await;
    env.turn("NOCOST three").await;
    env.turn_with("Bob", "hello").await;

    let b = env.budget(&env.ann).await;
    assert_eq!(b["used_tokens"].as_u64(), Some(3 * FRESH), "{b}");
    assert_eq!(b["today_tokens"].as_u64(), Some(3 * FRESH));
    assert!((b["used_usd"].as_f64().unwrap() - 0.84).abs() < 1e-9, "only reported cost counts: {b}");
    assert_eq!((b["usage_reported"].as_bool(), b["cost_reported"].as_bool(), b["some_unreported"].as_bool()), (Some(true), Some(true), Some(false)));
    assert_eq!(b["state"], "off", "no budget set: counted, never paused");
    assert_eq!(b["engine"], "claude");
    assert_eq!(env.budget(&env.bob).await["used_tokens"].as_u64(), Some(FRESH), "per bot");

    // Every booked token lands in exactly one bot's day: the bots add up to the daily stop's total
    let today = ok(&env.app, "GET", "/api/usage/today", None).await;
    let day = today["day"].as_str().unwrap().to_string();
    assert_eq!(today["today_tokens"].as_u64(), Some(4 * FRESH));
    assert_eq!(env.state.db.bot_spend_day_total(&day).await.unwrap() as u64, 4 * FRESH);

    let all = ok(&env.app, "GET", "/api/usage/budgets", None).await;
    let names: Vec<&str> = all["bots"].as_array().unwrap().iter().map(|b| b["kwant_name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Ann") && names.contains(&"Bob"), "{all}");
    assert_eq!(all["month"], b["month"]);
}

/// 80%: one note in the bot's 1:1 when a turn crosses it, never again for the same budget, and
/// not when the person saves a budget the bot is already past 80% of.
#[tokio::test]
async fn warns_once_when_a_turn_crosses_80_percent() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.state.spend.set(Some(at("2026-11-06T12:00:00Z")));
    let budget = FRESH * 6 / 5; // one turn is 83%
    let b = env.set_budget(&env.ann, json!({"budget_tokens": budget})).await;
    assert_eq!((b["state"].as_str(), b["budget_tokens"].as_u64()), (Some("under"), Some(budget)), "{b}");

    env.turn("first").await;
    let b = env.budget(&env.ann).await;
    assert_eq!(b["state"], "warn", "{b}");
    assert_eq!(b["percent"].as_f64(), Some(83.3));
    let warns = env.direct("bot_budget_warn").await;
    assert_eq!(warns.len(), 1);
    assert_eq!(warns[0].text, "Ann has used 80% of its November budget. At 100% it stops starting new work.");

    // Raised, then set back and lowered: still past 80%, but no second note
    env.set_budget(&env.ann, json!({"budget_tokens": 10 * FRESH})).await;
    assert_eq!(env.budget(&env.ann).await["state"], "under");
    env.set_budget(&env.ann, json!({"budget_tokens": budget})).await;
    env.set_budget(&env.ann, json!({"budget_tokens": FRESH * 11 / 10})).await;
    assert_eq!(env.budget(&env.ann).await["state"], "warn");
    assert_eq!(env.direct("bot_budget_warn").await.len(), 1, "one 80% note");
    assert!(env.direct(bot_budget::STOPPED_KIND).await.is_empty());
}

/// 100%: the turn that crosses finishes; one pause, one notice in the 1:1 and one per chat; the
/// next message is saved and held (folded into one held turn) while other bots keep working;
/// Continue releases it, undoing Continue pauses again, and raising the budget releases again.
#[tokio::test]
async fn crossing_turn_finishes_next_is_held_until_continue_or_raise() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.state.spend.set(Some(at("2026-11-06T12:00:00Z")));
    env.set_budget(&env.ann, json!({"budget_tokens": FRESH})).await;

    env.turn("first").await; // starts under the budget, reaches it, and finishes
    let b = env.budget(&env.ann).await;
    assert_eq!((b["state"].as_str(), b["over"].as_str(), b["percent"].as_f64()), (Some("paused"), Some("tokens"), Some(100.0)), "{b}");
    assert!(b["paused_since"].is_string(), "{b}");
    assert_eq!(b["resets_at"], "2026-12-01T00:00:00Z");
    let stops = env.direct(bot_budget::STOPPED_KIND).await;
    assert_eq!(stops.len(), 1, "one notice when the pause starts");
    assert_eq!(
        stops[0].text,
        "Ann stopped at its monthly budget. It finished what it was doing and won't start anything new until you raise the budget, continue, or December starts."
    );
    assert_eq!(meta(&stops[0])["detail"], "8k of 8k fresh tokens in November");

    env.held("second").await;
    env.held("third").await;
    assert_eq!(env.notices(bot_budget::STOPPED_KIND).await.len(), 1, "one notice per chat per pause");
    let parks = env.budget_parks().await;
    assert_eq!(parks.len(), 1, "held messages fold into one turn: {parks:?}");
    assert!(parks[0].prompt.contains("second") && parks[0].prompt.contains("third"), "{}", parks[0].prompt);
    assert_eq!(parks[0].kwant_id, env.ann);
    let items = env.waiting_items().await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["continue_url"], format!("/api/kwants/{}/budget/continue", env.ann));
    assert_eq!(env.direct(bot_budget::STOPPED_KIND).await.len(), 1, "still one pause");

    // Only Ann is paused: Bob works, and the space-wide daily stop isn't involved
    env.turn_with("Bob", "still here?").await;
    assert_eq!(ok(&env.app, "GET", "/api/usage/today", None).await["stopped"], false);

    // Continue this month
    let b = ok(&env.app, "POST", &format!("/api/kwants/{}/budget/continue", env.ann), None).await;
    assert_eq!((b["state"].as_str(), b["continued_month"].as_str()), (Some("continued"), Some("2026-11")), "{b}");
    assert_eq!(env.direct("bot_budget_continued").await.len(), 1);
    assert!(env.waiting_items().await.is_empty());
    env.run_parked().await;
    env.turn("fourth").await;

    // Undo Continue a few minutes later: paused again, a new pause with its own notice
    env.state.spend.set(Some(at("2026-11-06T12:05:00Z")));
    let b = ok(&env.app, "DELETE", &format!("/api/kwants/{}/budget/continue", env.ann), None).await;
    assert_eq!(b["state"], "paused", "{b}");
    assert_eq!(env.direct(bot_budget::STOPPED_KIND).await.len(), 2);
    env.held("fifth").await;
    assert_eq!(env.notices(bot_budget::STOPPED_KIND).await.len(), 2, "the new pause notices the chat once");
    assert_eq!(env.budget_parks().await.len(), 1);

    // Raise the budget
    let b = env.set_budget(&env.ann, json!({"budget_tokens": 10 * FRESH})).await;
    assert_eq!(b["state"], "under", "{b}");
    let resumed = env.direct("bot_budget_resumed").await;
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].text, "Ann can start new work again.");
    env.run_parked().await;
    assert!(env.budget_parks().await.is_empty());
}

/// The USD budget counts only reported cost, and either limit pauses.
#[tokio::test]
async fn usd_budget_pauses_on_reported_cost() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.set_budget(&env.ann, json!({"budget_usd": 0.5})).await;
    env.turn("priced").await;
    env.turn("NOCOST free").await;
    assert_eq!(env.budget(&env.ann).await["state"], "warn");
    env.turn("priced again").await;
    let b = env.budget(&env.ann).await;
    assert_eq!((b["state"].as_str(), b["over"].as_str()), (Some("paused"), Some("usd")), "{b}");
    assert!(env.direct(bot_budget::STOPPED_KIND).await[0].text.contains("stopped at its monthly budget"));
    env.held("NOCOST stopped").await;
}

/// A new month releases the bot on its own; a routine held during the pause runs once, late,
/// with a note in its history.
#[tokio::test]
async fn new_month_releases_and_held_routine_runs_once_late() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.state.spend.set(Some(at("2026-11-30T12:00:00Z")));
    env.set_budget(&env.ann, json!({"budget_tokens": FRESH})).await;
    env.turn("last of the month").await;
    assert_eq!(env.budget(&env.ann).await["state"], "paused");

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
        let (s, v) = call(&env.app, "POST", &format!("/api/routines/{rid}/run"), None).await;
        assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    }
    let runs = ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await;
    let runs = runs.as_array().unwrap().clone();
    assert_eq!(runs.len(), 2, "{runs:?}");
    for run in &runs {
        assert_eq!(run["status"], "skipped", "{run}");
        assert!(run["error"].as_str().unwrap().starts_with("bot_budget"), "{run}");
    }

    // Still November: nothing to release
    bot_budget::release_due(&env.state).await;
    assert_eq!(env.budget(&env.ann).await["state"], "paused");
    assert_eq!(ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await.as_array().unwrap().len(), 2);

    // December
    env.state.spend.set(Some(at("2026-12-01T00:30:00Z")));
    let b = env.budget(&env.ann).await;
    assert_eq!((b["month"].as_str(), b["used_tokens"].as_u64(), b["state"].as_str()), (Some("2026-12"), Some(0), Some("under")), "{b}");
    bot_budget::release_due(&env.state).await;
    assert_eq!(env.direct("bot_budget_resumed").await.len(), 1);
    let mut late = Value::Null;
    for _ in 0..200 {
        let runs = ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await;
        if runs.as_array().unwrap().len() == 3 && runs[0]["status"] == "ok" {
            late = runs[0].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(late["note"], "Ran late: Ann was paused at its monthly budget.", "{late}");
    assert_eq!(late["trigger"], "schedule");

    // Released once: the next tick has nothing left
    bot_budget::release_due(&env.state).await;
    assert_eq!(ok(&env.app, "GET", &format!("/api/routines/{rid}/runs"), None).await.as_array().unwrap().len(), 3);
    assert_eq!(env.direct("bot_budget_resumed").await.len(), 1);
}

/// Only the person sets, continues or re-pauses a budget: a bot's turn token gets 403 and changes
/// nothing; reading stays open.
#[tokio::test]
async fn a_bot_token_cannot_change_a_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.set_budget(&env.ann, json!({"budget_tokens": FRESH})).await;
    env.turn("spend it").await;
    assert_eq!(env.budget(&env.ann).await["state"], "paused");

    let ann = env.state.db.get_kwant(&env.ann).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: ann.id.clone(),
        kwant_name: ann.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(env.channel.clone()), ..Default::default() }),
        turn_id: Some("t-budget".into()),
        ..Default::default()
    };
    let token = mcp_runtime::prepare_turn_with_self(&env.state, &ann, Some("t-budget".into()), None, Some(scope)).await.unwrap().1;

    for kwant in [&env.ann, &env.bob] {
        for (method, uri, body) in [
            ("POST", format!("/api/kwants/{kwant}/budget"), Some(json!({"budget_tokens": null}))),
            ("POST", format!("/api/kwants/{kwant}/budget/continue"), None),
            ("DELETE", format!("/api/kwants/{kwant}/budget/continue"), None),
        ] {
            let (s, v) = call_as(&env.app, method, &uri, body, Some(&token)).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
        }
    }
    let b = env.budget(&env.ann).await;
    assert_eq!((b["state"].as_str(), b["budget_tokens"].as_u64()), (Some("paused"), Some(FRESH)), "unchanged: {b}");
    assert!(env.direct("bot_budget_continued").await.is_empty());

    let (s, v) = call_as(&env.app, "GET", &format!("/api/kwants/{}/budget", env.ann), None, Some(&token)).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("paused")), "{v}");
    let (s, _) = call_as(&env.app, "GET", "/api/usage/budgets", None, Some(&token)).await;
    assert_eq!(s, StatusCode::OK);

    // The person can
    let (s, v) = call(&env.app, "POST", &format!("/api/kwants/{}/budget/continue", env.ann), None).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("continued")), "{v}");
}

/// Bad limits are refused and change nothing; null turns a limit off.
#[tokio::test]
async fn validates_limits_and_turns_off() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path()).await;
    env.set_budget(&env.ann, json!({"budget_tokens": 1000, "budget_usd": 2.345})).await;
    for bad in [
        json!({"budget_tokens": -1}),
        json!({"budget_tokens": 1.5}),
        json!({"budget_tokens": 10_000_000_001u64}),
        json!({"budget_usd": -0.01}),
        json!({"budget_usd": 100_001}),
        json!({"budget_tokens": "lots"}),
    ] {
        let (s, v) = call(&env.app, "POST", &format!("/api/kwants/{}/budget", env.ann), Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    let b = env.budget(&env.ann).await;
    assert_eq!((b["budget_tokens"].as_u64(), b["budget_usd"].as_f64()), (Some(1000), Some(2.35)), "{b}");

    let b = env.set_budget(&env.ann, json!({"budget_usd": null})).await;
    assert_eq!((b["budget_tokens"].as_u64(), b["budget_usd"].as_f64()), (Some(1000), Some(0.0)), "left out stays: {b}");
    let b = env.set_budget(&env.ann, json!({"budget_tokens": 0})).await;
    assert_eq!(b["state"], "off");
    let (s, _) = call(&env.app, "POST", &format!("/api/kwants/{}/budget/continue", env.ann), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "nothing to continue");
    let (s, _) = call(&env.app, "GET", "/api/kwants/nobody/budget", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
