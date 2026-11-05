//! 0.7: the Laya engine setting. `laya_engine` reads "vega" on a fresh install, saves "standard" or
//! "vega" (anything else is refused and saves nothing), and a saved change reaches the sidecar
//! through the router, which is how a change switches engines live.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::{sidecar::LayaEngine, LayaRouter},
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::tempdir;
use tower::ServiceExt;

async fn settings(app: &Router, state: &AppState, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri("/api/settings")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", state.api_token.reveal()));
    if body.is_none() {
        req = req.header("content-length", "0");
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn laya_engine_defaults_to_vega_and_a_change_reaches_the_sidecar() {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let router = LayaRouter::new(config.laya_url.clone());
    let mut engine = router.engine_watch();
    let state = AppState::new(config, db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), router.clone());
    let app = create_router(state.clone());

    let (status, s) = settings(&app, &state, None).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["laya_engine"], "vega");
    assert_eq!(router.engine(), LayaEngine::Vega);

    let (status, _) = settings(&app, &state, Some(json!({"laya_engine": "standard"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings(&app, &state, None).await.1["laya_engine"], "standard");
    assert_eq!(router.engine(), LayaEngine::Standard);
    assert!(engine.has_changed().unwrap(), "the sidecar hears about the change");
    engine.borrow_and_update();

    // a bad value is refused and changes nothing
    for bad in ["vegaml", "", "Standard; rm -rf /"] {
        let (status, v) = settings(&app, &state, Some(json!({"laya_engine": bad, "work_ahead": false}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(v["error"], "laya_engine must be \"vega\" or \"standard\"");
    }
    let s = settings(&app, &state, None).await.1;
    assert_eq!((s["laya_engine"].as_str(), s["work_ahead"].as_bool()), (Some("standard"), Some(true)));
    assert!(!engine.has_changed().unwrap());

    settings(&app, &state, Some(json!({"laya_engine": "vega"}))).await;
    assert_eq!(router.engine(), LayaEngine::Vega);
    assert_eq!(state.db.get_setting("laya_engine").await.unwrap().as_deref(), Some("vega"));
}
