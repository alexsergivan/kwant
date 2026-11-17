//! 26f: on a Home Assistant MCP server (it answers `initialize` as `home-assistant`), a call that
//! acts on a lock, a door, a garage door, a gate or an alarm always shows the card: with an
//! "Always allow" for the tool, and with approvals off. Its "Always allow" and "Allow once" cover
//! that call only. The lights still run on their grant, a server that isn't Home Assistant's is
//! unaffected, and Home Assistant's read of the house (GetLiveContext) still runs.

mod common;

use common::*;
use kwnt::{
    broker::ApprovalDecision,
    db::{models::McpServer, spaces::SpaceId},
    drivers::events::RuntimeEvent,
    mcp::{home_floor, ApprovalOutcome, McpClient, ToolApprovalRequest, ToolApprover},
    server::mcp_runtime::BrokerApprover,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const OFF: &str = "mcp__house__intent__HassTurnOff";

fn approver(env: &Env, conversation: &str, timeout: Duration) -> BrokerApprover {
    BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout,
        conversation: conversation.to_string(),
        chat: None,
        always_ask: false,
        strict_ask: false,
        started_by: None,
    }
}

fn mcp_call(env: &Env, tool_name: &str, arguments: Value) -> ToolApprovalRequest {
    let (server, tool) = tool_name.strip_prefix("mcp__").and_then(|r| r.split_once("__")).unwrap();
    ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-home".into(),
        tool_name: tool_name.into(),
        server: server.into(),
        tool: tool.into(),
        arguments,
        builtin: None,
        smart: None,
        payment: None,
    }
}

/// Registers the echo server with the Home Assistant-shaped tools as bot A's `house`, saying it
/// is `server_name`; `connect` has kwnt connect to it once, as a turn would.
async fn house(env: &Env, server_name: &str, connect: bool) -> McpServer {
    let server = McpServer {
        id: String::new(),
        name: "house".into(),
        transport: "stdio".into(),
        command: Some(env!("CARGO_BIN_EXE_kwnt").into()),
        args: vec!["mcp-echo-server".into(), "--home-tools".into(), "--server-name".into(), server_name.into()],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    };
    let server = env.state.db.create_mcp_server(&SpaceId::default_space(), &server).await.unwrap();
    env.state.db.set_kwant_mcp_servers(&env.a, std::slice::from_ref(&server.id)).await.unwrap();
    if connect {
        let mut client = McpClient::connect(&server, None).await.unwrap();
        let tools = client.list_tools().await.unwrap();
        assert!(tools.iter().any(|t| t.name == "intent__HassTurnOff"));
        client.shutdown().await;
    }
    server
}

/// Makes the call, waits for its card and answers it with `decision`: the outcome and the card.
async fn answer_card(env: &Env, conversation: &str, req: ToolApprovalRequest, decision: ApprovalDecision) -> (ApprovalOutcome, Value) {
    let mut events = env.state.event_sender.subscribe();
    let a = Arc::new(approver(env, conversation, Duration::from_secs(10)));
    let pending = tokio::spawn(async move { a.decide(req).await });
    let (id, details) = loop {
        if let RuntimeEvent::ApprovalRequired { id, details, .. } = events.recv().await.unwrap() {
            break (id, details);
        }
    };
    assert!(env.state.broker.resolve(&id, decision).await);
    (pending.await.unwrap(), details)
}

/// Asks without anyone answering: `false` means it ran without a card.
async fn asks(env: &Env, conversation: &str, req: ToolApprovalRequest) -> bool {
    match approver(env, conversation, Duration::from_millis(300)).decide(req).await {
        ApprovalOutcome::Allowed => false,
        ApprovalOutcome::Unanswered => true,
        other => panic!("unexpected outcome {other:?}"),
    }
}

async fn stored_grants(env: &Env) -> i64 {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", env.state.config.data_dir.join("kwnt.db").display())).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM always_allowed").fetch_one(&pool).await.unwrap();
    pool.close().await;
    n
}

fn the_locks(env: &Env) -> Vec<ToolApprovalRequest> {
    vec![
        mcp_call(env, OFF, json!({"domain": ["lock"]})),
        mcp_call(env, OFF, json!({"name": "Front door"})),
        mcp_call(env, OFF, json!({"name": "Garage"})),
        mcp_call(env, "mcp__house__homeassistant__schedule_command", json!({"command": "unlock the back door"})),
    ]
}

#[tokio::test]
async fn the_lights_run_on_their_grant_and_locks_doors_and_alarms_always_ask() {
    let env = setup().await;
    house(&env, "home-assistant", true).await;
    env.state.db.set_kwant_approvals(&env.a, "ask").await.unwrap();
    let a = env.a.clone();
    env.state.broker.allow_for_session(&a, &a, OFF).await;
    env.state.broker.allow_for_session(&a, &a, "mcp__house__homeassistant__schedule_command").await;

    assert!(!asks(&env, &a, mcp_call(&env, OFF, json!({"domain": ["light"]}))).await, "the lights run on the grant");
    for req in the_locks(&env) {
        assert!(asks(&env, &a, req.clone()).await, "with the grant: {}", req.arguments);
    }
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    assert!(!asks(&env, &a, mcp_call(&env, OFF, json!({"domain": ["light"]}))).await);
    for req in the_locks(&env) {
        assert!(asks(&env, &a, req.clone()).await, "with approvals off: {}", req.arguments);
    }
    env.state.db.set_kwant_approvals(&env.a, "smart").await.unwrap();
    for req in the_locks(&env) {
        assert!(asks(&env, &a, req.clone()).await, "asking only when risky: {}", req.arguments);
    }
}

#[tokio::test]
async fn the_card_says_why_and_neither_answer_leaves_a_grant() {
    let env = setup().await;
    house(&env, "home-assistant", true).await;
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    let a = env.a.clone();
    let front_door = || mcp_call(&env, OFF, json!({"name": "Front door"}));

    let (outcome, card) = answer_card(&env, &a, front_door(), ApprovalDecision::AlwaysAllowSession).await;
    assert_eq!(outcome, ApprovalOutcome::Allowed, "that one call runs");
    assert_eq!(card["home_always_ask"], json!(true), "{card}");
    assert_eq!(card["home_reason"], json!(home_floor::REASON), "{card}");
    assert!(card.get("always_ask").is_none_or(|v| v != &json!(true)), "its own field: {card}");
    assert!(!env.state.broker.is_allowed_for_session(&a, &a, OFF).await, "always allow made no grant");
    assert_eq!(stored_grants(&env).await, 0);
    assert!(asks(&env, &a, front_door()).await, "the next call asks again");

    let (outcome, _) = answer_card(&env, &a, front_door(), ApprovalDecision::AllowOnce).await;
    assert_eq!(outcome, ApprovalOutcome::Allowed);
    assert!(!env.state.broker.is_allowed_for_session(&a, &a, OFF).await, "allow once made no grant");
    assert_eq!(stored_grants(&env).await, 0);
    assert!(asks(&env, &a, front_door()).await);

    let (outcome, _) = answer_card(&env, &a, front_door(), ApprovalDecision::Deny).await;
    assert_eq!(outcome, ApprovalOutcome::Denied);
}

#[tokio::test]
async fn a_server_that_isnt_home_assistant_is_unaffected() {
    let env = setup().await;
    house(&env, "kwnt-echo", true).await;
    env.state.db.set_kwant_approvals(&env.a, "ask").await.unwrap();
    let a = env.a.clone();
    env.state.broker.allow_for_session(&a, &a, OFF).await;
    env.state.broker.allow_for_session(&a, &a, "mcp__house__homeassistant__schedule_command").await;
    for req in the_locks(&env) {
        assert!(!asks(&env, &a, req.clone()).await, "same tool, other server, the grant covers it: {}", req.arguments);
    }
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    for req in the_locks(&env) {
        assert!(!asks(&env, &a, req.clone()).await, "approvals off: {}", req.arguments);
    }
}

#[tokio::test]
async fn get_live_context_still_runs() {
    let env = setup().await;
    house(&env, "home-assistant", true).await;
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    let a = env.a.clone();
    let read = |args: Value| mcp_call(&env, "mcp__house__homeassistant__GetLiveContext", args);
    assert!(!asks(&env, &a, read(json!({}))).await);
    assert!(!asks(&env, &a, read(json!({"name": "Front door"}))).await, "a read, whatever it names");
}

#[tokio::test]
async fn what_the_server_said_at_its_test_is_kept_and_an_unknown_server_asks() {
    let env = setup().await;
    let server = house(&env, "home-assistant", false).await;
    env.state.db.set_kwant_approvals(&env.a, "off").await.unwrap();
    let a = env.a.clone();
    let lock = || mcp_call(&env, OFF, json!({"domain": ["lock"]}));
    let light = || mcp_call(&env, OFF, json!({"domain": ["light"]}));

    // Never heard from: a lock asks (the safe side); the lights don't
    assert_eq!(env.state.db.mcp_reported_name(&server.id).await.unwrap(), None);
    assert!(asks(&env, &a, lock()).await);
    assert!(!asks(&env, &a, light()).await);

    // Its test stores what it said, and that holds after kwnt forgets this run's connections
    let (s, v) = call(&env.app, "POST", &format!("/api/mcp/servers/{}/test", server.id), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(env.state.db.mcp_reported_name(&server.id).await.unwrap(), Some(Some("home-assistant".into())));
    home_floor::forget(&server.id);
    assert!(asks(&env, &a, lock()).await, "stored: still Home Assistant's");

    // A server stored as something else is unaffected
    env.state.db.save_mcp_reported_name(&server.id, Some("kwnt-echo")).await.unwrap();
    assert!(!asks(&env, &a, lock()).await);

    // A new address forgets what the old one said
    let (s, v) = call(
        &env.app,
        "PUT",
        &format!("/api/mcp/servers/{}", server.id),
        Some(json!({"name": "house", "transport": "stdio", "command": env!("CARGO_BIN_EXE_kwnt"), "args": ["mcp-echo-server"], "enabled": true})),
    )
    .await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.mcp_reported_name(&server.id).await.unwrap(), None);
}
