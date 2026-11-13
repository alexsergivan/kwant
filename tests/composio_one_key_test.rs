//! Composio with one key: the person pastes their Composio API key and kwant makes the Tool
//! Router session itself. A refused key saves nothing, an old URL setup stays as it is until
//! Switch (which keeps its Composio user), the key can be replaced, and the app search says which
//! apps are connected with one account call. A Composio Connect key (`ck_`) is checked and used
//! through Composio Connect's MCP server. A fake Composio API, with a fake Composio Connect MCP
//! server answering in SSE, stands in for the real ones.

mod common;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use common::*;
use kwnt::mcp::composio::ComposioApi;
use kwnt::server::{continuity::SelfScope, mcp_runtime};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const REFUSED: &str = "Composio didn't accept this key. Copy your key again from your Composio dashboard.";

#[derive(Default)]
struct Fake {
    /// method, path with sorted query, x-api-key, JSON body
    seen: Vec<(String, String, String, Value)>,
    /// session id → (project, user)
    sessions: HashMap<String, (String, String)>,
    /// apps with a working account on Composio Connect
    active: BTreeSet<String>,
}
type Shared = Arc<Mutex<Fake>>;

fn key_of(h: &HeaderMap) -> String {
    h.get("x-api-key").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
}

/// The fake's keys: two of project p1, one of p2, and the refusals.
fn project(key: &str) -> Result<&'static str, (StatusCode, Json<Value>)> {
    match key {
        "ak_good" | "ak_good2" | "ak_big" => Ok("p1"),
        "ak_other" => Ok("p2"),
        "ak_scoped" => Err((StatusCode::FORBIDDEN, Json(json!({"error": {"message": "missing permission"}})))),
        "ak_down" => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"message": "boom"}})))),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"message": "Invalid API key: ak_**l123", "code": 10401, "slug": "APIKey_InvalidAPIKey"}})),
        )),
    }
}

fn query_path(path: &str, q: &HashMap<String, String>) -> String {
    let mut pairs: Vec<_> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
    pairs.sort();
    if pairs.is_empty() { path.to_string() } else { format!("{path}?{}", pairs.join("&")) }
}

/// Composio Connect's MCP server as Composio runs it: stateless, every answer one SSE event.
/// `ck_good` and `ck_other` are accepted, `ck_down` gets 500, any other key 401. Calls are
/// recorded as `/connect/mcp <method>` (and the action of a MANAGE_CONNECTIONS call) with the
/// `x-consumer-api-key` and the call's arguments.
async fn fake_connect(State(f): State<Shared>, h: HeaderMap, Json(body): Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let key = h.get("x-consumer-api-key").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    assert!(h.get("x-api-key").is_none(), "a Composio Connect key goes only in x-consumer-api-key");
    let method = body["method"].as_str().unwrap_or_default().to_string();
    let args = body["params"]["arguments"].clone();
    let what = match method.as_str() {
        "tools/call" if body["params"]["name"] == "COMPOSIO_SEARCH_TOOLS" => "/connect/mcp tools/call search".to_string(),
        "tools/call" => format!("/connect/mcp tools/call {}", args["toolkits"][0]["action"].as_str().unwrap_or_default()),
        m => format!("/connect/mcp {m}"),
    };
    let mut f = f.lock().unwrap();
    f.seen.push(("POST".into(), what, key.clone(), args.clone()));
    match key.as_str() {
        "ck_good" | "ck_other" => {}
        "ck_down" => return (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response(),
        _ => return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid consumer api key"}))).into_response(),
    }
    let Some(id) = body.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match method.as_str() {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-composio-connect", "version": "1"},
        }),
        "tools/list" => json!({"tools": [
            {"name": "COMPOSIO_SEARCH_TOOLS", "description": "search", "inputSchema": {"type": "object"}},
            {"name": "COMPOSIO_MANAGE_CONNECTIONS", "description": "connections", "inputSchema": {"type": "object"}},
            {"name": "COMPOSIO_MULTI_EXECUTE_TOOL", "description": "run", "inputSchema": {"type": "object"}},
        ]}),
        "tools/call" if body["params"]["name"] == "COMPOSIO_MANAGE_CONNECTIONS" => {
            let mut results = serde_json::Map::new();
            for t in args["toolkits"].as_array().cloned().unwrap_or_default() {
                let slug = t["name"].as_str().unwrap_or_default().to_string();
                let entry = match t["action"].as_str() {
                    Some("add") => json!({"toolkit": slug, "status": "initiated", "redirect_url": format!("https://connect.composio.dev/link/lk_{slug}")}),
                    _ if f.active.contains(&slug) => json!({"toolkit": slug, "status": "active", "accounts": [{"id": format!("ca_{slug}"), "status": "active"}]}),
                    _ => json!({"toolkit": slug, "status": "not_connected", "accounts": []}),
                };
                results.insert(slug, entry);
            }
            let text = json!({"successful": true, "data": {"results": results}, "error": null}).to_string();
            json!({"content": [{"type": "text", "text": text}], "isError": false})
        }
        // Composio's recorded answers (tests/fixtures/composio): ElevenLabs typed any way, and a
        // made-up name, which still gets unrelated toolkits back.
        "tools/call" if body["params"]["name"] == "COMPOSIO_SEARCH_TOOLS" => {
            let asked = args["queries"][0]["use_case"].as_str().unwrap_or_default().to_ascii_lowercase();
            let file = match asked.as_str() {
                "elevenlabs" => "search_tools_elevenlabs.json",
                "eleven labs" => "search_tools_eleven_labs.json",
                _ => "search_tools_notarealapp123.json",
            };
            let text = std::fs::read_to_string(format!("{}/tests/fixtures/composio/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
            json!({"content": [{"type": "text", "text": text}], "isError": false})
        }
        _ => json!({"content": [{"type": "text", "text": "not in this fake"}], "isError": true}),
    };
    let event = format!("event: message\ndata: {}\n\n", json!({"jsonrpc": "2.0", "id": id, "result": result}));
    ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], event).into_response()
}

async fn fake_composio() -> (String, Shared) {
    let fake: Shared = Arc::default();
    let app = Router::new()
        .route("/connect/mcp", post(fake_connect))
        // composio.dev's public app pages, as recorded: elevenlabs exists, nothing else here does.
        .route(
            "/toolkits/:slug",
            get(|State(f): State<Shared>, h: HeaderMap, Path(slug): Path<String>| async move {
                let key = [key_of(&h), h.get("x-consumer-api-key").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()].concat();
                f.lock().unwrap().seen.push(("HEAD".into(), format!("/toolkits/{slug}"), key, Value::Null));
                if slug == "elevenlabs" { StatusCode::OK } else { StatusCode::NOT_FOUND }
            }),
        )
        .route(
            "/api/v3.1/tool_router/session",
            post(|State(f): State<Shared>, h: HeaderMap, Json(body): Json<Value>| async move {
                let mut f = f.lock().unwrap();
                f.seen.push(("POST".into(), "/api/v3.1/tool_router/session".into(), key_of(&h), body.clone()));
                let p = match project(&key_of(&h)) {
                    Ok(p) => p,
                    Err(e) => return e,
                };
                let id = format!("trs_{}", f.sessions.len() + 1);
                let user = body["user_id"].as_str().unwrap_or_default().to_string();
                f.sessions.insert(id.clone(), (p.into(), user.clone()));
                (
                    StatusCode::CREATED,
                    Json(json!({
                        "session_id": id,
                        "mcp": {"type": "http", "url": format!("https://backend.composio.dev/tool_router/{id}/mcp")},
                        "tool_router_tools": ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_MANAGE_CONNECTIONS", "COMPOSIO_MULTI_EXECUTE_TOOL"],
                        "config": {"user_id": user, "manage_connections": {"enabled": true}},
                    })),
                )
            }),
        )
        .route(
            "/api/v3.1/tool_router/session/:id",
            get(|State(f): State<Shared>, Path(id): Path<String>, h: HeaderMap| async move {
                let mut f = f.lock().unwrap();
                f.seen.push(("GET".into(), format!("/api/v3.1/tool_router/session/{id}"), key_of(&h), Value::Null));
                let p = match project(&key_of(&h)) {
                    Ok(p) => p,
                    Err(e) => return e,
                };
                match f.sessions.get(&id) {
                    Some((sp, user)) if sp == p => {
                        (StatusCode::OK, Json(json!({"session_id": id, "config": {"user_id": user}})))
                    }
                    _ => (StatusCode::NOT_FOUND, Json(json!({"error": {"message": "session not found"}}))),
                }
            }),
        )
        .route(
            "/api/v3.1/tool_router/session/:id/link",
            post(|State(f): State<Shared>, Path(id): Path<String>, h: HeaderMap, Json(body): Json<Value>| async move {
                f.lock().unwrap().seen.push(("POST".into(), format!("/api/v3.1/tool_router/session/{id}/link"), key_of(&h), body));
                if let Err(e) = project(&key_of(&h)) {
                    return e;
                }
                (StatusCode::CREATED, Json(json!({"redirect_url": "https://connect.composio.dev/link/ln_1"})))
            }),
        )
        .route(
            "/api/v3.1/toolkits",
            get(|State(f): State<Shared>, h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                f.lock().unwrap().seen.push(("GET".into(), query_path("/api/v3.1/toolkits", &q), key_of(&h), Value::Null));
                if let Err(e) = project(&key_of(&h)) {
                    return e;
                }
                // ak_big's project has a catalogue the size of Composio's: 1,000 apps in pages of 500.
                if key_of(&h) == "ak_big" && !q.contains_key("search") {
                    let start = match q.get("cursor").map(String::as_str) { Some("p2") => 500, _ => 0 };
                    let items: Vec<Value> = (start..start + 500)
                        .map(|i| json!({"slug": format!("app_{i:04}"), "name": format!("Example App {i}"),
                            "meta": {"tools_count": i % 120, "logo": format!("https://logos.composio.dev/api/app_{i:04}"),
                                     "description": "A long description that the compact list leaves out."}}))
                        .collect();
                    return (StatusCode::OK, Json(json!({"items": items, "next_cursor": if start == 0 { json!("p2") } else { Value::Null }})));
                }
                // The whole catalogue (no search) comes in pages.
                if !q.contains_key("search") {
                    return match q.get("cursor").map(String::as_str) {
                        None => (StatusCode::OK, Json(json!({"items": [
                            {"slug": "gmail", "name": "Gmail", "meta": {"tools_count": 23, "logo": "https://logos.composio.dev/gmail.png", "description": "long text left out"}},
                            {"slug": "slack", "name": "Slack", "meta": {"tools_count": 40, "logo": "http://insecure.example/slack.png"}}
                        ], "next_cursor": "c2"}))),
                        Some("c2") => (StatusCode::OK, Json(json!({"items": [
                            {"slug": "notion", "name": "Notion"},
                            {"slug": "gmail", "name": "Gmail again"},
                            {"slug": "Bad Slug", "name": "Bad"}
                        ], "next_cursor": null}))),
                        _ => (StatusCode::BAD_REQUEST, Json(json!({"error": {"message": "bad cursor"}}))),
                    };
                }
                (
                    StatusCode::OK,
                    Json(json!({"items": [
                        {"slug": "gmail", "name": "Gmail", "meta": {"tools_count": 23}},
                        {"slug": "slack", "name": "Slack", "meta": {"tools_count": 40}},
                        {"slug": "notion", "name": "Notion", "meta": {"tools_count": 30}}
                    ]})),
                )
            }),
        )
        .route(
            "/api/v3.1/connected_accounts",
            get(|State(f): State<Shared>, h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                f.lock().unwrap().seen.push(("GET".into(), query_path("/api/v3.1/connected_accounts", &q), key_of(&h), Value::Null));
                if let Err(e) = project(&key_of(&h)) {
                    return e;
                }
                let user = q.get("user_ids").cloned().unwrap_or_default();
                let mut items = vec![];
                // the space's own user has Gmail and Notion; the old URL's user has Slack; Linear
                // is only half signed in
                let apps: &[&str] = if user.starts_with("kwant-") { &["gmail", "notion"] } else if user == "u-old" { &["slack"] } else { &[] };
                for a in apps {
                    items.push(json!({"id": format!("ca_{a}"), "status": "ACTIVE", "user_id": user, "toolkit": {"slug": a}}));
                }
                items.push(json!({"id": "ca_linear", "status": "INITIATED", "user_id": user, "toolkit": {"slug": "linear"}}));
                if let Some(t) = q.get("toolkit_slugs") {
                    items.retain(|i| i["toolkit"]["slug"] == json!(t));
                }
                (StatusCode::OK, Json(json!({"items": items, "next_cursor": null})))
            }),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, fake)
}

async fn env_with_fake() -> (Env, Shared) {
    let (url, fake) = fake_composio().await;
    let env = setup_with(|state| state.composio_api = Arc::new(ComposioApi::new(&url))).await;
    (env, fake)
}

fn calls(f: &Shared) -> Vec<(String, String, String, Value)> {
    std::mem::take(&mut f.lock().unwrap().seen)
}

async fn servers(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/mcp/servers", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

async fn composio_entry(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["catalog"].as_array().unwrap().iter().find(|e| e["id"] == "composio").cloned().unwrap()
}

#[tokio::test]
async fn a_key_alone_adds_composio_through_a_session() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "  ak_good \n"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "session", "{v}");
    let user = v["user_id"].as_str().unwrap().to_string();
    assert!(user.starts_with("kwant-") && user.len() > "kwant-".len(), "the space's own Composio user: {user}");
    assert_eq!(v["server"]["name"], "composio");
    assert_eq!(v["server"]["url"], "https://backend.composio.dev/tool_router/trs_1/mcp");
    assert_eq!(v["server"]["enabled"], false, "off for bots until the person turns it on");
    assert!(v["connection_id"].as_str().unwrap().starts_with("mcp:"), "{v}");
    assert!(v["warning"].as_str().unwrap().contains("Composio's servers"));
    assert_eq!(v["tools"][1], "COMPOSIO_MANAGE_CONNECTIONS", "{v}");
    assert!(!v.to_string().contains("ak_good"), "the key is masked: {v}");

    // Exactly one call: the session, for the space's user, with connections managed in it.
    let c = calls(&fake);
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].1, "/api/v3.1/tool_router/session");
    assert_eq!(c[0].2, "ak_good", "the key is sent trimmed");
    assert_eq!(c[0].3, json!({"user_id": user, "manage_connections": {"enable": true}}));

    let list = servers(&env).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(!list.to_string().contains("ak_good"), "the key is masked: {list}");

    let entry = composio_entry(&env).await;
    assert_eq!(entry["added"], true);
    assert_eq!(entry["enabled"], false);
    assert_eq!(entry["mode"], "session", "{entry}");
    assert_eq!(entry["user_id"], json!(user));
    assert_eq!(entry["can_search"], true);
    assert_eq!(entry["can_connect"], true);
    assert_eq!(entry["can_switch"], false);
    assert_eq!(entry["problem"], Value::Null, "{entry}");
    assert_eq!(
        entry["apps_connected"],
        json!([{"slug": "gmail", "name": "Gmail"}, {"slug": "notion", "name": "Notion"}]),
        "{entry}"
    );
    assert!(!entry.to_string().contains("ak_good"));
    // the list asks once a minute
    composio_entry(&env).await;
    let c = calls(&fake);
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].1, format!("/api/v3.1/connected_accounts?limit=200&statuses=ACTIVE&user_ids={user}"));

    // Connect uses the saved session: no new one.
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/apps/slack/connect", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["redirect_url"], "https://connect.composio.dev/link/ln_1");
    let c = calls(&fake);
    assert_eq!(c.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), vec!["/api/v3.1/tool_router/session/trs_1/link"]);
    assert_eq!(c[0].3, json!({"toolkit": "slack"}));

    // Status asks for the session's user.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps/gmail/status", None).await;
    assert_eq!((s, v), (StatusCode::OK, json!({"connected": true})));
    assert!(calls(&fake)[0].1.contains(&format!("user_ids={user}")));

    // Adding again is refused: replace the key instead.
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_good2"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["action"], "replace_key");
    assert!(calls(&fake).is_empty());
}

#[tokio::test]
async fn a_refused_key_saves_nothing() {
    let (env, fake) = env_with_fake().await;
    let add = |key: &'static str| {
        let app = env.app.clone();
        async move { call(&app, "POST", "/api/connections/composio", Some(json!({"api_key": key}))).await }
    };

    let (s, v) = add("ak_wrong").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], REFUSED, "{v}");
    assert_eq!(v["action"], "replace_key");
    assert_eq!(v["key_url"], "https://dashboard.composio.dev");
    assert!(!v["error"].as_str().unwrap().contains("ak_"), "either kind of key is welcome: {v}");

    // Neither kind: tried as a project key, then on Composio Connect.
    let (s, v) = add("uak_x").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("That's a Composio user key"), "{v}");
    assert!(v["error"].as_str().unwrap().contains("It starts with ck_ or ak_."), "{v}");

    let (s, v) = add("ak_scoped").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("isn't allowed to do this"), "{v}");
    assert_eq!(v["action"], "replace_key");

    let (s, v) = add("ak_down").await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
    assert_eq!(v["error"], "Composio couldn't be reached; try again in a minute.");
    assert_eq!(v["action"], "try_again");

    let (s, v) = add("   ").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["action"], "add_composio");
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let c = calls(&fake);
    assert_eq!(c.len(), 5, "the blank keys never reach Composio: {c:?}");
    assert_eq!((c[1].1.as_str(), c[2].1.as_str()), ("/api/v3.1/connected_accounts?limit=1", "/connect/mcp initialize"), "{c:?}");
    assert_eq!(servers(&env).await, json!([]), "nothing saved");
    let entry = composio_entry(&env).await;
    assert_eq!((entry["added"].clone(), entry["mode"].clone()), (json!(false), Value::Null));
    assert!(calls(&fake).is_empty(), "nothing to list before Composio is added");
    for (_, _, _, body) in &c {
        assert!(!body.to_string().contains("ak_"), "the key goes in the header only");
    }

    // A key that can be added after all those works on the first try.
    let (s, v) = add("ak_good").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["server"]["url"], "https://backend.composio.dev/tool_router/trs_1/mcp");
}

#[tokio::test]
async fn composio_out_of_reach_saves_nothing() {
    // Nothing listens on port 9 here.
    let env = setup_with(|state| state.composio_api = Arc::new(ComposioApi::new("http://127.0.0.1:9"))).await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_good"}))).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
    assert_eq!(v["error"], "Composio couldn't be reached; try again in a minute.");
    assert_eq!(v["action"], "try_again");
    assert_eq!(servers(&env).await, json!([]));
}

#[tokio::test]
async fn switch_keeps_the_user_of_the_old_setup() {
    let (env, fake) = env_with_fake().await;
    let old_url = "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-old";
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/connections/composio",
        Some(json!({"url": old_url, "api_key": "ak_good", "enabled": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "url");
    let id = v["server"]["id"].clone();
    assert!(calls(&fake).is_empty(), "a URL setup is saved as before, with no call");

    // Until Switch, listing, searching and status leave it as it is.
    let entry = composio_entry(&env).await;
    assert_eq!(entry["mode"], "url", "{entry}");
    assert_eq!(entry["can_switch"], true);
    assert_eq!(entry["user_id"], "u-old");
    assert_eq!(entry["apps_connected"], json!([{"slug": "slack", "name": "Slack"}]), "{entry}");
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=sl", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "url");
    let (s, _) = call(&env.app, "GET", "/api/connections/composio/apps/slack/status", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(servers(&env).await[0]["url"], old_url, "untouched until Switch");
    assert!(calls(&fake).iter().all(|c| !c.1.starts_with("/api/v3.1/tool_router")), "no session made");

    let (s, v) = call(&env.app, "POST", "/api/connections/composio/switch", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["switched"].clone(), v["mode"].clone(), v["user_id"].clone()), (json!(true), json!("session"), json!("u-old")), "{v}");
    assert_eq!(v["server"]["id"], id, "the same server, so which bots have it doesn't change");
    assert_eq!(v["server"]["enabled"], true, "its switch stays as it was");
    assert_eq!(v["server"]["url"], "https://backend.composio.dev/tool_router/trs_1/mcp");
    let c = calls(&fake);
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].3, json!({"user_id": "u-old", "manage_connections": {"enable": true}}));
    assert_eq!(c[0].2, "ak_good", "the saved key");

    let list = servers(&env).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["url"], "https://backend.composio.dev/tool_router/trs_1/mcp");
    let entry = composio_entry(&env).await;
    assert_eq!((entry["mode"].clone(), entry["user_id"].clone(), entry["can_switch"].clone()), (json!("session"), json!("u-old"), json!(false)));
    assert_eq!(entry["apps_connected"], json!([{"slug": "slack", "name": "Slack"}]), "apps connected before stay connected");

    // Again: nothing to do, nobody called.
    calls(&fake);
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/switch", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["switched"], false);
    assert!(calls(&fake).is_empty());

    // Connect and status now go through the session and its user.
    let (s, _) = call(&env.app, "POST", "/api/connections/composio/apps/gmail/connect", None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps/slack/status", None).await;
    assert_eq!((s, v), (StatusCode::OK, json!({"connected": true})));
    let c: Vec<_> = calls(&fake).into_iter().map(|c| c.1).collect();
    assert_eq!(c[0], "/api/v3.1/tool_router/session/trs_1/link");
    assert!(c[1].contains("user_ids=u-old"), "{c:?}");
}

#[tokio::test]
async fn switch_without_a_user_picks_the_spaces_own_and_a_refused_key_changes_nothing() {
    let (env, fake) = env_with_fake().await;
    let old_url = "https://backend.composio.dev/v3/mcp/abc/mcp";
    let (s, _) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"url": old_url, "api_key": "ak_wrong"}))).await;
    assert_eq!(s, StatusCode::OK);
    let entry = composio_entry(&env).await;
    assert_eq!(entry["problem"]["action"], "switch", "{entry}");
    assert_eq!(entry["can_connect"], false);

    let (s, v) = call(&env.app, "POST", "/api/connections/composio/switch", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], REFUSED);
    assert_eq!(servers(&env).await[0]["url"], old_url, "a refused key changes nothing");

    // A key given with the switch is used and saved.
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/switch", Some(json!({"api_key": "ak_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["user_id"].as_str().unwrap().starts_with("kwant-"), "{v}");
    let c = calls(&fake);
    assert_eq!(c.last().unwrap().2, "ak_good");
    let (s, _) = call(&env.app, "GET", "/api/connections/composio/apps?q=gm", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(calls(&fake).iter().all(|c| c.2 == "ak_good"), "the new key is the saved one");

    // Not added at all: Switch says to add Composio.
    let (env2, _) = env_with_fake().await;
    let (s, v) = call(&env2.app, "POST", "/api/connections/composio/switch", None).await;
    assert_eq!((s, v["action"].clone()), (StatusCode::CONFLICT, json!("add_composio")), "{v}");
}

#[tokio::test]
async fn search_returns_connected_per_app_from_one_call() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let user = v["user_id"].as_str().unwrap().to_string();
    calls(&fake);

    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=a", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let connected: Vec<(String, bool)> = v["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["slug"].as_str().unwrap().to_string(), a["connected"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        connected,
        vec![("gmail".into(), true), ("slack".into(), false), ("notion".into(), true)],
        "{v}"
    );
    assert_eq!(v["connected_checked"], true);
    assert_eq!(v["can_connect"], true, "the session can always connect");
    assert_eq!(v["mode"], "session");
    assert_eq!(v["apps"][0]["tools_count"], 23);

    let c: Vec<_> = calls(&fake).into_iter().map(|c| c.1).collect();
    assert_eq!(
        c,
        vec![
            "/api/v3.1/toolkits?limit=20&search=a&sort_by=usage".to_string(),
            format!("/api/v3.1/connected_accounts?limit=200&statuses=ACTIVE&user_ids={user}"),
        ],
        "one account call for the whole page"
    );

    // The search filled the list's cache: listing now costs nothing.
    let entry = composio_entry(&env).await;
    assert_eq!(entry["apps_connected"].as_array().unwrap().len(), 2, "{entry}");
    assert!(calls(&fake).is_empty());
}

#[tokio::test]
async fn replacing_the_key_checks_it_first() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_good", "enabled": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let user = v["user_id"].clone();
    let id = v["server"]["id"].clone();
    calls(&fake);
    let put = |key: &'static str| {
        let app = env.app.clone();
        async move { call(&app, "PUT", "/api/connections/composio/key", Some(json!({"api_key": key}))).await }
    };
    let searched_with = || {
        let app = env.app.clone();
        let fake = fake.clone();
        async move {
            calls(&fake);
            let (s, _) = call(&app, "GET", "/api/connections/composio/apps?q=gm", None).await;
            assert_eq!(s, StatusCode::OK);
            calls(&fake)[0].2.clone()
        }
    };

    let (s, v) = put("ak_wrong").await;
    assert_eq!((s, v["error"].clone(), v["action"].clone()), (StatusCode::CONFLICT, json!(REFUSED), json!("replace_key")));
    assert_eq!(searched_with().await, "ak_good", "a refused key changes nothing");
    let (s, v) = put("").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    // Same Composio project: the session stays.
    let (s, v) = put("ak_good2").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["session_renewed"].clone(), v["user_id"].clone()), (json!(false), user.clone()));
    assert_eq!(v["server"]["url"], "https://backend.composio.dev/tool_router/trs_1/mcp");
    assert!(!v.to_string().contains("ak_good2"), "masked: {v}");
    assert_eq!(searched_with().await, "ak_good2");

    // Another project: a new session for the same user, on the same server.
    calls(&fake);
    let (s, v) = put("ak_other").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["session_renewed"].clone(), v["user_id"].clone()), (json!(true), user.clone()));
    assert_eq!(v["server"]["id"], id);
    assert_eq!(v["server"]["enabled"], true);
    assert_eq!(v["server"]["url"], "https://backend.composio.dev/tool_router/trs_2/mcp");
    let c = calls(&fake);
    assert_eq!(c.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), vec!["/api/v3.1/tool_router/session/trs_1", "/api/v3.1/tool_router/session"]);
    assert_eq!(c[1].3["user_id"], user);
    let (s, _) = call(&env.app, "POST", "/api/connections/composio/apps/gmail/connect", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(calls(&fake)[0].1, "/api/v3.1/tool_router/session/trs_2/link");

    // A URL setup: the key is checked, then only the header changes.
    let (env2, fake2) = env_with_fake().await;
    let old_url = "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-old";
    call(&env2.app, "POST", "/api/connections/composio", Some(json!({"url": old_url, "api_key": "ak_good"}))).await;
    let (s, v) = call(&env2.app, "PUT", "/api/connections/composio/key", Some(json!({"api_key": "ak_wrong"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (s, v) = call(&env2.app, "PUT", "/api/connections/composio/key", Some(json!({"api_key": "ak_other"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["mode"].clone(), v["server"]["url"].clone()), (json!("url"), json!(old_url)));
    let c: Vec<_> = calls(&fake2).into_iter().map(|c| (c.1, c.2)).collect();
    assert_eq!(
        c,
        vec![
            ("/api/v3.1/connected_accounts?limit=1".to_string(), "ak_wrong".to_string()),
            ("/api/v3.1/connected_accounts?limit=1".to_string(), "ak_other".to_string()),
        ]
    );
}

#[tokio::test]
async fn switch_and_replace_key_are_person_only() {
    let (env, fake) = env_with_fake().await;
    call(&env.app, "POST", "/api/connections/composio", Some(json!({"url": "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-old", "api_key": "ak_good"}))).await;
    assert_eq!(call_remote(&env.app, "POST", "/api/connections/composio/switch").await, StatusCode::UNAUTHORIZED);
    assert_eq!(call_remote(&env.app, "PUT", "/api/connections/composio/key").await, StatusCode::UNAUTHORIZED);

    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let folder = tempfile::tempdir().unwrap();
    let (_, token) = mcp_runtime::prepare_turn_with_self(&env.state, &bot, None, Some(folder.path().to_path_buf()), Some(scope))
        .await
        .unwrap();
    let bearer = format!("Bearer {token}");
    for (method, path, body) in [
        ("POST", "/api/connections/composio/switch", json!({})),
        ("PUT", "/api/connections/composio/key", json!({"api_key": "ak_other"})),
        ("POST", "/api/connections/composio", json!({"api_key": "ak_other"})),
    ] {
        let (s, v) = call_with(&env.app, method, path, Some(body), &[("authorization", &bearer)]).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
    }
    assert!(calls(&fake).is_empty(), "a bot's token never reaches Composio");
    assert_eq!(servers(&env).await[0]["url"], "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-old");
}

/// A GET with optional If-None-Match: status, ETag, JSON body.
async fn get_list(env: &Env, etag: Option<&str>) -> (StatusCode, Option<String>, Value) {
    let mut b = axum::http::Request::builder().method("GET").uri("/api/connections/composio/apps");
    if let Some(t) = etag {
        b = b.header("if-none-match", t);
    }
    let resp = env.app.clone().oneshot(b.body(axum::body::Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let tag = resp.headers().get("etag").map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, tag, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn without_a_query_the_whole_catalogue_comes_back_for_autocomplete() {
    let (env, fake) = env_with_fake().await;
    // Before setup: Composio's popular apps, bundled with kwant, no Composio call.
    let (s, _, v) = get_list(&env, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["source"].clone(), v["mode"].clone(), v["can_connect"].clone()), (json!("popular"), Value::Null, json!(false)), "{v}");
    assert_eq!(v["count"], 120);
    assert_eq!(v["apps"][0]["slug"], "gmail");
    assert!(v["apps"].as_array().unwrap().iter().all(|a| a["connected"] == false), "{v}");
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=gma", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let slugs: Vec<&str> = v["apps"].as_array().unwrap().iter().map(|a| a["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, vec!["gmail", "figma"], "{v}");
    assert_eq!(v["source"], "popular");
    assert!(calls(&fake).is_empty());

    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let user = v["user_id"].as_str().unwrap().to_string();
    calls(&fake);

    let (s, etag, v) = get_list(&env, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let etag = etag.expect("an ETag");
    assert_eq!(
        v["apps"],
        json!([
            {"slug": "gmail", "name": "Gmail", "logo": "https://logos.composio.dev/gmail.png", "tools_count": 23, "connected": true},
            {"slug": "slack", "name": "Slack", "tools_count": 40, "connected": false},
            {"slug": "notion", "name": "Notion", "connected": true}
        ]),
        "compact, every page, no duplicates, no bad slugs, https logos only: {v}"
    );
    assert_eq!(v["count"], 3);
    assert_eq!(v["connected_checked"], true);
    assert_eq!(v["mode"], "session");
    assert_eq!(v["source"], "all", "a project key lists Composio's whole catalogue");
    assert!(v["fetched_at"].is_string(), "{v}");
    let c: Vec<_> = calls(&fake).into_iter().map(|c| c.1).collect();
    assert_eq!(
        c,
        vec![
            "/api/v3.1/toolkits?limit=500&sort_by=usage".to_string(),
            "/api/v3.1/toolkits?cursor=c2&limit=500&sort_by=usage".to_string(),
            format!("/api/v3.1/connected_accounts?limit=200&statuses=ACTIVE&user_ids={user}"),
        ]
    );

    // Unchanged: 304 with the same ETag and no Composio call.
    let (s, again, v) = get_list(&env, Some(&etag)).await;
    assert_eq!((s, again.as_deref(), v), (StatusCode::NOT_MODIFIED, Some(etag.as_str()), Value::Null));
    let (s, _, _) = get_list(&env, Some(&format!("W/{etag}, \"other\""))).await;
    assert_eq!(s, StatusCode::NOT_MODIFIED, "weak and listed tags match too");
    let (s, _, v) = get_list(&env, Some("\"stale\"")).await;
    assert_eq!((s, v["count"].clone()), (StatusCode::OK, json!(3)));
    assert!(calls(&fake).is_empty(), "the list is kept a day and the connected apps a minute");

    // A blank q is still a mistake; q searches as before.
    let (s, _) = call(&env.app, "GET", "/api/connections/composio/apps?q=", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=gm", None).await;
    assert_eq!((s, v["apps"].as_array().unwrap().len()), (StatusCode::OK, 3), "{v}");

    // Another key gets its own list; a refused key gets the usual 409.
    let (s, _) = call(&env.app, "PUT", "/api/connections/composio/key", Some(json!({"api_key": "ak_good2"}))).await;
    assert_eq!(s, StatusCode::OK);
    calls(&fake);
    let (s, etag2, _) = get_list(&env, Some(&etag)).await;
    assert_eq!(s, StatusCode::OK, "a new key is a new list");
    assert_ne!(etag2.as_deref(), Some(etag.as_str()));
    assert_eq!(calls(&fake).iter().filter(|c| c.1.starts_with("/api/v3.1/toolkits")).count(), 2);
}

/// The figures for impl.md: a Composio-sized catalogue (1,000 apps) as the UI gets it, with the
/// response size raw and gzipped, and the time cold (fetched from the fake) and warm (from memory).
#[tokio::test]
async fn a_thousand_app_catalogue_is_small_and_served_from_memory() {
    use std::io::Write;
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_big"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    calls(&fake);
    let raw = |env: &Env| {
        let app = env.app.clone();
        async move {
            let req = axum::http::Request::builder().uri("/api/connections/composio/apps").body(axum::body::Body::empty()).unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap()
        }
    };
    let t = std::time::Instant::now();
    let cold = raw(&env).await;
    let cold_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = std::time::Instant::now();
    let warm = raw(&env).await;
    let warm_ms = t.elapsed().as_secs_f64() * 1000.0;
    let v: Value = serde_json::from_slice(&warm).unwrap();
    assert_eq!(v["count"], 1000);
    assert_eq!(v["apps"][0], json!({"slug": "app_0000", "name": "Example App 0", "logo": "https://logos.composio.dev/api/app_0000", "tools_count": 0, "connected": false}));
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&warm).unwrap();
    let gz = gz.finish().unwrap();
    eprintln!(
        "composio list: {} apps, {} bytes raw, {} bytes gzipped, cold {cold_ms:.1} ms, warm {warm_ms:.1} ms",
        v["count"], warm.len(), gz.len()
    );
    assert_eq!(cold.len(), warm.len());
    assert!(gz.len() < 64 * 1024, "about 60 KB gzipped at most: {}", gz.len());
    assert!(warm_ms < 1000.0, "the warm list comes from memory: {warm_ms} ms");
    let toolkit_calls = calls(&fake).iter().filter(|c| c.1.starts_with("/api/v3.1/toolkits")).count();
    assert_eq!(toolkit_calls, 2, "two pages, fetched once");
}

/// The `/connect/mcp` calls since the last look, as (what, key).
fn connect_calls(f: &Shared) -> Vec<(String, String)> {
    calls(f).into_iter().map(|c| (c.1, c.2)).collect()
}

fn handshake(key: &str) -> Vec<(String, String)> {
    ["initialize", "notifications/initialized"].iter().map(|m| (format!("/connect/mcp {m}"), key.to_string())).collect()
}

#[tokio::test]
async fn a_composio_connect_key_is_checked_over_mcp_and_saved_for_composio_connect() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": " ck_good\n"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["mode"], "connect", "{v}");
    assert_eq!(v["user_id"], Value::Null, "the key is the person's own Composio account");
    assert_eq!(v["server"]["name"], "composio");
    assert_eq!(v["server"]["url"], "https://connect.composio.dev/mcp", "bots use Composio Connect itself");
    assert_eq!(v["server"]["enabled"], false, "off for bots until the person turns it on");
    assert_eq!(v["tools"][1], "COMPOSIO_MANAGE_CONNECTIONS", "{v}");
    assert!(v["warning"].as_str().unwrap().contains("Composio's servers"));
    assert!(!v.to_string().contains("ck_good"), "the key is masked: {v}");

    // The check is the MCP handshake and tools/list, with the key trimmed, and nothing else.
    let mut want = handshake("ck_good");
    want.push(("/connect/mcp tools/list".into(), "ck_good".into()));
    assert_eq!(connect_calls(&fake), want);

    let list = servers(&env).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["url"], "https://connect.composio.dev/mcp");
    assert!(!list.to_string().contains("ck_good"), "the key is masked: {list}");
    let saved = env.state.db.list_mcp_servers(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert_eq!(
        saved[0].headers,
        std::collections::BTreeMap::from([("x-consumer-api-key".to_string(), "ck_good".to_string())]),
        "the key is saved in Composio Connect's header"
    );

    // The tile: one list call over the popular apps, kept a minute.
    fake.lock().unwrap().active.insert("gmail".into());
    let entry = composio_entry(&env).await;
    assert_eq!(entry["mode"], "connect", "{entry}");
    assert_eq!(entry["user_id"], Value::Null);
    assert_eq!((entry["can_search"].clone(), entry["can_connect"].clone(), entry["can_switch"].clone()), (json!(true), json!(true), json!(false)));
    assert_eq!(entry["problem"], Value::Null, "{entry}");
    assert_eq!(entry["apps_connected"], json!([{"slug": "gmail", "name": "Gmail"}]), "{entry}");
    let c = calls(&fake);
    assert_eq!(c.iter().filter(|x| x.1 == "/connect/mcp tools/call list").count(), 1, "{c:?}");
    let asked = c.iter().find(|x| x.1 == "/connect/mcp tools/call list").unwrap();
    assert_eq!(asked.3["toolkits"].as_array().unwrap().len(), 120);
    assert!(c.iter().all(|x| x.1.starts_with("/connect/mcp")), "never the project API: {c:?}");
    composio_entry(&env).await;
    assert!(calls(&fake).is_empty(), "kept a minute");

    // The list: Composio's popular apps with connected, from the same minute.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["source"].clone(), v["mode"].clone(), v["can_connect"].clone(), v["count"].clone()), (json!("popular"), json!("connect"), json!(true), json!(120)));
    assert_eq!(v["apps"][0], json!({"slug": "gmail", "name": "Gmail", "rank": 0, "logo": v["apps"][0]["logo"], "connected": true}));
    assert!(calls(&fake).is_empty(), "from the tile's minute");
}

#[tokio::test]
async fn composio_connect_connects_and_checks_apps_through_manage_connections() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ck_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    calls(&fake);

    // Connect: MANAGE_CONNECTIONS add, and Composio's sign-in link.
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/apps/slack/connect", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["redirect_url"], "https://connect.composio.dev/link/lk_slack");
    let c = calls(&fake);
    let call_ = c.iter().find(|x| x.1.starts_with("/connect/mcp tools/call")).unwrap();
    assert_eq!(call_.1, "/connect/mcp tools/call add");
    assert_eq!(call_.3, json!({"toolkits": [{"name": "slack", "action": "add"}]}));
    assert!(c.iter().all(|x| x.1 != "/connect/mcp tools/list"), "no tools/list, no search: {c:?}");

    // Status: MANAGE_CONNECTIONS list for the one app; never SEARCH_TOOLS.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps/slack/status", None).await;
    assert_eq!((s, v), (StatusCode::OK, json!({"connected": false})));
    fake.lock().unwrap().active.insert("slack".into());
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps/slack/status", None).await;
    assert_eq!((s, v), (StatusCode::OK, json!({"connected": true})));
    let c = calls(&fake);
    let lists: Vec<&Value> = c.iter().filter(|x| x.1 == "/connect/mcp tools/call list").map(|x| &x.3).collect();
    assert_eq!(lists, vec![&json!({"toolkits": [{"name": "slack", "action": "list"}]}); 2]);

    // A search asks about its own apps only, in one call kept a minute.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=sla", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["apps"][0]["slug"], "slack");
    assert_eq!(v["apps"][0]["connected"], true, "{v}");
    assert_eq!((v["source"].clone(), v["mode"].clone()), (json!("popular"), json!("connect")));
    let c = calls(&fake);
    let lists: Vec<&Value> = c.iter().filter(|x| x.1 == "/connect/mcp tools/call list").map(|x| &x.3).collect();
    assert_eq!(lists.len(), 1, "{c:?}");
    assert!(lists[0]["toolkits"].as_array().unwrap().len() <= 20);
    call(&env.app, "GET", "/api/connections/composio/apps?q=sla", None).await;
    assert!(calls(&fake).is_empty(), "the same search within the minute costs nothing");

    // Composio Connect down: a 502 that says to try again, and the setup stays.
    let (env2, fake2) = env_with_fake().await;
    let (s, v) = call(&env2.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ck_down"}))).await;
    assert_eq!((s, v["action"].clone()), (StatusCode::BAD_GATEWAY, json!("try_again")), "{v}");
    assert_eq!(servers(&env2).await, json!([]));
    assert!(calls(&fake2).iter().all(|c| c.2 == "ck_down"));
}

#[tokio::test]
async fn a_refused_composio_connect_key_saves_nothing() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ck_wrong"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!((v["error"].clone(), v["action"].clone(), v["key_url"].clone()), (json!(REFUSED), json!("replace_key"), json!("https://dashboard.composio.dev")));
    assert!(!v.to_string().contains("ck_wrong"), "{v}");
    assert_eq!(connect_calls(&fake), vec![("/connect/mcp initialize".to_string(), "ck_wrong".to_string())], "a ck_ key is never sent to the project API");
    assert_eq!(servers(&env).await, json!([]), "nothing saved");
    let entry = composio_entry(&env).await;
    assert_eq!(entry["added"], false);

    // A good key then works at once.
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ck_good"}))).await;
    assert_eq!((s, v["mode"].clone()), (StatusCode::OK, json!("connect")), "{v}");
}

#[tokio::test]
async fn replacing_the_key_switches_between_project_and_composio_connect() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ak_good", "enabled": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["server"]["id"].clone();
    let user = v["user_id"].clone();
    calls(&fake);
    let put = |key: &'static str| {
        let app = env.app.clone();
        async move { call(&app, "PUT", "/api/connections/composio/key", Some(json!({"api_key": key}))).await }
    };

    // A refused ck_ changes nothing.
    let (s, v) = put("ck_wrong").await;
    assert_eq!((s, v["action"].clone()), (StatusCode::CONFLICT, json!("replace_key")), "{v}");
    assert_eq!(servers(&env).await[0]["url"], "https://backend.composio.dev/tool_router/trs_1/mcp");

    // ak_ → ck_: the same server, now Composio Connect, its switch as it was.
    let (s, v) = put("ck_good").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["mode"].clone(), v["user_id"].clone()), (json!("connect"), Value::Null), "{v}");
    assert_eq!(v["server"]["id"], id);
    assert_eq!(v["server"]["enabled"], true);
    assert_eq!(v["server"]["url"], "https://connect.composio.dev/mcp");
    assert!(!v.to_string().contains("ck_good"), "masked: {v}");
    let saved = env.state.db.list_mcp_servers(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert_eq!(
        saved[0].headers,
        std::collections::BTreeMap::from([("x-consumer-api-key".to_string(), "ck_good".to_string())]),
        "the project key is gone"
    );
    assert_eq!(composio_entry(&env).await["mode"], "connect");
    calls(&fake);
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/apps/notion/connect", None).await;
    assert_eq!((s, v["redirect_url"].clone()), (StatusCode::OK, json!("https://connect.composio.dev/link/lk_notion")), "{v}");
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/switch", None).await;
    assert_eq!((s, v["switched"].clone(), v["mode"].clone()), (StatusCode::OK, json!(false), json!("connect")), "{v}");

    // ck_ → ak_: a session for the space's own Composio user again.
    calls(&fake);
    let (s, v) = put("ak_other").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["mode"].clone(), v["user_id"].clone(), v["session_renewed"].clone()), (json!("session"), user.clone(), json!(true)), "{v}");
    assert_eq!(v["server"]["id"], id);
    assert_eq!(v["server"]["url"], "https://backend.composio.dev/tool_router/trs_2/mcp");
    let saved = env.state.db.list_mcp_servers(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap();
    assert_eq!(saved[0].headers, std::collections::BTreeMap::from([("x-api-key".to_string(), "ak_other".to_string())]));
    let c = calls(&fake);
    assert_eq!(c.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(), vec!["/api/v3.1/tool_router/session"], "{c:?}");
}

/// The search with a Composio Connect key: a name the popular list doesn't have is asked about
/// with Composio Connect's search, however it is typed, and comes back with Connect.
#[tokio::test]
async fn composio_connect_finds_an_app_beyond_the_popular_list_by_its_name() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ck_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    calls(&fake);

    for (typed, name) in [("ElevenLabs", "ElevenLabs"), ("elevenlabs", "ElevenLabs"), ("Eleven%20Labs", "ElevenLabs")] {
        let (s, v) = call(&env.app, "GET", &format!("/api/connections/composio/apps?q={typed}"), None).await;
        assert_eq!(s, StatusCode::OK, "{typed}: {v}");
        let found: Vec<&Value> = v["apps"].as_array().unwrap().iter().filter(|a| a["slug"] == "elevenlabs").collect();
        assert_eq!(found.len(), 1, "{typed}: {v}");
        let app = found[0];
        assert_eq!(app["name"], name, "{typed}: the name as first typed: {v}");
        assert_eq!(app["logo"], "https://logos.composio.dev/api/elevenlabs");
        assert_eq!(app["connected"], true, "from the search's connection status: {v}");
        assert_eq!(app["found_by_name"], true);
        assert!(app["description"].as_str().unwrap().contains("voices"), "{v}");
        assert_eq!(v["can_connect"], true);
        assert_eq!(v["count"], json!(v["apps"].as_array().unwrap().len()));
        assert!(v.get("hint").is_none(), "{v}");
        let c = calls(&fake);
        let searches: Vec<&Value> = c.iter().filter(|x| x.1 == "/connect/mcp tools/call search").map(|x| &x.3).collect();
        assert!(searches.len() <= 1, "{typed}: one search at most: {c:?}");
        if let Some(asked) = searches.first() {
            assert_eq!(asked["queries"][0]["use_case"], typed.replace("%20", " "), "only the typed name goes: {asked}");
            assert!(c.iter().all(|x| x.2 == "ck_good" && x.1.starts_with("/connect/mcp")), "Composio Connect only, with the key: {c:?}");
        }
    }

    // The same name again within minutes costs nothing.
    call(&env.app, "GET", "/api/connections/composio/apps?q=ElevenLabs", None).await;
    assert!(calls(&fake).iter().all(|c| c.1 != "/connect/mcp tools/call search"), "kept");

    // Connect works for it like for any popular app.
    let (s, v) = call(&env.app, "POST", "/api/connections/composio/apps/elevenlabs/connect", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["redirect_url"], "https://connect.composio.dev/link/lk_elevenlabs");

    // A name Composio has no app for: no apps, Composio's unrelated toolkits left out, and the
    // line to show instead.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=notarealapp123", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["apps"], json!([]), "{v}");
    assert_eq!(v["count"], 0);
    assert_eq!(v["hint"], "Composio has no app called “notarealapp123”. Check the spelling or try the app's full name, or ask a bot to connect it.");
    let c = calls(&fake);
    assert_eq!(c.iter().filter(|x| x.1 == "/connect/mcp tools/call search").count(), 1, "{c:?}");
    assert_eq!(c.iter().filter(|x| x.0 == "HEAD").count(), 1, "then its one app page: {c:?}");
    assert!(c.iter().filter(|x| x.0 == "HEAD").all(|x| x.2.is_empty()), "no key to the app pages: {c:?}");

    // A popular app: the popular rows exactly as before, and no search.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=gmail", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["apps"][0]["slug"], "gmail");
    assert!(v["apps"].as_array().unwrap().iter().all(|a| a.get("found_by_name").is_none()), "{v}");
    assert!(calls(&fake).iter().all(|c| c.1 != "/connect/mcp tools/call search" && c.0 != "HEAD"));
    // A popular app typed as words: its popular row, no call.
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=Google%20Search%20Console", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["apps"].as_array().unwrap().iter().any(|a| a["slug"] == "google_search_console"), "{v}");
    assert!(calls(&fake).iter().all(|c| c.1 != "/connect/mcp tools/call search" && c.0 != "HEAD"));
}

/// Composio Connect can't answer: the public app page still finds the app, and a name nobody can
/// answer about gets the top-apps line.
#[tokio::test]
async fn when_composio_connect_cannot_search_the_app_page_still_finds_it() {
    let (env, fake) = env_with_fake().await;
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(json!({"api_key": "ck_good"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The saved key now fails at Composio Connect.
    let space = kwnt::db::spaces::SpaceId::default_space();
    let mut server = env.state.db.list_mcp_servers(&space).await.unwrap().remove(0);
    server.headers.insert("x-consumer-api-key".into(), "ck_down".into());
    env.state.db.update_mcp_server(&server).await.unwrap();
    calls(&fake);
    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=ElevenLabs", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let app = v["apps"].as_array().unwrap().iter().find(|a| a["slug"] == "elevenlabs").cloned().unwrap_or_else(|| panic!("{v}"));
    assert_eq!(app["name"], "ElevenLabs");
    assert_eq!(app["connected"], false, "unknown without Composio Connect: {v}");
    assert!(calls(&fake).iter().any(|c| c.0 == "HEAD" && c.1 == "/toolkits/elevenlabs"));
}

/// Before Composio is added: the public app page, with nothing but the app's name.
#[tokio::test]
async fn before_setup_an_app_beyond_the_popular_list_is_found_on_its_page() {
    let (env, fake) = env_with_fake().await;
    for typed in ["ElevenLabs", "elevenlabs", "Eleven%20Labs"] {
        let (s, v) = call(&env.app, "GET", &format!("/api/connections/composio/apps?q={typed}"), None).await;
        assert_eq!(s, StatusCode::OK, "{typed}: {v}");
        assert_eq!((v["mode"].clone(), v["can_connect"].clone()), (Value::Null, json!(false)), "{v}");
        let app = v["apps"].as_array().unwrap().iter().find(|a| a["slug"] == "elevenlabs").cloned().unwrap_or_else(|| panic!("{typed}: {v}"));
        assert_eq!(app["name"], "ElevenLabs", "{typed}: {v}");
        assert_eq!(app["logo"], "https://logos.composio.dev/api/elevenlabs");
        assert!(v.get("hint").is_none());
    }
    let c = calls(&fake);
    assert!(!c.is_empty() && c.iter().all(|x| x.0 == "HEAD" && x.1.starts_with("/toolkits/") && x.2.is_empty()), "only app pages, no key: {c:?}");
    // ElevenLabs: its one page (elevenlabs reuses the answer); Eleven Labs: its 3 candidates.
    assert_eq!(c.len(), 4, "at most 3 pages a name, each name once: {c:?}");

    let (s, v) = call(&env.app, "GET", "/api/connections/composio/apps?q=notarealapp123", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["apps"], json!([]));
    assert_eq!(v["hint"], "Composio has no app called “notarealapp123”. Check the spelling or try the app's full name, or ask a bot to connect it.");
    // Pages out of reach: the line says what kwant could check.
    let env2 = setup_with(|state| state.composio_api = Arc::new(ComposioApi::new("http://127.0.0.1:9"))).await;
    let (s, v) = call(&env2.app, "GET", "/api/connections/composio/apps?q=Eleven%20Labs", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["apps"], json!([]), "{v}");
    assert_eq!(v["hint"], "No app called “Eleven Labs” in Composio's top apps. Type its exact name, or ask a bot to connect it.");
}
