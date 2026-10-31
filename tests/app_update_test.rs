//! The Mac app's in-app update routes: `/api/app/update` reads the state for anyone, check and
//! install are the person's, and off the Mac app (nothing registered) updates are unsupported.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    app_update::{Available, State, UpdateHub, Updater},
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    drivers::RuntimeEvent,
    router::LayaRouter,
    server::{
        app_update_routes,
        continuity::{self, SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

#[derive(Default)]
struct FakeUpdater {
    calls: Mutex<Vec<String>>,
}

impl Updater for FakeUpdater {
    fn check(&self) {
        self.calls.lock().unwrap().push("check".into());
    }
    fn install(&self, version: &str) {
        self.calls.lock().unwrap().push(format!("install {version}"));
    }
}

struct Env {
    app: Router,
    state: AppState,
    _tmp: TempDir,
}

/// A server with its own hub (not the process-wide one), with or without the Mac updater.
async fn setup(updater: Option<Arc<FakeUpdater>>) -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let hub = Arc::new(UpdateHub::new());
    if let Some(u) = updater {
        hub.register("0.1.1", u);
    }
    state.app_update = hub;
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let resp = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A bot's turn token in #general.
async fn bot_token(e: &Env) -> String {
    let space = SpaceId::default_space();
    let bot = e.state.db.list_kwants(&space).await.unwrap().remove(0);
    let general = e.state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(general.id.clone()), ..Default::default() }),
        turn_id: Some("t-update".into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, &bot, Some("t-update".into()), None, Some(scope)).await.unwrap().1
}

fn release(version: &str) -> Available {
    Available {
        version: version.into(),
        notes: "### Added\n- In-app updates".into(),
        date: Some("2026-10-27T10:00:00Z".into()),
        url: format!("https://github.com/alexsergivan/kwant/releases/tag/mac-v{version}"),
    }
}

#[tokio::test]
async fn unsupported_off_the_mac_app() {
    let e = setup(None).await;
    let (status, v) = call(&e.app, "GET", "/api/app/update", None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["supported"], false);
    assert_eq!(v["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["state"], "idle");
    assert!(v["available"].is_null());
    assert!(v["progress"].is_null());
    assert!(v["error"].is_null());
    for uri in ["/api/app/update/check", "/api/app/update/install"] {
        let (status, v) = call(&e.app, "POST", uri, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{uri}: {v}");
        assert!(v["error"].as_str().unwrap().contains("Mac app"), "{v}");
    }
}

#[tokio::test]
async fn bots_read_but_never_check_or_install() {
    let fake = Arc::new(FakeUpdater::default());
    let e = setup(Some(fake.clone())).await;
    e.state.app_update.check().unwrap();
    e.state.app_update.found(Some(release("0.1.2")));
    let token = bot_token(&e).await;

    let (status, v) = call(&e.app, "GET", "/api/app/update", Some(&token)).await;
    assert_eq!(status, StatusCode::OK, "a bot reads the state: {v}");
    assert_eq!(v["state"], "available");
    for uri in ["/api/app/update/check", "/api/app/update/install"] {
        let (status, v) = call(&e.app, "POST", uri, Some(&token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} with a bot's turn token: {v}");
    }
    assert_eq!(*fake.calls.lock().unwrap(), vec!["check"], "only the direct check above ran");
    assert_eq!(e.state.app_update.status().state, State::Available, "nothing changed");

    // From another machine with no token: 401
    let req = Request::builder()
        .method("POST")
        .uri("/api/app/update/install")
        .header("host", "kwnt.example.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(e.app.clone().oneshot(req).await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn person_checks_and_installs() {
    let fake = Arc::new(FakeUpdater::default());
    let e = setup(Some(fake.clone())).await;
    let person = e.state.api_token.reveal().to_string();

    let (status, v) = call(&e.app, "POST", "/api/app/update/install", Some(&person)).await;
    assert_eq!(status, StatusCode::CONFLICT, "nothing found yet: {v}");

    let (status, v) = call(&e.app, "POST", "/api/app/update/check", Some(&person)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["state"], "checking");
    assert_eq!(v["supported"], true);
    assert_eq!(v["current"], "0.1.1");
    let (status, _) = call(&e.app, "POST", "/api/app/update/check", Some(&person)).await;
    assert_eq!(status, StatusCode::CONFLICT, "one check at a time");

    e.state.app_update.found(Some(release("0.2.0")));
    let (_, v) = call(&e.app, "GET", "/api/app/update", None).await;
    assert_eq!(v["state"], "available");
    assert_eq!(v["available"]["version"], "0.2.0");
    assert_eq!(v["available"]["notes"], "### Added\n- In-app updates");

    // The person's cookie from another machine works too
    let req = Request::builder()
        .method("POST")
        .uri("/api/app/update/install")
        .header("host", "kwnt.example.com")
        .header("cookie", format!("kwnt_token={person}"))
        .body(Body::empty())
        .unwrap();
    let resp = e.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(v["state"], "downloading");
    assert_eq!(v["progress"], json!(0.0));
    assert_eq!(*fake.calls.lock().unwrap(), vec!["check", "install 0.2.0"]);

    // A failed download leaves the old app running and the version on offer
    e.state.app_update.failed("signature check failed");
    let (_, v) = call(&e.app, "GET", "/api/app/update", None).await;
    assert_eq!(v["state"], "error");
    assert_eq!(v["error"], "signature check failed");
    assert_eq!(v["available"]["version"], "0.2.0");
}

#[tokio::test]
async fn notifies_once_per_version() {
    let e = setup(Some(Arc::new(FakeUpdater::default()))).await;
    let mut events = e.state.event_sender.subscribe();
    app_update_routes::spawn_notifier(e.state.clone());

    // Everything sent within `wait` (the notifier writes a setting first, which is slow under load)
    async fn notifications(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>, wait: Duration) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let deadline = tokio::time::Instant::now() + wait;
        while let Ok(Ok(ev)) = tokio::time::timeout_at(deadline, events.recv()).await {
            if let RuntimeEvent::Notify { kind, title, body, .. } = ev {
                out.push((kind, title, body));
                break;
            }
        }
        out
    }
    let hub = e.state.app_update.clone();
    let (quiet, wait) = (Duration::from_millis(500), Duration::from_secs(10));

    hub.check().unwrap();
    hub.found(None);
    assert!(notifications(&mut events, quiet).await.is_empty(), "nothing new, nothing sent");

    hub.check().unwrap();
    hub.found(Some(release("0.1.2")));
    let sent = notifications(&mut events, wait).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, "app_update");
    assert_eq!(sent[0].1, "kwant bots 0.1.2 is ready");
    assert_eq!(sent[0].2, "Open kwant bots to update and restart.");

    // The 6-hourly check finds the same version again: no second notification
    hub.check().unwrap();
    hub.found(Some(release("0.1.2")));
    assert!(notifications(&mut events, quiet).await.is_empty());
    assert!(!app_update_routes::notify_once(&e.state, "0.1.2").await, "remembered in settings");

    hub.check().unwrap();
    hub.found(Some(release("0.1.3")));
    assert_eq!(notifications(&mut events, wait).await.len(), 1, "a newer version is announced");
    assert_eq!(
        e.state.db.get_setting(app_update_routes::NOTIFIED_SETTING).await.unwrap().as_deref(),
        Some("0.1.3")
    );
}

/// Whether `notify_once` for a fresh version reaches the UI.
async fn update_notification_delivered(e: &Env, version: &str) -> bool {
    let mut events = e.state.event_sender.subscribe();
    assert!(app_update_routes::notify_once(&e.state, version).await);
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, RuntimeEvent::Notify { ref kind, .. } if kind == "app_update") {
            return true;
        }
    }
    false
}

fn kinds(list: &[&str]) -> Vec<String> {
    list.iter().map(|k| k.to_string()).collect()
}

#[tokio::test]
async fn app_update_and_work_ahead_are_on_for_lists_saved_before_they_existed() {
    let e = setup(Some(Arc::new(FakeUpdater::default()))).await;
    // What a person who saved their notification settings before #78 has stored
    e.state.db.set_setting("notify_on", r#"["turn_finished","approval_needed"]"#).await.unwrap();
    let on = continuity::notify_on(&e.state).await;
    assert_eq!(on, kinds(&["turn_finished", "approval_needed", "app_update", "work_ahead", "automation_suggested", "tier_proposed"]));
    assert!(update_notification_delivered(&e, "0.1.2").await);
    let (_, s) = call(&e.app, "GET", "/api/settings", None).await;
    assert!(s["notify_on"].as_array().unwrap().contains(&json!("app_update")), "the switch shows on: {s}");
}

#[tokio::test]
async fn an_explicit_untick_sticks_and_new_kinds_stay_on() {
    let e = setup(Some(Arc::new(FakeUpdater::default()))).await;
    e.state.db.set_setting("notify_on", r#"["turn_finished"]"#).await.unwrap();
    // The Mac app's settings page offers app_update; the person unticks it
    continuity::save_notify_settings(&e.state, None, Some(kinds(&["turn_finished", "question_asked"]))).await.unwrap();
    assert_eq!(continuity::notify_on(&e.state).await, kinds(&["turn_finished", "question_asked"]));
    assert!(!update_notification_delivered(&e, "0.1.2").await);
    // A later save that doesn't touch notify_on keeps it off
    continuity::save_notify_settings(&e.state, Some(String::new()), None).await.unwrap();
    assert!(!update_notification_delivered(&e, "0.1.3").await);
    // Kinds this list was chosen from are remembered, so only kinds added later default on
    let known: Vec<String> =
        serde_json::from_str(&e.state.db.get_setting(continuity::NOTIFY_KNOWN_SETTING).await.unwrap().unwrap()).unwrap();
    assert!(known.contains(&"app_update".to_string()));
    let all_but_update: Vec<String> =
        continuity::NOTIFY_KINDS.iter().filter(|k| **k != "app_update").map(|k| k.to_string()).collect();
    assert_eq!(
        continuity::effective_kinds(&kinds(&["turn_finished"]), &kinds(&["turn_finished", "app_update"])),
        all_but_update
    );
}

#[tokio::test]
async fn saving_off_the_mac_app_keeps_the_app_update_choice() {
    // Off the Mac app the settings page leaves app_update out of the save (#79)
    let e = setup(None).await;
    continuity::save_notify_settings(&e.state, None, Some(kinds(&["turn_finished"]))).await.unwrap();
    assert_eq!(continuity::notify_on(&e.state).await, kinds(&["turn_finished", "app_update"]), "default on survives");

    // Turned off in the Mac app, then saved from the VPS: still off
    e.state.db.set_setting("notify_on", r#"["turn_finished"]"#).await.unwrap();
    let all = serde_json::to_string(&continuity::NOTIFY_KINDS).unwrap();
    e.state.db.set_setting(continuity::NOTIFY_KNOWN_SETTING, &all).await.unwrap();
    assert_eq!(continuity::notify_on(&e.state).await, kinds(&["turn_finished"]));
    continuity::save_notify_settings(&e.state, None, Some(kinds(&["turn_finished", "approval_needed"]))).await.unwrap();
    assert_eq!(continuity::notify_on(&e.state).await, kinds(&["turn_finished", "approval_needed"]));
    // and a client there can't switch it on either
    continuity::save_notify_settings(&e.state, None, Some(kinds(&["turn_finished", "app_update"]))).await.unwrap();
    assert_eq!(continuity::notify_on(&e.state).await, kinds(&["turn_finished"]));
}
