//! Composio with one key: the person pastes their Composio API key and kwant makes the Tool
//! Router session itself. A refused key saves nothing, an old URL setup stays as it is until
//! Switch (which keeps its Composio user), the key can be replaced, and the app search says which
//! apps are connected with one account call. A fake Composio API stands in for the real one.

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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const REFUSED: &str = "Composio didn't accept this key. Copy the API key from your Composio dashboard, \
    under Project settings › API keys. It starts with ak_.";

#[derive(Default)]
struct Fake {
    /// method, path with sorted query, x-api-key, JSON body
    seen: Vec<(String, String, String, Value)>,
    /// session id → (project, user)
    sessions: HashMap<String, (String, String)>,
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

async fn fake_composio() -> (String, Shared) {
    let fake: Shared = Arc::default();
    let app = Router::new()
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
    assert_eq!(v["key_url"], "https://dashboard.composio.dev/~/project/settings/api-keys");

    let (s, v) = add("uak_x").await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("That's a Composio user key"), "{v}");
    assert!(v["error"].as_str().unwrap().contains("It starts with ak_."), "{v}");

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
    assert_eq!(c.len(), 4, "the blank keys never reach Composio: {c:?}");
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
    // Before setup there is no list: Composio needs the key even for its catalogue.
    let (s, _, v) = get_list(&env, None).await;
    assert_eq!((s, v["action"].clone()), (StatusCode::CONFLICT, json!("add_composio")), "{v}");
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
