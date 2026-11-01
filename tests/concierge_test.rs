//! The Concierge in #general: a message no specialist fits goes to it (made on the first one,
//! with a card the person can undo), a clear specialist message doesn't, and once the person
//! removes it, it is never made again. A fake claude CLI answers turns.
//! Also the close calls the person settles on a card, a bot handing a message back, and the
//! covers/avoids lists on the bot routes.
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
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
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

async fn call_as(app: &Router, uri: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"));
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A turn of `bot` in `channel` with its own tools; returns the turn token.
async fn turn_token(state: &AppState, bot: &str, channel: &str) -> String {
    let k = state.db.get_kwant(bot).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: k.id.clone(),
        kwant_name: k.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(channel.into()), ..Default::default() }),
        turn_id: Some(format!("turn-{bot}")),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(state, &k, Some(format!("turn-{bot}")), None, Some(scope)).await.unwrap().1
}

async fn hand_back(app: &Router, token: &str, reason: &str) -> (String, bool) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "hand_back", "arguments": {"reason": reason}}});
    let (s, v) = call_as(app, "/api/mcp/self", token, rpc).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
}

/// Waits for the next turn to end and returns the routing of the reply it saved in `channel`.
async fn next_reply_routing(state: &AppState, events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>, channel: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(RuntimeEvent::TurnEnded { .. }) = events.recv().await.map(|e| e.unscoped().clone()) {
                break;
            }
        }
    })
    .await
    .expect("the turn ended");
    let msgs = state.db.list_messages(channel, 50).await.unwrap();
    let reply = msgs.iter().rev().find(|m| m.sender_role == "kwant").expect("a reply");
    let meta: Value = reply.meta.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or(Value::Null);
    meta["routing"].clone()
}

/// A close call (two bots with the same role and words) posts a card instead of a turn; the
/// person's choice runs that bot once, and "answer generally" in #general goes to the Concierge.
#[tokio::test]
async fn a_close_call_asks_and_the_choice_answers() {
    let (app, state, _tmp) = setup().await;
    let fe = ok(&app, "POST", "/api/kwants", Some(specialist("Bea", "Frontend engineer", "You build the app's pages."))).await["id"].as_str().unwrap().to_string();
    let web = ok(&app, "POST", "/api/kwants", Some(specialist("Cy", "Web developer", "You build websites."))).await["id"].as_str().unwrap().to_string();
    only(&app, &state, &[&fe, &web]).await;
    let general = state.db.general_channel(&SpaceId::default_space()).await.unwrap().expect("#general").id;

    let v = ok(&app, "POST", "/api/chat/send", Some(json!({"channel_id": general, "prompt": "The button CSS is off on mobile Safari", "sender_id": "user"}))).await;
    assert_eq!(v["status"], "asking", "{v}");
    let ids: Vec<&str> = v["choices"].as_array().unwrap().iter().map(|c| c["kwant_id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&fe.as_str()) && ids.contains(&web.as_str()), "{v}");
    assert_eq!(v["general"]["name"], "Concierge");
    let choice = v["route_choice_id"].as_str().unwrap().to_string();
    let card = state.db.list_messages(&general, 50).await.unwrap().into_iter().find(|m| Some(m.id.as_str()) == v["card_message_id"].as_str()).expect("the card");
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap_or("null")).unwrap();
    assert_eq!(meta["kind"], "route_choice");
    let uri = format!("/api/route-choices/{choice}/resolve");

    // Only an offered bot, only one way, only the person
    let (s, _) = call(&app, "POST", &uri, Some(json!({"kwant_id": "nope"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&app, "POST", &uri, Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let token = turn_token(&state, &fe, &general).await;
    let (s, _) = call_as(&app, &uri, &token, json!({"kwant_id": web})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let mut events = state.event_sender.subscribe();
    let v = ok(&app, "POST", &uri, Some(json!({"kwant_id": web}))).await;
    assert_eq!((v["status"].as_str(), v["kwant_id"].as_str()), (Some("dispatched"), Some(web.as_str())));
    let routing = next_reply_routing(&state, &mut events, &general).await;
    assert_eq!((routing["basis"].as_str(), routing["kwant_id"].as_str()), (Some("ask"), Some(web.as_str())), "{routing}");
    let (s, _) = call(&app, "POST", &uri, Some(json!({"kwant_id": fe}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a second tap does nothing");

    // "Answer generally" in #general: the Concierge, made now
    let v = ok(&app, "POST", "/api/chat/send", Some(json!({"channel_id": general, "prompt": "Is the CSS grid off on the button?", "sender_id": "user"}))).await;
    assert_eq!(v["status"], "asking", "{v}");
    let mut events = state.event_sender.subscribe();
    let v = ok(&app, "POST", &format!("/api/route-choices/{}/resolve", v["route_choice_id"].as_str().unwrap()), Some(json!({"general": true}))).await;
    let made = concierges(&state).await;
    assert_eq!(made.len(), 1);
    assert_eq!(v["kwant_id"].as_str(), Some(made[0].as_str()));
    let routing = next_reply_routing(&state, &mut events, &general).await;
    assert_eq!(routing["basis"], "ask", "{routing}");
    assert_eq!(state.db.get_scope(&made[0]).await.unwrap().covers.len(), kwnt::server::concierge::COVERS.len());
}

/// "Not my area": the message goes on once, to the next bot that fits or the Concierge; a second
/// hand-back of the same message is refused.
#[tokio::test]
async fn a_hand_back_takes_one_hop() {
    let (app, state, _tmp) = setup().await;
    let script = ok(&app, "POST", "/api/kwants", Some(specialist("Scriptwriter", "TikTok Scriptwriter", "Scripts and hooks for book videos."))).await["id"].as_str().unwrap().to_string();
    let atlas = ok(&app, "POST", "/api/kwants", Some(specialist("Atlas", "Technical SEO", "Crawl errors and sitemaps."))).await["id"].as_str().unwrap().to_string();
    only(&app, &state, &[&script, &atlas]).await;
    let general = state.db.general_channel(&SpaceId::default_space()).await.unwrap().expect("#general").id;
    let (to, _) = routed_to(&app, &state, &general, "Write a TikTok hook about the sitemap launch party").await;
    assert_eq!(to, script);

    let token = turn_token(&state, &script, &general).await;
    let (text, err) = hand_back(&app, &token, "").await;
    assert!(err, "a reason is required: {text}");
    let mut events = state.event_sender.subscribe();
    let (text, err) = hand_back(&app, &token, "This is about the sitemap, not a script").await;
    assert!(!err, "{text}");
    assert!(text.contains("Atlas"), "{text}");
    let routing = next_reply_routing(&state, &mut events, &general).await;
    assert_eq!((routing["basis"].as_str(), routing["kwant_id"].as_str()), (Some("handback"), Some(atlas.as_str())), "{routing}");

    // One hop: Atlas can't bounce it on
    let token = turn_token(&state, &atlas, &general).await;
    let (text, err) = hand_back(&app, &token, "Not mine either").await;
    assert!(err && text.contains("already handed back"), "{text}");
}

/// The bot routes read and update covers/avoids; left out, a list stays as it is.
#[tokio::test]
async fn covers_and_avoids_are_read_and_updated() {
    let (app, _state, _tmp) = setup().await;
    let bot = ok(&app, "POST", "/api/kwants", Some(specialist("Chef", "Recipe planner", "Meals."))).await;
    assert_eq!((bot["covers"].clone(), bot["avoids"].clone()), (json!([]), json!([])));
    let id = bot["id"].as_str().unwrap();
    let mut body = specialist("Chef", "Recipe planner", "Meals.");
    body["covers"] = json!([" restaurants ", "Restaurants", "street food"]);
    body["avoids"] = json!(["wine"]);
    let v = ok(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
    assert_eq!((v["covers"].clone(), v["avoids"].clone()), (json!(["restaurants", "street food"]), json!(["wine"])));
    let v = ok(&app, "PUT", &format!("/api/kwants/{id}"), Some(specialist("Chef", "Recipe planner", "Meals."))).await;
    assert_eq!(v["covers"], json!(["restaurants", "street food"]));
    let mut body = specialist("Chef", "Recipe planner", "Meals.");
    body["avoids"] = json!(["x".repeat(61)]);
    let (s, _) = call(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let list = ok(&app, "GET", "/api/kwants", None).await;
    let listed = list.as_array().unwrap().iter().find(|k| k["id"] == id).unwrap().clone();
    assert_eq!(listed["avoids"], json!(["wine"]));
}
