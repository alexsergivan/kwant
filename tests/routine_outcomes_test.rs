//! Mission Control Phase D4, build step 1: a routine can carry what each run's board task gets,
//! a Flight School class, a description of a good result and checks the person approved. The
//! columns are add-only (a routine without them runs exactly as before), and the class and checks
//! are the person's: a bot's token, or a bot's own routine tools, can't set them.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{continuity::NewRoutine, missions::Task, models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::{self, listener},
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
    bot: Kwant,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let space = SpaceId::default_space();
    let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app, state, bot, general, _tmp: tmp };
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    e
}

async fn send(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
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
    let (status, v) = send(&e.app, method, uri, None, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

/// A turn of the bot in #general with its own tools; returns the turn token.
async fn turn(e: &Env, turn_id: &str) -> String {
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = send(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
}

fn routine_body(e: &Env, extra: Value) -> Value {
    let mut body = json!({
        "kwant_id": e.bot.id,
        "channel_id": e.general,
        "name": "Weekly sales report",
        "instructions": "Write this week's sales report to report.md",
        "schedule": {"type": "weekly", "time": "09:00", "weekdays": ["mon"], "tz": "UTC"},
        "enabled": false,
    });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

async fn routine(e: &Env, id: &str) -> Value {
    let list = person(e, "GET", "/api/routines", None).await;
    list.as_array().unwrap().iter().find(|r| r["id"] == id).cloned().expect("the routine is listed")
}

/// One run of `rid`: its board task, as it is while the run works.
async fn start_run(e: &Env, rid: &str) -> (String, Task) {
    let run = e.state.db.start_routine_run(rid, "schedule", "running", None).await.unwrap();
    listener::on_routine_run(&e.state, rid, &run.id, "running").await;
    let task = e.state.db.task_by_source("routine", &run.id).await.unwrap().expect("each run has a task");
    (run.id, task)
}

async fn finish_run(e: &Env, rid: &str, run_id: &str) -> Task {
    e.state.db.finish_routine_run(run_id, "ok", None, None).await.unwrap();
    listener::on_routine_run(&e.state, rid, run_id, "ok").await;
    e.state.db.task_by_source("routine", run_id).await.unwrap().unwrap()
}

#[tokio::test]
async fn the_migration_runs_twice_on_an_old_database_and_keeps_its_routines() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let bot = db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let old = db
        .create_routine(&NewRoutine {
            kwant_id: bot.id.clone(),
            name: "Old routine".into(),
            instructions: "Do the old thing".into(),
            schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
            enabled: true,
            ..Default::default()
        })
        .await
        .unwrap();
    drop(db);
    // Back to a database from before this migration: no new columns, no new tables
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
    for stmt in [
        "DROP TABLE routine_suggestion_events",
        "DROP TABLE routine_suggestions",
        "ALTER TABLE routines DROP COLUMN origin",
        "ALTER TABLE routines DROP COLUMN suggestion_id",
        "ALTER TABLE routines DROP COLUMN task_class",
        "ALTER TABLE routines DROP COLUMN acceptance",
        "ALTER TABLE routines DROP COLUMN checks",
    ] {
        sqlx::query(stmt).execute(&pool).await.unwrap();
    }
    pool.close().await;

    for _ in 0..2 {
        let db = Database::init(&data).await.unwrap();
        let r = db.get_routine(&old.id).await.unwrap().expect("the old routine survives");
        assert_eq!((r.name.as_str(), r.instructions.as_str(), r.enabled), ("Old routine", "Do the old thing", true));
        assert_eq!(r.schedule, old.schedule);
        assert_eq!(r.next_run_at, old.next_run_at);
        assert!(r.origin.is_none() && r.suggestion_id.is_none() && r.task_class.is_none() && r.acceptance.is_none() && r.checks.is_none(), "{r:?}");
    }

    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
    let cols: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('routines')").fetch_all(&pool).await.unwrap();
    for c in ["origin", "suggestion_id", "task_class", "acceptance", "checks"] {
        assert_eq!(cols.iter().filter(|(n,)| n == c).count(), 1, "{c} is added once");
    }
    // One live offer per habit, and the history is append-only
    let offer = "INSERT INTO routine_suggestions (id, space_id, cluster_key, kwant_id, name, instructions, schedule, first_run_at, delivery,
                 source_task_ids, evidence, status, offered_at) VALUES (?, 'default', 'k1', 'b', 'n', 'i', '{}', '2026-11-02T09:00:00Z', '{}', '[]', '{}', 'offered', '2026-10-31T00:00:00Z')";
    sqlx::query(offer).bind("s1").execute(&pool).await.unwrap();
    assert!(sqlx::query(offer).bind("s2").execute(&pool).await.is_err(), "a second live offer for the same habit");
    sqlx::query("INSERT INTO routine_suggestion_events (suggestion_id, kind, actor, created_at) VALUES ('s1', 'offered', 'kwant', '2026-10-31T00:00:00Z')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(sqlx::query("UPDATE routine_suggestion_events SET actor = 'x'").execute(&pool).await.is_err());
    assert!(sqlx::query("DELETE FROM routine_suggestion_events").execute(&pool).await.is_err());
}

#[tokio::test]
async fn a_routine_without_outcomes_runs_exactly_as_before() {
    let e = setup().await;
    let r = person(&e, "POST", "/api/routines", Some(routine_body(&e, json!({})))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    assert!(r["task_class"].is_null() && r["acceptance"].is_null() && r["origin"].is_null(), "{r}");
    assert_eq!(r["checks"], json!([]));

    let (run_id, task) = start_run(&e, &rid).await;
    assert_eq!(task.acceptance, "Write this week's sales report to report.md", "the instructions, as today");
    assert_eq!(e.state.db.task_class(&task.id).await.unwrap(), "general");
    assert!(e.state.db.list_checks(&task.id).await.unwrap().is_empty());
    let done = finish_run(&e, &rid, &run_id).await;
    assert_eq!(done.state, "review", "nothing to check: the person reviews it");
}

#[tokio::test]
async fn a_run_task_gets_the_class_acceptance_and_approved_checks_and_can_be_verified() {
    let e = setup().await;
    let body = routine_body(
        &e,
        json!({
            "task_class": "reporting",
            "acceptance": "report.md exists and names this week's top seller",
            "checks": [{"kind": "file_exists", "spec": {"path": "report.md"}}],
        }),
    );
    let r = person(&e, "POST", "/api/routines", Some(body)).await;
    let rid = r["id"].as_str().unwrap().to_string();
    assert_eq!(r["task_class"], "reporting");
    assert_eq!(r["checks"], json!([{"kind": "file_exists", "spec": {"path": "report.md"}, "required": true}]));
    assert_eq!(routine(&e, &rid).await["acceptance"], "report.md exists and names this week's top seller");

    let (run_id, task) = start_run(&e, &rid).await;
    assert_eq!(task.acceptance, "report.md exists and names this week's top seller");
    assert_eq!(e.state.db.task_class(&task.id).await.unwrap(), "reporting");
    let checks = e.state.db.list_checks(&task.id).await.unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].kind, "file_exists");
    assert!(checks[0].approved && checks[0].required);
    assert_eq!(checks[0].approved_by.as_deref(), Some("person"));

    // The run writes its report, then finishes: the check passes and the task is verified
    let folder = missions::task_folder(&e.state, &task).await.expect("the run's task has a folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("report.md"), "Top seller: Dune").unwrap();
    let done = finish_run(&e, &rid, &run_id).await;
    assert_eq!(done.state, "verified", "{done:?}");

    // Every run gets its own copy: a run without the report doesn't pass
    let (run_id, task) = start_run(&e, &rid).await;
    assert_eq!(e.state.db.list_checks(&task.id).await.unwrap().len(), 1);
    std::fs::remove_file(folder.join("report.md")).unwrap();
    let done = finish_run(&e, &rid, &run_id).await;
    assert_ne!(done.state, "verified");
}

#[tokio::test]
async fn class_and_checks_are_the_persons_a_bot_gets_403() {
    let e = setup().await;
    let token = turn(&e, "t-bot").await;
    let r = person(&e, "POST", "/api/routines", Some(routine_body(&e, json!({"checks": [{"kind": "file_exists", "spec": {"path": "a.md"}}]})))).await;
    let rid = r["id"].as_str().unwrap().to_string();
    let patch = format!("/api/routines/{rid}");

    // Over HTTP with the bot's turn token
    for body in [
        json!({"checks": [{"kind": "command", "spec": {"command": "true"}}]}),
        json!({"checks": null}),
        json!({"task_class": "reporting"}),
    ] {
        let (status, v) = send(&e.app, "PATCH", &patch, Some(&token), Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}: {v}");
        let (status, v) = send(&e.app, "POST", "/api/routines", Some(&token), Some(routine_body(&e, body.clone()))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}: {v}");
    }
    let (status, v) = send(&e.app, "PATCH", &patch, Some("not-a-token"), Some(json!({"task_class": "reporting"}))).await;
    assert!(matches!(status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED), "{status} {v}");
    let after = routine(&e, &rid).await;
    assert_eq!(after["checks"], r["checks"], "nothing changed");
    assert!(after["task_class"].is_null());

    // A bot may still edit the rest, and the acceptance, and the checks stay as the person set them
    let (status, v) = send(&e.app, "PATCH", &patch, Some(&token), Some(json!({"name": "Renamed", "acceptance": "a.md exists"}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let after = routine(&e, &rid).await;
    assert_eq!((after["name"].as_str(), after["acceptance"].as_str()), (Some("Renamed"), Some("a.md exists")));
    assert_eq!(after["checks"], r["checks"]);

    // Through the bot's own routine tools too: an update keeps them
    let (text, err) = tool(&e, &token, "routine_update", json!({"routine": rid, "instructions": "Write a.md"})).await;
    assert!(!err, "{text}");
    assert_eq!(routine(&e, &rid).await["checks"], r["checks"]);
}

#[tokio::test]
async fn bad_outcomes_are_refused() {
    let e = setup().await;
    let many: Vec<Value> = (0..11).map(|i| json!({"kind": "file_exists", "spec": {"path": format!("f{i}.md")}})).collect();
    for (extra, needle) in [
        (json!({"task_class": "Not A Class"}), "class"),
        (json!({"acceptance": "x".repeat(4001)}), "acceptance"),
        (json!({"checks": many}), "checks"),
        (json!({"checks": [{"kind": "person"}]}), "person"),
        (json!({"checks": [{"kind": "command", "spec": {"command": "curl -H 'Authorization: [redacted]' x"}}]}), "redacted"),
    ] {
        let (status, v) = send(&e.app, "POST", "/api/routines", None, Some(routine_body(&e, extra.clone()))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{extra}: {v}");
        assert!(v.to_string().to_lowercase().contains(needle), "{needle}: {v}");
    }
    assert!(person(&e, "GET", "/api/routines", None).await.as_array().unwrap().is_empty());

    // "general" and blanks clear; an empty list clears the checks
    let r = person(&e, "POST", "/api/routines", Some(routine_body(&e, json!({"task_class": "general", "acceptance": "  ", "checks": []})))).await;
    assert!(r["task_class"].is_null() && r["acceptance"].is_null(), "{r}");
    assert_eq!(r["checks"], json!([]));
}

#[tokio::test]
async fn secrets_in_check_specs_are_masked_when_shown() {
    let e = setup().await;
    let cmd = "curl -fsS -H 'Authorization: Bearer sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789' https://example.com/health";
    let r = person(&e, "POST", "/api/routines", Some(routine_body(&e, json!({"checks": [{"kind": "command", "spec": {"command": cmd}}]})))).await;
    let shown = routine(&e, r["id"].as_str().unwrap()).await["checks"].to_string();
    assert!(!shown.contains("sk-ant-api03-abcdefghij"), "{shown}");
    assert!(shown.contains("[redacted]"), "{shown}");
    // The stored spec is the real one: the run's task gets it
    let (_, task) = start_run(&e, r["id"].as_str().unwrap()).await;
    assert!(e.state.db.list_checks(&task.id).await.unwrap()[0].spec.contains("sk-ant-api03-abcdefghij"));
}

#[tokio::test]
async fn undoing_a_bots_delete_brings_the_outcomes_back() {
    let e = setup().await;
    let token = turn(&e, "t-del").await;
    let body = routine_body(&e, json!({"task_class": "reporting", "acceptance": "a.md exists", "checks": [{"kind": "file_exists", "spec": {"path": "a.md"}}]}));
    let r = person(&e, "POST", "/api/routines", Some(body)).await;
    let (text, err) = tool(&e, &token, "routine_delete", json!({"routine": r["id"]})).await;
    assert!(!err, "{text}");
    let actions = person(&e, "GET", "/api/bot-actions", None).await;
    let del = actions.as_array().unwrap().iter().find(|a| a["action"] == "routine_delete").unwrap().clone();
    let v = person(&e, "POST", &format!("/api/bot-actions/{}/undo", del["id"].as_str().unwrap()), None).await;
    assert_eq!(v["status"], "undone", "{v}");
    let back = person(&e, "GET", "/api/routines", None).await[0].clone();
    assert_eq!(back["task_class"], "reporting");
    assert_eq!(back["acceptance"], "a.md exists");
    assert_eq!(back["checks"], r["checks"]);
}
