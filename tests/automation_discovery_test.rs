//! Automation Discovery (Phase D4). Off by default and then reads and writes nothing; on, scans
//! (hourly, or when the person asks) keep offers from the person's board tasks, within the daily
//! cap and 5 live per space, one notification each; Accept makes the routine once, Dismiss cools
//! the habit off; a preview writes nothing; bots get 403 everywhere; it only learns from tasks
//! the person made. The miner's own cases are unit tests in `src/automation_discovery.rs`.
//! Contract: docs/mission-control.md §20.

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Utc};
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
        create_router, mcp_runtime,
        missions::discovery::{self, AcceptBody},
        AppState,
    },
    tools::ToolRunner,
    RuntimeEvent,
};
use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TZ: &str = "Europe/Berlin";
const SCAN: &str = "/api/automation-discovery/scan";
const PREVIEW: &str = "/api/automation-discovery/scan?preview=1";
const LIST: &str = "/api/automation-discovery";

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
    let (status, _, v) = call_with(app, method, uri, auth, None, body).await;
    (status, v)
}

async fn call_with(app: &Router, method: &str, uri: &str, auth: Option<&str>, key: Option<&str>, body: Option<Value>) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = key {
        req = req.header("idempotency-key", key);
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Accept as the person with Idempotency-Key `key`.
async fn accept(e: &Env, id: &str, key: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
    call_with(&e.app, "POST", &format!("{LIST}/{id}/accept"), None, Some(key), Some(body)).await
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
    ended_task(e, owner, title, at, source_kind, created_by, "verified", None).await
}

/// [`done_task`] ending in `state`, forked from `fork_of` if given.
#[allow(clippy::too_many_arguments)]
async fn ended_task(e: &Env, owner: &Kwant, title: &str, at: DateTime<Utc>, source_kind: &str, created_by: &str, state: &str, fork_of: Option<&str>) -> String {
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
    sqlx::query("UPDATE tasks SET state = ?, forked_from = ?, created_at = ?, finished_at = ? WHERE id = ?")
        .bind(state)
        .bind(fork_of)
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
    let before = written_rows(&e).await;
    let v = person(&e, "POST", SCAN, None).await;
    assert_eq!(v, json!({"blocked_by": "off", "preview": false, "offered": [], "skipped": [], "expired": [], "superseded": []}));
    let v = person(&e, "POST", PREVIEW, None).await;
    assert_eq!(v, json!({"blocked_by": "off", "preview": true, "offered": [], "skipped": []}));
    // Nor does the hourly scan, ever
    assert_eq!(discovery::tick(&e.state, Utc::now()).await, 0);
    assert_eq!(discovery::tick(&e.state, Utc::now() + Duration::hours(3)).await, 0);
    assert_eq!(written_rows(&e).await, before);
    let l = person(&e, "GET", LIST, None).await;
    assert_eq!((l["setting"].clone(), l["blocked_by"].clone(), l["suggestions"].clone()), (json!("off"), json!(["off"]), json!([])));
}

#[tokio::test]
async fn the_scan_needs_the_board_and_a_time_zone() {
    let e = setup().await;
    discovery_on(&e).await;
    person(&e, "POST", "/api/settings", Some(json!({"timezone": ""}))).await;
    assert_eq!(person(&e, "POST", SCAN, None).await["blocked_by"], "no_timezone");
    assert_eq!(person(&e, "GET", LIST, None).await["blocked_by"], json!(["no_timezone"]));
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": false, "timezone": TZ}))).await;
    assert_eq!(person(&e, "POST", SCAN, None).await["blocked_by"], "board_off");
    assert_eq!(person(&e, "POST", PREVIEW, None).await["blocked_by"], "board_off");
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
        // The person's own, but cancelled, or a fork of another task
        let cancelled = ended_task(&e, &a, "Weekly backup", weeks_ago(w), "person", "person", "cancelled", None).await;
        ended_task(&e, &a, "Weekly cleanup", weeks_ago(w), "person", "person", "verified", Some(&cancelled)).await;
    }
    let before: Vec<i64> = written_rows(&e).await;

    let v = person(&e, "POST", PREVIEW, None).await;
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
    assert_eq!(person(&e, "POST", PREVIEW, None).await, v);
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
    // Not a number at all: refused before anything is saved
    let (status, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"automation_discovery_daily_cap": "abc"}))).await;
    assert!(status.is_client_error(), "{status} {v}");
    assert_eq!(person(&e, "GET", "/api/settings", None).await["automation_discovery_daily_cap"], 2);
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

// Step 3: offers that are kept, Accept, Dismiss, the hourly scan

/// Six weekly asks of `title` by `owner`, at 09:00 Berlin, the last on `last`.
async fn mondays(e: &Env, owner: &Kwant, title: &str, last: NaiveDate) {
    for w in 0..6 {
        let at = berlin(last - Duration::weeks(w), 9, 0);
        done_task(e, owner, &format!("{title} {}", at.date_naive()), at, "person", "person").await;
    }
}

fn berlin(d: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    local_instant(TZ, d, NaiveTime::from_hms_opt(h, m, 0).unwrap()).unwrap().unwrap()
}

fn day(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// A habit on a weekday of its own (`k` days before today's), for its own bot.
async fn habit_on(e: &Env, k: i64, name: &str) -> Kwant {
    let b = bot(e, name).await;
    for w in 1..=6 {
        let at = weeks_ago(w) - Duration::days(k);
        done_task(e, &b, &format!("{name} status digest {}", at.date_naive()), at, "person", "person").await;
    }
    b
}

async fn live(e: &Env) -> i64 {
    sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM routine_suggestions WHERE status = 'offered'").fetch_one(&e.pool).await.unwrap().0
}

async fn events(e: &Env, suggestion: &str) -> Vec<(String, String)> {
    sqlx::query_as("SELECT kind, actor FROM routine_suggestion_events WHERE suggestion_id = ? ORDER BY id").bind(suggestion).fetch_all(&e.pool).await.unwrap()
}

fn suggested(rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::Notify { kind, title, body, .. } = ev {
            if kind == "automation_suggested" {
                out.push((title, body));
            }
        }
    }
    out
}

#[tokio::test]
async fn a_scan_keeps_one_offer_per_habit_and_notifies_once() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let mut rx = e.state.event_sender.subscribe();

    let v = person(&e, "POST", SCAN, None).await;
    assert_eq!((v["preview"].clone(), titles(&v, "offered")), (json!(false), vec!["Weekly report".to_string()]), "{v}");
    let o = &v["offered"][0];
    let id = o["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("sug_"), "{o}");
    assert_eq!(o["status"], "offered");
    assert_eq!(o["sources"].as_array().unwrap().len(), 6, "{o}");
    assert_eq!(o["permissions"]["payments_ask"], true);
    assert!(o["permissions"]["tier_label"].as_str().is_some(), "{o}");
    assert!(o["first_run_text"].as_str().unwrap().ends_with(TZ));
    assert_eq!(events(&e, &id).await, [("offered".to_string(), "kwant".to_string())]);
    let n = suggested(&mut rx);
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0].0, "Suggested routine: Weekly report");
    assert!(n[0].1.starts_with("Ana could do this"), "{n:?}");

    // Again: the live offer stays the one, no second notification
    let v = person(&e, "POST", SCAN, None).await;
    assert!(v["offered"].as_array().unwrap().is_empty(), "{v}");
    assert_eq!(v["skipped"][0]["reason"], "already_offered");
    assert_eq!(live(&e).await, 1);
    assert!(suggested(&mut rx).is_empty());

    let l = person(&e, "GET", LIST, None).await;
    assert_eq!((l["setting"].clone(), l["blocked_by"].clone()), (json!("on"), json!([])));
    assert_eq!(l["suggestions"].as_array().unwrap().iter().map(|s| s["id"].clone()).collect::<Vec<_>>(), [json!(id)]);
    assert!(l["last_scan_at"].is_string(), "{l}");
    assert_eq!(l["stats"]["offered_30d"], 1, "{l}");
    // While scans can't run, live offers are hidden
    person(&e, "POST", "/api/settings", Some(json!({"automation_discovery": "off"}))).await;
    assert_eq!(person(&e, "GET", LIST, None).await["suggestions"], json!([]));
    let (status, _) = call(&e.app, "GET", &format!("{LIST}?status=maybe"), None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accept_makes_the_routine_once() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let id = person(&e, "POST", SCAN, None).await["offered"][0]["id"].as_str().unwrap().to_string();

    // Without a key nothing happens
    let (status, _, v) = call_with(&e.app, "POST", &format!("{LIST}/{id}/accept"), None, None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    // Out-of-range check, unknown field, another space: refused, nothing made
    assert_eq!(accept(&e, &id, "k0", json!({"keep_checks": [3]})).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(accept(&e, &id, "k0b", json!({"surprise": 1})).await.0.is_client_error());
    let (status, _, _) = call_with(&e.app, "POST", &format!("{LIST}/{id}/accept?space=spc_other"), None, Some("k0c"), Some(json!({}))).await;
    assert!(status.is_client_error(), "{status}");
    assert_eq!(count(&e, "routines").await, 0);

    let (status, _, v) = accept(&e, &id, "k1", json!({})).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let r = &v["routine"];
    assert_eq!((r["origin"].clone(), r["suggestion_id"].clone(), r["enabled"].clone()), (json!("discovery"), json!(id), json!(true)), "{r}");
    assert_eq!(r["kwant_id"], a.id);
    assert_eq!(r["checks"], json!([{"kind": "file_exists", "spec": {"path": "reports/weekly.md"}, "required": true}]));
    assert_eq!(r["acceptance"], "The report is in the shared folder");
    assert!(v["first_run_text"].as_str().unwrap().ends_with(TZ), "{v}");
    assert_eq!((v["suggestion"]["status"].clone(), v["suggestion"]["routine_id"].clone()), (json!("accepted"), r["id"].clone()));

    // The same key replays it; another key, or Dismiss, finds it already accepted
    let (status, headers, again) = accept(&e, &id, "k1", json!({})).await;
    assert_eq!((status, again["routine"]["id"].clone()), (StatusCode::CREATED, r["id"].clone()));
    assert_eq!(headers.get("idempotent-replayed").and_then(|h| h.to_str().ok()), Some("true"));
    let (status, _, v) = accept(&e, &id, "k2", json!({})).await;
    assert_eq!((status, v["status"].clone(), v["routine_id"].clone()), (StatusCode::CONFLICT, json!("accepted"), r["id"].clone()), "{v}");
    let (status, v) = call(&e.app, "POST", &format!("{LIST}/{id}/dismiss"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(count(&e, "routines").await, 1);
    assert_eq!(events(&e, &id).await.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), ["offered", "accepted"]);

    // The routine now covers the habit
    let v = person(&e, "POST", SCAN, None).await;
    assert_eq!(v["skipped"][0]["reason"], "covered", "{v}");
    // Switching it off and on is in the suggestion's history
    let rid = r["id"].as_str().unwrap();
    person(&e, "PATCH", &format!("/api/routines/{rid}"), Some(json!({"enabled": false}))).await;
    person(&e, "PATCH", &format!("/api/routines/{rid}"), Some(json!({"enabled": true}))).await;
    assert_eq!(events(&e, &id).await.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), ["offered", "accepted", "disabled", "enabled"]);
    let (status, v) = call(&e.app, "GET", &format!("{LIST}?status=accepted"), None, None).await;
    assert_eq!((status, v["suggestions"].as_array().unwrap().len()), (StatusCode::OK, 1));
}

#[tokio::test]
async fn dismiss_cools_the_habit_off_and_accept_is_refused() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let id = person(&e, "POST", SCAN, None).await["offered"][0]["id"].as_str().unwrap().to_string();
    let (status, v) = call(&e.app, "POST", &format!("{LIST}/{id}/dismiss?space=spc_other"), None, None).await;
    assert!(status.is_client_error(), "{v}");
    let v = person(&e, "POST", &format!("{LIST}/{id}/dismiss"), None).await;
    assert_eq!(v["suggestion"]["status"], "dismissed");
    let until = DateTime::parse_from_rfc3339(v["suggestion"]["cooldown_until"].as_str().unwrap()).unwrap();
    assert!((until.with_timezone(&Utc) - Utc::now() - Duration::days(30)).num_minutes().abs() < 5, "{v}");

    let (status, _, v) = accept(&e, &id, "k1", json!({})).await;
    assert_eq!((status, v["status"].clone()), (StatusCode::CONFLICT, json!("dismissed")), "{v}");
    assert_eq!(count(&e, "routines").await, 0);
    let v = person(&e, "POST", SCAN, None).await;
    assert_eq!((v["offered"].clone(), v["skipped"][0]["reason"].clone()), (json!([]), json!("cooling_off")), "{v}");
    let (status, _) = call(&e.app, "POST", &format!("{LIST}/sug_nope/dismiss"), None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn accepted_days_later_the_first_run_is_the_next_slot() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let space = SpaceId::default_space();
    mondays(&e, &a, "Weekly report", day("2026-10-12")).await;
    mondays(&e, &a, "Send invoices", day("2026-10-12")).await;
    // Offered on Monday 12 Oct, after that day's asks: first run the Monday after
    let monday = berlin(day("2026-10-12"), 10, 0);
    let stored = discovery::scan_space(&e.state, &space, TZ, monday).await.unwrap();
    assert_eq!(stored.offered.len(), 2, "{stored:?}");
    let rows = e.state.db.list_suggestions(space.as_str(), Some("offered")).await.unwrap();
    assert!(rows.iter().all(|r| r.first_run_at == berlin(day("2026-10-19"), 9, 0)));
    let id = |name: &str| rows.iter().find(|r| r.name == name).unwrap().id.clone();

    // Accepted on Wednesday 21 Oct, after the 19th passed: Monday 26 Oct (winter time by then)
    let wed = berlin(day("2026-10-21"), 15, 0);
    let v = discovery::accept_at(&e.state, None, &id("Weekly report"), AcceptBody::default(), wed).await.unwrap();
    assert_eq!(v["routine"]["next_run_at"], json!(berlin(day("2026-10-26"), 9, 0)), "{v}");
    assert_eq!(v["first_run_text"], v["suggestion"]["first_run_text"]);
    // Moved to Thursdays: the next Thursday
    let body = AcceptBody { schedule: Some(json!({"type": "weekly", "time": "09:00", "weekdays": ["thu"], "tz": TZ})), ..Default::default() };
    let v = discovery::accept_at(&e.state, None, &id("Send invoices"), body, wed).await.unwrap();
    assert_eq!(v["routine"]["next_run_at"], json!(berlin(day("2026-10-22"), 9, 0)), "{v}");
    assert_eq!(v["routine"]["schedule"]["weekdays"], json!(["thu"]));
}

#[tokio::test]
async fn accept_takes_the_persons_edits() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let b = bot(&e, "Ben").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let id = person(&e, "POST", SCAN, None).await["offered"][0]["id"].as_str().unwrap().to_string();
    let body = json!({"name": "Friday report", "instructions": "Write it", "kwant_id": b.id, "keep_checks": [], "acceptance": null});
    let (status, _, v) = accept(&e, &id, "k1", body).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let r = &v["routine"];
    assert_eq!((r["name"].clone(), r["instructions"].clone(), r["kwant_id"].clone()), (json!("Friday report"), json!("Write it"), json!(b.id)));
    assert_eq!((r["checks"].clone(), r["acceptance"].clone(), r["channel_id"].clone(), r["thread_id"].clone()), (json!([]), Value::Null, Value::Null, Value::Null));
}

#[tokio::test]
async fn the_hourly_scan_keeps_the_cap_and_five_live() {
    let e = setup().await;
    person(&e, "POST", "/api/settings", Some(json!({"automation_discovery": "on", "automation_discovery_daily_cap": 10}))).await;
    for (k, name) in ["Ana", "Ben", "Cleo", "Dan", "Eve", "Fay", "Gus"].iter().enumerate() {
        habit_on(&e, k as i64, name).await;
    }
    let now = Utc::now();
    assert_eq!(discovery::tick(&e.state, now).await, 5);
    assert_eq!(live(&e).await, 5);
    // Within the hour nothing runs; after it, the space is full
    assert_eq!(discovery::tick(&e.state, now + Duration::minutes(30)).await, 0);
    assert_eq!(discovery::tick(&e.state, now + Duration::minutes(61)).await, 0);
    assert_eq!(live(&e).await, 5);
}

#[tokio::test]
async fn the_daily_cap_holds_across_scans() {
    let e = setup().await;
    discovery_on(&e).await;
    for (k, name) in ["Ana", "Ben", "Cleo", "Dan"].iter().enumerate() {
        habit_on(&e, k as i64, name).await;
    }
    let mut rx = e.state.event_sender.subscribe();
    let now = Utc::now();
    assert_eq!(discovery::tick(&e.state, now).await, 2);
    assert_eq!(discovery::tick(&e.state, now + Duration::minutes(61)).await, 0);
    let v = person(&e, "POST", SCAN, None).await;
    assert!(v["offered"].as_array().unwrap().is_empty() && v["skipped"].as_array().unwrap().iter().any(|s| s["reason"] == "capped"), "{v}");
    assert_eq!(live(&e).await, 2);
    assert_eq!(discovery::tick(&e.state, now + Duration::hours(25)).await, 2);
    assert_eq!(live(&e).await, 4);
    assert_eq!(suggested(&mut rx).len(), 4);
}

#[tokio::test]
async fn a_live_offer_ends_when_its_habit_does() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    let space = SpaceId::default_space();
    mondays(&e, &a, "Weekly report", day("2026-10-12")).await;
    let monday = berlin(day("2026-10-12"), 10, 0);
    let id = discovery::scan_space(&e.state, &space, TZ, monday).await.unwrap().offered[0].clone();
    // Three Mondays later without an ask, it has stopped
    let later = berlin(day("2026-11-03"), 10, 0);
    let s = discovery::scan_space(&e.state, &space, TZ, later).await.unwrap();
    assert_eq!((s.expired.clone(), s.offered.len()), (vec![id.clone()], 0), "{s:?}");
    assert_eq!(e.state.db.get_suggestion(&id).await.unwrap().unwrap().status, "expired");
    assert_eq!(events(&e, &id).await.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), ["offered", "expired"]);
}

#[tokio::test]
async fn bots_get_403_on_every_route() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let id = person(&e, "POST", SCAN, None).await["offered"][0]["id"].as_str().unwrap().to_string();
    let t = token(&e, &a).await;
    for (method, uri) in [("GET", LIST.to_string()), ("POST", SCAN.to_string()), ("POST", PREVIEW.to_string()), ("POST", format!("{LIST}/{id}/accept")), ("POST", format!("{LIST}/{id}/dismiss"))] {
        let (status, _, v) = call_with(&e.app, method, &uri, Some(&t), Some("k"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    assert_eq!((count(&e, "routines").await, live(&e).await), (0, 1));
}

async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
}

#[tokio::test]
async fn undoing_a_bots_delete_keeps_the_routine_discovered() {
    let e = setup().await;
    let a = bot(&e, "Ana").await;
    discovery_on(&e).await;
    habit(&e, &a, "Weekly report").await;
    let id = person(&e, "POST", SCAN, None).await["offered"][0]["id"].as_str().unwrap().to_string();
    let r = accept(&e, &id, "k1", json!({})).await.2["routine"].clone();

    let t = token(&e, &a).await;
    let (text, failed) = tool(&e, &t, "routine_delete", json!({"routine": r["id"]})).await;
    assert!(!failed, "{text}");
    let s = e.state.db.get_suggestion(&id).await.unwrap().unwrap();
    assert!(s.routine_deleted_at.is_some());
    assert_eq!(events(&e, &id).await.last().unwrap().0, "deleted");
    // Deleted, the habit cools off rather than being offered again
    assert_eq!(person(&e, "POST", SCAN, None).await["skipped"][0]["reason"], "cooling_off");

    let actions = person(&e, "GET", "/api/bot-actions", None).await;
    let del = actions.as_array().unwrap().iter().find(|x| x["action"] == "routine_delete").unwrap().clone();
    let v = person(&e, "POST", &format!("/api/bot-actions/{}/undo", del["id"].as_str().unwrap()), None).await;
    assert_eq!(v["status"], "undone", "{v}");
    let back = person(&e, "GET", "/api/routines", None).await[0].clone();
    assert_eq!((back["origin"].clone(), back["suggestion_id"].clone()), (json!("discovery"), json!(id)), "{back}");
    let s = e.state.db.get_suggestion(&id).await.unwrap().unwrap();
    assert_eq!((s.routine_id.as_deref(), s.routine_deleted_at), (back["id"].as_str(), None));
    assert_eq!(person(&e, "POST", SCAN, None).await["skipped"][0]["reason"], "covered");
}
