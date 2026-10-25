//! Web Push to the installed app (2026-10-25): a browser subscribes with our VAPID key, and a
//! notification reaches a (fake, local) push service as an RFC 8291 aes128gcm body with an
//! RFC 8292 VAPID token, TTL and Urgency. A 410 from the push service drops the subscription,
//! `notify_on` is honoured, the routes are person-only, and signing a session out stops its pushes.

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::{ConnectInfo, State},
    http::{HeaderMap, Request, StatusCode},
    routing::post,
    Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        continuity::{self, Notification},
        create_router,
        push::{self, Push},
        signin, AppState,
    },
    tools::ToolRunner,
};
use ring::agreement;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const REMOTE: &str = "203.0.113.9:40000";

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.push = Arc::new(Push::for_tests_allowing_local(&data));
    (create_router(state.clone()), state, tmp)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>, bearer: Option<&str>, cookie: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "kwnt.example.com")
        .header("user-agent", "Mozilla/5.0 (Linux; Android 14; Pixel) Chrome/129.0 Mobile Safari/537.36");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    if let Some(c) = cookie {
        req = req.header("cookie", format!("kwnt_token={c}"));
    }
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let mut req = req.body(body).unwrap();
    req.extensions_mut().insert(ConnectInfo(REMOTE.parse::<SocketAddr>().unwrap()));
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<(HeaderMap, Bytes)>>>,
    status: Arc<AtomicU16>,
}

async fn fake_push(State(f): State<Fake>, headers: HeaderMap, body: Bytes) -> StatusCode {
    f.seen.lock().unwrap().push((headers, body));
    StatusCode::from_u16(f.status.load(Ordering::SeqCst)).unwrap()
}

/// A push service on 127.0.0.1 that records requests and answers with `status`.
async fn start_fake() -> (Fake, String) {
    let fake = Fake { status: Arc::new(AtomicU16::new(201)), ..Default::default() };
    let app = Router::new().route("/push/:id", post(fake_push)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, format!("http://{addr}"))
}

async fn wait_for(fake: &Fake, n: usize) {
    for _ in 0..100 {
        if fake.seen.lock().unwrap().len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("push service got {} requests, wanted {n}", fake.seen.lock().unwrap().len());
}

/// A browser's push keys: (private key, p256dh, auth).
fn browser_keys() -> (agreement::EphemeralPrivateKey, Vec<u8>, [u8; 16]) {
    let rng = ring::rand::SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
    let public = private.compute_public_key().unwrap().as_ref().to_vec();
    (private, public, [9u8; 16])
}

fn note(kind: &'static str) -> Notification {
    Notification {
        kind,
        kwant_id: "k-ada".into(),
        kwant_name: "Ada".into(),
        channel_id: Some("ch-1".into()),
        title: "Ada needs your approval".into(),
        body: "Run `cargo test`?".into(),
    }
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

#[tokio::test]
async fn a_notification_reaches_the_push_service_encrypted_and_a_410_drops_it() {
    let (app, state, _tmp) = setup().await;
    let (fake, base) = start_fake().await;

    let (status, key) = call(&app, "GET", "/api/push/key", None, Some(TOKEN), None).await;
    assert_eq!(status, StatusCode::OK);
    let public_key = key["public_key"].as_str().unwrap().to_string();
    assert_eq!(B64.decode(&public_key).unwrap().len(), 65);

    let (ua_private, ua_public, auth) = browser_keys();
    let endpoint = format!("{base}/push/phone-1");
    let sub = json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": { "p256dh": B64.encode(&ua_public), "auth": B64.encode(auth) },
    });
    let (status, _) = call(&app, "POST", "/api/push/subscribe", Some(sub), Some(TOKEN), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(state.push.subscriptions()[0].label, "Chrome on Android");

    continuity::notify(&state, note("approval_needed")).await;
    wait_for(&fake, 1).await;
    let (headers, body) = fake.seen.lock().unwrap()[0].clone();
    assert_eq!(header(&headers, "content-encoding"), "aes128gcm");
    assert_eq!(header(&headers, "ttl"), "86400");
    assert_eq!(header(&headers, "urgency"), "high");
    assert_eq!(header(&headers, "topic").len(), 32);
    let auth_header = header(&headers, "authorization");
    let (jwt, k) = auth_header.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
    assert_eq!(k, public_key);
    let claims: Value = serde_json::from_slice(&B64.decode(jwt.split('.').nth(1).unwrap()).unwrap()).unwrap();
    assert_eq!(claims["aud"], base);
    assert_eq!(claims["sub"], "mailto:contact@kwant.si");

    // The browser opens it with its private key
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, body[21..86].to_vec());
    let ecdh = agreement::agree_ephemeral(ua_private, &peer, |s| s.to_vec()).unwrap();
    let plain = push::decrypt_with_secret(&ecdh, &auth, &ua_public, &body).expect("decrypts");
    let payload: Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(payload["title"], "Ada needs your approval");
    assert_eq!(payload["body"], "Run `cargo test`?");
    assert_eq!(payload["kind"], "approval_needed");
    assert_eq!(payload["kwant_id"], "k-ada");
    assert_eq!(payload["channel_id"], "ch-1");
    assert_eq!(payload["url"], "/?channel=ch-1");
    assert_eq!(payload["tag"], "kwnt-none-approval_needed-ch-1");
    assert!(plain.len() <= push::MAX_PAYLOAD_BYTES);

    // The browser unsubscribed on its side: the push service says 410 and we forget it
    fake.status.store(410, Ordering::SeqCst);
    continuity::notify(&state, note("turn_finished")).await;
    wait_for(&fake, 2).await;
    let (headers, _) = fake.seen.lock().unwrap()[1].clone();
    assert_eq!(header(&headers, "ttl"), "3600");
    assert_eq!(header(&headers, "urgency"), "normal");
    for _ in 0..50 {
        if state.push.subscriptions().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(state.push.subscriptions().is_empty(), "a 410 drops the subscription");
}

#[tokio::test]
async fn kinds_the_person_turned_off_are_not_pushed() {
    let (app, state, _tmp) = setup().await;
    let (fake, base) = start_fake().await;
    let (_, ua_public, auth) = browser_keys();
    let sub = json!({ "endpoint": format!("{base}/push/p"), "keys": { "p256dh": B64.encode(&ua_public), "auth": B64.encode(auth) } });
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(sub), Some(TOKEN), None).await.0, StatusCode::NO_CONTENT);
    continuity::save_notify_settings(&state, None, Some(vec!["approval_needed".into()])).await.unwrap();
    continuity::notify(&state, note("turn_finished")).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(fake.seen.lock().unwrap().is_empty());
    continuity::notify(&state, note("approval_needed")).await;
    wait_for(&fake, 1).await;
}

#[tokio::test]
async fn push_routes_are_person_only_and_check_what_they_store() {
    let (app, state, _tmp) = setup().await;
    let (_, ua_public, auth) = browser_keys();
    let keys = json!({ "p256dh": B64.encode(&ua_public), "auth": B64.encode(auth) });

    // From another machine without a sign-in
    assert_eq!(call(&app, "GET", "/api/push/key", None, None, None).await.0, StatusCode::UNAUTHORIZED);
    let sub = json!({ "endpoint": "https://fcm.googleapis.com/fcm/send/x", "keys": keys });
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(sub.clone()), None, None).await.0, StatusCode::UNAUTHORIZED);
    // A bearer that isn't the API token (a bot's turn token) is refused
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(sub.clone()), Some("kt1.some-turn-token"), None).await.0.as_u16() / 100, 4);
    assert!(state.push.subscriptions().is_empty());

    for (endpoint, why) in [
        ("http://fcm.googleapis.com/x", "https"),
        ("https://169.254.169.254/latest/meta-data", "public"),
        ("https://localhost/x", "public"),
        ("https://[::1]/x", "public"),
    ] {
        let (status, body) = call(&app, "POST", "/api/push/subscribe", Some(json!({ "endpoint": endpoint, "keys": keys })), Some(TOKEN), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{endpoint}");
        assert!(body["error"].as_str().unwrap().contains(why), "{endpoint}: {body}");
    }
    let bad_keys = json!({ "endpoint": "https://fcm.googleapis.com/fcm/send/x", "keys": { "p256dh": "abc", "auth": B64.encode(auth) } });
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(bad_keys), Some(TOKEN), None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(json!({ "nope": 1 })), Some(TOKEN), None).await.0, StatusCode::BAD_REQUEST);
    assert!(state.push.subscriptions().is_empty());

    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(sub), Some(TOKEN), None).await.0, StatusCode::NO_CONTENT);
    let gone = json!({ "endpoint": "https://fcm.googleapis.com/fcm/send/x" });
    assert_eq!(call(&app, "POST", "/api/push/unsubscribe", Some(gone.clone()), Some(TOKEN), None).await.0, StatusCode::NO_CONTENT);
    assert!(state.push.subscriptions().is_empty());
    // Unsubscribing twice is fine
    assert_eq!(call(&app, "POST", "/api/push/unsubscribe", Some(gone), Some(TOKEN), None).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn signing_a_browser_out_stops_its_pushes() {
    let (app, state, _tmp) = setup().await;
    let (_, ua_public, auth) = browser_keys();
    let keys = json!({ "p256dh": B64.encode(&ua_public), "auth": B64.encode(auth) });
    let client = signin::Client { browser: "Chrome on Android".into(), ip: "203.0.113.9".into() };
    let phone = state.api_token.signin.create_session(signin::Via::Device, &client);
    let laptop = state.api_token.signin.create_session(signin::Via::Link, &client);

    let sub = |e: &str| json!({ "endpoint": format!("https://fcm.googleapis.com/fcm/send/{e}"), "keys": keys, "label": "Pixel 11 Pro" });
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(sub("phone")), None, Some(&phone)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "POST", "/api/push/subscribe", Some(sub("laptop")), None, Some(&laptop)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(state.push.subscriptions().len(), 2);
    assert_eq!(state.push.subscriptions()[0].label, "Pixel 11 Pro");

    // Signed out from the Signed-in devices list
    let phone_id = state.api_token.signin.session_id(&phone).unwrap();
    assert_eq!(call(&app, "DELETE", &format!("/api/signin/sessions/{phone_id}"), None, None, Some(&laptop)).await.0, StatusCode::NO_CONTENT);
    let left: Vec<String> = state.push.subscriptions().into_iter().map(|s| s.endpoint).collect();
    assert_eq!(left, vec!["https://fcm.googleapis.com/fcm/send/laptop".to_string()]);

    // Signing itself out
    assert_eq!(call(&app, "DELETE", "/api/auth/session", None, None, Some(&laptop)).await.0, StatusCode::NO_CONTENT);
    assert!(state.push.subscriptions().is_empty());
}
