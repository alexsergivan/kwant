//! Mission Control Phase B1: the work graph. Evidence drift and forks of verified work raise
//! "may be invalid" flags on the task and its started direct dependents, never move a task, and are
//! resolved only by the person. Fork redoes a task without touching verified history. These are
//! the contract's 11 acceptance tests (docs/mission-control.md §11), numbered as there.

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
        create_router, mcp_runtime, AppState,
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


fn flags(t: &Value) -> Vec<Value> {
    t["task"]["flags"].as_array().cloned().unwrap_or_default()
}

fn causes(t: &Value) -> Vec<String> {
    flags(t).iter().map(|f| f["cause"].as_str().unwrap().to_string()).collect()
}

/// A mission of `titles` (one bot owns them all), each checked by `file_exists <key>.txt` and
/// depending on the keys in `deps`; returns the mission id and task ids in `titles` order.
async fn graph_mission(e: &Env, tasks: &[(&str, &[&str])]) -> (String, Vec<String>) {
    let plan: Vec<Value> = tasks
        .iter()
        .map(|(k, deps)| json!({"key": k, "title": format!("{k} step"), "owner": e.bot.id, "depends_on": deps,
                                "checks": [{"kind": "file_exists", "spec": {"path": format!("{k}.txt")}}]}))
        .collect();
    let (m, listed) = mission(e, 900_000, json!(plan)).await;
    let ids = tasks.iter().map(|(k, _)| listed.iter().find(|t| t["title"] == format!("{k} step")).map(id).unwrap()).collect();
    (id(&m), ids)
}

/// The owner does `tid`: writes `<key>.txt`, attaches it as file evidence and hands it in; its
/// plan check passes, so it is verified.
async fn do_task(e: &Env, mid: &str, tid: &str, key: &str) {
    std::fs::write(folder(e, mid).await.join(format!("{key}.txt")), format!("{key} v1")).unwrap();
    let token = turn(e, &e.bot, &format!("t-{tid}")).await;
    ok(e, &token, "task_claim", json!({"task_id": tid})).await;
    ok(e, &token, "task_attach_evidence", json!({"task_id": tid, "kind": "file", "label": key, "path": format!("{key}.txt")})).await;
    ok(e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    assert_eq!(state_of(e, tid).await, "verified");
}

async fn scan(e: &Env, mid: &str) -> Value {
    person(e, "POST", &format!("/api/missions/{mid}/graph/scan"), None).await
}

async fn drift(e: &Env, mid: &str, key: &str) {
    std::fs::write(folder(e, mid).await.join(format!("{key}.txt")), format!("{key} v2, changed")).unwrap();
}

// 1
#[tokio::test]
async fn a_flag_never_changes_a_tasks_state() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    let (_, tasks) = mission(&e, 200_000, json!([{"title": "Draft", "owner": e.bot.id}])).await;
    let r = id(&tasks[0]);
    let token = turn(&e, &e.bot, "t-review").await;
    ok(&e, &token, "task_claim", json!({"task_id": r})).await;
    ok(&e, &token, "task_update", json!({"task_id": r, "status": "review"})).await;
    assert_eq!(state_of(&e, &r).await, "review");

    for (t, cause) in [(&ids[0], "evidence_changed"), (&r, "upstream_forked")] {
        let f = e.state.db.raise_flag(t, cause, &ids[0], &json!({"path": "a.txt"})).await.unwrap();
        assert!(f.is_some());
    }
    assert_eq!(state_of(&e, &ids[0]).await, "verified");
    assert_eq!(state_of(&e, &r).await, "review");
    // The Phase A triggers still hold
    let a = &ids[0];
    assert!(e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'ready' WHERE id = '{a}'")).await.is_err());
    assert!(e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'verified' WHERE id = '{r}'")).await.is_err());
    let events = e.state.db.task_events(a, 100).await.unwrap();
    let flagged = events.iter().find(|ev| ev.kind == "flagged").expect("a flagged event");
    assert_eq!(flagged.actor, "kwant");
    assert!(flagged.from_state.is_none() && flagged.to_state.is_none());
}

// 2
#[tokio::test]
async fn only_the_person_resolves_a_flag_once_and_flags_are_never_deleted() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    let f = e.state.db.raise_flag(&ids[0], "evidence_changed", &ids[0], &json!({})).await.unwrap().unwrap();
    let fid = &f.id;
    for by in ["kwant", &e.bot.id, ""] {
        let sql = format!("UPDATE task_flags SET resolved_at = '2026-10-27T00:00:00Z', resolved_by = '{by}', resolution = 'still_valid' WHERE id = '{fid}'");
        assert!(e.state.db.exec_raw_for_test(&sql).await.is_err(), "resolved_by {by:?} must be refused");
    }
    let sql = format!("UPDATE task_flags SET resolved_at = '2026-10-27T00:00:00Z', resolution = 'still_valid' WHERE id = '{fid}'");
    assert!(e.state.db.exec_raw_for_test(&sql).await.is_err(), "no resolver is refused");
    assert!(e.state.db.resolve_flag(fid, "still_valid", None, None).await.unwrap());
    assert!(!e.state.db.resolve_flag(fid, "redo", None, None).await.unwrap(), "resolved once");
    let sql = format!("UPDATE task_flags SET resolution = 'redo' WHERE id = '{fid}'");
    assert!(e.state.db.exec_raw_for_test(&sql).await.is_err(), "a resolved flag can't change");
    assert!(e.state.db.exec_raw_for_test(&format!("DELETE FROM task_flags WHERE id = '{fid}'")).await.is_err());
    let events = e.state.db.task_events(&ids[0], 100).await.unwrap();
    let resolved = events.iter().find(|ev| ev.kind == "flag_resolved").expect("a flag_resolved event");
    assert_eq!(resolved.actor, "person");
}

// 3
#[tokio::test]
async fn two_scans_of_the_same_drift_give_one_open_flag() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    let clean = scan(&e, &mid).await;
    assert_eq!(clean["scanned"], 1, "{clean}");
    assert_eq!(clean["flagged"], json!([]));
    drift(&e, &mid, "a").await;
    let first = scan(&e, &mid).await;
    assert_eq!(first["flagged"].as_array().unwrap().len(), 1, "{first}");
    let second = scan(&e, &mid).await;
    assert_eq!(second["flagged"], json!([]), "{second}");
    assert_eq!(e.state.db.flags_of(&ids[0], true).await.unwrap().len(), 1);
    // Straight at the table too: the unique open index
    let again = e.state.db.raise_flag(&ids[0], "evidence_changed", &ids[0], &json!({})).await.unwrap();
    assert!(again.is_none());
}

// 4
#[tokio::test]
async fn drift_flags_the_task_and_its_started_dependent_both_stay_verified() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &["a"])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    do_task(&e, &mid, &ids[1], "b").await;
    let before = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(before["needs_you_now"], 0, "{before}");

    drift(&e, &mid, "a").await;
    let r = scan(&e, &mid).await;
    assert_eq!(r["flagged"].as_array().unwrap().len(), 2, "{r}");
    let (a, b) = (task(&e, &ids[0]).await, task(&e, &ids[1]).await);
    assert_eq!(causes(&a), ["evidence_changed"]);
    assert_eq!(causes(&b), ["upstream_evidence_changed"]);
    assert_eq!(a["task"]["state"], "verified");
    assert_eq!(b["task"]["state"], "verified");
    let fa = &flags(&a)[0];
    assert_eq!(fa["detail"]["path"], "a.txt");
    assert!(fa["detail"]["old_sha256"].is_string() && fa["detail"]["new_sha256"].is_string());
    assert_ne!(fa["detail"]["old_sha256"], fa["detail"]["new_sha256"]);
    assert_eq!(flags(&b)[0]["cause_task_id"], ids[0].as_str());
    assert_eq!(flags(&b)[0]["cause_title"], "a step");
    for t in [&a, &b] {
        let items = t["task"]["needs_you"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{t}");
        assert_eq!(items[0]["kind"], "may_be_invalid");
        assert_eq!(items[0]["id"], flags(t)[0]["id"]);
        assert!(items[0]["text"].as_str().unwrap().starts_with("Check this still holds: "));
    }
    let after = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(after["needs_you_now"], 2, "{after}");

    // A file that is gone is drift too, and a new hash of it is a new drift
    std::fs::remove_file(folder(&e, &mid).await.join("a.txt")).unwrap();
    let gone = e.state.db.get_flag(fa["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert!(gone.resolved_at.is_none());
    person(&e, "POST", &format!("/api/tasks/{}/flags/{}/resolve", ids[0], fa["id"].as_str().unwrap()), Some(json!({"resolution": "still_valid"}))).await;
    scan(&e, &mid).await;
    let a = task(&e, &ids[0]).await;
    assert_eq!(flags(&a).len(), 1, "{a}");
    assert!(flags(&a)[0]["detail"]["new_sha256"].is_null());
    assert!(flags(&a)[0]["detail"]["path"] == "a.txt");
}

// 5
#[tokio::test]
async fn only_direct_dependents_are_flagged() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &["a"]), ("c", &["b"])]).await;
    for (i, k) in ["a", "b", "c"].iter().enumerate() {
        do_task(&e, &mid, &ids[i], k).await;
    }
    drift(&e, &mid, "a").await;
    scan(&e, &mid).await;
    assert_eq!(causes(&task(&e, &ids[0]).await), ["evidence_changed"]);
    assert_eq!(causes(&task(&e, &ids[1]).await), ["upstream_evidence_changed"]);
    assert!(causes(&task(&e, &ids[2]).await).is_empty(), "C is two steps down: not flagged");
    let g = person(&e, "GET", &format!("/api/missions/{mid}/graph"), None).await;
    let node = |tid: &str| g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == tid).unwrap().clone();
    assert_eq!(node(&ids[1])["downstream"], 1, "B shows C as one more downstream");
    assert!(node(&ids[2])["flags"].as_array().unwrap().is_empty());
}

// 6
#[tokio::test]
async fn a_dependent_that_has_not_started_is_not_flagged() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("d", &["a"])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    assert_eq!(state_of(&e, &ids[1]).await, "ready");
    drift(&e, &mid, "a").await;
    let r = scan(&e, &mid).await;
    assert_eq!(r["flagged"].as_array().unwrap().len(), 1, "only A: {r}");
    assert!(causes(&task(&e, &ids[1]).await).is_empty());
    assert_eq!(state_of(&e, &ids[1]).await, "ready");
}

// 7
#[tokio::test]
async fn resolving_still_valid_redo_and_cancelled() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &["a"])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    do_task(&e, &mid, &ids[1], "b").await;
    // B also has a check a teammate added and kwant approved on its own: that approval doesn't carry over
    e.state.db.add_check(&kwnt::db::missions::NewCheck {
        task_id: ids[1].clone(), kind: "file_contains".into(), spec: json!({"path": "b.txt", "text": "b"}),
        required: false, approved: true, requested_by: e.other.id.clone(), approved_by: Some("auto".into()),
    }).await.unwrap();
    drift(&e, &mid, "a").await;
    scan(&e, &mid).await;
    let fa = id(&flags(&task(&e, &ids[0]).await)[0]);
    let fb = id(&flags(&task(&e, &ids[1]).await)[0]);
    let resolve = |t: &str, f: &str| format!("/api/tasks/{t}/flags/{f}/resolve");

    // still_valid: the flag closes, the task is unchanged, and the same drift isn't raised again
    let before = task(&e, &ids[0]).await;
    let r = person(&e, "POST", &resolve(&ids[0], &fa), Some(json!({"resolution": "still_valid", "note": "same numbers"}))).await;
    assert_eq!(r["flag"]["resolution"], "still_valid");
    assert!(r["flag"]["resolved_at"].is_string());
    assert!(r["fork"].is_null());
    let after = task(&e, &ids[0]).await;
    assert_eq!(after["task"]["state"], before["task"]["state"]);
    assert_eq!(after["task"]["updated_at"], before["task"]["updated_at"]);
    assert!(flags(&after).is_empty());
    scan(&e, &mid).await;
    assert!(flags(&task(&e, &ids[0]).await).is_empty(), "the person said it still holds");
    let (status, v) = call(&e.app, "POST", &resolve(&ids[0], &fa), None, Some(json!({"resolution": "redo"}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");

    // cancelled on a verified task: refused
    let (status, v) = call(&e.app, "POST", &resolve(&ids[1], &fb), None, Some(json!({"resolution": "cancelled"}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("can't be cancelled"));
    let (status, _) = call(&e.app, "POST", &resolve(&ids[1], &fb), None, Some(json!({"resolution": "maybe"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&e.app, "POST", &resolve(&ids[0], &fb), None, Some(json!({"resolution": "redo"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a flag of another task");

    // redo on a verified task: a fork, the original untouched
    let r = person(&e, "POST", &resolve(&ids[1], &fb), Some(json!({"resolution": "redo"}))).await;
    assert_eq!(r["flag"]["resolution"], "redo");
    assert_eq!(r["task"]["state"], "verified");
    let fork = r["fork"].clone();
    let fid = id(&fork);
    assert_eq!(fork["forked_from"], ids[1].as_str());
    assert_eq!(fork["depends_on"], json!([ids[0]]));
    assert_eq!(fork["suggested_kwant_id"], e.bot.id.as_str());
    assert_eq!(fork["created_by"], "person");
    assert_eq!(fork["title"], "b step");
    assert!(fork["source_id"].is_null());
    assert_eq!(fork["state"], "ready", "A is verified, so the fork can start");
    let f = task(&e, &fid).await;
    let checks = f["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2, "{f}");
    let plan = checks.iter().find(|c| c["kind"] == "file_exists").unwrap();
    assert_eq!(plan["approved"], true);
    assert_eq!(plan["approved_by"], "person", "the person wrote this check with the plan");
    assert_eq!(plan["status"], "pending");
    assert!(plan["checked_by"].is_null() && plan["output_excerpt"].is_null());
    let auto = checks.iter().find(|c| c["kind"] == "file_contains").unwrap();
    assert_eq!(auto["approved"], false);
    assert!(auto["approved_by"].is_null());
    assert_eq!(auto["status"], "needs_approval");
    let original = task(&e, &ids[1]).await;
    assert_eq!(original["task"]["state"], "verified");
    assert!(original["checks"].as_array().unwrap().iter().all(|c| c["status"] == "passed" || c["kind"] == "file_contains"));
    let kinds: Vec<&str> = original["events"].as_array().unwrap().iter().map(|ev| ev["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"forked") && kinds.contains(&"flag_resolved"), "{kinds:?}");
    let created = f["events"].as_array().unwrap().iter().find(|ev| ev["kind"] == "created").unwrap().clone();
    assert_eq!(created["detail"]["forked_from"], ids[1].as_str(), "{created}");
}

// 8
#[tokio::test]
async fn bot_tokens_cannot_resolve_fork_or_scan() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    drift(&e, &mid, "a").await;
    scan(&e, &mid).await;
    let fid = id(&flags(&task(&e, &ids[0]).await)[0]);
    let token = turn(&e, &e.bot, "t-403").await;
    let a = &ids[0];
    for (uri, body) in [
        (format!("/api/tasks/{a}/flags/{fid}/resolve"), json!({"resolution": "still_valid"})),
        (format!("/api/tasks/{a}/fork"), json!({"reason": "redo"})),
        (format!("/api/missions/{mid}/graph/scan"), json!({})),
    ] {
        let (status, _) = call(&e.app, "POST", &uri, Some(&token), Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }
    assert_eq!(flags(&task(&e, a).await).len(), 1, "nothing changed");
    assert!(e.state.db.forks_of(a).await.unwrap().is_empty());
    let (status, _) = call(&e.app, "GET", &format!("/api/missions/{mid}/graph"), Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "reads stay open");
}

// 9
#[tokio::test]
async fn forking_a_verified_task_flags_its_started_dependents_only() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &["a"]), ("d", &["a"])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    do_task(&e, &mid, &ids[1], "b").await;
    assert_eq!(state_of(&e, &ids[2]).await, "ready");
    let uri = format!("/api/tasks/{}/fork", ids[0]);
    let fork = person(&e, "POST", &uri, Some(json!({"reason": "the source data was wrong"}))).await;
    assert_eq!(fork["forked_from"], ids[0].as_str());
    assert_eq!(state_of(&e, &ids[0]).await, "verified");
    let b = task(&e, &ids[1]).await;
    assert_eq!(causes(&b), ["upstream_forked"]);
    assert_eq!(flags(&b)[0]["detail"]["fork_id"], fork["id"]);
    assert_eq!(b["task"]["state"], "verified");
    assert!(causes(&task(&e, &ids[2]).await).is_empty(), "D is ready, not started");
    assert_eq!(task(&e, &ids[2]).await["task"]["depends_on"], json!([ids[0]]), "dependents aren't rewired");
    // A done mission opened again so the fork can run
    let m = person(&e, "GET", &format!("/api/missions/{mid}"), None).await;
    assert_eq!(m["mission"]["status"], "active", "{m}");
    let (status, v) = call(&e.app, "POST", &uri, None, Some(json!({"reason": " "}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn an_idempotency_key_forks_once_and_belongs_to_one_route() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    let send = |uri: String, key: &'static str| {
        let app = e.app.clone();
        async move {
            let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json").header("idempotency-key", key)
                .body(Body::from(json!({"reason": "redo"}).to_string())).unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
        }
    };
    let uri = format!("/api/tasks/{}/fork", ids[0]);
    let (s1, first) = send(uri.clone(), "fork-a-1").await;
    let (s2, second) = send(uri.clone(), "fork-a-1").await;
    assert!(s1.is_success() && s2.is_success(), "{first} {second}");
    assert_eq!(first["id"], second["id"]);
    assert_eq!(e.state.db.forks_of(&ids[0]).await.unwrap().len(), 1, "one fork for one key");
    let (s3, v) = send(format!("/api/tasks/{}/fork", ids[1]), "fork-a-1").await;
    assert_eq!(s3, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s4, v) = send(format!("/api/tasks/{}/retry", ids[1]), "fork-a-1").await;
    assert_eq!(s4, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s5, _) = send(uri, "fork-a-2").await;
    assert!(s5.is_success());
    assert_eq!(e.state.db.forks_of(&ids[0]).await.unwrap().len(), 2, "a new key is a new fork");
}

#[tokio::test]
async fn an_idempotency_key_resolves_a_redo_once_and_belongs_to_one_flag() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &["a"])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    do_task(&e, &mid, &ids[1], "b").await;
    drift(&e, &mid, "a").await;
    scan(&e, &mid).await;
    let fa = id(&flags(&task(&e, &ids[0]).await)[0]);
    let fb = id(&flags(&task(&e, &ids[1]).await)[0]);
    let resolve = |t: &str, f: &str| format!("/api/tasks/{t}/flags/{f}/resolve");
    let send = |uri: String, key: Option<&'static str>| {
        let app = e.app.clone();
        async move {
            let mut req = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
            if let Some(k) = key {
                req = req.header("idempotency-key", k);
            }
            let resp = app.oneshot(req.body(Body::from(json!({"resolution": "redo"}).to_string())).unwrap()).await.unwrap();
            let status = resp.status();
            let replayed = resp.headers().get("idempotent-replayed").is_some();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, replayed, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
        }
    };

    // Two Redo clicks (two tabs) with one key: one fork, the second reply is the first one replayed
    let (s1, r1, first) = send(resolve(&ids[0], &fa), Some("redo-a-1")).await;
    let (s2, r2, second) = send(resolve(&ids[0], &fa), Some("redo-a-1")).await;
    assert!(s1.is_success() && s2.is_success(), "{first} {second}");
    assert!(!r1 && r2, "only the second reply is replayed");
    assert!(first["fork"]["id"].is_string(), "{first}");
    assert_eq!(first["fork"]["id"], second["fork"]["id"]);
    assert_eq!(e.state.db.forks_of(&ids[0]).await.unwrap().len(), 1, "one fork for one key");

    // The same key on another flag is refused and changes nothing
    let (s3, _, v) = send(resolve(&ids[1], &fb), Some("redo-a-1")).await;
    assert_eq!(s3, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(flags(&task(&e, &ids[1]).await).iter().any(|f| id(f) == fb), "B's flag is still open");
    assert!(e.state.db.forks_of(&ids[1]).await.unwrap().is_empty());

    // Without a key nothing changes: the first resolve forks, a second one is 409
    let (s4, r4, v) = send(resolve(&ids[1], &fb), None).await;
    assert!(s4.is_success() && !r4, "{v}");
    let (s5, _, v) = send(resolve(&ids[1], &fb), None).await;
    assert_eq!(s5, StatusCode::CONFLICT, "{v}");
    assert_eq!(e.state.db.forks_of(&ids[1]).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_keyless_redos_make_one_fork_and_the_rest_are_409() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    drift(&e, &mid, "a").await;
    scan(&e, &mid).await;
    let fa = id(&flags(&task(&e, &ids[0]).await)[0]);
    let uri = format!("/api/tasks/{}/flags/{fa}/resolve", ids[0]);

    // Eight Redo requests at once, none with an Idempotency-Key
    let sends = (0..8).map(|_| {
        let (app, uri) = (e.app.clone(), uri.clone());
        tokio::spawn(async move {
            let req = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
            let resp = app.oneshot(req.body(Body::from(json!({"resolution": "redo"}).to_string())).unwrap()).await.unwrap();
            let status = resp.status();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
        })
    });
    let replies: Vec<(StatusCode, Value)> = futures::future::join_all(sends).await.into_iter().map(Result::unwrap).collect();
    let won: Vec<&Value> = replies.iter().filter(|(s, _)| s.is_success()).map(|(_, v)| v).collect();
    assert_eq!(won.len(), 1, "{replies:?}");
    assert!(replies.iter().all(|(s, _)| s.is_success() || *s == StatusCode::CONFLICT), "{replies:?}");
    let forks = e.state.db.forks_of(&ids[0]).await.unwrap();
    assert_eq!(forks.len(), 1, "a loser makes no copy: {replies:?}");
    assert_eq!(won[0]["fork"]["id"], forks[0].id.as_str());

    // One flag_resolved event, naming that fork; the original is still verified
    let events = e.state.db.task_events(&ids[0], 100).await.unwrap();
    let resolved: Vec<Value> = events.iter().filter(|ev| ev.kind == "flag_resolved").map(|ev| ev.detail_value()).collect();
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(resolved[0]["fork_id"], forks[0].id.as_str());
    assert_eq!(resolved[0]["resolution"], "redo");
    assert_eq!(events.iter().filter(|ev| ev.kind == "forked").count(), 1);
    assert_eq!(state_of(&e, &ids[0]).await, "verified");
}

// 10
#[tokio::test]
async fn a_scan_reads_nothing_outside_the_task_folder() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[])]).await;
    do_task(&e, &mid, &ids[0], "a").await;
    let outside = folder(&e, &mid).await.parent().unwrap().join("x");
    std::fs::write(&outside, "secret-ish").unwrap();
    for path in ["../x", ".env"] {
        e.state.db.add_evidence(&kwnt::db::missions::NewEvidence {
            task_id: ids[0].clone(), kind: "file".into(), label: path.into(), path: Some(path.into()),
            sha256: Some("0".repeat(64)), captured_by: e.bot.id.clone(), ..Default::default()
        }).await.unwrap();
    }
    std::fs::write(folder(&e, &mid).await.join(".env"), "TOKEN=1").unwrap();
    let r = scan(&e, &mid).await;
    assert_eq!(r["scanned"], 1, "only a.txt is hashed: {r}");
    assert_eq!(r["flagged"], json!([]), "a skipped file is never 'changed': {r}");
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 2, "{r}");
    let why = |p: &str| skipped.iter().find(|s| s["path"] == p).unwrap()["why"].as_str().unwrap().to_string();
    assert!(why("../x").contains(".."), "{r}");
    assert!(why(".env").contains("env"), "{r}");
    let g = person(&e, "GET", &format!("/api/missions/{mid}/graph"), None).await;
    assert_eq!(g["scan_skipped"].as_array().unwrap().len(), 2, "{g}");
    assert!(g["scanned_at"].is_string());
}

// 11
#[tokio::test]
async fn the_graph_edges_match_the_dependencies_and_shows_open_flags_only() {
    let e = setup().await;
    let (mid, ids) = graph_mission(&e, &[("a", &[]), ("b", &["a"]), ("c", &["a", "b"])]).await;
    let g = person(&e, "GET", &format!("/api/missions/{mid}/graph"), None).await;
    assert_eq!(g["mission_id"], mid.as_str());
    assert!(g["scanned_at"].is_null());
    assert_eq!(g["nodes"].as_array().unwrap().len(), 3);
    let mut edges: Vec<(String, String)> = g["edges"].as_array().unwrap().iter()
        .map(|x| (x["from"].as_str().unwrap().to_string(), x["to"].as_str().unwrap().to_string())).collect();
    edges.sort();
    let mut want = vec![(ids[0].clone(), ids[1].clone()), (ids[0].clone(), ids[2].clone()), (ids[1].clone(), ids[2].clone())];
    want.sort();
    assert_eq!(edges, want);
    let node = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == ids[0].as_str()).unwrap().clone();
    for k in ["title", "state", "owner_kwant_id", "owner_name", "attempts", "max_attempts", "forked_from", "flags", "checks_summary", "downstream"] {
        assert!(node.get(k).is_some(), "node lacks {k}: {node}");
    }
    assert_eq!(node["downstream"], 2);

    do_task(&e, &mid, &ids[0], "a").await;
    drift(&e, &mid, "a").await;
    scan(&e, &mid).await;
    let fid = id(&flags(&task(&e, &ids[0]).await)[0]);
    let flags_of = |g: &Value| g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == ids[0].as_str()).unwrap()["flags"].as_array().unwrap().len();
    let g = person(&e, "GET", &format!("/api/missions/{mid}/graph"), None).await;
    assert_eq!(flags_of(&g), 1);
    person(&e, "POST", &format!("/api/tasks/{}/flags/{fid}/resolve", ids[0]), Some(json!({"resolution": "still_valid"}))).await;
    let g = person(&e, "GET", &format!("/api/missions/{mid}/graph"), None).await;
    assert_eq!(flags_of(&g), 0, "resolved flags leave the graph");
    let open = person(&e, "GET", &format!("/api/tasks/{}/flags", ids[0]), None).await;
    assert!(open["flags"].as_array().unwrap().is_empty());
    let all = person(&e, "GET", &format!("/api/tasks/{}/flags?all=1", ids[0]), None).await;
    assert_eq!(all["flags"].as_array().unwrap().len(), 1);
    assert_eq!(all["flags"][0]["resolution"], "still_valid");

    // Tasks outside a mission have a graph too
    let loose = person(&e, "POST", "/api/tasks", Some(json!({"title": "Loose end"}))).await;
    let lg = person(&e, "GET", "/api/graph", None).await;
    assert!(lg["mission_id"].is_null());
    let lid = loose["id"].as_str().or(loose["task"]["id"].as_str()).unwrap();
    assert!(lg["nodes"].as_array().unwrap().iter().any(|n| n["id"] == lid), "{lg}");
    assert!(lg["nodes"].as_array().unwrap().iter().all(|n| n["id"] != ids[0].as_str()), "mission tasks stay in their mission's graph");
}
