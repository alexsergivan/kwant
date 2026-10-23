//! Unattended turns keep going: a routine's or delegation's turn that runs out of model calls
//! gets one automatic continuation (never two; chats with the person get none), the API cap is
//! 25, and `ask_user` tells bots to state an assumption for reversible choices.

mod common;

use common::*;
use serde_json::json;
use std::time::Duration;

async fn run_routine(env: &Env, kwant: &str, instructions: &str) {
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": kwant, "name": "Nightly", "instructions": instructions,
            "schedule": {"type": "daily", "time": "03:00", "tz": "UTC"}})),
    )
    .await;
    assert_eq!(s.as_u16(), 201, "{r}");
    let (s, v) = call(&env.app, "POST", &format!("/api/routines/{}/run", r["id"].as_str().unwrap()), None).await;
    assert!(s.is_success(), "{v}");
}

#[tokio::test]
async fn a_routine_that_runs_out_of_model_calls_continues_once() {
    let env = setup().await;
    run_routine(&env, &env.a, "Sort the inbox CAPPED:r1").await;
    let done = wait_reply(&env, &env.a, "continued r1").await;
    assert!(done.reasoning_content.as_deref().unwrap_or("").contains("Automatic continuation"), "{done:?}");
    assert_eq!(invocations_with("CAPPED:r1").len(), 2, "the turn and one continuation");
}

#[tokio::test]
async fn the_continuation_happens_only_once() {
    let env = setup().await;
    run_routine(&env, &env.b, "Sort everything CAPALWAYS:r2").await;
    wait_reply(&env, &env.b, "continued r2").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(invocations_with("CAPALWAYS:r2").len(), 2, "never a second continuation");
}

#[tokio::test]
async fn a_chat_with_the_person_gets_no_automatic_continuation() {
    let env = setup().await;
    send_direct(&env, &env.a, "Sort it CAPPED:d1", json!({})).await;
    wait_reply(&env, &env.a, "partial d1").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(invocations_with("CAPPED:d1").len(), 1);
}

#[tokio::test]
async fn a_delegation_continues_once_and_reports_both_parts() {
    let env = setup().await;
    let (s, d) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({"lead_kwant_id": env.a, "specialist_kwant_id": env.b, "kwant_id": env.a,
            "task": "Research CAPPED:g1", "title": "Long research", "continue_lead": false})),
    )
    .await;
    assert_eq!(s.as_u16(), 202, "{d}");
    let result = wait_reply(&env, &env.a, "continued g1").await;
    assert!(result.text.contains("partial g1"), "{}", result.text);
    assert_eq!(invocations_with("CAPPED:g1").len(), 2);
}

#[test]
fn the_cap_note_is_recognised_and_the_default_is_25() {
    use kwnt::drivers::api::hit_call_cap;
    assert_eq!(kwnt::mcp::DEFAULT_MAX_TOOL_ITERATIONS, 25);
    assert!(hit_call_cap("Half done.\n\n(Stopped: reached the limit of 25 model calls for MCP tools this turn.)"));
    assert!(hit_call_cap("(Stopped: reached the limit of 3 model calls for MCP tools this turn.)\n"));
    assert!(!hit_call_cap("I quoted “(Stopped: reached the limit of 25 model calls for MCP tools this turn.)” earlier.\n\nAll done."));
    assert!(!hit_call_cap("All done."));
}

#[test]
fn ask_user_says_state_the_assumption_for_reversible_choices() {
    let d = kwnt::server::questions::ASK_USER_DESCRIPTION;
    assert!(d.contains("state the assumption") && d.contains("reversible"), "{d}");
    assert!(d.contains("can't be undone or reaches outside"), "outward steps still ask: {d}");
    assert_eq!(kwnt::server::questions::tools()[0]["description"], d);
}

/// Runs routine `id` now and waits until that run has finished ok.
async fn run_now_and_wait(env: &Env, id: &str) {
    let (_, before) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
    let seen = before.as_array().map_or(0, Vec::len);
    let (s, v) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert!(s.is_success(), "{v}");
    wait_for("the routine run", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
        let runs = runs.as_array().cloned().unwrap_or_default();
        runs.len() > seen && runs[0]["status"] == "ok"
    })
    .await;
}

#[tokio::test]
async fn a_recurring_routine_sees_its_last_output() {
    let env = setup().await;
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": env.a, "name": "Daily tip", "instructions": "Send a tip of the day TIPRUN REPLYTAG:tip1",
            "schedule": {"type": "daily", "time": "08:00", "tz": "UTC"}})),
    )
    .await;
    assert_eq!(s.as_u16(), 201, "{r}");
    let id = r["id"].as_str().unwrap().to_string();

    // The first run has nothing before it
    run_now_and_wait(&env, &id).await;
    assert_eq!(invocations_with("TIPRUN").len(), 1);
    assert!(!invocations_with("TIPRUN")[0].contains("This routine last ran"));

    // The next one, with the last reply still in view, is pointed at it
    run_now_and_wait(&env, &id).await;
    let second = invocations_with("its reply is above in this chat");
    assert_eq!(second.len(), 1, "{second:?}");

    // Once that reply has scrolled out of the history, it is quoted
    for i in 0..25 {
        let filler = kwnt::db::models::NewMessage {
            channel_id: None,
            direct_kwant_id: Some(env.a.clone()),
            sender_role: "user".into(),
            sender_id: "user".into(),
            text: format!("unrelated chat {i}"),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 1,
        };
        env.state.db.insert_message(&filler).await.unwrap();
    }
    run_now_and_wait(&env, &id).await;
    let third = invocations_with("Its reply then, for continuity");
    assert_eq!(third.len(), 1, "{third:?}");
    assert!(third[0].contains("answer tip1"), "{}", third[0]);
}
