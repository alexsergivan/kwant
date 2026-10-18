//! Names through the API: a duplicate team, bot or channel name in one space is a 409 and an
//! empty one a 400, on create and on rename, each with a `{ "error": … }` body the UI can show.
//! Another space may reuse the name.

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
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

/// The status and the readable `error` text of a refused request.
async fn refused(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, String) {
    let (s, v) = call(app, method, uri, Some(body)).await;
    let text = v["error"].as_str().unwrap_or_else(|| panic!("{method} {uri}: {s} without a readable error: {v}"));
    (s, text.to_string())
}

async fn setup() -> (Router, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state), tmp)
}

fn bot(name: &str) -> Value {
    json!({
        "name": name,
        "title": "Tester",
        "persona_prompt": "Test persona",
        "avatar_sprite": "s",
        "driver_type": "api",
        "preferred_model": "claude",
    })
}

fn team(name: &str, lead: &str) -> Value {
    json!({"name": name, "lead_kwant_id": lead, "member_kwants": [lead]})
}

async fn new_bot(app: &Router, space: &str, name: &str) -> String {
    let v = ok(app, "POST", &format!("/api/kwants?space={space}"), Some(bot(name))).await;
    v["id"].as_str().unwrap().to_string()
}

async fn new_space(app: &Router, name: &str) -> String {
    let v = ok(app, "POST", "/api/spaces", Some(json!({"name": name}))).await;
    v["id"].as_str().unwrap().to_string()
}

async fn team_names(app: &Router, space: &str) -> Vec<String> {
    let v = ok(app, "GET", &format!("/api/teams?space={space}"), None).await;
    v.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn a_duplicate_team_name_is_a_readable_409_on_create_and_rename() {
    let (app, _tmp) = setup().await;
    let app = &app;
    // Default's seed already has a core-engineering team
    let lead = new_bot(app, "spc_default", "Lead").await;
    assert!(team_names(app, "spc_default").await.contains(&"core-engineering".to_string()));
    let before = team_names(app, "spc_default").await;

    let clash = "a team called core-engineering already exists in this space";
    for name in ["core-engineering", "  core-engineering ", "Core-Engineering"] {
        let (s, text) = refused(app, "POST", "/api/teams", team(name, &lead)).await;
        assert_eq!(s, StatusCode::CONFLICT, "{name:?}");
        assert!(text.eq_ignore_ascii_case(clash), "{name:?}: {text}");
    }
    let (s, text) = refused(app, "POST", "/api/teams?space=spc_default", team("core-engineering", &lead)).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, clash));
    assert_eq!(team_names(app, "spc_default").await, before);

    // Renaming another team onto it is refused; keeping a team's own name is not
    let other = ok(app, "POST", "/api/teams", Some(team("design", &lead))).await;
    let other_id = other["id"].as_str().unwrap();
    let (s, text) = refused(app, "PUT", &format!("/api/teams/{other_id}"), team("core-engineering", &lead)).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, clash));
    ok(app, "PUT", &format!("/api/teams/{other_id}"), Some(team("design", &lead))).await;
    let renamed = ok(app, "PUT", &format!("/api/teams/{other_id}"), Some(team("Design", &lead))).await;
    assert_eq!(renamed["name"], "Design");

    // Another space may use the name
    let b = new_space(app, "Side").await;
    let b_lead = new_bot(app, &b, "Bea").await;
    ok(app, "POST", &format!("/api/teams?space={b}"), Some(team("core-engineering", &b_lead))).await;
    assert_eq!(team_names(app, &b).await, ["core-engineering"]);
    let (s, text) = refused(app, "POST", &format!("/api/teams?space={b}"), team("core-engineering", &b_lead)).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, clash));
}

#[tokio::test]
async fn an_empty_team_name_is_a_400_and_names_are_trimmed() {
    let (app, _tmp) = setup().await;
    let app = &app;
    let b = new_space(app, "Side").await;
    let teams = format!("/api/teams?space={b}");
    let lead = new_bot(app, &b, "Lead").await;
    for name in ["", "   ", "\t\n"] {
        let (s, text) = refused(app, "POST", &teams, team(name, &lead)).await;
        assert_eq!((s, text.as_str()), (StatusCode::BAD_REQUEST, "a team needs a name"), "{name:?}");
    }
    assert!(team_names(app, &b).await.is_empty());

    let t = ok(app, "POST", &teams, Some(team("  beta  ", &lead))).await;
    assert_eq!(t["name"], "beta");
    let id = t["id"].as_str().unwrap();
    let (s, text) = refused(app, "PUT", &format!("/api/teams/{id}"), team(" ", &lead)).await;
    assert_eq!((s, text.as_str()), (StatusCode::BAD_REQUEST, "a team needs a name"));
    let renamed = ok(app, "PUT", &format!("/api/teams/{id}"), Some(team(" gamma ", &lead))).await;
    assert_eq!(renamed["name"], "gamma");
    assert_eq!(team_names(app, &b).await, ["gamma"]);
}

#[tokio::test]
async fn bot_names_get_readable_errors_on_create_and_rename() {
    let (app, _tmp) = setup().await;
    let app = &app;
    new_bot(app, "spc_default", "Remy").await;
    let (s, text) = refused(app, "POST", "/api/kwants", bot("remy")).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, "a bot called remy already exists in this space"));
    let (s, text) = refused(app, "POST", "/api/kwants", bot("  ")).await;
    assert_eq!((s, text.as_str()), (StatusCode::BAD_REQUEST, "a bot needs a name"));
    let (s, text) = refused(app, "POST", "/api/kwants", bot("Two words")).await;
    assert_eq!((s, text.as_str()), (StatusCode::BAD_REQUEST, "a bot name can't contain spaces or @"));

    let other = new_bot(app, "spc_default", "Otto").await;
    let (s, text) = refused(app, "PUT", &format!("/api/kwants/{other}"), bot("REMY")).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, "a bot called REMY already exists in this space"));
    let (s, text) = refused(app, "PUT", &format!("/api/kwants/{other}"), bot("")).await;
    assert_eq!((s, text.as_str()), (StatusCode::BAD_REQUEST, "a bot needs a name"));
    ok(app, "PUT", &format!("/api/kwants/{other}"), Some(bot("Otto"))).await;

    let b = new_space(app, "Side").await;
    new_bot(app, &b, "Remy").await;
}

#[tokio::test]
async fn channel_names_get_readable_errors() {
    let (app, _tmp) = setup().await;
    let app = &app;
    let lead = new_bot(app, "spc_default", "Lead").await;
    let channel = |name: &str| json!({"name": name, "lead_kwant_id": lead, "member_kwants": [lead], "routing_mode": "auto"});
    ok(app, "POST", "/api/channels", Some(channel("ops"))).await;

    let (s, text) = refused(app, "POST", "/api/channels", channel(" Ops ")).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, "a channel called #Ops already exists in this space"));
    let (s, text) = refused(app, "POST", "/api/channels", channel("general")).await;
    assert_eq!((s, text.as_str()), (StatusCode::CONFLICT, "a channel called #general already exists in this space"));
    let (s, text) = refused(app, "POST", "/api/channels", channel("  ")).await;
    assert_eq!((s, text.as_str()), (StatusCode::BAD_REQUEST, "a channel needs a name"));

    let b = new_space(app, "Side").await;
    let b_lead = new_bot(app, &b, "Bea").await;
    let in_b = json!({"name": "ops", "lead_kwant_id": b_lead, "member_kwants": [b_lead], "routing_mode": "auto"});
    ok(app, "POST", &format!("/api/channels?space={b}"), Some(in_b)).await;
}
