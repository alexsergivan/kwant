//! Cited reads in kwnt's own browser, through a real Chrome: a bot quotes what it read and gets
//! back `{quote, url, link}` only when the words are on the page once (in the page or a
//! same-origin frame); the link, opened, scrolls the browser to the quote (Chrome highlights it).
//! A quote that isn't there, or is there twice, is refused. Blocks: a read with ids, cited by id.
//! Skipped when no Chrome is installed.
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
    sandbox::{computer, BrowserController},
    server::{continuity::SelfScope, create_router, mcp_runtime, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// A long page: the quote's words in other capitals near the top (Chrome would highlight those
/// for a bare fragment), a repeated line, the quote far down, and a same-origin frame.
fn long_page() -> String {
    let mut page = String::from("<!doctype html><title>report</title><h1>Quarterly notes</h1><p id=decoy>The Report Below is late.</p>");
    for i in 1..=60 {
        page.push_str(&format!("<p style=\"height:40px\">Filler paragraph {i}.</p><p>Same words here.</p>"));
    }
    page.push_str("<p id=target>See the report below.</p><p>Revenue grew 12% to €4.2m in Q3, up from 3.7m.</p>");
    page.push_str("<iframe src=\"/frame\" height=80></iframe>");
    page
}

/// Rigel's lookalikes: (decoy, target, quote). Chrome matches a text fragment without case or
/// accents, so a bare fragment of the quote would highlight the decoy above the target.
const LOOKALIKES: [(&str, &str, &str); 4] = [
    ("Send your résumé today.", "Send your resume today.", "your resume today"),
    ("A naive cafe visit.", "A naïve café visit.", "naïve café visit"),
    ("Die STRASSE ist lang.", "Die Straße ist lang.", "Die Straße ist lang."),
    ("Ünïcode ΣΑΣ test", "ünïcode σας test", "ünïcode σας test"),
];

/// The decoy, then the target far below it. `note`: each on a line with its own words before it
/// ("Old note:", "New note:"); else between the same lines, so nothing on the page tells them apart.
fn lookalike_page(n: usize, note: bool) -> String {
    let (decoy, target, _) = LOOKALIKES[n.min(3)];
    let gap = "<div style=\"height:3000px\"></div>";
    let around = |id: &str, words: &str, label: &str| {
        if note {
            format!("<p id={id}>{label} note: {words}</p>")
        } else {
            format!("<p>Intro words here.</p><p id={id}>{words}</p><p>Outro words there.</p>")
        }
    };
    format!("<!doctype html><meta charset=utf-8><title>look</title>{gap}{}{gap}{}{gap}", around("d", decoy, "Old"), around("t", target, "New"))
}

/// Percent-encodes words for a bare text fragment, as kwnt does.
fn fragment_words(words: &str) -> String {
    words.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let page = if head.starts_with("GET /frame") {
                    "<!doctype html><p>Reviews: the light is very bright.</p>".to_string()
                } else if let Some(rest) = head.strip_prefix("GET /look/") {
                    let (n, rest) = rest.split_once('/').unwrap_or(("0", ""));
                    lookalike_page(n.parse().unwrap_or(0), rest.starts_with("note"))
                } else {
                    long_page()
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/long")
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
    let (status, v) = send_any(app, method, uri, auth, body).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v
}

async fn send_any(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = if method == "GET" { Body::empty() } else { Body::from(body.to_string()) };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn page(e: &Env, query: &str) -> Value {
    send(&e.app, "GET", &format!("/api/sandbox/{}/page{query}", e.bot.id), None, Value::Null).await
}

async fn cite(e: &Env, body: Value) -> (StatusCode, Value) {
    send_any(&e.app, "POST", &format!("/api/sandbox/{}/cite", e.bot.id), None, body).await
}

async fn eval(ctrl: &BrowserController, js: &str) -> Value {
    let res = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": js, "returnByValue": true})).await.unwrap();
    res["result"]["value"].clone()
}

/// Whether the first text on the page that starts with `words` is on screen.
async fn in_view(ctrl: &BrowserController, words: &str) -> bool {
    let js = format!(
        r#"(function() {{
            var w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT), n;
            while ((n = w.nextNode())) {{
                var i = n.data.indexOf({words});
                if (i < 0) continue;
                var r = document.createRange(); r.setStart(n, i); r.setEnd(n, i + {words}.length);
                var b = r.getBoundingClientRect();
                return b.height > 0 && b.top >= 0 && b.bottom <= innerHeight;
            }}
            return false;
        }})()"#,
        words = json!(words)
    );
    eval(ctrl, &js).await == json!(true)
}

/// Whether the element with `id` is on screen.
async fn id_in_view(ctrl: &BrowserController, id: &str) -> bool {
    let js = format!("(function(){{var b=document.getElementById({}).getBoundingClientRect();return b.height>0&&b.top>=0&&b.bottom<=innerHeight;}})()", json!(id));
    eval(ctrl, &js).await == json!(true)
}

/// Opens `link` as a new page load (a fragment-only change in the same document doesn't run a
/// text fragment) and waits for Chrome to scroll to the quote; the scroll offset it ended at.
async fn open_link(ctrl: &BrowserController, link: &str) -> f64 {
    ctrl.navigate("about:blank").await.unwrap();
    ctrl.navigate(link).await.unwrap();
    let mut y = 0.0;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        y = eval(ctrl, "scrollY").await.as_f64().unwrap_or(0.0);
        if y > 0.0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
            return eval(ctrl, "scrollY").await.as_f64().unwrap_or(y);
        }
    }
    y
}

#[tokio::test]
async fn a_quote_on_the_page_gets_a_link_that_opens_at_it_and_a_made_up_one_is_refused() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let url = serve().await;
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate(&url).await.unwrap();
    for _ in 0..30 {
        if eval(&ctrl, "document.querySelector('iframe') && document.querySelector('iframe').contentDocument && document.querySelector('iframe').contentDocument.body.innerText.length > 0").await == json!(true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // On the page once: the page's own words, its address, and a link with context, since a
    // bare "the report below" would match the decoy at the top first
    let (status, c) = cite(&e, json!({"text": "the  report\nbelow"})).await;
    assert_eq!(status, StatusCode::OK, "{c}");
    assert_eq!(c["quote"], "the report below");
    assert_eq!(c["url"], url.as_str());
    assert_eq!(c["link"], format!("{url}#:~:text=See-,the%20report%20below"));
    let y = open_link(&ctrl, c["link"].as_str().unwrap()).await;
    assert!(y > 1000.0, "Chrome scrolled to the quote: {y}");
    assert!(in_view(&ctrl, "See the report below.").await, "the quote is on screen");
    assert!(!in_view(&ctrl, "The Report Below").await, "not the decoy");

    // Encoded: %, €, commas; still opens at it
    let (_, c) = cite(&e, json!({"text": "Revenue grew 12% to €4.2m in Q3,"})).await;
    assert_eq!(c["link"], format!("{url}#:~:text=Revenue%20grew%2012%25%20to%20%E2%82%AC4.2m%20in%20Q3%2C"), "{c}");
    ctrl.navigate(&url).await.unwrap();
    assert!(open_link(&ctrl, c["link"].as_str().unwrap()).await > 1000.0);
    assert!(in_view(&ctrl, "Revenue grew").await);

    // Refused, with the reason: not there, twice, other capitals, half a word, nothing given
    ctrl.navigate(&url).await.unwrap();
    let refused = |v: &Value| v["error"].as_str().unwrap_or_default().to_string();
    let (status, v) = cite(&e, json!({"text": "Revenue fell 12%"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(refused(&v).contains("aren't on the page"), "{v}");
    assert!(refused(&cite(&e, json!({"text": "Same words here."})).await.1).contains("60 times"));
    assert!(refused(&cite(&e, json!({"text": "see the report below"})).await.1).contains("other capitals"));
    assert!(refused(&cite(&e, json!({"text": "evenue grew"})).await.1).contains("middle of a word"));
    assert!(refused(&cite(&e, json!({})).await.1).contains("Give the exact words"));

    // In the frame: linked to the frame's own address
    let (_, c) = cite(&e, json!({"text": "the light is very bright"})).await;
    assert_eq!(c["url"], url.replace("/long", "/frame"), "{c}");

    // A read with blocks: ids instead of the plain text, and its own hash for "unchanged"
    let plain = page(&e, "").await;
    let read = page(&e, "?blocks=1").await;
    assert!(read.get("text").is_none(), "{read}");
    let lines: Vec<&str> = read["blocks"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(lines[0].starts_with("[b1-") && lines[0].ends_with("] Quarterly notes"), "{lines:?}");
    assert!(read["frames_note"].is_string(), "{read}");
    assert_ne!(read["hash"], plain["hash"]);
    let again = page(&e, &format!("?blocks=1&since={}", read["hash"].as_str().unwrap())).await;
    assert_eq!(again["unchanged"], true, "{again}");
    let line = lines.iter().find(|l| l.ends_with("See the report below.")).unwrap();
    let id = &line[1..line.find(']').unwrap()];
    let (_, c) = cite(&e, json!({"block": id})).await;
    assert_eq!(c["quote"], "See the report below.", "{c}");
    assert_eq!(c["link"], format!("{url}#:~:text=See%20the%20report%20below."));
    let (_, v) = cite(&e, json!({"block": "b2-0000"})).await;
    assert!(refused(&v).contains("isn't on the page any more"), "{v}");

    // browser_cite in a turn: the same, as a tool; a refusal is a tool error, never a quote
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        sandbox_target: Some(e.bot.id.clone()),
        turn_id: Some("t1".into()),
        ..Default::default()
    };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&e.state, &e.bot, None, None, Some(scope)).await.unwrap();
    let call = |args: Value| {
        let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "browser_cite", "arguments": args}});
        send(&e.app, "POST", "/api/mcp/self", Some(&token), rpc)
    };
    let ok = call(json!({"text": "See the report below."})).await;
    assert_ne!(ok["result"]["isError"], true, "{ok}");
    let c: Value = serde_json::from_str(ok["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(c["link"], format!("{url}#:~:text=See%20the%20report%20below."));
    let bad = call(json!({"text": "The report above"})).await;
    assert_eq!(bad["result"]["isError"], true, "{bad}");
    assert!(bad["result"]["content"][0]["text"].as_str().unwrap().contains("aren't on the page"), "{bad}");
    mcp_runtime::finish_turn(&e.state, &token).await;
}

/// The same on a public page (network): a block far down GNU's "What is Free Software?" essay
/// (long, static, no cookie banner) is cited, and its link opens the essay scrolled to it.
#[tokio::test]
#[ignore = "needs the network"]
async fn a_public_page_quote_opens_at_it() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    ctrl.navigate("https://www.gnu.org/philosophy/free-sw.en.html").await.unwrap();
    let read = page(&e, "?blocks=1").await;
    let lines: Vec<&str> = read["blocks"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    // A sentence-long block well down the article
    let line = lines[lines.len() / 2..]
        .iter()
        .find(|l| l.len() > 80 && l.len() < 300 && !l.ends_with('…'))
        .unwrap_or_else(|| panic!("{lines:?}"));
    let id = &line[1..line.find(']').unwrap()];
    let (status, c) = cite(&e, json!({"block": id})).await;
    assert_eq!(status, StatusCode::OK, "{c}");
    eprintln!("cited: {c}");
    let quote = c["quote"].as_str().unwrap();
    let y = open_link(&ctrl, c["link"].as_str().unwrap()).await;
    let start: String = quote.chars().take(25).collect();
    assert!(y > 300.0, "scrolled: {y}");
    assert!(in_view(&ctrl, &start).await, "the quote is on screen: {start}");

    // A made-up quote is refused
    let (status, v) = cite(&e, json!({"text": "Free software was invented on the Moon in 1802."})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[tokio::test]
async fn a_lookalike_earlier_on_the_page_never_gets_the_link() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await.replace("/long", "");
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.expect("local Chrome session");
    for (n, (_, _, quote)) in LOOKALIKES.iter().enumerate() {
        // Same words around both: a bare fragment opens at the decoy in Chrome, so citing refuses
        let url = format!("{base}/look/{n}/same");
        ctrl.navigate(&url).await.unwrap();
        let (status, c) = cite(&e, json!({"text": quote})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{quote}: {c}");
        assert!(c["error"].as_str().unwrap().contains("Couldn't make a link"), "{quote}: {c}");
        open_link(&ctrl, &format!("{url}#:~:text={}", fragment_words(quote))).await;
        assert!(id_in_view(&ctrl, "d").await && !id_in_view(&ctrl, "t").await, "{quote}: Chrome treats the decoy as the same words");

        // Other words on the target's line: they go in the link, and it opens at the target
        let url = format!("{base}/look/{n}/note");
        ctrl.navigate(&url).await.unwrap();
        let (status, c) = cite(&e, json!({"text": quote})).await;
        assert_eq!(status, StatusCode::OK, "{quote}: {c}");
        let link = c["link"].as_str().unwrap();
        assert!(link.contains("#:~:text=New%20note"), "{quote}: {link}");
        let y = open_link(&ctrl, link).await;
        assert!(y > 3000.0, "{quote}: Chrome scrolled past the decoy: {y}");
        assert!(id_in_view(&ctrl, "t").await, "{quote}: the target is on screen");
        assert!(!id_in_view(&ctrl, "d").await, "{quote}: not the decoy");
    }

    // Text and a block together: refused, not one picked silently
    let (status, c) = cite(&e, json!({"text": "New note", "block": "b1-00000000"})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{c}");
    assert!(c["error"].as_str().unwrap().contains("not both"), "{c}");
}
