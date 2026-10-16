//! `/api/profiles`, kwant profile binding and the API lockdown (Origin allowlist, token).

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "profiles-test-token-0123456789";
const SECRET: &str = "sk-ant-api03-SECRETSECRETSECRET0123456789";

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    (create_router(state.clone()), state, tmp)
}

async fn req(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, String, Vec<(String, String)>) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(
            b.body(
                body.map(|v| Body::from(v.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let hdrs = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    (status, text, hdrs)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let (s, text, _) = req(app, method, uri, body, &[]).await;
    (s, serde_json::from_str(&text).unwrap_or(Value::Null), text)
}

#[tokio::test]
async fn system_profiles_exist_and_api_keys_are_write_only() {
    let (app, state, tmp) = setup().await;

    let (s, list, _) = call(&app, "GET", "/api/profiles", None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    for e in ["claude", "codex", "grok"] {
        assert!(ids.contains(&format!("prf_system_{e}").as_str()), "{list}");
    }
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["is_default"] == true && p["kind"] == "system"));

    // Create an api_key profile with a key: the key never comes back
    let (s, created, text) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Work API", "kind": "api_key", "api_key": SECRET})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{text}");
    assert!(!text.contains("SECRET"), "{text}");
    assert_eq!(created["has_key"], true);
    assert_eq!(created["signed_in"], true);
    assert_eq!(created["account_label"], "API key");
    let id = created["id"].as_str().unwrap().to_string();

    let (_, _, text) = call(&app, "GET", "/api/profiles", None).await;
    assert!(!text.contains("SECRET"), "{text}");
    assert_eq!(
        state.db.get_profile_key(&id).await.unwrap().unwrap().0,
        SECRET
    );

    // Replace, then delete the key
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/api/profiles/{id}/key"),
        Some(json!({"api_key": "sk-ant-other-0123456789"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/api/profiles/{id}/key"),
        Some(json!({"api_key": "short"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(&app, "DELETE", &format!("/api/profiles/{id}/key"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list, _) = call(&app, "GET", "/api/profiles", None).await;
    let p = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id.as_str())
        .unwrap()
        .clone();
    assert_eq!(p["has_key"], false);
    assert_eq!(p["signed_in"], false);

    // The key never reaches the db file in a form we print, and nothing leaks into a response body
    assert!(state.db.get_profile_key(&id).await.unwrap().is_none());

    // A cli_home profile gets a private kwnt-managed home
    let (s, created, text) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "codex", "name": "Personal", "kind": "cli_home"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{text}");
    let home = created["home_dir"].as_str().unwrap().to_string();
    assert!(home.starts_with(tmp.path().to_str().unwrap()), "{home}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[tokio::test]
async fn profile_validation_and_delete_rules() {
    let (app, _state, _tmp) = setup().await;
    let bad = [
        json!({"engine": "claude", "name": "x", "kind": "system"}),
        json!({"engine": "gemini", "name": "x", "kind": "api_key"}),
        json!({"engine": "claude", "name": "", "kind": "api_key"}),
        json!({"engine": "claude", "name": "x", "kind": "cli_home", "api_key": SECRET}),
        json!({"engine": "claude", "name": "x", "kind": "cli_home", "home_dir": "relative/dir"}),
        json!({"engine": "claude", "name": "x", "kind": "cli_home", "home_dir": "/"}),
    ];
    for body in bad {
        let (s, v, _) = call(&app, "POST", "/api/profiles", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body} -> {v}");
        assert!(v["error"].is_string());
    }
    // Unknown fields are refused rather than ignored
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "x", "kind": "api_key", "secret": 1})),
    )
    .await;
    assert!(s.is_client_error());

    let (s, a, _) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Work", "kind": "api_key", "is_default": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _, _) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Work", "kind": "api_key"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    let id = a["id"].as_str().unwrap();
    let (_, list, _) = call(&app, "GET", "/api/profiles", None).await;
    let sys = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "prf_system_claude")
        .unwrap()
        .clone();
    assert_eq!(sys["is_default"], false);
    assert_eq!(a["is_default"], true);

    // The default can't be disabled or un-defaulted directly
    let (s, _, _) = call(
        &app,
        "PATCH",
        &format!("/api/profiles/{id}"),
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v, _) = call(
        &app,
        "PATCH",
        &format!("/api/profiles/{id}"),
        Some(json!({"name": "Work 2", "priority": 5})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["name"], "Work 2");

    // System profiles stay; deleting the default hands it back to the system profile
    let (s, _, _) = call(&app, "DELETE", "/api/profiles/prf_system_claude", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(&app, "DELETE", &format!("/api/profiles/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, list, _) = call(&app, "GET", "/api/profiles", None).await;
    let sys = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "prf_system_claude")
        .unwrap()
        .clone();
    assert_eq!(sys["is_default"], true);
    let (s, _, _) = call(&app, "DELETE", &format!("/api/profiles/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Sign-in is only for cli_home profiles
    let (s, _, _) = call(&app, "POST", "/api/profiles/prf_system_claude/login", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn kwants_carry_their_profile_binding() {
    let (app, _state, _tmp) = setup().await;
    let (_, kwants, _) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    // Existing kwants: engine default, failover on
    assert_eq!(k["profile_id"], Value::Null);
    assert_eq!(k["profile_failover"], true);
    let id = k["id"].as_str().unwrap();

    let (_, p, _) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "codex", "name": "Personal", "kind": "api_key"})),
    )
    .await;
    let codex_profile = p["id"].as_str().unwrap();

    let base = json!({
        "name": k["name"], "title": k["title"], "persona_prompt": k["persona_prompt"],
        "soul": k["soul"], "avatar_sprite": k["avatar_sprite"],
    });
    let with = |model: &str, extra: Value| {
        let mut b = base.clone();
        b["preferred_model"] = json!(model);
        for (key, v) in extra.as_object().unwrap() {
            b[key] = v.clone();
        }
        b
    };

    // A claude kwant can't be pinned to a codex profile; unknown profiles are refused
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/api/kwants/{id}"),
        Some(with("claude", json!({"profile_id": codex_profile}))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(
        &app,
        "PUT",
        &format!("/api/kwants/{id}"),
        Some(with("codex", json!({"profile_id": "prf_nope"}))),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v, text) = call(
        &app,
        "PUT",
        &format!("/api/kwants/{id}"),
        Some(with(
            "codex",
            json!({"profile_id": codex_profile, "profile_failover": false}),
        )),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{text}");
    assert_eq!(v["profile_id"], codex_profile);
    assert_eq!(v["profile_failover"], false);

    // Omitted: unchanged. null: back to the engine default.
    let (_, v, _) = call(
        &app,
        "PUT",
        &format!("/api/kwants/{id}"),
        Some(with("codex", json!({}))),
    )
    .await;
    assert_eq!(v["profile_id"], codex_profile);
    let (_, v, _) = call(
        &app,
        "PUT",
        &format!("/api/kwants/{id}"),
        Some(with("codex", json!({"profile_id": null}))),
    )
    .await;
    assert_eq!(v["profile_id"], Value::Null);
    assert_eq!(v["profile_failover"], false);

    // Deleting a profile unpins its kwants
    call(
        &app,
        "PUT",
        &format!("/api/kwants/{id}"),
        Some(with("codex", json!({"profile_id": codex_profile}))),
    )
    .await;
    call(
        &app,
        "DELETE",
        &format!("/api/profiles/{codex_profile}"),
        None,
    )
    .await;
    let (_, kwants, _) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["id"] == id)
        .unwrap()
        .clone();
    assert_eq!(k["profile_id"], Value::Null);
}

#[tokio::test]
async fn foreign_origins_are_refused() {
    let (app, _state, _tmp) = setup().await;
    // No Origin (curl, the CLI agents): fine
    let (s, _, _) = req(
        &app,
        "GET",
        "/api/kwants",
        None,
        &[("host", "127.0.0.1:3000")],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // kwnt's own page and the dev server
    for origin in ["http://127.0.0.1:3000", "http://localhost:5199"] {
        let (s, _, h) = req(
            &app,
            "GET",
            "/api/kwants",
            None,
            &[("host", "127.0.0.1:3000"), ("origin", origin)],
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{origin}");
        assert!(h
            .iter()
            .any(|(k, v)| k == "access-control-allow-origin" && v == origin));
    }
    // Any other site, simple request or preflight
    let (s, _, h) = req(
        &app,
        "POST",
        "/api/chat/send",
        Some(json!({})),
        &[
            ("host", "127.0.0.1:3000"),
            ("origin", "https://evil.example"),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(!h.iter().any(|(k, _)| k == "access-control-allow-origin"));
    let (s, _, _) = req(
        &app,
        "OPTIONS",
        "/api/profiles",
        None,
        &[
            ("host", "127.0.0.1:3000"),
            ("origin", "https://evil.example"),
            ("access-control-request-method", "POST"),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // The UI itself (not /api) is never blocked
    let (s, _, _) = req(
        &app,
        "GET",
        "/",
        None,
        &[("origin", "https://evil.example")],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn remote_requests_need_the_token() {
    let (app, _state, _tmp) = setup().await;
    let remote = ("host", "kwnt.example.com");

    let (s, _, _) = req(&app, "GET", "/api/kwants", None, &[remote]).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // A local proxy forwarding a remote request is remote
    let (s, _, _) = req(
        &app,
        "GET",
        "/api/kwants",
        None,
        &[
            ("host", "127.0.0.1:3000"),
            ("x-forwarded-for", "203.0.113.9"),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Health stays open for probes
    let (s, _, _) = req(&app, "GET", "/api/health", None, &[remote]).await;
    assert_eq!(s, StatusCode::OK);

    let (s, body, _) = req(&app, "GET", "/api/auth/status", None, &[remote]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"auth_required": true, "authenticated": false})
    );

    let bearer = format!("Bearer {TOKEN}");
    let (s, _, _) = req(
        &app,
        "GET",
        "/api/kwants",
        None,
        &[remote, ("authorization", &bearer)],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = req(
        &app,
        "GET",
        "/api/kwants",
        None,
        &[remote, ("authorization", "Bearer wrong-token-0123456789")],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // The UI trades the token for an HttpOnly cookie, which EventSource and WebSockets send
    let (s, _, _) = req(
        &app,
        "POST",
        "/api/auth/session",
        Some(json!({"token": "nope-nope-nope-nope"})),
        &[remote],
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, h) = req(
        &app,
        "POST",
        "/api/auth/session",
        Some(json!({"token": TOKEN})),
        &[
            remote,
            ("origin", "https://kwnt.example.com"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let cookie = h
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict")
            && cookie.contains("Secure"),
        "{cookie}"
    );
    let pair = cookie.split(';').next().unwrap().to_string();
    let (s, _, _) = req(
        &app,
        "GET",
        "/api/kwants",
        None,
        &[remote, ("cookie", &pair)],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, body, _) = req(
        &app,
        "GET",
        "/api/auth/status",
        None,
        &[remote, ("cookie", &pair)],
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["authenticated"],
        true
    );

    // Local requests (the UI on this machine, agents curling the browser API) need nothing
    let (s, _, _) = req(
        &app,
        "GET",
        "/api/kwants",
        None,
        &[("host", "localhost:3000")],
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, body, _) = req(
        &app,
        "GET",
        "/api/auth/status",
        None,
        &[("host", "localhost:3000")],
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"auth_required": false, "authenticated": true})
    );
}
