//! Cheaper page reads in kwnt's own browser, through a real Chrome: a read of a page that still
//! reads the same as the bot's last look answers "unchanged" instead of the page (`?since=` on
//! the HTTP API, per turn for `browser_page`), and a page with many controls lists 40, the ones
//! matching the bot's `find` words first, with their refs kept. Skipped when no Chrome is
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
    sandbox::computer,
    server::{continuity::SelfScope, create_router, mcp_runtime, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// 60 products, one of them far down the page, and a counter.
fn shop_page() -> String {
    let mut page = String::from("<!doctype html><title>shop</title><p id=n>Cart 0</p>");
    for i in 1..=60 {
        let name = if i == 55 { "Bike Light".to_string() } else { format!("Item {i}") };
        page.push_str(&format!(
            "<div><h3>{name}</h3><button onclick=\"var n=document.getElementById('n');n.textContent='Cart '+(Number(n.textContent.slice(5))+1)\">Add {name}</button></div>"
        ));
    }
    page
}

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let page = shop_page();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/shop")
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

async fn send(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Value) -> Value {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = if method == "GET" { Body::empty() } else { Body::from(body.to_string()) };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::OK, "{v}");
    v
}

async fn page(e: &Env, query: &str) -> Value {
    send(&e.app, "GET", &format!("/api/sandbox/{}/page{query}", e.bot.id), None, Value::Null).await
}

async fn act(e: &Env, body: Value) -> Value {
    send(&e.app, "POST", &format!("/api/sandbox/{}/act", e.bot.id), None, body).await
}

fn controls(page: &Value) -> Vec<String> {
    page["clickable"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect()
}

fn ref_of(line: &str) -> u64 {
    line[1..line.find(']').unwrap()].parse().unwrap()
}

/// Calls one of the turn's own browser tools; returns the page it answered.
async fn tool(e: &Env, token: &str, name: &str, args: Value) -> Value {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let v = send(&e.app, "POST", "/api/mcp/self", Some(token), rpc).await;
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or_default();
    serde_json::from_str(text).unwrap_or_else(|_| panic!("{v}"))
}

#[tokio::test]
async fn an_unchanged_page_is_not_sent_again_and_a_long_list_puts_matches_first() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let url = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&url).await.unwrap();

    // 60 controls: 40 listed in page order, and a note on how to see the others
    let first = page(&e, "").await;
    let hash = first["hash"].as_str().unwrap().to_string();
    let listed = controls(&first);
    assert_eq!(listed.len(), 40, "{first}");
    assert!(listed[0].contains("Add Item 1"), "{listed:?}");
    assert!(!first.to_string().contains("Bike Light\\\""), "far down the page");
    assert!(first["controls_note"].as_str().unwrap().starts_with("Showing the first 40 of 60 controls."), "{first}");

    // The same page again: only "unchanged"
    let again = page(&e, &format!("?since={hash}")).await;
    assert_eq!(again["unchanged"], true, "{again}");
    assert!(again.get("text").is_none() && again.get("clickable").is_none(), "{again}");
    assert_eq!(again["url"], first["url"]);
    // Without since, or with another hash: the whole page
    assert!(page(&e, "").await.get("text").is_some());
    assert!(page(&e, "?since=0000").await.get("text").is_some());

    // find lists the matching control first, under its own ref, which clicks it
    let found = page(&e, "?find=bike%20light").await;
    let bike = controls(&found)[0].clone();
    assert!(bike.contains("Add Bike Light"), "{found}");
    assert_eq!(found["controls_note"], "Showing 40 of 60 controls: the 1 matching \"bike light\" first.", "{found}");
    assert_ne!(found["hash"], first["hash"], "another read");
    let done = act(&e, json!({"action": "click", "ref": ref_of(&bike)})).await;
    assert_eq!(done["changed"], "New: \"Cart 1\". Gone: \"Cart 0\".", "{done}");

    // The page changed, so the old hash gets it whole
    let after = page(&e, &format!("?since={hash}")).await;
    assert!(after["text"].as_str().unwrap().contains("Cart 1"), "{after}");

    // browser_page in a turn: unchanged after the turn's own last look, whole for a new turn
    let scope = |turn: &str| SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        sandbox_target: Some(e.bot.id.clone()),
        turn_id: Some(turn.to_string()),
        ..Default::default()
    };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, None, None, Some(scope("t1"))).await.unwrap();
    let opened = tool(&e, &token, "browser_open", json!({"url": url})).await;
    assert!(opened["text"].as_str().unwrap().contains("Cart 0"), "{opened}");
    let read = tool(&e, &token, "browser_page", json!({})).await;
    assert_eq!(read["unchanged"], true, "the open already showed it: {read}");
    let read = tool(&e, &token, "browser_page", json!({"full": true})).await;
    assert!(read["text"].as_str().unwrap().contains("Cart 0"), "{read}");
    let found = tool(&e, &token, "browser_page", json!({"find": "bike"})).await;
    assert!(controls(&found)[0].contains("Add Bike Light"), "other words, another read: {found}");
    let acted = tool(&e, &token, "browser_act", json!({"action": "click", "ref": ref_of(&controls(&found)[0])})).await;
    assert!(acted["text"].as_str().unwrap().contains("Cart 1"), "{acted}");
    assert_eq!(tool(&e, &token, "browser_page", json!({})).await["unchanged"], true, "the act's answer showed it");

    let (_ctx2, other_turn) = mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, None, None, Some(scope("t2"))).await.unwrap();
    let read = tool(&e, &other_turn, "browser_page", json!({})).await;
    assert!(read["text"].as_str().unwrap().contains("Cart 1"), "a new turn never saw it: {read}");
    assert_eq!(tool(&e, &other_turn, "browser_page", json!({})).await["unchanged"], true);
    mcp_runtime::finish_turn(&e.state, &token).await;
    mcp_runtime::finish_turn(&e.state, &other_turn).await;
}

/// The same on a real shop (network): the inventory read twice is "unchanged" the second time,
/// and find ("add to cart bike light") puts the Bike Light's Add to cart button first.
#[tokio::test]
#[ignore = "needs the network"]
async fn saucedemo_reads_are_cheaper() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate("https://www.saucedemo.com/").await.unwrap();
    let login = page(&e, "").await;
    let field = |name: &str| ref_of(controls(&login).iter().find(|l| l.contains(name)).unwrap());
    act(&e, json!({"action": "type", "ref": field("Username"), "text": "standard_user"})).await;
    act(&e, json!({"action": "type", "ref": field("Password"), "text": "secret_sauce"})).await;
    let shop = act(&e, json!({"action": "click", "ref": field("Login")})).await;
    assert!(shop["url"].as_str().unwrap().contains("inventory"), "{shop}");

    let again = page(&e, &format!("?since={}", shop["hash"].as_str().unwrap())).await;
    assert_eq!(again["unchanged"], true, "{again}");
    let found = page(&e, "?find=add%20to%20cart%20bike%20light").await;
    let first = &controls(&found)[0];
    assert!(first.contains("Add to cart") && first.contains("Bike Light"), "{found}");
    eprintln!("first control for \"bike light\": {first}");
}
