//! The fake claude's once-per-marker flags belong to the kwant instance a test started, not to
//! the test binary: two tests in one file that send the identical `CALLTOOL` marker both get the
//! tool call (Juno's #1112 went red at fffc1a0d when the second one didn't). Within one instance
//! the marker still runs once: a later turn that sees it again in the history doesn't call again.
//!
//! The two tests take turns on one lock, so whichever runs second sends its marker only after the
//! first one's call has finished: on the old per-binary flags that second test always failed.

mod common;

use common::*;
use serde_json::json;
use tokio::sync::Mutex;

/// One turn at a time across both tests (tokio's Mutex works across their runtimes).
static TURNSTILE: Mutex<()> = Mutex::const_new(());

/// The identical marker both tests send.
fn marker() -> String {
    call_tool_marker(
        "skill_propose",
        json!({"name": "shared-marker", "description": "Both tests send this exact marker", "body": "1. step"}),
    )
}

async fn same_marker_calls_the_tool_in_this_instance(tag: &str) {
    let _turn = TURNSTILE.lock().await;
    let env = setup().await;
    send_direct(&env, &env.a, &format!("Save it {} REPLYTAG:{tag}", marker()), json!({})).await;
    let reply = wait_reply(&env, &env.a, &format!("answer {tag}")).await.text;
    assert!(reply.contains("TOOL[Proposed the skill shared-marker"), "the tool ran in this test's own kwant: {reply}");
    assert!(!reply.contains("TOOLERROR"), "{reply}");

    // The same instance still runs a marker once: the next turn sees it in the history only
    let again = format!("{tag}again");
    send_direct(&env, &env.a, &format!("Thanks REPLYTAG:{again}"), json!({})).await;
    let reply = wait_reply(&env, &env.a, &format!("answer {again}")).await.text;
    assert!(!reply.contains("TOOL["), "no second call in one instance: {reply}");
}

#[tokio::test]
async fn first_test_with_the_shared_marker_calls_the_tool() {
    same_marker_calls_the_tool_in_this_instance("once1").await;
}

#[tokio::test]
async fn second_test_with_the_same_marker_calls_the_tool_too() {
    same_marker_calls_the_tool_in_this_instance("once2").await;
}
