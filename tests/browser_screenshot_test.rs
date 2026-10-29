//! `browser_screenshot`: a bot's own tool list has it, and a call returns a real JPEG as an MCP
//! image block, at the width the bot asked for, with the browser's viewport put back afterwards.
//! A browser held for the person refuses it.
//!
//! Runs a real windowless Chrome as the "VM" browser (an external CDP endpoint) and is skipped
//! when none is installed; tests/browser_test.rs covers the viewport steps on a fake browser.

#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use base64::Engine;
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::computer,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// A page that is red below 600 px wide and blue above, so a phone shot looks different.
const PAGE: &str = "<!doctype html><title>Shot test</title><meta name=viewport content=\"width=device-width\">\
    <style>body{margin:0;background:#00f}@media (max-width:600px){body{background:#f00}}</style><h1>Hello</h1>";

async fn serve_page() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

async fn start_chrome(profile: &Path) -> Option<(String, ChromeGuard)> {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return None;
    }
    let (endpoint, _) = computer::ensure_local_chrome(profile, true).await.expect("Chrome starts");
    Some((endpoint, ChromeGuard(profile.to_path_buf())))
}

async fn state_with(tmp: &TempDir, endpoint: String) -> AppState {
    std::env::set_var("KWNT_COMPUTER_MODE", "vm");
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some(endpoint), ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url))
}

async fn rpc(app: &Router, token: &str, method: &str, params: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri("/api/mcp/self")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"));
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    serde_json::from_slice::<Value>(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap()["result"].clone()
}

async fn shot(app: &Router, token: &str, args: Value) -> Value {
    rpc(app, token, "tools/call", json!({"name": "browser_screenshot", "arguments": args})).await
}

/// Width and height from a JPEG's start-of-frame marker.
fn jpeg_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "a JPEG");
    let mut i = 2;
    while i + 9 < bytes.len() {
        assert_eq!(bytes[i], 0xFF);
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC2) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return (w, h);
        }
        i += 2 + len;
    }
    panic!("no frame header in the JPEG");
}

fn image(result: &Value) -> (String, Vec<u8>) {
    assert_eq!(result["isError"], false, "{result}");
    let content = result["content"].as_array().expect("content blocks");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image", "{}", content[1]);
    assert_eq!(content[1]["mimeType"], "image/jpeg");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content[1]["data"].as_str().unwrap())
        .expect("valid base64");
    (content[0]["text"].as_str().unwrap().to_string(), bytes)
}

/// The colour in the middle of a JPEG's first row would need a decoder; the page's own width is
/// enough to tell the layout the shot was taken at.
async fn inner_width(ctrl: &kwnt::sandbox::BrowserController) -> f64 {
    let v = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "innerWidth", "returnByValue": true})).await.unwrap();
    v["result"]["value"].as_f64().unwrap()
}

#[tokio::test]
async fn a_bot_sees_the_page_as_a_jpeg_at_the_width_it_asks_for() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = state_with(&tmp, endpoint).await;
    let app = create_router(state.clone());
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some("turn-shot".into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    let token = mcp_runtime::prepare_turn_with_self(&state, &bot, Some("turn-shot".into()), None, Some(scope))
        .await
        .unwrap()
        .1;

    // The bot's tool list has it, with a width it may set
    let tools = rpc(&app, &token, "tools/list", json!({})).await;
    let listed = tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == "browser_screenshot").cloned();
    let listed = listed.expect("browser_screenshot is listed");
    assert_eq!(listed["inputSchema"]["properties"]["width"]["maximum"], 1280);

    let url = serve_page().await;
    let opened = rpc(&app, &token, "tools/call", json!({"name": "browser_open", "arguments": {"url": url}})).await;
    assert_eq!(opened["isError"], false, "{opened}");
    let ctrl = state.browser_controllers.lock().await.get(&bot.id).cloned().unwrap();
    let panel_before = ctrl.last_screenshot.lock().await.clone();

    // At its own size: the VM browser's 1280×800
    let (text, bytes) = image(&shot(&app, &token, json!({})).await);
    assert!(text.contains(&url) && text.contains("Shot test") && text.contains("1280×800"), "{text}");
    assert_eq!(jpeg_size(&bytes), (1280, 800));

    // A phone shot: 390 wide, then the viewport goes back to 1280
    let (text, bytes) = image(&shot(&app, &token, json!({"width": 390})).await);
    assert!(text.contains("390×844"), "{text}");
    assert_eq!(jpeg_size(&bytes), (390, 844));
    assert_eq!(inner_width(&ctrl).await, 1280.0, "the viewport is restored");
    assert_eq!(*ctrl.last_screenshot.lock().await, panel_before, "the live panel's picture is left alone");

    // Too wide is capped at 1280; too narrow at 320
    assert_eq!(jpeg_size(&image(&shot(&app, &token, json!({"width": 4000})).await).1).0, 1280);
    assert_eq!(jpeg_size(&image(&shot(&app, &token, json!({"width": 10})).await).1).0, 320);
    let bad = shot(&app, &token, json!({"width": "wide"})).await;
    assert_eq!(bad["isError"], true, "{bad}");

    // Held for the person (a sign-in or CAPTCHA step): refused, and nothing is captured
    assert!(ctrl.hold_for_person().await);
    let held = shot(&app, &token, json!({"width": 390})).await;
    assert_eq!(held["isError"], true, "{held}");
    assert_eq!(held["content"].as_array().unwrap().len(), 1, "no image while held");
    assert!(held["content"][0]["text"].as_str().unwrap().contains("waiting on the person"), "{held}");
    ctrl.release_person_hold();

    // The person has taken control: refused too
    let control = ctrl.take_control().await.unwrap();
    let taken = shot(&app, &token, json!({})).await;
    assert_eq!(taken["isError"], true, "{taken}");
    assert!(taken["content"][0]["text"].as_str().unwrap().contains("taken control"), "{taken}");
    drop(control);
    tokio::time::sleep(Duration::from_millis(10)).await;
    image(&shot(&app, &token, json!({})).await);
}
