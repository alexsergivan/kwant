//! Settings → Computer API and local-mode shell approval. One test function, because it changes
//! process environment (Chrome path, display, KWNT_COMPUTER_MODE) step by step.
//! The real-Chrome side of local mode is in `local_chrome_test.rs`.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    sandbox::computer::ComputerMode,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::tempdir;
use tower::ServiceExt;

async fn call(app: &axum::Router, method: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri("/api/settings/computer")
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn put_window(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings/computer/window")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn next_approval(rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> (String, Value) {
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("approval card")
            .unwrap();
        if let RuntimeEvent::ApprovalRequired {
            id,
            tool_name,
            details,
        } = event
        {
            assert_eq!(tool_name, "execute_shell");
            return (id, details);
        }
    }
}

async fn resolve(app: &axum::Router, token: &str, id: &str, decision: &str) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/approvals/{id}/resolve"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(json!({ "decision": decision }).to_string()))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn computer_mode_api_and_local_shell_approval() {
    let tmp = tempdir().unwrap();
    let fake_chrome = tmp.path().join("fake-chrome");
    std::fs::write(&fake_chrome, "").unwrap();
    std::env::remove_var("KWNT_COMPUTER_MODE");
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    std::env::set_var("KWNT_CHROME_PATH", tmp.path().join("missing-chrome"));

    let db = Database::init(tmp.path()).await.unwrap();
    // a configured browser URL makes vm available without Docker on the test machine
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let router = LayaRouter::new(config.laya_url.clone());
    let state = AppState::new(config, db.clone(), tools, PermissionBroker::new(), router);
    let app = create_router(state.clone());

    // GET: the contract shape; no Chrome → local unavailable with a reason
    let (status, view) = call(&app, "GET", None).await;
    assert_eq!(status, StatusCode::OK);
    let expected_default = if cfg!(target_os = "macos") {
        "local"
    } else {
        "vm"
    };
    assert_eq!(view["mode"], expected_default);
    assert_eq!(view["available"], json!({"local": false, "vm": true}));
    assert!(view["reasons"]["local"]
        .as_str()
        .unwrap()
        .contains("KWNT_CHROME_PATH"));
    assert_eq!(view["reasons"]["vm"], Value::Null);
    // VM mode is honest about what it contains: not the Claude/Codex/Grok CLIs (yet), and the
    // built-in shell only when Docker can run its executor (not in the compose stack).
    let isolates = if kwnt::sandbox::SandboxManager::new().is_docker_available().await {
        json!(["browser", "kwnt_shell"])
    } else {
        json!(["browser"])
    };
    assert_eq!(view["isolates"], isolates);
    assert_eq!(view["window"]["visible"], false);
    if cfg!(target_os = "linux") {
        // no display: Chrome can still run in the app, just not as a window
        assert_eq!(view["window"]["available"], false);
        assert!(view["window"]["reason"].as_str().unwrap().contains("display"));
    }

    // PUT: bad bodies and unavailable modes are 400 and save nothing
    for body in [
        json!({"mode": "docker"}),
        json!({"mode": "VM"}),
        json!({}),
        json!("vm"),
    ] {
        let (status, err) = call(&app, "PUT", Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(err["error"].is_string(), "{body}");
    }
    let (status, err) = call(&app, "PUT", Some(json!({"mode": "local"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("Chrome"));
    assert_eq!(db.get_setting("computer_mode").await.unwrap(), None);

    // env default applies while nothing is saved
    std::env::set_var("KWNT_COMPUTER_MODE", "local");
    assert_eq!(call(&app, "GET", None).await.1["mode"], "local");

    // a saved choice wins over the env default
    let (status, view) = call(&app, "PUT", Some(json!({"mode": "vm"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["mode"], "vm");
    assert_eq!(
        view["isolates"], isolates,
        "PUT returns the same shape"
    );
    assert_eq!(
        db.get_setting("computer_mode").await.unwrap().as_deref(),
        Some("vm")
    );
    assert_eq!(call(&app, "GET", None).await.1["mode"], "vm");
    assert_eq!(state.browser_service.mode(), ComputerMode::Vm);

    // the window switch is local-only
    let (status, err) = put_window(&app, json!({"visible": true})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("Local computer"), "{err}");

    // with a Chrome (no display needed), local becomes available and switching works at runtime
    std::env::set_var("KWNT_CHROME_PATH", &fake_chrome);
    let (status, view) = call(&app, "PUT", Some(json!({"mode": "local"}))).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["mode"], "local");
    assert_eq!(view["available"]["local"], true);
    assert_eq!(state.browser_service.mode(), ComputerMode::Local);

    // a window needs a display; a bad body is 400 too. Neither changes anything.
    for body in [json!({}), json!({"visible": "yes"})] {
        assert_eq!(put_window(&app, body).await.0, StatusCode::BAD_REQUEST);
    }
    if cfg!(target_os = "linux") {
        let (status, err) = put_window(&app, json!({"visible": true})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(err["error"].as_str().unwrap().contains("display"), "{err}");
        assert!(!state.browser_service.local_window());
    }

    // local shell: every command needs approval; a denied one does not run
    let mut rx = state.event_sender.subscribe();
    let marker = tmp.path().join("workspace").join("ran.txt");
    let run = |cmd: &'static str| {
        let state = state.clone();
        tokio::spawn(async move {
            kwnt::server::routes::execute_tool(
                &state,
                None,
                "execute_shell",
                &json!({ "command": cmd }),
            )
            .await
        })
    };
    let denied = run("touch ran.txt");
    let (id, details) = next_approval(&mut rx).await;
    assert_eq!(details["source"], "local_shell");
    assert_eq!(details["arguments"]["command"], "touch ran.txt");
    resolve(&app, state.api_token.reveal(), &id, "deny").await;
    assert!(denied.await.unwrap().is_err());
    assert!(!marker.exists(), "a denied command must not run");

    // "always allow" runs this command only; the next one asks again
    let allowed = run("touch ran.txt && echo local-ok");
    let (id, _) = next_approval(&mut rx).await;
    resolve(&app, state.api_token.reveal(), &id, "always_allow").await;
    let out = allowed.await.unwrap().unwrap();
    assert!(out["stdout"].as_str().unwrap().contains("local-ok"));
    assert!(marker.exists());
    let again = run("echo again");
    let (id, _) = next_approval(&mut rx).await;
    resolve(&app, state.api_token.reveal(), &id, "allow_once").await;
    assert!(again.await.unwrap().is_ok());

    // switching back to vm takes effect without a restart
    let (status, view) = call(&app, "PUT", Some(json!({"mode": "vm"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["mode"], "vm");
    assert_eq!(state.browser_service.mode(), ComputerMode::Vm);

    std::env::remove_var("KWNT_COMPUTER_MODE");
    std::env::remove_var("KWNT_CHROME_PATH");
    std::env::remove_var("DISPLAY");
}
