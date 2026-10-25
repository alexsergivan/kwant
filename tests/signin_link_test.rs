//! One-time sign-in links (2026-10-25): a link signs one browser in once and then stops working,
//! runs out after its time, and gives a session cookie that is never the API token and can be
//! signed out on its own. Making a link is the person's: another machine needs a sign-in, and a
//! bot's turn token is refused.

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
use std::time::Duration;
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

async fn redeem(app: &Router, code: &str) -> Resp {
    call(app, "POST", "/api/auth/session", Some(json!({ "token": code })), None, None).await
}

#[tokio::test]
async fn a_link_signs_in_one_browser_once() {
    let (app, _state, _tmp) = setup().await;
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, None).await.status, StatusCode::UNAUTHORIZED);

    // The person (with the token here; the CLI does the same) makes a link
    let made = call(&app, "POST", "/api/signin/links", None, Some(TOKEN), None).await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.body);
    let code = made.body["code"].as_str().unwrap().to_string();
    assert!(code.starts_with("kl1.") && !code.contains(TOKEN));
    assert_eq!(made.body["expires_in"], 900);
    assert!(made.body["url"].is_null(), "no public address configured");

    let first = redeem(&app, &code).await;
    assert_eq!(first.status, StatusCode::NO_CONTENT, "{}", first.body);
    let cookie = first.cookie.unwrap();
    assert!(cookie.starts_with("kwnt_token=ks1."), "{cookie}");
    assert!(!cookie.contains(TOKEN), "the cookie must not be the API token");
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&cookie)).await.status, StatusCode::OK);
    // A session is the person: it may make a link for the next device
    assert_eq!(call(&app, "POST", "/api/signin/links", None, None, Some(&cookie)).await.status, StatusCode::OK);

    let again = redeem(&app, &code).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);
    assert_eq!(again.body["reason"], "link_used_or_expired");
    assert!(again.cookie.is_none());
}

#[tokio::test]
async fn an_expired_link_is_refused() {
    let (app, state, _tmp) = setup().await;
    let code = state.api_token.signin.create_link(Duration::from_millis(1));
    tokio::time::sleep(Duration::from_millis(10)).await;
    let r = redeem(&app, &code).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["reason"], "link_used_or_expired");
    assert!(r.cookie.is_none());
}

#[tokio::test]
async fn a_made_up_code_is_a_wrong_token() {
    let (app, _state, _tmp) = setup().await;
    let r = redeem(&app, "kl1.0000000000000000000000000000000000000000000000000000000000000000").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["reason"], "wrong_token");
}

#[tokio::test]
async fn the_token_still_signs_in_but_the_cookie_is_a_session() {
    let (app, state, _tmp) = setup().await;
    let r = redeem(&app, TOKEN).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let cookie = r.cookie.unwrap();
    assert!(!cookie.contains(TOKEN), "{cookie}");
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&cookie)).await.status, StatusCode::OK);
    // Browsers signed in before sessions hold the token itself: still accepted
    let old = format!("kwnt_token={TOKEN}");
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&old)).await.status, StatusCode::OK);

    // Signing out ends that session on the server, not just in this browser
    assert_eq!(state.api_token.signin.sessions().len(), 1);
    let out = call(&app, "DELETE", "/api/auth/session", None, None, Some(&cookie)).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(state.api_token.signin.sessions().is_empty());
    assert_eq!(call(&app, "GET", "/api/kwants", None, None, Some(&cookie)).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn only_the_person_makes_links() {
    let (app, state, _tmp) = setup().await;
    let none = call(&app, "POST", "/api/signin/links", None, None, None).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let forged = call(&app, "POST", "/api/signin/links", None, None, Some("kwnt_token=ks1.forged")).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    let folder = tempdir().unwrap();
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_, turn) = mcp_runtime::prepare_turn_with_self(&state, &bot, None, Some(folder.path().to_path_buf()), Some(scope))
        .await
        .unwrap();
    // From another machine a turn token isn't the API token (401); on this machine, where no
    // token is needed, it is still refused (403)
    let r = call(&app, "POST", "/api/signin/links", None, Some(&turn), None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.body);
    let r = call_from(&app, "127.0.0.1:50000", "127.0.0.1:8799", "POST", "/api/signin/links", None, Some(&turn), None).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "a bot's turn token: {}", r.body);
}
