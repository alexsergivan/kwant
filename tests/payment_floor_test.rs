//! The payment floor (Phase C D8): with approvals off everything runs without a card except a
//! payment, which asks in every approval mode, even when "Ask only when risky" says it's safe or
//! the person said "always allow". Allowing a payment covers that one call.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    mcp::{payment_floor, ApprovalOutcome, SmartVerdict, ToolApprovalRequest, ToolApprover, SELF_SERVER_NAME},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, SelfToolsApprover, TurnOrigin},
        create_router, mcp_runtime,
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const STRIPE_POST: &str = "curl -X POST https://api.stripe.com/v1/charges -u sk_test_x: -d amount=100 -d currency=usd";

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let space = SpaceId::default_space();
    let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    Env { app: create_router(state.clone()), bot, general, state, _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn set_approvals(e: &Env, mode: &str) {
    let uri = format!("/api/kwants/{}/approvals", e.bot.id);
    let (s, v) = call(&e.app, "PUT", &uri, Some(e.state.api_token.reveal()), Some(json!({"approvals": mode}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

/// A live turn of the bot in #general with its own tools; returns (turn id, token).
async fn turn(e: &Env) -> (String, String, TempDir) {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let folder = tempdir().unwrap();
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() }),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    let (_, token) = mcp_runtime::prepare_turn_with_opts(
        &e.state,
        &e.bot,
        Some(turn_id.clone()),
        Some(folder.path().to_path_buf()),
        Some(scope),
        Default::default(),
    )
    .await
    .unwrap();
    e.state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.clone(),
        kwant_id: e.bot.id.clone(),
        context_id: e.general.clone(),
        thread_id: None,
        state: "running".into(),
        since: Utc::now(),
        purpose: None,
    });
    (turn_id, token, folder)
}

/// What claude's permission prompt gets for `tool {input}`: `Ok(behavior)` when kwnt answers
/// without a card, `Err((card details, behavior after the card got `answer`))` when it shows one.
async fn claude_asks(e: &Env, tool: &str, input: Value, answer: ApprovalDecision) -> Result<String, (Value, String)> {
    let (turn_id, token, _folder) = turn(e).await;
    let mut events = e.state.event_sender.subscribe();
    let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
        "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_1"}}});
    let app = e.app.clone();
    let path = format!("/api/mcp/permission/{token}");
    let mut prompt = tokio::spawn(async move { call(&app, "POST", &path, None, Some(ask)).await });
    let behavior = |v: &Value| {
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()["behavior"].as_str().unwrap().to_string()
    };
    let answered = tokio::time::timeout(Duration::from_millis(1500), &mut prompt).await;
    let card = loop {
        match events.try_recv() {
            Ok(RuntimeEvent::ApprovalRequired { id, details, .. }) => break Some((id, details)),
            Ok(_) => continue,
            Err(_) => break None,
        }
    };
    let out = match (answered, card) {
        (Ok(done), None) => Ok(behavior(&done.unwrap().1)),
        (Err(_), Some((id, details))) => {
            assert!(e.state.broker.resolve(&id, answer).await);
            let (_, v) = prompt.await.unwrap();
            Err((details, behavior(&v)))
        }
        (answered, card) => panic!("answered {} with card {card:?}", answered.is_ok()),
    };
    mcp_runtime::finish_turn(&e.state, &token).await;
    e.state.turns.end_turn(&turn_id);
    out
}

fn approver(e: &Env, timeout: Duration) -> mcp_runtime::BrokerApprover {
    mcp_runtime::BrokerApprover {
        db: e.state.db.clone(),
        data_dir: e.state.config.data_dir.clone(),
        broker: e.state.broker.clone(),
        events: e.state.event_sender.clone(),
        timeout,
        conversation: e.general.clone(),
        chat: None,
        always_ask: false,
    }
}

fn request(e: &Env, server: &str, tool: &str, arguments: Value) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        turn_id: "t-pay".into(),
        tool_name: if server.is_empty() { tool.to_string() } else { format!("mcp__{server}__{tool}") },
        server: server.into(),
        tool: tool.into(),
        arguments,
        builtin: None,
        smart: None,
        payment: None,
    }
}

fn bash(e: &Env, command: &str) -> ToolApprovalRequest {
    let mut req = request(e, "", "Bash", json!({"command": command}));
    req.builtin = Some(kwnt::mcp::BuiltinCall { cwd: None, outside_folder: false });
    req
}

#[tokio::test]
async fn with_approvals_off_only_the_payment_gets_a_card_and_a_denial_stops_it() {
    let e = setup().await;
    set_approvals(&e, "off").await;

    // Everything else runs without asking: a send, a delete, a GET to the payment API, a POST elsewhere
    for command in [
        "touch x",
        "rm -rf build",
        "curl https://api.stripe.com/v1/balance -u sk_test_x:",
        "curl -X POST https://api.github.com/repos/o/r/issues -d '{}'",
    ] {
        let got = claude_asks(&e, "Bash", json!({"command": command}), ApprovalDecision::Deny).await;
        assert_eq!(got, Ok("allow".to_string()), "{command}");
    }

    // The payment asks, and the person's Deny stops it
    let Err((details, behavior)) = claude_asks(&e, "Bash", json!({"command": STRIPE_POST}), ApprovalDecision::Deny).await else {
        panic!("a payment must show a card with approvals off");
    };
    assert_eq!(behavior, "deny");
    assert!(details["payment_reason"].as_str().unwrap().contains("stripe.com"), "{details}");
    let (_, rows) = call(&e.app, "GET", "/api/approvals", None, None).await;
    let row = rows.as_array().unwrap().iter().find(|r| r["details"]["payment_reason"].is_string()).expect("the card is on record");
    assert!(row["details"]["arguments"]["command"].as_str().unwrap().starts_with("curl -X POST"), "{row}");
    assert_ne!(row["status"], "auto_allowed", "{row}");

    // Allowing it lets it run
    let got = claude_asks(&e, "Bash", json!({"command": STRIPE_POST}), ApprovalDecision::AllowOnce).await;
    assert!(matches!(got, Err((_, ref b)) if b == "allow"), "{got:?}");
}

#[tokio::test]
async fn smart_allow_and_always_allow_never_skip_a_payment() {
    let e = setup().await;
    let a = approver(&e, Duration::from_millis(200));

    // "Ask only when risky" says it's safe: still a card
    set_approvals(&e, "smart").await;
    let mut req = bash(&e, STRIPE_POST);
    req.smart = Some(SmartVerdict::Allow("looked safe".into()));
    assert_eq!(a.decide(req).await, ApprovalOutcome::Unanswered);
    let mut req = bash(&e, "touch x");
    req.smart = Some(SmartVerdict::Allow("inside the folder".into()));
    assert_eq!(a.decide(req).await, ApprovalOutcome::Allowed, "non-payments keep the smart shortcut");

    // "Always allow" for Bash in this conversation: still a card, in ask and in off
    e.state.broker.allow_for_session(&e.bot.id, &e.general, "Bash").await;
    for mode in ["ask", "off"] {
        set_approvals(&e, mode).await;
        assert_eq!(a.decide(bash(&e, STRIPE_POST)).await, ApprovalOutcome::Unanswered, "{mode}");
        assert_eq!(a.decide(bash(&e, "touch x")).await, ApprovalOutcome::Allowed, "{mode}");
    }
}

#[tokio::test]
async fn always_allow_on_a_payment_card_covers_that_one_call() {
    let e = setup().await;
    set_approvals(&e, "ask").await;
    let mut events = e.state.event_sender.subscribe();
    let a = Arc::new(approver(&e, Duration::from_secs(5)));
    let pending = {
        let (a, req) = (a.clone(), bash(&e, STRIPE_POST));
        tokio::spawn(async move { a.decide(req).await })
    };
    let id = loop {
        if let RuntimeEvent::ApprovalRequired { id, .. } = events.recv().await.unwrap() {
            break id;
        }
    };
    assert!(e.state.broker.resolve(&id, ApprovalDecision::AlwaysAllowSession).await);
    assert_eq!(pending.await.unwrap(), ApprovalOutcome::Allowed);
    assert!(!e.state.broker.is_allowed_for_session(&e.bot.id, &e.general, "Bash").await, "nothing is remembered");
    let a = approver(&e, Duration::from_millis(200));
    assert_eq!(a.decide(bash(&e, STRIPE_POST)).await, ApprovalOutcome::Unanswered, "the next payment asks again");
}

#[tokio::test]
async fn a_payment_app_through_composio_asks_with_approvals_off() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    let a = approver(&e, Duration::from_millis(200));
    let multi = |slug: &str| {
        request(&e, "composio", "COMPOSIO_MULTI_EXECUTE_TOOL", json!({"tools": [{"tool_slug": slug, "arguments": {"amount": 500}}]}))
    };
    assert_eq!(a.decide(multi("STRIPE_CREATE_PAYMENT_INTENT")).await, ApprovalOutcome::Unanswered);
    assert_eq!(a.decide(request(&e, "paypal", "create_order", json!({}))).await, ApprovalOutcome::Unanswered);
    assert_eq!(a.decide(request(&e, "shop", "place_order", json!({}))).await, ApprovalOutcome::Unanswered);
    // Sending, deleting and publishing run without asking
    assert_eq!(a.decide(multi("GMAIL_SEND_EMAIL")).await, ApprovalOutcome::Allowed);
    assert_eq!(a.decide(multi("GOOGLEDRIVE_DELETE_FILE")).await, ApprovalOutcome::Allowed);
    assert_eq!(a.decide(request(&e, "wordpress", "publish_post", json!({}))).await, ApprovalOutcome::Allowed);
    assert_eq!(a.decide(multi("STRIPE_LIST_CHARGES")).await, ApprovalOutcome::Allowed, "reads don't pay");

    let (_, rows) = call(&e.app, "GET", "/api/approvals?status=waiting", None, None).await;
    let reasons: Vec<&str> = rows.as_array().unwrap().iter().filter_map(|r| r["details"]["payment_reason"].as_str()).collect();
    assert_eq!(reasons.len(), 3, "{rows}");
}

#[tokio::test]
async fn kwnt_browser_pay_clicks_and_card_numbers_ask_with_approvals_off() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    let a = SelfToolsApprover { inner: Arc::new(approver(&e, Duration::from_millis(200))), always_ask: false, browser: None };
    let act = |args: Value| request(&e, SELF_SERVER_NAME, "browser_act", args);
    assert_eq!(a.decide(act(json!({"action": "click_text", "text": "Place order"}))).await, ApprovalOutcome::Unanswered);
    assert_eq!(a.decide(act(json!({"action": "click_text", "text": "Pay $12.99"}))).await, ApprovalOutcome::Unanswered);
    assert_eq!(a.decide(act(json!({"action": "type", "text": "4242 4242 4242 4242"}))).await, ApprovalOutcome::Unanswered);
    assert_eq!(a.decide(act(json!({"action": "click_text", "text": "Continue"}))).await, ApprovalOutcome::Allowed);
    assert_eq!(a.decide(act(json!({"action": "type", "text": "hello"}))).await, ApprovalOutcome::Allowed);
    assert_eq!(a.decide(request(&e, SELF_SERVER_NAME, "memory_add", json!({"text": "pay now"}))).await, ApprovalOutcome::Allowed);
}

#[tokio::test]
async fn a_local_shell_payment_waits_for_the_person_with_approvals_off() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    let folder = tempdir().unwrap();
    let ran = tokio::time::timeout(
        Duration::from_secs(3),
        kwnt::server::computer::approve_local_shell(&e.state, &e.bot.id, "touch x", folder.path()),
    )
    .await
    .expect("no card");
    assert!(ran);
    let waiting = tokio::time::timeout(
        Duration::from_millis(300),
        kwnt::server::computer::approve_local_shell(&e.state, &e.bot.id, STRIPE_POST, folder.path()),
    )
    .await;
    assert!(waiting.is_err(), "a payment waits for the person");
}

/// kwnt's own tools don't pay. A new one whose name looks like paying must be classified in
/// payment_floor.rs before it ships.
#[tokio::test]
async fn self_tools_have_no_money_words() {
    let e = setup().await;
    e.state.db.set_setting("mission_board", "true").await.unwrap();
    let (turn_id, token, _f) = turn(&e).await;
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))).await;
    let names: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.len() > 20, "{names:?}");
    for name in names {
        assert_eq!(payment_floor::name_reason("", name), None, "{name} looks like a payment: classify it in payment_floor.rs");
    }
    mcp_runtime::finish_turn(&e.state, &token).await;
    e.state.turns.end_turn(&turn_id);
}
