//! Waiting checks: a `command` check that exits 75, or an `http` check that answers its
//! `waiting_status` / `waiting_contains`, means "not ready yet". The check waits (no failure,
//! no AutoFix, no bot turn), the missions tick runs it again when due, and past its `wait_hours`
//! it stops with failure class `waited_too_long` and one Needs-you item the person answers with
//! "Wait 24 h more" or Verify. Every run of a round shares an idempotency key; a new round after a
//! fail gets a new one.

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use chrono::{DateTime, Duration, Utc};
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
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: String,
    tmp: TempDir,
}

async fn setup(autofix: &str) -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app: create_router(state.clone()), bot: kwants[0].clone(), general, state, tmp };
    let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"mission_board": true, "autofix": autofix, "verifier": "off"}))).await;
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

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (bool, String) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
    (!v["result"]["isError"].as_bool().unwrap_or(false), text)
}

async fn ok(e: &Env, token: &str, name: &str, args: Value) {
    let (fine, text) = tool(e, token, name, args.clone()).await;
    assert!(fine, "{name} {args} failed: {text}");
}

async fn turn(e: &Env, turn_id: &str) -> String {
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope { kwant_id: e.bot.id.clone(), kwant_name: e.bot.name.clone(), origin: Some(origin), turn_id: Some(turn_id.into()), ..Default::default() };
    mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn task(e: &Env, id: &str) -> Value {
    person(e, "GET", &format!("/api/tasks/{id}"), None).await
}

fn check<'a>(t: &'a Value, id: &str) -> &'a Value {
    t["checks"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap_or_else(|| panic!("no check {id} in {t}"))
}

fn at(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap_or_else(|| panic!("not a time: {v}"))).unwrap().with_timezone(&Utc)
}

fn near(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_seconds().abs() < 60
}

/// A command that answers with the next exit code from `codes-<name>` (one per line, counted per
/// check) and logs the env it got: round, run, key, task id, check id.
fn scripted(name: &str) -> String {
    format!(
        "n=$(( $(cat .n-$KWANT_CHECK_ID 2>/dev/null || echo 0) + 1 )); echo $n > .n-$KWANT_CHECK_ID; \
         echo \"$KWANT_CHECK_ROUND $KWANT_CHECK_RUN $KWANT_IDEMPOTENCY_KEY $KWANT_TASK_ID $KWANT_CHECK_ID\" >> env-{name}.log; \
         echo \"poll $n of {name}\"; exit $(sed -n \"${{n}}p\" codes-{name})"
    )
}

/// The env lines a scripted check logged: `[round, run, key, task id, check id]` each.
fn env_log(dir: &std::path::Path, name: &str) -> Vec<Vec<String>> {
    std::fs::read_to_string(dir.join(format!("env-{name}.log"))).unwrap_or_default().lines().map(|l| l.split(' ').map(String::from).collect()).collect()
}

struct Task {
    id: String,
    dir: std::path::PathBuf,
    token: String,
}

/// A ready task owned by the bot, in a mission the person made; its folder exists.
async fn new_task(e: &Env) -> Task {
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": e.bot.id, "budget_tokens": 200_000,
                      "channel_id": e.general, "tasks": [{"title": "Deploy", "owner": e.bot.id, "acceptance": "the deploy is live"}]});
    let m = person(e, "POST", "/api/missions", Some(body)).await;
    let mid = m["id"].as_str().unwrap().to_string();
    let detail = person(e, "GET", &format!("/api/missions/{mid}"), None).await;
    let id = detail["tasks"][0]["id"].as_str().unwrap().to_string();
    let mission = e.state.db.get_mission(&mid).await.unwrap().unwrap();
    let dir = std::path::PathBuf::from(mission.workspace_path);
    std::fs::create_dir_all(&dir).unwrap();
    let token = turn(e, &format!("t-{id}")).await;
    Task { id, dir, token }
}

/// The person adds a check to the task; returns its id.
async fn add_check(e: &Env, t: &Task, kind: &str, spec: Value) -> String {
    let c = person(e, "POST", &format!("/api/tasks/{}/checks", t.id), Some(json!({"kind": kind, "spec": spec}))).await;
    c["id"].as_str().or(c["check"]["id"].as_str()).unwrap_or_else(|| panic!("no check id in {c}")).to_string()
}

/// A scripted command check answering `codes` in turn.
async fn add_scripted(e: &Env, t: &Task, name: &str, codes: &[i32], extra: Value) -> String {
    std::fs::write(t.dir.join(format!("codes-{name}")), codes.iter().map(|c| format!("{c}\n")).collect::<String>()).unwrap();
    let mut spec = json!({"command": scripted(name)});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        spec[k] = v;
    }
    add_check(e, t, "command", spec).await
}

/// The bot claims the task and hands it in: kwant runs its checks.
async fn hand_in(e: &Env, t: &Task) {
    let token = turn(e, &format!("t-{}", uuid::Uuid::new_v4())).await;
    ok(e, &token, "task_claim", json!({"task_id": t.id})).await;
    ok(e, &token, "task_update", json!({"task_id": t.id, "status": "review"})).await;
}

#[tokio::test]
async fn exit_75_waits_in_review_with_no_failure_and_no_autofix() {
    let e = setup("on").await;
    let t = new_task(&e).await;
    let cid = add_scripted(&e, &t, "a", &[75], json!({})).await;
    let before = Utc::now();
    hand_in(&e, &t).await;

    let v = task(&e, &t.id).await;
    assert_eq!(v["task"]["state"], "review", "a waiting check holds the task in review: {v}");
    assert_eq!(v["task"]["attempts"], 0, "no attempt used: {v}");
    let c = check(&v, &cid);
    assert_eq!(c["status"], "waiting", "{c}");
    assert_eq!(c["runs"], 1);
    // No recheck_mins or wait_hours in the spec: the defaults, 15 minutes and 72 hours
    let since = at(&c["waiting_since"]);
    assert!(since >= before - Duration::seconds(1) && near(since, Utc::now()), "{c}");
    assert!(near(at(&c["next_run_at"]), since + Duration::minutes(15)), "{c}");
    assert!(near(at(&c["wait_until"]), since + Duration::hours(72)), "{c}");
    assert!(c["timed_out_at"].is_null());
    assert!(c["output_excerpt"].as_str().unwrap().contains("poll 1 of a"), "{c}");
    let summary = &v["task"]["checks_summary"];
    assert_eq!(summary["status"], "pending", "{summary}");
    assert_eq!(summary["waiting"], 1, "{summary}");
    assert_eq!(summary["failed"], 0, "{summary}");
    assert_eq!(v["task"]["needs_you"], json!([]), "waiting is never a Needs-you item: {v}");
    assert!(e.state.db.failures_of(&t.id).await.unwrap().is_empty(), "no failure row");
    assert!(e.state.db.attempts_of(&t.id).await.unwrap().is_empty(), "no AutoFix attempt");
    assert!(v["evidence"].as_array().unwrap().iter().any(|ev| ev["kind"] == "check_output" && ev["label"] == "command check: waiting"), "{v}");

    // The env: the task, the check, run 1 of round 1, the round's key
    let log = env_log(&t.dir, "a");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0][0], "1");
    assert_eq!(log[0][1], "1");
    assert_eq!(log[0][2], missions::checks::idempotency_key(&cid, 1));
    assert_eq!(log[0][3], t.id);
    assert_eq!(log[0][4], cid);
}

#[tokio::test]
async fn the_tick_runs_only_due_checks_and_75_75_0_verifies() {
    let e = setup("on").await;
    let t = new_task(&e).await;
    let fast = add_scripted(&e, &t, "fast", &[75, 75, 0], json!({"recheck_mins": 5})).await;
    let slow = add_scripted(&e, &t, "slow", &[75, 0], json!({"recheck_mins": 60})).await;
    let now = Utc::now();
    hand_in(&e, &t).await;
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &fast)["status"], "waiting");
    assert_eq!(check(&v, &slow)["status"], "waiting");
    assert_eq!(v["task"]["checks_summary"]["waiting"], 2);

    assert_eq!(missions::checks::run_due(&e.state, now + Duration::minutes(1)).await, 0, "nothing due yet");
    assert_eq!(missions::checks::run_due(&e.state, now + Duration::minutes(6)).await, 1, "only the 5-minute check");
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &fast)["runs"], 2);
    assert_eq!(check(&v, &fast)["status"], "waiting");
    assert_eq!(check(&v, &slow)["runs"], 1, "not due: not run");
    assert!(near(at(&check(&v, &fast)["next_run_at"]), now + Duration::minutes(11)));
    // The wait started at the first run and keeps its deadline
    assert!(near(at(&check(&v, &fast)["waiting_since"]), now));

    assert_eq!(missions::checks::run_due(&e.state, now + Duration::minutes(12)).await, 1);
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &fast)["status"], "passed", "75, 75, 0 passes: {v}");
    assert_eq!(check(&v, &fast)["runs"], 3);
    assert!(check(&v, &fast)["next_run_at"].is_null() && check(&v, &fast)["waiting_since"].is_null());
    assert_eq!(v["task"]["state"], "review", "the slow check still waits");

    assert_eq!(missions::checks::run_due(&e.state, now + Duration::minutes(61)).await, 1);
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &slow)["status"], "passed");
    assert_eq!(v["task"]["state"], "verified", "{v}");
    assert_eq!(v["task"]["checks_summary"]["status"], "passed");
    assert_eq!(v["task"]["checks_summary"]["waiting"], 0);
    assert!(e.state.db.failures_of(&t.id).await.unwrap().is_empty());
    assert!(e.state.db.attempts_of(&t.id).await.unwrap().is_empty());

    // KWANT_CHECK_RUN counts up; the key stays the same for every run of the round
    let log = env_log(&t.dir, "fast");
    assert_eq!(log.iter().map(|l| l[1].as_str()).collect::<Vec<_>>(), ["1", "2", "3"]);
    assert!(log.iter().all(|l| l[0] == "1" && l[2] == log[0][2]), "{log:?}");
    assert_ne!(env_log(&t.dir, "slow")[0][2], log[0][2], "each check has its own key");
    // Every run's output is evidence
    let runs = v["evidence"].as_array().unwrap().iter().filter(|ev| ev["kind"] == "check_output").count();
    assert_eq!(runs, 5, "{v}");
}

#[tokio::test]
async fn a_fail_after_waiting_goes_to_the_bot_with_its_output_and_the_next_round_has_a_new_key() {
    // suggest: the retry waits for the person, so the next round is this test's to drive
    let e = setup("suggest").await;
    let t = new_task(&e).await;
    let cid = add_scripted(&e, &t, "a", &[75, 1, 0], json!({})).await;
    let now = Utc::now();
    hand_in(&e, &t).await;
    assert_eq!(check(&task(&e, &t.id).await, &cid)["status"], "waiting");
    assert_eq!(missions::checks::run_due(&e.state, now + Duration::minutes(16)).await, 1);

    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &cid)["status"], "failed", "{v}");
    assert_eq!(v["task"]["state"], "failed", "a fail leaves review: {v}");
    let failures = e.state.db.failures_of(&t.id).await.unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].class, "check_failed");
    assert!(failures[0].excerpt.as_deref().unwrap_or("").contains("poll 2 of a"), "the bot gets the output: {:?}", failures[0]);
    let attempts = e.state.db.attempts_of(&t.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!((attempts[0].strategy.as_str(), attempts[0].status.as_str()), ("retry_with_diagnostics", "suggested"));

    // The next round (the person retries; the bot hands in again): a new key
    person(&e, "POST", &format!("/api/tasks/{}/retry", t.id), None).await;
    hand_in(&e, &t).await;
    let v = task(&e, &t.id).await;
    assert_eq!(v["task"]["state"], "verified", "{v}");
    let log = env_log(&t.dir, "a");
    assert_eq!(log.len(), 3);
    assert_eq!((log[0][0].as_str(), log[0][1].as_str()), ("1", "1"));
    assert_eq!((log[1][0].as_str(), log[1][1].as_str()), ("1", "2"));
    assert_eq!((log[2][0].as_str(), log[2][1].as_str()), ("2", "1"), "a new round starts at run 1");
    assert_eq!(log[0][2], log[1][2]);
    assert_ne!(log[2][2], log[0][2], "the round after a fail has a new key");
    assert_eq!(log[2][2], missions::checks::idempotency_key(&cid, 2));
}

#[tokio::test]
async fn past_wait_hours_it_waited_too_long_once_and_wait_more_rearms_it() {
    let e = setup("on").await;
    let t = new_task(&e).await;
    let cid = add_scripted(&e, &t, "a", &[75, 75, 75, 0], json!({"wait_hours": 1, "recheck_mins": 5})).await;
    let now = Utc::now();
    hand_in(&e, &t).await;
    let deadline = at(&check(&task(&e, &t.id).await, &cid)["wait_until"]);
    assert!(near(deadline, now + Duration::hours(1)));

    assert_eq!(missions::checks::run_due(&e.state, now + Duration::hours(2)).await, 1);
    let v = task(&e, &t.id).await;
    let c = check(&v, &cid);
    assert_eq!(c["status"], "error", "{c}");
    assert!(!c["timed_out_at"].is_null(), "{c}");
    assert_eq!(v["task"]["state"], "review", "it stays in review for the person: {v}");
    assert_eq!(v["task"]["attempts"], 0);
    assert_eq!(v["task"]["checks_summary"]["status"], "pending", "{v}");
    assert_eq!(v["task"]["checks_summary"]["failed"], 0);
    let failures = e.state.db.failures_of(&t.id).await.unwrap();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].class, "waited_too_long");
    let attempts = e.state.db.attempts_of(&t.id).await.unwrap();
    assert!(attempts.iter().all(|a| a.status == "skipped" && a.reason.starts_with("not_retryable")), "never retried: {attempts:?}");
    let items = v["task"]["needs_you"].as_array().unwrap();
    assert_eq!(items.len(), 1, "one Needs-you item: {items:?}");
    assert_eq!(items[0]["kind"], "check_timed_out");
    assert_eq!(items[0]["id"], cid.as_str());
    let text = items[0]["text"].as_str().unwrap();
    assert!(text.contains("didn't answer in 1 h. Wait longer, or verify by hand."), "{text}");
    assert_eq!(items[0]["actions"], json!(["wait_more", "verify"]));
    // Not waiting any more: the tick leaves it alone
    assert_eq!(missions::checks::run_due(&e.state, now + Duration::hours(3)).await, 0);

    // "Wait 24 h more" is the person's
    let token = t.token.clone();
    let (status, _) = call(&e.app, "POST", &format!("/api/checks/{cid}/wait-more"), Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let c = person(&e, "POST", &format!("/api/checks/{cid}/wait-more"), None).await;
    assert_eq!(c["status"], "waiting", "{c}");
    assert!(near(at(&c["wait_until"]), deadline + Duration::hours(24)), "{c}");
    assert!(at(&c["next_run_at"]) <= Utc::now(), "due at once: {c}");
    assert_eq!(task(&e, &t.id).await["task"]["needs_you"], json!([]));

    assert_eq!(missions::checks::run_due(&e.state, now + Duration::hours(2)).await, 1);
    assert_eq!(check(&task(&e, &t.id).await, &cid)["status"], "waiting", "inside the new deadline");
    assert_eq!(missions::checks::run_due(&e.state, now + Duration::hours(2) + Duration::minutes(6)).await, 1);
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &cid)["status"], "passed", "{v}");
    assert_eq!(v["task"]["state"], "verified");
    assert_eq!(e.state.db.failures_of(&t.id).await.unwrap().len(), 1, "still the one failure");
    // The key never changed: all four runs were round 1
    let log = env_log(&t.dir, "a");
    assert_eq!(log.len(), 4);
    assert!(log.iter().all(|l| l[0] == "1" && l[2] == log[0][2]), "{log:?}");
}

#[tokio::test]
async fn a_check_that_waited_too_long_can_be_verified_by_hand() {
    let e = setup("on").await;
    let t = new_task(&e).await;
    let cid = add_scripted(&e, &t, "a", &[75, 75], json!({"wait_hours": 1})).await;
    let now = Utc::now();
    hand_in(&e, &t).await;
    missions::checks::run_due(&e.state, now + Duration::hours(2)).await;
    assert!(!check(&task(&e, &t.id).await, &cid)["timed_out_at"].is_null());
    person(&e, "POST", &format!("/api/tasks/{}/verify", t.id), Some(json!({"note": "checked the dashboard"}))).await;
    let v = task(&e, &t.id).await;
    assert_eq!(v["task"]["state"], "verified", "{v}");
    assert_eq!(v["task"]["needs_you"], json!([]));
}

#[tokio::test]
async fn a_restart_keeps_a_waiting_check_and_its_schedule() {
    let e = setup("on").await;
    let t = new_task(&e).await;
    let cid = add_scripted(&e, &t, "a", &[75, 75], json!({"recheck_mins": 30})).await;
    let other = add_scripted(&e, &t, "b", &[75, 75], json!({})).await;
    hand_in(&e, &t).await;
    let before = e.state.db.get_check(&cid).await.unwrap().unwrap();
    assert_eq!(before.status, "waiting");
    // kwnt stops while `other` runs again
    assert!(e.state.db.start_check(&other).await.unwrap());

    let db = Database::init(e.tmp.path()).await.unwrap();
    let after = db.get_check(&cid).await.unwrap().unwrap();
    assert_eq!(after.status, "waiting");
    assert_eq!(after.next_run_at, before.next_run_at, "the schedule survives");
    assert_eq!(after.wait_until, before.wait_until);
    assert_eq!(after.waiting_since, before.waiting_since);
    assert_eq!(after.runs, 1);
    let cut = db.get_check(&other).await.unwrap().unwrap();
    assert_eq!(cut.status, "waiting", "a waiting check cut short by the restart waits again, not error");
    assert!(cut.next_run_at.unwrap() <= Utc::now(), "and is due at once");
    assert_eq!(db.due_waiting_checks(Utc::now(), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_database_from_before_waiting_checks_is_rebuilt_with_its_rows_and_triggers() {
    use sqlx::sqlite::SqlitePoolOptions;
    let e = setup("off").await;
    let t = new_task(&e).await;
    let cid = add_check(&e, &t, "file_exists", json!({"path": "x"})).await;
    let url = format!("sqlite://{}", e.tmp.path().join("kwnt.db").display());
    let pool = SqlitePoolOptions::new().max_connections(1).connect(&url).await.unwrap();
    let triggers = || async {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'").fetch_one(&pool).await.unwrap();
        n
    };
    let count = triggers().await;
    // Put both CHECK lists back as an older kwant made them
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *conn).await.unwrap();
    sqlx::query("PRAGMA legacy_alter_table = ON").execute(&mut *conn).await.unwrap();
    for (table, new, old) in [
        ("outcome_checks", "'failed', 'error', 'waiting'))", "'failed', 'error'))"),
        ("task_failures", "'unknown', 'waited_too_long'))", "'unknown'))"),
    ] {
        let (sql,): (String,) = sqlx::query_as("SELECT sql FROM sqlite_master WHERE name = ?").bind(table).fetch_one(&mut *conn).await.unwrap();
        assert!(sql.contains(new), "{sql}");
        let old_sql = sql.replace(new, old).replacen(&format!("CREATE TABLE {table} "), &format!("CREATE TABLE {table}_old "), 1);
        sqlx::query(&old_sql).execute(&mut *conn).await.unwrap();
        sqlx::query(&format!("INSERT INTO {table}_old SELECT * FROM {table}")).execute(&mut *conn).await.unwrap();
        sqlx::query(&format!("DROP TABLE {table}")).execute(&mut *conn).await.unwrap();
        sqlx::query(&format!("ALTER TABLE {table}_old RENAME TO {table}")).execute(&mut *conn).await.unwrap();
    }
    let (sql,): (String,) = sqlx::query_as("SELECT sql FROM sqlite_master WHERE name = 'outcome_checks'").fetch_one(&mut *conn).await.unwrap();
    assert!(!sql.contains("'waiting'"));
    drop(conn);
    pool.close().await;

    let db = Database::init(e.tmp.path()).await.unwrap();
    let pool = SqlitePoolOptions::new().max_connections(1).connect(&url).await.unwrap();
    for (table, want) in [("outcome_checks", "'waiting'"), ("task_failures", "'waited_too_long'")] {
        let (sql,): (String,) = sqlx::query_as("SELECT sql FROM sqlite_master WHERE name = ?").bind(table).fetch_one(&pool).await.unwrap();
        assert!(sql.contains(want), "{table}: {sql}");
    }
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'").fetch_one(&pool).await.unwrap();
    assert!(n >= count, "every trigger is back: {n} of {count}");
    assert_eq!(db.get_check(&cid).await.unwrap().unwrap().kind, "file_exists", "rows kept");
    pool.close().await;
}

/// A server whose `/job` and `/page` answer the next queued `(status, body)` each.
async fn server(job: Vec<(u16, &'static str)>, page: Vec<(u16, &'static str)>) -> String {
    type Q = Arc<Mutex<VecDeque<(u16, &'static str)>>>;
    async fn next(State(q): State<Q>) -> (StatusCode, &'static str) {
        let (s, b) = q.lock().unwrap().pop_front().unwrap_or((500, "empty"));
        (StatusCode::from_u16(s).unwrap(), b)
    }
    let app = Router::new()
        .route("/job", get(next).with_state(Arc::new(Mutex::new(VecDeque::from(job)))))
        .route("/page", get(next).with_state(Arc::new(Mutex::new(VecDeque::from(page)))));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn http_waiting_status_and_waiting_contains() {
    let e = setup("on").await;
    let base = server(vec![(202, "queued"), (200, "ok")], vec![(200, "status: pending"), (200, "status: done")]).await;
    let t = new_task(&e).await;
    let job = add_check(&e, &t, "http", json!({"url": format!("{base}/job"), "waiting_status": 202})).await;
    let page = add_check(&e, &t, "http", json!({"url": format!("{base}/page"), "contains": "status", "waiting_contains": "pending"})).await;
    let now = Utc::now();
    hand_in(&e, &t).await;
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &job)["status"], "waiting", "202 is a 2xx, but the waiting status: {v}");
    assert_eq!(check(&v, &page)["status"], "waiting", "contains matched, but so did waiting_contains: {v}");
    assert!(check(&v, &page)["output_excerpt"].as_str().unwrap().contains("not ready yet"));
    assert_eq!(v["task"]["state"], "review");
    assert!(e.state.db.failures_of(&t.id).await.unwrap().is_empty());

    assert_eq!(missions::checks::run_due(&e.state, now + Duration::minutes(16)).await, 2);
    let v = task(&e, &t.id).await;
    assert_eq!(check(&v, &job)["status"], "passed", "{v}");
    assert_eq!(check(&v, &page)["status"], "passed", "{v}");
    assert_eq!(v["task"]["state"], "verified");
}

#[tokio::test]
async fn bots_can_set_the_wait_fields_within_their_bounds() {
    let e = setup("off").await;
    let t = new_task(&e).await;
    for (spec, why) in [
        (json!({"command": "true", "recheck_mins": 4}), "recheck_mins"),
        (json!({"command": "true", "recheck_mins": 1441}), "recheck_mins"),
        (json!({"command": "true", "wait_hours": 0}), "wait_hours"),
        (json!({"command": "true", "wait_hours": 337}), "wait_hours"),
        (json!({"command": "true", "wait_hours": "a day"}), "wait_hours"),
    ] {
        let (fine, text) = tool(&e, &t.token, "task_request_check", json!({"task_id": t.id, "kind": "command", "spec": spec})).await;
        assert!(!fine && text.contains(why), "{spec}: {text}");
    }
    let (fine, text) = tool(&e, &t.token, "task_request_check", json!({"task_id": t.id, "kind": "http", "spec": {"url": "https://example.com", "waiting_status": 42}})).await;
    assert!(!fine && text.contains("waiting_status"), "{text}");
    // In range: accepted (a bot's command check still waits for the person's approval, as before)
    let (fine, text) = tool(&e, &t.token, "task_request_check", json!({"task_id": t.id, "kind": "command", "spec": {"command": "true", "recheck_mins": 5, "wait_hours": 336}})).await;
    assert!(fine, "{text}");
    let (fine, text) = tool(&e, &t.token, "mission_plan", json!({
        "title": "Gate", "goal": "wait on a job",
        "tasks": [{"key": "a", "title": "Job", "owner": e.bot.id, "checks": [{"kind": "command", "spec": {"command": "true", "recheck_mins": 2}}]}]
    })).await;
    assert!(!fine && text.contains("recheck_mins"), "{text}");
    let (fine, text) = tool(&e, &t.token, "mission_plan", json!({
        "title": "Gate", "goal": "wait on a job",
        "tasks": [{"key": "a", "title": "Job", "owner": e.bot.id, "checks": [{"kind": "command", "spec": {"command": "true", "recheck_mins": 1440, "wait_hours": 1}}]}]
    })).await;
    assert!(fine, "{text}");
}
