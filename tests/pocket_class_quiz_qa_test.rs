//! pa11 QA probes (Rigel): gaps and boundaries around the Class Quiz. Tests named `qa_gap_*` are
//! expected RED on 048b76f5 and prove a defect; the rest pin boundaries that hold. Do not merge.
#![allow(dead_code, unused_imports)]

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

// ---- Gap: a class quiz nobody ends is never deleted ----

#[tokio::test]
async fn qa_gap_a_class_quiz_nobody_ends_keeps_its_players_forever() {
    let t = setup().await;
    let p = make(&t, quiz_spec(json!({ "class": true }))).await;
    let code = code_of(&p);
    joined(&t, &code, "Mia").await;
    joined(&t, &code, "Leo").await;
    // The teacher closes the laptop: no End, the quiz never reaches its final phase.
    let later = Utc::now() + Duration::days(30);
    schedule::run_once(&t.state, later).await;
    pocket::sweep(&t.state, later).await;
    let counts = t.state.db.pocket_row_counts(&code).await.unwrap();
    assert_eq!(counts[1], 0, "30 days on, a class quiz that was never ended still holds its players: {counts:?}");
}

// ---- Gap: names the filter should refuse ----

#[tokio::test]
async fn qa_gap_phone_numbers_in_other_digits_and_a_small_at_sign_pass_as_names() {
    let t = setup().await;
    let code = code_of(&make(&t, quiz_spec(json!({ "class": true }))).await);
    for bad in ["０７７００９００１２３", "٠٧٧٠٠٩٠٠١٢٣", "mia﹫school.org"] {
        let r = join(&t, &code, bad).await;
        assert_eq!(r.body["error"].as_str(), Some("name_first_only"), "{bad:?} got through: {} {}", r.status, r.body);
    }
}

// ---- Boundaries that hold ----

#[tokio::test]
async fn qa_end_twice_then_after_the_sweep() {
    let t = setup().await;
    let code = code_of(&make(&t, quiz_spec(json!({ "class": true }))).await);
    joined(&t, &code, "Mia").await;
    let a = send(&t.main, "POST", &format!("/api/pockets/{code}/end"), None, None).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.body);
    let first = person_get(&t, &code).await["pocket"].clone();
    let b = send(&t.main, "POST", &format!("/api/pockets/{code}/end"), None, None).await;
    assert_eq!(b.status, StatusCode::OK, "End twice is fine: {}", b.body);
    let second = person_get(&t, &code).await["pocket"].clone();
    assert_eq!((first["ended_at"].clone(), first["expires_at"].clone()), (second["ended_at"].clone(), second["expires_at"].clone()), "the second End moves nothing");
    // Joining an ended pocket is refused before the sweep
    let r = join(&t, &code, "Late").await;
    assert_eq!(r.status, StatusCode::GONE, "{}", r.body);
    assert_eq!(r.body["error"], "ended");
    let now = Utc::now() + Duration::seconds(1);
    schedule::run_once(&t.state, now).await;
    pocket::sweep(&t.state, now).await;
    // After the sweep every door is a plain 404, and End a third time too
    assert_eq!(send(&t.main, "POST", &format!("/api/pockets/{code}/end"), None, None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(join(&t, &code, "Later").await.status, StatusCode::NOT_FOUND);
    let info = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), None, Some("198.51.200.9")).await;
    assert_eq!(info.status, StatusCode::NOT_FOUND, "{}", info.body);
    let rj = send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(json!({ "name": "Mia" })), Some("198.51.200.10")).await;
    assert_eq!(rj.status, StatusCode::NOT_FOUND, "{}", rj.body);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0]);
}

#[tokio::test]
async fn qa_player_limits_at_60_61_120_121() {
    let t = setup().await;
    // Class quiz asking for exactly 60: full at 61, still offered the quick link
    let c60 = make(&t, quiz_spec(json!({ "class": true, "max_players": 60 }))).await;
    assert_eq!(person_get(&t, &code_of(&c60)).await["join"]["quick_tunnel"], true);
    for i in 0..60 {
        joined(&t, &code_of(&c60), &format!("K{i}")).await;
    }
    assert_eq!(join(&t, &code_of(&c60), "K60").await.body["error"], "full");
    // 61: first size with no quick link
    let c61 = make(&t, quiz_spec(json!({ "class": true, "max_players": 61 }))).await;
    let code61 = code_of(&c61);
    let r = send(&t.main, "POST", &format!("/api/pockets/{code61}/tunnel"), Some(json!({ "on": true })), None).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("needs_stable_address")), "{}", r.body);
    assert_eq!(person_get(&t, &code61).await["join"]["quick_tunnel"], false);
    for i in 0..61 {
        joined(&t, &code61, &format!("K{i}")).await;
    }
    assert_eq!(join(&t, &code61, "K61").await.body["error"], "full");
    // 120 fits and every one is in the game; 121 and 0 are refused when the spec is made
    let c120 = make(&t, quiz_spec(json!({ "class": true, "max_players": 120 }))).await;
    let code120 = code_of(&c120);
    for i in 0..120 {
        joined(&t, &code120, &format!("K{i}")).await;
    }
    let r = join(&t, &code120, "K120").await;
    assert_eq!((r.status, r.body["error"].as_str(), r.body["max_players"].as_u64()), (StatusCode::CONFLICT, Some("full"), Some(120)), "{}", r.body);
    assert_eq!(person_get(&t, &code120).await["summary"]["players"], 120);
    for bad in [121, 0] {
        let r = try_make(&t, quiz_spec(json!({ "class": true, "max_players": bad }))).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_spec")), "{bad}: {}", r.body);
    }
    // class: false with 61 is the normal quiz's limit
    let r = try_make(&t, quiz_spec(json!({ "class": false, "max_players": 61 }))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
}

#[tokio::test]
async fn qa_name_edges_that_hold() {
    let t = setup().await;
    let code = code_of(&make(&t, quiz_spec(json!({ "class": true }))).await);
    // Three words with odd spaces, the fullwidth at sign, six ASCII digits, the 21st character
    for bad in ["Mia\u{a0}Rose\u{a0}Jones", "Mia\u{3000}Rose\u{3000}Jones", "mia＠school", "123456", "+4477009001"] {
        let r = join(&t, &code, bad).await;
        assert_eq!(r.body["error"].as_str(), Some("name_first_only"), "{bad:?}: {} {}", r.status, r.body);
    }
    // Two words and five digits still pass; an exact duplicate is name_taken as ever
    for ok in ["Mary Ann", "12345", "Žofia", "李小龙"] {
        joined(&t, &code, ok).await;
    }
    assert_eq!(join(&t, &code, "mary  ann").await.body["error"], "name_taken");
    // The refusal does not echo the name back
    let r = join(&t, &code, "mia@school.org").await;
    assert!(!r.body.to_string().contains("school.org"), "{}", r.body);
}
