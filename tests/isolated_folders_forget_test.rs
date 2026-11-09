//! #446 item 6: the person lists the folders isolated bots used and forgets one
//! (`GET /api/isolated-folders`, `POST /api/isolated-folders/forget`). Contract:
//! release-080/forget-folder-contract.md. A bot can't do either: if it could, it could plant a
//! hook in its folder and then have a CLI on this computer load it.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{isolated_folders::canonical, spaces::SpaceId, Database},
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
const LOCAL: &str = "127.0.0.1:50000";
const REMOTE: &str = "203.0.113.9:50000";

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

async fn call(app: &Router, peer: &str, method: &str, uri: &str, body: Option<Value>, bearer: Option<&str>) -> (StatusCode, Value) {
    let host = if peer == LOCAL { "127.0.0.1:8799" } else { "kwnt.example.com" };
    let mut req = Request::builder().method(method).uri(uri).header("host", host).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A POST with a raw body, and a content type only when one is given.
async fn post_raw(app: &Router, uri: &str, body: &'static str, content_type: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method("POST").uri(uri).header("host", "127.0.0.1:8799");
    if let Some(ct) = content_type {
        req = req.header("content-type", ct);
    }
    let mut req = req.body(Body::from(body)).unwrap();
    req.extensions_mut().insert(ConnectInfo(LOCAL.parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn turn_token(state: &AppState, folder: &std::path::Path) -> (String, String) {
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_, token) = mcp_runtime::prepare_turn_with_self(state, &bot, None, Some(folder.to_path_buf()), Some(scope))
        .await
        .unwrap();
    (bot.id, token)
}

#[tokio::test]
async fn the_person_lists_and_forgets_a_folder_and_a_bot_cant() {
    let (app, state, tmp) = setup().await;
    let proj = tmp.path().join("proj");
    let other = tmp.path().join("other");
    std::fs::create_dir_all(proj.join(".claude")).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(proj.join(".claude/settings.json"), r#"{"hooks":{}}"#).unwrap();
    let (bot_id, turn) = turn_token(&state, &proj).await;
    state.db.record_isolated_folder(&canonical(&proj), &bot_id).await.unwrap();
    state.db.record_isolated_folder(&canonical(&other), "deleted-bot").await.unwrap();

    let (s, v) = call(&app, LOCAL, "GET", "/api/isolated-folders", None, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let folders = v["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2, "{v}");
    let p = folders.iter().find(|f| f["path"] == canonical(&proj).to_string_lossy().as_ref()).unwrap();
    assert_eq!(p["first_bot_id"], bot_id.as_str());
    assert!(p["first_bot_name"].is_string(), "{p}");
    assert_eq!(p["exists"], true);
    assert_eq!(p["check_first"], json!([".claude/settings.json"]));
    assert!(p["first_used_at"].is_string() && p["last_used_at"].is_string(), "{p}");
    let o = folders.iter().find(|f| f["first_bot_id"] == "deleted-bot").unwrap();
    assert_eq!(o["first_bot_name"], Value::Null, "a deleted bot has no name: {o}");

    // A bot's own turn token is refused on both routes, from this machine too; nothing changes
    let path = canonical(&proj).to_string_lossy().into_owned();
    let (s, v) = call(&app, LOCAL, "GET", "/api/isolated-folders", None, Some(&turn)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert!(v.get("folders").is_none(), "{v}");
    let (s, _) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(json!({ "path": path })), Some(&turn)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // Another machine without the API token
    let (s, _) = call(&app, REMOTE, "POST", "/api/isolated-folders/forget", Some(json!({ "path": path })), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(state.db.isolated_folder_for(&proj).await.unwrap().is_some(), "still recorded");

    // Bad bodies
    for body in [json!({}), json!({ "path": "" }), json!({ "path": "relative/proj" }), json!({ "path": 5 }), json!("x")] {
        let (s, v) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(body.clone()), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}: {v}");
        assert_eq!(v["code"], "invalid");
    }
    // #462 review must-fix 3: a NUL, a path over 4096 bytes, a body that isn't JSON and one sent
    // without a JSON content type are 400 too, with the same code; exactly 4096 bytes is allowed
    // (and isn't recorded, so 404)
    let long = |n: usize| format!("/{}", "a".repeat(n - 1));
    for body in [json!({ "path": "/a\u{0000}b" }), json!({ "path": long(4097) })] {
        let (s, v) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(body.clone()), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{}: {v}", &body.to_string()[..40]);
        assert_eq!(v["code"], "invalid");
    }
    assert_eq!(long(4096).len(), 4096);
    let (s, v) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(json!({ "path": long(4096) })), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "4096 bytes is a path kwnt looks up: {v}");
    assert_eq!(v["code"], "not_recorded");
    for (body, ct) in [("not json", Some("application/json")), (r#"{"path":"/x"}"#, None), (r#"{"path":"/x"}"#, Some("text/plain"))] {
        let (s, v) = post_raw(&app, "/api/isolated-folders/forget", body, ct).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} as {ct:?}: {v}");
        assert_eq!(v["code"], "invalid", "{body} as {ct:?}");
    }
    assert!(state.db.isolated_folder_for(&proj).await.unwrap().is_some(), "still recorded after every refusal");
    // A folder that isn't recorded, even one inside a recorded folder
    let inside = canonical(&proj).join(".claude").to_string_lossy().into_owned();
    let (s, v) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(json!({ "path": inside })), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert_eq!(v["code"], "not_recorded");

    // The person forgets it: only that record goes, and kwnt trusts the folder again
    assert!(state.db.host_command_refusal(&proj).await.is_some());
    let (s, v) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(json!({ "path": path })), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["forgotten"], path.as_str());
    assert_eq!(v["folders"].as_array().unwrap().len(), 1, "the other folder stays: {v}");
    assert!(state.db.host_command_refusal(&proj).await.is_none(), "host commands run in it again");
    let (s, v) = call(&app, LOCAL, "POST", "/api/isolated-folders/forget", Some(json!({ "path": path })), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "twice: {v}");

    // A folder deleted since is still listed, and forgotten by the path the list shows
    std::fs::remove_dir_all(&other).unwrap();
    let (_, v) = call(&app, LOCAL, "GET", "/api/isolated-folders", None, None).await;
    let gone = &v["folders"][0];
    assert_eq!(gone["exists"], false, "{v}");
    assert_eq!(gone["check_first"], json!([]));
    let (s, v) = call(&app, REMOTE, "POST", "/api/isolated-folders/forget", Some(json!({ "path": gone["path"] })), Some(TOKEN)).await;
    assert_eq!(s, StatusCode::OK, "with the API token from another machine: {v}");
    assert_eq!(v["folders"], json!([]));
}
