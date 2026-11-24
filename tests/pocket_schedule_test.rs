//! pa7 For Friday: times from `when`, the reminder the evening before, the results post, and the
//! chat a pocket came from. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa7/brief.md ("Tests").

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, NaiveTime, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        Database,
    },
    drivers::events::RuntimeEvent,
    pocket::{
        self,
        kits::Viewer,
        listener,
        schedule::{self, Origin},
        CreateRequest, JoinRequest,
    },
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tokio::sync::broadcast::Receiver;
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
/// A guest name no notification may ever carry.
const CANARY: &str = "Zyxqv Canary";

struct T {
    main: Router,
    state: AppState,
    data: PathBuf,
    _tmp: TempDir,
}

async fn state_at(data: &std::path::Path) -> AppState {
    let db = Database::init(data).await.unwrap();
    let config = Config { data_dir: data.to_path_buf(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state
}

async fn setup() -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let state = state_at(&data).await;
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    // The person's zone: UTC, so "8 PM" below is 20:00 UTC
    state.db.set_setting("timezone", "UTC").await.unwrap();
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), state, data, _tmp: tmp }
}

/// The server started again on the same data: nothing in memory survives.
async fn restart(t: &T) -> AppState {
    state_at(&t.data).await
}

async fn bot(t: &T, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Quizmaster".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    t.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// `days` from today at 20:00 UTC (always in the future for days >= 1).
fn eight_pm_in(days: i64) -> DateTime<Utc> {
    (Utc::now().date_naive() + Duration::days(days)).and_time(NaiveTime::from_hms_opt(20, 0, 0).unwrap()).and_utc()
}

/// 19:00 UTC the evening before `when`.
fn seven_pm_before(when: DateTime<Utc>) -> DateTime<Utc> {
    (when.date_naive() - Duration::days(1)).and_time(NaiveTime::from_hms_opt(19, 0, 0).unwrap()).and_utc()
}

fn origin(k: &Kwant) -> Option<Origin> {
    Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None })
}

async fn make(t: &T, spec: Value, origin: Option<Origin>, opens_at: Option<DateTime<Utc>>) -> String {
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "test".into(), spec, origin, opens_at, ..Default::default() }).await.unwrap();
    s.row.code.clone()
}

async fn join(t: &T, code: &str, name: &str) {
    pocket::join(&t.state, code, JoinRequest { name: name.into(), avatar: None, team: None, pin: None }).await.unwrap();
}

/// The notifications sent so far: (kind, title, body).
fn notes(rx: &mut Receiver<RuntimeEvent>) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::Notify { kind, title, body, .. } = ev {
            out.push((kind, title, body));
        }
    }
    out
}

/// The pocket cards in the bot's direct chat: (reason, text, meta).
async fn cards(state: &AppState, k: &Kwant) -> Vec<(String, String, Value)> {
    state
        .db
        .list_messages(&k.id, 100)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|m| {
            let meta: Value = serde_json::from_str(m.meta.as_deref()?).ok()?;
            (meta["kind"] == "pocket").then(|| (meta["reason"].as_str().unwrap_or("").to_string(), m.text, meta))
        })
        .collect()
}

#[tokio::test]
async fn when_sets_the_times_and_explicit_ones_win() {
    let t = setup().await;
    let w = eight_pm_in(3);
    let code = make(&t, json!({ "title": "Pub quiz", "when": w.to_rfc3339() }), None, None).await;
    let s = pocket::snapshot(&t.state, &code).await.unwrap().unwrap();
    assert_eq!(s.row.opens_at, Some(w - Duration::minutes(30)), "the lobby opens 30 minutes before");
    assert_eq!(s.row.closes_at, Some(w + Duration::hours(4)), "closes 4 hours after");

    let explicit_open = w - Duration::hours(2);
    let code = make(&t, json!({ "title": "Pub quiz", "when": w.to_rfc3339() }), None, Some(explicit_open)).await;
    let s = pocket::snapshot(&t.state, &code).await.unwrap().unwrap();
    assert_eq!(s.row.opens_at, Some(explicit_open), "an explicit time wins");
    assert_eq!(s.row.closes_at, Some(w + Duration::hours(4)));

    // A past `when` opens now: no times, no reminder
    let code = make(&t, json!({ "title": "Pub quiz", "when": (Utc::now() - Duration::days(1)).to_rfc3339() }), None, None).await;
    let s = pocket::snapshot(&t.state, &code).await.unwrap().unwrap();
    assert_eq!((s.row.opens_at, s.row.closes_at), (None, None));
    assert_eq!(t.state.db.pocket_schedule(&code).await.unwrap().unwrap().remind_at, None);
    join(&t, &code, "Mia").await;

    // Through the person's API, with the chat it came from
    let k = bot(&t, "Quizmaster").await;
    let (status, body) = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "test", "spec": { "title": "Pub quiz", "when": w.to_rfc3339() }, "origin": { "kwant_id": k.id } }))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let row = t.state.db.pocket_schedule(body["pocket"]["code"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(row.starts_at, Some(w));
    assert_eq!(row.remind_at, Some(seven_pm_before(w)));
    assert!(row.origin.unwrap().contains(&k.id));
    // A chat that doesn't exist is refused
    let (status, body) = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "test", "spec": { "title": "Pub quiz" }, "origin": { "kwant_id": "no-such-bot" } }))).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_origin")));
}

#[tokio::test]
async fn the_reminder_fires_once_at_seven_the_evening_before() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let w = eight_pm_in(3);
    let r = seven_pm_before(w);
    let code = make(&t, json!({ "title": "Pub quiz", "when": w.to_rfc3339() }), origin(&k), None).await;
    let mut rx = t.state.event_sender.subscribe();

    assert!(schedule::run_once(&t.state, r - Duration::minutes(1)).await.reminded.is_empty(), "not before 19:00");
    assert_eq!(schedule::run_once(&t.state, r + Duration::seconds(30)).await.reminded, vec![code.clone()]);
    assert!(schedule::run_once(&t.state, r + Duration::minutes(2)).await.reminded.is_empty(), "once");
    let again = restart(&t).await;
    assert!(schedule::run_once(&again, r + Duration::minutes(3)).await.reminded.is_empty(), "once across a restart");

    let n = notes(&mut rx);
    assert_eq!(n, vec![("pocket_app".to_string(), "Pub quiz tomorrow".to_string(), "Pub quiz tomorrow at 8 PM. The card is in your chat.".to_string())]);
    let c = cards(&t.state, &k).await;
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].0, "reminder");
    assert_eq!(c[0].1, "Reminder: Pub quiz is tomorrow at 8 PM. The lobby opens at 7:30 PM by itself.");
    assert_eq!(c[0].2["code"], json!(code));
}

#[tokio::test]
async fn no_reminder_after_end_or_delete_or_when_made_too_late() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let w = eight_pm_in(3);
    let r = seven_pm_before(w);
    let ended = make(&t, json!({ "title": "Ended quiz", "when": w.to_rfc3339() }), origin(&k), None).await;
    let deleted = make(&t, json!({ "title": "Deleted quiz", "when": w.to_rfc3339() }), origin(&k), None).await;
    pocket::end(&t.state, &ended).await.unwrap();
    pocket::delete(&t.state, &deleted).await.unwrap();
    assert!(t.state.db.pocket_schedule(&deleted).await.unwrap().is_none(), "deleted with the pocket");
    let mut rx = t.state.event_sender.subscribe();
    let ran = schedule::run_once(&t.state, r + Duration::minutes(1)).await;
    assert!(ran.reminded.is_empty(), "{ran:?}");
    assert!(notes(&mut rx).iter().all(|n| !n.1.contains("tomorrow")));

    // Made less than a day ahead, after 19:00 the evening before: no reminder at all
    let soon = make(&t, json!({ "title": "Soon", "when": (Utc::now() + Duration::hours(2)).to_rfc3339() }), origin(&k), None).await;
    assert_eq!(t.state.db.pocket_schedule(&soon).await.unwrap().unwrap().remind_at, None);
}

#[tokio::test]
async fn a_new_when_moves_the_times_and_re_plans_the_reminder() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let w = eight_pm_in(3);
    let explicit_close = w + Duration::days(2);
    let (s, _) = pocket::create(
        &t.state,
        CreateRequest { kit: "test".into(), spec: json!({ "title": "Pub quiz", "when": w.to_rfc3339() }), origin: origin(&k), closes_at: Some(explicit_close), ..Default::default() },
    )
    .await
    .unwrap();
    let code = s.row.code.clone();
    assert_eq!(schedule::run_once(&t.state, seven_pm_before(w) + Duration::minutes(1)).await.reminded, vec![code.clone()]);

    // A day later, through the person's API
    let w2 = w + Duration::days(1);
    let (status, body) = send(&t.main, "POST", &format!("/api/pockets/{code}/when"), Some(json!({ "when": w2.to_rfc3339() }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let s = pocket::snapshot(&t.state, &code).await.unwrap().unwrap();
    assert_eq!(s.row.opens_at, Some(w2 - Duration::minutes(30)), "the derived opening moved");
    assert_eq!(s.row.closes_at, Some(explicit_close), "the explicit closing stayed");
    assert_eq!(s.spec["when"], json!(w2.to_rfc3339()));
    let row = t.state.db.pocket_schedule(&code).await.unwrap().unwrap();
    assert_eq!((row.remind_at, row.reminded_at), (Some(seven_pm_before(w2)), None), "re-planned");
    assert!(schedule::run_once(&t.state, seven_pm_before(w) + Duration::minutes(5)).await.reminded.is_empty(), "the old time is gone");
    assert_eq!(schedule::run_once(&t.state, seven_pm_before(w2) + Duration::minutes(1)).await.reminded, vec![code.clone()]);

    // The same `when` again keeps the reminder as sent
    schedule::set_when(&t.state, &code, Some(w2.fixed_offset())).await.unwrap();
    assert!(schedule::run_once(&t.state, seven_pm_before(w2) + Duration::minutes(2)).await.reminded.is_empty());
    // A bad `when` is refused; an ended pocket can't move
    let (status, _) = send(&t.main, "POST", &format!("/api/pockets/{code}/when"), Some(json!({ "when": "friday" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    pocket::end(&t.state, &code).await.unwrap();
    let (status, _) = send(&t.main, "POST", &format!("/api/pockets/{code}/when"), Some(json!({ "when": w2.to_rfc3339() }))).await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn results_are_posted_once_across_end_sweep_and_restart() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, json!({ "title": "Pub quiz" }), origin(&k), None).await;
    join(&t, &code, CANARY).await;
    join(&t, &code, "Mia").await;
    let mut rx = t.state.event_sender.subscribe();

    let (status, _) = send(&t.main, "POST", &format!("/api/pockets/{code}/end"), None).await;
    assert_eq!(status, StatusCode::OK);
    let now = Utc::now();
    assert_eq!(schedule::run_once(&t.state, now).await.results, vec![code.clone()]);
    assert!(schedule::run_once(&t.state, now).await.results.is_empty());
    pocket::sweep(&t.state, now).await;
    let again = restart(&t).await;
    assert!(schedule::run_once(&again, now + Duration::minutes(1)).await.results.is_empty(), "once across a restart");

    let c = cards(&t.state, &k).await;
    assert_eq!(c.len(), 1, "{c:?}");
    let (reason, text, meta) = &c[0];
    assert_eq!(reason, "results");
    assert_eq!(text, "Pub quiz has ended. 2 players took part.");
    let block = &meta["block"];
    assert_eq!((block["type"].as_str(), block["state"].as_str(), block["code"].as_str()), (Some("pocket"), Some("ended"), Some(code.as_str())));
    assert_eq!(block["results"]["teams"].as_array().map(Vec::len), Some(2));
    assert!(block["ended"]["winner"].is_object());
    assert_eq!(block["provenance"]["ref"], json!(format!("pocket:{code}")));
    assert!(block["results"]["names_deleted_at"].is_null(), "names stay until the data expires");

    let n = notes(&mut rx);
    assert_eq!(n, vec![("pocket_app".to_string(), "Pub quiz has ended".to_string(), "2 players took part. The results are in your chat.".to_string())]);
    for (_, title, body) in &n {
        assert!(!title.contains("Zyxqv") && !body.contains("Zyxqv"), "a guest's name in a notification");
    }
    assert!(!text.contains("Zyxqv"), "the chat line holds counts only");
}

#[tokio::test]
async fn a_pocket_kept_only_until_it_ends_still_gets_its_results() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, json!({ "title": "Quick quiz", "retention": "at_end" }), origin(&k), None).await;
    join(&t, &code, CANARY).await;
    pocket::end(&t.state, &code).await.unwrap();
    // The listener's order: pa7 first, then the sweep
    let now = Utc::now() + Duration::seconds(1);
    assert_eq!(schedule::run_once(&t.state, now).await.results, vec![code.clone()]);
    assert_eq!(pocket::sweep(&t.state, now).await.deleted, 1);
    let c = cards(&t.state, &k).await;
    assert_eq!(c.len(), 1);
    // Its data goes as it ends, so the names never reach the chat; the team and score do
    let results = &c[0].2["block"]["results"];
    assert_eq!(results["teams"][0]["name"], json!("Team 1"));
    assert_eq!(results["teams"][0]["members"], json!([]));
    assert!(results["names_deleted_at"].is_string());
    assert!(!c[0].2.to_string().contains("Zyxqv"), "a guest's name in the chat after its data was deleted");
    assert_eq!(schedule::run_once(&t.state, now + Duration::minutes(1)).await.names_deleted, 0, "nothing left to blank");
}

#[tokio::test]
async fn guest_names_leave_the_chat_when_the_pocket_expires_or_is_deleted() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let kept = make(&t, json!({ "title": "Pub quiz" }), origin(&k), None).await;
    let deleted = make(&t, json!({ "title": "Other quiz" }), origin(&k), None).await;
    for code in [&kept, &deleted] {
        join(&t, code, CANARY).await;
        join(&t, code, "Mia").await;
        pocket::end(&t.state, code).await.unwrap();
    }
    let now = Utc::now() + Duration::seconds(1);
    assert_eq!(schedule::run_once(&t.state, now).await.results.len(), 2);
    let posted = cards(&t.state, &k).await;
    assert_eq!(posted.len(), 2);
    assert!(posted.iter().all(|(_, _, m)| m.to_string().contains("Zyxqv")), "names are shown while the data is kept");

    // The host deletes one: its names go on the next run
    assert!(pocket::delete(&t.state, &deleted).await.unwrap());
    let mut rx = t.state.event_sender.subscribe();
    assert_eq!(schedule::run_once(&t.state, now + Duration::minutes(1)).await.names_deleted, 1);
    let mut updated = 0;
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::MessageAdded { message } = ev {
            assert!(!message.to_string().contains("Zyxqv"), "{message}");
            updated += 1;
        }
    }
    assert_eq!(updated, 1, "the open chat hears about the change");

    // The other one keeps its names until it expires (the sweep deletes it in the same tick)
    let expires = pocket::snapshot(&t.state, &kept).await.unwrap().unwrap().row.expires_at.expect("kept for a while after it ends");
    assert_eq!(schedule::run_once(&t.state, expires - Duration::minutes(1)).await.names_deleted, 0);
    assert_eq!(schedule::run_once(&t.state, expires + Duration::seconds(1)).await.names_deleted, 1);
    pocket::sweep(&t.state, expires + Duration::seconds(1)).await;
    assert_eq!(schedule::run_once(&t.state, expires + Duration::minutes(1)).await.names_deleted, 0, "once");

    for (_, text, meta) in cards(&t.state, &k).await {
        let results = &meta["block"]["results"];
        assert!(!meta.to_string().contains("Zyxqv") && !meta.to_string().contains("Mia"), "{meta}");
        assert!(results["names_deleted_at"].is_string());
        assert_eq!(results["teams"].as_array().map(Vec::len), Some(2), "the teams and scores stay");
        assert_eq!(results["teams"][0]["name"], json!("Team 1"));
        assert!(results["teams"][0]["points"].is_number());
        assert!(text.ends_with("2 players took part."));
    }
}

#[tokio::test]
async fn the_kit_finishing_or_the_closing_time_ends_the_pocket() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, json!({ "title": "Two rounds", "final_round": 2 }), origin(&k), None).await;
    join(&t, &code, "Mia").await;
    pocket::act(&t.state, &code, &Viewer::Host, "advance", &json!({})).await.unwrap();
    assert_eq!(schedule::run_once(&t.state, Utc::now()).await.ended, 0, "not finished after round 1");
    pocket::act(&t.state, &code, &Viewer::Host, "advance", &json!({})).await.unwrap();
    let ran = schedule::run_once(&t.state, Utc::now()).await;
    assert_eq!((ran.ended, ran.results.clone()), (1, vec![code.clone()]));

    let w = eight_pm_in(2);
    let late = make(&t, json!({ "title": "Friday quiz", "when": w.to_rfc3339() }), origin(&k), None).await;
    let ran = schedule::run_once(&t.state, w + Duration::hours(4) + Duration::minutes(1)).await;
    assert!(ran.results.contains(&late), "{ran:?}");
    assert_eq!(cards(&t.state, &k).await.iter().filter(|c| c.0 == "results").count(), 2);
}

#[tokio::test]
async fn switched_off_nothing_is_sent() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let w = eight_pm_in(3);
    let code = make(&t, json!({ "title": "Pub quiz", "when": w.to_rfc3339() }), origin(&k), None).await;
    let ended = make(&t, json!({ "title": "Done quiz" }), origin(&k), None).await;
    pocket::end(&t.state, &ended).await.unwrap();
    t.state.db.set_setting(pocket::SETTING, "off").await.unwrap();
    let mut rx = t.state.event_sender.subscribe();
    let ran = schedule::run_once(&t.state, seven_pm_before(w) + Duration::minutes(1)).await;
    assert_eq!(ran, schedule::Ran::default());
    assert!(notes(&mut rx).is_empty());
    assert!(cards(&t.state, &k).await.is_empty());
    // On again: both go, once
    t.state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    let ran = schedule::run_once(&t.state, seven_pm_before(w) + Duration::minutes(2)).await;
    assert_eq!((ran.reminded, ran.results), (vec![code], vec![ended]));
}

#[tokio::test]
async fn without_an_origin_only_the_notification_goes() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let w = eight_pm_in(3);
    let code = make(&t, json!({ "title": "Pub quiz", "when": w.to_rfc3339() }), None, None).await;
    let mut rx = t.state.event_sender.subscribe();
    assert_eq!(schedule::run_once(&t.state, seven_pm_before(w) + Duration::minutes(1)).await.reminded, vec![code.clone()]);
    pocket::end(&t.state, &code).await.unwrap();
    assert_eq!(schedule::run_once(&t.state, Utc::now()).await.results, vec![code.clone()]);
    let n = notes(&mut rx);
    assert_eq!(
        n,
        vec![
            ("pocket_app".to_string(), "Pub quiz tomorrow".to_string(), "Pub quiz tomorrow at 8 PM. The lobby opens at 7:30 PM by itself.".to_string()),
            ("pocket_app".to_string(), "Pub quiz has ended".to_string(), "Nobody joined.".to_string()),
        ]
    );
    assert!(cards(&t.state, &k).await.is_empty());
}

/// pa11 qa F3: a pocket that keeps nothing after it ends and finishes by itself is ended by the
/// run's own tick, a moment after the run's `now`; its results still reach the chat without names.
#[tokio::test]
async fn a_keep_nothing_pocket_that_finishes_by_itself_never_shows_names() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, json!({ "title": "Two rounds", "final_round": 2, "retention": "at_end" }), origin(&k), None).await;
    join(&t, &code, CANARY).await;
    pocket::act(&t.state, &code, &Viewer::Host, "advance", &json!({})).await.unwrap();
    pocket::act(&t.state, &code, &Viewer::Host, "advance", &json!({})).await.unwrap();
    // The tick's `now` is taken before the run ends the pocket
    let now = Utc::now();
    let ran = schedule::run_once(&t.state, now).await;
    assert_eq!((ran.ended, ran.results.clone()), (1, vec![code.clone()]));
    let c = cards(&t.state, &k).await;
    assert_eq!(c.len(), 1);
    assert!(!c[0].2.to_string().contains("Zyxqv"), "a guest's name reached the chat for a tick: {}", c[0].2);
    assert!(c[0].2["block"]["results"]["names_deleted_at"].is_string());
    assert_eq!(pocket::sweep(&t.state, now + Duration::minutes(1)).await.deleted, 1);
}

/// pa11 qa F1: an open pocket that keeps nothing after it ends, with no closing time, is ended
/// 12 hours after it opened (results posted without names, then swept). One that keeps its data
/// after it ends, or has its own closing time, is left to that.
#[tokio::test]
async fn a_keep_nothing_pocket_nobody_ends_is_ended_after_12_hours() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let lost = make(&t, json!({ "title": "Lost quiz", "retention": "at_end" }), origin(&k), None).await;
    let kept = make(&t, json!({ "title": "Kept quiz" }), origin(&k), None).await;
    join(&t, &lost, CANARY).await;
    join(&t, &kept, "Mia").await;
    let start = Utc::now();
    assert_eq!(schedule::run_once(&t.state, start + Duration::hours(11) + Duration::minutes(59)).await.ended, 0);
    let now = start + pocket::AT_END_MAX_OPEN + Duration::minutes(1);
    let ran = schedule::run_once(&t.state, now).await;
    assert_eq!((ran.ended, ran.results.clone()), (1, vec![lost.clone()]), "only the keep-nothing pocket");
    assert!(!cards(&t.state, &k).await.iter().any(|c| c.2.to_string().contains("Zyxqv")));
    assert_eq!(pocket::sweep(&t.state, now).await.deleted, 1);
    assert_eq!(t.state.db.pocket_row_counts(&lost).await.unwrap(), [0, 0, 0, 0]);
    assert_eq!(t.state.db.pocket_get(&kept).await.unwrap().unwrap().state, "open");
}
