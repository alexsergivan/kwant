//! P1 "watch a mission": the person picks a watcher; when the mission stops, the watcher's turn
//! sees and runs only its allowlist, reports through `watch_report`, and kwant applies the
//! send-backs. Contract: release-090/p1-contract.md.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        mission_watch::NewWatchCheck,
        missions::{Mission, NewMission, NewTask, Task},
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::watch,
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), laya_url: "http://127.0.0.1:9".into(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let space = SpaceId::default_space();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app: create_router(state.clone()), state, general, _tmp: tmp };
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

async fn bot(e: &Env, name: &str, title: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: title.into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    e.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

async fn mission(e: &Env, lead: &Kwant) -> Mission {
    let m = NewMission {
        title: "Ship the export".into(),
        goal: "People can export their notes".into(),
        status: "active".into(),
        lead_kwant_id: lead.id.clone(),
        channel_id: Some(e.general.clone()),
        workspace_path: e._tmp.path().join("m").to_string_lossy().into(),
        created_by: "person".into(),
        ..Default::default()
    };
    e.state.db.create_mission(&SpaceId::default_space(), &m).await.unwrap()
}

async fn task(e: &Env, m: &Mission, owner: &Kwant, title: &str, state: &str) -> Task {
    let t = NewTask {
        mission_id: Some(m.id.clone()),
        title: title.into(),
        acceptance: "The export file opens".into(),
        state: state.into(),
        owner_kwant_id: Some(owner.id.clone()),
        max_attempts: 3,
        source_kind: "mission".into(),
        created_by: "person".into(),
        ..Default::default()
    };
    e.state.db.create_task(&SpaceId::default_space(), &t).await.unwrap()
}

/// A running turn of `bot` with purpose `purpose` (None: an ordinary turn), and its tool token.
async fn turn(e: &Env, bot: &Kwant, purpose: Option<&str>) -> String {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    e.state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.clone(),
        kwant_id: bot.id.clone(),
        context_id: bot.id.clone(),
        thread_id: None,
        state: "running".into(),
        since: chrono::Utc::now(),
        purpose: purpose.map(String::from),
    });
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), origin: Some(origin), turn_id: Some(turn_id.clone()), ..Default::default() };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id), None, Some(scope)).await.unwrap().1
}

async fn rpc(e: &Env, token: &str, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (bool, String) {
    let v = rpc(e, token, "tools/call", json!({"name": name, "arguments": args})).await;
    (v["result"]["isError"].as_bool().unwrap_or(false), v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
}

async fn tool_names(e: &Env, token: &str) -> Vec<String> {
    let v = rpc(e, token, "tools/list", json!({})).await;
    v["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str().map(String::from)).collect()
}

async fn checking(e: &Env, m: &Mission, watcher: &Kwant) -> kwnt::db::mission_watch::WatchCheck {
    let new = NewWatchCheck { mission_id: &m.id, kwant_id: &watcher.id, trigger: "manual", status: "checking", fingerprint: "fp", reason: None, stopped_at: Some(chrono::Utc::now()) };
    e.state.db.insert_watch_check(&new).await.unwrap()
}

#[tokio::test]
async fn the_person_picks_the_watcher_and_its_report_sends_back_what_doesnt_hold() {
    let e = setup().await;
    let (lead, owner, rigel, other) = (bot(&e, "Ada", "Lead").await, bot(&e, "Pixel", "Frontend").await, bot(&e, "Rigel", "QA and Release").await, bot(&e, "Iris", "Frontend").await);
    let m = mission(&e, &lead).await;
    let base = format!("/api/missions/{}/watch", m.id);

    // No tasks yet: nothing to watch
    let v = person(&e, "GET", &base, None).await;
    assert_eq!(v["watch"]["state"], "no_tasks", "{v}");
    let (status, v) = call(&e.app, "PUT", &base, None, Some(json!({"kwant_id": rigel.id}))).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("no_tasks")), "{v}");

    let api = task(&e, &m, &owner, "Export API", "review").await;
    let docs = task(&e, &m, &owner, "Export docs", "review").await;
    let deploy = task(&e, &m, &owner, "Deploy", "review").await;

    // Off; the QA bot with no task is the default candidate
    let v = person(&e, "GET", &base, None).await;
    assert_eq!(v["watch"]["state"], "off", "{v}");
    let cands = v["candidates"].as_array().unwrap();
    let default: Vec<&str> = cands.iter().filter(|c| c["default"] == true).map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(default, ["Rigel"], "{v}");
    let pixel = cands.iter().find(|c| c["name"] == "Pixel").unwrap();
    assert_eq!(pixel["relation"], "has_task");
    assert_eq!(cands.iter().find(|c| c["name"] == "Ada").unwrap()["relation"], "lead");

    // Only the person sets the watch, and only to a bot of the space
    let bot_token = turn(&e, &other, None).await;
    let (status, _) = call(&e.app, "PUT", &base, Some(&bot_token), Some(json!({"kwant_id": other.id}))).await;
    assert!(!status.is_success(), "a bot can't pick the watcher: {status}");
    for bad in [json!({}), json!({"kwant_id": "k-nobody"}), json!({"kwant_id": 7})] {
        let (status, v) = call(&e.app, "PUT", &base, None, Some(bad.clone())).await;
        assert_eq!((status, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_kwant")), "{bad}: {v}");
    }
    let v = person(&e, "PUT", &base, Some(json!({"kwant_id": rigel.id}))).await;
    assert_eq!((v["watch"]["state"].as_str(), v["watch"]["kwant_name"].as_str()), (Some("watching"), Some("Rigel")), "{v}");
    // The tasks changed under a minute ago: not stopped, so no manual check yet
    assert_eq!(v["watch"]["stopped"], false);
    let (status, v) = call(&e.app, "POST", &format!("{base}/check"), None, None).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_stopped")), "{v}");
    // The mission itself carries the chip
    let v = person(&e, "GET", &format!("/api/missions/{}", m.id), None).await;
    assert_eq!(v["mission"]["watch"]["state"], "watching", "{v}");

    // A check runs: the watch turn sees only its allowlist and the report tool
    let check = checking(&e, &m, &rigel).await;
    assert_eq!(person(&e, "GET", &base, None).await["watch"]["state"], "checking");
    watch::open_report(&rigel.id, &check.id, vec![api.id.clone(), docs.id.clone(), deploy.id.clone()]);
    let token = turn(&e, &rigel, Some(watch::PURPOSE)).await;
    let names = tool_names(&e, &token).await;
    assert!(names.contains(&watch::TOOL.to_string()), "{names:?}");
    assert!(names.iter().all(|n| watch::ALLOWED_TOOLS.contains(&n.as_str())), "{names:?}");
    for n in ["task_review", "task_update", "delegate", "ask_user", "bot_create"] {
        assert!(!names.contains(&n.to_string()), "{n} is hidden");
    }
    // ... and runs nothing else
    let (err, text) = tool(&e, &token, "task_review", json!({"task_id": api.id, "verdict": "pass"})).await;
    assert!(err && text == watch::REFUSED_TEXT, "{text}");
    assert_eq!(e.state.db.get_task(&api.id).await.unwrap().unwrap().state, "review");
    // Other turns don't see or run the report tool
    assert!(!tool_names(&e, &bot_token).await.contains(&watch::TOOL.to_string()));
    let (err, text) = tool(&e, &bot_token, watch::TOOL, json!({"check_id": check.id, "claims": []})).await;
    assert!(err && text.contains("only runs in a watch turn"), "{text}");
    // The report must be for the running check and name only its claims
    let (err, text) = tool(&e, &token, watch::TOOL, json!({"check_id": "wc-other", "claims": []})).await;
    assert!(err && text.contains("check_id"), "{text}");
    let (err, text) = tool(&e, &token, watch::TOOL, json!({"check_id": check.id, "claims": [{"task_id": "t-elsewhere", "checked": "x", "verdict": "holds"}]})).await;
    assert!(err && text.contains("Fix it"), "{text}");
    let report = json!({"check_id": check.id,
        "claims": [{"task_id": api.id, "checked": "GET /export answers 500.", "verdict": "doesnt_hold"},
                   {"task_id": docs.id, "checked": "The page lists every format.", "verdict": "holds"},
                   {"task_id": deploy.id, "checked": "Ready, waits for the OK.", "verdict": "holds"}],
        "actions": [{"kind": "restart", "task_id": api.id, "reason": "GET /export answers 500."},
                    {"kind": "restart", "task_id": docs.id, "reason": "Just because."},
                    {"kind": "needs_you", "task_id": deploy.id, "what": "your OK to deploy"},
                    {"kind": "verify", "task_id": deploy.id, "what": "mark it done"}]});
    let (err, text) = tool(&e, &token, watch::TOOL, report.clone()).await;
    assert!(!err, "{text}");
    let (err, text) = tool(&e, &token, watch::TOOL, report).await;
    assert!(err && text.contains("already"), "{text}");
    // Nothing moved during the turn
    assert_eq!(e.state.db.get_task(&api.id).await.unwrap().unwrap().state, "review");

    // The turn ends: kwant applies the report
    let taken = watch::take_report(&rigel.id);
    assert_eq!(taken.refused_tools, ["task_review"]);
    assert!(watch::settle(&e.state, &check, taken, None, 1200).await);
    assert!(!watch::settle(&e.state, &check, watch::Taken::default(), None, 0).await, "a check ends once");

    let api_now = e.state.db.get_task(&api.id).await.unwrap().unwrap();
    assert_eq!((api_now.state.as_str(), api_now.attempts), ("ready", 1));
    let last = e.state.db.task_events(&api.id, 1).await.unwrap().pop().unwrap();
    assert_eq!((last.kind.as_str(), last.actor.as_str()), ("watch", rigel.id.as_str()));
    assert!(last.detail_value()["reason"].as_str().unwrap().contains("GET /export answers 500."));
    assert_eq!(e.state.db.get_task(&docs.id).await.unwrap().unwrap().state, "review", "a claim that holds isn't restarted");
    assert_eq!(e.state.db.get_task(&deploy.id).await.unwrap().unwrap().state, "review", "needs you moves nothing");
    assert_eq!(e.state.db.get_mission(&m.id).await.unwrap().unwrap().spent_tokens, 1200);

    let v = person(&e, "GET", &format!("{base}/checks"), None).await;
    let c = &v["checks"][0];
    assert_eq!((c["status"].as_str(), c["counts"]["dont_hold"].as_i64(), c["counts"]["holds"].as_i64(), c["counts"]["needs_you"].as_i64()), (Some("done"), Some(1), Some(2), Some(1)), "{c}");
    assert_eq!(c["claims"][0]["task_id"], json!(api.id), "doesn't hold comes first");
    assert_eq!(c["claims"][1]["task_id"], json!(deploy.id), "then needs you");
    assert_eq!(c["claims"][0]["claimed"], "In review");
    let actions = c["actions"].as_array().unwrap();
    let of = |kind: &str, id: Option<&str>| actions.iter().find(|a| a["kind"] == kind && id.is_none_or(|id| a["task_id"] == id)).cloned().unwrap_or(Value::Null);
    assert_eq!(of("restart", Some(&api.id))["applied"], true, "{actions:?}");
    assert_eq!(of("restart", Some(&docs.id))["why"], "the claim holds");
    assert_eq!(of("needs_you", Some(&deploy.id))["state"], "review");
    let refused: Vec<&Value> = actions.iter().filter(|a| a["kind"] == "refused").collect();
    assert_eq!(refused.len(), 2, "the unknown action and the refused tool: {actions:?}");
    assert!(refused.iter().any(|a| a["tool"] == "task_review"));
    assert_eq!(c["inbox"]["open"], true);
    assert_eq!(c["inbox"]["line"].as_str().unwrap().split(" since ").next().unwrap(), "Rigel: Deploy has waited for your OK to deploy");
    assert!(c["message_id"].is_string(), "the mission chat got the result: {c}");
    let one = person(&e, "GET", &format!("/api/watch-checks/{}", check.id), None).await;
    assert_eq!(one["id"], json!(check.id));
    let (status, _) = call(&e.app, "GET", "/api/watch-checks/wc-nope", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // What waits on the person is in Waiting on you
    let away = person(&e, "GET", "/api/away", None).await;
    let item = away["waiting"].as_array().unwrap().iter().find(|i| i["kind"] == "watch").cloned().unwrap_or(Value::Null);
    assert_eq!((item["mission_id"].clone(), item["task_id"].clone()), (json!(m.id), json!(deploy.id)), "{away}");
    assert!(away["unseen"]["waiting"].as_i64().unwrap() >= 1);
    assert_eq!(person(&e, "GET", &base, None).await["watch"]["state"], "dont_hold");

    // Off: a check that ends after the watch was removed applies nothing
    let check2 = checking(&e, &m, &rigel).await;
    watch::open_report(&rigel.id, &check2.id, vec![docs.id.clone()]);
    let token = turn(&e, &rigel, Some(watch::PURPOSE)).await;
    let (err, text) = tool(&e, &token, watch::TOOL, json!({"check_id": check2.id, "claims": [{"task_id": docs.id, "checked": "x", "verdict": "doesnt_hold"}],
        "actions": [{"kind": "restart", "task_id": docs.id, "reason": "No."}]})).await;
    assert!(!err, "{text}");
    let v = person(&e, "DELETE", &base, None).await;
    assert_eq!(v["watch"]["state"], "off", "{v}");
    assert!(watch::settle(&e.state, &check2, watch::take_report(&rigel.id), None, 0).await);
    let c2 = e.state.db.get_watch_check(&check2.id).await.unwrap().unwrap();
    assert_eq!((c2.status.as_str(), c2.reason_code.as_deref()), ("failed", Some("watch_off")));
    assert_eq!(e.state.db.get_task(&docs.id).await.unwrap().unwrap().state, "review");
}

#[tokio::test]
async fn a_turn_without_a_report_fails_the_check_and_moves_nothing() {
    let e = setup().await;
    let (lead, owner, rigel) = (bot(&e, "Ada", "Lead").await, bot(&e, "Pixel", "Frontend").await, bot(&e, "Rigel", "QA").await);
    let m = mission(&e, &lead).await;
    let t = task(&e, &m, &owner, "Export API", "failed").await;
    person(&e, "PUT", &format!("/api/missions/{}/watch", m.id), Some(json!({"kwant_id": rigel.id}))).await;
    let check = checking(&e, &m, &rigel).await;
    watch::open_report(&rigel.id, &check.id, vec![t.id.clone()]);
    assert!(watch::settle(&e.state, &check, watch::take_report(&rigel.id), Some("the watch turn took longer than 20 minutes".into()), 50).await);
    let c = e.state.db.get_watch_check(&check.id).await.unwrap().unwrap();
    assert_eq!((c.status.as_str(), c.reason_code.as_deref()), ("failed", Some("timed_out")));
    assert_eq!(e.state.db.get_task(&t.id).await.unwrap().unwrap().state, "failed");
    let v = person(&e, "GET", &format!("/api/missions/{}/watch", m.id), None).await;
    assert_eq!((v["watch"]["state"].as_str(), v["watch"]["reason"]["code"].as_str()), (Some("failed"), Some("timed_out")), "{v}");
    assert_eq!(v["report"]["error"], "took longer than 20 minutes");

    // A restart cuts a running check off: it fails with "restarted"
    let check2 = checking(&e, &m, &rigel).await;
    watch::resume_after_restart(&e.state).await;
    let c2 = e.state.db.get_watch_check(&check2.id).await.unwrap().unwrap();
    assert_eq!((c2.status.as_str(), c2.reason_code.as_deref()), ("failed", Some("restarted")));
}

#[tokio::test]
async fn the_tick_finds_the_stop_and_records_a_limit_once_without_a_turn() {
    let e = setup().await;
    let (lead, owner, rigel) = (bot(&e, "Ada", "Lead").await, bot(&e, "Pixel", "Frontend").await, bot(&e, "Rigel", "QA").await);
    let m = mission(&e, &lead).await;
    let base = format!("/api/missions/{}/watch", m.id);
    let t = task(&e, &m, &owner, "Export API", "review").await;
    person(&e, "PUT", &base, Some(json!({"kwant_id": rigel.id}))).await;

    // Changed under a minute ago: not stopped, the tick starts nothing
    watch::tick(&e.state).await;
    assert!(e.state.db.latest_watch_check(&m.id).await.unwrap().is_none());

    // Quiet for five minutes, and the mission is at its token limit
    let old = "strftime('%Y-%m-%dT%H:%M:%S+00:00', 'now', '-5 minutes')";
    // The event log is append-only; this test's own database lifts that to age it
    e.state.db.exec_raw_for_test("DROP TRIGGER task_events_append_only_update").await.unwrap();
    e.state.db.exec_raw_for_test(&format!("UPDATE task_events SET created_at = {old} WHERE task_id = '{}'", t.id)).await.unwrap();
    e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET updated_at = {old} WHERE id = '{}'", t.id)).await.unwrap();
    e.state.db.exec_raw_for_test(&format!("UPDATE missions SET budget_tokens = 100, spent_tokens = 100 WHERE id = '{}'", m.id)).await.unwrap();
    let v = person(&e, "GET", &base, None).await;
    assert_eq!((v["watch"]["stopped"].as_bool(), v["watch"]["state"].as_str()), (Some(true), Some("over_limit")), "{v}");
    assert!(v["watch"]["stopped_at"].is_string() && v["watch"]["can_check_now"] == false, "{v}");
    let (status, v) = call(&e.app, "POST", &format!("{base}/check"), None, None).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("over_limit")), "{v}");

    // The tick records the stop it can't check once, and runs no turn
    watch::tick(&e.state).await;
    watch::tick(&e.state).await;
    let checks = e.state.db.watch_checks(&m.id, 10).await.unwrap();
    assert_eq!(checks.len(), 1, "recorded once per stop: {checks:?}");
    assert_eq!((checks[0].status.as_str(), checks[0].reason_code.as_deref(), checks[0].trigger.as_str()), ("over_limit", Some("over_limit"), "stop"));
    assert!(checks[0].stopped_at.is_some());
    assert_eq!(e.state.db.get_task(&t.id).await.unwrap().unwrap().state, "review", "nothing moved");
    assert_eq!(e.state.db.get_mission(&m.id).await.unwrap().unwrap().spent_tokens, 100, "no turn, no tokens");
}
