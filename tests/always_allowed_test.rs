//! 25e: "Always allow" survives a kwnt restart, keeps the scope it was given for (that bot, that
//! conversation, that tool), never covers a payment or a production step, stays out of exports
//! and imports, goes with its bot, thread or channel, and the person can see and take back each
//! one through person-only routes; the very next call asks again.

mod common;

use common::*;
use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    db::{models::NewChannel, spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    mcp::{ApprovalOutcome, ToolApprovalRequest, ToolApprover},
    server::{continuity::SelfScope, mcp_runtime::{self, BrokerApprover}},
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

const STRIPE_POST: &str = "curl -X POST https://api.stripe.com/v1/charges -u sk_test_x: -d amount=100 -d currency=usd";
const TOOL: &str = "mcp__notes__write_note";

fn approver(db: &Database, broker: &PermissionBroker, env: &Env, conversation: &str, timeout: Duration) -> BrokerApprover {
    BrokerApprover {
        db: db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: broker.clone(),
        events: env.state.event_sender.clone(),
        timeout,
        conversation: conversation.to_string(),
        chat: None,
        always_ask: false,
        strict_ask: false,
        started_by: None,
    }
}

fn mcp_call(env: &Env, tool_name: &str) -> ToolApprovalRequest {
    let (server, tool) = tool_name.strip_prefix("mcp__").and_then(|r| r.split_once("__")).unwrap_or(("", tool_name));
    ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-always".into(),
        tool_name: tool_name.into(),
        server: server.into(),
        tool: tool.into(),
        arguments: json!({"text": "hi"}),
        builtin: None,
        smart: None,
        payment: None,
    }
}

fn bash(env: &Env, command: &str) -> ToolApprovalRequest {
    ToolApprovalRequest {
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: json!({"command": command}),
        builtin: Some(kwnt::mcp::BuiltinCall { cwd: None, outside_folder: false }),
        ..mcp_call(env, "Bash")
    }
}

/// Makes the call, waits for its card and answers it with `decision`.
async fn answer_card(env: &Env, conversation: &str, req: ToolApprovalRequest, decision: ApprovalDecision) -> ApprovalOutcome {
    let mut events = env.state.event_sender.subscribe();
    let a = Arc::new(approver(&env.state.db, &env.state.broker, env, conversation, Duration::from_secs(10)));
    let pending = tokio::spawn(async move { a.decide(req).await });
    let id = loop {
        if let RuntimeEvent::ApprovalRequired { id, .. } = events.recv().await.unwrap() {
            break id;
        }
    };
    assert!(env.state.broker.resolve(&id, decision).await);
    pending.await.unwrap()
}

/// Asks without anyone answering: `Allowed` means it ran without a card.
async fn asks(env: &Env, db: &Database, broker: &PermissionBroker, conversation: &str, req: ToolApprovalRequest) -> bool {
    match approver(db, broker, env, conversation, Duration::from_millis(200)).decide(req).await {
        ApprovalOutcome::Allowed => false,
        ApprovalOutcome::Unanswered => true,
        other => panic!("unexpected outcome {other:?}"),
    }
}

async fn stored(env: &Env) -> i64 {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", env.state.config.data_dir.join("kwnt.db").display())).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM always_allowed").fetch_one(&pool).await.unwrap();
    pool.close().await;
    n
}

async fn ask_mode(env: &Env) {
    env.state.db.set_kwant_approvals(&env.a, "ask").await.unwrap();
}

async fn thread(env: &Env, title: &str) -> String {
    env.state.db.create_thread(&env.a, title, "chat", None, "person").await.unwrap().id
}

fn uri(env: &Env) -> String {
    format!("/api/kwants/{}/always-allowed", env.a)
}

#[tokio::test]
async fn a_grant_still_works_after_a_restart_and_another_conversation_still_asks() {
    let env = setup().await;
    ask_mode(&env).await;
    let (t1, t2) = (thread(&env, "Weekly notes").await, thread(&env, "Other work").await);
    assert_eq!(answer_card(&env, &t1, mcp_call(&env, TOOL), ApprovalDecision::AlwaysAllowSession).await, ApprovalOutcome::Allowed);
    assert!(!asks(&env, &env.state.db, &env.state.broker, &t1, mcp_call(&env, TOOL)).await, "same run, same thread: no card");

    // A restart: a new database handle and a new broker on the same file
    let db = Database::init(&env.state.config.data_dir).await.unwrap();
    let broker = PermissionBroker::new();
    broker.attach_store(db.clone());
    assert_eq!(broker.load_saved().await, 1);
    assert!(!asks(&env, &db, &broker, &t1, mcp_call(&env, TOOL)).await, "the grant came back");
    assert!(asks(&env, &db, &broker, &t2, mcp_call(&env, TOOL)).await, "another thread still asks");
    assert!(asks(&env, &db, &broker, &env.a, mcp_call(&env, TOOL)).await, "the direct chat still asks");
    assert!(asks(&env, &db, &broker, &t1, mcp_call(&env, "mcp__notes__delete_note")).await, "another tool still asks");
    let mut other = mcp_call(&env, TOOL);
    other.kwant_id = env.b.clone();
    env.state.db.set_kwant_approvals(&env.b, "ask").await.unwrap();
    assert!(asks(&env, &db, &broker, &t1, other).await, "another bot still asks");

    // The person's list says where it applies
    let (s, v) = call(&env.app, "GET", &uri(&env), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{v}");
    assert_eq!(rows[0]["tool_name"], TOOL);
    assert_eq!(rows[0]["conversation"], t1.as_str());
    assert_eq!(rows[0]["conversation_kind"], "thread");
    assert_eq!(rows[0]["conversation_label"], "Weekly notes");
    assert!(rows[0]["created_at"].is_string(), "{v}");
}

#[tokio::test]
async fn payments_and_production_steps_are_never_stored() {
    let env = setup().await;
    ask_mode(&env).await;
    let t = thread(&env, "Pay").await;
    assert_eq!(answer_card(&env, &t, bash(&env, STRIPE_POST), ApprovalDecision::AlwaysAllowSession).await, ApprovalOutcome::Allowed);
    assert_eq!(stored(&env).await, 0, "a payment's always allow covers that one call");

    let space = env.state.db.get_kwant(&env.a).await.unwrap().unwrap().space_id;
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, Some("pg_dump demo > /tmp/demo.sql")).await.unwrap();
    let step = bash(&env, "sqlite3 demo.db 'DROP TABLE demo'");
    assert_eq!(answer_card(&env, &t, step, ApprovalDecision::AlwaysAllowSession).await, ApprovalOutcome::Allowed);
    assert_eq!(stored(&env).await, 0, "a production step's always allow covers that one call");
    assert!(!env.state.broker.is_allowed_for_session(&env.a, &t, "Bash").await);

    // An ordinary tool is stored
    assert_eq!(answer_card(&env, &t, mcp_call(&env, TOOL), ApprovalDecision::AlwaysAllowSession).await, ApprovalOutcome::Allowed);
    assert_eq!(stored(&env).await, 1);
}

#[tokio::test]
async fn deleting_the_bot_thread_or_channel_removes_its_rows() {
    let env = setup().await;
    let space = SpaceId::new(&env.state.db.get_kwant(&env.a).await.unwrap().unwrap().space_id);
    let t = thread(&env, "Goes away").await;
    let ch = env
        .state
        .db
        .create_channel(&space, &NewChannel { name: "scratch".into(), description: None, lead_kwant_id: env.a.clone(), member_kwants: vec![env.a.clone()], routing_mode: "lead".into() })
        .await
        .unwrap()
        .id;
    let b = &env.state.broker;
    for c in [&t, &ch, &env.a] {
        b.allow_for_session(&env.a, c, TOOL).await;
    }
    b.allow_for_session(&env.b, &env.b, TOOL).await;
    assert_eq!(stored(&env).await, 4);

    let (_, v) = call(&env.app, "GET", &uri(&env), None).await;
    let kinds: Vec<(&str, &str)> = v.as_array().unwrap().iter().map(|r| (r["conversation_kind"].as_str().unwrap(), r["conversation_label"].as_str().unwrap())).collect();
    for want in [("thread", "Goes away"), ("channel", "#scratch"), ("direct", "your chat")] {
        assert!(kinds.contains(&want), "{want:?} in {kinds:?}");
    }

    // Threads only go with their bot; whichever path deletes one, its rows go too
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", env.state.config.data_dir.join("kwnt.db").display())).await.unwrap();
    sqlx::query("DELETE FROM threads WHERE id = ?").bind(&t).execute(&pool).await.unwrap();
    pool.close().await;
    assert_eq!(stored(&env).await, 3);
    assert!(env.state.db.delete_empty_channel(&ch).await.unwrap());
    assert_eq!(stored(&env).await, 2);

    let (s, v) = call(&env.app, "DELETE", &format!("/api/kwants/{}", env.a), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(stored(&env).await, 1, "only the other bot's grant is left");
    assert!(!b.is_allowed_for_session(&env.a, &env.a, TOOL).await);
    assert!(b.is_allowed_for_session(&env.b, &env.b, TOOL).await);
}

#[tokio::test]
async fn taking_back_one_or_all_works_on_the_very_next_call() {
    let env = setup().await;
    ask_mode(&env).await;
    let (t1, t2) = (thread(&env, "One").await, thread(&env, "Two").await);
    let b = &env.state.broker;
    b.allow_for_session(&env.a, &t1, TOOL).await;
    b.allow_for_session(&env.a, &t2, TOOL).await;
    b.allow_for_session(&env.a, &t1, "Bash").await;
    b.allow_for_session(&env.b, &env.b, TOOL).await;

    // Bodies that don't say what to take back change nothing
    for body in [None, Some(json!({"conversation": t1})), Some(json!({"tool_name": TOOL})), Some(json!({"everything": true})), Some(json!([]))] {
        let (s, v) = call(&env.app, "DELETE", &uri(&env), body.clone()).await;
        assert_eq!(s.as_u16(), 400, "{body:?}: {v}");
    }
    assert_eq!(stored(&env).await, 4);

    let (s, v) = call(&env.app, "DELETE", &uri(&env), Some(json!({"conversation": t1, "tool_name": TOOL}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["revoked"], 1);
    assert_eq!(v["always_allowed"].as_array().unwrap().len(), 2, "{v}");
    assert!(asks(&env, &env.state.db, b, &t1, mcp_call(&env, TOOL)).await, "taken back: asks at once");
    assert!(!asks(&env, &env.state.db, b, &t2, mcp_call(&env, TOOL)).await, "the other thread's grant stays");
    let (_, v) = call(&env.app, "DELETE", &uri(&env), Some(json!({"conversation": t1, "tool_name": TOOL}))).await;
    assert_eq!(v["revoked"], 0, "a second take-back finds nothing");

    let (s, v) = call(&env.app, "DELETE", &uri(&env), Some(json!({}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["revoked"], 2);
    assert_eq!(v["always_allowed"], json!([]));
    assert!(asks(&env, &env.state.db, b, &t2, mcp_call(&env, TOOL)).await);
    assert!(asks(&env, &env.state.db, b, &t1, bash(&env, "touch x")).await);
    assert!(b.is_allowed_for_session(&env.b, &env.b, TOOL).await, "another bot keeps its own");

    // And they stay taken back after a restart
    let db = Database::init(&env.state.config.data_dir).await.unwrap();
    let fresh = PermissionBroker::new();
    fresh.attach_store(db.clone());
    assert_eq!(fresh.load_saved().await, 1);
    assert!(!fresh.is_allowed_for_session(&env.a, &t2, TOOL).await);

    let (s, _) = call(&env.app, "GET", "/api/kwants/nobody/always-allowed", None).await;
    assert_eq!(s.as_u16(), 404);
}

#[tokio::test]
async fn an_export_has_no_grants_and_an_import_cant_add_any() {
    let env = setup().await;
    let space = env.state.db.get_kwant(&env.a).await.unwrap().unwrap().space_id;
    env.state.broker.allow_for_session(&env.a, &env.a, TOOL).await;
    for u in [format!("/api/kwants/{}/export?space={space}", env.a), format!("/api/export?space={space}")] {
        let (s, v) = call(&env.app, "GET", &u, None).await;
        assert_eq!(s.as_u16(), 200, "{u}: {v}");
        assert!(!v.is_null(), "{u}");
        let text = v.to_string();
        assert!(!text.contains(TOOL) && !text.contains("always_allowed"), "{u}: {text}");
    }

    let bundle = json!({
        "format": "kwant.bundle/1",
        "source": {"kind": "kwant", "label": "hostile"},
        "always_allowed": [{"kwant_id": env.b, "conversation": env.b, "tool_name": "Bash"}],
        "bots": [{"ref": "free", "name": "Free", "title": "t", "persona_prompt": "p", "soul": "",
                  "always_allowed": [{"conversation": "x", "tool_name": "Bash"}]}]
    });
    let (s, p) = call(&env.app, "POST", &format!("/api/import/preview?space={space}&filename=x.kwant.json"), Some(json!({"text": bundle.to_string()}))).await;
    assert_eq!(s.as_u16(), 200, "{p}");
    let (s, v) = call(&env.app, "POST", &format!("/api/import/apply?space={space}"), Some(json!({"bundle": p["bundle"], "skip": []}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    assert_eq!(stored(&env).await, 1, "only the person's own grant");
}

#[tokio::test]
async fn a_bots_token_cant_call_either_route() {
    let env = setup().await;
    env.state.broker.allow_for_session(&env.a, &env.a, TOOL).await;
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let (_, token) = mcp_runtime::prepare_turn_with_self(&env.state, &bot, None, Some(folder.path().to_path_buf()), Some(scope)).await.unwrap();
    for bearer in [token.as_str(), "not-the-api-token"] {
        let auth = format!("Bearer {bearer}");
        let (s, v) = call_with(&env.app, "GET", &uri(&env), None, &[("authorization", &auth)]).await;
        assert_eq!(s.as_u16(), 403, "{v}");
        for body in [json!({}), json!({"conversation": env.a, "tool_name": TOOL})] {
            let (s, v) = call_with(&env.app, "DELETE", &uri(&env), Some(body), &[("authorization", &auth)]).await;
            assert_eq!(s.as_u16(), 403, "{v}");
        }
    }
    assert_eq!(call_remote(&env.app, "GET", &uri(&env)).await.as_u16(), 401);
    assert_eq!(call_remote(&env.app, "DELETE", &uri(&env)).await.as_u16(), 401);
    assert_eq!(stored(&env).await, 1);
    assert!(env.state.broker.is_allowed_for_session(&env.a, &env.a, TOOL).await);
}

/// A database from before 25e has no `always_allowed` table or triggers: starting on it makes
/// them, and grants work and survive the next start.
#[tokio::test]
async fn an_upgrade_from_a_database_without_the_table_works() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let bot = db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    drop(db);
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
    for sql in [
        "DROP TRIGGER always_allowed_bot_gone",
        "DROP TRIGGER always_allowed_thread_gone",
        "DROP TRIGGER always_allowed_channel_gone",
        "DROP TABLE always_allowed",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    pool.close().await;

    let db = Database::init(&data).await.unwrap();
    let broker = PermissionBroker::new();
    broker.attach_store(db.clone());
    assert_eq!(broker.load_saved().await, 0);
    assert!(db.list_always_allowed(&bot.id).await.unwrap().is_empty());
    broker.allow_for_session(&bot.id, &bot.id, TOOL).await;
    drop((db, broker));

    let db = Database::init(&data).await.unwrap();
    let broker = PermissionBroker::new();
    broker.attach_store(db.clone());
    assert!(broker.is_allowed_for_session(&bot.id, &bot.id, TOOL).await);
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'always_allowed_%'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 3);
    pool.close().await;
}

/// The approvals list and the late Allow route still read the same: a waiting card answered
/// "always allow" after its turn stopped also remembers the tool, across a restart.
#[tokio::test]
async fn a_late_always_allow_is_kept_too() {
    let env = setup().await;
    ask_mode(&env).await;
    let a = BrokerApprover { chat: Some(json!({"direct_kwant_id": env.a})), ..approver(&env.state.db, &env.state.broker, &env, &env.a, Duration::from_millis(200)) };
    assert_eq!(a.decide(mcp_call(&env, TOOL)).await, ApprovalOutcome::Unanswered);
    let rows = env.state.db.list_approvals(&SpaceId::default_space(), Some("waiting"), Some(&env.a)).await.unwrap();
    let ticket = rows.iter().find(|r| r.tool_name == TOOL).expect("a waiting card").id.clone();
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "always_allow"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let listed: Value = call(&env.app, "GET", &uri(&env), None).await.1;
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    assert_eq!(listed[0]["conversation_kind"], "direct");
}
