//! Sign-in popups in a real Chrome, the way "Continue with Google" opens one: the page calls
//! `window.open`, the popup posts its answer back to the page that opened it and closes. The bot's
//! browser tools and the live view both move to the popup while it is open and come back to the
//! opener when it closes. Skipped when no Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use chrome_guard::ChromeGuard;
use futures::{SinkExt, StreamExt};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{computer, BrowserController},
    server::{create_router, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, Message as WsMessage}};

/// A page with a sign-in button over all of it that opens the sign-in in a popup window, and
/// shows the popup's answer in its title a little later, as a real sign-in page does after its
/// own requests: the view is back on the opener, and has its picture, before the answer shows.
const OPENER: &str = "<!doctype html><html><head><title>opener</title></head><body style=margin:0>\
<button onclick=\"window.open('/popup','signin','width=480,height=600')\" style='position:fixed;inset:0'>Continue with Google</button>\
<script>addEventListener('message',function(e){setTimeout(function(){document.title='signed in: '+e.data},700)})</script></body></html>";
/// The sign-in: a button over all of it posts the answer to the opener and closes the popup.
const POPUP: &str = "<!doctype html><html><head><title>popup</title></head><body style=margin:0>\
<button onclick=\"opener.postMessage('ok','*');window.close()\" style='position:fixed;inset:0'>Choose account</button></body></html>";

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let page = match req.split_whitespace().nth(1).unwrap_or("/") {
                    "/opener" => OPENER,
                    "/popup" => POPUP,
                    _ => "<!doctype html><title>plain</title>",
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

/// A DevTools command; `None` when it failed or took too long (the page it went to may be closing).
async fn cdp(ctrl: &BrowserController, method: &str, params: Value) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(10), ctrl.send_cdp_command(method, params)).await.ok()?.ok()
}

/// The title of the page the bot's tools act on, once it is `want` (or after 5 s).
async fn title_becomes(ctrl: &BrowserController, want: &str) -> String {
    let mut title = String::new();
    for _ in 0..50 {
        let value = cdp(ctrl, "Runtime.evaluate", json!({"expression": "document.title", "returnByValue": true})).await;
        title = value.and_then(|v| v["result"]["value"].as_str().map(str::to_string)).unwrap_or_default();
        if title == want { break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    title
}

async fn click(ctrl: &BrowserController) {
    for kind in ["mousePressed", "mouseReleased"] {
        cdp(ctrl, "Input.dispatchMouseEvent", json!({"type": kind, "x": 40, "y": 40, "button": "left", "buttons": if kind == "mousePressed" { 1 } else { 0 }, "clickCount": 1})).await;
    }
}

#[tokio::test]
async fn a_popup_the_page_opens_is_where_the_bots_tools_act_until_it_closes() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/opener")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "opener").await, "opener");
    click(&ctrl).await;
    assert_eq!(title_becomes(&ctrl, "popup").await, "popup", "the bot's tools act on the popup the page opened");
    click(&ctrl).await;
    assert_eq!(title_becomes(&ctrl, "signed in: ok").await, "signed in: ok", "back on the opener once the popup closed, with its answer");
    ctrl.navigate(&format!("{base}/plain")).await.expect("the bot's tools still work on the opener");
    assert_eq!(title_becomes(&ctrl, "plain").await, "plain");
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next live-view message of `kind` that `matches`, answering pings meanwhile; `None` after 10 s.
async fn next_message(ws: &mut Ws, kind: &str, matches: impl Fn(&Value) -> bool) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = ws.next().await?.ok()?;
            let WsMessage::Text(text) = message else { continue };
            let value: Value = serde_json::from_str(&text).unwrap_or_default();
            // the person sees every error: opening a popup or closing it by a click is none
            assert_ne!(value["type"], "error", "the live view reported an error: {value}");
            if value["type"] == "ping" { ws.send(WsMessage::Text(json!({"action": "pong"}).to_string())).await.ok()?; }
            if value["type"] == "frame" { ws.send(WsMessage::Text(json!({"action": "frame_ack", "id": value["id"]}).to_string())).await.ok()?; }
            if value["type"] == kind && matches(&value) { return Some(value); }
        }
    }).await.ok().flatten()
}

#[tokio::test]
async fn the_live_view_shows_a_popup_the_page_opens_and_comes_back_when_it_closes() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/opener")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "opener").await, "opener");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = create_router(e.state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut request = format!("ws://127.0.0.1:{port}/api/sandbox/{}/stream", e.bot.id).into_client_request().unwrap();
    request.headers_mut().insert("Origin", format!("http://127.0.0.1:{port}").parse().unwrap());
    let (mut ws, _) = connect_async(request).await.unwrap();
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some(), "the opener is shown");
    ws.send(WsMessage::Text(json!({"action": "take_control"}).to_string())).await.unwrap();
    assert!(next_message(&mut ws, "control", |c| c["active"] == true).await.is_some());

    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 1}).to_string())).await.unwrap();
    let shown = next_message(&mut ws, "location", |l| l["url"].as_str().is_some_and(|u| u.ends_with("/popup"))).await;
    assert!(shown.is_some(), "the live view moves to the popup the person's click opened");
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some(), "and shows it");

    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 2}).to_string())).await.unwrap();
    let on_opener = |l: &Value| l["url"].as_str().is_some_and(|u| u.ends_with("/opener"));
    assert!(next_message(&mut ws, "location", on_opener).await.is_some(), "once the popup closed, the live view is back on the opener");
    // the stream sends the address before the new page's pictures, so this frame is the opener's
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some(), "and shows it again");
    let answered = next_message(&mut ws, "location", |l| on_opener(l) && l["title"] == "signed in: ok").await;
    assert!(answered.is_some(), "with the popup's answer once the opener has it");
    ws.close(None).await.unwrap();
    server.abort();
}
