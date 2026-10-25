//! Letting a new browser in from a signed-in device (2026-10-25): the new browser gets a secret
//! device code and a short user code, and is let in once the person approves that code from a
//! signed-in session. Approving needs a sign-in (never a bot's turn token), codes are single use,
//! requests are rate limited, and signed-in browsers can be listed and signed out.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        continuity::SelfScope,
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const REMOTE: &str = "203.0.113.9:40000";
const HOST: &str = "kwnt.example.com";

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
    (create_router(state.clone()), state, tmp)
}

struct Resp {
    status: StatusCode,
    body: Value,
    cookie: Option<String>,
}

/// A request from another machine, with an optional bearer and cookie.
async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>, bearer: Option<&str>, cookie: Option<&str>) -> Resp {
    call_from(app, REMOTE, HOST, method, uri, body, bearer, cookie).await
}

#[allow(clippy::too_many_arguments)]
async fn call_from(
    app: &Router,
    peer: &str,
    host: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
    cookie: Option<&str>,
) -> Resp {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host)
        .header("user-agent", "Mozilla/5.0 (Linux; Android 14) Chrome/129.0 Mobile Safari/537.36");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let mut req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())).unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    req.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(|c| c.split(';').next().unwrap().to_string());
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Resp { status, body: serde_json::from_slice(&bytes).unwrap_or(Value::Null), cookie }
}


async fn sign_in_with_token(app: &Router) -> String {
    let r = call(app, "POST", "/api/auth/session", Some(json!({ "token": TOKEN })), None, None).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    r.cookie.unwrap()
}

async fn ask(app: &Router) -> (String, String) {
    let r = call(app, "POST", "/api/auth/device", Some(json!({})), None, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["expires_in"], 600);
    assert_eq!(r.body["interval"], 2);
    (r.body["device_code"].as_str().unwrap().into(), r.body["user_code"].as_str().unwrap().into())
}

async fn poll(app: &Router, device: &str) -> Resp {
    call(app, "POST", "/api/auth/device/poll", Some(json!({ "device_code": device })), None, None).await
}

#[tokio::test]
async fn a_signed_in_device_lets_a_new_one_in_once() {
    let (app, _state, _tmp) = setup().await;
    let status = call(&app, "GET", "/api/auth/status", None, None, None).await;
    assert_eq!(status.body["device_login"], true);

    let (device, user) = ask(&app).await;
    let p = poll(&app, &device).await;
    assert_eq!((p.status, p.body["status"].as_str()), (StatusCode::OK, Some("pending")));
    assert!(p.cookie.is_none());

    // The person, signed in elsewhere, sees the request and approves it
    let mine = sign_in_with_token(&app).await;
    let list = call(&app, "GET", "/api/signin/requests", None, None, Some(&mine)).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body[0]["user_code"], user.as_str());
    assert_eq!(list.body[0]["browser"], "Chrome on Android");
    assert_eq!(list.body[0]["ip"], "203.0.113.9");
    let ok = call(&app, "POST", "/api/signin/requests/approve", Some(json!({ "user_code": user })), None, Some(&mine)).await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT, "{}", ok.body);

    let p = poll(&app, &device).await;
    assert_eq!((p.status, p.body["status"].as_str()), (StatusCode::OK, Some("approved")));
    let theirs = p.cookie.unwrap();
    assert!(theirs.starts_with("kwnt_token=ks1.") && theirs != mine);
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&theirs)).await.status, StatusCode::OK);

    // Single use: the device code is spent, and the user code can't be approved again
    let again = poll(&app, &device).await;
    assert_eq!((again.status, again.body["status"].as_str()), (StatusCode::GONE, Some("expired")));
    assert!(again.cookie.is_none());
    let twice = call(&app, "POST", "/api/signin/requests/approve", Some(json!({ "user_code": user })), None, Some(&mine)).await;
    assert_eq!(twice.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn approving_needs_a_sign_in() {
    let (app, state, _tmp) = setup().await;
    let (device, user) = ask(&app).await;
    let code = json!({ "user_code": user });

    // Nobody signed in, a made-up session, the new browser itself
    let none = call(&app, "POST", "/api/signin/requests/approve", Some(code.clone()), None, None).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let forged = call(&app, "POST", "/api/signin/requests/approve", Some(code.clone()), None, Some("kwnt_token=ks1.forged")).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    let own = call(&app, "POST", "/api/signin/requests/approve", Some(code.clone()), Some(&device), None).await;
    assert_eq!(own.status, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "GET", "/api/signin/requests", None, None, None).await.status, StatusCode::UNAUTHORIZED);

    // A bot's turn token, even on this machine where no token is needed
    let folder = tempdir().unwrap();
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_, turn) = mcp_runtime::prepare_turn_with_self(&state, &bot, None, Some(folder.path().to_path_buf()), Some(scope))
        .await
        .unwrap();
    let r = call_from(&app, "127.0.0.1:50000", "127.0.0.1:8799", "POST", "/api/signin/requests/approve", Some(code.clone()), Some(&turn), None).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.body);

    // Still waiting: none of that let it in
    assert_eq!(poll(&app, &device).await.body["status"], "pending");
    assert!(state.api_token.signin.sessions().is_empty());
}

#[tokio::test]
async fn a_denied_device_is_told_once() {
    let (app, _state, _tmp) = setup().await;
    let (device, user) = ask(&app).await;
    let mine = sign_in_with_token(&app).await;
    let r = call(&app, "POST", "/api/signin/requests/deny", Some(json!({ "user_code": user })), None, Some(&mine)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let p = poll(&app, &device).await;
    assert_eq!((p.status, p.body["status"].as_str()), (StatusCode::FORBIDDEN, Some("denied")));
    assert!(p.cookie.is_none());
    assert_eq!(poll(&app, &device).await.status, StatusCode::GONE);
}

#[tokio::test]
async fn an_expired_request_cannot_be_approved() {
    let (app, state, _tmp) = setup().await;
    let client = kwnt::server::signin::Client { browser: "x".into(), ip: "198.51.100.7".into() };
    let (device, user) = state.api_token.signin.start_device_for(&client, std::time::Duration::from_millis(1)).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let mine = sign_in_with_token(&app).await;
    let r = call(&app, "POST", "/api/signin/requests/approve", Some(json!({ "user_code": user })), None, Some(&mine)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(poll(&app, &device).await.status, StatusCode::GONE);
    assert_eq!(poll(&app, "kd1.made-up").await.status, StatusCode::GONE);
}

#[tokio::test]
async fn one_address_is_rate_limited() {
    let (app, _state, _tmp) = setup().await;
    for _ in 0..5 {
        ask(&app).await;
    }
    let r = call(&app, "POST", "/api/auth/device", Some(json!({})), None, None).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.body["retry_after"].as_u64().is_some_and(|s| (1..=60).contains(&s)), "{}", r.body);
}

#[tokio::test]
async fn sessions_are_listed_and_signed_out() {
    let (app, _state, _tmp) = setup().await;
    let a = sign_in_with_token(&app).await;
    let b = sign_in_with_token(&app).await;
    let list = call(&app, "GET", "/api/signin/sessions", None, None, Some(&a)).await;
    assert_eq!(list.status, StatusCode::OK);
    let rows = list.body.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|r| r["current"] == true).count(), 1);
    assert!(rows.iter().all(|r| r["via"] == "token" && r["browser"] == "Chrome on Android"));
    assert!(!list.body.to_string().contains("ks1."), "no secrets in the list");

    let other = rows.iter().find(|r| r["current"] == false).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(call(&app, "DELETE", &format!("/api/signin/sessions/{other}"), None, None, Some(&a)).await.status, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&b)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&a)).await.status, StatusCode::OK);
    assert_eq!(call(&app, "DELETE", &format!("/api/signin/sessions/{other}"), None, None, Some(&a)).await.status, StatusCode::NOT_FOUND);
    // Signed out browsers can't list or revoke
    assert_eq!(call(&app, "GET", "/api/signin/sessions", None, None, Some(&b)).await.status, StatusCode::UNAUTHORIZED);
}
