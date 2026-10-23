//! The person's message reaches the model once: as this turn's request, not also as the last line
//! of the history before it.

mod common;

use common::*;
use serde_json::json;

#[tokio::test]
async fn the_current_message_reaches_the_model_once() {
    let env = setup().await;
    send_direct(&env, &env.a, "First ask EARLIER_ASK_41 REPLYTAG:dup1", json!({})).await;
    wait_reply(&env, &env.a, "answer dup1").await;
    send_direct(&env, &env.a, "Second ask CURRENT_ASK_42 REPLYTAG:dup2", json!({})).await;
    wait_reply(&env, &env.a, "answer dup2").await;

    let prompts = invocations_with("CURRENT_ASK_42");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    let p = &prompts[0];
    assert_eq!(p.matches("CURRENT_ASK_42").count(), 1, "{p}");
    // The earlier exchange is still there as history
    assert!(p.contains("EARLIER_ASK_41") && p.contains("answer dup1"), "{p}");
}
