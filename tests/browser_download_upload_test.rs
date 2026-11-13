//! A bot's own browser saving a download straight into its folder, and a new `browser_upload`
//! tool answering a page's own file picker with a file from that folder or a chat attachment.
//! Runs a real windowless Chrome in Local mode and is skipped when none is installed.

#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use chrome_guard::ChromeGuard;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::computer,
    server::{continuity::SelfScope, create_router, mcp_runtime, routes::resolve_sandbox_target_info, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// A button that navigates to `/note`, which replies with `Content-Disposition: attachment`
/// so Chrome treats it as a download rather than showing it.
const DOWNLOAD_PAGE: &str = "<!doctype html><title>dl</title><body>\
<button onclick=\"location.href='/note'\">Get the file</button>";
/// A hidden file input behind a button, same shape as `browser_file_chooser_test.rs`.
const UPLOAD_PAGE: &str = "<!doctype html><html><head><title>pick</title></head><body style=margin:0>\
<input type=file id=f style=display:none>\
<button onclick=f.click() style='position:fixed;inset:0'>Pick a file</button>\
<script>f.onchange=function(){document.title='picked:'+Array.from(f.files).map(function(x){return x.name}).join(',')}</script>\
</body></html>";
const NOTE_BYTES: &[u8] = b"hello from the download test, not a real document";

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/");
                let reply = match path {
                    "/note" => format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\n\
                         content-disposition: attachment; filename=\"note.txt\"\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n",
                        NOTE_BYTES.len()
                    )
                    .into_bytes()
                    .into_iter()
                    .chain(NOTE_BYTES.iter().copied())
                    .collect::<Vec<u8>>(),
                    "/upload" => {
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{UPLOAD_PAGE}",
                            UPLOAD_PAGE.len()
                        );
                        reply.into_bytes()
                    }
                    _ => {
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{DOWNLOAD_PAGE}",
                            DOWNLOAD_PAGE.len()
                        );
                        reply.into_bytes()
                    }
                };
                let _ = stream.write_all(&reply).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

struct Env {
    app: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup() -> Env {
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    std::env::set_var("KWNT_COMPUTER_MODE", "local");
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

async fn post(app: &Router, uri: &str, token: &str, body: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Calls one self tool; returns its parsed `page` JSON and whether it errored.
async fn self_tool(app: &Router, token: &str, name: &str, args: Value) -> (Value, bool) {
    let v = post(app, "/api/mcp/self", token, rpc("tools/call", json!({"name": name, "arguments": args}))).await;
    let is_error = v["result"]["isError"].as_bool().unwrap_or(false);
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    (serde_json::from_str(text).unwrap_or(Value::Null), is_error)
}

/// Uploads a small file through the real `/api/attachments` route, returning its id.
async fn upload_attachment(state: &AppState, name: &str, bytes: &'static [u8]) -> String {
    let app = create_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/attachments?name={name}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    v["id"].as_str().expect("attachment id").to_string()
}

/// Polls `browser_page` until a file picker is reported open, or panics.
async fn wait_for_chooser(app: &Router, token: &str) -> Value {
    for _ in 0..50 {
        let (page, is_error) = self_tool(app, token, "browser_page", json!({})).await;
        assert!(!is_error, "{page}");
        if page["file_chooser"].is_object() {
            return page["file_chooser"].clone();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no file picker was ever reported open");
}

/// Polls `browser_page` until `download.status` is terminal (done/cancelled/failed), or panics.
async fn wait_for_download(app: &Router, token: &str) -> Value {
    for _ in 0..100 {
        let (page, is_error) = self_tool(app, token, "browser_page", json!({})).await;
        assert!(!is_error, "{page}");
        if let Some(status) = page["download"]["status"].as_str() {
            if status != "in_progress" {
                return page["download"].clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("download never reached a terminal state");
}

#[tokio::test]
async fn a_download_lands_in_the_bots_own_folder() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.state.config.data_dir.join("local-chrome"));
    let kwants = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = &kwants[0];
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), sandbox_target: Some(bot.id.clone()), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, bot, None, None, Some(scope)).await.unwrap();

    let base = serve().await;
    let (page, is_error) = self_tool(&e.app, &token, "browser_open", json!({"url": base})).await;
    assert!(!is_error, "{page}");

    let (page, is_error) = self_tool(&e.app, &token, "browser_act", json!({"action": "click_text", "text": "Get the file"})).await;
    assert!(!is_error, "{page}");

    let report = wait_for_download(&e.app, &token).await;
    assert_eq!(report["status"], "done", "{report}");
    assert_eq!(report["name"], "note.txt");
    assert_eq!(report["path"], "downloads/note.txt");
    assert!(report["note"].as_str().unwrap().contains("Downloaded note.txt"), "{report}");

    let (_, workspace) = resolve_sandbox_target_info(&e.state, &bot.id).await.expect("bot workspace");
    let landed = std::fs::read(workspace.join("downloads/note.txt")).expect("file landed on disk");
    assert_eq!(landed, NOTE_BYTES);

    mcp_runtime::finish_turn(&e.state, &token).await;
}

#[tokio::test]
async fn a_second_download_with_the_same_name_never_overwrites_the_first() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.state.config.data_dir.join("local-chrome"));
    let kwants = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = &kwants[0];
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), sandbox_target: Some(bot.id.clone()), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, bot, None, None, Some(scope)).await.unwrap();

    let base = serve().await;
    for _ in 0..2 {
        let (page, is_error) = self_tool(&e.app, &token, "browser_open", json!({"url": base})).await;
        assert!(!is_error, "{page}");
        let (page, is_error) = self_tool(&e.app, &token, "browser_act", json!({"action": "click_text", "text": "Get the file"})).await;
        assert!(!is_error, "{page}");
        wait_for_download(&e.app, &token).await;
    }

    let (_, workspace) = resolve_sandbox_target_info(&e.state, &bot.id).await.expect("bot workspace");
    assert!(workspace.join("downloads/note.txt").exists());
    assert!(workspace.join("downloads/note (2).txt").exists(), "the second download got a collision-safe name instead of overwriting");

    mcp_runtime::finish_turn(&e.state, &token).await;
}

#[tokio::test]
async fn browser_upload_answers_the_pages_own_picker_with_a_workspace_file() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.state.config.data_dir.join("local-chrome"));
    let kwants = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = &kwants[0];
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), sandbox_target: Some(bot.id.clone()), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, bot, None, None, Some(scope)).await.unwrap();

    let (_, workspace) = resolve_sandbox_target_info(&e.state, &bot.id).await.expect("bot workspace");
    std::fs::create_dir_all(workspace.join("downloads")).unwrap();
    std::fs::write(workspace.join("downloads/clip.mp4"), b"not really a video, just test bytes").unwrap();

    let base = serve().await;
    let (page, is_error) = self_tool(&e.app, &token, "browser_open", json!({"url": format!("{base}/upload")})).await;
    assert!(!is_error, "{page}");

    let (page, is_error) = self_tool(&e.app, &token, "browser_act", json!({"action": "click_text", "text": "Pick a file"})).await;
    assert!(!is_error, "{page}");
    let chooser = wait_for_chooser(&e.app, &token).await;
    assert!(chooser["note"].as_str().unwrap_or("").contains("A file picker opened"), "{chooser}");

    let (page, is_error) = self_tool(&e.app, &token, "browser_upload", json!({"files": ["downloads/clip.mp4"]})).await;
    assert!(!is_error, "{page}");
    assert!(page["result"].as_str().unwrap_or("").contains("clip.mp4"), "{page}");
    assert!(page["text"].as_str().unwrap_or("").contains("picked:clip.mp4"), "the page's own change handler should have seen the file: {page}");

    mcp_runtime::finish_turn(&e.state, &token).await;
}

#[tokio::test]
async fn browser_upload_answers_the_pages_own_picker_with_a_chat_attachment() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.state.config.data_dir.join("local-chrome"));
    let kwants = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = &kwants[0];
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), sandbox_target: Some(bot.id.clone()), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, bot, None, None, Some(scope)).await.unwrap();

    let attachment_id = upload_attachment(&e.state, "post.mov", b"attachment bytes, also not a real video").await;

    let base = serve().await;
    let (page, is_error) = self_tool(&e.app, &token, "browser_open", json!({"url": format!("{base}/upload")})).await;
    assert!(!is_error, "{page}");
    let (page, is_error) = self_tool(&e.app, &token, "browser_act", json!({"action": "click_text", "text": "Pick a file"})).await;
    assert!(!is_error, "{page}");
    wait_for_chooser(&e.app, &token).await;

    let (page, is_error) = self_tool(&e.app, &token, "browser_upload", json!({"files": [attachment_id]})).await;
    assert!(!is_error, "{page}");
    assert!(page["text"].as_str().unwrap_or("").contains("picked:post.mov"), "{page}");

    mcp_runtime::finish_turn(&e.state, &token).await;
}

#[tokio::test]
async fn browser_upload_is_refused_with_no_picker_open() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.state.config.data_dir.join("local-chrome"));
    let kwants = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = &kwants[0];
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), sandbox_target: Some(bot.id.clone()), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, bot, None, None, Some(scope)).await.unwrap();

    let base = serve().await;
    let (page, is_error) = self_tool(&e.app, &token, "browser_open", json!({"url": format!("{base}/upload")})).await;
    assert!(!is_error, "{page}");

    let (_, workspace) = resolve_sandbox_target_info(&e.state, &bot.id).await.expect("bot workspace");
    std::fs::write(workspace.join("stray.txt"), b"nothing should want this").unwrap();
    let (page, is_error) = self_tool(&e.app, &token, "browser_upload", json!({"files": ["stray.txt"]})).await;
    assert!(is_error, "{page}");
    assert!(page["error"].as_str().unwrap_or("").contains("No file picker is open"), "{page}");

    mcp_runtime::finish_turn(&e.state, &token).await;
}
