//! "Find an app" on the Composio connector: kwnt searches Composio's catalogue and starts an app's
//! sign-in with the person's own Composio key, through person-only routes. Before Composio is
//! added, or without a key, kwnt never sends Composio a request: a fake Composio API counts them.

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

/// Every request the fake Composio API got: method, path with query, x-api-key, JSON body.
type Seen = Arc<Mutex<Vec<(String, String, String, Value)>>>;

fn key_of(h: &HeaderMap) -> String {
    h.get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

async fn fake_composio() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route(
            "/api/v3.1/toolkits",
            get(|State(seen): State<Seen>, h: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                let mut pairs: Vec<_> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
                pairs.sort();
                seen.lock().unwrap().push(("GET".into(), format!("/api/v3.1/toolkits?{}", pairs.join("&")), key_of(&h), Value::Null));
                if key_of(&h) == "ck-wrong" {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": {"message": "bad key"}})));
                }
                (
                    StatusCode::OK,
                    Json(json!({"items": [
                        {"slug": "gmail", "name": "Gmail", "no_auth": false,
                         "meta": {"description": "Mail from Google", "logo": "https://logos.composio.dev/gmail.png", "tools_count": 23}},
                        {"slug": "../evil", "name": "Evil"}
                    ], "next_cursor": null})),
                )
            }),
        )
        .route(
            "/api/v3.1/tool_router/session",
            post(|State(seen): State<Seen>, h: HeaderMap, Json(body): Json<Value>| async move {
                seen.lock().unwrap().push(("POST".into(), "/api/v3.1/tool_router/session".into(), key_of(&h), body));
                (StatusCode::CREATED, Json(json!({"session_id": "trs_1", "mcp": {"type": "http", "url": "https://x"}})))
            }),
        )
        .route(
            "/api/v3.1/tool_router/session/:id/link",
            post(|State(seen): State<Seen>, Path(id): Path<String>, h: HeaderMap, Json(body): Json<Value>| async move {
                seen.lock().unwrap().push(("POST".into(), format!("/api/v3.1/tool_router/session/{id}/link"), key_of(&h), body));
                (StatusCode::CREATED, Json(json!({"redirect_url": "https://connect.composio.dev/link/ln_1", "link_token": "ln_1"})))
            }),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, seen)
}

async fn env_with_fake() -> (Env, Seen) {
    let (url, seen) = fake_composio().await;
    let env = setup_with(|state| state.composio_api = Arc::new(ComposioApi::new(&url))).await;
    (env, seen)
}

async fn add_composio(env: &Env, url: &str, key: Option<&str>) {
    let mut body = json!({"url": url});
    if let Some(k) = key {
        body["api_key"] = json!(k);
    }
    let (s, v) = call(&env.app, "POST", "/api/connections/composio", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn nothing_goes_to_composio_until_it_is_added_with_a_key() {
    let (env, seen) = env_with_fake().await;

    // Not added: the page loads, other servers are added, and finding or connecting an app is refused.
    let (s, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["catalog"][0]["can_search"], false);
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/mcp/servers",
        Some(json!({"name": "other", "transport": "http", "url": "https://example.com/mcp"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(
        &env.app,
        "GET",
        "/api/connections/composio/apps?q=gmail",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("Add the Composio connector first"),
        "{v}"
    );
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/connections/composio/apps/gmail/connect",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Added without a key: still refused, still nothing sent.
    add_composio(
        &env,
        "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-1",
        None,
    )
    .await;
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(list["catalog"][0]["added"], true);
    assert_eq!(list["catalog"][0]["can_search"], false);
    let (s, v) = call(
        &env.app,
        "GET",
        "/api/connections/composio/apps?q=gmail",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("x-api-key"), "{v}");
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/connections/composio/apps/gmail/connect",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    assert!(
        seen.lock().unwrap().is_empty(),
        "no request to Composio: {:?}",
        seen.lock().unwrap()
    );
}

#[tokio::test]
async fn search_and_connect_use_the_persons_key_and_user() {
    let (env, seen) = env_with_fake().await;
    add_composio(
        &env,
        "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-1",
        Some("ck-secret-1"),
    )
    .await;
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    let entry = &list["catalog"][0];
    assert_eq!(
        (entry["can_search"].clone(), entry["can_connect"].clone()),
        (json!(true), json!(true)),
        "{entry}"
    );
    assert!(
        !list.to_string().contains("ck-secret-1"),
        "the key is never listed"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "listing doesn't call Composio"
    );

    // Bad queries and app names are refused before any call.
    for q in ["", "%20%20", &"a".repeat(61)] {
        let (s, _) = call(
            &env.app,
            "GET",
            &format!("/api/connections/composio/apps?q={q}"),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "q={q}");
    }
    for slug in ["Gmail", "a.b", "x%2F..%2Fy"] {
        let (s, _) = call(
            &env.app,
            "POST",
            &format!("/api/connections/composio/apps/{slug}/connect"),
            None,
        )
        .await;
        assert!(
            s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND,
            "{slug}: {s}"
        );
    }
    assert!(seen.lock().unwrap().is_empty());

    let (s, v) = call(
        &env.app,
        "GET",
        "/api/connections/composio/apps?q=g%20mail",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        v["apps"].as_array().unwrap().len(),
        1,
        "an entry with a bad slug is left out: {v}"
    );
    assert_eq!(v["apps"][0]["slug"], "gmail");
    assert_eq!(v["apps"][0]["tools_count"], 23);
    assert_eq!(v["can_connect"], true);
    assert!(v["warning"]
        .as_str()
        .unwrap()
        .contains("Composio's servers"));

    let (s, v) = call(
        &env.app,
        "POST",
        "/api/connections/composio/apps/gmail/connect",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["redirect_url"], "https://connect.composio.dev/link/ln_1");
    assert!(v["warning"]
        .as_str()
        .unwrap()
        .contains("Composio's servers"));

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert!(
        seen.iter().all(|(_, _, key, _)| key == "ck-secret-1"),
        "every call carries the person's key"
    );
    assert_eq!(
        seen[0].1,
        "/api/v3.1/toolkits?limit=20&search=g mail&sort_by=usage"
    );
    assert_eq!(
        (seen[1].1.as_str(), &seen[1].3),
        ("/api/v3.1/tool_router/session", &json!({"user_id": "u-1"}))
    );
    assert_eq!(
        (seen[2].1.as_str(), &seen[2].3),
        (
            "/api/v3.1/tool_router/session/trs_1/link",
            &json!({"toolkit": "gmail"})
        )
    );
}

#[tokio::test]
async fn a_refused_key_or_missing_user_is_explained() {
    let (env, seen) = env_with_fake().await;
    add_composio(
        &env,
        "https://backend.composio.dev/v3/mcp/abc/mcp",
        Some("ck-wrong"),
    )
    .await;
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(list["catalog"][0]["can_connect"], false);

    let (s, v) = call(
        &env.app,
        "GET",
        "/api/connections/composio/apps?q=gmail",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("refused the API key"),
        "{v}"
    );
    assert!(
        !v.to_string().contains("ck-wrong"),
        "the key is never echoed"
    );

    let (s, v) = call(
        &env.app,
        "POST",
        "/api/connections/composio/apps/gmail/connect",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("no user_id"), "{v}");
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "only the search reached Composio"
    );
}

#[tokio::test]
async fn finding_apps_is_person_only() {
    let (env, seen) = env_with_fake().await;
    add_composio(
        &env,
        "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-1",
        Some("ck-secret-1"),
    )
    .await;
    assert_eq!(
        call_remote(&env.app, "GET", "/api/connections/composio/apps?q=gmail").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call_remote(
            &env.app,
            "POST",
            "/api/connections/composio/apps/gmail/connect"
        )
        .await,
        StatusCode::UNAUTHORIZED
    );

    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        ..Default::default()
    };
    let folder = tempfile::tempdir().unwrap();
    let (_, token) = mcp_runtime::prepare_turn_with_self(
        &env.state,
        &bot,
        None,
        Some(folder.path().to_path_buf()),
        Some(scope),
    )
    .await
    .unwrap();
    let bearer = format!("Bearer {token}");
    let (s, _) = call_with(
        &env.app,
        "GET",
        "/api/connections/composio/apps?q=gmail",
        None,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = call_with(
        &env.app,
        "POST",
        "/api/connections/composio/apps/gmail/connect",
        None,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(
        seen.lock().unwrap().is_empty(),
        "a bot's token never reaches Composio"
    );
}
