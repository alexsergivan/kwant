//! What changed after each action in kwnt's own browser, and the loop stop, through a real
//! Chrome: each `/act` says what is new or gone (or that it went to another page), waits a moment
//! for a slow change, and stops a bot whose last 3 actions changed nothing or who asks a 3rd time
//! in a row for the same action that changed nothing. Opening an address starts over. Skipped when no Chrome is
//! installed.
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
    sandbox::{computer, progress},
    server::{create_router, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const COUNTER_PAGE: &str = "<!doctype html><title>counter</title><p id=n>Count 0</p>\
<button onclick=\"var n=document.getElementById('n');n.textContent='Count '+(Number(n.textContent.slice(6))+1)\">Add</button>\
<button>Nothing</button><button>Also nothing</button>\
<button onclick=\"setTimeout(function(){document.getElementById('s').textContent='Slow done'},500)\">Slow</button><p id=s></p>\
<input name=q placeholder=Search><a href='/other'>Other page</a>";
const OTHER_PAGE: &str = "<!doctype html><title>other</title><p>Elsewhere</p>";

/// A small HTTP server for the pages above on 127.0.0.1, so the test needs no network.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let page = match path.as_str() {
                    "/counter" => COUNTER_PAGE,
                    _ => OTHER_PAGE,
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
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

async fn open(e: &Env, url: &str) -> kwnt::sandbox::BrowserController {
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(url).await.unwrap();
    ctrl
}


async fn post_act(app: &Router, bot: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sandbox/{bot}/act"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn act(e: &Env, body: Value) -> Value {
    let (status, page) = post_act(&e.app, &e.bot.id, body).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page
}

fn click(label: &str) -> Value {
    json!({"action": "click_text", "text": label})
}

#[tokio::test]
async fn each_action_says_what_changed_and_a_loop_is_stopped() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    open(&e, &format!("{base}/counter")).await;

    // New and gone lines
    let page = act(&e, click("Add")).await;
    assert_eq!(page["changed"], "New: \"Count 1\". Gone: \"Count 0\".", "{page}");
    assert!(page["error"].is_null());
    // A change that lands after the click returns still shows
    let page = act(&e, click("Slow")).await;
    assert_eq!(page["changed"], "New: \"Slow done\".", "{page}");
    // Typing changes no text, but it is a change
    let search = page["clickable"].as_array().unwrap().iter().filter_map(Value::as_str).find(|l| l.contains("Search")).unwrap();
    let search = search.trim_start_matches('*');
    let search: u32 = search[1..search.find(']').unwrap()].parse().unwrap();
    let page = act(&e, json!({"action": "type", "ref": search, "text": "boots"})).await;
    assert!(page["changed"].as_str().unwrap().starts_with("No new text"), "{page}");

    // Three actions in a row that change nothing: the third says stop
    let page = act(&e, click("Nothing")).await;
    assert_eq!(page["changed"], "Nothing on the page changed.");
    assert!(page["error"].is_null(), "{page}");
    act(&e, click("Also nothing")).await;
    let page = act(&e, json!({"action": "scroll", "delta_y": 600})).await;
    assert_eq!(page["error"], progress::STALL_STOP, "{page}");
    assert_eq!(page["stalled"], true);

    // The same click that changes the page every time runs every time (Rigel's counter page)
    for n in 2..=6 {
        let page = act(&e, click("Add")).await;
        assert!(page["error"].is_null(), "click {n}: {page}");
        assert_eq!(page["changed"], format!("New: \"Count {n}\". Gone: \"Count {}\".", n - 1), "{page}");
    }
    // The same click that changed nothing twice is refused a third time before it runs, until
    // something else
    act(&e, click("Nothing")).await;
    act(&e, click("Nothing")).await;
    let page = act(&e, click("Nothing")).await;
    assert_eq!(page["error"], progress::REPEAT_STOP, "{page}");
    let page = act(&e, click("Nothing")).await;
    assert_eq!(page["error"], progress::REPEAT_STOP, "still refused");
    act(&e, json!({"action": "key", "key": "Tab"})).await;
    let page = act(&e, click("Nothing")).await;
    assert_eq!(page["error"], progress::REPEAT_STOP, "a key doesn't count as something else");
    let page = act(&e, click("Add")).await;
    assert!(page["error"].is_null() && page["text"].as_str().unwrap().contains("Count 7"), "{page}");
    let page = act(&e, click("Nothing")).await;
    assert!(page["error"].is_null(), "after something else it may again: {page}");

    // Another page
    let page = act(&e, click("Other page")).await;
    assert_eq!(page["changed"], format!("Went to {base}/other (\"other\")."), "{page}");

    // Opening an address starts over
    open(&e, &format!("{base}/counter")).await;
    act(&e, click("Nothing")).await;
    act(&e, click("Nothing")).await;
    open(&e, &format!("{base}/counter")).await;
    let page = act(&e, click("Nothing")).await;
    assert!(page["error"].is_null(), "{page}");
}

/// A real demo shop: logging in says where it went, adding to the cart says what changed.
/// Needs the network.
#[tokio::test]
#[ignore]
async fn saucedemo_says_what_changed() {
    if computer::local_chrome_path().is_none() {
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    open(&e, "https://www.saucedemo.com/").await;
    act(&e, json!({"action": "click_text", "text": "Username"})).await;
    act(&e, json!({"action": "type", "text": "standard_user"})).await;
    act(&e, json!({"action": "click_text", "text": "Password"})).await;
    act(&e, json!({"action": "type", "text": "secret_sauce"})).await;
    let page = act(&e, click("Login")).await;
    assert_eq!(page["changed"], "Went to https://www.saucedemo.com/inventory.html (\"Swag Labs\").", "{page}");
    let page = act(&e, click("Add to cart")).await;
    let changed = page["changed"].as_str().unwrap();
    assert!(changed.contains("New: \"Remove\"") || changed.contains("\"1\""), "{changed}");
    eprintln!("{changed}");
}
