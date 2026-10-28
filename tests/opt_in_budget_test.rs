//! Token budgets are opt-in: `budget_tokens` 0 means no limit, which new tasks and missions get
//! unless the person sets one (on the item, or as a default in Settings). No limit is never
//! enforced; a positive one still blocks. Contract: docs/mission-control.md §15.

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

/// A mission the person creates (active at once), led by `e.bot`; returns `(mission, tasks)`.
async fn mission_with(e: &Env, budget: Option<i64>, tasks: Value) -> (Value, Vec<Value>) {
    let mut body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": e.bot.id, "channel_id": e.general, "tasks": tasks});
    if let Some(b) = budget {
        body["budget_tokens"] = json!(b);
    }
    let m = person(e, "POST", "/api/missions", Some(body)).await;
    let detail = person(e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    let mut tasks = detail["tasks"].as_array().unwrap().clone();
    tasks.sort_by_key(|t| t["title"].as_str().unwrap().to_string());
    (detail["mission"].clone(), tasks)
}

async fn claim(e: &Env, tid: &str) -> String {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let token = turn(e, &e.bot, &turn_id).await;
    let (v, err, text) = tool(e, &token, "task_claim", json!({"task_id": tid})).await;
    assert!(!err, "claim failed: {text} {v}");
    turn_id
}

async fn spend(e: &Env, turn_id: &str, tokens: u64) {
    let started_ms = chrono::Utc::now().timestamp_millis() - 1000;
    missions::spend(&e.state, &e.bot.id, &e.general, Some(turn_id), started_ms, tokens, 0).await;
}

#[tokio::test]
async fn new_tasks_and_missions_have_no_limit_unless_one_is_given() {
    let e = setup().await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((s["mission_default_budget_tokens"].as_i64(), s["mission_default_task_budget_tokens"].as_i64()), (Some(0), Some(0)), "{s}");

    let t = person(&e, "POST", "/api/tasks", Some(json!({"title": "Plain"}))).await;
    assert_eq!(t["budget_tokens"], 0, "{t}");
    let t = person(&e, "POST", "/api/tasks", Some(json!({"title": "Capped", "budget_tokens": 5_000}))).await;
    assert_eq!(t["budget_tokens"], 5_000, "{t}");

    let (m, tasks) = mission_with(&e, None, json!([{"title": "A"}, {"title": "B", "budget_tokens": 7_000}])).await;
    assert_eq!((m["budget_tokens"].as_i64(), m["over_budget"].as_bool()), (Some(0), Some(false)), "{m}");
    assert_eq!((tasks[0]["budget_tokens"].as_i64(), tasks[1]["budget_tokens"].as_i64()), (Some(0), Some(7_000)));
    // Inside a limited mission a plan task still gets no cap of its own unless given
    let (m, tasks) = mission_with(&e, Some(10_000), json!([{"title": "A"}])).await;
    assert_eq!((m["budget_tokens"].as_i64(), tasks[0]["budget_tokens"].as_i64()), (Some(10_000), Some(0)));
    let (status, v) = call(&e.app, "POST", "/api/missions", None, Some(json!({"title": "Too big", "goal": "g", "lead_kwant_id": e.bot.id,
        "channel_id": e.general, "budget_tokens": 10_000, "tasks": [{"title": "A", "budget_tokens": 20_000}]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");

    // Defaults the person sets apply to new items (a task default never exceeds its mission)
    person(&e, "POST", "/api/settings", Some(json!({"mission_default_budget_tokens": 300_000, "mission_default_task_budget_tokens": 50_000}))).await;
    assert_eq!(person(&e, "POST", "/api/tasks", Some(json!({"title": "D"}))).await["budget_tokens"], 50_000);
    let (m, tasks) = mission_with(&e, None, json!([{"title": "A"}])).await;
    assert_eq!((m["budget_tokens"].as_i64(), tasks[0]["budget_tokens"].as_i64()), (Some(300_000), Some(50_000)));
    let (_, tasks) = mission_with(&e, Some(20_000), json!([{"title": "A"}])).await;
    assert_eq!(tasks[0]["budget_tokens"], 20_000);
}

#[tokio::test]
async fn settings_take_zero_as_no_limit_and_refuse_bad_values() {
    let e = setup().await;
    for bad in [
        json!({"mission_default_budget_tokens": -1}),
        json!({"mission_default_budget_tokens": 999}),
        json!({"mission_default_budget_tokens": 5_000_001}),
        json!({"mission_default_task_budget_tokens": -1}),
        json!({"mission_default_task_budget_tokens": 5_000_001}),
    ] {
        let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} → {v}");
    }
    person(&e, "POST", "/api/settings", Some(json!({"mission_default_budget_tokens": 200_000, "mission_default_task_budget_tokens": 1}))).await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((s["mission_default_budget_tokens"].as_i64(), s["mission_default_task_budget_tokens"].as_i64()), (Some(200_000), Some(1)));
    person(&e, "POST", "/api/settings", Some(json!({"mission_default_budget_tokens": 0, "mission_default_task_budget_tokens": 0}))).await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((s["mission_default_budget_tokens"].as_i64(), s["mission_default_task_budget_tokens"].as_i64()), (Some(0), Some(0)));
}

#[tokio::test]
async fn no_limit_never_blocks_a_positive_limit_still_does_and_removing_it_unblocks() {
    let e = setup().await;
    let free = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": "Free", "owner_kwant_id": e.bot.id}))).await);
    let turn_id = claim(&e, &free).await;
    spend(&e, &turn_id, 5_000_000).await;
    let t = task(&e, &free).await["task"].clone();
    assert_eq!((t["state"].as_str(), t["spent_tokens"].as_i64()), (Some("working"), Some(5_000_000)), "{t}");
    person(&e, "POST", &format!("/api/tasks/{free}/cancel"), None).await;

    let capped = id(&person(&e, "POST", "/api/tasks", Some(json!({"title": "Capped", "owner_kwant_id": e.bot.id, "budget_tokens": 100}))).await);
    let turn_id = claim(&e, &capped).await;
    spend(&e, &turn_id, 150).await;
    let t = task(&e, &capped).await["task"].clone();
    assert_eq!((t["state"].as_str(), t["blocked_reason"].as_str()), (Some("blocked"), Some("budget")), "{t}");

    // The unblock the UI's "Remove limit and continue" makes: PATCH the limit off, then unblock
    let (status, v) = call(&e.app, "PATCH", &format!("/api/tasks/{capped}"), None, Some(json!({"budget_tokens": -1}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let t = person(&e, "PATCH", &format!("/api/tasks/{capped}"), Some(json!({"budget_tokens": 0}))).await;
    assert_eq!((t["budget_tokens"].as_i64(), t["state"].as_str()), (Some(0), Some("blocked")), "{t}");
    let t = person(&e, "POST", &format!("/api/tasks/{capped}/unblock"), None).await;
    assert_eq!(t["state"], "ready", "{t}");
    let turn_id = claim(&e, &capped).await;
    spend(&e, &turn_id, 1_000_000).await;
    assert_eq!(task(&e, &capped).await["task"]["state"], "working");
}

/// A claim kwant takes (as the mission runner does), so one bot can hold several.
async fn system_claim(e: &Env, t: &Value) -> Result<Result<kwnt::db::missions::Task, kwnt::db::missions::ClaimRefused>, kwnt::db::DbError> {
    e.state.db.claim_task(t["id"].as_str().unwrap(), &e.bot.id, None, None, 60_000, true, "kwant").await
}

#[tokio::test]
async fn reserve_math_counts_only_limited_tasks_and_a_mission_with_no_limit_never_refuses() {
    let e = setup().await;
    let db = &e.state.db;
    // A limited mission: unlimited tasks reserve nothing, limited ones reserve what they have left
    let (m, tasks) = mission_with(&e, Some(10_000), json!([
        {"title": "A"}, {"title": "B", "budget_tokens": 6_000}, {"title": "C", "budget_tokens": 6_000}, {"title": "D"}
    ])).await;
    assert!(system_claim(&e, &tasks[0]).await.unwrap().is_ok());
    assert!(system_claim(&e, &tasks[1]).await.unwrap().is_ok());
    let detail = person(&e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    assert_eq!(detail["mission"]["reserved_tokens"], 6_000, "{detail}");
    let third = system_claim(&e, &tasks[2]).await.unwrap();
    assert!(matches!(third, Err(kwnt::db::missions::ClaimRefused::Budget { remaining: 4_000, needed: 6_000 })), "{third:?}");
    // Once the mission's tokens are gone, even an unlimited task can't start
    db.exec_raw_for_test(&format!("UPDATE missions SET spent_tokens = 4000 WHERE id = '{}'", id(&m))).await.unwrap();
    let fourth = system_claim(&e, &tasks[3]).await.unwrap();
    assert!(matches!(fourth, Err(kwnt::db::missions::ClaimRefused::Budget { remaining: 0, needed: 1 })), "{fourth:?}");
    // Spent plus reserved fills it, but only spending at or past the limit is "over budget"
    let detail = person(&e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    assert_eq!(detail["mission"]["over_budget"], false);
    db.exec_raw_for_test(&format!("UPDATE missions SET spent_tokens = 10000 WHERE id = '{}'", id(&m))).await.unwrap();
    let detail = person(&e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    assert_eq!(detail["mission"]["over_budget"], true);

    // A mission with no limit: everything starts, nothing is "over budget"
    let (m, tasks) = mission_with(&e, None, json!([
        {"title": "A", "budget_tokens": 3_000_000}, {"title": "B", "budget_tokens": 3_000_000}, {"title": "C"}
    ])).await;
    for t in &tasks {
        assert!(system_claim(&e, t).await.unwrap().is_ok(), "{t}");
    }
    db.exec_raw_for_test(&format!("UPDATE missions SET spent_tokens = 9000000 WHERE id = '{}'", id(&m))).await.unwrap();
    let detail = person(&e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    assert_eq!((detail["mission"]["over_budget"].as_bool(), detail["mission"]["reserved_tokens"].as_i64()), (Some(false), Some(6_000_000)), "{detail}");

    // PATCH takes 0 off a mission's limit and refuses what isn't 0 or in range
    let (status, _) = call(&e.app, "PATCH", &format!("/api/missions/{}", id(&m)), None, Some(json!({"budget_tokens": 500}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let m2 = person(&e, "PATCH", &format!("/api/missions/{}", id(&m)), Some(json!({"budget_tokens": 50_000}))).await;
    assert_eq!(m2["budget_tokens"].as_i64(), Some(50_000), "{m2}");
    let m2 = person(&e, "PATCH", &format!("/api/missions/{}", id(&m)), Some(json!({"budget_tokens": 0}))).await;
    assert_eq!(m2["budget_tokens"].as_i64(), Some(0), "{m2}");
}
