//! Which sites a bot's browser may open, through a real Chrome: the rules are set on the bot
//! (only by the person, normalized, a bad entry refused), and then hold on every navigation the
//! bot causes (opening an address, a link click, a redirect, a new tab), while the person's own
//! clicks are never held to them. With no rules the guard doesn't run at all. Skipped when no
//! Chrome is installed.
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
    server::{create_router, routes::get_or_create_browser_controller, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Pages on 127.0.0.1, reached as `localhost`, `shop.localhost` (Chrome resolves every
/// `*.localhost` to loopback) or `127.0.0.1`: three sites without any network.
async fn serve() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let off = format!("http://127.0.0.1:{port}/plain");
                let reply = if path == "/redir" {
                    format!("HTTP/1.1 302 Found\r\nlocation: {off}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                } else {
                    let page = match path.as_str() {
                        "/links" => format!(
                            "<!doctype html><title>links</title><a href='{off}'>Off list</a> \
                             <a href='{off}' target=_blank>New tab</a> <a href='/plain'>Same site</a>"
                        ),
                        "/person" => format!(
                            "<!doctype html><title>person</title><style>body{{margin:0}}</style>\
                             <a href='{off}' style='display:block;height:400px;background:#eee'>Go</a>"
                        ),
                        _ => "<!doctype html><title>plain</title><p>plain</p>".to_string(),
                    };
                    format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}", page.len())
                };
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    port
}

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    tmp: TempDir,
}

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

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The bot's update body (the route takes the whole bot) with `sites` on top.
fn with_bot(e: &Env, sites: Value) -> Value {
    let b = &e.bot;
    let mut body = json!({"name": b.name, "title": b.title, "persona_prompt": b.persona_prompt, "soul": b.soul,
        "avatar_sprite": b.avatar_sprite, "preferred_model": b.preferred_model});
    body.as_object_mut().unwrap().extend(sites.as_object().unwrap().clone());
    body
}

async fn set_sites(e: &Env, sites: Value) -> (StatusCode, Value) {
    call(&e.app, "PUT", &format!("/api/kwants/{}", e.bot.id), Some(with_bot(e, sites)), None).await
}

async fn bot_open(e: &Env, url: &str) -> (StatusCode, Value) {
    call(&e.app, "POST", &format!("/api/sandbox/{}/navigate", e.bot.id), Some(json!({"url": url})), None).await
}

async fn bot_click(e: &Env, label: &str) -> Value {
    let (_, page) = call(&e.app, "GET", &format!("/api/sandbox/{}/page", e.bot.id), None, None).await;
    let lines = page["clickable"].as_array().unwrap_or_else(|| panic!("a clickable list in {page}"));
    let line = lines.iter().filter_map(Value::as_str).find(|l| l.contains(label)).unwrap_or_else(|| panic!("{label} in {page}"));
    let start = line.find('[').unwrap() + 1;
    let r: u64 = line[start..line.find(']').unwrap()].parse().unwrap();
    let (status, page) = call(&e.app, "POST", &format!("/api/sandbox/{}/act", e.bot.id), Some(json!({"action": "click", "ref": r})), None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page
}

async fn href(ctrl: &kwnt::sandbox::BrowserController) -> String {
    let v = ctrl
        .send_cdp_command("Runtime.evaluate", json!({"expression": "location.href", "returnByValue": true}))
        .await
        .unwrap();
    v["result"]["value"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn a_bots_site_rules_hold_on_every_navigation_it_causes() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let port = serve().await;
    let off = format!("http://127.0.0.1:{port}/plain");

    // Defaults: no rules, nothing checked and no guard running, as before
    let (_, list) = call(&e.app, "GET", "/api/kwants", None, None).await;
    let view = list.as_array().and_then(|l| l.iter().find(|k| k["id"] == json!(e.bot.id))).cloned().unwrap_or_else(|| panic!("{list}"));
    assert_eq!(view["browser_allowed_sites"], json!([]), "{view}");
    assert_eq!(view["browser_blocked_sites"], json!([]));
    assert_eq!(view["browser_block_private_network"], json!(false));
    let (status, nav) = bot_open(&e, &off).await;
    assert_eq!(status, StatusCode::OK, "{nav}");
    let ctrl = get_or_create_browser_controller(&e.state, &e.bot.id).await.unwrap();
    assert!(!ctrl.site_guard.is_running());

    // Only the person sets the rules; entries are normalized, a bad one refused whole
    let (status, _) = call(
        &e.app,
        "PUT",
        &format!("/api/kwants/{}", e.bot.id),
        Some(with_bot(&e, json!({"browser_allowed_sites": ["example.com"]}))),
        Some("some-bot-token"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, err) = set_sites(&e, json!({"browser_allowed_sites": ["ok.com", "a*b.com"]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    assert!(err["error"].as_str().unwrap().contains("a*b.com"), "{err}");
    let (status, view) = set_sites(&e, json!({"browser_allowed_sites": ["LOCALHOST.", "http://localhost/x"]})).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["browser_allowed_sites"], json!(["localhost"]));

    // The allow-list refuses an off-list site, naming the rule
    let (status, refused) = bot_open(&e, &off).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["site_rule"]["rule"], "browser_allowed_sites");
    assert!(refused["error"].as_str().unwrap().contains("isn't on this bot's allowed sites"), "{refused}");

    // A subdomain of an allowed site opens
    let (status, nav) = bot_open(&e, &format!("http://shop.localhost:{port}/links")).await;
    assert_eq!(status, StatusCode::OK, "{nav}");
    assert!(ctrl.site_guard.is_running());

    // A link click off the list is stopped and said; the bot stays where it was
    let page = bot_click(&e, "Off list").await;
    assert_eq!(page["site_rule"]["rule"], "browser_allowed_sites", "{page}");
    let at = href(&ctrl).await;
    assert!(at.starts_with("http://shop.localhost"), "{at}");
    // A same-site link still works
    let page = bot_click(&e, "Same site").await;
    assert!(page.get("site_rule").is_none(), "{page}");
    assert_eq!(href(&ctrl).await, format!("http://shop.localhost:{port}/plain"));

    // A new tab off the list is stopped too
    bot_open(&e, &format!("http://shop.localhost:{port}/links")).await;
    let pages = || async {
        let list = ctrl.send_cdp_command("Target.getTargets", json!({})).await.unwrap();
        list["targetInfos"].as_array().map(|l| l.iter().filter(|t| t["type"] == "page").count()).unwrap_or(0)
    };
    let before = pages().await;
    let page = bot_click(&e, "New tab").await;
    assert_eq!(page["site_rule"]["host"], "127.0.0.1", "{page}");
    // ...and the refused tab is closed, so only the bot's own page is left
    let mut tabs = 0;
    for _ in 0..20 {
        tabs = pages().await;
        if tabs == before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(tabs, before, "the refused tab was closed");

    // A redirect from an allowed site into an off-list one is stopped
    let (status, refused) = bot_open(&e, &format!("http://localhost:{port}/redir")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["site_rule"]["host"], "127.0.0.1");

    // The block-list wins over the allow-list
    let (status, _) = set_sites(&e, json!({"browser_blocked_sites": ["shop.localhost"]})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, refused) = bot_open(&e, &format!("http://shop.localhost:{port}/plain")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["site_rule"]["rule"], "browser_blocked_sites");
    assert_eq!(refused["site_rule"]["entry"], "shop.localhost");

    // The person's own click is never held to the bot's rules
    let (status, _) = bot_open(&e, &format!("http://localhost:{port}/person")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) =
        call(&e.app, "POST", &format!("/api/sandbox/{}/interact", e.bot.id), Some(json!({"action": "click", "x": 50, "y": 50})), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut landed = String::new();
    for _ in 0..30 {
        landed = href(&ctrl).await;
        if landed == off {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(landed, off, "the person's click went through");

    // Blocking private addresses: loopback by address or by a name resolving to it, internal
    // ranges and the cloud metadata address
    let (status, _) =
        set_sites(&e, json!({"browser_allowed_sites": [], "browser_blocked_sites": [], "browser_block_private_network": true})).await;
    assert_eq!(status, StatusCode::OK);
    for url in [off.clone(), format!("http://localhost:{port}/plain"), "http://10.1.2.3/".into(), "http://169.254.169.254/latest/meta-data/".into()] {
        let (status, refused) = bot_open(&e, &url).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{url}: {refused}");
        assert_eq!(refused["site_rule"]["rule"], "browser_block_private_network", "{url}");
    }

    // Everything off again: today's behaviour, and the guard stops
    let (status, view) = set_sites(&e, json!({"browser_block_private_network": false})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["browser_block_private_network"], json!(false));
    let (status, nav) = bot_open(&e, &off).await;
    assert_eq!(status, StatusCode::OK, "{nav}");
    assert!(!ctrl.site_guard.is_running());
}
