//! A card nobody answers before its turn stops waiting: the bot is told the truth (not
//! "denied"), the card stays open, and a late Allow starts a new turn whose same call runs once
//! without another card. A late Deny just closes it.

mod common;

use common::*;
use kwnt::mcp::{ApprovalOutcome, ToolApprovalRequest, ToolApprover, UNANSWERED_TEXT};
use kwnt::server::mcp_runtime::BrokerApprover;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

fn approver(env: &Env, timeout: Duration) -> Arc<BrokerApprover> {
    Arc::new(BrokerApprover {
        db: env.state.db.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout,
        conversation: env.a.clone(),
        chat: Some(json!({"direct_kwant_id": env.a})),
        always_ask: false,
    })
}

fn request(env: &Env, args: Value) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-late".into(),
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: args,
        builtin: None,
    }
}

/// Runs one call that nobody answers and returns its ticket id.
async fn unanswered(env: &Env, args: Value) -> String {
    let a = approver(env, Duration::from_millis(200));
    assert_eq!(a.decide(request(env, args)).await, ApprovalOutcome::Unanswered);
    let rows = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), Some("waiting"), Some(&env.a)).await.unwrap();
    rows.first().expect("a waiting card").id.clone()
}

#[tokio::test]
async fn an_unanswered_card_stays_open_and_a_late_allow_resumes_the_work() {
    let env = setup().await;
    let args = json!({"command": "ls LATE_ALLOW_MARK", "description": "list"});
    let ticket = unanswered(&env, args.clone()).await;
    assert!(!env.state.broker.is_pending(&ticket).await, "the turn stopped waiting");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "waiting");

    // Still in the person's waiting list, linked to the chat it resumes in
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    let item = away["waiting"].as_array().unwrap().iter().find(|i| i["id"] == ticket.as_str()).expect("card listed").clone();
    assert_eq!(item["kind"], "approval");
    assert_eq!(item["link"]["direct_kwant_id"], env.a.as_str(), "{item}");

    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["resumed"], "new_turn");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "allowed_once");
    wait_for("the resumed turn", || async { !invocations_with("ls LATE_ALLOW_MARK").is_empty() }).await;
    let prompt = invocations_with("ls LATE_ALLOW_MARK").pop().unwrap();
    assert!(prompt.contains("[The person allowed a tool call you asked for earlier]"), "{prompt}");
    assert!(prompt.contains("Tool: Bash"));

    // The stub's resumed turn ends without making the call: its grant goes with it, so the
    // same call later gets a card again instead of running unasked
    wait_reply(&env, &env.a, "answer none").await;
    wait_for("the resumed turn to end", || async { env.state.turns.active_turns().is_empty() }).await;
    let a = approver(&env, Duration::from_millis(200));
    let again = json!({"command": "ls LATE_ALLOW_MARK", "description": "list the files"});
    wait_for("the unused grant to be dropped", || async {
        !env.state.broker.drop_grant(&env.a, &env.a, "Bash", &again).await
    })
    .await;
    assert_eq!(a.decide(request(&env, again)).await, ApprovalOutcome::Unanswered, "an unused grant must not outlive its turn");

    // An answered card can't be answered again
    let (s, _) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 409);
}

#[tokio::test]
async fn a_grant_runs_once_only_in_its_own_conversation() {
    let env = setup().await;
    let args = json!({"command": "ls GRANT_SCOPE_MARK", "description": "list"});
    env.state.broker.grant_once(&env.a, &env.a, "Bash", &args).await;
    // Another conversation of the same bot can't use it
    let elsewhere = BrokerApprover {
        db: env.state.db.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: "some-other-thread".into(),
        chat: Some(json!({"thread_id": "some-other-thread"})),
        always_ask: false,
    };
    assert_eq!(elsewhere.decide(request(&env, args.clone())).await, ApprovalOutcome::Unanswered);
    // Its own conversation can, once (a new description is fine), and only that call
    let a = approver(&env, Duration::from_millis(200));
    let again = json!({"command": "ls GRANT_SCOPE_MARK", "description": "list the files"});
    assert_eq!(tokio::time::timeout(Duration::from_secs(1), a.decide(request(&env, again.clone()))).await.unwrap(), ApprovalOutcome::Allowed);
    assert_eq!(a.decide(request(&env, again)).await, ApprovalOutcome::Unanswered);
    env.state.broker.grant_once(&env.a, &env.a, "Bash", &args).await;
    assert_eq!(a.decide(request(&env, json!({"command": "rm -rf GRANT_SCOPE_MARK"}))).await, ApprovalOutcome::Unanswered);
}

/// The resumed turn runs on the agent and effort of the turn that asked (kept with the card),
/// not on whatever Laya makes of the resume prompt (here Laya is down, which would mean
/// "medium").
#[tokio::test]
async fn the_resumed_turn_keeps_the_asking_turns_effort() {
    let env = setup().await;
    let a = Arc::new(BrokerApprover {
        chat: Some(json!({"direct_kwant_id": env.a, "agent": "claude", "effort": "low"})),
        ..Arc::try_unwrap(approver(&env, Duration::from_millis(200))).ok().unwrap()
    });
    let args = json!({"command": "ls EFFORT_MARK", "description": "list"});
    assert_eq!(a.decide(request(&env, args)).await, ApprovalOutcome::Unanswered);
    let rows = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), Some("waiting"), Some(&env.a)).await.unwrap();
    let ticket = rows.first().expect("a waiting card").id.clone();

    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let reply = wait_reply(&env, &env.a, "answer none").await;
    let meta: Value = serde_json::from_str(reply.meta.as_deref().expect("reply meta")).unwrap();
    assert_eq!(meta["ran_on"]["effort"], "low", "{meta}");
}

#[tokio::test]
async fn a_late_deny_closes_the_card_and_starts_nothing() {
    let env = setup().await;
    let args = json!({"command": "ls LATE_DENY_MARK"});
    let ticket = unanswered(&env, args.clone()).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "deny"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["resumed"], false);
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "denied");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(invocations_with("ls LATE_DENY_MARK").is_empty(), "no turn after a deny");
    let a = approver(&env, Duration::from_millis(200));
    assert_eq!(a.decide(request(&env, args)).await, ApprovalOutcome::Unanswered, "a deny grants nothing");
}

#[tokio::test]
async fn kwnt_secrets_stay_refused_even_after_a_late_allow() {
    let env = setup().await;
    let a = approver(&env, Duration::from_millis(200));
    // A grant can't reach past the secret check (it runs first)
    let secret = json!({"command": "cat ~/.kwnt/api-token"});
    assert!(kwnt::sandbox::credentials::call_touches_kwnt_secrets(&secret, None));
    env.state.broker.grant_once(&env.a, &env.a, "Bash", &secret).await;
    assert_eq!(a.decide(request(&env, secret)).await, ApprovalOutcome::Denied);
}

#[tokio::test]
async fn an_old_waiting_card_expires_instead_of_resuming() {
    let env = setup().await;
    let ticket = unanswered(&env, json!({"command": "ls OLD_CARD_MARK"})).await;
    env.state.db.backdate_approval_for_test(&ticket, 25).await.unwrap();
    let (s, _) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 409);
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "expired");
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    assert!(!away["waiting"].as_array().unwrap().iter().any(|i| i["id"] == ticket.as_str()));
}

#[test]
fn the_bot_is_told_the_card_stays_open_not_that_it_was_denied() {
    assert!(!UNANSWERED_TEXT.contains("denied"));
    assert!(UNANSWERED_TEXT.contains("stays open"));
    assert_eq!(ApprovalOutcome::Unanswered.refusal_text(), UNANSWERED_TEXT);
}
