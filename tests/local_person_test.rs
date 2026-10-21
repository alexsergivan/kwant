//! Person-only routes from the person's own machine (2026-10-21): the person's browser on this
//! machine isn't asked for the API token, a bot's turn token is still a 403 there, another machine
//! still needs the token (401), and `KWNT_REQUIRE_TOKEN=always` asks for it on loopback too.

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
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";

async fn setup(require: Require) -> (Router, AppState, TempDir) {
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
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, require, config.port));
    (create_router(state.clone()), state, tmp)
}

/// A GET from `peer` with `host`, as the real server sees it (with its peer address).
async fn get(app: &Router, peer: &str, host: &str, uri: &str, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method("GET").uri(uri).header("host", host);
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let mut req = req.body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A real bot's saved passwords: the route the "Confirm it's you" popup guarded.
async fn passwords(state: &AppState) -> String {
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    format!("/api/sandbox/{}/passwords", bot.id)
}

async fn turn_token(state: &AppState, folder: &std::path::Path) -> String {
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_, token) = mcp_runtime::prepare_turn_with_self(state, &bot, None, Some(folder.to_path_buf()), Some(scope))
        .await
        .unwrap();
    token
}

#[tokio::test]
async fn local_without_a_token_goes_through() {
    let (app, state, _tmp) = setup(Require::Auto).await;
    let pw = passwords(&state).await;
    let (s, v) = get(&app, "127.0.0.1:50000", "127.0.0.1:8799", &pw, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["passwords"].is_array(), "{v}");
    let (s, v) = get(&app, "[::1]:50000", "localhost:8799", "/api/auth/status", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["person_auth_required"], false, "the UI never asks for the token here: {v}");
    assert_eq!(v["person_authenticated"], true, "{v}");
}

#[tokio::test]
async fn local_bot_token_is_refused() {
    let (app, state, _tmp) = setup(Require::Auto).await;
    let pw = passwords(&state).await;
    let folder = tempdir().unwrap();
    let turn = turn_token(&state, folder.path()).await;
    let (s, v) = get(&app, "127.0.0.1:50000", "127.0.0.1:8799", &pw, Some(&turn)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a bot's turn token: {v}");
    let (s, v) = get(&app, "127.0.0.1:50000", "127.0.0.1:8799", &pw, Some("not-the-token")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a wrong token: {v}");
    assert!(v.get("passwords").is_none(), "{v}");
}

#[tokio::test]
async fn another_machine_without_a_token_is_refused() {
    let (app, state, _tmp) = setup(Require::Auto).await;
    let pw = passwords(&state).await;
    let (s, v) = get(&app, "203.0.113.9:50000", "kwnt.example.com", &pw, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    // Loopback Host through a remote peer is still remote
    let (s, _) = get(&app, "203.0.113.9:50000", "127.0.0.1:8799", &pw, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (_, v) = get(&app, "203.0.113.9:50000", "kwnt.example.com", "/api/auth/status", None).await;
    assert_eq!(v["person_auth_required"], true, "{v}");
    let (s, v) = get(&app, "203.0.113.9:50000", "kwnt.example.com", &pw, Some(TOKEN)).await;
    assert_eq!(s, StatusCode::OK, "with the API token: {v}");
}

#[tokio::test]
async fn require_always_asks_for_the_token_locally_too() {
    let (app, state, _tmp) = setup(Require::Always).await;
    let pw = passwords(&state).await;
    let (s, v) = get(&app, "127.0.0.1:50000", "127.0.0.1:8799", &pw, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    let (_, v) = get(&app, "127.0.0.1:50000", "127.0.0.1:8799", "/api/auth/status", None).await;
    assert_eq!(v["person_auth_required"], true, "{v}");
    let (s, v) = get(&app, "127.0.0.1:50000", "127.0.0.1:8799", &pw, Some(TOKEN)).await;
    assert_eq!(s, StatusCode::OK, "with the API token: {v}");
}
