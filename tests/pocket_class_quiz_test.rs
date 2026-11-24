//! pa11 first slice: the Class Quiz (plan section 8 rule 9). A quiz with `settings.class: true`
//! keeps nothing once it ends (deleted at End, not 7 days later), takes first names or nicknames
//! only, may take more than 60 players only on an address that stays the same (never a quick
//! link), and says so in plain words before anyone types. The normal quiz is unchanged. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa11/brief.md ("Tests").

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    pocket::{self, guest, listener, schedule},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const LOCAL: &str = "127.0.0.1:50000";
const GHOST: &str = "pocket.example.net";
/// The words the class quiz shows before anyone types (Mira checks the copy).
const CLASS_LINE: &str = "First names or nicknames only. Everything about players is deleted when the quiz ends. Nothing is shared.";

struct T {
    main: Router,
    guest: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup() -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), guest: guest::router(state.clone()), state, _tmp: tmp }
}

struct Res {
    status: StatusCode,
    body: Value,
}

/// `peer: true` is the person on this computer (main port); otherwise a guest from `ip`.
async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>, ip: Option<&str>) -> Res {
    let host = if ip.is_none() { "127.0.0.1:8799" } else { GHOST };
    let mut req = Request::builder().method(method).uri(uri).header("host", host).header("content-type", "application/json");
    if let Some(ip) = ip {
        req = req.header("x-forwarded-for", ip);
    }
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    if ip.is_none() {
        req.extensions_mut().insert(ConnectInfo(LOCAL.parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Res { status, body: serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())) }
}

/// The Quiz kit's example, solo, with no start time (so it is open now), and `settings` merged in.
fn quiz_spec(settings: Value) -> Value {
    let mut v: Value = serde_json::from_str(kwnt::kits::quiz::EXAMPLE).unwrap();
    v.as_object_mut().unwrap().remove("when");
    v["teams"] = json!({ "mode": "solo" });
    for (k, val) in settings.as_object().unwrap() {
        v["settings"][k] = val.clone();
    }
    v
}

async fn try_make(t: &T, spec: Value) -> Res {
    send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "quiz", "spec": spec })), None).await
}

async fn make(t: &T, spec: Value) -> Value {
    let r = try_make(t, spec).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body["pocket"].clone()
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

async fn join(t: &T, code: &str, name: &str) -> Res {
    let ip = fresh_ip();
    send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": name })), Some(ip.as_str())).await
}

async fn joined(t: &T, code: &str, name: &str) {
    let r = join(t, code, name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{name}: {}", r.body);
}

async fn person_get(t: &T, code: &str) -> Value {
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

fn at(v: &Value) -> DateTime<Utc> {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v}: {e}"))
}

fn code_of(p: &Value) -> String {
    p["code"].as_str().unwrap().to_string()
}

// ---- Retention ----

#[tokio::test]
async fn a_class_quiz_keeps_nothing_once_it_ends() {
    let t = setup().await;
    let p = make(&t, quiz_spec(json!({ "class": true }))).await;
    assert_eq!(p["retention"], json!({ "kind": "at_end" }), "{p}");
    let code = code_of(&p);
    joined(&t, &code, "Mia").await;
    joined(&t, &code, "Leo").await;
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap()[1], 2);

    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/end"), None, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let p = person_get(&t, &code).await["pocket"].clone();
    assert_eq!(p["state"], "ended");
    assert_eq!(at(&p["expires_at"]), at(&p["ended_at"]), "the data goes at the end, not 7 days later: {p}");

    // The minute tick: results first (no names, see below), then the sweep deletes everything
    let now = Utc::now() + Duration::seconds(1);
    schedule::run_once(&t.state, now).await;
    assert_eq!(pocket::sweep(&t.state, now).await.deleted, 1);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0], "players, answers, scores, keys and events are gone");
    assert_eq!(send(&t.main, "GET", &format!("/api/pockets/{code}"), None, None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_normal_quiz_still_keeps_its_data_seven_days() {
    let t = setup().await;
    let p = make(&t, quiz_spec(json!({}))).await;
    assert_eq!(p["retention"], json!({ "kind": "after_end", "days": 7 }), "{p}");
    let off = make(&t, quiz_spec(json!({ "class": false }))).await;
    assert_eq!(off["retention"], json!({ "kind": "after_end", "days": 7 }), "class: false is the normal quiz");
    let code = code_of(&p);
    joined(&t, &code, "Mia Rose Jones").await;
    pocket::end(&t.state, &code).await.unwrap();
    let p = person_get(&t, &code).await["pocket"].clone();
    assert_eq!(at(&p["expires_at"]), at(&p["ended_at"]) + Duration::days(7), "{p}");
    let now = Utc::now() + Duration::seconds(1);
    schedule::run_once(&t.state, now).await;
    assert_eq!(pocket::sweep(&t.state, now + Duration::days(6)).await.deleted, 0, "kept for the week");
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap()[1], 1);
}

// ---- Names ----

#[tokio::test]
async fn a_class_quiz_takes_first_names_or_nicknames_only() {
    let t = setup().await;
    let code = code_of(&make(&t, quiz_spec(json!({ "class": true }))).await);
    for ok in ["Mia", "Big Sam", "Zoë", "Leo B", "Kid2012"] {
        joined(&t, &code, ok).await;
    }
    for bad in ["Mia Rose Jones", "mia@school.org", "mia.jones@gmail", "07700 900123", "+44 7700 900123", "Mia 0770090012"] {
        let r = join(&t, &code, bad).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("name_first_only")), "{bad}: {}", r.body);
        let m = r.body["message"].as_str().unwrap();
        assert!(m.contains("First name or nickname"), "a plain line: {m}");
    }
    // Still at most 20 characters (the shared name rule)
    let r = join(&t, &code, "Maximilianus-Theodor").await;
    assert_eq!(r.status, StatusCode::CREATED, "20 is fine: {}", r.body);
    let r = join(&t, &code, "Maximilianus-Theodore").await;
    assert_eq!(r.body["error"], "name_too_long", "{}", r.body);
    // The normal quiz takes any name the word filter lets through
    let pub_quiz = code_of(&make(&t, quiz_spec(json!({}))).await);
    joined(&t, &pub_quiz, "Mia Rose Jones").await;
}

// ---- More than 60 ----

#[tokio::test]
async fn more_than_sixty_only_for_a_class_quiz_and_never_on_a_quick_link() {
    let t = setup().await;
    // The normal quiz stays at 60, and so does a class quiz that doesn't ask for more
    let r = try_make(&t, quiz_spec(json!({ "max_players": 61 }))).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_spec")), "{}", r.body);
    let plain = make(&t, quiz_spec(json!({ "class": true }))).await;
    assert_eq!(plain["max_players"], 60, "{plain}");
    let r = try_make(&t, quiz_spec(json!({ "class": true, "max_players": 121 }))).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_spec")), "{}", r.body);
    assert!(r.body["message"].as_str().unwrap().contains("120"), "{}", r.body);

    let big = make(&t, quiz_spec(json!({ "class": true, "max_players": 70 }))).await;
    assert_eq!(big["max_players"], 70, "{big}");
    let code = code_of(&big);
    // No quick link: it needs an address that stays the same (own https, Funnel or Same Wi-Fi)
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/tunnel"), Some(json!({ "on": true })), None).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("needs_stable_address")), "{}", r.body);
    assert!(r.body["message"].as_str().unwrap().contains("60"), "says why in one line: {}", r.body);
    assert_eq!(person_get(&t, &code).await["join"]["quick_tunnel"], false);
    // A class quiz of 60 or fewer still gets the quick link offered
    assert_eq!(person_get(&t, &code_of(&plain)).await["join"]["quick_tunnel"], true);

    for i in 0..70 {
        joined(&t, &code, &format!("Kid{i}")).await;
    }
    let r = join(&t, &code, "Kid70").await;
    assert_eq!((r.status, r.body["error"].as_str(), r.body["max_players"].as_u64()), (StatusCode::CONFLICT, Some("full"), Some(70)), "{}", r.body);
    // Every one of the 70 is in the game, not only in the pocket
    let host = person_get(&t, &code).await;
    assert_eq!(host["summary"]["players"], 70, "{}", host["summary"]);
}

// ---- Plain words ----

#[tokio::test]
async fn the_class_line_is_there_before_anyone_types() {
    let t = setup().await;
    let class = code_of(&make(&t, quiz_spec(json!({ "class": true }))).await);
    let info = send(&t.guest, "GET", &format!("/api/pocket/{class}/info"), None, Some("198.51.200.1")).await;
    assert_eq!(info.status, StatusCode::OK, "{}", info.body);
    assert_eq!(info.body["pocket"]["notice"], CLASS_LINE, "the guest page: {}", info.body);
    assert_eq!(info.body["pocket"]["first_names_only"], true);
    let card = person_get(&t, &class).await;
    assert_eq!(card["pocket"]["notice"], CLASS_LINE, "the kit page / card: {}", card["pocket"]);
    let normal = code_of(&make(&t, quiz_spec(json!({}))).await);
    let info = send(&t.guest, "GET", &format!("/api/pocket/{normal}/info"), None, Some("198.51.200.2")).await;
    assert!(info.body["pocket"]["notice"].is_null(), "{}", info.body);
    assert_eq!(info.body["pocket"]["first_names_only"], false);
}

// ---- QA round 1 (Rigel, qa.md F1 and F2) ----

/// F1: the teacher closes the laptop. No End, the quiz never reaches its final phase, and still
/// everything about the players goes (12 h after it opened; Rigel's probe, #1148).
#[tokio::test]
async fn a_class_quiz_nobody_ends_is_ended_and_deleted() {
    let t = setup().await;
    let p = make(&t, quiz_spec(json!({ "class": true }))).await;
    let code = code_of(&p);
    joined(&t, &code, "Mia").await;
    joined(&t, &code, "Leo").await;
    // Not before the 12 hours
    let early = Utc::now() + Duration::hours(11);
    assert_eq!(schedule::run_once(&t.state, early).await.ended, 0);
    assert_eq!(pocket::sweep(&t.state, early).await, pocket::Swept::default());
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap()[1], 2, "still running at 11 h");
    let later = Utc::now() + Duration::days(30);
    assert_eq!(schedule::run_once(&t.state, later).await.ended, 1);
    assert_eq!(pocket::sweep(&t.state, later).await.deleted, 1);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0], "30 days on, nothing about the players is left");
    assert_eq!(send(&t.main, "GET", &format!("/api/pockets/{code}"), None, None).await.status, StatusCode::NOT_FOUND);
}

/// F1's other side: the backstop is for pockets that keep nothing. A normal quiz nobody ends stays
/// open, as before.
#[tokio::test]
async fn a_normal_quiz_nobody_ends_is_left_alone() {
    let t = setup().await;
    let code = code_of(&make(&t, quiz_spec(json!({}))).await);
    joined(&t, &code, "Mia Rose Jones").await;
    let later = Utc::now() + Duration::days(30);
    assert_eq!(schedule::run_once(&t.state, later).await.ended, 0);
    assert_eq!(pocket::sweep(&t.state, later).await, pocket::Swept::default());
    assert_eq!(person_get(&t, &code).await["pocket"]["state"], "open");
}

/// F2: phone numbers in other digits and the small at sign are refused like their ASCII forms.
#[tokio::test]
async fn phone_numbers_in_other_digits_and_a_small_at_sign_are_refused() {
    let t = setup().await;
    let code = code_of(&make(&t, quiz_spec(json!({ "class": true }))).await);
    for bad in ["０７７００９００１２３", "٠٧٧٠٠٩٠٠١٢٣", "mia﹫school.org"] {
        let r = join(&t, &code, bad).await;
        assert_eq!(r.body["error"].as_str(), Some("name_first_only"), "{bad:?} got through: {} {}", r.status, r.body);
    }
}
