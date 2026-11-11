//! Links to another app fired the way TikTok's sign-in fires `bytedance://dispatch_message/`, in
//! a real Chrome: a hidden iframe whose address is set right after it is added, so it leaves its
//! first, empty document, which fires no `navigate` event for kwnt's guard to cancel. Without the
//! guard's frame policy Chrome stops passing clicks and keys to the tab while it asks whether to
//! open the app; with it, Chrome blocks the frame and the page keeps taking input. Skipped when no
//! Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{app_links, computer, BrowserController},
    server::{routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// TikTok's captcha bridge, as it sends a message to the app: a hidden iframe, then its address.
const BRIDGE: &str = "var t=document.createElement('iframe');t.style.display='none';document.body.appendChild(t);t.src='bytedance://dispatch_message/';";
/// A page that fires the bridge once it has loaded, with a button over the whole page.
const AFTER_LOAD: &str = "<!doctype html><html><head><title>sign in</title></head><body>\
<button id=b onclick=\"document.title='clicked'\" style='position:fixed;inset:0'>Log in</button>\
<script>setTimeout(function(){__BRIDGE__ document.title='fired'}, 300)</script></body></html>";
/// The same from a script in `<head>`, while the page is still loading.
const WHILE_LOADING: &str = "<!doctype html><html><head><title>sign in</title>\
<script>document.addEventListener('DOMContentLoaded', function(){__BRIDGE__ document.title='fired'})</script></head><body>\
<button id=b onclick=\"document.title='clicked'\" style='position:fixed;inset:0'>Log in</button></body></html>";
/// Frames the tab opens itself still load: each tells the page it did.
const FRAMES: &str = "<!doctype html><html><head><title>frames</title></head><body><script>\
var got=[];addEventListener('message',function(e){got.push(String(e.data));document.title=got.sort().join(',')});\
function add(k,v){var f=document.createElement('iframe');f[k]=v;document.body.appendChild(f)}\
add('srcdoc',\"<script>parent.postMessage('srcdoc','*')<\\/script>\");\
add('src',\"data:text/html,<script>parent.postMessage('data','*')<\\/script>\");\
add('src',URL.createObjectURL(new Blob([\"<script>parent.postMessage('blob','*')<\\/script>\"],{type:'text/html'})));\
add('src','/child');\
</script></body></html>";
const CHILD: &str = "<!doctype html><script>parent.postMessage('web','*')</script>";

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
                    "/after-load" => AFTER_LOAD.replace("__BRIDGE__", BRIDGE),
                    "/while-loading" => WHILE_LOADING.replace("__BRIDGE__", BRIDGE),
                    "/frames" => FRAMES.to_string(),
                    "/child" => CHILD.to_string(),
                    _ => "<!doctype html><title>plain</title>".to_string(),
                };
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}", page.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

struct Env {
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
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    Env { state, bot, tmp }
}

/// A DevTools command that gives up after a few seconds instead of hanging the test.
async fn cdp(ctrl: &BrowserController, method: &str, params: Value) -> Value {
    tokio::time::timeout(Duration::from_secs(10), ctrl.send_cdp_command(method, params))
        .await
        .unwrap_or_else(|_| panic!("{method} timed out"))
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

async fn eval(ctrl: &BrowserController, js: &str) -> Value {
    cdp(ctrl, "Runtime.evaluate", json!({"expression": js, "returnByValue": true})).await["result"]["value"].clone()
}

async fn title_becomes(ctrl: &BrowserController, want: &str) -> String {
    let mut title = String::new();
    for _ in 0..50 {
        title = eval(ctrl, "document.title").await.as_str().unwrap_or_default().to_string();
        if title == want { break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    title
}

/// How many of a few mouse moves reach the page, with the live view's own input probe.
async fn moves_reaching(ctrl: &BrowserController) -> i64 {
    let point = eval(ctrl, app_links::PROBE_ARM_JS).await;
    let (x, y) = (point[0].as_f64().expect("a point to probe"), point[1].as_f64().unwrap());
    for dx in 0..5 {
        cdp(ctrl, "Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x + dx as f64, "y": y, "button": "none", "buttons": 0})).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    eval(ctrl, app_links::PROBE_READ_JS).await.as_i64().unwrap()
}

#[tokio::test]
async fn a_hidden_frame_opening_another_app_leaves_the_page_taking_input() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    for page in ["after-load", "while-loading"] {
        ctrl.navigate(&format!("{base}/{page}")).await.unwrap();
        assert_eq!(title_becomes(&ctrl, "fired").await, "fired", "{page}: the bridge ran");
        // Chrome takes well under a second to ask; give it more
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(moves_reaching(&ctrl).await, 5, "{page}: every move reaches the page after the link fired");
        cdp(&ctrl, "Input.dispatchMouseEvent", json!({"type": "mousePressed", "x": 40, "y": 40, "button": "left", "buttons": 1, "clickCount": 1})).await;
        cdp(&ctrl, "Input.dispatchMouseEvent", json!({"type": "mouseReleased", "x": 40, "y": 40, "button": "left", "buttons": 0, "clickCount": 1})).await;
        assert_eq!(title_becomes(&ctrl, "clicked").await, "clicked", "{page}: a click still works");
    }
}

#[tokio::test]
async fn frames_the_tab_opens_itself_still_load_under_the_guard() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/frames")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "blob,data,srcdoc,web").await, "blob,data,srcdoc,web");
}
