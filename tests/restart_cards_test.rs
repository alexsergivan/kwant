//! Two filming bugs. "Answer in chat" on a task's card opens the chat the card is in (the
//! owner's hand-off thread or chat), not the chat the task came from. And a card left waiting
//! when kwant restarts is still answerable afterwards: a late Allow resumes the work in the
//! chat that asked, a payment's "always allow" still covers one call (D8), and a card that is
//! too old says plainly that it expired instead of erroring.

mod common;

use common::*;
use kwnt::broker::PermissionBroker;
use kwnt::db::continuity::RunningTurn;
use kwnt::db::models::NewApproval;
use kwnt::db::questions::QuestionPlace;
use kwnt::db::spaces::SpaceId;
use kwnt::drivers::events::RuntimeEvent;
use kwnt::mcp::{ToolApprovalRequest, ToolApprover};
use kwnt::server::mcp_runtime::{self, BrokerApprover};
use kwnt::server::missions::listener;
use serde_json::{json, Value};
use std::time::Duration;

async fn general(env: &Env) -> String {
    env.state.db.list_channels(&SpaceId::default_space()).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id
}

async fn get(env: &Env, uri: &str) -> Value {
    let (s, v) = call(&env.app, "GET", uri, None).await;
    assert!(s.is_success(), "GET {uri}: {s} {v}");
    v
}

async fn status_of(env: &Env, ticket: &str) -> String {
    env.state.db.get_approval(ticket).await.unwrap().unwrap().status
}

/// A card asked by `env.a` in its direct chat, by the kwant that ran before the restart: kept
/// in the database, unknown to this process's broker. Returns its ticket id.
async fn card_from_before_the_restart(env: &Env, args: Value) -> String {
    let old = BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: PermissionBroker::new(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_secs(600),
        conversation: env.a.clone(),
        chat: Some(json!({"direct_kwant_id": env.a})),
        always_ask: false,
    };
    let req = ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-before-restart".into(),
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: args,
        builtin: Some(kwnt::mcp::BuiltinCall { cwd: None, outside_folder: false }),
        smart: None,
        payment: None,
    };
    let mut events = env.state.event_sender.subscribe();
    // The asking turn never hears back: its process "stopped"
    std::mem::forget(tokio::spawn(async move { old.decide(req).await }));
    let id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(RuntimeEvent::ApprovalRequired { id, .. }) = events.recv().await {
                break id;
            }
        }
    })
    .await
    .expect("a card");
    assert!(!env.state.broker.is_pending(&id).await, "this process never knew the card");
    id
}

async fn resolve(env: &Env, ticket: &str, decision: &str) -> (u16, Value) {
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": decision}))).await;
    (s.as_u16(), v)
}

#[tokio::test]
async fn a_card_from_before_a_restart_is_still_answerable_and_resumes_the_work() {
    let env = setup().await;
    let ticket = card_from_before_the_restart(&env, json!({"command": "ls RESTART_ALLOW_MARK", "description": "list"})).await;
    assert_eq!(status_of(&env, &ticket).await, "pending");

    assert_eq!(mcp_runtime::keep_cards_after_restart(&env.state).await, 1);
    assert_eq!(status_of(&env, &ticket).await, "waiting");
    let away = get(&env, "/api/away").await;
    let item = away["waiting"].as_array().unwrap().iter().find(|i| i["id"] == ticket.as_str()).expect("card listed").clone();
    assert_eq!(item["link"]["direct_kwant_id"], env.a.as_str(), "{item}");

    let (s, v) = resolve(&env, &ticket, "allow_once").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["resumed"], "new_turn", "{v}");
    assert_eq!(status_of(&env, &ticket).await, "allowed_once");
    wait_for("the resumed turn", || async { !invocations_with("ls RESTART_ALLOW_MARK").is_empty() }).await;
    let prompt = invocations_with("ls RESTART_ALLOW_MARK").pop().unwrap();
    assert!(prompt.contains("[The person allowed a tool call you asked for earlier]"), "{prompt}");
}

#[tokio::test]
async fn a_card_with_no_chat_resumes_where_its_turn_was_running() {
    let env = setup().await;
    let thread = env.state.db.create_thread(&env.a, "Hand-off", "delegation", None, &env.b).await.unwrap();
    // An older kwant didn't keep the chat with a pending card; the running turn knows it
    let card = env
        .state
        .db
        .save_approval(&NewApproval {
            turn_id: "t-running".into(),
            kwant_id: env.a.clone(),
            tool_name: "Bash".into(),
            parameters: json!({"arguments": {"command": "ls"}}),
        })
        .await
        .unwrap();
    let turn = RunningTurn {
        turn_id: "t-running".into(),
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        channel_id: None,
        direct_kwant_id: None,
        thread_id: Some(thread.id.clone()),
        started_at: chrono::Utc::now(),
    };
    env.state.db.add_running_turn(&turn).await.unwrap();
    // And one from a turn nobody recorded lands in the bot's direct chat
    let lost = env
        .state
        .db
        .save_approval(&NewApproval { turn_id: "t-lost".into(), kwant_id: env.a.clone(), tool_name: "Bash".into(), parameters: json!({}) })
        .await
        .unwrap();

    assert_eq!(mcp_runtime::keep_cards_after_restart(&env.state).await, 2);
    let away = get(&env, "/api/away").await;
    let link = |id: &str| away["waiting"].as_array().unwrap().iter().find(|i| i["id"] == id).expect("card listed")["link"].clone();
    assert_eq!(link(&card.id)["thread_id"], thread.id.as_str(), "{away}");
    assert_eq!(link(&lost.id)["direct_kwant_id"], env.a.as_str(), "{away}");
}

#[tokio::test]
async fn always_allow_on_a_payment_card_after_a_restart_covers_one_call() {
    let env = setup().await;
    let charge = "curl -X POST https://api.stripe.com/v1/charges -u sk_test_x: -d amount=100 -d currency=usd";
    let ticket = card_from_before_the_restart(&env, json!({"command": charge})).await;
    let details: Value = serde_json::from_str(&env.state.db.get_approval(&ticket).await.unwrap().unwrap().parameters).unwrap();
    assert!(details.get("payment_reason").is_some(), "a payment card: {details}");

    mcp_runtime::keep_cards_after_restart(&env.state).await;
    let (s, v) = resolve(&env, &ticket, "always_allow").await;
    assert_eq!(s, 200, "{v}");
    assert!(!env.state.broker.is_allowed_for_session(&env.a, &env.a, "Bash").await, "a payment is never remembered");
}

#[tokio::test]
async fn a_card_too_old_to_resume_says_it_expired_instead_of_erroring() {
    let env = setup().await;
    let ticket = card_from_before_the_restart(&env, json!({"command": "ls RESTART_OLD_MARK"})).await;
    mcp_runtime::keep_cards_after_restart(&env.state).await;
    env.state.db.backdate_approval_for_test(&ticket, 25).await.unwrap();

    let (s, v) = resolve(&env, &ticket, "allow_once").await;
    assert_eq!(s, 409);
    assert_eq!(v["status"], "expired", "{v}");
    assert!(v["error"].as_str().unwrap().contains("Ask the bot again"), "{v}");
    assert_eq!(status_of(&env, &ticket).await, "expired");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(invocations_with("ls RESTART_OLD_MARK").is_empty(), "nothing resumed");
}

/// The lead (a) plans in #general; b owns the task and works in its hand-off thread. The task's
/// cards point at b's chats, which is where "Answer in chat" goes.
#[tokio::test]
async fn a_tasks_cards_say_which_chat_answers_them() {
    let env = setup().await;
    call(&env.app, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    let general = general(&env).await;
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": env.a, "budget_tokens": 900_000,
                      "channel_id": general, "tasks": [{"title": "A", "owner": env.b}]});
    let (s, m) = call(&env.app, "POST", "/api/missions", Some(body)).await;
    assert!(s.is_success(), "{m}");
    let detail = get(&env, &format!("/api/missions/{}", m["id"].as_str().unwrap())).await;
    let tid = detail["tasks"][0]["id"].as_str().unwrap().to_string();
    env.state.db.claim_task(&tid, &env.b, None, None, 600_000, false, &env.b).await.unwrap().unwrap();

    let thread = env.state.db.create_thread(&env.b, "Hand-off", "delegation", None, &env.a).await.unwrap();
    let place = QuestionPlace { kwant_id: env.b.clone(), channel_id: None, direct_kwant_id: None, thread_id: Some(thread.id.clone()), turn_id: None };
    let question = env.state.db.create_question(&place, "Which colour?", &[]).await.unwrap();
    let card = card_by(&env, &env.b).await;
    listener::refresh_needs_you(&env.state).await;

    let t = get(&env, &format!("/api/tasks/{tid}")).await;
    assert_eq!(t["task"]["state"], "needs_you", "{t}");
    assert_eq!(t["task"]["channel_id"], general.as_str(), "the task came from #general");
    let items = t["task"]["needs_you"].as_array().unwrap();
    let item = |id: &str| items.iter().find(|i| i["id"] == id).unwrap_or_else(|| panic!("{id} in {t}")).clone();
    let asked = item(&question.id);
    assert_eq!(asked["chat"], json!({"channel_id": null, "kwant_id": env.b, "thread_id": thread.id}), "{asked}");
    let approval = item(&card);
    assert_eq!(approval["chat"]["kwant_id"], env.b.as_str(), "{approval}");
    assert_eq!(approval["chat"]["channel_id"], Value::Null);

    // A card left waiting (its turn stopped) still holds the task; answering it lets go
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(status_of(&env, &card).await, "waiting");
    env.state.db.close_question(&question.id, "answered", Some("red")).await.unwrap();
    listener::refresh_needs_you(&env.state).await;
    let t = get(&env, &format!("/api/tasks/{tid}")).await;
    assert_eq!(t["task"]["needs_you"].as_array().unwrap().len(), 1, "{t}");
    assert_eq!(t["task"]["state"], "needs_you");
    let (s, v) = resolve(&env, &card, "deny").await;
    assert_eq!(s, 200, "{v}");
    let t = get(&env, &format!("/api/tasks/{tid}")).await;
    assert_eq!(t["task"]["state"], "working", "{t}");
}

/// A card `kwant` asks in its direct chat that nobody answers in time. Returns its ticket id.
async fn card_by(env: &Env, kwant: &str) -> String {
    let a = BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: kwant.to_string(),
        chat: Some(json!({"direct_kwant_id": kwant})),
        always_ask: false,
    };
    let req = ToolApprovalRequest {
        kwant_id: kwant.to_string(),
        kwant_name: "B".into(),
        turn_id: "t-card".into(),
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: json!({"command": "ls CARD_CHAT_MARK"}),
        builtin: None,
        smart: None,
        payment: None,
    };
    let mut events = env.state.event_sender.subscribe();
    tokio::spawn(async move { a.decide(req).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(RuntimeEvent::ApprovalRequired { id, .. }) = events.recv().await {
                break id;
            }
        }
    })
    .await
    .expect("a card")
}
