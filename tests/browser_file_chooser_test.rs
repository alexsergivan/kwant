//! The page's own file picker (`<input type=file>`), the way TikTok Studio's "Select video" opens
//! one: the screencast can't show Chrome's native dialog, so it's intercepted
//! (`Page.setInterceptFileChooserDialog`) and the live view shows its own sheet instead. Covers a
//! visible input and a hidden-input-behind-a-button page (the TikTok shape), answering with an
//! attachment already uploaded through `/api/attachments`, cancelling, and a chooser cleared by
//! navigation. Skipped when no Chrome is installed. Local delivery only — there's no remote Chrome
//! in this environment to exercise the base64/DataTransfer path.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
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
use tower::ServiceExt;
use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, Message as WsMessage}};

/// A visible file input over the whole page, reporting what it got in its title.
const VISIBLE: &str = "<!doctype html><html><head><title>visible</title></head><body style=margin:0>\
<input type=file id=f style='position:fixed;inset:0;opacity:0.01'>\
<script>f.onchange=function(){document.title='picked:'+Array.from(f.files).map(function(x){return x.name}).join(',')}</script>\
</body></html>";
/// TikTok-like: the real input is hidden, a button over the whole page opens it.
const HIDDEN: &str = "<!doctype html><html><head><title>hidden</title></head><body style=margin:0>\
<input type=file id=f style=display:none accept=video/*>\
<button onclick=f.click() style='position:fixed;inset:0'>Select video</button>\
<script>f.onchange=function(){document.title='picked:'+Array.from(f.files).map(function(x){return x.name}).join(',')}</script>\
</body></html>";

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
                    "/visible" => VISIBLE,
                    "/hidden" => HIDDEN,
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

/// kwnt in Local mode, so the bot's browser is a real Chrome on this machine and uploads share its
/// filesystem (the `DOM.setFileInputFiles` path, not the base64 one).
async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting(computer::SETTING_KEY, "local").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    Env { state, bot, tmp }
}

/// Uploads a small file through the real `/api/attachments` route (the same one the chat compose
/// box uses), returning its id — exactly the id a `file_chooser` answer carries.
async fn upload(state: &AppState, name: &str, bytes: &'static [u8]) -> String {
    let app = create_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/attachments?name={name}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["id"].as_str().expect("attachment id").to_string()
}

async fn cdp(ctrl: &BrowserController, method: &str, params: Value) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(10), ctrl.send_cdp_command(method, params)).await.ok()?.ok()
}

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

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next live-view message of `kind` that `matches`, answering pings/frames meanwhile; `None`
/// after 10 s. Same as `browser_popups_test.rs`'s helper, but doesn't fail on `"error"`: a couple
/// of these tests expect one (a mismatched/stale chooser id).
async fn next_message(ws: &mut Ws, kind: &str, matches: impl Fn(&Value) -> bool) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = ws.next().await?.ok()?;
            let WsMessage::Text(text) = message else { continue };
            let value: Value = serde_json::from_str(&text).unwrap_or_default();
            if value["type"] == "ping" { ws.send(WsMessage::Text(json!({"action": "pong"}).to_string())).await.ok()?; }
            if value["type"] == "frame" { ws.send(WsMessage::Text(json!({"action": "frame_ack", "id": value["id"]}).to_string())).await.ok()?; }
            if value["type"] == kind && matches(&value) { return Some(value); }
        }
    }).await.ok().flatten()
}

async fn connect(state: &AppState, bot_id: &str) -> (Ws, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = create_router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut request = format!("ws://127.0.0.1:{port}/api/sandbox/{bot_id}/stream").into_client_request().unwrap();
    request.headers_mut().insert("Origin", format!("http://127.0.0.1:{port}").parse().unwrap());
    let (ws, _) = connect_async(request).await.unwrap();
    (ws, server)
}

async fn take_control(ws: &mut Ws) {
    ws.send(WsMessage::Text(json!({"action": "take_control"}).to_string())).await.unwrap();
    assert!(next_message(ws, "control", |c| c["active"] == true).await.is_some());
}

#[tokio::test]
async fn picking_a_visible_input_uploads_and_delivers_the_file() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/visible")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "visible").await, "visible");

    let attachment_id = upload(&e.state, "clip.mp4", b"not really a video, just test bytes").await;
    let (mut ws, server) = connect(&e.state, &e.bot.id).await;
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some());
    take_control(&mut ws).await;

    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 1}).to_string())).await.unwrap();
    let opened = next_message(&mut ws, "file_chooser", |_| true).await.expect("the click opens the intercepted chooser");
    assert_eq!(opened["multiple"], false);
    let chooser_id = opened["id"].clone();

    ws.send(WsMessage::Text(json!({"action": "file_chooser", "id": chooser_id, "attachment_ids": [attachment_id]}).to_string())).await.unwrap();
    let done = next_message(&mut ws, "file_chooser_done", |_| true).await.expect("the server answers once the file lands");
    assert_eq!(done["id"], chooser_id);
    assert_eq!(done["names"], json!(["clip.mp4"]));
    assert_eq!(title_becomes(&ctrl, "picked:clip.mp4").await, "picked:clip.mp4", "the page's own change handler saw the file, as a real pick would");

    ws.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_tiktok_style_hidden_input_behind_a_button_is_intercepted_too() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/hidden")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "hidden").await, "hidden");

    let attachment_id = upload(&e.state, "post.mov", b"more test bytes, still not a real video").await;
    let (mut ws, server) = connect(&e.state, &e.bot.id).await;
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some());
    take_control(&mut ws).await;

    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 1}).to_string())).await.unwrap();
    let opened = next_message(&mut ws, "file_chooser", |_| true).await.expect("clicking the button over the hidden input still opens the intercepted chooser");
    assert_eq!(opened["accept"], "video/*", "the sheet gets the real input's accept, read straight off the DOM node");

    ws.send(WsMessage::Text(json!({"action": "file_chooser", "id": opened["id"], "attachment_ids": [attachment_id]}).to_string())).await.unwrap();
    let done = next_message(&mut ws, "file_chooser_done", |_| true).await.expect("answered the same way as a visible input");
    assert_eq!(done["names"], json!(["post.mov"]));
    assert_eq!(title_becomes(&ctrl, "picked:post.mov").await, "picked:post.mov");

    ws.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn cancelling_the_sheet_gets_no_reply_and_leaves_the_page_untouched() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/visible")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "visible").await, "visible");

    let (mut ws, server) = connect(&e.state, &e.bot.id).await;
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some());
    take_control(&mut ws).await;

    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 1}).to_string())).await.unwrap();
    let opened = next_message(&mut ws, "file_chooser", |_| true).await.expect("chooser opened");

    ws.send(WsMessage::Text(json!({"action": "file_chooser", "id": opened["id"], "cancel": true}).to_string())).await.unwrap();
    // Nothing more should come back for this chooser: clicking again must open a brand new one
    // (an old `file_chooser_done`/error lingering would prove the cancel got an unwanted reply).
    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 2}).to_string())).await.unwrap();
    let reopened = next_message(&mut ws, "file_chooser", |_| true).await.expect("a fresh click opens a fresh chooser");
    assert_ne!(reopened["id"], opened["id"], "cancel actually cleared the old chooser, this is a new one");
    assert_eq!(title_becomes(&ctrl, "visible").await, "visible", "no file ever reached the page");

    ws.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn navigating_away_while_the_sheet_is_open_clears_it() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&format!("{base}/visible")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "visible").await, "visible");

    let attachment_id = upload(&e.state, "late.mp4", b"arrives after the page already moved on").await;
    let (mut ws, server) = connect(&e.state, &e.bot.id).await;
    assert!(next_message(&mut ws, "frame", |_| true).await.is_some());
    take_control(&mut ws).await;

    ws.send(WsMessage::Text(json!({"action": "click", "x": 40, "y": 40, "id": 1}).to_string())).await.unwrap();
    let opened = next_message(&mut ws, "file_chooser", |_| true).await.expect("chooser opened");

    ctrl.navigate(&format!("{base}/plain")).await.expect("navigate away while the sheet is still open");
    let gone = next_message(&mut ws, "file_chooser_error", |_| true).await.expect("the sheet is told the chooser is gone");
    assert_eq!(gone["code"], "gone");

    // Answering the now-stale id gets the same "gone" treatment, not a generic protocol error.
    ws.send(WsMessage::Text(json!({"action": "file_chooser", "id": opened["id"], "attachment_ids": [attachment_id]}).to_string())).await.unwrap();
    let stale = next_message(&mut ws, "file_chooser_error", |_| true).await.expect("answering a cleared chooser still gets a file_chooser_error");
    assert_eq!(stale["code"], "gone");

    ws.close(None).await.unwrap();
    server.abort();
}
