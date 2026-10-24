//! Deleting a bot or team, and undoing a bot action or an import, are person-only (audit
//! 2026-10-20, F-13). Since F-11 they delete saved passwords too, so a bare `curl` from a bot's
//! shell could wipe another bot's logins: a bot's turn token or a wrong token is a 403, no token
//! from another machine a 401, and nothing is deleted. The person's token goes through.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::NewTeam, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::credentials::VAULT_DIR,
    server::{continuity::SelfScope, create_router, mcp_runtime, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    vault: PathBuf,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Start from an asking install (a fresh one asks only when risky)
    db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "ask").await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    Env { app: create_router(state.clone()), state, vault: data.join(VAULT_DIR), _tmp: tmp }
}

/// A request from this machine (no remote peer), with an optional bearer.
async fn call(app: &Router, method: &str, uri: &str, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let resp = app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// `call` from another machine (a non-loopback Host), without a token.
async fn call_remote(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json").header("host", "kwnt.example.com");
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Env {
    fn token(&self) -> &str {
        self.state.api_token.reveal()
    }
    async fn bots(&self) -> Vec<kwnt::db::models::Kwant> {
        self.state.db.list_kwants(&SpaceId::default_space()).await.unwrap()
    }
    async fn save_for(&self, owner: &str) {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/sandbox/{owner}/passwords"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.token()));
        let body = json!({"origin": "https://keep.test", "username": "ku", "password": "DUMMY-PW-KEEP"});
        let resp = self.app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
    fn file(&self, owner: &str) -> PathBuf {
        self.vault.join(format!("{owner}.bin"))
    }
    /// A live turn of `bot`; returns its turn token.
    async fn turn_token(&self, bot: &kwnt::db::models::Kwant, folder: &Path) -> String {
        let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
        let (_, token) = mcp_runtime::prepare_turn_with_self(&self.state, bot, None, Some(folder.to_path_buf()), Some(scope))
            .await
            .unwrap();
        token
    }
}

#[tokio::test]
async fn deletes_and_undos_refuse_no_token_and_bot_tokens() {
    let e = setup().await;
    let bots = e.bots().await;
    let (attacker, victim) = (&bots[0], &bots[1]);
    let team = NewTeam {
        name: "t".into(),
        description: None,
        icon: None,
        lead_kwant_id: victim.id.clone(),
        member_kwants: vec![victim.id.clone()],
        workspace_path: None,
        repo_url: None,
    };
    let team = e.state.db.create_team(&SpaceId::default_space(), &team).await.unwrap().id;
    e.save_for(&victim.id).await;
    e.save_for(&team).await;

    let folder = tempdir().unwrap();
    let turn = e.turn_token(attacker, folder.path()).await;
    let routes = [
        ("DELETE", format!("/api/kwants/{}", victim.id)),
        ("DELETE", format!("/api/teams/{team}")),
        ("POST", "/api/bot-actions/some-action/undo".to_string()),
        ("POST", "/api/imports/some-import/undo".to_string()),
    ];
    for (method, uri) in &routes {
        let (s, v) = call_remote(&e.app, method, uri, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{method} {uri} from another machine without a token: {v}");
        let (s, v) = call(&e.app, method, uri, Some(&turn)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a bot's turn token: {v}");
        let (s, _) = call(&e.app, method, uri, Some("not-the-token")).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri} with a wrong token");
    }
    // Nothing was deleted
    assert!(e.bots().await.iter().any(|b| b.id == victim.id));
    assert!(e.state.db.get_team(&team).await.unwrap().is_some());
    assert!(e.file(&victim.id).exists() && e.file(&team).exists());

    // The person's token goes through to the handlers
    let (s, v) = call(&e.app, "DELETE", &format!("/api/teams/{team}"), Some(e.token())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!e.file(&team).exists());
    let (s, v) = call(&e.app, "DELETE", &format!("/api/kwants/{}", victim.id), Some(e.token())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!e.file(&victim.id).exists());
    for uri in ["/api/bot-actions/some-action/undo", "/api/imports/some-import/undo"] {
        let (s, v) = call(&e.app, "POST", uri, Some(e.token())).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{uri} reaches the handler: {v}");
    }
}

/// Reading and editing stay open on this machine, as before.
#[tokio::test]
async fn reads_and_edits_stay_open_locally() {
    let e = setup().await;
    let bot = e.bots().await.remove(0);
    for uri in ["/api/kwants".to_string(), "/api/teams".to_string(), "/api/bot-actions".to_string(), format!("/api/kwants/{}/usage", bot.id)] {
        let (s, v) = call(&e.app, "GET", &uri, None).await;
        assert_eq!(s, StatusCode::OK, "GET {uri}: {v}");
    }
}
