//! Numbered refs in kwnt's own browser, through a real Chrome: the clickable list numbers each
//! control with its role (and nearby text when names repeat), a click by ref hits exactly that
//! control (scrolled into view, inside a frame), and a ref that is gone, hidden, renamed or
//! covered clicks nothing and says why, with a fresh list. Payments are judged on the element
//! the ref points to. Skipped when no Chrome is installed. `saucedemo_by_refs` (ignored, needs
//! the network) shops a real demo store by refs.
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
    sandbox::{computer, BrowserError},
    server::{continuity::browser_act_payment, create_router, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const SHOP_PAGE: &str = "<!doctype html><title>shop</title>\
<div class=item><h3>Backpack</h3><p>$29.99</p><button onclick=\"document.title='added backpack'\">Add to cart</button></div>\
<div class=item><h3>Bike Light</h3><p>$9.99</p><button id=light onclick=\"document.title='added light'\">Add to cart</button></div>\
<button onclick=\"document.getElementById('banner').style.display='block'\">Show banner</button>\
<input name=user placeholder=Username>\
<div style='height:2500px'></div><a href='#bottom' onclick=\"document.title='bottom'\">Far away</a>\
<iframe srcdoc=\"<button onclick='parent.document.title=&quot;framed&quot;'>In frame</button>\" style='width:300px;height:60px'></iframe>\
<div id=banner style='display:none;position:fixed;inset:0;background:#fff'>Cookie banner</div>";
const CARD_PAGE: &str = "<!doctype html><title>card</title><form onsubmit=\"return false\">\
<input autocomplete=cc-number placeholder='Card number'><button type=submit>Continue</button></form>";
const PLAIN_PAGE: &str = "<!doctype html><title>plain</title><button>Continue</button>";

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
                    "/shop" => SHOP_PAGE,
                    "/card" => CARD_PAGE,
                    _ => PLAIN_PAGE,
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


async fn pay(e: &Env, args: Value) -> Option<String> {
    browser_act_payment(&e.state, &e.bot.id, &args).await
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

async fn get_page(app: &Router, bot: &str) -> Value {
    let req = Request::builder().uri(format!("/api/sandbox/{bot}/page")).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The ref of the first listed control whose line contains `needle`.
fn ref_of(page: &Value, needle: &str) -> u64 {
    let lines = page["clickable"].as_array().expect("a clickable list");
    let line = lines.iter().filter_map(Value::as_str).find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle} in {page}"));
    line[1..line.find(']').unwrap()].parse().unwrap()
}

async fn eval(ctrl: &kwnt::sandbox::BrowserController, js: &str) {
    ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": js})).await.unwrap();
}

#[tokio::test]
async fn a_click_by_ref_hits_that_control_or_nothing() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = open(&e, &format!("{base}/shop")).await;

    // Role, name, and the product when two buttons share a name
    let page = get_page(&e.app, &e.bot.id).await;
    let list = page["clickable"].to_string();
    assert!(list.contains(r#"button \"Add to cart\" · Backpack"#), "{list}");
    assert!(list.contains(r#"button \"Add to cart\" · Bike Light"#), "{list}");
    assert!(list.contains(r#"field \"Username\""#), "{list}");
    assert!(list.contains(r#"button \"Show banner\"""#), "names that don't repeat carry no context: {list}");

    // The second "Add to cart", which a label can't tell apart
    let light = ref_of(&page, "· Bike Light");
    let (status, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": light})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["title"], "added light", "{page}");
    assert!(page["result"].as_str().unwrap().contains("Add to cart"), "{page}");

    // A copied line works as text too; a control far below is scrolled into view first
    let line = page["clickable"].as_array().unwrap().iter().filter_map(Value::as_str).find(|l| l.contains("Far away")).unwrap().to_string();
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": line})).await;
    assert_eq!(page["title"], "bottom", "{page}");

    // Inside a same-origin frame: the click lands at the frame's offset
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "In frame")})).await;
    assert_eq!(page["title"], "framed", "{page}");

    // Type into a field by its ref
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "type", "ref": ref_of(&page, "Username"), "text": "standard_user"})).await;
    assert!(page["result"].as_str().unwrap().contains("Username"), "{page}");
    eval(&ctrl, "document.title = document.querySelector('[name=user]').value").await;
    assert_eq!(ctrl.sync_location().await.unwrap().1, "standard_user");

    // Covered: a banner over the page; nothing is clicked, the answer says so and lists again
    open(&e, &format!("{base}/shop")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "Show banner")})).await;
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "· Backpack")})).await;
    assert_eq!(page["title"], "shop", "{page}");
    let err = page["error"].as_str().expect("an error");
    assert!(err.contains("covered by div \"Cookie banner\"") && err.contains("pick from the new list"), "{err}");
    assert!(page["clickable"].as_array().is_some_and(|l| !l.is_empty()));

    // Gone, hidden, renamed, unknown: each says why and clicks nothing
    open(&e, &format!("{base}/shop")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let light = ref_of(&page, "· Bike Light") as u32;
    eval(&ctrl, "document.getElementById('light').textContent = 'Remove'").await;
    let err = ctrl.click_ref(light).await.unwrap_err();
    assert!(matches!(err, BrowserError::StaleRef(_)) && err.to_string().contains("now reads \"Remove\""), "{err}");
    eval(&ctrl, "document.getElementById('light').textContent = 'Add to cart'; document.getElementById('light').style.display = 'none'").await;
    assert!(ctrl.click_ref(light).await.unwrap_err().to_string().contains("hidden"));
    eval(&ctrl, "document.getElementById('light').remove()").await;
    assert!(ctrl.click_ref(light).await.unwrap_err().to_string().contains("no longer on the page"));
    assert!(ctrl.click_ref(999).await.unwrap_err().to_string().contains("no ref 999"));
    assert_eq!(ctrl.sync_location().await.unwrap().1, "shop");
    // A reload forgets the list
    open(&e, &format!("{base}/shop")).await;
    assert!(ctrl.click_ref(1).await.unwrap_err().to_string().contains("no list of refs yet"));
}

#[tokio::test]
async fn a_payment_is_judged_on_the_element_a_ref_points_to() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;

    open(&e, &format!("{base}/card")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    let why = pay(&e, json!({"action": "click", "ref": ref_of(&page, "Continue")})).await.expect("Continue on a card form");
    assert!(why.contains("card fields"), "{why}");
    // The shell API refuses it and clicks nothing
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "Continue")})).await;
    assert!(page["payment"].is_string(), "{page}");

    open(&e, &format!("{base}/plain")).await;
    let page = get_page(&e.app, &e.bot.id).await;
    assert_eq!(pay(&e, json!({"action": "click", "ref": ref_of(&page, "Continue")})).await, None);
}

/// A real demo shop by refs: add the second product of several identical "Add to cart"
/// buttons. Needs the network.
#[tokio::test]
#[ignore]
async fn saucedemo_by_refs() {
    if computer::local_chrome_path().is_none() {
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    open(&e, "https://www.saucedemo.com/").await;
    let page = get_page(&e.app, &e.bot.id).await;
    post_act(&e.app, &e.bot.id, json!({"action": "type", "ref": ref_of(&page, "Username"), "text": "standard_user"})).await;
    post_act(&e.app, &e.bot.id, json!({"action": "type", "ref": ref_of(&page, "Password"), "text": "secret_sauce"})).await;
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "Login")})).await;
    assert!(page["url"].as_str().unwrap().contains("inventory"), "{page}");
    let list = page["clickable"].to_string();
    assert!(list.contains("Add to cart"), "{list}");
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click", "ref": ref_of(&page, "· Sauce Labs Bike Light")})).await;
    assert!(page["error"].is_null(), "{page}");
    assert!(page["clickable"].to_string().contains("Remove"), "{page}");
    eprintln!("{}", page["clickable"]);
}
