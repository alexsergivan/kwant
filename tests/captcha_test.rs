//! CAPTCHA hand-off: the page probe finds each vendor's widget (and its box) and ignores
//! look-alikes, bot navigation declines cookie banners but never accepts them, VM-mode cookies
//! outlive a browser restart, and a bot that lands on a slider gets a step with the widget's box
//! that closes by itself once the slider is dragged, or times out so the turn moves on.
//!
//! The browser parts run a real windowless Chrome as the "VM" browser (an external CDP endpoint)
//! and are skipped when none is installed; the time-out part needs no browser.

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
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{computer, service::BrowserService, BrowserController},
    server::{
        browser_stream::BrowserInput,
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

/// Serves `tests/fixtures/captcha/<name>` on 127.0.0.1, plus `/set-cookie` (a persistent cookie).
async fn serve_fixtures() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/captcha");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let dir = dir.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                let (status, extra, body) = if path == "set-cookie" {
                    ("200 OK", "set-cookie: kwnt_test=kept; Max-Age=86400; Path=/\r\n", "<title>cookie set</title>".to_string())
                } else if path.contains("..") || path.contains('/') {
                    ("404 Not Found", "", String::new())
                } else {
                    match std::fs::read_to_string(dir.join(&path)) {
                        Ok(body) => ("200 OK", "", body),
                        Err(_) => ("404 Not Found", "", String::new()),
                    }
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: text/html\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

/// A windowless Chrome on its own profile, standing in for the VM browser.
async fn start_chrome(profile: &Path) -> Option<(String, ChromeGuard)> {
    if computer::local_chrome_path().is_none() {
        eprintln!("browser part skipped: no Chrome or Chromium installed");
        return None;
    }
    let (endpoint, _) = computer::ensure_local_chrome(profile, true).await.expect("Chrome starts");
    Some((endpoint, ChromeGuard(profile.to_path_buf())))
}

async fn state_with(tmp: &TempDir, endpoint: Option<String>) -> AppState {
    // the VM path (per-bot contexts, cookie jar) on an external endpoint, as in Docker
    std::env::set_var("KWNT_COMPUTER_MODE", "vm");
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: endpoint, ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url))
}

async fn open(ctrl: &BrowserController, url: &str) {
    ctrl.navigate_unlocked(url).await.unwrap();
    for _ in 0..50 {
        let ready = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "document.readyState", "returnByValue": true})).await.unwrap();
        if ready["result"]["value"] == "complete" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn title(ctrl: &BrowserController) -> String {
    ctrl.sync_location().await.map(|(_, t)| t).unwrap_or_default()
}

/// The kind and the box size a fixture should give.
type Expected = (&'static str, f64, f64);

#[tokio::test]
async fn the_probe_finds_each_vendor_with_its_box_and_ignores_lookalikes() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = state_with(&tmp, Some(endpoint)).await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].id.clone();
    let ctrl = kwnt::server::routes::get_or_create_browser_controller(&state, &bot).await.expect("browser");
    let base = serve_fixtures().await;
    let cases: [(&str, Option<Expected>); 20] = [
        // a bare sign-up form with nothing but the checkbox counts as a wall
        ("recaptcha.html", Some(("recaptcha", 304.0, 78.0))),
        // an open image challenge counts on any page, with the challenge's box
        ("recaptcha_challenge.html", Some(("recaptcha", 400.0, 580.0))),
        ("hcaptcha.html", Some(("hcaptcha", 303.0, 78.0))),
        ("turnstile.html", Some(("turnstile", 300.0, 65.0))),
        ("arkose.html", Some(("arkose", 400.0, 400.0))),
        ("datadome.html", Some(("datadome", 400.0, 500.0))),
        ("geetest.html", Some(("geetest", 342.0, 382.0))),
        ("perimeterx.html", Some(("perimeterx", 312.0, 102.0))),
        ("tiktok_slider.html", Some(("tiktok", 360.0, 380.0))),
        ("shadow.html", Some(("tiktok", 360.0, 300.0))),
        // a slider prompt in a captcha-like box on a site without a known vendor
        ("dialog_puzzle.html", Some(("puzzle", 402.0, 322.0))),
        // reCAPTCHA v3's badge is no challenge, a hidden widget isn't shown, an article about
        // sliders is just text
        ("recaptcha_v3_badge.html", None),
        ("hidden.html", None),
        ("article.html", None),
        ("consent_onetrust.html", None),
        // look-alikes: a v2 checkbox on a contact form or in a footer form is something to tick
        // when submitting, not a wall, and neither is its closed challenge; "press and hold" in a
        // newsletter dialog or a sticky cookie bar isn't a CAPTCHA
        ("contact_recaptcha.html", None),
        ("footer_recaptcha.html", None),
        ("recaptcha_challenge_closed.html", None),
        ("consent_dialog_phrases.html", None),
        // below the fold: found, but without a box, and the page isn't scrolled
        ("offscreen.html", Some(("hcaptcha", 0.0, 0.0))),
    ];
    for (file, want) in cases {
        open(&ctrl, &format!("{base}{file}")).await;
        let hit = ctrl.probe_captcha().await.unwrap();
        let scrolled = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "window.scrollX + window.scrollY", "returnByValue": true})).await.unwrap();
        assert_eq!(scrolled["result"]["value"].as_f64(), Some(0.0), "{file}: the probe must never scroll");
        match (want, hit) {
            (None, None) => {}
            (Some((kind, 0.0, 0.0)), Some(hit)) => {
                assert_eq!(hit.kind, kind, "{file}");
                assert!(hit.offscreen && hit.clone().on_screen().is_none(), "{file}: {hit:?}");
            }
            (Some((kind, w, h)), Some(hit)) => {
                assert_eq!(hit.kind, kind, "{file}");
                assert!(!hit.offscreen, "{file}: {hit:?}");
                assert!((hit.width - w).abs() <= 2.0 && (hit.height - h).abs() <= 2.0, "{file}: {hit:?}");
                assert!(hit.x >= 0.0 && hit.y >= 0.0 && hit.x + hit.width <= 1280.0 && hit.y + hit.height <= 800.0, "{file}: {hit:?}");
            }
            (want, hit) => panic!("{file}: wanted {want:?}, got {hit:?}"),
        }
    }
    open(&ctrl, &format!("{base}tiktok_slider.html")).await;
    let hit = ctrl.probe_captcha().await.unwrap().unwrap();
    assert_eq!((hit.x, hit.y), (460.0, 180.0));
}

#[tokio::test]
async fn bot_navigation_declines_cookie_banners_and_never_accepts_one() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = state_with(&tmp, Some(endpoint)).await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].id.clone();
    let ctrl = kwnt::server::routes::get_or_create_browser_controller(&state, &bot).await.expect("browser");
    let base = serve_fixtures().await;
    for (file, want) in [
        ("consent_onetrust.html", "rejected"),
        ("consent_tiktok.html", "declined"),
        // only accept buttons: left alone for the person
        ("consent_accept_only.html", "untouched"),
    ] {
        ctrl.navigate(&format!("{base}{file}")).await.unwrap();
        assert_eq!(title(&ctrl).await, want, "{file}");
    }
    // The person's own navigation from the live view never touches banners
    ctrl.navigate_unlocked(&format!("{base}consent_onetrust.html")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(title(&ctrl).await, "OneTrust");
}

async fn cookie_names(ctrl: &BrowserController) -> Vec<String> {
    let all = ctrl.send_cdp_command("Network.getAllCookies", json!({})).await.unwrap();
    all["cookies"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn vm_cookies_are_kept_per_bot_across_a_browser_restart() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    std::env::set_var("KWNT_COMPUTER_MODE", "vm");
    let root = tmp.path().join("data");
    std::fs::create_dir_all(&root).unwrap();
    let base = serve_fixtures().await;
    let service = BrowserService::new(root.clone(), Some(endpoint));
    let a = service.session("bot-a", &root).await.unwrap();
    let b = service.session("bot-b", &root).await.unwrap();
    a.navigate(&format!("{base}set-cookie")).await.unwrap();
    assert!(cookie_names(&a).await.contains(&"kwnt_test".to_string()));
    assert!(!cookie_names(&b).await.contains(&"kwnt_test".to_string()), "each bot's context has its own cookies");
    let jar = service.cookie_jar("bot-a");
    let saved = std::fs::read_to_string(&jar).expect("bot navigation saves the jar");
    assert!(saved.contains("kwnt_test"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&jar).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(jar.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }
    assert!(!service.cookie_jar("bot-b").exists() || !std::fs::read_to_string(service.cookie_jar("bot-b")).unwrap().contains("kwnt_test"));

    // The container restarts: a new browser on an empty (tmpfs) profile
    drop((a, b, service, chrome));
    let _ = computer::wait_local_chrome_gone(&tmp.path().join("chrome")).await;
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome-after-restart")).await else { return };
    let service = BrowserService::new(root.clone(), Some(endpoint));
    let a = service.session("bot-a", &root).await.unwrap();
    assert!(cookie_names(&a).await.contains(&"kwnt_test".to_string()), "restored into the fresh context");
    open(&a, &format!("{base}article.html")).await;
    let cookie = a.send_cdp_command("Runtime.evaluate", json!({"expression": "document.cookie", "returnByValue": true})).await.unwrap();
    assert_eq!(cookie["result"]["value"], "kwnt_test=kept");
    let b = service.session("bot-b", &root).await.unwrap();
    assert!(!cookie_names(&b).await.contains(&"kwnt_test".to_string()));

    // A reset is a clean slate
    service.reset("bot-a").await.unwrap();
    assert!(!jar.exists());
}

// The bot's turn, through kwnt's own MCP tools

async fn post(app: &Router, uri: &str, token: &str, body: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"));
    let resp = app.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap()
}

async fn turn(state: &AppState, turn_id: &str) -> String {
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some(turn_id.into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(state, &bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

/// Calls a self tool in the background, as the CLI would while the turn waits.
fn call(app: &Router, token: &str, name: &str, args: Value) -> tokio::task::JoinHandle<(String, bool)> {
    let (app, token, name) = (app.clone(), token.to_string(), name.to_string());
    tokio::spawn(async move {
        let v = post(&app, "/api/mcp/self", &token, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": args}})).await;
        (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
    })
}

async fn open_step(state: &AppState) -> kwnt::db::human_steps::HumanStep {
    for _ in 0..200 {
        if let Some(s) = state.db.list_human_steps(Some("open"), None, None).await.unwrap().pop() {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no step was raised");
}

#[tokio::test]
async fn a_bot_landing_on_a_slider_gets_a_step_with_its_box_that_closes_once_solved_or_times_out() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = state_with(&tmp, Some(endpoint)).await;
    state.human_steps.set_captcha_timing(Duration::from_secs(60), Duration::from_millis(200));
    let app = create_router(state.clone());
    let base = serve_fixtures().await;
    let token = turn(&state, "turn-solve").await;

    // The bot opens a page that shows TikTok's slider: a step opens with the widget's box
    let task = call(&app, &token, "browser_open", json!({"url": format!("{base}tiktok_slider.html")}));
    let step = open_step(&state).await;
    assert_eq!((step.kind.as_str(), step.source.as_str()), ("captcha", "auto"));
    let json = serde_json::to_value(&step).unwrap();
    assert_eq!(json["target"], json!({"kind": "tiktok", "x": 460.0, "y": 180.0, "width": 360.0, "height": 380.0}), "{json}");
    let card = state.db.get_message(step.message_id.as_deref().unwrap()).await.unwrap().unwrap();
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["target"]["kind"], "tiktok");
    let ctrl = state.browser_controllers.lock().await.get(step.sandbox_target.as_deref().unwrap()).cloned().unwrap();
    // raise() holds the browser before the step is saved, so this can't see the step first
    assert!(ctrl.is_held_for_person());
    assert!(!task.is_finished(), "the turn waits for the person");

    // The person drags the slider in the live view: press, move with the button held, release
    let knob = (510.0, 500.0);
    for input in [
        BrowserInput::PointerDown { x: knob.0, y: knob.1 },
        BrowserInput::PointerMove { x: knob.0 + 80.0, y: knob.1 },
        BrowserInput::PointerMove { x: knob.0 + 240.0, y: knob.1 + 2.0 },
        BrowserInput::PointerUp { x: knob.0 + 240.0, y: knob.1 + 2.0 },
    ] {
        ctrl.dispatch_input(input).await.unwrap();
    }
    let log = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "window.dragLog ? dragLog.join(',') : ''", "returnByValue": true})).await.unwrap();
    assert_eq!(log["result"]["value"], "down:1,move:1,move:1,up:0", "the page sees a real held drag");

    // Nobody presses Done: the check is gone, so the step closes and the bot carries on
    let (text, is_error) = tokio::time::timeout(Duration::from_secs(20), task).await.expect("the turn resumes").unwrap();
    assert!(!is_error, "{text}");
    assert!(text.contains("human check is gone"), "{text}");
    assert!(text.contains("TikTok - solved"), "the bot sees the page after: {text}");
    let closed = state.db.get_human_step(&step.id).await.unwrap().unwrap();
    assert_eq!(closed.status, "resolved_auto");
    assert!(!ctrl.is_held_for_person(), "the browser is the bot's again");

    // Nobody solves the next one: it times out and the bot is told to move on, never hanging
    state.human_steps.set_captcha_timing(Duration::from_millis(1500), Duration::from_millis(200));
    let token = turn(&state, "turn-timeout").await;
    let task = call(&app, &token, "browser_open", json!({"url": format!("{base}tiktok_slider.html")}));
    let step = open_step(&state).await;
    let (text, is_error) = tokio::time::timeout(Duration::from_secs(20), task).await.expect("the turn never hangs").unwrap();
    assert!(!is_error, "{text}");
    assert!(text.contains("didn't solve the CAPTCHA in time") && text.contains("skip this site"), "{text}");
    assert_eq!(state.db.get_human_step(&step.id).await.unwrap().unwrap().status, "timed_out");
    assert!(!ctrl.is_held_for_person());
    // Done after the fact is refused, the step is settled
    let resp = app.clone().oneshot(Request::builder().method("POST").uri(format!("/api/human-steps/{}/done", step.id)).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_captcha_step_nobody_solves_times_out_without_a_browser() {
    let tmp = tempdir().unwrap();
    let state = state_with(&tmp, None).await;
    state.human_steps.set_captcha_timing(Duration::from_millis(800), Duration::from_millis(100));
    let app = create_router(state.clone());
    let token = turn(&state, "turn-ask").await;
    let started = std::time::Instant::now();
    let task = call(&app, &token, "request_human_step", json!({"kind": "captcha", "reason": "Solve the check on example.com"}));
    let step = open_step(&state).await;
    assert!(step.target.is_none());
    let (text, is_error) = tokio::time::timeout(Duration::from_secs(15), task).await.expect("the turn never hangs").unwrap();
    assert!(!is_error, "{text}");
    assert!(text.contains("didn't solve the CAPTCHA in time"), "{text}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(state.db.get_human_step(&step.id).await.unwrap().unwrap().status, "timed_out");
}

#[tokio::test]
async fn the_captcha_time_limit_is_a_checked_setting() {
    let tmp = tempdir().unwrap();
    let state = state_with(&tmp, None).await;
    let app = create_router(state.clone());
    let get = || async {
        let resp = app.clone().oneshot(Request::builder().uri("/api/settings").body(Body::empty()).unwrap()).await.unwrap();
        let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
        v["captcha_timeout_minutes"].clone()
    };
    let save = |body: Value| {
        let app = app.clone();
        async move {
            app.oneshot(Request::builder().method("POST").uri("/api/settings").header("content-type", "application/json")
                .body(Body::from(body.to_string())).unwrap()).await.unwrap().status()
        }
    };
    assert_eq!(get().await, 10);
    assert_eq!(kwnt::server::human_steps::captcha_timeout(&state).await, Duration::from_secs(600));
    assert_eq!(save(json!({"captcha_timeout_minutes": 0})).await, StatusCode::BAD_REQUEST);
    assert_eq!(save(json!({"captcha_timeout_minutes": 61})).await, StatusCode::BAD_REQUEST);
    assert_eq!(save(json!({"captcha_timeout_minutes": 3})).await, StatusCode::OK);
    assert_eq!(get().await, 3);
    assert_eq!(kwnt::server::human_steps::captcha_timeout(&state).await, Duration::from_secs(180));
}
