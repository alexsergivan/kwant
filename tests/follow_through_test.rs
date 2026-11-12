//! 0.9.4 follow-through: a turn whose last tool step failed and whose reply ends on a plan it
//! didn't carry out ("I'll retry the render…") gets exactly one automatic continuation, with no
//! delivery plan needed. A reply that ends with `Blocked: …` instead gets none, and its meta keeps
//! the line (`meta.blocked`).
//!
//! The bot's folder isn't a git repository and the asks aren't for a picture or a change, so
//! nothing but follow-through can continue them.

mod common;

use common::*;
use std::time::Duration;

/// The start of kwant's follow-through prompt (`delivery_check::follow_prompt`).
const FOLLOW: &str = "failed or was refused, and your reply ends with a plan that didn't run";

/// The request a CLI run answers: its prompt's current request, without the chat history.
fn request_of(args: &str) -> &str {
    let prompt = args.rsplit_once("\n-p\n").map_or(args, |(_, p)| p);
    prompt.rsplit_once("### Current User Request:\n").map_or(prompt, |(_, r)| r)
}

/// The requests of `env`'s CLI runs (their arguments name its data folder).
fn requests(env: &Env) -> Vec<String> {
    let data = env.tmp.path().display().to_string();
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.ok()?.path()).ok())
        .filter(|args| args.contains(&data))
        .map(|args| request_of(&args).to_string())
        .collect()
}

fn meta_of(m: &kwnt::db::models::Message) -> serde_json::Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
}

async fn replies_of(env: &Env, kwant: &str) -> Vec<kwnt::db::models::Message> {
    messages(env, kwant).await.into_iter().filter(|m| m.sender_role == "kwant").collect()
}

fn bot_idle(env: &Env, kwant: &str) -> bool {
    env.state.turns.active_turns().iter().all(|t| t.kwant_id != kwant)
}

#[tokio::test]
async fn a_failed_step_and_a_promise_get_one_continuation() {
    let env = setup().await;
    let a = env.a.clone();
    send_direct(&env, &a, "How long are the intro's frames? FAILSTEP:zqfail REPLYTAG:zqfail", serde_json::json!({})).await;
    wait_for("two replies", || async { replies_of(&env, &a).await.len() >= 2 }).await;
    wait_for("the bot to finish", || async { bot_idle(&env, &a) }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let replies = replies_of(&env, &a).await;
    assert_eq!(replies.len(), 2, "the reply and its one continuation: {replies:#?}");
    assert_eq!(replies[0].text, "The render command was refused (zqfail).\n\nI'll retry the render with a smaller size next.");
    let first = meta_of(&replies[0]);
    assert_eq!(first["delivery"]["missing"], serde_json::json!(["follow_through"]), "{first}");
    assert_eq!(first["delivery"]["continues"], true, "{first}");
    let steps = first["activity"]["steps"].as_array().cloned().unwrap_or_default();
    assert!(steps.iter().any(|s| s["kind"] == "tool" && s["status"] == "failed"), "the failed step is in the activity: {first}");

    // Exactly one continuation, told what failed and what it promised
    let all = requests(&env);
    let cont: Vec<&String> = all.iter().filter(|r| r.contains(FOLLOW)).collect();
    assert_eq!(cont.len(), 1, "{all:#?}");
    assert!(cont[0].contains("Your last step (Bash"), "{}", cont[0]);
    assert!(cont[0].contains("(\"I'll retry the render with a smaller size next.\")"), "{}", cont[0]);
    assert!(cont[0].contains("`Blocked: <what is blocked>. <what would unblock it>`"), "{}", cont[0]);
    assert_eq!(all.len(), 2, "{all:#?}");

    // The continuation ran no tool and answered without a promise: never continued, no kwant line
    assert_eq!(replies[1].text, "answer zqfail");
    let second = meta_of(&replies[1]);
    assert_ne!(second["delivery"]["continues"], true, "{second}");
    assert!(second.get("blocked").is_none(), "{second}");
    let label = replies[1].reasoning_content.as_deref().unwrap_or_default();
    assert!(label.contains("the last step failed and the reply ended on a plan"), "{label}");
}

#[tokio::test]
async fn a_blocked_line_ends_it_without_a_continuation() {
    let env = setup().await;
    let a = env.a.clone();
    send_direct(&env, &a, "How long are the outro's frames? BLOCKSTEP:zqblock REPLYTAG:zqblock", serde_json::json!({})).await;
    wait_for("a reply", || async { !replies_of(&env, &a).await.is_empty() }).await;
    wait_for("the bot to finish", || async { bot_idle(&env, &a) }).await;
    tokio::time::sleep(Duration::from_millis(800)).await;

    let replies = replies_of(&env, &a).await;
    assert_eq!(replies.len(), 1, "no continuation: {replies:#?}");
    let meta = meta_of(&replies[0]);
    assert_eq!(
        meta["blocked"],
        serde_json::json!({"text": "terminal commands are off for this bot. Allow Terminal for this team, then say continue."}),
        "{meta}"
    );
    assert_ne!(meta["delivery"]["continues"], true, "{meta}");
    let all = requests(&env);
    assert!(!all.iter().any(|r| r.contains(FOLLOW)), "{all:#?}");
    assert_eq!(all.len(), 1, "{all:#?}");
}
