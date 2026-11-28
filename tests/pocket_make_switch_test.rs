//! pa12 L1 (autonomy/pocket-apps/pa12/L1/brief.md, security-review.md G10 and T17): "Make your
//! own" is off twice. The `made` kit (screens a bot wrote) needs `pocket_make=on` as well as
//! `pocket_apps` `on` or a list naming `made`; `pocket_apps=on` alone never turns it on. A missing,
//! unreadable or mistyped `pocket_make` is off. While `made` is off the guest port answers a made
//! pocket like a code that doesn't exist, and turning it off again hides existing made pockets.
//! `made` isn't a registered kit until L2, so the made pockets here are rows put in directly.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{pocket::NewPocket, Database},
    pocket::{self, guest, keys, kits::MADE_ID, listener, Switch},
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
    T { main: create_router(state.clone()), guest: guest::router(state.clone()), state, _tmp: tmp }
}

async fn set(t: &T, key: &str, value: &str) {
    t.state.db.set_setting(key, value).await.unwrap();
}

/// `pocket_apps` and `pocket_make` (`None`: never set, so those cases come first), then the
/// listener's own check.
async fn switches(t: &T, apps: &str, make: Option<&str>) {
    set(t, pocket::SETTING, apps).await;
    match make {
        Some(m) => set(t, pocket::MAKE_SETTING, m).await,
        None => assert_eq!(t.state.db.get_setting(pocket::MAKE_SETTING).await.unwrap(), None, "a None case after pocket_make was set"),
    }
    listener::reconcile(&t.state).await;
}

/// The person on this computer on the main port, or a guest through the guest port's public name.
async fn send(app: &Router, local: bool, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let host = if local { "127.0.0.1:8799" } else { "pocket.example.net" };
    let req = Request::builder().method(method).uri(uri).header("host", host).header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    if local {
        req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn pockets(t: &T) -> Value {
    let (s, b) = send(&t.main, true, "GET", "/api/pockets", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b
}

fn kits(b: &Value) -> Vec<String> {
    b["kits"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect()
}

/// A pocket row of `kit`, as a made pocket will be once L2 makes them.
async fn put_pocket(t: &T, kit: &str) -> String {
    let code = keys::new_code();
    let now = Utc::now();
    t.state
        .db
        .pocket_insert(NewPocket {
            code: &code,
            kit,
            title: "Book shelf",
            spec: "{}",
            kit_state: "{}",
            owner_kwant_id: None,
            opens_at: None,
            closes_at: None,
            expires_at: None,
            host_key_hash: "not-a-real-hash",
            now,
        })
        .await
        .unwrap();
    code
}

/// What the guest port answers on each pocket route for `code`.
async fn guest_statuses(t: &T, code: &str) -> Vec<(String, StatusCode)> {
    let mut out = Vec::new();
    for (method, path, body) in [
        ("GET", format!("/p/{code}"), None),
        ("GET", format!("/api/pocket/{code}/info"), None),
        ("GET", format!("/api/pocket/{code}/state"), None),
        ("POST", format!("/api/pocket/{code}/join"), Some(json!({ "name": "Ana" }))),
        ("GET", format!("/api/pocket/{code}/host/state"), None),
    ] {
        let (s, _) = send(&t.guest, false, method, &path, body).await;
        out.push((path, s));
    }
    out
}

#[tokio::test]
async fn pocket_apps_on_alone_never_turns_on_made() {
    let t = setup().await;
    // Never set, and every value that isn't exactly `on`
    for make in [None, Some("off"), Some(""), Some("ON"), Some("On"), Some("yes"), Some("true"), Some("1"), Some("on,made")] {
        switches(&t, "on", make).await;
        let sw = pocket::switch(&t.state.db).await;
        assert!(!sw.allows(MADE_ID), "pocket_make {make:?}: {sw:?}");
        assert!(!pocket::enabled_for(&t.state.db, MADE_ID).await, "pocket_make {make:?}");
        for kit in pocket::kits::kit_ids() {
            assert!(sw.allows(kit), "pocket_apps=on still turns on {kit} (pocket_make {make:?})");
        }
        let b = pockets(&t).await;
        assert_eq!(b["enabled"], true, "{b}");
        assert_eq!(b["make"], false, "pocket_make {make:?}: {b}");
        assert!(!kits(&b).contains(&MADE_ID.to_string()), "pocket_make {make:?}: {b}");
    }
    // The pure parse fails closed the same way
    assert!(!Switch::parse("on").allows(MADE_ID));
    assert!(!Switch::parse_with("on", false).allows(MADE_ID));
    assert!(!Switch::parse("made").any(), "a list naming only made is off while pocket_make is off");
}

#[tokio::test]
async fn both_switches_turn_on_made_and_nothing_less_does() {
    let t = setup().await;
    // (pocket_apps, pocket_make, made on, any kit on)
    for (apps, make, made, any) in [
        ("made", None, false, false),
        ("santa,made", None, false, true),
        ("on", Some("on"), true, true),
        (" on ", Some(" on "), true, true),
        ("santa,made", Some("on"), true, true),
        ("MADE", Some("on"), true, true),
        ("made", Some("off"), false, false),
        ("santa", Some("on"), false, true),
        ("off", Some("on"), false, false),
        ("", Some("on"), false, false),
    ] {
        switches(&t, apps, make).await;
        let sw = pocket::switch(&t.state.db).await;
        assert_eq!(sw.allows(MADE_ID), made, "{apps:?} + {make:?}: {sw:?}");
        assert_eq!(sw.any(), any, "{apps:?} + {make:?}: {sw:?}");
        assert_eq!(pocket::enabled(&t.state.db).await, any, "{apps:?} + {make:?}");
        let b = pockets(&t).await;
        assert_eq!(b["make"], made, "{apps:?} + {make:?}: {b}");
        assert_eq!(b["enabled"], any, "{apps:?} + {make:?}: {b}");
        // Only kits this build can make are listed: `made` waits for L2's kit
        for k in kits(&b) {
            assert!(pocket::kits::kit(&k).is_some(), "{apps:?} lists {k}: {b}");
        }
        if apps.contains("santa") {
            assert!(sw.allows("santa"), "{apps:?} + {make:?}: santa stays on");
        }
    }
    // pocket_make on doesn't widen a list that names other kits
    switches(&t, "santa", Some("on")).await;
    assert!(!pocket::switch(&t.state.db).await.allows("quiz"));
}

async fn get(t: &T) -> (StatusCode, Value) {
    send(&t.main, true, "GET", "/api/settings", None).await
}

async fn save(t: &T, body: Value) -> (StatusCode, Value) {
    send(&t.main, true, "POST", "/api/settings", Some(body)).await
}

#[tokio::test]
async fn settings_store_and_show_pocket_make() {
    let t = setup().await;
    let (s, b) = get(&t).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["pocket_make"], "off", "off by default: {b}");

    let (s, b) = save(&t, json!({ "pocket_make": " On " })).await;
    assert_eq!((s, b["pocket_make"].clone()), (StatusCode::OK, json!("on")), "{b}");
    assert_eq!(t.state.db.get_setting(pocket::MAKE_SETTING).await.unwrap().as_deref(), Some("on"));
    assert_eq!(get(&t).await.1["pocket_make"], "on");

    // A typo is refused and saves nothing
    for bad in ["maybe", "yes", "true", "on,off"] {
        let (s, b) = save(&t, json!({ "pocket_make": bad })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad:?}: {b}");
        assert!(b["error"].as_str().unwrap_or("").contains("pocket_make"), "{bad:?}: {b}");
        assert_eq!(t.state.db.get_setting(pocket::MAKE_SETTING).await.unwrap().as_deref(), Some("on"), "{bad:?}");
    }
    let (s, b) = save(&t, json!({ "pocket_make": "x".repeat(500) })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(b["error"].as_str().unwrap().len() < 200, "a long value isn't echoed whole: {b}");

    let (s, b) = save(&t, json!({ "pocket_make": "" })).await;
    assert_eq!((s, b["pocket_make"].clone()), (StatusCode::OK, json!("off")), "{b}");
    assert_eq!(get(&t).await.1["pocket_make"], "off");

    // pocket_apps keeps `made` in its list whatever pocket_make says; both save together
    let (s, b) = save(&t, json!({ "pocket_apps": "Made, santa" })).await;
    assert_eq!((s, b["pocket_apps"].clone()), (StatusCode::OK, json!("made,santa")), "{b}");
    let (_, g) = get(&t).await;
    assert_eq!((g["pocket_apps"].clone(), g["pocket_make"].clone()), (json!("made,santa"), json!("off")), "{g}");
    assert!(!pocket::switch(&t.state.db).await.allows(MADE_ID));
    let (s, b) = save(&t, json!({ "pocket_apps": "on", "pocket_make": "on" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!((b["pocket_apps"].clone(), b["pocket_make"].clone()), (json!("on"), json!("on")), "{b}");
    assert!(pocket::switch(&t.state.db).await.allows(MADE_ID));
}

#[tokio::test]
async fn the_guest_port_hides_made_pockets_while_made_is_off() {
    let t = setup().await;
    switches(&t, "on", None).await;
    let made = put_pocket(&t, MADE_ID).await;
    let other = put_pocket(&t, "test").await;

    // Off: every route of the made pocket is 404, as for a code that doesn't exist
    let missing = keys::new_code();
    for ((path, got), (_, want)) in guest_statuses(&t, &made).await.into_iter().zip(guest_statuses(&t, &missing).await) {
        assert_eq!(got, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(got, want, "{path} answers like a missing code");
    }
    let (s, b) = send(&t.guest, false, "GET", &format!("/api/pocket/{made}/info"), None).await;
    assert_eq!((s, b["error"].clone()), (StatusCode::NOT_FOUND, json!("not_found")), "{b}");
    // Another kit's pocket is untouched
    let (s, b) = send(&t.guest, false, "GET", &format!("/api/pocket/{other}/info"), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Both on: the made pocket answers
    switches(&t, "on", Some("on")).await;
    let (s, b) = send(&t.guest, false, "GET", &format!("/api/pocket/{made}/info"), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["pocket"]["kit"], MADE_ID, "{b}");

    // pocket_make on but pocket_apps not naming made: hidden again
    switches(&t, "santa,test", Some("on")).await;
    let (s, _) = send(&t.guest, false, "GET", &format!("/api/pocket/{made}/info"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, b) = send(&t.guest, false, "GET", &format!("/api/pocket/{other}/info"), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Turning pocket_make off again hides the existing made pocket
    switches(&t, "on", Some("on")).await;
    assert_eq!(send(&t.guest, false, "GET", &format!("/api/pocket/{made}/info"), None).await.0, StatusCode::OK);
    switches(&t, "on", Some("off")).await;
    for (path, got) in guest_statuses(&t, &made).await {
        assert_eq!(got, StatusCode::NOT_FOUND, "{path} after pocket_make went off");
    }
    assert!(pocket::hidden(&t.state, &made).await);
    assert!(!pocket::hidden(&t.state, &other).await);
}
