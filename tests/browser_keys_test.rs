//! A bot's key presses in kwnt's own browser, through a real headless Chrome (0.8.0 F-1): Enter
//! submits a form, and letters, Space, Backspace and Tab still do what they do on a keyboard.
//! Before the fix the bot's Enter went out as a bare `rawKeyDown` with only `key` and `code`:
//! keydown fired, the form never submitted. Skipped when no Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{browser::BrowserController, computer},
    server::{create_router, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// A search form (two fields, no script that submits it) and the page it lands on.
const FORM: &str = "<!doctype html><title>form</title>\
    <form action=/done method=get><input id=q name=q><input id=other name=other><button>Go</button></form>\
    <script>window.keys=[];addEventListener('keydown',e=>keys.push(e.key),true)</script>";

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let page = if path.starts_with("/done") {
                    format!("<!doctype html><title>done</title><p>submitted {}</p>", path.replace('<', ""))
                } else {
                    FORM.to_string()
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/form")
}

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    tmp: TempDir,
}

/// kwnt in Local mode, so the bot's browser is a real Chrome on this machine.
async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting(computer::SETTING_KEY, "local").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    Env { app: create_router(state.clone()), state, bot, tmp }
}

/// The bot's own browser action route, as `browser_act` uses it.
async fn act(e: &Env, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{}/act", e.bot.id))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = e.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn press(e: &Env, key: &str) {
    let (status, v) = act(e, json!({"action": "key", "key": key})).await;
    assert_eq!(status, StatusCode::OK, "{key}: {v}");
    assert_eq!(v["result"], format!("pressed {key}"), "{key}: {v}");
}

async fn eval(ctrl: &BrowserController, js: &str) -> Value {
    let res = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": js, "returnByValue": true})).await.unwrap();
    res["result"]["value"].clone()
}

/// The page's address once it stops being the form (or the form's, after `wait`).
async fn url_after(ctrl: &BrowserController, wait: Duration) -> String {
    let until = tokio::time::Instant::now() + wait;
    loop {
        let url = eval(ctrl, "location.href").await.as_str().unwrap_or_default().to_string();
        if url.contains("/done") || tokio::time::Instant::now() >= until {
            return url;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn a_bots_enter_submits_the_form_and_other_keys_still_type() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let url = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");

    // The old event, a bare rawKeyDown/keyUp with key and code: keydown fires, nothing submits.
    // This is what the bot sent before, so the test below can tell the two apart.
    ctrl.navigate(&url).await.unwrap();
    eval(&ctrl, "document.getElementById('q').focus(); q.value='old'").await;
    for kind in ["rawKeyDown", "keyUp"] {
        ctrl.send_cdp_command("Input.dispatchKeyEvent", json!({"type": kind, "key": "Enter", "code": "Enter"})).await.unwrap();
    }
    assert_eq!(eval(&ctrl, "keys.join()").await, "Enter", "the page saw the key");
    assert!(url_after(&ctrl, Duration::from_millis(1500)).await.ends_with("/form"), "the old event never submitted");

    // The bot's keys now: letters, Space and punctuation type, Backspace deletes
    ctrl.navigate(&url).await.unwrap();
    eval(&ctrl, "document.getElementById('q').focus()").await;
    for key in ["h", "i", "x", "Backspace", "Space", "A", "!", "/"] {
        press(&e, key).await;
    }
    assert_eq!(eval(&ctrl, "q.value").await, "hi A!/");
    assert_eq!(eval(&ctrl, "keys.join(' ')").await, "h i x Backspace   A ! /");

    // Tab moves to the next field, and typing goes there
    press(&e, "Tab").await;
    assert_eq!(eval(&ctrl, "document.activeElement.id").await, "other");
    press(&e, "z").await;
    assert_eq!(eval(&ctrl, "other.value").await, "z");

    // Enter submits the form, with both fields
    press(&e, "Enter").await;
    let landed = url_after(&ctrl, Duration::from_secs(10)).await;
    assert!(landed.ends_with("/done?q=hi+A%21%2F&other=z"), "{landed}");
    assert_eq!(eval(&ctrl, "document.title").await, "done");

    // The default key of the act route is Enter too
    ctrl.navigate(&url).await.unwrap();
    eval(&ctrl, "document.getElementById('q').focus(); q.value='again'").await;
    let (status, v) = act(&e, json!({"action": "key"})).await;
    assert_eq!((status, &v["result"]), (StatusCode::OK, &json!("pressed Enter")), "{v}");
    assert!(url_after(&ctrl, Duration::from_secs(10)).await.ends_with("/done?q=again&other="));

    // A key kwnt can't describe is refused with a hint, and nothing reaches the page
    ctrl.navigate(&url).await.unwrap();
    let (status, v) = act(&e, json!({"action": "key", "key": "Control+a"})).await;
    assert_eq!(status, StatusCode::OK, "the act route answers the page: {v}");
    assert!(v.get("result").is_none(), "{v}");
    assert!(v["error"].as_str().unwrap_or_default().contains("To enter text, type it."), "{v}");
    assert_eq!(eval(&ctrl, "keys.length").await, 0);

    // The person's own v1 route: the same Enter, and a bad key is a 400 with the reason
    eval(&ctrl, "document.getElementById('q').focus(); q.value='person'").await;
    let person = |body: Value| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/sandbox/{}/interact", e.bot.id))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let resp = e.app.clone().oneshot(person(json!({"action": "key", "key": "Unidentified"}))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let resp = e.app.clone().oneshot(person(json!({"action": "key", "key": "Enter", "code": "Enter"}))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(url_after(&ctrl, Duration::from_secs(10)).await.ends_with("/done?q=person&other="));
}
