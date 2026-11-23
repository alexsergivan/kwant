//! unanswered-question-not-ok: a question nobody answered isn't "OK". A routine run whose bot asks
//! (`ask_user`) and gets no answer before the wait ends ends `needs_you` with the question's own
//! error, and the bot is told exactly what it was told before. The Inbox lists the open question
//! once, in Needs you, and nothing for the run: not in Done and not as a second Needs-you item;
//! answering the question later leaves nothing behind for the run. A question answered in time
//! keeps its run `ok`. The approval-card path (26b) is covered in `routine_unanswered_test.rs`.

mod common;

use common::*;
use serde_json::{json, Value};
use std::time::Duration;

const QUESTION_ERROR: &str = "A step is waiting for your answer, so it didn't finish";
/// What `tool_ask` tells the bot when nobody answered (unchanged by this item).
const NOT_ANSWERED_YET: &str = "The user hasn't answered yet. Don't ask again: end your turn now with a short note on what you're waiting for.";

fn ask_marker(question: &str, choices: &[&str]) -> String {
    call_tool_marker(
        "ask_user",
        json!({"question": question, "choices": choices, "why_you": "irreversible", "checked": "Read last week's digest."}),
    )
}

/// A daily routine of bot A in its direct chat (so "Run now" is the only run in a test).
async fn routine(env: &Env, name: &str, instructions: &str) -> String {
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(json!({"kwant_id": env.a, "name": name, "instructions": instructions,
            "schedule": {"type": "daily", "time": "03:00", "tz": "UTC"}})),
    )
    .await;
    assert_eq!(s.as_u16(), 201, "{r}");
    r["id"].as_str().unwrap().to_string()
}

async fn start(env: &Env, id: &str) -> String {
    let (s, run) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s.as_u16(), 202, "{run}");
    run["id"].as_str().unwrap().to_string()
}

/// Waits for run `run_id` of routine `id` to end, and returns it.
async fn ended(env: &Env, id: &str, run_id: &str) -> Value {
    let find = |runs: Value| runs.as_array().unwrap().iter().find(|r| r["id"] == run_id).cloned().unwrap_or(Value::Null);
    wait_for("the run to end", || async {
        let (_, runs) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
        let run = find(runs);
        !run.is_null() && run["status"] != "running"
    })
    .await;
    let (_, runs) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
    find(runs)
}

/// The open question in bot A's direct chat, once there is one.
async fn open_question(env: &Env) -> Value {
    for _ in 0..300 {
        let (_, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={}", env.a), None).await;
        if let Some(q) = list.as_array().and_then(|l| l.first()) {
            return q.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no open question");
}

/// Whether `/api/away` or `/api/inbox` lists run `run_id` anywhere (Done, Failed, Needs you).
async fn lists_run(env: &Env, run_id: &str) -> (bool, bool) {
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    (away.to_string().contains(run_id), inbox.to_string().contains(run_id))
}

#[tokio::test]
async fn a_routine_whose_question_nobody_answers_needs_you_and_isnt_done() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    let ask = ask_marker("Which one should I check first, EU or US?", &["EU", "US"]);
    let id = routine(&env, "Digest", &format!("Post the weekly digest {ask} REPLYTAG:digestq")).await;

    let run_id = start(&env, &id).await;
    let run = ended(&env, &id, &run_id).await;
    // Nothing was checked: the run waits on the person, in its own words
    assert_eq!(run["status"], "needs_you", "{run}");
    assert_eq!(run["error"], QUESTION_ERROR, "{run}");

    // The bot was told exactly what it was told before
    let reply = env.state.db.get_message(run["message_id"].as_str().expect("the run saved a reply")).await.unwrap().unwrap();
    assert!(reply.text.contains(NOT_ANSWERED_YET), "{}", reply.text);

    // The Inbox: the open question once, in Needs you; nothing for the run (not in Done either)
    let q = open_question(&env).await;
    let qid = q["id"].as_str().unwrap().to_string();
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    let waiting = away["waiting"].as_array().unwrap();
    assert_eq!(waiting.len(), 1, "{away}");
    assert_eq!((waiting[0]["kind"].as_str(), waiting[0]["id"].as_str()), (Some("question"), Some(qid.as_str())), "{away}");
    assert!(!away["finished"].to_string().contains(&run_id), "not in Done: {away}");
    assert!(!away["failed"].to_string().contains(&run_id), "no second Needs-you item: {away}");
    assert_eq!(lists_run(&env, &run_id).await, (false, false));
    assert_eq!(away["unseen"]["finished"], 0, "{away}");
    assert_eq!(away["unseen"]["failed"], 0, "{away}");

    // A late answer closes the question and leaves nothing behind for the run
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid}/answer"), Some(json!({"text": "EU"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    assert!(away["waiting"].as_array().unwrap().iter().all(|w| w["id"] != qid.as_str()), "{away}");
    assert_eq!(lists_run(&env, &run_id).await, (false, false));
}

#[tokio::test]
async fn a_routine_whose_question_is_answered_in_time_stays_ok() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_secs(30));
    // Its own question: the fake CLI runs each CALLTOOL marker once per test binary (a `called-<sha1>`
    // file next to the shared prompts folder), so the same marker as the test above would never ask
    let ask = ask_marker("Which market should the digest open with, EU or US?", &["EU", "US"]);
    let id = routine(&env, "Digest", &format!("Post the weekly digest {ask} REPLYTAG:digesta")).await;

    let run_id = start(&env, &id).await;
    let q = open_question(&env).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{}/answer", q["id"].as_str().unwrap()), Some(json!({"text": "EU"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let run = ended(&env, &id, &run_id).await;
    assert_eq!(run["status"], "ok", "{run}");
    assert!(run["error"].is_null(), "{run}");
}
