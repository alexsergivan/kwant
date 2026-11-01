//! The Concierge in #general: a message no specialist fits goes to it (made on the first one,
//! with a card the person can undo), a clear specialist message doesn't, and once the person
//! removes it, it is never made again. A fake claude CLI answers turns.
//! Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
    RuntimeEvent,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
echo '{"type":"result","is_error":false,"num_turns":1,"result":"answer ok"}'
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

async fn setup() -> (Router, AppState, tempfile::TempDir) {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state, tmp)
}

fn specialist(name: &str, title: &str, persona: &str) -> Value {
    json!({"name": name, "title": title, "persona_prompt": persona,
        "avatar_sprite": "s", "driver_type": "api", "preferred_model": "claude"})
}

/// Sends `prompt` to the channel and returns who it was routed to and how, once the turn ended.
async fn routed_to(app: &Router, state: &AppState, channel: &str, prompt: &str) -> (String, Option<String>) {
    let mut events = state.event_sender.subscribe();
    ok(app, "POST", "/api/chat/send", Some(json!({"channel_id": channel, "prompt": prompt, "sender_id": "user"}))).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut pick = None;
        loop {
            match events.recv().await.map(|e| e.unscoped().clone()) {
                Ok(RuntimeEvent::RoutingDecided { kwant_id, basis, .. }) => pick = Some((kwant_id, basis)),
                Ok(RuntimeEvent::TurnEnded { .. }) => return pick.expect("routed"),
                Err(e) => panic!("events: {e}"),
                _ => {}
            }
        }
    })
    .await
    .expect("the message was handled")
}

/// Leaves only `keep` in the Default space, so the seeded starter bots don't take part.
async fn only(app: &Router, state: &AppState, keep: &[&str]) {
    for k in state.db.list_kwants(&SpaceId::default_space()).await.unwrap() {
        if !keep.contains(&k.id.as_str()) {
            ok(app, "DELETE", &format!("/api/kwants/{}", k.id), None).await;
        }
    }
}

async fn concierges(state: &AppState) -> Vec<String> {
    let all = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    all.into_iter().filter(|k| k.name == "Concierge").map(|k| k.id).collect()
}

/// Oleks 2026-11-01: "Best place to eat corn dogs in Munich tomorrow?" in #general went to the
/// TikTok scriptwriter. Live HTTP path, Laya unreachable.
#[tokio::test]
async fn corn_dogs_go_to_the_concierge_and_undo_keeps_it_gone() {
    let (app, state, _tmp) = setup().await;
    let script = ok(&app, "POST", "/api/kwants", Some(specialist("Scriptwriter", "TikTok Scriptwriter & Hook Specialist",
        "You write short TikTok video scripts and scroll-stopping hooks for book content."))).await["id"].as_str().unwrap().to_string();
    let atlas = ok(&app, "POST", "/api/kwants", Some(specialist("Atlas", "Technical SEO", "You audit sites for crawl errors, sitemaps and page speed."))).await["id"].as_str().unwrap().to_string();
    only(&app, &state, &[&script, &atlas]).await;
    let general = state.db.general_channel(&SpaceId::default_space()).await.unwrap().expect("#general").id;

    // A clear specialist message: no Concierge is made
    let (to, basis) = routed_to(&app, &state, &general, "Write a TikTok hook for this fantasy book").await;
    assert_eq!((to.as_str(), basis.as_deref()), (script.as_str(), Some("keywords")));
    assert!(concierges(&state).await.is_empty());

    // Nothing fits: the Concierge is made, with a card, and answers
    let (to, basis) = routed_to(&app, &state, &general, "Best place to eat corn dogs in Munich tomorrow?").await;
    let made = concierges(&state).await;
    assert_eq!(made.len(), 1);
    assert_eq!((to.as_str(), basis.as_deref()), (made[0].as_str(), Some("concierge")));
    let actions = ok(&app, "GET", "/api/bot-actions", None).await;
    let list = actions.as_array().cloned().or_else(|| actions["actions"].as_array().cloned()).unwrap_or_default();
    let card = list.iter().find(|a| a["action"] == "concierge_create").expect("a Concierge card").clone();
    assert_eq!(card["target_id"], made[0].as_str());
    // It doesn't count against the daily cap on bots made without a card
    assert_eq!(state.db.auto_hires_today(&SpaceId::default_space()).await.unwrap(), 0);

    // The next one reuses it; role scoring never picks it, even for a word in its own title
    let (to, basis) = routed_to(&app, &state, &general, "Any good everyday questions to ask at a dinner party?").await;
    assert_eq!((to.as_str(), basis.as_deref()), (made[0].as_str(), Some("concierge")));
    assert_eq!(concierges(&state).await.len(), 1);

    // Undo removes it even though it has answered, and it isn't made again
    ok(&app, "POST", &format!("/api/bot-actions/{}/undo", card["id"].as_str().unwrap()), None).await;
    assert!(concierges(&state).await.is_empty());
    let (to, basis) = routed_to(&app, &state, &general, "Best place to eat corn dogs in Munich tomorrow?").await;
    assert_ne!(basis.as_deref(), Some("concierge"), "{to}");
    assert!(concierges(&state).await.is_empty(), "never made again unasked");
}

/// Outside #general a low score stays with the channel's own members.
#[tokio::test]
async fn other_channels_lean_to_their_own_members() {
    let (app, state, _tmp) = setup().await;
    let a = ok(&app, "POST", "/api/kwants", Some(specialist("Scriptwriter", "TikTok Scriptwriter", "Scripts and hooks."))).await["id"].as_str().unwrap().to_string();
    let b = ok(&app, "POST", "/api/kwants", Some(specialist("Atlas", "Technical SEO", "Crawl errors."))).await["id"].as_str().unwrap().to_string();
    let c = ok(&app, "POST", "/api/channels", Some(json!({"name": "content", "lead_kwant_id": a, "member_kwants": [a, b]}))).await;
    let (to, basis) = routed_to(&app, &state, c["id"].as_str().unwrap(), "Best place to eat corn dogs in Munich tomorrow?").await;
    assert_eq!((to.as_str(), basis.as_deref()), (a.as_str(), Some("lead")));
    assert!(concierges(&state).await.is_empty());
}
