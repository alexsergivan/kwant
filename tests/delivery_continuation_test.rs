//! #469 x #467: the delivery check's one automatic continuation under P6's one turn per bot per
//! chat. The continuation starts on its first turn's chat slot, so it never sits in the queue,
//! and a message the person sent meanwhile is answered after it, never in between. A P7
//! stall-retried try that then misses is still continued once at most.
//!
//! The bot's folder is a git repository (a `.git` folder), so "Fix ..." is a firm Change, and
//! the fake claude's reply has no link: the check asks for exactly one continuation.

mod common;

use common::*;
use kwnt::server::routes;
use std::path::Path;
use std::time::Duration;

/// The start of kwant's continuation prompt for a Change with no link (`LINK_PROMPT`).
const CONTINUATION: &str = "your reply has no link to it";

/// The bot's folder made a git repository, so the person's "Fix ..." is a Change.
fn in_a_repo(env: &Env, kwant: &str) {
    let ws = kwnt::tools::bot_workspace(&env.tmp.path().join("data"), kwant);
    std::fs::create_dir_all(ws.join(".git")).unwrap();
}

/// The request a CLI run answers: its prompt's current request, without the chat history (which
/// quotes earlier messages and kwant's continuation prompt).
fn request_of(args: &str) -> &str {
    let prompt = args.rsplit_once("\n-p\n").map_or(args, |(_, p)| p);
    prompt.rsplit_once("### Current User Request:\n").map_or(prompt, |(_, r)| r)
}

/// The fake names each record `<pid>-<time_ns>`, written as the CLI starts.
fn started_at(path: &Path) -> Option<u128> {
    path.file_name()?.to_str()?.split_once('-')?.1.parse().ok()
}

/// The CLI runs of `env`'s server (their arguments name its data folder), oldest first: (start
/// time, the request each answers). The test binary's other tests run beside it.
fn runs(env: &Env) -> Vec<(u128, String)> {
    let data = env.tmp.path().display().to_string();
    let mut out: Vec<(u128, String)> = std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| {
            let path = e.ok()?.path();
            let started = started_at(&path)?;
            let args = std::fs::read_to_string(&path).ok()?;
            args.contains(&data).then(|| (started, request_of(&args).to_string()))
        })
        .collect();
    out.sort();
    out
}

/// Start times of the runs whose request contains `words`.
fn started(runs: &[(u128, String)], words: &str) -> Vec<u128> {
    runs.iter().filter(|(_, r)| r.contains(words)).map(|(t, _)| *t).collect()
}

fn delivery_meta(m: &kwnt::db::models::Message) -> serde_json::Value {
    let meta: serde_json::Value = m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
    meta["delivery"].clone()
}

fn bot_idle(env: &Env, kwant: &str) -> bool {
    env.state.turns.active_turns().iter().all(|t| t.kwant_id != kwant)
}

#[tokio::test]
async fn the_continuation_runs_on_its_turns_slot_before_a_queued_message() {
    let env = setup().await;
    let a = env.a.clone();
    in_a_repo(&env, &a);

    // A firm Change with no link in the reply, slow enough for a second message to arrive
    send_direct(&env, &a, "Fix the footer spacing. REPLYTAG:zqfirst SLEEPONCE:zqfirst:2500", serde_json::json!({})).await;
    wait_for("the first turn's CLI to start", || async { !started(&runs(&env), "REPLYTAG:zqfirst").is_empty() }).await;
    // The person writes again while it runs: P6 queues it behind the running reply
    send_direct(&env, &a, "Which timezone do you use? REPLYTAG:zqsecond", serde_json::json!({})).await;
    wait_for("the second message to queue", || async {
        env.state.turns.active_turns().iter().any(|t| t.kwant_id == a && t.state == "queued")
    })
    .await;

    // The first reply, its continuation and the answer to the second message, in that order
    let replies = || async {
        let msgs = messages(&env, &a).await;
        msgs.into_iter().filter(|m| m.sender_role == "kwant").collect::<Vec<_>>()
    };
    wait_for("three replies", || async { replies().await.len() >= 3 }).await;
    wait_for("the bot to finish", || async { bot_idle(&env, &a) }).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let replies = replies().await;
    assert_eq!(replies.len(), 3, "{replies:#?}");
    assert_eq!(replies[0].text, "answer zqfirst");
    assert_eq!(delivery_meta(&replies[0])["continues"], true, "a firm Change with no link continues: {:?}", replies[0].meta);

    // Exactly one continuation (neither it nor the "Which timezone" answer is continued), and it
    // started before the queued message's turn. The fake answers the last REPLYTAG it sees, and
    // the continuation's history already holds the second message: both later replies say zqsecond.
    let all = runs(&env);
    let cont = started(&all, CONTINUATION);
    assert_eq!(cont.len(), 1, "exactly one continuation: {all:#?}");
    let second_started = started(&all, "REPLYTAG:zqsecond");
    assert_eq!(second_started.len(), 1, "{all:#?}");
    assert!(cont[0] < second_started[0], "the continuation ran first, never behind the queued message: {all:#?}");
    assert_eq!(all.len(), 3, "the first turn, its continuation, the second message: {all:#?}");
    assert_eq!(delivery_meta(&replies[1])["continues"], false, "the continuation is never continued: {:?}", replies[1].meta);
}

#[tokio::test]
async fn a_stall_retried_try_that_misses_is_continued_once() {
    let env = setup().await;
    let a = env.a.clone();
    in_a_repo(&env, &a);
    env.state.db.set_setting(routes::STALL_SETTING, "1").await.unwrap();

    // The first try goes silent past the 1 s limit (P7 retries it once); the retry answers with no link
    send_direct(&env, &a, "Fix the header spacing. REPLYTAG:zqstall SLEEPONCE:zqstall:4000", serde_json::json!({})).await;
    wait_for("the continuation to run", || async { !started(&runs(&env), CONTINUATION).is_empty() }).await;
    wait_for("the bot to finish", || async { bot_idle(&env, &a) }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let all = runs(&env);
    assert_eq!(started(&all, CONTINUATION).len(), 1, "one continuation: {all:#?}");
    // The stalled try and its retry
    assert_eq!(started(&all, "REPLYTAG:zqstall").len(), 2, "{all:#?}");
    assert_eq!(all.len(), 3, "{all:#?}");
    let msgs = messages(&env, &a).await;
    let first = msgs
        .iter()
        .find(|m| m.sender_role == "kwant" && m.meta.as_deref().is_some_and(|s| s.contains("stall_retried")))
        .unwrap_or_else(|| panic!("no stall-retried reply: {msgs:#?}"));
    assert_eq!(delivery_meta(first)["continues"], true, "{:?}", first.meta);
    assert!(!msgs.iter().any(|m| m.text.contains("sent nothing for")), "no stall stop line after one stall: {msgs:#?}");
}
