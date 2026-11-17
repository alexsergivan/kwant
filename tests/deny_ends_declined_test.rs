//! 28b: a Deny on a card ends the tool step `declined`, in the person's words. A routine whose
//! only step asks for a shell command, which the person denies on its card: the saved reply's
//! tool step is `declined` with "You said no; nothing ran" (not `failed` with the CLI's "The user
//! denied this tool call."), its card is `declined`, the bot was still told the CLI's own text,
//! and the run stays `incomplete` with "You said no to Bash; nothing ran".

mod common;

use common::*;
use kwnt::db::spaces::SpaceId;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn a_deny_on_the_card_ends_the_step_declined_and_the_run_incomplete() {
    let env = setup().await;
    env.state
        .db
        .set_space_approvals(&SpaceId::default_space(), "ask")
        .await
        .unwrap();
    // Long enough that the card is answered, never timed out
    env.state.approval_wait_ms.store(60_000, Ordering::Relaxed);
    // A shell command always asks; the fake CLI asks for it as Bash, then prints the CLI's denial
    let marker = call_tool_marker(
        "bash",
        json!({"command": "rm -rf DENY28B_BUILD", "description": "clean the build"}),
    );
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": env.a, "name": "Clean", "instructions":
            format!("Clean the build {marker} ASKFIRST ASKAS:Bash REPLYTAG:deny28b"),
            "schedule": {"type": "daily", "time": "03:00", "tz": "UTC"}})),
    )
    .await;
    assert_eq!(s.as_u16(), 201, "{r}");
    let id = r["id"].as_str().unwrap().to_string();
    let (s, run) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s.as_u16(), 202, "{run}");
    let run_id = run["id"].as_str().unwrap().to_string();

    // The person presses Deny on the card
    let waiting = || async {
        env.state
            .db
            .list_approvals(&SpaceId::default_space(), Some("waiting"), Some(&env.a))
            .await
            .unwrap()
    };
    wait_for("the card", || async { !waiting().await.is_empty() }).await;
    let ticket = waiting().await[0].id.clone();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/approvals/{ticket}/resolve"),
        Some(json!({"decision": "deny"})),
    )
    .await;
    assert!(s.is_success(), "{v}");

    let find = |runs: Value| {
        runs.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == run_id.as_str())
            .cloned()
            .unwrap_or(Value::Null)
    };
    wait_for("the run to end", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
        let run = find(runs);
        !run.is_null() && run["status"] != "running"
    })
    .await;
    let (_, runs) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
    let run = find(runs);

    // The saved reply: the tool step is the person's no, paired with its declined card
    let reply = env
        .state
        .db
        .get_message(run["message_id"].as_str().expect("the run saved a reply"))
        .await
        .unwrap()
        .unwrap();
    let meta: Value = reply
        .meta
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    let steps = meta["activity"]["steps"].as_array().cloned().unwrap_or_default();
    let shown: Vec<_> = steps
        .iter()
        .filter(|s| s["kind"] == "tool" || s["kind"] == "approval")
        .map(|s| (s["kind"].clone(), s["status"].clone(), s["tool"].clone(), s["detail"].clone()))
        .collect();
    let tool = shown.iter().find(|s| s.0 == "tool").unwrap_or_else(|| panic!("{steps:?}"));
    assert_eq!(
        (&tool.1, &tool.2, &tool.3),
        (&json!("declined"), &json!("Bash"), &json!("You said no; nothing ran")),
        "{shown:?}"
    );
    let card = shown.iter().find(|s| s.0 == "approval").unwrap_or_else(|| panic!("{steps:?}"));
    assert_eq!((&card.1, &card.2), (&json!("declined"), &json!("Bash")), "{shown:?}");
    assert_eq!(shown.len(), 2, "{shown:?}");
    // The bot still heard the CLI's own text, word for word
    assert!(
        reply.text.contains(&format!("TOOLDENIED[{}]", kwnt::mcp::DENIED_TEXT)),
        "{}",
        reply.text
    );

    // The routine didn't run its step: never ok, in the person's words
    assert_eq!(
        (&run["status"], &run["error"]),
        (&json!("incomplete"), &json!("You said no to Bash; nothing ran")),
        "{run}"
    );
}
