//! A turn a usage limit stopped before the bot did anything is parked until the limit resets and
//! retried once; a retry never parks again, and a later turn that got through drops it.

mod common;

use common::*;
use kwnt::server::parked;
use serde_json::json;
use std::time::Duration;

fn later() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::seconds(30)
}

/// Waits until the parked turns' reset time has passed (the profile skips turns until then).
async fn past_reset(env: &Env) {
    let at = parked_rows(env).await.iter().map(|p| p.retry_at).max().unwrap();
    let wait = (at - chrono::Utc::now()).to_std().unwrap_or_default() + Duration::from_millis(1200);
    tokio::time::sleep(wait).await;
}

async fn parked_rows(env: &Env) -> Vec<kwnt::db::parked::ParkedTurn> {
    env.state.db.list_parked().await.unwrap()
}

#[tokio::test]
async fn a_limited_turn_is_retried_once_when_the_limit_resets() {
    let env = setup().await;
    send_direct(&env, &env.a, "do it QUOTAONCE:q1 REPLYTAG:q1done", json!({})).await;
    let notice = wait_reply(&env, &env.a, "will retry this once").await;
    assert!(notice.text.contains("UTC"), "{}", notice.text);
    let rows = parked_rows(&env).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "pending");
    assert!(rows[0].retry_at > chrono::Utc::now() - chrono::Duration::seconds(5));
    assert!(rows[0].retry_at <= chrono::Utc::now() + chrono::Duration::seconds(5), "the provider's reset time");

    // Not due yet: nothing runs
    assert_eq!(parked::run_due(&env.state, rows[0].retry_at - chrono::Duration::seconds(1)).await, 0);
    past_reset(&env).await;
    assert_eq!(parked::run_due(&env.state, later()).await, 1);
    let reply = wait_reply(&env, &env.a, "answer q1done").await;
    assert!(!reply.text.contains("usage limit"));
    let all = messages(&env, &env.a).await;
    assert!(all.iter().any(|m| m.sender_role == "system" && m.text.contains("retrying")), "a visible retry line");
    let prompts = invocations_with("REPLYTAG:q1done");
    assert_eq!(prompts.len(), 2, "the first try and one retry");
    assert!(prompts.iter().any(|p| p.contains("[kwnt is retrying your last turn")));

    // Once
    assert_eq!(parked_rows(&env).await[0].status, "done");
    assert_eq!(parked::run_due(&env.state, later()).await, 0);
}

#[tokio::test]
async fn a_retry_that_hits_the_limit_again_is_not_parked_again() {
    let env = setup().await;
    send_direct(&env, &env.a, "do it QUOTAALWAYS REPLYTAG:never", json!({})).await;
    wait_reply(&env, &env.a, "will retry this once").await;
    past_reset(&env).await;
    assert_eq!(parked::run_due(&env.state, later()).await, 1);
    wait_for("the retry to fail", || async {
        messages(&env, &env.a).await.iter().filter(|m| m.sender_role == "kwant" && m.text.contains("out of usage")).count() == 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rows = parked_rows(&env).await;
    assert_eq!(rows.len(), 1, "no second park");
    let last = messages(&env, &env.a).await.into_iter().rfind(|m| m.sender_role == "kwant").unwrap();
    assert!(!last.text.contains("will retry"), "{}", last.text);
}

#[tokio::test]
async fn a_later_turn_that_got_through_drops_the_parked_one() {
    let env = setup().await;
    send_direct(&env, &env.b, "first QUOTAONCE:q3 REPLYTAG:q3a", json!({})).await;
    wait_reply(&env, &env.b, "will retry this once").await;
    past_reset(&env).await;
    // A second limited message in the same chat doesn't add a second retry
    send_direct(&env, &env.b, "second QUOTAONCE:q3c", json!({})).await;
    wait_reply(&env, &env.b, "already set").await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(parked_rows(&env).await.len(), 1);

    send_direct(&env, &env.b, "third REPLYTAG:q3b", json!({})).await;
    wait_reply(&env, &env.b, "answer q3b").await;
    assert_eq!(parked_rows(&env).await[0].status, "dropped");
    assert_eq!(parked::run_due(&env.state, later()).await, 0);
}

#[test]
fn the_retry_line_says_when() {
    let now = chrono::Utc::now();
    let line = parked::retry_line(now + chrono::Duration::minutes(200), now);
    assert!(line.contains("about 3 h"), "{line}");
    assert!(parked::retry_line(now + chrono::Duration::minutes(20), now).contains("about 20 min"));
}

#[tokio::test]
async fn a_limited_delegation_stays_open_and_its_retry_finishes_it() {
    let env = setup().await;
    let (s, d) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({
            "lead_kwant_id": env.a, "specialist_kwant_id": env.b, "kwant_id": env.a,
            "task": "research QUOTAONCE:qd REPLYTAG:qddone", "title": "Limited task", "continue_lead": false,
        })),
    )
    .await;
    assert_eq!(s.as_u16(), 202, "{d}");
    let id = d["id"].as_str().unwrap().to_string();
    let status = |env: &Env, id: String| {
        let db = env.state.db.clone();
        async move { db.get_delegation(&id).await.unwrap().unwrap().status }
    };
    wait_for("the lead chat to hear about the limit", || async {
        messages(&env, &env.a).await.iter().any(|m| m.text.contains("hit a usage limit") && m.text.contains("will retry"))
    })
    .await;
    assert_eq!(status(&env, id.clone()).await, "running", "not failed: it waits for the reset");
    let rows = parked_rows(&env).await;
    assert_eq!(rows[0].delegation_id.as_deref(), Some(id.as_str()));

    // A kwnt restart meanwhile leaves it to the parked retry
    let (again, cut) = env.state.db.restart_open_delegations().await.unwrap();
    assert!(again.is_empty() && cut.is_empty());

    past_reset(&env).await;
    assert_eq!(parked::run_due(&env.state, later()).await, 1);
    // The delegation is marked completed before its result is posted, so wait for the result
    wait_reply(&env, &env.a, "answer qddone").await;
    assert_eq!(status(&env, id.clone()).await, "completed");
    let lead = messages(&env, &env.a).await;
    assert!(lead.iter().any(|m| m.text.contains("retrying “Limited task” once")));
}
