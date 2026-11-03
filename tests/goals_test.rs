//! Goals (plan §B.1, B.0): the loop with its lease, `/api/goals`, and `goal_propose`.
//! qa/test-plan.md 5.17 (G-T) and the loop's B.0 rules in 5.20 (G-Q).

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
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router,
        goals::{self, runner},
        mcp_runtime, AppState,
    },
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
    general: String,
    space: String,
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
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    Env { app: create_router(state.clone()), state, bot, general, space: space.as_str().to_string(), _tmp: tmp }
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

fn new_goal(e: &Env, url: &str, confirm: i64) -> NewGoal {
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
        checks: vec![NewGoalCheck {
            label: "up".into(),
            kind: "http".into(),
            spec: json!({ "url": url }),
            metric: Some("status".into()),
            operator: Some("eq".into()),
            target: Some("200".into()),
            severity: "required".into(),
            freshness_minutes: 120,
        }],
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

async fn kinds(e: &Env, id: &str) -> Vec<String> {
    e.state.db.goal_events(id, 100).await.unwrap().into_iter().map(|e| e.kind).collect()
}

async fn notices(e: &Env, id: &str) -> Vec<String> {
    let mut out: Vec<String> = e
        .state
        .db
        .goal_events(id, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "notified")
        .map(|ev| ev.detail_value()["kind"].as_str().unwrap_or("").to_string())
        .collect();
    out.reverse();
    out
}

// G-T-04, G-Q-01: one failure is never news; the confirm count is; one notice per incident
#[tokio::test]
async fn a_goal_breaks_only_after_its_confirm_count_and_tells_once() {
    let e = setup().await;
    let url = server(vec![200, 500, 500, 500, 200]).await;
    let g = create(&e, &new_goal(&e, &url, 2)).await;
    let mut seq = Vec::new();
    for _ in 0..5 {
        seq.push(run_now(&e, &g.id).await.status);
    }
    assert_eq!(seq, ["met", "met", "not_met", "not_met", "met"]);
    assert_eq!(kinds(&e, &g.id).await.iter().filter(|k| *k == "not_met").count(), 1);
    assert_eq!(notices(&e, &g.id).await, ["broken", "recovered"]);
    let after = e.state.db.get_goal(&g.id).await.unwrap().unwrap();
    assert!(after.checking_since.is_none());
    assert!(after.next_check_at.unwrap() >= now_ms() + 59 * 60_000);
    // The observation keeps what was seen and when it goes out of date
    let obs = e.state.db.recent_observations(&g.id, 1).await.unwrap();
    assert_eq!((obs[0].verdict.as_str(), obs[0].observed_value.as_deref()), ("met", Some("200")));
    assert!(obs[0].expires_at > now_ms() + 119 * 60_000);
}

// G-T-05, G-Q-02: an error is unknown, breaks a run of failures, and never notifies
#[tokio::test]
async fn an_error_between_failures_is_unknown_and_never_confirms() {
    let e = setup().await;
    let flaky = server(vec![200, 500]).await;
    let g = create(&e, &new_goal(&e, &flaky, 2)).await;
    assert_eq!(run_now(&e, &g.id).await.status, "met");
    assert_eq!(run_now(&e, &g.id).await.fail_streak, 1);
    // The same goal now points at a closed port for one run
    let down = closed_port();
    let swap = |url: String| {
        move |g: &mut Goal| {
            let _ = g;
            Ok(Edit {
                checks: Some(vec![NewGoalCheck {
                    label: "up".into(),
                    kind: "http".into(),
                    spec: json!({ "url": url }),
                    metric: Some("status".into()),
                    operator: Some("eq".into()),
                    target: Some("200".into()),
                    severity: "required".into(),
                    freshness_minutes: 120,
                }]),
                events: vec![],
            })
        }
    };
    let edited = e.state.db.edit_goal(&g.id, &e.space, "person", now_ms(), swap(down)).await.unwrap().unwrap().unwrap();
    // The edit doesn't touch the streak: only runs do
    assert_eq!(edited.fail_streak, 1);
    let after = run_now(&e, &g.id).await;
    assert_eq!((after.status.as_str(), after.fail_streak, after.error_runs), ("unknown", 0, 1));
    assert!(after.unknown_since.is_some());
    e.state.db.edit_goal(&g.id, &e.space, "person", now_ms(), swap(flaky)).await.unwrap().unwrap().unwrap();
    let after = run_now(&e, &g.id).await;
    assert_eq!((after.status.as_str(), after.fail_streak), ("unknown", 1), "fail, error, fail is not two failures in a row");
    assert!(notices(&e, &g.id).await.is_empty());
}

// G-T-09: one run at a time; an edit during a run wins; a deleted goal's run writes nothing
#[tokio::test]
async fn the_lease_keeps_runs_apart_and_edits_win() {
    let e = setup().await;
    let url = server(vec![200]).await;
    let g = create(&e, &new_goal(&e, &url, 2)).await;
    let leased = e.state.db.lease_goal(&g.id, now_ms(), false).await.unwrap().unwrap();
    assert!(e.state.db.lease_goal(&g.id, now_ms(), false).await.unwrap().is_none());
    let (status, _) = call(&e.app, "POST", &format!("/api/goals/{}/check", g.id), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    e.state
        .db
        .edit_goal(&g.id, &e.space, "person", now_ms(), |g| {
            g.title = "Renamed".into();
            Ok(Edit { checks: None, events: vec![] })
        })
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let after = runner::run_goal(&e.state, leased).await.unwrap();
    assert_eq!((after.title.as_str(), after.status.as_str()), ("Renamed", "checking"));
    assert!(after.checking_since.is_none());
    let leased = e.state.db.lease_goal(&g.id, now_ms(), false).await.unwrap().unwrap();
    e.state.db.delete_goal(&g.id, &e.space).await.unwrap();
    assert!(runner::run_goal(&e.state, leased).await.is_none());
}

// G-T-06, G-Q-13: out of date is unknown, never met; drafts expire silently after a week
#[tokio::test]
async fn the_tick_expires_drafts_and_out_of_date_evidence_quietly() {
    let e = setup().await;
    let mut d = new_goal(&e, "https://example.com/", 2);
    d.draft = true;
    let draft = match e.state.db.create_goal(&d, now_ms() - goals::DRAFT_TTL_MS - 1000).await.unwrap() {
        Created::Goal(g) => *g,
        _ => panic!(),
    };
    let url = server(vec![200]).await;
    let mut quick = new_goal(&e, &url, 2);
    quick.checks[0].freshness_minutes = 1;
    let g = create(&e, &quick).await;
    assert_eq!(run_now(&e, &g.id).await.status, "met");
    // Not due, so nothing runs; the draft goes
    assert!(runner::tick(&e.state).await.is_empty());
    assert!(e.state.db.get_goal(&draft.id).await.unwrap().is_none());
    assert_eq!(e.state.db.get_goal(&g.id).await.unwrap().unwrap().status, "met");
    // A minute on, its evidence is out of date
    assert!(e.state.db.stale_met_goals(now_ms() + 61_000).await.unwrap().contains(&g.id));
    let (_, page) = call(&e.app, "GET", &format!("/api/goals/{}", g.id), None, None).await;
    assert_eq!(page["goal"]["status"], "met");
    e.state.db.set_goal_status(&g.id, "unknown", "out_of_date", &["met"], None).await.unwrap().unwrap();
    assert!(notices(&e, &g.id).await.is_empty());
}

// G-Q-06: unknown never notifies before a day, then exactly once
#[tokio::test]
async fn unknown_tells_once_after_a_day() {
    let e = setup().await;
    let g = create(&e, &new_goal(&e, &closed_port(), 2)).await;
    assert_eq!(run_now(&e, &g.id).await.status, "unknown");
    runner::tick(&e.state).await;
    assert!(notices(&e, &g.id).await.is_empty());
    // A day later (moved back by hand)
    let day_ago = now_ms() - goals::UNKNOWN_NOTICE_MS - 1000;
    assert!(!e.state.db.goals_unknown_since(day_ago).await.unwrap().iter().any(|x| x.id == g.id));
    let later = e.state.db.goals_unknown_since(now_ms() + goals::UNKNOWN_NOTICE_MS).await.unwrap();
    let stuck = later.into_iter().find(|x| x.id == g.id).unwrap();
    assert!(goals::notice(&e.state, &stuck, "cant_check", "t".into(), "b".into()).await);
    assert!(e.state.db.goals_unknown_since(now_ms() + goals::UNKNOWN_NOTICE_MS).await.unwrap().is_empty());
    assert_eq!(notices(&e, &g.id).await, ["cant_check"]);
}

// G-Q-24: no more than three notices per goal per day; held back ones are logged
#[tokio::test]
async fn notices_are_capped_at_three_a_day() {
    let e = setup().await;
    let g = create(&e, &new_goal(&e, "https://example.com/", 1)).await;
    for _ in 0..3 {
        assert!(goals::notice(&e.state, &g, "broken", "t".into(), "b".into()).await);
    }
    assert!(!goals::notice(&e.state, &g, "recovered", "t".into(), "b".into()).await);
    assert!(kinds(&e, &g.id).await.contains(&"notice_capped".to_string()));
}

// G-T-01..03, G-T-07, G-T-08, G-T-10: the API
#[tokio::test]
async fn the_api_creates_edits_pauses_and_deletes_goals() {
    let e = setup().await;
    let url = server(vec![200]).await;
    let body = json!({
        "title": "Site is up",
        "checks": [
            {"kind": "http", "spec": {"url": url}},
            {"kind": "http", "spec": {"url": url}, "metric": "latency_ms", "operator": "lt", "target": 5000, "severity": "advisory"},
        ],
    });
    let (status, v) = call(&e.app, "POST", "/api/goals", None, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let id = v["goal"]["id"].as_str().unwrap().to_string();
    assert_eq!((v["goal"]["status"].as_str(), v["goal"]["mode"].as_str(), v["goal"]["confirm_failures"].as_i64()), (Some("checking"), Some("suggest"), Some(2)));
    assert_eq!(v["goal"]["checks"][0]["says"], format!("{url} answers with status 200"));
    assert_eq!(v["goal"]["checks"][1]["required"], false);
    // M1: the lowest latency target that can be met (lt 1) is accepted, lt 0 is refused in plain words
    let edge = json!({"title": "Fast", "checks": [{"kind": "http", "spec": {"url": url}, "metric": "latency_ms", "operator": "lt", "target": 1}]});
    let (status, v2) = call(&e.app, "POST", "/api/goals", None, Some(edge)).await;
    assert_eq!(status, StatusCode::OK, "{v2}");
    assert_eq!(v2["goal"]["checks"][0]["target"], "1");
    let none = json!({"title": "Fast", "checks": [{"kind": "http", "spec": {"url": url}, "metric": "latency_ms", "operator": "lt", "target": 0}]});
    let (status, v3) = call(&e.app, "POST", "/api/goals", None, Some(none)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v3["error"].as_str().unwrap_or("").contains("never be met"), "{v3}");
    // Refusals
    for (bad, code) in [
        (json!({"title": "", "checks": [{"kind": "http", "spec": {"url": url}}]}), StatusCode::BAD_REQUEST),
        (json!({"title": "x", "checks": [{"kind": "http", "spec": {"url": url}, "severity": "advisory"}]}), StatusCode::BAD_REQUEST),
        (json!({"title": "x", "interval_minutes": 5, "checks": [{"kind": "http", "spec": {"url": url}}]}), StatusCode::BAD_REQUEST),
        // M1: a latency under 0 ms could never be met
        (json!({"title": "x", "checks": [{"kind": "http", "spec": {"url": url}, "metric": "latency_ms", "operator": "lt", "target": 0}]}), StatusCode::BAD_REQUEST),
        (json!({"from_task": "t1"}), StatusCode::NOT_FOUND), // wired by goals-remediation: an unknown task is a 404
    ] {
        let (status, v) = call(&e.app, "POST", "/api/goals", None, Some(bad.clone())).await;
        assert_eq!(status, code, "{bad}: {v}");
    }
    // Check now runs in the background and comes back met
    let (status, _) = call(&e.app, "POST", &format!("/api/goals/{id}/check"), None, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let mut met = false;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if e.state.db.get_goal(&id).await.unwrap().unwrap().status == "met" {
            met = true;
            break;
        }
    }
    assert!(met);
    let (_, list) = call(&e.app, "GET", "/api/goals", None, None).await;
    assert_eq!(list["goals"][0]["checks"][0]["latest"]["verdict"], "met");
    // Pause, then Check now is refused; resume starts fresh
    let (status, v) = call(&e.app, "PATCH", &format!("/api/goals/{id}"), None, Some(json!({"paused": true, "title": "Site answers"}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!((v["goal"]["status"].as_str(), v["goal"]["title"].as_str()), (Some("paused"), Some("Site answers")));
    assert!(v["goal"]["next_check_at"].is_null());
    let (status, _) = call(&e.app, "POST", &format!("/api/goals/{id}/check"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, v) = call(&e.app, "PATCH", &format!("/api/goals/{id}"), None, Some(json!({"paused": false}))).await;
    assert_eq!(v["goal"]["status"], "checking");
    let ev: Vec<&str> = v["events"].as_array().unwrap().iter().filter_map(|x| x["kind"].as_str()).collect();
    for k in ["resumed", "paused", "edited"] {
        assert!(ev.contains(&k), "{ev:?}");
    }
    // Approve is only for drafts; another space can't see it
    let (status, _) = call(&e.app, "POST", &format!("/api/goals/{id}/approve"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(&e.app, "GET", &format!("/api/goals/{id}?space=elsewhere"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&e.app, "DELETE", &format!("/api/goals/{id}"), None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&e.app, "GET", &format!("/api/goals/{id}"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn turn(e: &Env, turn_id: &str) -> String {
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope { kwant_id: e.bot.id.clone(), kwant_name: e.bot.name.clone(), origin: Some(origin), turn_id: Some(turn_id.into()), ..Default::default() };
    mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn propose(e: &Env, token: &str, args: Value) -> (bool, String) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "goal_propose", "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (v["result"]["isError"].as_bool().unwrap_or(false), v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
}

// G-T-11, G-T-12, G-Q-11, G-Q-14: a bot proposes, the person approves; three drafts at most
#[tokio::test]
async fn a_bot_proposes_drafts_and_only_the_person_approves() {
    let e = setup().await;
    let token = turn(&e, "t-goal").await;
    // M1: an impossible target is refused with a plain reason and takes no draft slot
    let impossible = json!({"title": "Fast", "checks": [{"kind": "http", "spec": {"url": "https://example.com"}, "metric": "latency_ms", "operator": "lt", "target": 0}]});
    let (err, text) = propose(&e, &token, impossible).await;
    assert!(err && text.contains("never be met"), "{text}");
    assert_eq!(e.state.db.count_draft_goals(&e.space).await.unwrap(), 0);
    let args = json!({"title": "Report exists", "mode": "fix", "checks": [{"kind": "file_exists", "spec": {"path": "report.md"}}]});
    let (err, text) = propose(&e, &token, args.clone()).await;
    assert!(!err, "{text}");
    let id = serde_json::from_str::<Value>(&text).unwrap()["goal_id"].as_str().unwrap().to_string();
    let g = e.state.db.get_goal(&id).await.unwrap().unwrap();
    assert_eq!((g.status.as_str(), g.mode.as_str(), g.created_by.as_str()), ("draft", "suggest", e.bot.id.as_str()));
    // A draft runs nothing, even at the tick
    assert!(g.next_check_at.is_none());
    assert!(runner::tick(&e.state).await.is_empty());
    for _ in 0..2 {
        assert!(!propose(&e, &token, args.clone()).await.0);
    }
    let (err, text) = propose(&e, &token, args.clone()).await;
    assert!(err && text.contains("already 3 goals"), "{text}");
    // A bot can't approve, edit or run it
    for (method, uri) in [("POST", format!("/api/goals/{id}/approve")), ("PATCH", format!("/api/goals/{id}")), ("POST", format!("/api/goals/{id}/check"))] {
        let (status, _) = call(&e.app, method, &uri, Some(&token), Some(json!({"title": "x"}))).await;
        assert!(status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED, "{method} {uri}: {status}");
    }
    let (status, v) = call(&e.app, "POST", &format!("/api/goals/{id}/approve"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["goal"]["status"], "checking");
    assert!(v["goal"]["next_check_at"].as_i64().is_some());
}

// G-Q-14: an upgrade (a fresh install) has no goals and starts nothing
#[tokio::test]
async fn a_fresh_install_runs_nothing() {
    let e = setup().await;
    assert!(runner::tick(&e.state).await.is_empty());
    let (status, v) = call(&e.app, "GET", "/api/goals", None, None).await;
    assert_eq!((status, v["goals"].as_array().map(Vec::len)), (StatusCode::OK, Some(0)));
}
