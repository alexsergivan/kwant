//! The payment floor reads the page in kwnt's own browser before a `browser_act` (D8): a plain
//! "Continue" on a card form, a label that matches a "Place order" button, a submit into a
//! checkout form, and Enter in a card form all ask, with approvals off, through a real Chrome.
//! The shell bots' `/act` API can't show a card, so it refuses those and does nothing.
//! Skipped when no Chrome is installed. `saucedemo_checkout_asks_before_finish` (ignored, needs
//! the network) walks a real demo shop to its last step.
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
    mcp::{ApprovalOutcome, ToolApprovalRequest, ToolApprover, SELF_SERVER_NAME},
    router::LayaRouter,
    sandbox::computer,
    server::{
        continuity::{browser_act_payment, SelfToolsApprover},
        create_router, mcp_runtime,
        routes::get_or_create_browser_controller,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const CARD_PAGE: &str = "<!doctype html><title>card</title><form onsubmit=\"document.title='sent';return false\">\
<input name=name placeholder='Name on card'><input autocomplete=cc-number placeholder='1234 1234 1234 1234'>\
<input name=cvc placeholder=CVC><button type=button onclick=\"history.back()\">Cancel</button>\
<button type=submit>Continue</button></form>";
const FRAME_PAGE: &str = "<!doctype html><title>frame</title>\
<iframe title='Secure card payment input frame' srcdoc='<input>' style='width:300px;height:40px'></iframe>\
<button onclick=\"document.title='next'\">Next</button>";
const ORDER_PAGE: &str = "<!doctype html><title>order</title>\
<button onclick=\"document.title='ordered'\">Place order</button><a href='#help'>Help</a>";
const FORM_PAGE: &str = "<!doctype html><title>form</title><form action='/shop/checkout/confirm' method=post>\
<input name=email placeholder=Email><button type=submit>Continue</button></form>\
<form action='/search'><input name=q placeholder=Search></form>";
const FINISH_PAGE: &str = "<!doctype html><title>overview</title>\
<button autofocus onclick=\"document.title='finished'\">Finish</button>";
const PLAIN_PAGE: &str = "<!doctype html><title>plain</title>\
<button onclick=\"document.title='went on'\">Continue</button><input name=q placeholder=Search>";

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
                    "/card" => CARD_PAGE,
                    "/frame" => FRAME_PAGE,
                    "/order" => ORDER_PAGE,
                    "/form" => FORM_PAGE,
                    "/checkout-step-two.html" => FINISH_PAGE,
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

async fn title(ctrl: &kwnt::sandbox::BrowserController) -> String {
    ctrl.sync_location().await.map(|(_, t)| t).unwrap_or_default()
}

async fn pay(e: &Env, args: Value) -> Option<String> {
    browser_act_payment(&e.state, &e.bot.id, &args).await
}

fn act_request(e: &Env, args: Value) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        turn_id: "t-pay".into(),
        tool_name: format!("mcp__{SELF_SERVER_NAME}__browser_act"),
        server: SELF_SERVER_NAME.into(),
        tool: "browser_act".into(),
        arguments: args,
        builtin: None,
        smart: None,
        payment: None,
    }
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

#[tokio::test]
async fn the_page_decides_whether_a_browser_click_pays() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let click = |label: &str| json!({"action": "click_text", "text": label});

    // A plain "Continue" on a card form asks; Cancel and clicking into a field don't
    open(&e, &format!("{base}/card")).await;
    let why = pay(&e, click("Continue")).await.expect("Continue on a card form");
    assert!(why.contains("card fields"), "{why}");
    assert_eq!(pay(&e, click("Cancel")).await, None);
    assert_eq!(pay(&e, click("CVC")).await, None, "clicking into a field");
    // Enter in the card form asks too (focus the CVC field first)
    let ctrl = open(&e, &format!("{base}/card")).await;
    ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "document.querySelector('[name=cvc]').focus()"})).await.unwrap();
    assert!(pay(&e, json!({"action": "key", "key": "Enter"})).await.is_some());
    assert_eq!(pay(&e, json!({"action": "key", "key": "Tab"})).await, None);

    // A card frame from a payment provider, known by its title
    open(&e, &format!("{base}/frame")).await;
    assert!(pay(&e, click("Next")).await.expect("Next beside a card frame").contains("payment frame"));

    // The bot writes "order"; the button it would really hit is "Place order"
    open(&e, &format!("{base}/order")).await;
    assert!(pay(&e, click("order")).await.expect("matched Place order").contains("place order"));
    assert_eq!(pay(&e, click("Help")).await, None);

    // A submit whose form posts to a checkout address; the search form's Enter doesn't ask
    let ctrl = open(&e, &format!("{base}/form")).await;
    assert!(pay(&e, click("Continue")).await.expect("checkout form").contains("checkout"));
    ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "document.querySelector('[name=q]').focus()"})).await.unwrap();
    assert_eq!(pay(&e, json!({"action": "key", "key": "Enter"})).await, None);

    // Nothing to pay on a plain page
    open(&e, &format!("{base}/plain")).await;
    assert_eq!(pay(&e, click("Continue")).await, None);
}

#[tokio::test]
async fn space_on_a_focused_button_asks_like_enter() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let space = json!({"action": "key", "key": " "});

    // An autofocused "Finish" on a checkout page: Space presses it, so it asks like Enter and click
    let ctrl = open(&e, &format!("{base}/checkout-step-two.html")).await;
    for args in [space.clone(), json!({"action": "key", "key": "Space"}), json!({"action": "key", "key": "Enter"})] {
        assert!(pay(&e, args.clone()).await.is_some(), "{args}");
    }
    let (status, page) = post_act(&e.app, &e.bot.id, space.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page["error"].as_str().unwrap_or("").starts_with("Not done: this looks like a payment"), "{page}");
    assert_eq!(title(&ctrl).await, "overview", "Finish was not pressed");

    // A focused "Place order" asks by its label, on any address
    let ctrl = open(&e, &format!("{base}/order")).await;
    ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "document.querySelector('button').focus()"})).await.unwrap();
    assert!(pay(&e, space.clone()).await.expect("Space on Place order").contains("place order"));

    // Space on a plain page's button runs
    let ctrl = open(&e, &format!("{base}/plain")).await;
    ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": "document.querySelector('button').focus()"})).await.unwrap();
    assert_eq!(pay(&e, space).await, None);
}

#[tokio::test]
async fn approvals_off_still_cards_a_card_form_click_and_the_shell_api_refuses_it() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let base = serve().await;
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/kwants/{}/approvals", e.bot.id))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", e.state.api_token.reveal()))
        .body(Body::from(json!({"approvals": "off"}).to_string()))
        .unwrap();
    assert_eq!(e.app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
    let inner = mcp_runtime::BrokerApprover {
        db: e.state.db.clone(),
        data_dir: e.state.config.data_dir.clone(),
        broker: e.state.broker.clone(),
        events: e.state.event_sender.clone(),
        timeout: Duration::from_millis(300),
        conversation: "c-pay".into(),
        chat: None,
        always_ask: false,
    };
    let approver = SelfToolsApprover {
        inner: Arc::new(inner),
        always_ask: false,
        browser: Some((e.state.clone(), e.bot.id.clone())),
    };
    let ctrl = open(&e, &format!("{base}/card")).await;
    let continue_ = json!({"action": "click_text", "text": "Continue"});
    // Unanswered: the card was shown and nobody answered it in time
    assert_eq!(approver.decide(act_request(&e, continue_.clone())).await, ApprovalOutcome::Unanswered);
    // The shell bots' API can't ask, so it doesn't click and says why
    let (status, page) = post_act(&e.app, &e.bot.id, continue_).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page["error"].as_str().unwrap().starts_with("Not done: this looks like a payment"), "{page}");
    assert!(page["payment"].as_str().unwrap().contains("card fields"), "{page}");
    assert_eq!(title(&ctrl).await, "card", "the form was not sent");

    // On a plain page both run without a card
    let ctrl = open(&e, &format!("{base}/plain")).await;
    assert_eq!(
        approver.decide(act_request(&e, json!({"action": "click_text", "text": "Continue"}))).await,
        ApprovalOutcome::Allowed
    );
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": "Continue"})).await;
    assert!(page.get("error").is_none(), "{page}");
    assert_eq!(title(&ctrl).await, "went on");
}

/// A real demo shop (needs the network): sign in, add a backpack, go through checkout. Nothing on
/// the way asks; "Finish" on the overview page does. Run with `--ignored`.
#[tokio::test]
#[ignore]
async fn saucedemo_checkout_asks_before_finish() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let e = setup().await;
    let _chrome = ChromeGuard(e.tmp.path().join("local-chrome"));
    let ctrl = open(&e, "https://www.saucedemo.com/").await;
    let js = |expr: &str| json!({"expression": expr, "awaitPromise": true});
    let fill = |sel: &str, v: &str| {
        format!(
            "(function(){{var el=document.querySelector('{sel}');var set=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set;set.call(el,'{v}');el.dispatchEvent(new Event('input',{{bubbles:true}}));}})()"
        )
    };
    ctrl.send_cdp_command("Runtime.evaluate", js(&fill("#user-name", "standard_user"))).await.unwrap();
    ctrl.send_cdp_command("Runtime.evaluate", js(&fill("#password", "secret_sauce"))).await.unwrap();
    let step = |label: &'static str| {
        let e = &e;
        async move {
            let args = json!({"action": "click_text", "text": label});
            assert_eq!(pay(e, args.clone()).await, None, "{label} should not ask");
            let (_, page) = post_act(&e.app, &e.bot.id, args).await;
            assert!(page.get("error").is_none(), "{label}: {page}");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    step("Login").await;
    step("Add to cart").await;
    ctrl.navigate("https://www.saucedemo.com/cart.html").await.unwrap();
    // The cart page itself isn't a checkout page; the button names it, so it asks
    assert!(pay(&e, json!({"action": "click_text", "text": "Checkout"})).await.is_some());
    ctrl.navigate("https://www.saucedemo.com/checkout-step-one.html").await.unwrap();
    for (sel, v) in [("#first-name", "Ada"), ("#last-name", "Lovelace"), ("#postal-code", "10115")] {
        ctrl.send_cdp_command("Runtime.evaluate", js(&fill(sel, v))).await.unwrap();
    }
    let why = pay(&e, json!({"action": "click_text", "text": "Continue"})).await;
    assert!(why.is_some(), "a click on checkout-step-one asks by its address");
    ctrl.navigate("https://www.saucedemo.com/checkout-step-two.html").await.unwrap();
    let why = pay(&e, json!({"action": "click_text", "text": "Finish"})).await.expect("Finish asks");
    eprintln!("Finish: {why}");
    let (_, page) = post_act(&e.app, &e.bot.id, json!({"action": "click_text", "text": "Finish"})).await;
    assert!(page["error"].as_str().unwrap_or("").starts_with("Not done"), "{page}");
    assert!(page["url"].as_str().unwrap().contains("checkout-step-two"), "{page}");
}
