//! The production floor through the real approval route (plan §3.5): in a Dev mode space a step
//! that needs a backup gets a card with `prod_floor`, and **Allow** on it is refused by the server
//! (409, the card stays waiting) until the space has an ok backup at most 10 minutes old. Deny is
//! always accepted.

mod common;

use common::*;
use kwnt::db::dev_mode::DevBackupRow;
use kwnt::mcp::{ApprovalOutcome, ToolApprovalRequest, ToolApprover};
use kwnt::server::mcp_runtime::BrokerApprover;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

fn approver(env: &Env) -> Arc<BrokerApprover> {
    Arc::new(BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: env.a.clone(),
        chat: Some(json!({"direct_kwant_id": env.a})),
        always_ask: false,
    })
}

fn request(env: &Env, command: &str) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t-floor".into(),
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: json!({"command": command, "description": "demo"}),
        builtin: None,
        smart: None,
        payment: None,
    }
}

/// Runs one call nobody answers and returns its waiting card.
async fn unanswered(env: &Env, command: &str) -> (String, Value) {
    assert_eq!(approver(env).decide(request(env, command)).await, ApprovalOutcome::Unanswered);
    let rows = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), Some("waiting"), Some(&env.a)).await.unwrap();
    let row = rows.into_iter().find(|r| r.parameters.contains(command)).expect("a waiting card");
    let details: Value = serde_json::from_str(&row.parameters).unwrap();
    (row.id, details)
}

async fn space_of_a(env: &Env) -> String {
    env.state.db.get_kwant(&env.a).await.unwrap().unwrap().space_id
}

fn backup(space: &str, ok: bool, age_s: i64) -> DevBackupRow {
    DevBackupRow {
        id: format!("dvb_{}", uuid::Uuid::new_v4().simple()),
        space_id: space.into(),
        kwant_id: None,
        path: Some("/tmp/demo.sql".into()),
        bytes: Some(10),
        ok,
        error: (!ok).then(|| "exit 1".into()),
        taken_at: chrono::Utc::now() - chrono::Duration::seconds(age_s),
    }
}

#[tokio::test]
async fn allow_on_a_step_that_needs_a_backup_is_refused_until_a_fresh_one() {
    let env = setup().await;
    let space = space_of_a(&env).await;

    // Dev mode off: no floor on the card
    let (_, off) = unanswered(&env, "sqlite3 demo.db 'DROP TABLE demo_off'").await;
    assert!(off.get("prod_floor").is_none(), "{off}");

    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, Some("pg_dump demo > /tmp/demo.sql")).await.unwrap();
    let (ticket, details) = unanswered(&env, "sqlite3 demo.db 'DROP TABLE demo_on'").await;
    let floor = &details["prod_floor"];
    assert_eq!(floor["kind"], "destructive", "{details}");
    assert_eq!(floor["needs_backup"], true);
    assert_eq!(floor["space_id"], space.as_str());
    assert_eq!(floor["backup"]["state"], "none");

    // No backup, then an 11-minute-old one: Allow and Always allow are refused, the card waits
    env.state.db.insert_dev_backup(&backup(&space, true, 660)).await.unwrap();
    for decision in ["allow_once", "always_allow"] {
        let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": decision}))).await;
        assert_eq!(s.as_u16(), 409, "{v}");
        assert_eq!(v["status"], "needs_backup");
        assert_eq!(v["resolved"], false);
        assert_eq!(v["ticket_id"], ticket.as_str());
        assert_eq!(v["backup"]["state"], "old");
        assert!(v["error"].as_str().unwrap().starts_with("Back up first"));
    }
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "waiting");

    // A fresh ok backup, taken while the card waits, lets Allow through
    env.state.db.insert_dev_backup(&backup(&space, true, 20)).await.unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "allowed_once");
    wait_for("the resumed turn to end", || async { env.state.turns.active_turns().is_empty() }).await;
}

#[tokio::test]
async fn deny_is_always_accepted_on_a_floor_card() {
    let env = setup().await;
    let space = space_of_a(&env).await;
    env.state.db.put_dev_mode(&space, true, "demo-org/demo-app", "main", None, None).await.unwrap();
    let (ticket, details) = unanswered(&env, "psql postgres://db.example.com/demo -c 'TRUNCATE demo'").await;
    assert_eq!(details["prod_floor"]["needs_backup"], true, "{details}");
    assert_eq!(details["prod_floor"]["backup"]["state"], "no_command");
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "deny"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.get_approval(&ticket).await.unwrap().unwrap().status, "denied");
}
