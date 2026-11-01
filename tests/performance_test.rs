//! The Performance Market (Phase D1, report-only): a settled task is observed once, a re-settled
//! one replaces its row, the backfill runs once and never duplicates, and GET /api/performance
//! groups with a Wilson interval. Contract: docs/mission-control.md §17.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, missions, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app: create_router(state.clone()), bot: kwants[0].clone(), general, state, _tmp: tmp };
    let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"mission_board": true}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    e
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, v) = call(&e.app, method, uri, None, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (Value, bool, String) {
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc("tools/call", json!({"name": name, "arguments": args})))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
    let parsed = serde_json::from_str(&text).unwrap_or(Value::Null);
    (parsed, v["result"]["isError"].as_bool().unwrap_or(false), text)
}

/// A turn of `bot` in #general with its own tools; returns the turn token.
async fn turn(e: &Env, bot: &Kwant, turn_id: &str) -> String {
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn task(e: &Env, id: &str) -> Value {
    person(e, "GET", &format!("/api/tasks/{id}"), None).await
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

async fn claim(e: &Env, tid: &str) -> String {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let token = turn(e, &e.bot, &turn_id).await;
    let (v, err, text) = tool(e, &token, "task_claim", json!({"task_id": tid})).await;
    assert!(!err, "claim failed: {text} {v}");
    turn_id
}


use kwnt::db::performance::NewObservation;
use kwnt::db::usage::NewUsage;
use kwnt::db::missions::TaskChange;
use std::time::Duration;

async fn observed(e: &Env, tid: &str, result: &str) -> kwnt::db::performance::Observation {
    for _ in 0..100 {
        if let Some(o) = e.state.db.observation_of(tid).await.unwrap() {
            if o.result == result {
                return o;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no {result} observation for {tid}: {:?}", e.state.db.observation_of(tid).await.unwrap());
}

async fn to_review(e: &Env, tid: &str) {
    let turn_id = claim(e, tid).await;
    // The model call this run made (cost and model come from the owner's calls inside the claim)
    e.state
        .db
        .record_usage(&NewUsage {
            kwant_id: e.bot.id.clone(),
            context_id: e.general.clone(),
            purpose: kwnt::db::usage::PURPOSE_REPLY,
            model: "claude-opus-5-5".into(),
            fresh_input: 900,
            output: 100,
            reported: true,
            cost_usd: Some(0.25),
            ..Default::default()
        })
        .await
        .unwrap();
    let token = turn(e, &e.bot, &turn_id).await;
    let (v, err, text) = tool(e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    assert!(!err, "{text} {v}");
}

fn ms(x: &kwnt::db::performance::Observation) -> i64 {
    x.latency_ms
}

#[tokio::test]
async fn a_settled_task_is_observed_and_settling_again_replaces_its_row() {
    let e = setup().await;
    let tid = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": "Write it", "owner_kwant_id": e.bot.id}))).await);
    // Not settled yet: nothing observed
    assert!(!e.state.db.record_performance(&tid).await.unwrap());

    // Run 1: the person blocks and unblocks it, then kwant fails it
    claim(&e, &tid).await;
    person(&e, "POST", &format!("/api/tasks/{tid}/block"), Some(json!({"reason": "wait"}))).await;
    person(&e, "POST", &format!("/api/tasks/{tid}/unblock"), None).await;
    claim(&e, &tid).await;
    missions::transition(&e.state, &tid, "working", "failed", missions::Actor::Kwant, "kwant", TaskChange::default()).await.unwrap();
    let o = observed(&e, &tid, "failed").await;
    assert_eq!((o.kwant_id.as_str(), o.task_class.as_str(), o.source.as_str()), (e.bot.id.as_str(), "general", "live"));
    assert_eq!((o.runs, o.unblocks, o.retries, o.interventions), (2, 1, 0, 1), "{o:?}");
    assert!(o.started_at.is_some() && ms(&o) >= 0, "{o:?}");

    // The person retries it; it runs, goes to review and the person verifies it
    person(&e, "POST", &format!("/api/tasks/{tid}/retry"), None).await;
    to_review(&e, &tid).await;
    let t = person(&e, "POST", &format!("/api/tasks/{tid}/verify"), Some(json!({"note": "looks right"}))).await;
    assert_eq!(t["state"], "verified", "{t}");
    let o = observed(&e, &tid, "verified").await;
    assert_eq!((o.runs, o.unblocks, o.retries, o.interventions), (3, 1, 1, 2), "one row, by its last outcome: {o:?}");
    assert_eq!((o.model.as_str(), o.cost_usd), ("claude-opus-5-5", Some(0.25)), "{o:?}");
    assert_eq!(o.fresh_tokens, task(&e, &tid).await["task"]["spent_tokens"].as_i64().unwrap());
    let n: Vec<_> = e.state.db.observations(kwnt::db::spaces::DEFAULT_SPACE_ID, chrono::Utc::now() - chrono::Duration::days(1), None, None).await.unwrap();
    assert_eq!(n.iter().filter(|x| x.task_id == tid).count(), 1);
}

#[tokio::test]
async fn a_cancelled_task_counts_only_if_it_ran() {
    let e = setup().await;
    let never = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": "Never ran", "owner_kwant_id": e.bot.id}))).await);
    person(&e, "POST", &format!("/api/tasks/{never}/cancel"), None).await;
    let ran = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": "Ran", "owner_kwant_id": e.bot.id}))).await);
    claim(&e, &ran).await;
    person(&e, "POST", &format!("/api/tasks/{ran}/cancel"), None).await;
    let o = observed(&e, &ran, "unverified").await;
    assert_eq!(o.runs, 1);
    assert!(e.state.db.observe_task(&never).await.unwrap().is_none());
    assert!(e.state.db.observation_of(&never).await.unwrap().is_none());
}

#[tokio::test]
async fn the_backfill_runs_once_and_never_duplicates_or_overwrites() {
    let e = setup().await;
    let db = &e.state.db;
    let mut ids = vec![];
    for title in ["A", "B", "C"] {
        let tid = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": title, "owner_kwant_id": e.bot.id}))).await);
        claim(&e, &tid).await;
        person(&e, "POST", &format!("/api/tasks/{tid}/cancel"), None).await;
        observed(&e, &tid, "unverified").await;
        ids.push(tid);
    }
    // As if the first two settled before the table existed; the third stays a live row
    db.exec_raw_for_test(&format!("DELETE FROM performance_observations WHERE task_id IN ('{}', '{}')", ids[0], ids[1])).await.unwrap();
    // The fresh database already ran its (empty) backfill at startup
    assert_eq!(db.backfill_performance().await.unwrap(), 0, "marked done");
    db.exec_raw_for_test("DELETE FROM settings WHERE key = 'performance_backfill_v1'").await.unwrap();
    assert_eq!(db.backfill_performance().await.unwrap(), 2);
    assert_eq!(db.backfill_performance().await.unwrap(), 0, "once");
    db.exec_raw_for_test("DELETE FROM settings WHERE key = 'performance_backfill_v1'").await.unwrap();
    assert_eq!(db.backfill_performance().await.unwrap(), 0, "rows that exist are left alone");
    let sources: Vec<String> = sources_of(db, &ids).await;
    assert_eq!(sources, ["backfill", "backfill", "live"]);
}

async fn sources_of(db: &Database, ids: &[String]) -> Vec<String> {
    let mut out = vec![];
    for id in ids {
        out.push(db.observation_of(id).await.unwrap().unwrap().source);
    }
    out
}

/// A settled task `days_ago`; latency, tokens and cost are filled in by the caller.
fn obs(n: usize, class: &str, bot: &str, model: &str, result: &'static str, days_ago: i64) -> NewObservation {
    let finished_at = chrono::Utc::now() - chrono::Duration::days(days_ago) - chrono::Duration::minutes(n as i64);
    NewObservation {
        task_id: format!("{class}-{bot}-{model}-{result}-{n}-{days_ago}"),
        space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
        mission_id: None,
        task_class: class.into(),
        kwant_id: bot.into(),
        engine: "cli".into(),
        model: model.into(),
        started_at: None,
        finished_at,
        latency_ms: 0,
        runs: 1,
        fresh_tokens: 0,
        cost_usd: None,
        result,
        approvals: (n % 2) as i64,
        check_approvals: 0,
        unblocks: 0,
        retries: 0,
        autofix_attempts: 0,
    }
}

#[tokio::test]
async fn the_report_groups_with_a_wilson_interval_and_flags_small_groups() {
    let e = setup().await;
    let db = &e.state.db;
    let bot = e.bot.id.clone();
    // Bot on m1: 9 verified, 2 failed, 1 unverified (n = 12); latencies 1..12 s, tokens 100..1200
    for i in 0..12 {
        let result = if i < 9 { "verified" } else if i < 11 { "failed" } else { "unverified" };
        let cost = if i < 6 { Some(0.5) } else { None };
        db.store_observation(&NewObservation { latency_ms: (i as i64 + 1) * 1000, fresh_tokens: (i as i64 + 1) * 100, cost_usd: cost, ..obs(i, "general", &bot, "m1", result, 1) }, "live").await.unwrap();
    }
    // Same bot, other model: 2 rows (low confidence), no cost reported
    for i in 0..2 {
        db.store_observation(&obs(i, "general", &bot, "m2", "verified", 1), "live").await.unwrap();
    }
    // Another class, and one too old for the default window
    db.store_observation(&obs(0, "code", &bot, "m1", "failed", 1), "live").await.unwrap();
    db.store_observation(&obs(0, "general", &bot, "m1", "verified", 60), "live").await.unwrap();

    let r = person(&e, "GET", "/api/performance", None).await;
    assert_eq!((r["report_only"].as_bool(), r["min_n"].as_u64(), r["observations"].as_u64()), (Some(true), Some(5), Some(15)), "{r}");
    let groups = r["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 3, "{r}");
    // Sorted by class, then largest first
    assert_eq!(groups[0]["task_class"], "code");
    let g = &groups[1];
    assert_eq!((g["task_class"].as_str(), g["model"].as_str(), g["n"].as_u64()), (Some("general"), Some("m1"), Some(12)), "{g}");
    assert_eq!((g["verified"].as_u64(), g["failed"].as_u64(), g["unverified"].as_u64()), (Some(9), Some(2), Some(1)));
    assert_eq!(g["verified_rate"], 0.75);
    assert_eq!(g["verified_ci95"], json!([0.4677, 0.9111]), "{g}");
    assert_eq!((g["median_latency_ms"].as_f64(), g["median_fresh_tokens"].as_f64()), (Some(6500.0), Some(650.0)));
    assert_eq!((g["cost_usd"].as_f64(), g["cost_per_task_usd"].as_f64()), (Some(3.0), Some(0.5)), "averaged over rows with a cost");
    assert_eq!(g["interventions_per_task"], 0.5, "{g}");
    assert_eq!((g["low_confidence"].as_bool(), g["kwant_name"].as_str()), (Some(false), Some(e.bot.name.as_str())));
    assert_eq!((groups[2]["model"].as_str(), groups[2]["low_confidence"].as_bool(), groups[2]["cost_usd"].is_null()), (Some("m2"), Some(true), true));

    // by=bot folds the models together; filters narrow; since widens
    let r = person(&e, "GET", "/api/performance?by=bot&class=general", None).await;
    let g = &r["groups"][0];
    assert_eq!((r["groups"].as_array().unwrap().len(), g["n"].as_u64(), g["model"].is_null()), (1, Some(14), true), "{r}");
    let r = person(&e, "GET", "/api/performance?bot=nobody", None).await;
    assert_eq!(r["groups"], json!([]));
    let r = person(&e, "GET", "/api/performance?since=90d&class=general&by=bot", None).await;
    assert_eq!(r["groups"][0]["n"], 15, "{r}");
    for bad in ["since=0d", "since=yesterday", "by=engine"] {
        let (s, v) = call(&e.app, "GET", &format!("/api/performance?{bad}"), None, None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    // Bots never see the scoreboard
    let token = turn(&e, &e.bot, "t-perf").await;
    let (s, _) = call(&e.app, "GET", "/api/performance", Some(&token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

/// Insights measures (Phase C): weekly verified tasks with the active weeks, and cost per verified
/// outcome from the settled tasks' observations. Counts only.
#[tokio::test]
async fn insights_counts_verified_tasks_per_week_and_cost_per_verified_outcome() {
    let e = setup().await;
    let db = &e.state.db;
    let empty = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(empty["verified_weekly"].as_array().map(Vec::len), Some(8), "{empty}");
    assert_eq!((empty["active_weeks"].as_u64(), &empty["verified_per_active_week"]), (Some(0), &Value::Null), "{empty}");
    assert_eq!((&empty["cost_7d_usd"], &empty["cost_per_verified_7d_usd"], empty["settled_7d"].as_u64()), (&Value::Null, &Value::Null, Some(0)));

    // Two tasks verified this week through the real flow (each run reports $0.25)
    for title in ["One", "Two"] {
        let tid = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": title, "owner_kwant_id": e.bot.id}))).await);
        to_review(&e, &tid).await;
        person(&e, "POST", &format!("/api/tasks/{tid}/verify"), Some(json!({"note": "ok"}))).await;
        observed(&e, &tid, "verified").await;
    }
    // This week also settled a failed task that cost $0.15 and one whose calls reported no cost;
    // an observation from 10 days ago is outside the window
    db.store_observation(&NewObservation { cost_usd: Some(0.15), ..obs(1, "general", &e.bot.id, "m1", "failed", 1) }, "live").await.unwrap();
    db.store_observation(&obs(2, "general", &e.bot.id, "m1", "unverified", 2), "live").await.unwrap();
    // A verified task whose engine reported no cost: counted, but not in the per-outcome divisor
    db.store_observation(&obs(4, "general", &e.bot.id, "m1", "verified", 3), "live").await.unwrap();
    db.store_observation(&NewObservation { cost_usd: Some(9.0), ..obs(3, "general", &e.bot.id, "m1", "verified", 10) }, "live").await.unwrap();

    let v = person(&e, "GET", "/api/missions/insights", None).await;
    let weeks: Vec<(u64, bool)> = v["verified_weekly"].as_array().unwrap().iter().map(|w| (w["verified"].as_u64().unwrap(), w["active"].as_bool().unwrap())).collect();
    assert_eq!(weeks[..7], [(0, false); 7], "{v}");
    assert_eq!(weeks[7], (2, true), "{v}");
    assert_eq!(v["verified_weekly"][7]["verified"], v["verified_7d"], "the last week is the verified_7d window");
    assert_eq!((v["active_weeks"].as_u64(), v["verified_per_active_week"].as_f64()), (Some(1), Some(2.0)), "{v}");
    // $0.25 + $0.25 verified + $0.15 failed, over the two verified outcomes that reported a cost
    // (over all three it would read $0.216667)
    assert_eq!((v["cost_7d_usd"].as_f64(), v["cost_per_verified_7d_usd"].as_f64()), (Some(0.65), Some(0.325)), "{v}");
    assert_eq!(
        (v["verified_observed_7d"].as_u64(), v["verified_priced_7d"].as_u64(), v["settled_7d"].as_u64(), v["unpriced_7d"].as_u64()),
        (Some(3), Some(2), Some(5), Some(2)),
        "{v}"
    );
    // The old fields are unchanged
    assert_eq!(v["verified_7d"], 2);
}
