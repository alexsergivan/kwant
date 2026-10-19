//! Agents finding MCP servers in the registry and suggesting them; the person installing them.
//! A fake registry and a fake http MCP server stand in for the real ones: nothing here touches
//! the network beyond 127.0.0.1.

use axum::{
    body::{to_bytes, Body},
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    mcp::{self, registry::McpRegistry, McpClient, McpServerInput, SECRET_MASK},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const SECRET: &str = "s3cret-key-123";

async fn listen() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

/// A streamable-HTTP MCP server with one `echo` tool that wants `Authorization: Bearer <SECRET>`.
async fn fake_mcp() -> String {
    async fn mcp(headers: HeaderMap, Json(msg): Json<Value>) -> Response {
        if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(&format!("Bearer {SECRET}")) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let id = msg["id"].clone();
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                                   "serverInfo": {"name": "fake-echo", "version": "1.0.0"}}),
            "tools/list" => json!({"tools": [{"name": "echo", "description": "Echoes text",
                                   "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}]}),
            "tools/call" => json!({"content": [{"type": "text", "text": format!("echo: {}", msg["params"]["arguments"]["text"].as_str().unwrap_or(""))}]}),
            _ => return StatusCode::ACCEPTED.into_response(),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    }
    let (listener, base) = listen().await;
    tokio::spawn(async move { axum::serve(listener, Router::new().route("/mcp", post(mcp))).await });
    format!("{base}/mcp")
}

/// The OAuth mock: an MCP server that needs sign-in.
async fn fake_signin_mcp() -> String {
    let (listener, base) = listen().await;
    tokio::spawn(mcp::oauth_mock::serve(listener, mcp::oauth_mock::MockOptions::default()));
    format!("{base}/mcp")
}

struct Registry {
    entries: Vec<Value>,
    hits: AtomicUsize,
}

async fn list(State(r): State<Arc<Registry>>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    r.hits.fetch_add(1, Ordering::SeqCst);
    let search = q.get("search").cloned().unwrap_or_default();
    let servers: Vec<Value> = r.entries.iter().filter(|e| e["server"]["name"].as_str().unwrap().contains(&search)).cloned().collect();
    Json(json!({"servers": servers, "metadata": {"count": servers.len()}}))
}

async fn one(State(r): State<Arc<Registry>>, Path((name, version)): Path<(String, String)>) -> Response {
    assert_eq!(version, "latest");
    match r.entries.iter().find(|e| e["server"]["name"] == name.as_str()) {
        Some(e) => Json(e.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn entry(name: &str, server: Value, status: &str) -> Value {
    let mut s = json!({"$schema": "https://static.modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                       "name": name, "version": "1.0.0", "repository": {"url": "https://github.com/example/echo", "source": "github"}});
    for (k, v) in server.as_object().unwrap() {
        s[k] = v.clone();
    }
    json!({"server": s, "_meta": {"io.modelcontextprotocol.registry/official": {"status": status, "isLatest": true}}})
}

async fn fake_registry(echo_url: &str, signin_url: &str) -> (String, Arc<Registry>) {
    let auth = json!([{"name": "Authorization", "value": "Bearer {api_key}", "isRequired": true, "isSecret": true,
                       "description": "Your Echo API key"}]);
    let entries = vec![
        entry("io.github.kwnt-test/echo", json!({"title": "Echo", "description": "Echoes text back.",
            "remotes": [{"type": "streamable-http", "url": echo_url, "headers": auth}]}), "active"),
        entry("io.github.kwnt-test/echo-local", json!({"description": "Echo, run locally.",
            "packages": [{"registryType": "npm", "identifier": "@kwnt-test/echo-mcp", "version": "1.0.0", "transport": {"type": "stdio"},
                          "environmentVariables": [{"name": "ECHO_KEY", "isRequired": true, "isSecret": true, "description": "Key"}]}]}), "active"),
        entry("io.github.kwnt-test/echo-broken", json!({"description": "Never answers.",
            "remotes": [{"type": "streamable-http", "url": "http://127.0.0.1:9/mcp"}]}), "active"),
        entry("io.github.kwnt-test/echo-signin", json!({"description": "Needs sign-in.",
            "remotes": [{"type": "streamable-http", "url": signin_url}]}), "active"),
        entry("io.github.kwnt-test/echo-old", json!({"description": "Gone.",
            "remotes": [{"type": "streamable-http", "url": "https://old.example/mcp"}]}), "deleted"),
        entry("io.github.kwnt-test/echo-shell", json!({"description": "Wants a shell.",
            "packages": [{"registryType": "npm", "identifier": "x", "version": "1.0.0", "runtimeHint": "bash", "transport": {"type": "stdio"}}]}), "active"),
    ];
    let reg = Arc::new(Registry { entries, hits: AtomicUsize::new(0) });
    let app = Router::new()
        .route("/v0.1/servers", get(list))
        .route("/v0.1/servers/:name/versions/:version", get(one))
        .with_state(reg.clone());
    let (listener, base) = listen().await;
    tokio::spawn(async move { axum::serve(listener, app).await });
    (base, reg)
}

struct Env {
    app: Router,
    state: AppState,
    reg: Arc<Registry>,
    bot: Kwant,
    other: Kwant,
    echo_url: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
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
    let echo_url = fake_mcp().await;
    let (registry, reg) = fake_registry(&echo_url, &fake_signin_mcp().await).await;
    state.mcp_registry = Arc::new(McpRegistry::new(&registry));
    let kwants = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    Env {
        app: create_router(state.clone()),
        bot: kwants[0].clone(),
        other: kwants[1].clone(),
        state,
        reg,
        echo_url,
        _tmp: tmp,
    }
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

async fn self_tool(app: &Router, token: &str, name: &str, args: Value) -> (String, bool) {
    let (status, v) = call(app, "POST", "/api/mcp/self", Some(token), Some(rpc("tools/call", json!({"name": name, "arguments": args})))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (
        v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(),
        v["result"]["isError"].as_bool().unwrap_or(false),
    )
}

/// A turn of `bot` in its direct chat, with its own tools; returns the turn token.
async fn turn(e: &Env, turn_id: &str) -> String {
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() }),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

async fn suggest(e: &Env, token: &str, id: &str) -> (String, bool) {
    self_tool(&e.app, token, "mcp_suggest", json!({"registry_id": id, "reason": "You asked me to echo text through a service."})).await
}

async fn suggestions(e: &Env) -> Vec<Value> {
    let (status, v) = call(&e.app, "GET", "/api/mcp/suggestions", None, None).await;
    assert_eq!(status, StatusCode::OK);
    v.as_array().unwrap().clone()
}

async fn servers(e: &Env) -> Vec<mcp::McpServerView> {
    let (_, v) = call(&e.app, "GET", "/api/mcp/servers", None, None).await;
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn search_shows_what_would_run_and_what_is_already_there() {
    let e = setup().await;
    let token = turn(&e, "t-search").await;
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), Some(rpc("tools/list", json!({})))).await;
    let names: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"mcp_search") && names.contains(&"mcp_suggest"), "{names:?}");

    let (text, err) = self_tool(&e.app, &token, "mcp_search", json!({"query": "echo"})).await;
    assert!(!err, "{text}");
    assert!(text.contains("not instructions"), "untrusted-data note: {text}");
    let json_part: Value = serde_json::from_str(&text[text.find('[').unwrap()..]).unwrap();
    let hit = |id: &str| json_part.as_array().unwrap().iter().find(|h| h["registry_id"] == id).cloned();
    let echo = hit("io.github.kwnt-test/echo").unwrap();
    assert_eq!(echo["transport"], "http");
    assert_eq!(echo["runs"], e.echo_url.as_str());
    assert_eq!(echo["needs"][0]["name"], "Authorization");
    assert!(echo.get("status").is_none());
    let local = hit("io.github.kwnt-test/echo-local").unwrap();
    assert_eq!(local["runs"], "npx -y @kwnt-test/echo-mcp@1.0.0");
    assert_eq!(local["runs_on_users_computer"], true);
    assert!(hit("io.github.kwnt-test/echo-shell").unwrap()["not_one_click"].as_str().unwrap().contains("bash"));
    assert!(hit("io.github.kwnt-test/echo-old").is_none(), "deleted entries are hidden");

    // The same search again is served from the cache
    let before = e.reg.hits.load(Ordering::SeqCst);
    self_tool(&e.app, &token, "mcp_search", json!({"query": "echo"})).await;
    assert_eq!(e.reg.hits.load(Ordering::SeqCst), before);

    // Added by hand already: the agent is told to use it, and suggesting it creates nothing
    let input: McpServerInput = serde_json::from_value(json!({"name": "my-echo", "transport": "http", "url": e.echo_url,
        "headers": {"Authorization": format!("Bearer {SECRET}")}})).unwrap();
    let server = e.state.db.create_mcp_server(&SpaceId::default_space(), &mcp::build_server(input, None).unwrap()).await.unwrap();
    let (text, _) = self_tool(&e.app, &token, "mcp_search", json!({"query": "kwnt-test/echo"})).await;
    assert!(text.contains("\"available\"") && text.contains("mcp__my-echo__"), "{text}");
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/echo").await;
    assert!(!err && text.contains("Already installed as \"my-echo\""), "{text}");
    assert!(suggestions(&e).await.is_empty());

    // Switched off for this agent: it says so instead of suggesting a second copy
    e.state.db.set_kwant_mcp_servers(&e.bot.id, &[]).await.unwrap();
    let (text, _) = self_tool(&e.app, &token, "mcp_search", json!({"query": "kwnt-test/echo"})).await;
    assert!(text.contains("installed_but_off"), "{text}");
    e.state.db.delete_mcp_server(&server.id).await.unwrap();

    // Unknown and not-one-click entries can't be suggested
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/nope").await;
    assert!(err && text.contains("no active server"), "{text}");
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/echo-shell").await;
    assert!(err && text.contains("one click"), "{text}");
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/echo-old").await;
    assert!(err, "{text}");
    assert!(suggestions(&e).await.is_empty());
}

#[tokio::test]
async fn registry_down_is_a_plain_tool_error() {
    let mut e = setup().await;
    e.state.mcp_registry = Arc::new(McpRegistry::new("http://127.0.0.1:9"));
    e.app = create_router(e.state.clone());
    let token = turn(&e, "t-down").await;
    let (text, err) = self_tool(&e.app, &token, "mcp_search", json!({"query": "echo"})).await;
    assert!(err && text.contains("couldn't be reached"), "{text}");
}

#[tokio::test]
async fn suggest_posts_a_card_and_installs_nothing_and_is_limited_per_turn() {
    let e = setup().await;
    let token = turn(&e, "t-suggest").await;
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/echo").await;
    assert!(!err && text.contains("waiting for the user") && text.contains("Nothing is installed"), "{text}");
    assert!(text.contains("types it into that card") && text.contains("one approval"), "{text}");
    assert!(servers(&e).await.is_empty(), "suggesting installs nothing");

    let list = suggestions(&e).await;
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!((s["status"].as_str(), s["kwant_id"].as_str(), s["turn_id"].as_str()), (Some("pending"), Some(e.bot.id.as_str()), Some("t-suggest")));
    let card = &s["card"];
    assert_eq!(card["kind"], "mcp_suggestion");
    assert_eq!(card["url"], e.echo_url.as_str());
    assert_eq!(card["runs_on_your_computer"], false);
    assert_eq!(card["inputs"][0]["name"], "Authorization");
    assert_eq!(card["inputs"][0]["secret"], true);
    assert_eq!(card["inputs"][0]["template"], "Bearer {api_key}");
    assert_eq!(card["reason"], "You asked me to echo text through a service.");

    // The card is a message in the bot's direct chat
    let messages = e.state.db.list_messages(&e.bot.id, 50).await.unwrap();
    let msg = messages.iter().find(|m| Some(m.id.as_str()) == s["message_id"].as_str()).expect("card message");
    let meta: Value = serde_json::from_str(msg.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["suggestion_id"], s["id"]);

    // A stdio entry is marked as running on the person's computer, with the exact command
    suggest(&e, &token, "io.github.kwnt-test/echo-local").await;
    let local = suggestions(&e).await.into_iter().find(|s| s["registry_name"] == "io.github.kwnt-test/echo-local").unwrap();
    assert_eq!(local["card"]["runs_on_your_computer"], true);
    assert_eq!(local["card"]["command"], "npx");
    assert_eq!(local["card"]["args"], json!(["-y", "@kwnt-test/echo-mcp@1.0.0"]));

    // The same one again is not a new suggestion
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/echo").await;
    assert!(!err && text.contains("already suggested"), "{text}");
    assert_eq!(suggestions(&e).await.len(), 2);

    // Three per turn
    suggest(&e, &token, "io.github.kwnt-test/echo-broken").await;
    let (text, err) = suggest(&e, &token, "io.github.kwnt-test/echo-signin").await;
    assert!(err && text.contains("3 suggestions this turn"), "{text}");
    assert_eq!(suggestions(&e).await.len(), 3);
    // A new turn may suggest again
    let next = turn(&e, "t-suggest-2").await;
    let (_, err) = suggest(&e, &next, "io.github.kwnt-test/echo-signin").await;
    assert!(!err);
    assert!(servers(&e).await.is_empty());
}

#[tokio::test]
async fn agents_tokens_cannot_install_or_dismiss() {
    let e = setup().await;
    let token = turn(&e, "t-bot").await;
    suggest(&e, &token, "io.github.kwnt-test/echo").await;
    let id = suggestions(&e).await[0]["id"].as_str().unwrap().to_string();
    let body = json!({"headers": {"Authorization": SECRET}});
    for bearer in [token.as_str(), "made-up-token"] {
        let (status, v) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/install"), Some(bearer), Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        let (status, _) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/dismiss"), Some(bearer), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    // No self tool installs, assigns or enables
    let (text, err) = self_tool(&e.app, &token, "mcp_install", json!({"id": id})).await;
    assert!(err && text.contains("Unknown tool"), "{text}");
    assert!(servers(&e).await.is_empty());
    assert_eq!(suggestions(&e).await[0]["status"], "pending");
    mcp_runtime::finish_turn(&e.state, &token).await;
    // Still refused once the turn is over
    let (status, _) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/install"), Some(&token), Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn person_installs_it_for_that_bot_only_and_its_next_turn_has_the_tool() {
    let e = setup().await;
    let token = turn(&e, "t-install").await;
    suggest(&e, &token, "io.github.kwnt-test/echo").await;
    let id = suggestions(&e).await[0]["id"].as_str().unwrap().to_string();
    let uri = format!("/api/mcp/suggestions/{id}/install");

    // Required secret missing, or a value the plan doesn't take: refused, nothing created
    let (status, v) = call(&e.app, "POST", &uri, None, Some(json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("Authorization is required"));
    let (status, _) = call(&e.app, "POST", &uri, None, Some(json!({"headers": {"Authorization": SECRET, "Cookie": "x"}}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(servers(&e).await.is_empty());

    // The person types just the key; the registry's "Bearer {api_key}" puts it in place
    let (status, v) = call(&e.app, "POST", &uri, None, Some(json!({"headers": {"Authorization": SECRET}}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["test"]["ok"], true, "{v}");
    assert_eq!(v["test"]["tools"][0]["name"], "echo");
    assert_eq!(v["oauth_required"], false);
    assert_eq!(v["suggestion"]["status"], "installed");
    assert_eq!(v["suggestion"]["card"]["status"], "installed");
    let server_id = v["server"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["suggestion"]["server_id"], server_id.as_str());
    assert_eq!(v["server"]["name"], "echo");
    assert_eq!(v["server"]["headers"]["Authorization"], SECRET_MASK);
    assert!(!v.to_string().contains(SECRET), "secret leaked: {v}");
    let others: Vec<String> = v["server"]["disabled_for_kwants"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect();
    assert!(others.contains(&e.other.id) && !others.contains(&e.bot.id), "{others:?}");

    // Stored for real, masked everywhere it is shown
    let stored = e.state.db.get_mcp_server(&server_id).await.unwrap().unwrap();
    assert_eq!(stored.headers["Authorization"], format!("Bearer {SECRET}"));
    let listed = servers(&e).await;
    assert_eq!(listed[0].headers["Authorization"], SECRET_MASK);
    let (_, all) = call(&e.app, "GET", "/api/mcp/suggestions", None, None).await;
    assert!(!all.to_string().contains(SECRET));
    let messages = e.state.db.list_messages(&e.bot.id, 50).await.unwrap();
    assert!(messages.iter().all(|m| !m.meta.as_deref().unwrap_or("").contains(SECRET) && !m.text.contains(SECRET)));
    let card_msg = messages.iter().find(|m| Some(m.id.as_str()) == v["suggestion"]["message_id"].as_str()).unwrap();
    assert!(card_msg.meta.as_deref().unwrap().contains("\"installed\""), "card updated");

    // Only the suggesting bot has it; its next turn gets the tool without a restart
    let mine: Vec<String> = e.state.db.list_active_mcp_servers_for_kwant(&e.bot.id).await.unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(mine, ["echo"]);
    assert!(e.state.db.list_active_mcp_servers_for_kwant(&e.other.id).await.unwrap().is_empty());
    let scope = SelfScope { kwant_id: e.bot.id.clone(), kwant_name: e.bot.name.clone(), ..Default::default() };
    let (ctx, next) = mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, Some("t-next".into()), None, Some(scope)).await.unwrap();
    let echo = ctx.servers.iter().find(|s| s.name == "echo").expect("echo in the next turn");
    let mut client = McpClient::connect(echo, None).await.unwrap();
    let out = client.call_tool("echo", json!({"text": "hi"})).await.unwrap();
    assert_eq!(out.text, "echo: hi");
    client.shutdown().await;
    mcp_runtime::finish_turn(&e.state, &next).await;

    // Once installed it can't be installed or dismissed again, and search says it's there
    let (status, _) = call(&e.app, "POST", &uri, None, Some(json!({"headers": {"Authorization": SECRET}}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/dismiss"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (text, _) = self_tool(&e.app, &token, "mcp_search", json!({"query": "kwnt-test/echo"})).await;
    assert!(text.contains("Already installed as \\\"echo\\\""), "{text}");
}

#[tokio::test]
async fn failed_test_removes_the_server_and_dismiss_closes_it() {
    let e = setup().await;
    let token = turn(&e, "t-fail").await;
    suggest(&e, &token, "io.github.kwnt-test/echo-broken").await;
    let id = suggestions(&e).await[0]["id"].as_str().unwrap().to_string();
    let (status, v) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/install"), None, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["test"]["ok"], false);
    assert_eq!(v["suggestion"]["status"], "pending");
    assert!(v["suggestion"]["last_error"].as_str().unwrap().contains("didn't start"));
    assert!(servers(&e).await.is_empty(), "a server that fails its test is removed");

    let (status, v) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/dismiss"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "dismissed");
    assert_eq!(v["card"]["status"], "dismissed");
    let (status, _) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/dismiss"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/install"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(&e.app, "POST", "/api/mcp/suggestions/nope/dismiss", None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, pending) = call(&e.app, "GET", "/api/mcp/suggestions?status=pending", None, None).await;
    assert!(pending.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn server_that_needs_sign_in_is_kept_and_says_so() {
    let e = setup().await;
    let token = turn(&e, "t-oauth").await;
    suggest(&e, &token, "io.github.kwnt-test/echo-signin").await;
    let id = suggestions(&e).await[0]["id"].as_str().unwrap().to_string();
    let (status, v) = call(&e.app, "POST", &format!("/api/mcp/suggestions/{id}/install"), None, None).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["oauth_required"], true);
    assert_eq!(v["test"]["auth_required"], true);
    assert_eq!(v["suggestion"]["status"], "installed");
    let server_id = v["server"]["id"].as_str().unwrap();
    assert!(e.state.db.get_mcp_server(server_id).await.unwrap().is_some());
}

/// The real registry, by hand only: `cargo test --test mcp_suggest_test -- --ignored`.
#[tokio::test]
#[ignore = "talks to registry.modelcontextprotocol.io"]
async fn live_registry_smoke() {
    let registry = McpRegistry::new("https://registry.modelcontextprotocol.io");
    let hits = registry.search("github").await.unwrap();
    assert!(!hits.is_empty());
    let one_click = hits.iter().filter(|h| h.plan.is_ok()).count();
    println!("{} hits, {one_click} one-click", hits.len());
    for h in hits.iter().take(8) {
        println!("  {} {} -> {:?}", h.name, h.version, h.plan.as_ref().map(|p| p.url.clone().unwrap_or_else(|| p.command.clone().unwrap_or_default())));
    }
    let first = registry.get_latest(&hits[0].name).await.unwrap().expect("latest of a search hit");
    assert_eq!(first.name, hits[0].name);
}
