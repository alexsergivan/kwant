//! 18a: on the claude engine, kwant's own tools stop repeating a call that keeps failing. The
//! claude CLI posts `mcp__kwnt_tools__*` calls straight to `/api/mcp/self`, past
//! `McpToolSession::call`'s guard (15c), so the endpoint keeps one guard per turn token.
//!
//! The failing tool is `skill_get` for a skill that is turned off: "You have no enabled skill
//! called …" every time. Turning the skill on between calls makes the same call work, so a call
//! that answers with the stop text after that provably didn't run.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::db::skills::clean_skill_text;
use kwnt::db::spaces::SpaceId;
use kwnt::mcp::repeat_guard::{RepeatGuard, STOP_PREFIX};
use kwnt::mcp::session::{Caller, McpToolSession};
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::{fresh_eyes, mcp_runtime};
use serde_json::{json, Value};

const SKILL: &str = "deploy-notes";
const OFF_TEXT: &str = "You have no enabled skill called deploy-notes.";

/// A `tools/call` to kwant's own tools, as the claude CLI sends it: (text, isError).
async fn call_tool(env: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let (s, v) = call_with(
        &env.app,
        "POST",
        "/api/mcp/self",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})),
        &[("authorization", &format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let r = &v["result"];
    let text = r["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
    (text, r["isError"].as_bool().unwrap())
}

async fn get_skill(env: &Env, token: &str) -> (String, bool) {
    call_tool(env, token, "skill_get", json!({"name": SKILL})).await
}

/// A live turn for `kwant`, opened the way a chat turn opens it; returns its token.
async fn open_turn(env: &Env, kwant: &str, turn_id: &str, opts: mcp_runtime::TurnApprovalOpts) -> String {
    let k = env.state.db.get_kwant(kwant).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: k.id.clone(),
        kwant_name: k.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(k.id.clone()), ..Default::default() }),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_opts(&env.state, &k, Some(turn_id.into()), None, Some(scope), opts).await.unwrap().1
}

/// A shared skill, turned off; returns its id.
async fn skill_off(env: &Env) -> String {
    let s = env
        .state
        .db
        .create_user_skill(&SpaceId::default_space(), None, clean_skill_text(SKILL, "How we deploy", "Run the checks first.").unwrap())
        .await
        .unwrap();
    env.state.db.set_skill_status(&s.id, "disabled").await.unwrap();
    s.id
}

async fn set_skill(env: &Env, id: &str, on: bool) {
    env.state.db.set_skill_status(id, if on { "enabled" } else { "disabled" }).await.unwrap();
}

fn is_stop(text: &str) -> bool {
    text.starts_with(STOP_PREFIX)
}

/// Calls 1–3 run; the 4th (and 5th) answer the stop text with isError and don't run, even though
/// the call would now work. Other arguments still run, and another turn has its own count.
#[tokio::test]
async fn the_fourth_identical_failure_does_not_run() {
    let env = setup().await;
    let skill = skill_off(&env).await;
    let tok = open_turn(&env, &env.a, "turn-a", Default::default()).await;
    for i in 1..=3 {
        assert_eq!(get_skill(&env, &tok).await, (OFF_TEXT.to_string(), true), "call {i} runs");
    }
    // If the 4th call ran, it would now work
    set_skill(&env, &skill, true).await;
    for i in 4..=5 {
        let (text, is_error) = get_skill(&env, &tok).await;
        assert!(is_error, "call {i}");
        assert!(text.starts_with("kwnt didn't send this call: the same call (same tool, same arguments) already failed 3 times in this turn"), "call {i}: {text}");
        assert!(text.contains(&format!("\"{OFF_TEXT}\"")), "{text}");
        assert!(text.ends_with("end your reply with \"Blocked: <what is blocked>. <what would unblock it>\"."), "{text}");
    }
    // Other arguments are another call: it runs
    let (text, is_error) = call_tool(&env, &tok, "skill_get", json!({"name": "other-notes"})).await;
    assert_eq!((text.as_str(), is_error), ("You have no enabled skill called other-notes.", true));
    // Another turn's token has its own count: the same call runs (and works) there
    let other = open_turn(&env, &env.a, "turn-a2", Default::default()).await;
    let (text, is_error) = get_skill(&env, &other).await;
    assert!(!is_error && text.starts_with("Skill deploy-notes"), "{text}");
    // ...and the first turn is still stopped
    assert!(is_stop(&get_skill(&env, &tok).await.0));
    // The turn's end drops its guard with its scope
    mcp_runtime::finish_turn(&env.state, &tok).await;
    assert!(env.state.self_tools.lock().await.get(&tok).is_none());
    mcp_runtime::finish_turn(&env.state, &other).await;
}

/// A success in between forgets the call: it fails 3 more times before it is stopped.
#[tokio::test]
async fn a_success_in_between_resets_the_count() {
    let env = setup().await;
    let skill = skill_off(&env).await;
    let tok = open_turn(&env, &env.a, "turn-reset", Default::default()).await;
    for _ in 0..2 {
        assert_eq!(get_skill(&env, &tok).await, (OFF_TEXT.to_string(), true));
    }
    set_skill(&env, &skill, true).await;
    let (text, is_error) = get_skill(&env, &tok).await;
    assert!(!is_error && text.starts_with("Skill deploy-notes"), "{text}");
    set_skill(&env, &skill, false).await;
    for i in 1..=3 {
        assert_eq!(get_skill(&env, &tok).await, (OFF_TEXT.to_string(), true), "failure {i} after the success runs");
    }
    assert!(is_stop(&get_skill(&env, &tok).await.0));
    mcp_runtime::finish_turn(&env.state, &tok).await;
}

/// A refused call isn't a failed call: a fresh-eyes or customer refusal stays a refusal however
/// often it comes, and never counts toward the stop.
#[tokio::test]
async fn refusals_stay_refusals() {
    let env = setup().await;
    skill_off(&env).await;

    // Fresh eyes: skill_get isn't on its list
    let tok = open_turn(&env, &env.a, "turn-fresh", Default::default()).await;
    {
        let _fresh = fresh_eyes::Guard::new("turn-fresh");
        for i in 1..=5 {
            assert_eq!(get_skill(&env, &tok).await, (fresh_eyes::REFUSED_TEXT.to_string(), true), "fresh-eyes refusal {i}");
        }
    }
    // Once the review is over the call runs: none of the 5 refusals counted
    for i in 1..=3 {
        assert_eq!(get_skill(&env, &tok).await, (OFF_TEXT.to_string(), true), "call {i} runs");
    }
    assert!(is_stop(&get_skill(&env, &tok).await.0));
    mcp_runtime::finish_turn(&env.state, &tok).await;

    // A customer's turn: no pass from the approver, so every call is refused, never stopped
    let opts = mcp_runtime::TurnApprovalOpts { every_call: true, ..Default::default() };
    let tok = open_turn(&env, &env.a, "turn-customer", opts).await;
    for i in 1..=5 {
        assert_eq!(get_skill(&env, &tok).await, (mcp_runtime::EVERY_CALL_REFUSED.to_string(), true), "customer refusal {i}");
    }
    let scope = env.state.self_tools.lock().await.get(&tok).cloned().unwrap();
    assert!(scope.repeats.check(&RepeatGuard::key("skill_get", &json!({"name": SKILL}))).is_none(), "refusals never counted");
    mcp_runtime::finish_turn(&env.state, &tok).await;
}

/// API and gateway turns reach kwant's own tools through `McpToolSession`, which still stops the
/// 4th call first. The endpoint's guard saw the same 3 failures and holds the same count, and the
/// session's stop never reaches it.
#[tokio::test]
async fn api_turns_still_stop_at_the_session_with_the_same_count() {
    let env = setup().await;
    let skill = skill_off(&env).await;
    let k = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope { kwant_id: k.id.clone(), kwant_name: k.name.clone(), turn_id: Some("turn-api".into()), ..Default::default() };
    let (ctx, tok) = mcp_runtime::prepare_turn_with_self(&env.state, &k, Some("turn-api".into()), None, Some(scope)).await.unwrap();
    let (session, problems) = McpToolSession::open(&ctx, None).await;
    assert!(problems.is_empty(), "{problems:?}");
    let caller = Caller { kwant_id: k.id.clone(), kwant_name: k.name.clone(), turn_id: "turn-api".into() };
    let name = "mcp__kwnt_tools__skill_get";
    let args = format!(r#"{{"name": "{SKILL}"}}"#);
    for i in 1..=3 {
        let o = session.call(&caller, &format!("c{i}"), name, &args).await;
        assert_eq!((o.status, o.text.as_str()), ("error", OFF_TEXT), "call {i}");
    }
    let endpoint_count = |scope: &SelfScope| scope.repeats.check(&RepeatGuard::key("skill_get", &json!({"name": SKILL}))).map(|s| s.count);
    let scope = env.state.self_tools.lock().await.get(&tok).cloned().unwrap();
    assert_eq!(endpoint_count(&scope), Some(3), "the endpoint counted the same 3 failures");
    // The session stops the 4th first, with its record, before it reaches the endpoint
    set_skill(&env, &skill, true).await;
    let o = session.call(&caller, "c4", name, &args).await;
    assert_eq!(o.status, "error");
    assert!(is_stop(&o.text), "{}", o.text);
    assert_eq!(o.record["repeat_stopped"], 3);
    assert_eq!(endpoint_count(&scope), Some(3), "the session's stop never reached the endpoint");
    // Both guards give the model the same words
    let (text, is_error) = get_skill(&env, &tok).await;
    assert!(is_error);
    assert_eq!(text, o.text);
    session.shutdown().await;
    mcp_runtime::finish_turn(&env.state, &tok).await;
}
