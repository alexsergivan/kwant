//! routine-answered-later (Mira's UX pass 0.9.45, Minor i): a routine run whose question nobody
//! answered ends `needs_you` with the question's own error (unanswered-question-not-ok, #1112).
//! When the person answers that question later (on its card or by typing in the chat), the run's
//! History row says so: its API row carries `answered_later: true` (and the routine's
//! `last_answered_later`, for the card's header badge). Nothing else about the run changes: its
//! status stays `needs_you`, its error stays the question error, and it stays out of the Inbox and
//! the unseen counts. A dismissed question changes nothing. Two runs in one chat each keep their own.
//!
//! Every test asks its own question: the fake CLI runs each CALLTOOL marker once per test binary.

mod common;

use common::*;
use serde_json::{json, Value};
use std::time::Duration;

const QUESTION_ERROR: &str = "A step is waiting for your answer, so it didn't finish";

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

/// Run `run_id` of routine `id` as `/api/routines/{id}/runs` lists it now.
async fn run_row(env: &Env, id: &str, run_id: &str) -> Value {
    let (_, runs) = call(&env.app, "GET", &format!("/api/routines/{id}/runs"), None).await;
    runs.as_array().unwrap().iter().find(|r| r["id"] == run_id).cloned().unwrap_or(Value::Null)
}

/// Waits for run `run_id` of routine `id` to end, and returns it.
async fn ended(env: &Env, id: &str, run_id: &str) -> Value {
    wait_for("the run to end", || async {
        let run = run_row(env, id, run_id).await;
        !run.is_null() && run["status"] != "running"
    })
    .await;
    run_row(env, id, run_id).await
}

/// Routine `id` as `/api/routines` lists it (its header badge reads `last_*`).
async fn routine_row(env: &Env, id: &str) -> Value {
    let (_, list) = call(&env.app, "GET", "/api/routines", None).await;
    let list = list.as_array().cloned().or_else(|| list["routines"].as_array().cloned()).unwrap_or_default();
    list.into_iter().find(|r| r["id"] == id).unwrap_or(Value::Null)
}

/// The open question in bot A's direct chat asking `text`, once there is one.
async fn open_question(env: &Env, text: &str) -> String {
    for _ in 0..300 {
        let (_, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={}", env.a), None).await;
        if let Some(q) = list.as_array().and_then(|l| l.iter().find(|q| q["question"] == text)) {
            return q["id"].as_str().unwrap().to_string();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no open question {text:?}");
}

/// Whether `/api/away` or `/api/inbox` lists run `run_id` anywhere, and the unseen counts.
async fn inbox_view(env: &Env, run_id: &str) -> (bool, bool, Value) {
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    let (_, inbox) = call(&env.app, "GET", "/api/inbox", None).await;
    (away.to_string().contains(run_id), inbox.to_string().contains(run_id), away["unseen"].clone())
}

/// A run that ended waiting on question `question` (asked with nobody answering).
async fn question_run(env: &Env, name: &str, question: &str, tag: &str) -> (String, String, String) {
    let ask = ask_marker(question, &["EU", "US"]);
    let id = routine(env, name, &format!("Post the weekly digest {ask} REPLYTAG:{tag}")).await;
    let run_id = start(env, &id).await;
    let run = ended(env, &id, &run_id).await;
    assert_eq!((run["status"].as_str(), run["error"].as_str()), (Some("needs_you"), Some(QUESTION_ERROR)), "{run}");
    let qid = open_question(env, question).await;
    (id, run_id, qid)
}

#[tokio::test]
async fn a_late_answer_marks_the_run_answered_later_and_nothing_else() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    let (id, run_id, qid) = question_run(&env, "Digest", "Which region should the late digest cover first, EU or US?", "lateq1").await;

    // Before the answer: not answered later, on the row and on the routine's header badge
    let run = run_row(&env, &id, &run_id).await;
    assert_eq!(run["answered_later"], false, "{run}");
    assert_eq!(routine_row(&env, &id).await["last_answered_later"], false);
    let (in_away, in_inbox, unseen) = inbox_view(&env, &run_id).await;
    assert_eq!((in_away, in_inbox), (false, false));

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid}/answer"), Some(json!({"text": "EU"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["resumed"], "new_turn", "a late answer: {v}");

    // The run says it was answered later; its status and error stay as they were
    let run = run_row(&env, &id, &run_id).await;
    assert_eq!(run["answered_later"], true, "{run}");
    assert_eq!((run["status"].as_str(), run["error"].as_str()), (Some("needs_you"), Some(QUESTION_ERROR)), "{run}");
    let r = routine_row(&env, &id).await;
    assert_eq!((r["last_status"].as_str(), r["last_answered_later"].as_bool()), (Some("needs_you"), Some(true)), "{r}");
    // Still nowhere in the Inbox, and the unseen counts didn't move
    let (in_away, in_inbox, after) = inbox_view(&env, &run_id).await;
    assert_eq!((in_away, in_inbox), (false, false));
    assert_eq!(after["finished"], unseen["finished"], "{after}");
    assert_eq!(after["failed"], unseen["failed"], "{after}");
}

#[tokio::test]
async fn a_dismissed_question_leaves_the_run_as_it_was() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    let (id, run_id, qid) = question_run(&env, "Digest", "Which region should the dismissed digest cover first, EU or US?", "lateq2").await;

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid}/cancel"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let run = run_row(&env, &id, &run_id).await;
    assert_eq!(run["answered_later"], false, "{run}");
    assert_eq!((run["status"].as_str(), run["error"].as_str()), (Some("needs_you"), Some(QUESTION_ERROR)), "{run}");
    assert_eq!(routine_row(&env, &id).await["last_answered_later"], false);
    assert_eq!(&inbox_view(&env, &run_id).await.0, &false);
}

#[tokio::test]
async fn an_answer_typed_in_the_chat_later_marks_the_run_too() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    let (id, run_id, qid) = question_run(&env, "Digest", "Which region should the typed digest cover first, EU or US?", "lateq3").await;

    // Typed instead of tapped: no turn waits any more, so the message closes the question and goes
    // on to the bot as a new turn
    send_direct(&env, &env.a, "TYPED_LATE EU first, please", json!({})).await;
    let (_, q) = call(&env.app, "GET", &format!("/api/questions/{qid}"), None).await;
    assert_eq!(q["question"]["status"], "answered", "{q}");
    let run = run_row(&env, &id, &run_id).await;
    assert_eq!(run["answered_later"], true, "{run}");
    assert_eq!((run["status"].as_str(), run["error"].as_str()), (Some("needs_you"), Some(QUESTION_ERROR)), "{run}");
}

#[tokio::test]
async fn two_runs_in_one_chat_each_keep_their_own_answer() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    let q1 = "Which region should the first two-run digest cover, EU or US?";
    let q2 = "Which region should the second two-run digest cover, EU or US?";
    let (id1, run1, qid1) = question_run(&env, "Digest one", q1, "lateq4a").await;
    let (id2, run2, qid2) = question_run(&env, "Digest two", q2, "lateq4b").await;
    assert_ne!(qid1, qid2);

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid1}/answer"), Some(json!({"choice": 1}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(run_row(&env, &id1, &run1).await["answered_later"], true);
    assert_eq!(run_row(&env, &id2, &run2).await["answered_later"], false, "the other run's question is still open");

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid2}/answer"), Some(json!({"text": "US"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(run_row(&env, &id2, &run2).await["answered_later"], true);
}
