//! Automation Discovery step 2 (Phase D4): the read-only scan route. It is off by default and
//! then reads nothing; on, it previews offers from the person's board tasks and never writes an
//! offer or a routine; bots can't call it; it only learns from tasks the person made. The miner's
//! own cases are unit tests in `src/automation_discovery.rs`. Contract: docs/mission-control.md §20.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc};
use kwnt::{
    automation_discovery::cluster_key,
    broker::PermissionBroker,
    config::Config,
    db::{
        continuity::NewRoutine,
        missions::{NewCheck, NewTask},
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    routines::{local_date, local_instant},
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TZ: &str = "Europe/Berlin";
const SCAN: &str = "/api/automation-discovery/scan";

struct Env {
    app: Router,
    state: AppState,
    general: String,
    pool: SqlitePool,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), laya_url: "http://127.0.0.1:9".into(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let general = state.db.list_channels(&SpaceId::default_space()).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let pool = SqlitePoolOptions::new().max_connections(1).connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
    let e = Env { app: create_router(state.clone()), state, general, pool, _tmp: tmp };
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": true, "timezone": TZ}))).await;
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

async fn discovery_on(e: &Env) {
    person(e, "POST", "/api/settings", Some(json!({"automation_discovery": "on"}))).await;
}

async fn bot(e: &Env, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Tester".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "codex".into(),
    };
    e.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

/// Noon in Berlin, `weeks` weeks before today there: a weekly habit on today's weekday.
fn weeks_ago(weeks: i64) -> DateTime<Utc> {
    let today = local_date(TZ, Utc::now()).unwrap().unwrap();
    local_instant(TZ, today - Duration::weeks(weeks), NaiveTime::from_hms_opt(12, 0, 0).unwrap()).unwrap().unwrap()
}

/// A task `owner` verified, made `created_by` from `source_kind` at `at`.
async fn done_task(e: &Env, owner: &Kwant, title: &str, at: DateTime<Utc>, source_kind: &str, created_by: &str) -> String {
    let t = NewTask {
        title: title.into(),
        acceptance: "The report is in the shared folder".into(),
        state: "ready".into(),
        priority: 1,
        owner_kwant_id: Some(owner.id.clone()),
        max_attempts: 3,
        source_kind: source_kind.into(),
        created_by: created_by.into(),
        ..Default::default()
    };
    let t = e.state.db.create_task(&SpaceId::default_space(), &t).await.unwrap();
    let check = NewCheck {
        task_id: t.id.clone(),
        kind: "file_exists".into(),
        spec: json!({"path": "reports/weekly.md"}),
        required: true,
        approved: true,
        requested_by: "person".into(),
        approved_by: None,
    };
    let c = e.state.db.add_check(&check).await.unwrap();
    e.state.db.finish_check(&c.id, "passed", None, "person", None).await.unwrap();
    sqlx::query("UPDATE tasks SET state = 'verified', created_at = ?, finished_at = ? WHERE id = ?")
        .bind(at)
        .bind(at + Duration::minutes(30))
        .bind(&t.id)
        .execute(&e.pool)
        .await
        .unwrap();
    t.id
}

/// Six weekly asks of `title`, made by the person on the board.
async fn habit(e: &Env, owner: &Kwant, title: &str) {
    for w in 1..=6 {
        done_task(e, owner, &format!("{title} {}", weeks_ago(w).date_naive()), weeks_ago(w), "person", "person").await;
    }
}

async fn count(e: &Env, table: &str) -> i64 {
    sqlx::query_as::<_, (i64,)>(&format!("SELECT COUNT(*) FROM {table}")).fetch_one(&e.pool).await.unwrap().0
}

async fn token(e: &Env, bot: &Kwant) -> String {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), origin: Some(origin), turn_id: Some(turn_id.clone()), ..Default::default() };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id), None, Some(scope)).await.unwrap().1
}

fn titles(v: &Value, list: &str) -> Vec<String> {
    v[list].as_array().unwrap().iter().map(|o| o[if list == "offered" { "name" } else { "title" }].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn the_scan_is_off_by_default_and_then_finds_nothing() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    habit(&e, &a, "Weekly report").await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((s["automation_discovery"].as_str(), s["automation_discovery_daily_cap"].as_i64()), (Some("off"), Some(2)));
    let v = person(&e, "POST", SCAN, None).await;
    assert_eq!(v, json!({"blocked_by": "off", "preview": true, "offered": [], "skipped": []}));
}

#[tokio::test]
async fn the_scan_needs_the_board_and_a_time_zone() {
    let e = setup().await;
    discovery_on(&e).await;
    person(&e, "POST", "/api/settings", Some(json!({"timezone": ""}))).await;
    assert_eq!(person(&e, "POST", SCAN, None).await["blocked_by"], "no_timezone");
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": false, "timezone": TZ}))).await;
    assert_eq!(person(&e, "POST", SCAN, None).await["blocked_by"], "board_off");
}

#[tokio::test]
async fn a_scan_previews_a_habit_and_writes_nothing() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    // A one-off, and the same habit made by a bot, a routine or a hand-off: none of it counts
    done_task(&e, &a, "Fix the login page", weeks_ago(1), "person", "person").await;
    for w in 1..=6 {
        done_task(&e, &a, "Weekly digest", weeks_ago(w), "routine", "person").await;
        done_task(&e, &a, "Weekly digest", weeks_ago(w), "delegation", &a.id).await;
        done_task(&e, &a, "Weekly digest", weeks_ago(w), "mission", &a.id).await;
        done_task(&e, &a, "Weekly digest", weeks_ago(w), "chat", "person").await;
    }
    let before: Vec<i64> = written_rows(&e).await;

    let v = person(&e, "POST", SCAN, None).await;
    assert_eq!((v["blocked_by"].clone(), v["preview"].clone(), v["timezone"].clone()), (Value::Null, json!(true), json!(TZ)), "{v}");
    assert_eq!(titles(&v, "offered"), ["Weekly report"], "{v}");
    let o = &v["offered"][0];
    let local = local_date(TZ, weeks_ago(1)).unwrap().unwrap();
    let weekday = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"][local.weekday().num_days_from_monday() as usize];
    assert_eq!(o["schedule"], json!({"type": "weekly", "time": "12:00", "weekdays": [weekday], "tz": TZ}), "{o}");
    assert_eq!(o["kwant_id"], a.id);
    assert_eq!(o["cluster_key"], cluster_key("general", "Weekly report").unwrap());
    assert_eq!(o["delivery"], json!({"direct_kwant_id": a.id}));
    assert_eq!(o["checks"], json!([{"kind": "file_exists", "spec": {"path": "reports/weekly.md"}, "required": true}]));
    assert_eq!(o["evidence"]["pattern"], "weekly");
    assert_eq!(o["source_task_ids"].as_array().unwrap().len(), 6);
    assert!(o["first_run_at"].as_str().is_some() && o["first_run_text"].as_str().unwrap().ends_with(TZ), "{o}");
    // The one-off is skipped; the digest made by others isn't seen at all
    assert_eq!(v["skipped"], json!([{"cluster_key": cluster_key("general", "Fix the login page").unwrap(), "title": "Fix the login page", "asks": 1, "reason": "too_short"}]));

    // Nothing written: no offer, no history, no routine, no setting
    assert_eq!(written_rows(&e).await, before);
    assert_eq!(before[..3], [0, 0, 0]);
    // And the same again
    assert_eq!(person(&e, "POST", SCAN, None).await, v);
}

/// Rows in routine_suggestions, routine_suggestion_events, routines and settings.
async fn written_rows(e: &Env) -> Vec<i64> {
    let mut n = Vec::new();
    for t in ["routine_suggestions", "routine_suggestion_events", "routines", "settings"] {
        n.push(count(e, t).await);
    }
    n
}

#[tokio::test]
async fn a_routine_on_or_off_covers_the_habit() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let r = NewRoutine {
        kwant_id: a.id.clone(),
        name: "Weekly report".into(),
        instructions: "Write the weekly report".into(),
        schedule: json!({"type": "weekly", "time": "12:00", "weekdays": ["mon"], "tz": TZ}).to_string(),
        enabled: false,
        ..Default::default()
    };
    let routine = e.state.db.create_routine(&r).await.unwrap();
    let key = cluster_key("general", "Weekly report").unwrap();
    let skipped = |v: &Value| v["skipped"].as_array().unwrap().iter().map(|s| (s["cluster_key"].as_str().unwrap().to_string(), s["reason"].as_str().unwrap().to_string())).collect::<Vec<_>>();
    let v = person(&e, "POST", SCAN, None).await;
    assert!(v["offered"].as_array().unwrap().is_empty(), "{v}");
    assert_eq!(skipped(&v), [(key.clone(), "covered".to_string())]);
    sqlx::query("UPDATE routines SET enabled = 1 WHERE id = ?").bind(&routine.id).execute(&e.pool).await.unwrap();
    assert_eq!(skipped(&person(&e, "POST", SCAN, None).await), [(key.clone(), "covered".to_string())]);
}

#[tokio::test]
async fn bots_cannot_scan() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    let t = token(&e, &a).await;
    let (status, _) = call(&e.app, "POST", SCAN, Some(&t), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&e.app, "POST", "/api/settings", Some(&t), Some(json!({"automation_discovery": "on"}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn settings_are_checked() {
    let e = setup().await;
    for bad in [json!({"automation_discovery": "maybe"}), json!({"automation_discovery_daily_cap": 11}), json!({"automation_discovery_daily_cap": -1})] {
        let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }
    person(&e, "POST", "/api/settings", Some(json!({"automation_discovery": "on", "automation_discovery_daily_cap": 0}))).await;
    let s = person(&e, "GET", "/api/settings", None).await;
    assert_eq!((s["automation_discovery"].as_str(), s["automation_discovery_daily_cap"].as_i64()), (Some("on"), Some(0)));
}

#[tokio::test]
async fn a_daily_cap_of_zero_offers_nothing() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    person(&e, "POST", "/api/settings", Some(json!({"automation_discovery": "on", "automation_discovery_daily_cap": 0}))).await;
    habit(&e, &a, "Weekly report").await;
    let v = person(&e, "POST", SCAN, None).await;
    assert!(v["offered"].as_array().unwrap().is_empty());
    assert_eq!(v["skipped"][0]["reason"], "capped");
}
