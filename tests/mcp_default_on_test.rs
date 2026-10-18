//! MCP servers are on for every kwant by default; a kwant opts out per server.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::{McpServer, NewKwant}, Database},
    router::LayaRouter,
    server::{create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

fn server(name: &str, enabled: bool) -> McpServer {
    McpServer {
        id: String::new(),
        name: name.into(),
        transport: "stdio".into(),
        command: Some("/usr/bin/true".into()),
        args: vec![],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled,
        timeout_secs: 30,
        created_at: None,
    }
}

async fn env() -> (AppState, Router, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (state.clone(), create_router(state), tmp)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn turn_servers(state: &AppState, kwant_id: &str) -> Vec<String> {
    let kwant = state.db.get_kwant(kwant_id).await.unwrap().unwrap();
    match mcp_runtime::prepare_turn(state, &kwant, None).await {
        Some((ctx, token)) => {
            let names = ctx.servers.iter().map(|s| s.name.clone()).collect();
            mcp_runtime::finish_turn(state, &token).await;
            names
        }
        None => vec![],
    }
}

#[tokio::test]
async fn servers_are_on_for_every_kwant_until_opted_out() {
    let (state, app, _tmp) = env().await;
    let db = &state.db;
    let kwants = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let (a, b) = (kwants[0].id.clone(), kwants[1].id.clone());

    // A server added after the kwants exist is on for all of them, with nothing assigned
    let notion_id = db.create_mcp_server(&kwnt::db::spaces::SpaceId::default_space(), &server("notion", true)).await.unwrap().id;
    let off_id = db.create_mcp_server(&kwnt::db::spaces::SpaceId::default_space(), &server("off", false)).await.unwrap().id;
    for k in [&a, &b] {
        assert_eq!(turn_servers(&state, k).await, vec!["notion"], "globally disabled servers stay out");
    }
    // ...and for a kwant created later
    let late = db
        .create_kwant(&kwnt::db::spaces::SpaceId::default_space(), &NewKwant {
            name: "Late".into(),
            title: "t".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "architect".into(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
        })
        .await
        .unwrap();
    assert_eq!(turn_servers(&state, &late.id).await, vec!["notion"]);

    // GET lists every server the kwant hasn't opted out of, enabled or not, so the picker round-trips
    let uri = format!("/api/kwants/{a}/mcp-servers");
    let (status, got) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["server_ids"], json!([notion_id, off_id]));

    // Opting kwant a out of notion: gone from its turns, still on for b
    let (status, put) = call(&app, "PUT", &uri, Some(json!({"server_ids": [off_id]}))).await;
    assert_eq!(status, StatusCode::OK, "{put}");
    assert_eq!(put["server_ids"], json!([off_id]));
    let (_, got) = call(&app, "GET", &uri, None).await;
    assert_eq!(got["server_ids"], json!([off_id]));
    assert!(turn_servers(&state, &a).await.is_empty());
    assert_eq!(turn_servers(&state, &b).await, vec!["notion"]);

    // The opt-out survives the server being switched off and on again globally
    let mut notion = db.get_mcp_server(&notion_id).await.unwrap().unwrap();
    notion.enabled = false;
    db.update_mcp_server(&notion).await.unwrap();
    notion.enabled = true;
    db.update_mcp_server(&notion).await.unwrap();
    assert!(turn_servers(&state, &a).await.is_empty());

    // A server added after the opt-out is on for a too
    let github_id = db.create_mcp_server(&kwnt::db::spaces::SpaceId::default_space(), &server("github", true)).await.unwrap().id;
    assert_eq!(turn_servers(&state, &a).await, vec!["github"]);

    // The server list says who has each server off
    let (_, list) = call(&app, "GET", "/api/mcp/servers", None).await;
    let by_name = |n: &str| list.as_array().unwrap().iter().find(|s| s["name"] == n).unwrap().clone();
    assert_eq!(by_name("notion")["disabled_for_kwants"], json!([a]));
    assert_eq!(by_name("github")["disabled_for_kwants"], json!([]));
    let (_, one) = call(&app, "GET", &format!("/api/mcp/servers/{notion_id}"), None).await;
    assert_eq!(one["disabled_for_kwants"], json!([a]));

    // Turning it back on clears the opt-out
    call(&app, "PUT", &uri, Some(json!({"server_ids": [notion_id, off_id, github_id]}))).await;
    assert_eq!(turn_servers(&state, &a).await, vec!["github", "notion"]);
    assert!(db.list_mcp_kwant_opt_outs(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap().is_empty());

    // Validation is unchanged
    let (status, _) = call(&app, "PUT", &uri, Some(json!({"server_ids": ["nope"]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&app, "GET", "/api/kwants/nope/mcp-servers", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_server_removes_its_opt_outs() {
    let (state, app, _tmp) = env().await;
    let db = &state.db;
    let kwants = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    let notion_id = db.create_mcp_server(&kwnt::db::spaces::SpaceId::default_space(), &server("notion", true)).await.unwrap().id;
    for k in &kwants {
        db.set_kwant_mcp_servers(&k.id, &[]).await.unwrap();
    }
    assert_eq!(db.list_mcp_kwant_opt_outs(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[&notion_id].len(), kwants.len());

    let (status, _) = call(&app, "DELETE", &format!("/api/mcp/servers/{notion_id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(db.list_mcp_kwant_opt_outs(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap().is_empty(), "no orphaned opt-out rows");

    // A server added again starts on for everyone
    db.create_mcp_server(&kwnt::db::spaces::SpaceId::default_space(), &server("notion", true)).await.unwrap();
    for k in &kwants {
        assert_eq!(turn_servers(&state, &k.id).await, vec!["notion"]);
    }
}
