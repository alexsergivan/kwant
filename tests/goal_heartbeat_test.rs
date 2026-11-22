//! 33z goal-heartbeat-fills-in (Mira's UX pass 0.9.40, fix #4). The goal heartbeat draws one square per
//! `checked` event, but the runner only logged status changes, so it never filled in and "Steady" (and the
//! row's "First goal kept" sticker) could never show. Now every run logs one `checked` event with what it
//! saw (verdict, required checks passed, required checks in all). Checks are kept to the newest 50 per goal,
//! apart from the 500 kept for everything else, so hourly checks never push a goal's story out; and the
//! readers of that story (the detail page, the fix flow) see it whole, with the page adding the newest checks
//! for the heartbeat.
//!
//! The events are read straight from the database file, so these tests compile against main too (red proof).

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        goals::{Created, Edit, Goal, NewGoal, NewGoalCheck},
        missions::now_ms,
        models::Kwant,
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{create_router, goals::runner, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    space: String,
    raw: sqlx::SqlitePool,
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
    let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
    let raw = sqlx::SqlitePool::connect(&format!("sqlite://{}", tmp.path().join("kwnt.db").display())).await.unwrap();
    Env { app: create_router(state.clone()), state, bot, space: space.as_str().to_string(), raw, _tmp: tmp }
}

/// A one-route server answering with the statuses in `codes`, in turn (the last repeats).
async fn server(codes: Vec<u16>) -> String {
    use axum::routing::get;
    let n = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/",
        get(move || {
            let n = n.clone();
            let codes = codes.clone();
            async move { StatusCode::from_u16(codes[n.fetch_add(1, Ordering::SeqCst).min(codes.len() - 1)]).unwrap() }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/")
}

fn closed_port() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://127.0.0.1:{}/", l.local_addr().unwrap().port())
}

fn http_check(url: &str, severity: &str) -> NewGoalCheck {
    NewGoalCheck {
        label: "up".into(),
        kind: "http".into(),
        spec: json!({ "url": url }),
        metric: Some("status".into()),
        operator: Some("eq".into()),
        target: Some("200".into()),
        severity: severity.into(),
        freshness_minutes: 120,
    }
}

fn new_goal(e: &Env, checks: Vec<NewGoalCheck>, confirm: i64) -> NewGoal {
    NewGoal {
        space_id: e.space.clone(),
        title: "Site is up".into(),
        description: String::new(),
        kwant_id: e.bot.id.clone(),
        folder: String::new(),
        mode: "suggest".into(),
        interval_minutes: 60,
        confirm_failures: confirm,
        budget_tokens: 0,
        max_fixes_per_day: 3,
        priority: 2,
        created_by: "person".into(),
        draft: false,
        checks,
    }
}

async fn create(e: &Env, g: &NewGoal) -> Goal {
    match e.state.db.create_goal(g, now_ms()).await.unwrap() {
        Created::Goal(g) => *g,
        Created::TooManyDrafts => panic!("too many drafts"),
    }
}

async fn run_now(e: &Env, id: &str) -> Goal {
    let g = e.state.db.lease_goal(id, now_ms(), false).await.unwrap().expect("leased");
    runner::run_goal(&e.state, g).await.expect("ran")
}

/// Points the goal at one required check of `url`.
async fn point(e: &Env, id: &str, url: &str) {
    let url = url.to_string();
    e.state
        .db
        .edit_goal(id, &e.space, "person", now_ms(), move |_g: &mut Goal| Ok(Edit { checks: Some(vec![http_check(&url, "required")]), events: vec![] }))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

/// Every event the goal has in the database, oldest first: (id, kind, detail).
async fn rows(e: &Env, id: &str) -> Vec<(i64, String, Value)> {
    let r: Vec<(i64, String, String)> = sqlx::query_as("SELECT id, kind, detail FROM goal_events WHERE goal_id = ? ORDER BY id").bind(id).fetch_all(&e.raw).await.unwrap();
    r.into_iter().map(|(i, k, d)| (i, k, serde_json::from_str(&d).unwrap_or(Value::Null))).collect()
}

/// What a run added: the events newer than `before`.
async fn added(e: &Env, id: &str, before: &[(i64, String, Value)]) -> Vec<(String, Value)> {
    let last = before.last().map(|r| r.0).unwrap_or(0);
    rows(e, id).await.into_iter().filter(|r| r.0 > last).map(|r| (r.1, r.2)).collect()
}

fn checked(added: &[(String, Value)]) -> Vec<Value> {
    added.iter().filter(|(k, _)| k == "checked").map(|(_, d)| d.clone()).collect()
}

async fn call(app: &Router, uri: &str) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

// A run that changes nothing still logs one `checked` with what it saw; only required checks count.
#[tokio::test]
async fn every_run_logs_one_checked_event_with_what_it_saw() {
    let e = setup().await;
    let up = server(vec![200]).await;
    let down = server(vec![500]).await;
    let g = create(&e, &new_goal(&e, vec![http_check(&up, "required"), http_check(&down, "advisory")], 2)).await;

    let before = rows(&e, &g.id).await;
    assert_eq!(run_now(&e, &g.id).await.status, "met");
    let first = added(&e, &g.id, &before).await;
    assert_eq!(first.iter().filter(|(k, _)| k == "met").count(), 1, "the status change is still logged: {first:?}");
    assert_eq!(checked(&first), [json!({ "verdict": "met", "passed": 1, "total": 1 })], "plus one check, the advisory one not counted");

    let before = rows(&e, &g.id).await;
    assert_eq!(run_now(&e, &g.id).await.status, "met");
    let second = added(&e, &g.id, &before).await;
    assert_eq!(second, [("checked".to_string(), json!({ "verdict": "met", "passed": 1, "total": 1 }))], "an unchanged status logs exactly one check");

    // A held blip (met stays met) logs no status event, but its check says it couldn't tell
    point(&e, &g.id, &closed_port()).await;
    let before = rows(&e, &g.id).await;
    let blip = run_now(&e, &g.id).await;
    assert_eq!(blip.status, "met", "one blip is held");
    assert_eq!(added(&e, &g.id, &before).await, [("checked".to_string(), json!({ "verdict": "unknown", "passed": 0, "total": 1 }))]);
}

// A run that changes the status logs the change and one `checked`; a failure not yet confirmed logs what it saw.
#[tokio::test]
async fn a_status_change_logs_its_event_and_one_check() {
    let e = setup().await;
    let url = server(vec![200, 500, 500, 200]).await;
    let g = create(&e, &new_goal(&e, vec![http_check(&url, "required")], 2)).await;
    run_now(&e, &g.id).await;

    let before = rows(&e, &g.id).await;
    assert_eq!(run_now(&e, &g.id).await.status, "met", "one failure is not news yet");
    let unconfirmed = added(&e, &g.id, &before).await;
    assert_eq!(unconfirmed, [("checked".to_string(), json!({ "verdict": "not_met", "passed": 0, "total": 1 }))]);

    let before = rows(&e, &g.id).await;
    assert_eq!(run_now(&e, &g.id).await.status, "not_met");
    let broke = added(&e, &g.id, &before).await;
    assert_eq!(broke.iter().filter(|(k, _)| k == "not_met").count(), 1, "{broke:?}");
    assert_eq!(checked(&broke), [json!({ "verdict": "not_met", "passed": 0, "total": 1 })]);
    let at = |kind: &str| broke.iter().position(|(k, _)| k == kind).unwrap();
    assert!(at("not_met") < at("checked"), "the change first, then the check that saw it");

    let before = rows(&e, &g.id).await;
    assert_eq!(run_now(&e, &g.id).await.status, "met");
    let back = added(&e, &g.id, &before).await;
    assert_eq!(back.iter().filter(|(k, _)| k == "met").count(), 1, "{back:?}");
    assert_eq!(checked(&back), [json!({ "verdict": "met", "passed": 1, "total": 1 })]);
}

// 60 runs: the newest 50 checks stay, and no other event is pruned to make room for them. The fix flow's
// readers and the detail page still see the story; the page adds the newest checks for the heartbeat.
#[tokio::test]
async fn sixty_runs_keep_fifty_checks_and_every_other_event() {
    let e = setup().await;
    let mut codes = vec![200; 20];
    codes.extend([500; 20]);
    codes.extend([200; 20]);
    let url = server(codes).await;
    let g = create(&e, &new_goal(&e, vec![http_check(&url, "required")], 2)).await;
    let mut story: Vec<(i64, String)> = Vec::new();
    for _ in 0..60 {
        run_now(&e, &g.id).await;
        for (id, kind, _) in rows(&e, &g.id).await {
            if kind != "checked" && !story.iter().any(|s| s.0 == id) {
                story.push((id, kind));
            }
        }
    }
    let all = rows(&e, &g.id).await;
    let checks: Vec<&(i64, String, Value)> = all.iter().filter(|r| r.1 == "checked").collect();
    assert_eq!(checks.len(), 50, "the newest 50 checks are kept");
    // The newest 50: the last 20 runs met, the 20 before them failed, and 10 of the first 20 runs remain
    let verdicts: Vec<&str> = checks.iter().map(|r| r.2["verdict"].as_str().unwrap()).collect();
    assert_eq!(verdicts.iter().filter(|v| **v == "not_met").count(), 20);
    assert_eq!(verdicts[..10], ["met"; 10]);
    assert_eq!(verdicts[40..], ["met"; 10]);
    let kept: Vec<(i64, String)> = all.iter().filter(|r| r.1 != "checked").map(|r| (r.0, r.1.clone())).collect();
    assert_eq!(kept, story, "every other event stays");
    for kind in ["created", "met", "not_met"] {
        assert!(kept.iter().any(|k| k.1 == kind), "{kind} kept: {kept:?}");
    }

    // The fix flow reads the newest 30 or 40 events looking for its own; checks don't crowd them out
    let window = e.state.db.goal_events(&g.id, 30).await.unwrap();
    assert!(window.iter().all(|ev| ev.kind != "checked"), "the story window holds only the story");
    assert!(window.iter().any(|ev| ev.kind == "created"));

    // The detail page: the whole story, plus the newest 7 checks (as many as the heartbeat draws)
    let (status, v) = call(&e.app, &format!("/api/goals/{}", g.id)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let events = v["events"].as_array().unwrap();
    let page_checks: Vec<i64> = events.iter().filter(|ev| ev["kind"] == "checked").map(|ev| ev["id"].as_i64().unwrap()).collect();
    let newest7: Vec<i64> = checks.iter().rev().take(7).map(|r| r.0).collect();
    assert_eq!(page_checks, newest7, "newest first, like the rest of the page");
    assert!(events.iter().any(|ev| ev["kind"] == "created"), "the story is still there");
    let ids: Vec<i64> = events.iter().map(|ev| ev["id"].as_i64().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] > w[1]), "one list, newest first: {ids:?}");
}

// The list row draws the heartbeat too (Rigel's QA): a healthy goal's row never reads the detail, so the list
// carries each goal's newest heartbeat events (checks, fixes opened, replans), newest first, at most 7.
#[tokio::test]
async fn the_list_carries_each_goals_newest_heartbeat_events() {
    let e = setup().await;
    let url = server(vec![200]).await;
    let g = create(&e, &new_goal(&e, vec![http_check(&url, "required")], 2)).await;
    for _ in 0..3 {
        assert_eq!(run_now(&e, &g.id).await.status, "met");
    }
    let (status, v) = call(&e.app, "/api/goals").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let row = v["goals"].as_array().unwrap().iter().find(|r| r["id"] == g.id.as_str()).expect("the goal's row").clone();
    let beats = row["heartbeat"].as_array().expect("the row carries its heartbeat");
    assert_eq!(beats.len(), 3, "{beats:?}");
    assert!(beats.iter().all(|b| b["kind"] == "checked" && b["detail"]["verdict"] == "met"), "{beats:?}");

    // A fix opened is a square in its place; the story (created, met) isn't
    e.state.db.add_goal_event(&g.id, "fix_opened", "kwant", &json!({ "task_id": "t1" })).await.unwrap();
    for _ in 0..2 {
        run_now(&e, &g.id).await;
    }
    let (_, v) = call(&e.app, "/api/goals").await;
    let row = v["goals"].as_array().unwrap().iter().find(|r| r["id"] == g.id.as_str()).unwrap().clone();
    let kinds: Vec<&str> = row["heartbeat"].as_array().unwrap().iter().map(|b| b["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["checked", "checked", "fix_opened", "checked", "checked", "checked"]);

    // 5 more runs: only the newest 7 squares come along
    for _ in 0..5 {
        run_now(&e, &g.id).await;
    }
    let (_, v) = call(&e.app, "/api/goals").await;
    let row = v["goals"].as_array().unwrap().iter().find(|r| r["id"] == g.id.as_str()).unwrap().clone();
    let beats = row["heartbeat"].as_array().unwrap();
    assert_eq!(beats.len(), 7, "as many squares as the heartbeat draws");
    assert!(beats.iter().all(|b| b["kind"] == "checked"), "the fix is now older than the newest 7: {beats:?}");
    let ids: Vec<i64> = beats.iter().map(|b| b["id"].as_i64().unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] > w[1]), "newest first: {ids:?}");
    let newest: Vec<i64> = rows(&e, &g.id).await.iter().rev().filter(|r| r.1 == "checked").take(7).map(|r| r.0).collect();
    assert_eq!(ids, newest);
}
