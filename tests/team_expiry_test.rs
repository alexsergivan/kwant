//! 0.7.7 clocks (Q-X1): the 24-hour template Undo window and the 1-hour suggestion expiry, moved
//! with the one test clock. Its own binary, because the clock is process-wide; one test, so the
//! steps run in order.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, team_templates, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::tempdir;
use tower::ServiceExt;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn undo_closes_after_24_hours_and_a_suggestion_after_1_hour() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().join("data"), ..Config::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let space = SpaceId::default_space();
    team_templates::set_clock_skew(0);

    // Undo: open at 23 h 59 min, closed at 24 h 1 min
    let (s, made) = call(&app, "POST", "/api/teams/from-template", Some(json!({"template_id": "inbox"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let team = made["team"]["id"].as_str().unwrap();
    let action = made["action_id"].as_str().unwrap();
    team_templates::set_clock_skew(24 * 3600 - 60);
    let (_, t) = call(&app, "GET", &format!("/api/teams/{team}/template"), None).await;
    assert_eq!(t["can_undo"], true, "{t}");
    team_templates::set_clock_skew(24 * 3600 + 60);
    let (_, t) = call(&app, "GET", &format!("/api/teams/{team}/template"), None).await;
    assert_eq!(t["can_undo"], false, "{t}");
    let bots = state.db.list_kwants(&space).await.unwrap().len();
    let (s, v) = call(&app, "POST", &format!("/api/bot-actions/{action}/undo"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("undo_expired")), "{v}");
    assert_eq!(state.db.list_kwants(&space).await.unwrap().len(), bots, "an expired Undo changes nothing");
    assert!(state.db.get_team(team).await.unwrap().is_some());

    // A suggestion: usable at 59 min, gone at 61 min
    team_templates::set_clock_skew(0);
    let root = tmp.path().join("shop");
    std::fs::create_dir_all(root.join("web")).unwrap();
    std::fs::write(root.join("web/package.json"), r#"{"dependencies":{"react":"18.0.0"}}"#).unwrap();
    std::fs::create_dir_all(root.join("api")).unwrap();
    std::fs::write(root.join("api/Cargo.toml"), "[dependencies]\naxum = \"0.7\"\n").unwrap();
    let suggest = || call(&app, "POST", "/api/teams/suggest", Some(json!({"workspace_path": root.to_str().unwrap()})));
    let rename = |s: &Value| -> Value {
        let rows: serde_json::Map<String, Value> =
            s["bots"].as_array().unwrap().iter().map(|r| (r["key"].as_str().unwrap().to_string(), json!({"name": format!("Clock{}", r["key"].as_str().unwrap())}))).collect();
        json!({"suggestion_id": s["suggestion_id"], "edits": {"rows": rows}})
    };
    let (s, late) = suggest().await;
    assert_eq!(s, StatusCode::OK, "{late}");
    let (_, soon) = suggest().await;
    team_templates::set_clock_skew(61 * 60);
    let (s, v) = call(&app, "POST", "/api/teams/from-template", Some(rename(&late))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::GONE, Some("suggestion_expired")), "{v}");
    team_templates::set_clock_skew(59 * 60);
    let (s, v) = call(&app, "POST", "/api/teams/from-template", Some(rename(&soon))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    team_templates::set_clock_skew(0);
}
