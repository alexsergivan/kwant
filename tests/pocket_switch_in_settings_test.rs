//! pocket-switch-in-settings (autonomy/pocket-switch-in-settings/brief.md): the person turns
//! pocket apps on from Settings, without touching the database. `POST /api/settings` takes
//! `pocket_apps` and stores its normal form (`off`, `on`, or the sorted kit list); a value that
//! names no kit is refused with a plain 400 and saves nothing. `GET /api/settings` returns the
//! switch and every kit this build has. The guest port starts listening without a restart.

mod quizmaster_support;

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::{Datelike, NaiveDate, Utc};
use kwnt::server::continuity::TurnOrigin;
use quizmaster_support::{setup, tool, tool_names, turn, turn_in, Env};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;

/// The person on this computer, on the main port.
async fn person_on(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    person_on(&e.app, method, uri, body).await
}

async fn save(e: &Env, body: Value) -> (StatusCode, Value) {
    person(e, "POST", "/api/settings", Some(body)).await
}

async fn settings(e: &Env) -> Value {
    let (s, b) = person(e, "GET", "/api/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b
}

/// The raw stored setting, as the listener and every gate read it.
async fn stored(e: &Env) -> Option<String> {
    e.state.db.get_setting(kwnt::pocket::SETTING).await.unwrap()
}

fn error(b: &Value) -> Option<&str> {
    b["error"].as_str()
}

/// The next Dec 18 at least a week away (the kit refuses past dates).
fn gift_day() -> NaiveDate {
    let today = Utc::now().date_naive();
    let this_year = NaiveDate::from_ymd_opt(today.year(), 12, 18).unwrap();
    if this_year - today >= chrono::Duration::days(7) {
        this_year
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, 12, 18).unwrap()
    }
}

fn santa_spec() -> Value {
    let d = gift_day().format("%Y-%m-%d").to_string();
    json!({
        "title": "Office Secret Santa",
        "gift_on": d,
        "reveal": true,
        "reveal_at": format!("{d}T19:00:00+00:00"),
        "people": ["Anna", "Ben", "Chloe", "Dev"],
    })
}

#[tokio::test]
async fn settings_shows_the_switch_off_by_default_and_every_kit() {
    let e = setup(false).await;
    let s = settings(&e).await;
    assert_eq!(s["pocket_apps"], json!("off"), "{s}");
    let kits: Vec<&str> = s["pocket_kits"].as_array().unwrap_or_else(|| panic!("no pocket_kits: {s}")).iter().filter_map(Value::as_str).collect();
    assert_eq!(kits, kwnt::pocket::kits::kit_ids(), "{s}");
    for k in ["quiz", "santa"] {
        assert!(kits.contains(&k), "{k}: {kits:?}");
    }
    assert_eq!(stored(&e).await, None, "GET writes nothing");
}

#[tokio::test]
async fn saving_santa_turns_santa_on_and_keeps_the_quiz_off() {
    let e = setup(false).await;
    let (s, b) = save(&e, json!({ "pocket_apps": "santa" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b, json!({ "status": "saved", "pocket_apps": "santa" }));
    assert_eq!(settings(&e).await["pocket_apps"], json!("santa"));
    assert_eq!(stored(&e).await.as_deref(), Some("santa"));
    let (s, b) = person(&e, "GET", "/api/pockets", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!((&b["enabled"], &b["kits"]), (&json!(true), &json!(["santa"])), "{b}");

    // A Santa from chat works, and the bot is offered no Quizmaster tool
    e.state.db.set_setting("timezone", "Europe/London").await.unwrap();
    let origin = TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    let token = turn_in(&e, &e.bot, "t-settings-santa", origin).await;
    let names = tool_names(&e, &token).await;
    assert!(names.iter().any(|n| n == "santa_create"), "{names:?}");
    for t in ["pocket_create", "pocket_update", "pocket_dry_run", "pocket_open", "pocket_close"] {
        assert!(!names.iter().any(|n| n == t), "{t} offered: {names:?}");
    }
    let (text, err) = tool(&e, &token, "santa_create", json!({ "spec": santa_spec() })).await;
    assert!(!err, "{text}");
    let (_, b) = person(&e, "GET", "/api/pockets", None).await;
    assert!(b["pockets"].as_array().unwrap().iter().any(|p| p["kit"] == json!("santa")), "{b}");

    // The Quiz still answers pocket_apps_off: its create, its drafts API and its tools
    let (s, b) = person(&e, "POST", "/api/pockets", Some(json!({ "kit": "quiz", "spec": {} }))).await;
    assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{b}");
    let (s, b) = person(&e, "GET", "/api/quiz-drafts/qd_whatever1", None).await;
    assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{b}");
    let token = turn(&e, &e.bot, "t-settings-quiz").await;
    let (text, err) = tool(&e, &token, "pocket_create", json!({ "spec": {} })).await;
    assert!(err && text.contains("off"), "{text}");
}

#[tokio::test]
async fn the_value_is_stored_in_its_normal_form() {
    let e = setup(false).await;
    for (raw, want) in [
        (" Santa , Quiz ", "quiz,santa"),
        ("on", "on"),
        ("off", "off"),
        ("santa", "santa"),
        ("", "off"),
        ("ON", "on"),
        ("santa,bogus", "santa"),
        ("quiz,quiz", "quiz"),
    ] {
        let (s, b) = save(&e, json!({ "pocket_apps": raw })).await;
        assert_eq!(s, StatusCode::OK, "{raw:?}: {b}");
        assert_eq!(b["pocket_apps"], json!(want), "{raw:?}: the reply says what took");
        assert_eq!(stored(&e).await.as_deref(), Some(want), "{raw:?}");
        assert_eq!(settings(&e).await["pocket_apps"], json!(want), "{raw:?}");
    }
    // `off` and `` really are off; `on` really is every kit
    save(&e, json!({ "pocket_apps": "" })).await;
    assert!(!kwnt::pocket::enabled(&e.state.db).await);
    save(&e, json!({ "pocket_apps": "on" })).await;
    assert_eq!(kwnt::pocket::switch(&e.state.db).await, kwnt::pocket::Switch::All);
}

#[tokio::test]
async fn a_value_naming_no_kit_is_refused_and_nothing_changes() {
    let e = setup(false).await;
    save(&e, json!({ "pocket_apps": "santa" })).await;
    for raw in ["bogus", "yes", "true", "On,", ","] {
        let (s, b) = save(&e, json!({ "pocket_apps": raw, "location": "Ljubljana" })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{raw:?}: {b}");
        let msg = error(&b).unwrap_or_default();
        assert!(msg.starts_with(&format!("Unknown Pocket app: {}. Known: ", raw.trim())), "{raw:?}: {msg}");
        assert!(msg.contains("quiz") && msg.contains("santa"), "{msg}");
        assert_eq!(stored(&e).await.as_deref(), Some("santa"), "{raw:?}: the switch is as it was");
        assert_eq!(settings(&e).await["location"], json!(""), "{raw:?}: a refused save saves nothing");
    }
    // Not a string: refused by the payload check, nothing changes
    let (s, _) = save(&e, json!({ "pocket_apps": true })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(stored(&e).await.as_deref(), Some("santa"));
}

#[tokio::test]
async fn a_save_without_pocket_apps_leaves_the_switch_alone() {
    let e = setup(false).await;
    save(&e, json!({ "pocket_apps": "santa" })).await;
    let (s, b) = save(&e, json!({ "location": "Ljubljana", "decide_first": false })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b, json!({ "status": "saved" }), "no pocket_apps in the reply when none was sent");
    let got = settings(&e).await;
    assert_eq!((&got["location"], &got["decide_first"], &got["pocket_apps"]), (&json!("Ljubljana"), &json!(false), &json!("santa")), "{got}");
    assert_eq!(stored(&e).await.as_deref(), Some("santa"));
}

/// The guest port follows the saved switch without a restart (its own port 0), and the guest port
/// never answers `/api/settings`.
#[tokio::test]
async fn the_guest_port_listens_after_the_save_without_a_restart() {
    use kwnt::{broker::PermissionBroker, config::Config, db::Database, pocket::listener, router::LayaRouter, server::AppState, tools::ToolRunner};
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let app = kwnt::server::create_router(state.clone());
    assert!(!listener::reconcile(&state).await, "off by default");
    for (value, on) in [("santa", true), ("off", false), ("quiz,santa", true), ("", false)] {
        let (s, b) = person_on(&app, "POST", "/api/settings", Some(json!({ "pocket_apps": value }))).await;
        assert_eq!(s, StatusCode::OK, "{value:?}: {b}");
        assert_eq!(listener::reconcile(&state).await, on, "{value:?}");
        assert_eq!(state.pocket.listener.addr().is_some(), on, "{value:?}");
    }
    let (s, b) = person_on(&app, "POST", "/api/settings", Some(json!({ "pocket_apps": "santa" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = person_on(&app, "GET", "/api/pockets", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!((&b["enabled"], &b["kits"]), (&json!(true), &json!(["santa"])), "{b}");

    // /api/settings stays main-port only
    let guest = kwnt::pocket::guest::router(state.clone());
    for method in ["GET", "POST"] {
        let req = Request::builder().method(method).uri("/api/settings").header("host", "pocket.example.net").header("content-type", "application/json");
        let mut req = req.body(Body::from(json!({ "pocket_apps": "on" }).to_string())).unwrap();
        req.extensions_mut().insert(ConnectInfo("203.0.113.9:50000".parse::<SocketAddr>().unwrap()));
        let r = guest.clone().oneshot(req).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{method} /api/settings on the guest port");
    }
    assert_eq!(state.db.get_setting(kwnt::pocket::SETTING).await.unwrap().as_deref(), Some("santa"));
}
