//! Mission Control (Phase A): one task board where a task is `verified` only by an outcome check
//! that isn't its owner's own claim. Covers the hard rules: verification, dependencies, atomic
//! claims with expiring leases, the append-only event log, the mission budget, person-only routes,
//! approval of bot-added commands, folder escapes, and the run hook Rigel's eval drives.

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
    other: Kwant,
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
    let e = Env { app: create_router(state.clone()), bot: kwants[0].clone(), other: kwants[1].clone(), general, state, _tmp: tmp };
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

async fn ok(e: &Env, token: &str, name: &str, args: Value) -> Value {
    let (v, err, text) = tool(e, token, name, args.clone()).await;
    assert!(!err, "{name} {args} failed: {text}");
    v
}

async fn refused(e: &Env, token: &str, name: &str, args: Value) -> String {
    let (_, err, text) = tool(e, token, name, args.clone()).await;
    assert!(err, "{name} {args} should be refused: {text}");
    text
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

async fn state_of(e: &Env, id: &str) -> String {
    task(e, id).await["task"]["state"].as_str().unwrap().to_string()
}

/// A mission the person creates (active at once), led by `e.bot`; returns `(mission, tasks)`.
async fn mission(e: &Env, budget: i64, tasks: Value) -> (Value, Vec<Value>) {
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": e.bot.id, "budget_tokens": budget,
                      "channel_id": e.general, "tasks": tasks});
    let m = person(e, "POST", "/api/missions", Some(body)).await;
    let detail = person(e, "GET", &format!("/api/missions/{}", m["id"].as_str().unwrap()), None).await;
    let mut tasks = detail["tasks"].as_array().unwrap().clone();
    tasks.sort_by_key(|t| t["title"].as_str().unwrap().to_string());
    (detail["mission"].clone(), tasks)
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

async fn folder(e: &Env, mission_id: &str) -> std::path::PathBuf {
    let m = e.state.db.get_mission(mission_id).await.unwrap().unwrap();
    let p = std::path::PathBuf::from(m.workspace_path);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[tokio::test]
async fn tools_exist_only_while_the_board_is_on() {
    let e = setup().await;
    let token = turn(&e, &e.bot, "t-list").await;
    let names = |v: &Value| v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), Some(rpc("tools/list", json!({})))).await;
    assert!(names(&v).contains(&"task_claim".to_string()));
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": false}))).await;
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), Some(rpc("tools/list", json!({})))).await;
    assert!(!names(&v).iter().any(|n| n.starts_with("task_") || n == "mission_plan"));
    refused(&e, &token, "task_list", json!({})).await;
}

#[tokio::test]
async fn a_bot_cannot_verify_its_own_work_only_a_passing_check_does() {
    let e = setup().await;
    let token = turn(&e, &e.bot, "t-plan").await;
    let plan = ok(&e, &token, "mission_plan", json!({
        "title": "Notes", "goal": "write notes",
        "tasks": [{"key": "a", "title": "Write notes", "owner": e.bot.id,
                   "checks": [{"kind": "file_contains", "spec": {"path": "notes.md", "text": "done"}}]}]
    })).await;
    let mid = plan["mission"]["id"].as_str().or(plan["mission_id"].as_str()).unwrap().to_string();
    let detail = person(&e, "GET", &format!("/api/missions/{mid}"), None).await;
    assert_eq!(detail["mission"]["status"], "draft");
    let tid = id(&detail["tasks"][0]);
    assert_eq!(state_of(&e, &tid).await, "backlog", "nothing runs before the person approves");
    refused(&e, &token, "task_claim", json!({"task_id": tid})).await;

    // Approval is the person's
    let (status, _) = call(&e.app, "POST", &format!("/api/missions/{mid}/approve"), Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    person(&e, "POST", &format!("/api/missions/{mid}/approve"), None).await;
    assert_eq!(state_of(&e, &tid).await, "ready");

    ok(&e, &token, "task_claim", json!({"task_id": tid})).await;
    refused(&e, &token, "task_update", json!({"task_id": tid, "status": "verified"})).await;
    // Claiming done without the outcome: the check fails, the task goes back to ready
    ok(&e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    let t = task(&e, &tid).await;
    assert_eq!(t["task"]["state"], "ready", "{t}");
    assert_eq!(t["task"]["attempts"], 1);
    assert_eq!(t["checks"][0]["status"], "failed");
    assert_eq!(t["checks"][0]["checked_by"], "kwant");
    let insights = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(insights["false_success_7d"], 1, "{insights}");

    // Now the outcome is real
    std::fs::write(folder(&e, &mid).await.join("notes.md"), "all done").unwrap();
    ok(&e, &token, "task_claim", json!({"task_id": tid})).await;
    ok(&e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    let t = task(&e, &tid).await;
    assert_eq!(t["task"]["state"], "verified", "{t}");
    assert_eq!(t["task"]["checks_summary"]["status"], "passed");
    assert!(t["evidence"].as_array().unwrap().iter().any(|ev| ev["kind"] == "check_output" && ev["captured_by"] == "kwant"));
    let kinds: Vec<&str> = t["events"].as_array().unwrap().iter().map(|ev| ev["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"check_result") && kinds.contains(&"claimed"), "{kinds:?}");
    let insights = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(insights["verified_7d"], 1);
}

#[tokio::test]
async fn the_owner_cannot_review_itself_a_teammate_needs_evidence() {
    let e = setup().await;
    let (m, tasks) = mission(&e, 200_000, json!([{"title": "Draft", "owner": e.bot.id}])).await;
    let tid = id(&tasks[0]);
    let token = turn(&e, &e.bot, "t-own").await;
    let peer = turn(&e, &e.other, "t-peer").await;

    ok(&e, &token, "task_claim", json!({"task_id": tid})).await;
    refused(&e, &peer, "task_update", json!({"task_id": tid, "status": "review"})).await;
    let ev = ok(&e, &token, "task_attach_evidence", json!({"task_id": tid, "kind": "note", "label": "draft", "text": "here it is"})).await;
    let ev_id = ev["evidence_id"].as_str().unwrap().to_string();
    ok(&e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    let t = task(&e, &tid).await;
    assert_eq!(t["task"]["state"], "review", "no checks: it waits for a review");
    assert_eq!(t["task"]["checks_summary"]["status"], "not_checked");

    refused(&e, &token, "task_review", json!({"task_id": tid, "verdict": "pass", "evidence_ids": [ev_id], "note": "lgtm"})).await;
    refused(&e, &peer, "task_review", json!({"task_id": tid, "verdict": "pass", "evidence_ids": [], "note": "lgtm"})).await;
    refused(&e, &peer, "task_review", json!({"task_id": tid, "verdict": "pass", "evidence_ids": ["not-on-this-task"], "note": "lgtm"})).await;
    ok(&e, &peer, "task_review", json!({"task_id": tid, "verdict": "pass", "evidence_ids": [ev_id], "note": "read it"})).await;
    let t = task(&e, &tid).await;
    assert_eq!(t["task"]["state"], "verified", "{t}");
    let review = t["checks"].as_array().unwrap().iter().find(|c| c["kind"] == "peer_review").unwrap().clone();
    assert_eq!(review["checked_by"], e.other.id.as_str());
    assert_eq!(review["status"], "passed");
    let _ = m;
}

#[tokio::test]
async fn a_task_is_ready_only_after_its_dependencies_are_verified() {
    let e = setup().await;
    let token = turn(&e, &e.bot, "t-deps").await;
    let plan = ok(&e, &token, "mission_plan", json!({
        "title": "Two steps", "goal": "in order",
        "tasks": [{"key": "a", "title": "A first", "owner": e.bot.id},
                  {"key": "b", "title": "B second", "owner": e.bot.id, "depends_on": ["a"]}]
    })).await;
    let mid = plan["mission"]["id"].as_str().or(plan["mission_id"].as_str()).unwrap().to_string();
    person(&e, "POST", &format!("/api/missions/{mid}/approve"), None).await;
    let detail = person(&e, "GET", &format!("/api/missions/{mid}"), None).await;
    let by_title = |t: &str| detail["tasks"].as_array().unwrap().iter().find(|x| x["title"] == t).map(id).unwrap();
    let (a, b) = (by_title("A first"), by_title("B second"));
    let tb = task(&e, &b).await;
    assert_eq!(tb["task"]["state"], "backlog");
    assert_eq!(tb["task"]["waiting_on"], json!([a]));
    refused(&e, &token, "task_claim", json!({"task_id": b})).await;
    // Not even a raw write gets around it
    let raw = e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'ready' WHERE id = '{b}'")).await;
    assert!(raw.is_err(), "a raw write must not make a waiting task ready");

    ok(&e, &token, "task_claim", json!({"task_id": a})).await;
    ok(&e, &token, "task_update", json!({"task_id": a, "status": "review"})).await;
    assert_eq!(state_of(&e, &b).await, "backlog", "review is not verified");
    person(&e, "POST", &format!("/api/tasks/{a}/verify"), Some(json!({"note": "checked by hand"}))).await;
    assert_eq!(state_of(&e, &a).await, "verified");
    assert_eq!(state_of(&e, &b).await, "ready");
}

#[tokio::test]
async fn claims_are_atomic_one_at_a_time_and_leases_expire() {
    let e = setup().await;
    let (_, tasks) = mission(&e, 500_000, json!([{"title": "One"}, {"title": "Two"}])).await;
    let (one, two) = (id(&tasks[0]), id(&tasks[1]));
    let db = &e.state.db;
    let (r1, r2) = tokio::join!(
        db.claim_task(&one, &e.bot.id, None, None, 60_000, false, &e.bot.id),
        db.claim_task(&one, &e.other.id, None, None, 60_000, false, &e.other.id),
    );
    let wins = [r1.unwrap().is_ok(), r2.unwrap().is_ok()];
    assert_eq!(wins.iter().filter(|w| **w).count(), 1, "exactly one claim wins: {wins:?}");
    let holder = if wins[0] { &e.bot } else { &e.other };
    // One live claim per bot
    assert!(db.claim_task(&two, &holder.id, None, None, 60_000, false, &holder.id).await.unwrap().is_err());

    db.expire_claim_now(&one).await.unwrap();
    assert!(missions::sweep_leases(&e.state).await >= 1);
    let t = task(&e, &one).await;
    assert_eq!(t["task"]["state"], "ready");
    assert!(t["task"]["claim"].is_null());
    assert!(t["events"].as_array().unwrap().iter().any(|ev| ev["kind"] == "lease_expired"));
    // A worked-on task without a live lease is refused by the database itself
    let raw = db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'working' WHERE id = '{one}'")).await;
    assert!(raw.is_err());
}

#[tokio::test]
async fn the_event_log_is_append_only_and_verified_needs_a_check() {
    let e = setup().await;
    let (_, tasks) = mission(&e, 200_000, json!([{"title": "Only"}])).await;
    let tid = id(&tasks[0]);
    let db = &e.state.db;
    assert!(!task(&e, &tid).await["events"].as_array().unwrap().is_empty());
    assert!(db.exec_raw_for_test("UPDATE task_events SET kind = 'note'").await.is_err());
    assert!(db.exec_raw_for_test("DELETE FROM task_events").await.is_err());
    assert!(db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'verified' WHERE id = '{tid}'")).await.is_err());
    // Verify from anywhere but review is refused through the API too
    let (status, v) = call(&e.app, "POST", &format!("/api/tasks/{tid}/verify"), None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn the_mission_budget_is_never_exceeded() {
    let e = setup().await;
    let (m, tasks) = mission(&e, 10_000, json!([
        {"title": "A", "owner": e.bot.id, "budget_tokens": 6_000},
        {"title": "B", "owner": e.other.id, "budget_tokens": 6_000}
    ])).await;
    let db = &e.state.db;
    assert!(db.claim_task(&id(&tasks[0]), &e.bot.id, None, None, 60_000, false, &e.bot.id).await.unwrap().is_ok());
    let second = db.claim_task(&id(&tasks[1]), &e.other.id, None, None, 60_000, false, &e.other.id).await.unwrap();
    assert!(matches!(second, Err(kwnt::db::missions::ClaimRefused::Budget { .. })), "{second:?}");
    let detail = person(&e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    assert_eq!(detail["mission"]["reserved_tokens"], 6_000);
    assert_eq!(detail["mission"]["over_budget"], false);
    // The person can't set the budget below what's already reserved or spent either
    let (status, _) = call(&e.app, "PATCH", &format!("/api/missions/{}", id(&m)), None, Some(json!({"budget_tokens": 10}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn board_changes_are_the_persons_bot_tokens_get_403() {
    let e = setup().await;
    let (m, tasks) = mission(&e, 200_000, json!([{"title": "X"}])).await;
    let token = turn(&e, &e.bot, "t-403").await;
    let tid = id(&tasks[0]);
    for (method, uri) in [
        ("POST", "/api/missions".to_string()),
        ("POST", format!("/api/missions/{}/run", id(&m))),
        ("POST", format!("/api/missions/{}/cancel", id(&m))),
        ("POST", "/api/tasks".to_string()),
        ("POST", format!("/api/tasks/{tid}/verify")),
        ("PATCH", format!("/api/tasks/{tid}")),
        ("POST", format!("/api/tasks/{tid}/checks")),
        ("POST", "/api/checks/x/approve".to_string()),
    ] {
        let (status, _) = call(&e.app, method, &uri, Some(&token), Some(json!({"title": "t", "reason": "r"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    let (status, _) = call(&e.app, "GET", "/api/tasks", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "reads stay open");
}

#[tokio::test]
async fn a_command_a_bot_adds_later_waits_for_the_persons_approval() {
    let e = setup().await;
    let (m, tasks) = mission(&e, 200_000, json!([{"title": "Build", "owner": e.bot.id}])).await;
    let tid = id(&tasks[0]);
    let token = turn(&e, &e.bot, "t-cmd").await;
    let marker = folder(&e, &id(&m)).await.join("ran.txt");
    let added = ok(&e, &token, "task_request_check", json!({"task_id": tid, "kind": "command", "spec": {"command": "echo hi > ran.txt"}})).await;
    let check = added["checks"].as_array().unwrap().iter().find(|c| c["kind"] == "command").unwrap().clone();
    assert_eq!(check["approved"], false, "{added}");
    assert!(!marker.exists(), "an unapproved command never runs");
    // A URL on this computer is the person's call too, even a dev-server port WebFetch allows
    let local = ok(&e, &token, "task_request_check", json!({"task_id": tid, "kind": "http", "spec": {"url": "http://127.0.0.1:5173/"}})).await;
    assert!(local["checks"].as_array().unwrap().iter().all(|c| c["approved"] == false), "{local}");
    let t = task(&e, &tid).await;
    assert!(t["checks"].as_array().unwrap().iter().all(|c| c["status"] == "needs_approval"), "{t}");
    let (status, _) = call(&e.app, "POST", &format!("/api/checks/{}/approve", id(&check)), Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let approved = person(&e, "POST", &format!("/api/checks/{}/approve", id(&check)), None).await;
    assert_eq!(approved["approved"], true);
    // A command the person adds is approved at once
    let c = person(&e, "POST", &format!("/api/tasks/{tid}/checks"), Some(json!({"kind": "command", "spec": {"command": "true"}}))).await;
    assert_eq!(c["approved"], true);
    let run = person(&e, "POST", &format!("/api/tasks/{tid}/checks/run"), None).await;
    assert!(run["checks"].as_array().unwrap().iter().all(|c| c["status"] == "passed"), "{run}");
    assert!(marker.exists());
}

#[tokio::test]
async fn evidence_and_checks_stay_inside_the_folder_and_skip_secrets() {
    let e = setup().await;
    let (m, tasks) = mission(&e, 200_000, json!([{"title": "Files", "owner": e.bot.id}])).await;
    let tid = id(&tasks[0]);
    let dir = folder(&e, &id(&m)).await;
    std::fs::write(dir.join(".env"), "TOKEN=x").unwrap();
    std::fs::write(dir.join("out.txt"), "hello").unwrap();
    let token = turn(&e, &e.bot, "t-files").await;
    ok(&e, &token, "task_claim", json!({"task_id": tid})).await;
    for path in ["../../etc/passwd", "/etc/passwd", ".env"] {
        refused(&e, &token, "task_attach_evidence", json!({"task_id": tid, "kind": "file", "label": "x", "path": path})).await;
    }
    let ev = ok(&e, &token, "task_attach_evidence", json!({"task_id": tid, "kind": "file", "label": "out", "path": "out.txt", "sha256": "fake"})).await;
    assert_eq!(ev["bytes"], 5);
    assert_eq!(ev["sha256"], "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", "kwant hashes it itself");
    let c = person(&e, "POST", &format!("/api/tasks/{tid}/checks"), Some(json!({"kind": "file_exists", "spec": {"path": "../outside"}}))).await;
    let run = person(&e, "POST", &format!("/api/tasks/{tid}/checks/run"), None).await;
    let res = run["checks"].as_array().unwrap().iter().find(|x| x["id"] == c["id"]).unwrap().clone();
    assert_ne!(res["status"], "passed", "{res}");
}

#[tokio::test]
async fn the_run_hook_starts_one_turn_per_mission_and_releases_an_unfinished_task() {
    let e = setup().await;
    let (m, tasks) = mission(&e, 200_000, json!([{"title": "Go", "owner": e.bot.id}])).await;
    let tid = id(&tasks[0]);
    let out = person(&e, "POST", &format!("/api/missions/{}/run", id(&m)), Some(json!({"max_tasks": 1}))).await;
    assert_eq!(out["started"][0]["task_id"], tid.as_str(), "{out}");
    assert_eq!(out["started"][0]["kwant_id"], e.bot.id.as_str());
    let (status, _) = call(&e.app, "POST", &format!("/api/missions/{}/run", id(&m)), None, Some(json!({"max_tasks": 9}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // No engine is set up here, so the turn ends without a review: the lease is let go
    let mut released = false;
    for _ in 0..100 {
        let t = task(&e, &tid).await;
        if t["events"].as_array().unwrap().iter().any(|ev| ev["kind"] == "released") {
            assert_eq!(t["task"]["attempts"], 1);
            assert_ne!(t["task"]["state"], "working");
            released = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(released, "the runner releases a task its turn didn't finish");
}
