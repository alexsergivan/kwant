//! 0.6.5: every Mission Control feature is on for a fresh install. Unset reads as on; a value the
//! person saved (off, suggest, shadow, a lower Flight School start) is kept, across saves of other
//! settings and across restarts. Limits stay off (the 2026-10-29 decision) and caps stay as they were.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{create_router, missions, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::tempdir;
use tower::ServiceExt;

async fn open(dir: &Path) -> (Router, AppState) {
    let db = Database::init(dir).await.unwrap();
    let config = Config { data_dir: dir.to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(dir.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state)
}

async fn person(app: &Router, state: &AppState, method: &str, body: Option<Value>) -> Value {
    let mut req = Request::builder()
        .method(method)
        .uri("/api/settings")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", state.api_token.reveal()));
    if body.is_none() {
        req = req.header("content-length", "0");
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::OK, "{method} /api/settings: {v}");
    v
}

const SIX: [&str; 6] = ["autofix", "auditor", "flight_school", "skill_foundry", "automation_discovery", "truth_ledger"];

fn six(s: &Value) -> Vec<Value> {
    SIX.iter().map(|k| s[*k].clone()).collect()
}

async fn modes(state: &AppState) -> Vec<String> {
    vec![
        missions::autofix::mode(state).await.to_string(),
        missions::auditor::enabled(state).await.to_string(),
        missions::flight_school::enabled(&state.db).await.to_string(),
        missions::foundry::enabled(state).await.to_string(),
        missions::discovery::enabled(state).await.to_string(),
        missions::truth_ledger::mode(state).await.to_string(),
    ]
}

#[tokio::test]
async fn a_fresh_install_has_all_six_features_on_and_its_limits_and_caps_unchanged() {
    let tmp = tempdir().unwrap();
    let (app, state) = open(tmp.path()).await;
    let s = person(&app, &state, "GET", None).await;
    assert_eq!(six(&s), vec![json!("on"), json!("on"), json!(true), json!("on"), json!("on"), json!("on")], "{s}");
    assert_eq!(s["flight_school_new_bot_tier"], "act");
    // What the server acts on agrees with what Settings shows
    assert_eq!(modes(&state).await, ["on", "true", "true", "true", "true", "on"]);
    // The other switches were already on
    for k in ["mission_board", "checkpoints", "decide_first", "work_ahead", "agent_tools", "fact_dispute_scan"] {
        assert_eq!(s[k], true, "{k}: {s}");
    }
    // Limits stay off
    for k in ["mission_default_budget_tokens", "mission_default_task_budget_tokens", "chain_token_budget"] {
        assert_eq!(s[k], 0, "{k}: {s}");
    }
    // Caps as before
    assert_eq!(s["recovery_budget_pct"], 25);
    assert_eq!(s["auditor_daily_cap"], 5);
    // Nothing was written: unset is what reads as on
    for k in SIX {
        assert_eq!(state.db.get_setting(k).await.unwrap(), None, "{k}");
    }
}

#[tokio::test]
async fn an_off_the_person_saved_stays_off_through_other_saves_and_a_restart() {
    let tmp = tempdir().unwrap();
    let (app, state) = open(tmp.path()).await;
    let off = json!({"autofix": "off", "auditor": "off", "flight_school": false, "skill_foundry": "off",
                     "automation_discovery": "off", "truth_ledger": "off"});
    person(&app, &state, "POST", Some(off)).await;
    let all_off = vec![json!("off"), json!("off"), json!(false), json!("off"), json!("off"), json!("off")];
    assert_eq!(six(&person(&app, &state, "GET", None).await), all_off);
    assert_eq!(modes(&state).await, ["off", "false", "false", "false", "false", "off"]);
    // Saving something else leaves them alone
    person(&app, &state, "POST", Some(json!({"timezone": "Europe/Berlin"}))).await;
    assert_eq!(six(&person(&app, &state, "GET", None).await), all_off);
    // And so does a restart: migrations never touch a saved choice
    drop((app, state));
    let (app, state) = open(tmp.path()).await;
    assert_eq!(six(&person(&app, &state, "GET", None).await), all_off);
    assert_eq!(modes(&state).await, ["off", "false", "false", "false", "false", "off"]);
}

#[tokio::test]
async fn the_in_between_choices_are_kept_too() {
    let tmp = tempdir().unwrap();
    let (app, state) = open(tmp.path()).await;
    person(&app, &state, "POST", Some(json!({"autofix": "suggest", "truth_ledger": "shadow", "flight_school_new_bot_tier": "sandbox"}))).await;
    drop((app, state));
    let (app, state) = open(tmp.path()).await;
    let s = person(&app, &state, "GET", None).await;
    assert_eq!((s["autofix"].clone(), s["truth_ledger"].clone(), s["flight_school_new_bot_tier"].clone()), (json!("suggest"), json!("shadow"), json!("sandbox")));
    assert_eq!(missions::autofix::mode(&state).await, "suggest");
    assert_eq!(missions::truth_ledger::mode(&state).await, "shadow");
    // Switching one back on is an explicit save as well
    person(&app, &state, "POST", Some(json!({"autofix": "on"}))).await;
    assert_eq!(missions::autofix::mode(&state).await, "on");
}
