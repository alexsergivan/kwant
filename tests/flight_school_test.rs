//! Mission Control Phase C3: Flight School. A bot's tier for a class of tasks is a ceiling on top
//! of its approval mode: it can add a card, never remove one, and the payment floor stays first.
//! The contract's acceptance tests (docs/mission-control.md §14): the tier × effect table through
//! claude's real permission endpoint, approvals off and smart Allow still carding in sandbox,
//! observe carding an in-folder write, revoke gating the very next call, act and "off" being
//! today's path, the person-only routes, the task class a bot can't pick, and the engine guard.

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
    mcp::{ApprovalOutcome, SmartVerdict, ToolApprovalRequest, ToolApprover, SELF_SERVER_NAME},
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
use std::path::{Path, PathBuf};
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
    let e = Env { app: create_router(state.clone()), bot, general, state, _tmp: tmp };
    person(&e, "POST", "/api/settings", json!({"mission_board": true})).await;
    e
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

async fn person(e: &Env, method: &str, uri: &str, body: Value) -> Value {
    let body = (method != "GET").then_some(body);
    let (status, v) = call(&e.app, method, uri, Some(e.state.api_token.reveal()), body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

async fn set_approvals(e: &Env, mode: &str) {
    person(e, "PUT", &format!("/api/kwants/{}/approvals", e.bot.id), json!({"approvals": mode})).await;
}

async fn set_tier(e: &Env, class: &str, tier: &str) {
    person(e, "POST", "/api/capabilities", json!({"kwant_id": e.bot.id, "task_class": class, "tier": tier})).await;
}

async fn flight_school(e: &Env, on: bool) {
    person(e, "POST", "/api/settings", json!({"flight_school": on})).await;
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

/// A live turn of the bot in #general with its own tools, working in `folder`; returns (turn id, token).
async fn turn(e: &Env, folder: &Path) -> (String, String) {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() }),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    let (_, token) =
        mcp_runtime::prepare_turn_with_opts(&e.state, &e.bot, Some(turn_id.clone()), Some(folder.to_path_buf()), Some(scope), Default::default())
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
    (turn_id, token)
}

async fn end(e: &Env, turn_id: &str, token: &str) {
    mcp_runtime::finish_turn(&e.state, token).await;
    e.state.turns.end_turn(turn_id);
}

async fn self_tool(e: &Env, token: &str, name: &str, args: Value) -> (bool, String) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (v["result"]["isError"].as_bool().unwrap_or(false), v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
}

/// A person's mission with one task the bot owns, of `class`, claimed by the bot (with a
/// `before_run` checkpoint when `checkpoint`). Returns (task id, the task's folder).
async fn claimed_task(e: &Env, class: &str, checkpoint: bool) -> (String, PathBuf) {
    person(e, "POST", "/api/settings", json!({"checkpoints": checkpoint})).await;
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": e.bot.id, "budget_tokens": 900_000,
                      "channel_id": e.general, "tasks": [{"title": "A", "owner": e.bot.id, "task_class": class}]});
    let m = person(e, "POST", "/api/missions", body).await;
    let detail = person(e, "GET", &format!("/api/missions/{}", id(&m)), json!(null)).await;
    let tid = id(&detail["tasks"][0]);
    let mission = e.state.db.get_mission(&id(&m)).await.unwrap().unwrap();
    let dir = PathBuf::from(mission.workspace_path);
    std::fs::create_dir_all(&dir).unwrap();
    let (turn_id, token) = turn(e, &dir).await;
    let (err, text) = self_tool(e, &token, "task_claim", json!({"task_id": tid})).await;
    assert!(!err, "{text}");
    end(e, &turn_id, &token).await;
    assert_eq!(e.state.db.run_has_checkpoint(&tid).await.unwrap(), checkpoint);
    (tid, dir)
}

/// What claude's permission prompt gets for `tool {input}` in a turn working in `folder`:
/// `Ok(behavior)` when kwnt answers without a card, `Err(card details)` when it shows one (denied).
async fn claude_asks(e: &Env, folder: &Path, tool: &str, input: Value) -> Result<String, Value> {
    let (turn_id, token) = turn(e, folder).await;
    let out = ask_in(e, &token, tool, input).await;
    end(e, &turn_id, &token).await;
    out
}

async fn ask_in(e: &Env, token: &str, tool: &str, input: Value) -> Result<String, Value> {
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
    match (answered, card) {
        (Ok(done), None) => Ok(behavior(&done.unwrap().1)),
        (Err(_), Some((id, details))) => {
            assert!(e.state.broker.resolve(&id, ApprovalDecision::Deny).await);
            let (_, v) = prompt.await.unwrap();
            assert_eq!(behavior(&v), "deny");
            Err(details)
        }
        (answered, card) => panic!("answered {} with card {card:?}", answered.is_ok()),
    }
}

fn approver(e: &Env) -> mcp_runtime::BrokerApprover {
    mcp_runtime::BrokerApprover {
        db: e.state.db.clone(),
        data_dir: e.state.config.data_dir.clone(),
        broker: e.state.broker.clone(),
        events: e.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: e.general.clone(),
        chat: None,
        always_ask: false,
    }
}

fn request(e: &Env, server: &str, tool: &str, arguments: Value) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        turn_id: "t-fs".into(),
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

fn write_in(dir: &Path) -> Value {
    json!({"file_path": dir.join("notes.md").display().to_string(), "content": "hi"})
}

fn reason(details: &Value) -> &str {
    details["tier_reason"].as_str().unwrap_or("")
}

/// The calls the tier tests make, and what claude's permission prompt answered for each.
async fn outcomes(e: &Env, dir: &Path) -> Vec<Result<String, Value>> {
    let mut out = Vec::new();
    for (tool, input) in [
        ("Write", write_in(dir)),
        ("Read", json!({"file_path": dir.join("notes.md").display().to_string()})),
        ("Bash", json!({"command": "touch x"})),
        ("Write", json!({"file_path": "/tmp/flight-school-outside.txt", "content": "x"})),
    ] {
        out.push(claude_asks(e, dir, tool, input).await);
    }
    out
}

#[tokio::test]
async fn flight_school_off_gates_nothing_even_with_a_lower_tier_saved() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    let (_tid, dir) = claimed_task(&e, "general", false).await;
    let before = outcomes(&e, &dir).await;
    // The person saved observe for every class, but Flight School is off
    set_tier(&e, "*", "observe").await;
    let after = outcomes(&e, &dir).await;
    assert_eq!(before, after);
    assert!(after.iter().all(|o| o == &Ok("allow".to_string())), "{after:?}");
    // Kwant's own tools and the API drivers' shell too
    let a = SelfToolsApprover { inner: Arc::new(approver(&e)), always_ask: false, browser: None };
    assert_eq!(a.decide(request(&e, SELF_SERVER_NAME, "bot_update", json!({}))).await, ApprovalOutcome::Allowed);
    assert!(kwnt::server::computer::approve_local_shell(&e.state, &e.bot.id, "touch x", &dir).await);
    // The task says nothing about tiers while it is off
    let t = person(&e, "GET", &format!("/api/tasks/{_tid}"), json!(null)).await;
    assert_eq!(t["task"]["owner_tier"], Value::Null);
    assert_eq!(t["task"]["task_class"], "general");
    // Under ask, cards are today's: no tier_reason
    set_approvals(&e, "ask").await;
    let Err(details) = claude_asks(&e, &dir, "Bash", json!({"command": "touch x"})).await else { panic!("ask mode cards Bash") };
    assert!(details.get("tier_reason").is_none(), "{details}");
}

#[tokio::test]
async fn sandbox_cards_every_command_and_outside_write_and_needs_a_checkpoint_for_folder_writes() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    flight_school(&e, true).await;
    set_tier(&e, "release", "sandbox").await;

    // Without a before_run checkpoint, even a folder write asks
    let (tid, dir) = claimed_task(&e, "release", false).await;
    let Err(d) = claude_asks(&e, &dir, "Write", write_in(&dir)).await else { panic!("no checkpoint: a write asks") };
    assert!(reason(&d).contains("no checkpoint"), "{d}");
    person(&e, "POST", &format!("/api/tasks/{tid}/cancel"), json!({})).await;

    // With one, folder writes and edits run; every command and every outside write asks
    let (_tid, dir) = claimed_task(&e, "release", true).await;
    assert_eq!(claude_asks(&e, &dir, "Write", write_in(&dir)).await, Ok("allow".into()));
    std::fs::write(dir.join("notes.md"), "hi").unwrap();
    let edit = json!({"file_path": dir.join("notes.md").display().to_string(), "old_string": "hi", "new_string": "ho"});
    assert_eq!(claude_asks(&e, &dir, "Edit", edit).await, Ok("allow".into()));
    assert_eq!(claude_asks(&e, &dir, "Read", json!({"file_path": dir.join("notes.md").display().to_string()})).await, Ok("allow".into()));
    for command in ["touch x", "ls", "echo hi"] {
        let Err(d) = claude_asks(&e, &dir, "Bash", json!({"command": command})).await else { panic!("sandbox: {command} must ask") };
        assert_eq!(reason(&d), "Sandbox: commands always ask");
    }
    let Err(d) = claude_asks(&e, &dir, "Write", json!({"file_path": "/tmp/fs-outside.txt", "content": "x"})).await else { panic!("outside write") };
    assert!(reason(&d).starts_with("Sandbox"), "{d}");
    // Kwant's own tools that change things ask; memory and task tools don't
    let a = SelfToolsApprover { inner: Arc::new(approver(&e)), always_ask: false, browser: None };
    assert_eq!(a.decide(request(&e, SELF_SERVER_NAME, "bot_update", json!({}))).await, ApprovalOutcome::Unanswered);
    assert_eq!(a.decide(request(&e, SELF_SERVER_NAME, "memory_add", json!({"text": "x"}))).await, ApprovalOutcome::Allowed);
    assert_eq!(a.decide(request(&e, SELF_SERVER_NAME, "task_update", json!({}))).await, ApprovalOutcome::Allowed);
    // The API drivers' shell asks too
    let waited = tokio::time::timeout(Duration::from_millis(300), kwnt::server::computer::approve_local_shell(&e.state, &e.bot.id, "touch x", &dir)).await;
    assert!(waited.is_err(), "a sandboxed command waits for the person");
    // Every call above is in the ledger
    let calls = e.state.db.tool_calls_of(&_tid).await.unwrap();
    assert!(calls.iter().filter(|c| c.tool_name == "Bash").count() >= 3, "{calls:?}");
}

#[tokio::test]
async fn approvals_off_and_smart_allow_still_card_in_sandbox_but_always_allow_counts() {
    let e = setup().await;
    flight_school(&e, true).await;
    set_tier(&e, "*", "sandbox").await;
    let (_tid, _dir) = claimed_task(&e, "general", true).await;
    let a = approver(&e);
    for mode in ["off", "smart"] {
        set_approvals(&e, mode).await;
        let mut req = bash(&e, "ls");
        req.smart = Some(SmartVerdict::Allow("only reads".into()));
        assert_eq!(a.decide(req).await, ApprovalOutcome::Unanswered, "{mode}");
        assert_eq!(a.decide(request(&e, "github", "create_issue", json!({}))).await, ApprovalOutcome::Unanswered, "{mode}");
    }
    // D9: the person's own "always allow" for that tool in that chat still counts
    set_approvals(&e, "off").await;
    e.state.broker.allow_for_session(&e.bot.id, &e.general, "Bash").await;
    assert_eq!(a.decide(bash(&e, "ls")).await, ApprovalOutcome::Allowed);
    // …but never for a payment
    assert_eq!(a.decide(bash(&e, STRIPE_POST)).await, ApprovalOutcome::Unanswered);
    // An always-ask turn still cards
    let strict = mcp_runtime::BrokerApprover { always_ask: true, ..approver(&e) };
    assert_eq!(strict.decide(request(&e, "github", "create_issue", json!({}))).await, ApprovalOutcome::Unanswered);
}

#[tokio::test]
async fn observe_cards_an_in_folder_write_that_ask_mode_lets_through() {
    let e = setup().await;
    set_approvals(&e, "ask").await;
    let (_tid, dir) = claimed_task(&e, "research", true).await;
    // Today, in ask mode, a write in the folder runs without a card
    assert_eq!(claude_asks(&e, &dir, "Write", write_in(&dir)).await, Ok("allow".into()));
    flight_school(&e, true).await;
    set_tier(&e, "research", "observe").await;
    let Err(d) = claude_asks(&e, &dir, "Write", write_in(&dir)).await else { panic!("observe: a folder write asks") };
    assert_eq!(reason(&d), "Observe: every change asks");
    // Reading still doesn't
    assert_eq!(claude_asks(&e, &dir, "Read", json!({"file_path": dir.join("notes.md").display().to_string()})).await, Ok("allow".into()));
    let t = person(&e, "GET", &format!("/api/tasks/{_tid}"), json!(null)).await;
    assert_eq!((t["task"]["task_class"].as_str(), t["task"]["owner_tier"].as_str()), (Some("research"), Some("observe")));
}

#[tokio::test]
async fn act_is_todays_path_and_a_payment_still_cards_at_act() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    let (_tid, dir) = claimed_task(&e, "general", true).await;
    let today = outcomes(&e, &dir).await;
    flight_school(&e, true).await;
    let no_row = outcomes(&e, &dir).await;
    set_tier(&e, "general", "act").await;
    let act = outcomes(&e, &dir).await;
    assert_eq!(today, no_row, "no policy row is act");
    assert_eq!(today, act, "act is today's path");
    // The payment floor sits first in the gate: it cards at act too
    let Err(d) = claude_asks(&e, &dir, "Bash", json!({"command": STRIPE_POST})).await else { panic!("a payment asks at act") };
    assert!(d["payment_reason"].as_str().unwrap().contains("stripe.com"), "{d}");
    assert!(d.get("tier_reason").is_none(), "{d}");
}

#[tokio::test]
async fn revoke_mid_turn_gates_the_very_next_call() {
    let e = setup().await;
    set_approvals(&e, "off").await;
    flight_school(&e, true).await;
    let (_tid, dir) = claimed_task(&e, "general", true).await;
    let (turn_id, token) = turn(&e, &dir).await;
    assert_eq!(ask_in(&e, &token, "Bash", json!({"command": "touch a"})).await, Ok("allow".into()));
    let r = person(&e, "POST", &format!("/api/capabilities/{}/revoke", e.bot.id), json!({"task_class": "*", "to_tier": "sandbox"})).await;
    assert_eq!(r["unchanged"], false, "{r}");
    // Same turn, next call
    let Err(d) = ask_in(&e, &token, "Bash", json!({"command": "touch b"})).await else { panic!("revoked: the next call asks") };
    assert_eq!(reason(&d), "Sandbox: commands always ask");
    // Revoking again (or to a higher tier) changes nothing
    let again = person(&e, "POST", &format!("/api/capabilities/{}/revoke", e.bot.id), json!({"task_class": "*", "to_tier": "sandbox"})).await;
    assert_eq!(again["unchanged"], true);
    end(&e, &turn_id, &token).await;
    let caps = person(&e, "GET", &format!("/api/capabilities?kwant_id={}", e.bot.id), json!(null)).await;
    assert_eq!(caps["default_tier"], "sandbox", "{caps}");
    assert_eq!(caps["events"][0]["kind"], "revoked", "{caps}");
}

#[tokio::test]
async fn only_the_person_changes_tiers_and_task_classes() {
    let e = setup().await;
    let (tid, dir) = claimed_task(&e, "general", true).await;
    let (turn_id, token) = turn(&e, &dir).await;
    let body = json!({"kwant_id": e.bot.id, "task_class": "*", "tier": "act"});
    for (method, uri, body) in [
        ("POST", "/api/capabilities".to_string(), body.clone()),
        ("POST", format!("/api/capabilities/{}/revoke", e.bot.id), json!({"task_class": "*", "to_tier": "observe"})),
        ("PATCH", format!("/api/tasks/{tid}"), json!({"task_class": "anything"})),
    ] {
        let (s, v) = call(&e.app, method, &uri, Some(&token), Some(body)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    let (s, _) = call(&e.app, "GET", &format!("/api/capabilities?kwant_id={}", e.bot.id), Some(&token), None).await;
    assert_eq!(s, StatusCode::OK, "reads stay open");
    // tier_status reads; it can't write
    let (err, text) = self_tool(&e, &token, "tier_status", json!({})).await;
    assert!(!err, "{text}");
    assert!(text.contains("\"act\""), "{text}");
    // A bot's plan can't pick the class of a task it would run itself
    let plan = |owner: Option<&str>| {
        let mut t = json!({"title": "B", "acceptance": "done", "task_class": "anything"});
        if let Some(o) = owner {
            t["owner"] = json!(o);
        }
        json!({"title": "Plan", "goal": "g", "tasks": [t]})
    };
    let (err, text) = self_tool(&e, &token, "mission_plan", plan(None)).await;
    assert!(err && text.contains("task_class"), "{text}");
    let (err, text) = self_tool(&e, &token, "mission_plan", plan(Some(&e.bot.id))).await;
    assert!(err && text.contains("task_class"), "{text}");
    let other = e.state.db.list_kwants(&SpaceId::default_space()).await.unwrap().into_iter().find(|k| k.id != e.bot.id).unwrap();
    let (err, text) = self_tool(&e, &token, "mission_plan", plan(Some(&other.id))).await;
    assert!(!err, "a lead sets a teammate's class: {text}");
    end(&e, &turn_id, &token).await;
    // The person can; a bad slug is refused
    person(&e, "PATCH", &format!("/api/tasks/{tid}"), json!({"task_class": "release"})).await;
    assert_eq!(e.state.db.task_class(&tid).await.unwrap(), "release");
    // A redo keeps the class (and so the tier)
    let fork = person(&e, "POST", &format!("/api/tasks/{tid}/fork"), json!({"reason": "redo"})).await;
    let fork_id = fork["task"]["id"].as_str().or(fork["id"].as_str()).unwrap().to_string();
    assert_eq!(e.state.db.task_class(&fork_id).await.unwrap(), "release", "{fork}");
    let (s, _) = call(&e.app, "PATCH", &format!("/api/tasks/{tid}"), Some(e.state.api_token.reveal()), Some(json!({"task_class": "Not OK"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&e.app, "POST", "/api/capabilities", Some(e.state.api_token.reveal()), Some(json!({"kwant_id": e.bot.id, "task_class": "*", "tier": "root"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_codex_bot_below_act_blocks_its_task_instead_of_running_it() {
    let e = setup().await;
    flight_school(&e, true).await;
    let bot = json!({"name": "Cody", "title": "Codex bot", "persona_prompt": "p", "avatar_sprite": "s",
                     "driver_type": "cli", "preferred_model": "codex"});
    let cody = id(&person(&e, "POST", "/api/kwants", bot).await);
    // Hired while Flight School is on: starts at sandbox
    let caps = person(&e, "GET", &format!("/api/capabilities?kwant_id={cody}"), json!(null)).await;
    assert_eq!(caps["default_tier"], "sandbox", "{caps}");
    let body = json!({"title": "Ship it", "goal": "g", "lead_kwant_id": cody, "budget_tokens": 900_000,
                      "channel_id": e.general, "tasks": [{"title": "A", "owner": cody}]});
    let m = person(&e, "POST", "/api/missions", body).await;
    let r = person(&e, "POST", &format!("/api/missions/{}/run", id(&m)), json!({"max_tasks": 1})).await;
    assert_eq!(r["started"].as_array().map_or(0, Vec::len), 0, "{r}");
    let detail = person(&e, "GET", &format!("/api/missions/{}", id(&m)), json!(null)).await;
    let t = &detail["tasks"][0];
    assert_eq!(t["state"], "blocked", "{t}");
    assert_eq!(t["needs_you"][0]["kind"], "engine_cant_sandbox", "{t}");
    // Granting act lets it run again (the person unblocks it)
    person(&e, "POST", "/api/capabilities", json!({"kwant_id": cody, "task_class": "*", "tier": "act"})).await;
    let caps = person(&e, "GET", &format!("/api/capabilities?kwant_id={cody}"), json!(null)).await;
    assert_eq!(caps["default_tier"], "act", "{caps}");
}
