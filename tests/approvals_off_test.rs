//! The person's approvals switch: `ask` shows an approval card before a bot's tool
//! runs; `off` runs it and records it as `auto_allowed`. Set per space, overridden per bot, and
//! never changed by a bot.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        Database,
    },
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use kwnt::mcp::{ApprovalOutcome, ToolApprovalRequest, ToolApprover};
use serde_json::{json, Value};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    other: Kwant,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    // Start from an asking install (a fresh one asks only when risky)
    db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "ask").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let space = SpaceId::default_space();
    let kwants = state.db.list_kwants(&space).await.unwrap();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    Env { app: create_router(state.clone()), bot: kwants[0].clone(), other: kwants[1].clone(), general, state, _tmp: tmp }
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

async fn get(e: &Env, uri: &str) -> Value {
    let (s, v) = call(&e.app, "GET", uri, None, None).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

async fn put(e: &Env, uri: &str, approvals: &str) -> Value {
    let (s, v) = call(&e.app, "PUT", uri, Some(e.state.api_token.reveal()), Some(json!({"approvals": approvals}))).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

/// A live turn of `bot` in the general channel with its own tools; returns its token.
async fn turn(e: &Env, bot: &Kwant) -> (String, String, TempDir) {
    turn_with(e, bot, Default::default()).await
}

/// A turn kwant started on its own (`always_ask`), labelled like loose-ends' owner's.
fn kwant_started() -> mcp_runtime::TurnApprovalOpts {
    mcp_runtime::TurnApprovalOpts { always_ask: true, started_by: Some("Loose ends".into()), ..Default::default() }
}

/// [`kwant_started`], strict like Skill Foundry's: it still asks with approvals off.
fn strict_started() -> mcp_runtime::TurnApprovalOpts {
    mcp_runtime::TurnApprovalOpts { always_ask: true, strict_ask: true, started_by: Some("Skill Foundry".into()), ..Default::default() }
}

/// [`turn`] with the routed turn's approval options (`always_ask`: a turn like the loose-ends
/// owner's; `strict_ask`: Skill Foundry's).
async fn turn_with(e: &Env, bot: &Kwant, opts: mcp_runtime::TurnApprovalOpts) -> (String, String, TempDir) {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let folder = tempdir().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() }),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    let (_, token) =
        mcp_runtime::prepare_turn_with_opts(&e.state, bot, Some(turn_id.clone()), Some(folder.path().to_path_buf()), Some(scope), opts)
            .await
            .unwrap();
    e.state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.clone(),
        kwant_id: bot.id.clone(),
        context_id: e.general.clone(),
        thread_id: None,
        state: "running".into(),
        since: Utc::now(),
        purpose: None,
    });
    (turn_id, token, folder)
}

/// What claude's permission prompt gets back for `Bash {command}`: `Some(allowed)` when kwnt
/// answers without a card, `None` when it shows one (the card is then denied, to end the wait).
async fn ask_bash(e: &Env, bot: &Kwant, command: &str) -> Option<bool> {
    ask_with(e, bot, "Bash", json!({"command": command}), Default::default()).await.ok()
}

/// [`ask_bash`] for any tool, in a turn with `opts`: `Ok(allowed)` when kwnt answers without a card,
/// `Err(details)` with the card's details when it shows one (then denied, to end the wait).
async fn ask_with(e: &Env, bot: &Kwant, tool: &str, input: Value, opts: mcp_runtime::TurnApprovalOpts) -> Result<bool, Value> {
    let (turn_id, token, _folder) = turn_with(e, bot, opts).await;
    let mut events = e.state.event_sender.subscribe();
    let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
        "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_1"}}});
    let app = e.app.clone();
    let path = format!("/api/mcp/permission/{token}");
    let mut prompt = tokio::spawn(async move { call(&app, "POST", &path, None, Some(ask)).await });

    // Polled, not one fixed wait: under load the answer or the card can take seconds to come
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut carded = card_in(&mut events);
    while carded.is_none() && !prompt.is_finished() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        carded = card_in(&mut events);
    }
    // An answer without a card must not have raised one on its way either
    let answered = prompt.is_finished();
    let carded = carded.or_else(|| card_in(&mut events));
    let outcome = match (answered, carded) {
        (true, None) => {
            let (status, v) = (&mut prompt).await.unwrap();
            assert_eq!(status, StatusCode::OK, "{v}");
            Ok(v["result"]["content"][0]["text"].as_str().unwrap().contains(r#""behavior":"allow""#))
        }
        (false, Some((id, details))) => {
            assert!(e.state.broker.resolve(&id, ApprovalDecision::Deny).await);
            prompt.await.unwrap();
            Err(details)
        }
        (answered, carded) => panic!("answered {answered} with card {carded:?} (15 s)"),
    };
    mcp_runtime::finish_turn(&e.state, &token).await;
    e.state.turns.end_turn(&turn_id);
    outcome
}

/// The id and details of an approval card among the events received so far, if one was shown.
fn card_in(events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> Option<(String, Value)> {
    use tokio::sync::broadcast::error::TryRecvError;
    loop {
        match events.try_recv() {
            Ok(RuntimeEvent::ApprovalRequired { id, details, .. }) => return Some((id, details)),
            Ok(_) | Err(TryRecvError::Lagged(_)) => continue,
            Err(_) => return None,
        }
    }
}

async fn self_tool(e: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    (
        v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(),
        v["result"]["isError"].as_bool().unwrap_or(false),
    )
}

fn bot_in<'a>(list: &'a Value, id: &str) -> &'a Value {
    list.as_array().unwrap().iter().find(|b| b["id"] == id || b["kwant_id"] == id).expect("bot listed")
}

#[tokio::test]
async fn bots_follow_an_asking_space_by_default() {
    let e = setup().await;
    let space = get(&e, "/api/spaces/spc_default/approvals").await;
    assert_eq!(space["approvals"], "ask");
    assert_eq!(bot_in(&space["bots"], &e.bot.id)["approvals"], "inherit");
    assert_eq!(bot_in(&space["bots"], &e.bot.id)["approvals_effective"], "ask");
    assert_eq!(get(&e, "/api/spaces/spc_default").await["approvals"], "ask");
    let kwants = get(&e, "/api/kwants?space=spc_default").await;
    assert_eq!(bot_in(&kwants, &e.bot.id)["approvals"], "inherit");
    assert_eq!(bot_in(&kwants, &e.bot.id)["approvals_effective"], "ask");

    assert_eq!(ask_bash(&e, &e.bot, "touch x").await, None, "Bash asks by default");
    assert!(get(&e, "/api/approvals?status=auto_allowed").await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn space_off_runs_bash_without_a_card_and_records_it() {
    let e = setup().await;
    let v = put(&e, "/api/spaces/spc_default/approvals", "off").await;
    assert_eq!(v["approvals"], "off");
    assert_eq!(bot_in(&v["bots"], &e.bot.id)["approvals_effective"], "off");
    assert_eq!(get(&e, "/api/spaces").await[0]["approvals"], "off");
    assert_eq!(get(&e, &format!("/api/kwants/{}/approvals", e.bot.id)).await["approvals_effective"], "off");

    assert_eq!(ask_bash(&e, &e.bot, "touch x").await, Some(true));
    // A write outside the folder and an unknown tool run too: there is no floor
    let (turn_id, token, _f) = turn(&e, &e.bot).await;
    for (tool, input) in [("Write", json!({"file_path": "/etc/kwnt-test", "content": "x"})), ("SomeNewTool", json!({}))] {
        let ask = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
            "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_2"}}});
        let (s, v) = tokio::time::timeout(
            Duration::from_secs(3),
            call(&e.app, "POST", &format!("/api/mcp/permission/{token}"), None, Some(ask)),
        )
        .await
        .expect("answered without a card");
        assert_eq!(s, StatusCode::OK);
        assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains(r#""behavior":"allow""#), "{tool}: {v}");
    }
    mcp_runtime::finish_turn(&e.state, &token).await;
    e.state.turns.end_turn(&turn_id);

    // Nothing waits, and every call is on record
    let away = get(&e, "/api/away").await;
    assert!(away["waiting"].as_array().unwrap().iter().all(|i| i["kind"] != "approval"), "{away}");
    let ran = get(&e, "/api/approvals?space=spc_default&status=auto_allowed").await;
    let ran = ran.as_array().unwrap();
    assert_eq!(ran.len(), 3, "{ran:?}");
    let bash = ran.iter().find(|r| r["tool_name"] == "Bash").unwrap();
    assert_eq!(bash["status"], "auto_allowed");
    assert_eq!(bash["kwant_id"], e.bot.id);
    assert_eq!(bash["details"]["arguments"]["command"], "touch x");
    assert_eq!(bash["details"]["source"], "claude_builtin");
    let write = ran.iter().find(|r| r["tool_name"] == "Write").unwrap();
    assert_eq!(write["details"]["outside_folder"], true);

    // Back on: it asks again
    put(&e, "/api/spaces/spc_default/approvals", "ask").await;
    assert_eq!(ask_bash(&e, &e.bot, "touch y").await, None);
}

/// Oleks's report (0.9.5): with approvals off, a bot's loose-ends turn (kwant started it, nobody
/// asked) still asked to run `git show`. Off now wins there too: a read-only command, a change in
/// the folder, a write outside it and kwnt's own changing tools run without a card, on record.
#[tokio::test]
async fn approvals_off_wins_in_a_turn_kwant_started_on_its_own() {
    let e = setup().await;
    put(&e, "/api/spaces/spc_default/approvals", "off").await;
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "off").await;

    let self_tool = |name: &str| format!("mcp__{}__{name}", kwnt::mcp::SELF_SERVER_NAME);
    let oleks = "git show --stat HEAD 2>/dev/null | head -40; grep -rn \"TODO\" . | head -20";
    for (tool, input) in [
        ("Bash".to_string(), json!({"command": oleks})),
        ("Bash".to_string(), json!({"command": "touch x"})),
        ("Write".to_string(), json!({"file_path": "/tmp/kwnt-owner-test", "content": "x"})),
        (self_tool("routine_create"), json!({"name": "r", "instructions": "x", "schedule": {"type": "daily", "time": "09:00"}})),
        (self_tool("delegate"), json!({"to": "Other", "task": "send it"})),
        (self_tool("memory_add"), json!({"text": "x"})),
        (self_tool("search_history"), json!({})),
    ] {
        assert_eq!(ask_with(&e, &e.bot, &tool, input, kwant_started()).await, Ok(true), "{tool} runs with approvals off");
    }
    let ran = get(&e, "/api/approvals?space=spc_default&status=auto_allowed").await;
    let bash = ran.as_array().unwrap().iter().find(|r| r["details"]["arguments"]["command"] == oleks).expect("on record");
    assert_eq!(bash["details"]["always_ask"], true, "{bash}");
    assert_eq!(bash["details"]["started_by"], "Loose ends", "{bash}");
    let away = get(&e, "/api/away").await;
    assert!(away["waiting"].as_array().unwrap().iter().all(|i| i["kind"] != "approval"), "{away}");
}

/// A strict turn (Skill Foundry's draft: it runs steps the person hasn't reviewed) still asks
/// with approvals off, and says who started it.
#[tokio::test]
async fn a_strict_turn_still_asks_with_approvals_off() {
    let e = setup().await;
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "off").await;
    let card = ask_with(&e, &e.bot, "Bash", json!({"command": "touch x"}), strict_started()).await.expect_err("strict asks");
    assert_eq!(card["always_ask"], true, "{card}");
    assert_eq!(card["started_by"], "Skill Foundry", "{card}");
    // The card says "Skill drafts always ask" from this flag, not from the label
    assert_eq!(card["strict_ask"], true, "{card}");
    // Reads still run (claude's own Read never reaches the approver), and so do kwnt's read tools
    let tool = format!("mcp__{}__search_history", kwnt::mcp::SELF_SERVER_NAME);
    assert_eq!(ask_with(&e, &e.bot, &tool, json!({}), strict_started()).await, Ok(true));
}

/// With approvals on, a kwant-started turn still asks, and its card says why: `started_by`.
/// "Always allow" for the conversation doesn't carry into it either.
#[tokio::test]
async fn with_approvals_on_a_kwant_started_turn_asks_and_says_who_started_it() {
    let e = setup().await;
    let card = ask_with(&e, &e.bot, "Bash", json!({"command": "touch x"}), kwant_started()).await.expect_err("asks");
    assert_eq!(card["always_ask"], true, "{card}");
    assert_eq!(card["started_by"], "Loose ends", "{card}");
    // Not strict: the card points to Don’t ask instead
    assert!(card.get("strict_ask").is_none(), "{card}");
    // An ordinary turn's card has none of them
    let card = ask_with(&e, &e.bot, "Bash", json!({"command": "touch x"}), Default::default()).await.expect_err("asks");
    assert!(card.get("always_ask").is_none() && card.get("started_by").is_none() && card.get("strict_ask").is_none(), "{card}");
    // A blank label adds no line
    let blank = mcp_runtime::TurnApprovalOpts { started_by: Some("  ".into()), ..kwant_started() };
    let card = ask_with(&e, &e.bot, "Bash", json!({"command": "touch x"}), blank).await.expect_err("asks");
    assert!(card.get("started_by").is_none(), "{card}");

    e.state.broker.allow_for_session(&e.bot.id, &e.general, "Bash").await;
    for (always_ask, want) in [(false, ApprovalOutcome::Allowed), (true, ApprovalOutcome::Unanswered)] {
        let approver = mcp_runtime::BrokerApprover {
            db: e.state.db.clone(),
            data_dir: e.state.config.data_dir.clone(),
            broker: e.state.broker.clone(),
            events: e.state.event_sender.clone(),
            timeout: Duration::from_millis(200),
            conversation: e.general.clone(),
            chat: None,
            always_ask,
            strict_ask: false,
            started_by: None,
        };
        let req = ToolApprovalRequest {
            kwant_id: e.bot.id.clone(),
            kwant_name: e.bot.name.clone(),
            turn_id: "t-owner".into(),
            tool_name: "Bash".into(),
            server: String::new(),
            tool: "Bash".into(),
            arguments: json!({"command": "touch y"}),
            builtin: None,
            smart: None,
            payment: None,
        };
        assert_eq!(approver.decide(req).await, want, "always_ask {always_ask}");
    }
}

/// "Ask only when risky" in a kwant-started turn: a command that only reads runs; anything that
/// changes something asks. A strict turn asks for both.
#[tokio::test]
async fn smart_lets_a_read_only_command_through_in_a_kwant_started_turn() {
    let e = setup().await;
    e.state.db.set_kwant_approvals(&e.bot.id, "smart").await.unwrap();
    let bash = |c: &str| json!({"command": c});
    assert_eq!(ask_with(&e, &e.bot, "Bash", bash("git show --stat HEAD"), kwant_started()).await, Ok(true));
    assert_eq!(ask_with(&e, &e.bot, "Bash", bash("grep -rn TODO . | head -20"), kwant_started()).await, Ok(true));
    for command in ["rm -rf x", "touch x", "git add -A", "git show HEAD > out.txt"] {
        assert!(ask_with(&e, &e.bot, "Bash", bash(command), kwant_started()).await.is_err(), "{command} asks");
    }
    assert!(ask_with(&e, &e.bot, "Bash", bash("git show --stat HEAD"), strict_started()).await.is_err(), "strict asks");
    // An ordinary turn: smart's own rules, unchanged (an in-folder `touch` is not risky)
    assert_eq!(ask_with(&e, &e.bot, "Bash", bash("touch x"), Default::default()).await, Ok(true));
}

#[tokio::test]
async fn a_bot_override_beats_the_space_both_ways() {
    let e = setup().await;
    put(&e, "/api/spaces/spc_default/approvals", "off").await;
    let v = put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "ask").await;
    assert_eq!(v["approvals"], "ask");
    assert_eq!(v["approvals_effective"], "ask");
    assert_eq!(v["space_approvals"], "off");
    assert_eq!(ask_bash(&e, &e.bot, "touch x").await, None, "the bot's ask beats the space's off");
    assert_eq!(ask_bash(&e, &e.other, "touch x").await, Some(true), "other bots follow the space");

    put(&e, "/api/spaces/spc_default/approvals", "ask").await;
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "off").await;
    assert_eq!(ask_bash(&e, &e.bot, "touch x").await, Some(true), "the bot's off beats the space's ask");
    assert_eq!(ask_bash(&e, &e.other, "touch x").await, None);
    let kwants = get(&e, "/api/kwants").await;
    assert_eq!(bot_in(&kwants, &e.bot.id)["approvals"], "off");
    assert_eq!(bot_in(&kwants, &e.other.id)["approvals_effective"], "ask");

    let v = put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "inherit").await;
    assert_eq!(v["approvals"], "inherit");
    assert_eq!(v["approvals_effective"], "ask");
    assert_eq!(ask_bash(&e, &e.bot, "touch x").await, None);
}

#[tokio::test]
async fn only_the_person_changes_approvals() {
    let e = setup().await;
    let (turn_id, token, _f) = turn(&e, &e.bot).await;

    // The app API refuses a bot's turn token
    for uri in ["/api/spaces/spc_default/approvals".to_string(), format!("/api/kwants/{}/approvals", e.bot.id)] {
        let (s, v) = call(&e.app, "PUT", &uri, Some(&token), Some(json!({"approvals": "off"}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{uri}: {v}");
    }
    // The bot's own tools refuse it
    let (text, err) = self_tool(&e, &token, "settings_update", json!({"settings": {"approvals": "off"}})).await;
    assert!(err, "{text}");
    let (text, err) = self_tool(&e, &token, "bot_update", json!({"bot": e.bot.id, "approvals": "off"})).await;
    assert!(err && text.contains("approvals"), "{text}");
    let (text, err) = self_tool(&e, &token, "bot_create", json!({"requested_by_person": true, "name": "Free", "title": "t", "instructions": "i", "approvals": "off"})).await;
    assert!(err && text.contains("approvals"), "{text}");
    let (text, err) = self_tool(&e, &token, "space_update", json!({"name": "X", "approvals": "off"})).await;
    assert!(err && text.contains("approvals"), "{text}");
    mcp_runtime::finish_turn(&e.state, &token).await;
    e.state.turns.end_turn(&turn_id);

    assert_eq!(get(&e, "/api/spaces/spc_default/approvals").await["approvals"], "ask");
    assert_eq!(get(&e, &format!("/api/kwants/{}/approvals", e.bot.id)).await["approvals"], "inherit");
    assert_eq!(ask_bash(&e, &e.bot, "touch x").await, None);
}

#[tokio::test]
async fn bad_values_unknown_rows_and_other_spaces() {
    let e = setup().await;
    let (s, _) = call(&e.app, "PUT", "/api/spaces/spc_default/approvals", Some(e.state.api_token.reveal()), Some(json!({"approvals": "never"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&e.app, "PUT", "/api/spaces/spc_default/approvals", Some(e.state.api_token.reveal()), Some(json!({"approvals": "inherit"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a space has no inherit");
    let (s, _) = call(&e.app, "PUT", &format!("/api/kwants/{}/approvals", e.bot.id), Some(e.state.api_token.reveal()), Some(json!({"approvals": "maybe"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&e.app, "PUT", "/api/kwants/nope/approvals", Some(e.state.api_token.reveal()), Some(json!({"approvals": "off"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&e.app, "PUT", "/api/spaces/nope/approvals", Some(e.state.api_token.reveal()), Some(json!({"approvals": "off"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Another space has its own switch, and its bots follow it
    let other = e
        .state
        .db
        .create_space(&kwnt::db::spaces::NewSpace { name: "Lab".into(), icon: "🧪".into(), color: None, this_machine_enabled: true })
        .await
        .unwrap();
    let lab = SpaceId::new(&other.id);
    let lab_bot = e
        .state
        .db
        .create_kwant(
            &lab,
            &NewKwant {
                name: "Labby".into(),
                title: "t".into(),
                persona_prompt: "p".into(),
                soul: String::new(),
                avatar_sprite: "architect".into(),
                driver_type: "api".into(),
                preferred_model: "auto".into(),
            },
        )
        .await
        .unwrap();
    put(&e, &format!("/api/spaces/{}/approvals", other.id), "off").await;
    assert_eq!(get(&e, "/api/spaces/spc_default/approvals").await["approvals"], "ask");
    assert_eq!(get(&e, &format!("/api/kwants/{}/approvals", lab_bot.id)).await["approvals_effective"], "off");
    assert_eq!(get(&e, &format!("/api/kwants/{}/approvals", e.bot.id)).await["approvals_effective"], "ask");
    // A bot named with the wrong space is not found there
    let (s, _) = call(&e.app, "PUT", &format!("/api/kwants/{}/approvals?space=spc_default", lab_bot.id), Some(e.state.api_token.reveal()), Some(json!({"approvals": "ask"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // The record list stays in its space
    assert_eq!(ask_bash(&e, &lab_bot, "touch x").await, Some(true));
    assert_eq!(get(&e, &format!("/api/approvals?space={}", other.id)).await.as_array().unwrap().len(), 1);
    assert!(get(&e, "/api/approvals?space=spc_default").await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_local_command_runs_without_a_card_when_off() {
    let e = setup().await;
    let folder = tempdir().unwrap();
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "off").await;
    let ran = tokio::time::timeout(
        Duration::from_secs(3),
        kwnt::server::computer::approve_local_shell(&e.state, &e.bot.id, "touch x", folder.path()),
    )
    .await
    .expect("no card");
    assert!(ran);
    let rows = get(&e, "/api/approvals?status=auto_allowed").await;
    assert_eq!(rows[0]["tool_name"], "execute_shell");
    assert_eq!(rows[0]["details"]["arguments"]["command"], "touch x");

    // Asking still waits for the card
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "inherit").await;
    let waiting = tokio::time::timeout(
        Duration::from_millis(300),
        kwnt::server::computer::approve_local_shell(&e.state, &e.bot.id, "touch y", folder.path()),
    )
    .await;
    assert!(waiting.is_err(), "it should wait for the person");
}
