//! A Home Assistant on the home network over plain http (26a): sign-in is refused with an error
//! that names the header way, and the header way lists its tools, reads marked read-only.
//!
//! `homeassistant.lan` doesn't resolve in CI, so this binary sends kwnt's outgoing http through
//! a local proxy (`http_proxy`), which is the fake Home Assistant itself. That's why it lives in
//! its own test binary: the proxy setting is process-wide.

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

const HOST: &str = "kwnt.example.test";
const ORIGIN: &str = "https://kwnt.example.test";
const TOKEN: &str = "test-token-0123456789abcdef";
const HA_URL: &str = "http://homeassistant.lan:8123/api/mcp";
const HA_TOKEN: &str = "ha-long-lived-token";

#[derive(Default)]
struct Ha {
    /// Requests that reached it addressed to homeassistant.lan:8123 (through the proxy).
    lan_requests: AtomicUsize,
}

/// Home Assistant's MCP endpoint: 401 without its token, otherwise its tools as it lists them.
async fn ha_mcp(State(ha): State<Arc<Ha>>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    if headers.get("host").and_then(|h| h.to_str().ok()) == Some("homeassistant.lan:8123") {
        ha.lan_requests.fetch_add(1, Ordering::SeqCst);
    }
    let bearer = format!("Bearer {HA_TOKEN}");
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(bearer.as_str()) {
        return (StatusCode::UNAUTHORIZED, "401: Unauthorized").into_response();
    }
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": msg["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "home-assistant", "version": "1.26.0"},
        }),
        "tools/list" => {
            let fixture: Value = serde_json::from_str(include_str!("fixtures/mcp/home-assistant-tools.json")).unwrap();
            json!({"tools": fixture["tools"]})
        }
        _ => return Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})).into_response(),
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", HOST)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn home_assistant_on_lan_http_signs_in_with_a_header_not_oauth() {
    let ha = Arc::new(Ha::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let fake = Router::new()
        .route("/api/mcp", post(ha_mcp).delete(|| async { StatusCode::OK }))
        .with_state(ha.clone());
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    // Before any http client is built (kwnt reads the proxy when it makes one)
    for var in ["http_proxy", "HTTP_PROXY"] {
        std::env::set_var(var, &proxy);
    }
    for var in ["no_proxy", "NO_PROXY", "all_proxy", "ALL_PROXY"] {
        std::env::remove_var(var);
    }

    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(kwnt::server::auth::ApiToken::for_tests(TOKEN, kwnt::server::auth::Require::Auto, config.port));
    let app = create_router(state);

    // Added without a header, then Connect: refused, and the error says what to do instead
    let (status, created) =
        call(&app, "POST", "/api/mcp/servers", Some(json!({"name": "homeassistant", "transport": "http", "url": HA_URL}))).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let (status, body) = call(&app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": ORIGIN}))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("must use https"), "{error}");
    assert!(error.contains("Authorization = Bearer <token>") && error.contains("Test connection"), "{error}");
    assert!(ha.lan_requests.load(Ordering::SeqCst) >= 1, "kwnt asked the LAN server for its challenge through the proxy");

    // Without a header, Test connection asks for sign-in
    let (_, tested) = call(&app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(tested["ok"], false, "{tested}");

    // With the Bearer header the same plain-http LAN server works, and its reads are read-only
    let (status, updated) = call(
        &app,
        "PUT",
        &format!("/api/mcp/servers/{id}"),
        Some(json!({"name": "homeassistant", "transport": "http", "url": HA_URL,
                    "headers": {"Authorization": format!("Bearer {HA_TOKEN}")}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let (status, tested) = call(&app, "POST", &format!("/api/mcp/servers/{id}/test"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(tested["server_info"]["name"], "home-assistant");
    let read_only = |name: &str| {
        tested["tools"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name}: {tested}"))["read_only"]
            .clone()
    };
    assert_eq!(read_only("GetLiveContext"), true);
    assert_eq!(read_only("HassGetState"), true);
    assert_eq!(read_only("HassTurnOn"), false);
    assert_eq!(read_only("HassTurnOff"), false);
    assert!(!tested.to_string().contains(HA_TOKEN), "the token never comes back");
}
