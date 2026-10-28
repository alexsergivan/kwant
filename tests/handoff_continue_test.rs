//! A hand-off's board card and the real work agree (2026-10-28: "When I unblock tasks they are
//! going to ready, but nothing happening with them after"). The board's runner never starts
//! hand-offs, so a hand-off card the person retries or unblocks carries on in the hand-off's own
//! thread, or goes to review when the hand-off already finished; a budget block doesn't stop the
//! live hand-off, and its end is noted on the card. A fake claude CLI on PATH fails a "FAILTASK"
//! prompt unless it is a carry-on turn and holds a "LONGTASK:<file>" turn until that file exists.
//! Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{missions::Task, Database},
    router::LayaRouter,
    server::{create_router, missions, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const CARRY_ON: &str = "carry on from where you stopped";

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
printf '%s' "$last" > "$KWNT_TEST_PROMPTS/$$-$(date +%s%N)"
case "$last" in
  *"carry on from where you stopped"*) ;;
  *FAILTASK*) echo "boom" >&2; exit 1 ;;
esac
release=$(printf '%s\n' "$last" | sed -n 's/.*LONGTASK:\([^ ]*\).*/\1/p' | head -n 1)
if [ -n "$release" ]; then
  i=0
  while [ ! -f "$release" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"done"}'
"#;

fn prompts_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        prompts
    })
}

/// Prompts the fake CLI got that contain every one of `markers`.
fn prompts_with(markers: &[&str]) -> Vec<String> {
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .filter(|p| markers.iter().all(|m| p.contains(m)))
        .collect()
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn person(env: &Env, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(&env.app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

struct Env {
    app: Router,
    state: AppState,
    lead: String,
    specialist: String,
    tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    prompts_dir();
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
    let app = create_router(state.clone());
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let mut ids = Vec::new();
    for k in kwants.as_array().unwrap().iter().take(2) {
        let id = k["id"].as_str().unwrap().to_string();
        let mut body = k.clone();
        body.as_object_mut()
            .unwrap()
            .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
        body["preferred_model"] = json!("claude");
        let (s, v) = call(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        ids.push(id);
    }
    let (s, v) = call(&app, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The board follows hand-offs on the event bus, as the scheduler starts it in kwnt
    missions::listener::spawn(state.clone(), tokio_util::sync::CancellationToken::new());
    Env { app, state, lead: ids[0].clone(), specialist: ids[1].clone(), tmp }
}

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A hand-off from the lead to the specialist that reports back to the lead.
async fn delegate(env: &Env, task: &str, title: &str) -> Value {
    let body = json!({"lead_kwant_id": env.lead, "specialist_kwant_id": env.specialist, "task": task, "title": title, "kwant_id": env.lead});
    let (s, d) = call(&env.app, "POST", "/api/delegations", Some(body)).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    d
}

/// A hand-off row that runs nothing until something starts it.
async fn quiet_delegation(env: &Env, title: &str) -> kwnt::db::threads::Delegation {
    let new = kwnt::db::threads::NewDelegation {
        lead_kwant_id: env.lead.clone(),
        specialist_kwant_id: env.specialist.clone(),
        origin_channel_id: None,
        origin_kwant_id: Some(env.lead.clone()),
        origin_thread_id: None,
        team_id: None,
        task: format!("{title} quietly"),
        title: title.into(),
        continue_lead: true,
        depth: 1,
        follow_up_at: None,
    };
    env.state.db.create_delegation(&new).await.unwrap().1
}

async fn status_of(env: &Env, id: &str) -> String {
    env.state.db.get_delegation(id).await.unwrap().map(|d| d.status).unwrap_or_default()
}

async fn card(env: &Env, delegation_id: &str) -> Option<Task> {
    env.state.db.task_by_source("delegation", delegation_id).await.unwrap()
}

async fn card_state(env: &Env, delegation_id: &str) -> String {
    card(env, delegation_id).await.map(|t| t.state).unwrap_or_default()
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// Moves of the task, as "from>to".
async fn moves(env: &Env, task_id: &str) -> Vec<String> {
    let events = env.state.db.task_events(task_id, 200).await.unwrap();
    events.iter().filter_map(|e| Some(format!("{}>{}", e.from_state.as_deref()?, e.to_state.as_deref()?))).collect()
}

/// Oleks (2026-10-28): a hand-off card unblocked or retried went to Ready and nothing happened.
/// Retrying a failed hand-off now carries it on in its own thread: the card is back to working,
/// the teammate gets a carry-on turn, and its new result reaches the lead.
#[tokio::test]
async fn retrying_a_failed_hand_off_carries_it_on_and_the_card_works_again() {
    let env = setup().await;
    let d = delegate(&env, "Audit the pricing page FAILTASK RETRYMARK", "Pricing audit").await;
    let did = id(&d);
    wait_for("the hand-off to fail", || async { status_of(&env, &did).await == "failed" }).await;
    wait_for("the card to fail", || async { card_state(&env, &did).await == "failed" }).await;
    let task = card(&env, &did).await.unwrap();

    person(&env, "POST", &format!("/api/tasks/{}/retry", task.id), None).await;
    wait_for("the hand-off to finish", || async { status_of(&env, &did).await == "completed" }).await;
    let carried = prompts_with(&[CARRY_ON, "RETRYMARK"]);
    assert_eq!(carried.len(), 1, "one carry-on turn");
    assert!(carried[0].contains("stopped before it was done"), "{}", carried[0]);
    let reopened = env.state.db.get_delegation(&did).await.unwrap().unwrap();
    assert_eq!((reopened.thread_id.as_str(), reopened.error.as_deref()), (d["thread_id"].as_str().unwrap(), None), "same thread");

    wait_for("the card to leave working", || async { matches!(card_state(&env, &did).await.as_str(), "review" | "verified") }).await;
    let m = moves(&env, &task.id).await;
    let retried = m.iter().position(|x| x == "failed>ready").unwrap();
    assert!(m[retried..].iter().any(|x| x == "ready>working"), "the card works again: {m:?}");
    wait_for("the lead's turn", || async { !prompts_with(&["carried on with the task you delegated", "Pricing audit"]).is_empty() }).await;
}

/// A budget block lands at turn end and never stops the live hand-off: it finishes, and the card
/// notes it. Unblocked while still over its limit, the card is blocked again (not stuck in Ready);
/// with the limit removed, unblocking sends it to review without running the work again.
#[tokio::test]
async fn a_budget_blocked_hand_off_is_noted_and_goes_on_when_unblocked() {
    let env = setup().await;
    let release = env.tmp.path().join("release-budget");
    let d = delegate(&env, &format!("Long job BUDGETMARK LONGTASK:{}", release.display()), "Long job").await;
    let did = id(&d);
    wait_for("the card to work", || async { card_state(&env, &did).await == "working" }).await;
    let task = card(&env, &did).await.unwrap();
    person(&env, "PATCH", &format!("/api/tasks/{}", task.id), Some(json!({"budget_tokens": 100}))).await;
    let started_ms = chrono::Utc::now().timestamp_millis() - 1000;
    missions::spend(&env.state, &env.specialist, d["thread_id"].as_str().unwrap(), None, started_ms, 150, 0).await;
    let t = env.state.db.get_task(&task.id).await.unwrap().unwrap();
    assert_eq!(t.state, "blocked");
    assert!(matches!(status_of(&env, &did).await.as_str(), "queued" | "running"), "the hand-off goes on");

    std::fs::write(&release, "go").unwrap();
    wait_for("the hand-off to finish", || async { status_of(&env, &did).await == "completed" }).await;
    let noted = || async {
        env.state.db.task_events(&task.id, 50).await.unwrap().into_iter().filter(|e| e.kind == "handoff_ended").count()
    };
    wait_for("the card's note", || async { noted().await == 1 }).await;
    missions::listener::reconcile(&env.state).await;
    assert_eq!(noted().await, 1, "noted once");
    assert_eq!(card_state(&env, &did).await, "blocked");

    // Still over its limit: blocked again for the person, not left in Ready
    person(&env, "POST", &format!("/api/tasks/{}/unblock", task.id), None).await;
    wait_for("the card to block again", || async { moves(&env, &task.id).await.iter().filter(|m| m.ends_with(">blocked")).count() == 2 }).await;
    assert_eq!(card_state(&env, &did).await, "blocked");

    // "Remove limit and continue": its result is in, so it goes to review
    person(&env, "PATCH", &format!("/api/tasks/{}", task.id), Some(json!({"budget_tokens": 0}))).await;
    person(&env, "POST", &format!("/api/tasks/{}/unblock", task.id), None).await;
    wait_for("the card to reach review", || async { matches!(card_state(&env, &did).await.as_str(), "review" | "verified") }).await;
    assert!(prompts_with(&[CARRY_ON, "BUDGETMARK"]).is_empty(), "a finished hand-off isn't run again");
}

/// Two restarts cut a hand-off short: its card fails (offering Retry) instead of waiting in Ready
/// for nobody, and a retry carries it on.
#[tokio::test]
async fn an_interrupted_hand_off_fails_its_card_and_a_retry_carries_it_on() {
    let env = setup().await;
    let d = quiet_delegation(&env, "Cut short INTERRUPTMARK").await;
    missions::listener::on_delegation(&env.state, &d.id).await;
    assert_eq!(card_state(&env, &d.id).await, "working");
    assert!(env.state.db.finish_delegation(&d.id, "interrupted", Some("kwnt restarted"), None).await.unwrap());
    missions::listener::on_delegation(&env.state, &d.id).await;
    let task = card(&env, &d.id).await.unwrap();
    assert_eq!(task.state, "failed");

    person(&env, "POST", &format!("/api/tasks/{}/retry", task.id), None).await;
    wait_for("the hand-off to finish", || async { status_of(&env, &d.id).await == "completed" }).await;
    assert_eq!(prompts_with(&[CARRY_ON, "INTERRUPTMARK"]).len(), 1);
    wait_for("the card to reach review", || async { matches!(card_state(&env, &d.id).await.as_str(), "review" | "verified") }).await;
}

/// A card left in Ready before this fix, with its hand-off over, doesn't stay stuck: the next
/// reconcile carries it on, or blocks it with a reason when it can't (here, the teammate is gone).
#[tokio::test]
async fn a_stuck_ready_hand_off_card_is_carried_on_or_blocked_with_a_reason() {
    let env = setup().await;
    let stuck = |d: &str| format!("UPDATE tasks SET state = 'ready' WHERE source_kind = 'delegation' AND source_id = '{d}'");

    let d = quiet_delegation(&env, "Stuck card STUCKMARK").await;
    missions::listener::on_delegation(&env.state, &d.id).await;
    assert!(env.state.db.finish_delegation(&d.id, "failed", Some("the engine stopped"), None).await.unwrap());
    env.state.db.exec_raw_for_test(&stuck(&d.id)).await.unwrap();
    env.state.db.exec_raw_for_test("UPDATE agent_claims SET released_at = 1").await.unwrap();
    missions::listener::reconcile(&env.state).await;
    wait_for("the hand-off to finish", || async { status_of(&env, &d.id).await == "completed" }).await;
    let carried = prompts_with(&[CARRY_ON, "STUCKMARK"]);
    assert_eq!(carried.len(), 1);
    assert!(carried[0].contains("(the engine stopped)"), "{}", carried[0]);

    // A completed hand-off can't be reopened; one whose teammate is gone can't carry on
    assert!(!env.state.db.reopen_delegation(&d.id).await.unwrap(), "only an ended, unfinished hand-off reopens");
    let gone = quiet_delegation(&env, "Gone teammate").await;
    missions::listener::on_delegation(&env.state, &gone.id).await;
    assert!(env.state.db.finish_delegation(&gone.id, "cancelled", None, None).await.unwrap());
    env.state.db.exec_raw_for_test(&stuck(&gone.id)).await.unwrap();
    env.state.db.exec_raw_for_test("UPDATE agent_claims SET released_at = 1 WHERE released_at IS NULL").await.unwrap();
    env.state.db.exec_raw_for_test(&format!("UPDATE delegations SET specialist_kwant_id = 'nobody' WHERE id = '{}'", gone.id)).await.unwrap();
    missions::listener::reconcile(&env.state).await;
    let t = card(&env, &gone.id).await.unwrap();
    assert_eq!((t.state.as_str(), t.blocked_reason.as_deref()), ("blocked", Some(missions::listener::CANT_CARRY_ON)));
}
