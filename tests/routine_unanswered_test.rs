//! 26b: an approval nobody answered is never "ok". A routine run whose only call waits on a card
//! nobody answers ends `needs_you` (its run event says `waiting` while the card is open), its
//! activity step is `unanswered`; a step that failed with no later success ends `incomplete`; a
//! failure fixed by a later call of the same tool stays `ok`; "Run now" sets `last_run_at`; two
//! runs in one chat each read only their own replies (D1).

mod common;

use common::*;
use kwnt::db::spaces::SpaceId;
use kwnt::drivers::events::RuntimeEvent;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

/// A daily routine of bot A in its direct chat (so "Run now" is the only run in a test).
async fn routine(env: &Env, name: &str, instructions: &str) -> String {
    let (s, r) = call(
        &env.app,
        "POST",
        "/api/routines",
        Some(
            json!({"kwant_id": env.a, "name": name, "instructions": instructions,
            "schedule": {"type": "daily", "time": "03:00", "tz": "UTC"}}),
        ),
    )
    .await;
    assert_eq!(s.as_u16(), 201, "{r}");
    r["id"].as_str().unwrap().to_string()
}

async fn routine_row(env: &Env, id: &str) -> Value {
    let (_, list) = call(&env.app, "GET", "/api/routines", None).await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .cloned()
        .expect("routine listed")
}

/// Starts a manual run and waits for it to end; returns the run.
async fn run_now(env: &Env, id: &str) -> Value {
    let (s, run) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s.as_u16(), 202, "{run}");
    let run_id = run["id"].as_str().unwrap().to_string();
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
    find(runs)
}

/// The reply a run saved (its `message_id`), with its meta parsed.
async fn reply_of(env: &Env, run: &Value) -> Value {
    let id = run["message_id"].as_str().expect("the run saved a reply");
    let m = env.state.db.get_message(id).await.unwrap().unwrap();
    m.meta
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_routine_whose_only_call_nobody_answers_needs_you() {
    let env = setup().await;
    env.state
        .db
        .set_space_approvals(&SpaceId::default_space(), "ask")
        .await
        .unwrap();
    env.state.approval_wait_ms.store(400, Ordering::Relaxed);
    // A shell command always asks (kwnt's own tools don't); the fake CLI asks for it as Bash
    let marker = call_tool_marker(
        "bash",
        json!({"command": "touch LIGHTS26B_MARK", "description": "check the lights"}),
    );
    let id = routine(
        &env,
        "Lights",
        &format!("Check the lights {marker} ASKFIRST ASKAS:Bash REPLYTAG:lights26b"),
    )
    .await;
    let before = routine_row(&env, &id).await;
    assert!(before["last_run_at"].is_null(), "{before}");
    let mut events = env.state.event_sender.subscribe();

    let run = run_now(&env, &id).await;
    assert_eq!(run["status"], "needs_you", "{run}");
    assert_eq!(
        run["error"],
        "A step is waiting for your approval, so it didn't run"
    );

    // The card is still open for a late answer, and the run's events said so while it waited
    let waiting = env
        .state
        .db
        .list_approvals(&SpaceId::default_space(), Some("waiting"), Some(&env.a))
        .await
        .unwrap();
    let card = waiting.first().expect("the card stays open").id.clone();
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let RuntimeEvent::RoutineRun {
            run_id,
            status,
            waiting,
            approval_id,
            ..
        } = e
        {
            if run_id == run["id"].as_str().unwrap() {
                seen.push((status, waiting, approval_id));
            }
        }
    }
    let open = seen
        .iter()
        .position(|(s, w, a)| s == "running" && *w && a.as_deref() == Some(card.as_str()));
    let open = open.unwrap_or_else(|| panic!("a waiting event with the card: {seen:?}"));
    // Search only after the waiting event: the run's first "running" event
    // (before any card) also has the flag off.
    let closed = seen
        .iter()
        .skip(open + 1)
        .position(|(s, w, a)| s == "running" && !*w && a.is_none())
        .map(|i| i + open + 1);
    assert!(
        closed.is_some_and(|c| c > open),
        "the flag clears when the wait ends: {seen:?}"
    );
    assert_eq!(
        seen.last().map(|(s, w, _)| (s.as_str(), *w)),
        Some(("needs_you", false)),
        "{seen:?}"
    );

    // Its approval step is `unanswered`, not ok or stopped
    let meta = reply_of(&env, &run).await;
    let steps = meta["activity"]["steps"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let approval = steps
        .iter()
        .find(|s| s["kind"] == "approval")
        .unwrap_or_else(|| panic!("{meta}"));
    assert_eq!(approval["status"], "unanswered", "{approval}");
    // The bot was told once not to claim the step worked
    let text = env
        .state
        .db
        .get_message(run["message_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap()
        .text;
    assert_eq!(
        text.matches(kwnt::mcp::NO_FALSE_CLAIMS_TEXT).count(),
        1,
        "{text}"
    );

    // "Run now" counts as the last run; the next scheduled run stays where it was
    let after = routine_row(&env, &id).await;
    assert!(after["last_run_at"].is_string(), "{after}");
    assert_eq!(after["next_run_at"], before["next_run_at"]);
}

#[tokio::test]
async fn a_step_that_failed_for_good_is_incomplete_and_one_fixed_later_is_ok() {
    let env = setup().await;
    let use_tool = |id: &str, cmd: &str| {
        json!({"type": "assistant", "message": {"id": format!("msg_{id}"), "content": [
        {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": cmd, "description": "fetch the report"}}]}})
    };
    let result = |id: &str, error: bool, text: &str| {
        json!({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": id, "is_error": error, "content": text}]}})
    };

    let failed = stream_lines_marker(&[
        use_tool("toolu_f1", "fetch.sh"),
        result("toolu_f1", true, "exit 7: the report server said 502"),
    ]);
    let id = routine(
        &env,
        "Report",
        &format!("Fetch the report {failed} REPLYTAG:report26b"),
    )
    .await;
    let run = run_now(&env, &id).await;
    assert_eq!(run["status"], "incomplete", "{run}");
    assert_eq!(run["error"], "exit 7: the report server said 502");

    let fixed = stream_lines_marker(&[
        use_tool("toolu_g1", "fetch.sh"),
        result("toolu_g1", true, "exit 7: 502"),
        use_tool("toolu_g2", "fetch.sh --retry"),
        result("toolu_g2", false, "report.pdf"),
    ]);
    let id = routine(
        &env,
        "Report again",
        &format!("Fetch the report {fixed} REPLYTAG:again26b"),
    )
    .await;
    let run = run_now(&env, &id).await;
    assert_eq!(run["status"], "ok", "{run}");
    assert!(run["error"].is_null(), "{run}");
}

/// 26b D1 (Rigel, QA at 6923ad4b): two routines of one bot deliver to the same chat at the same
/// time. The one whose step failed ends `incomplete`; the plain one, whose turn ran no tool,
/// ends `ok`: a run reads only its own replies, never the other run's.
#[tokio::test]
async fn two_runs_in_one_chat_each_read_only_their_own_steps() {
    let env = setup().await;
    let failed = stream_lines_marker(&[
        json!({"type": "assistant", "message": {"id": "msg_s1", "content": [
        {"type": "tool_use", "id": "toolu_s1", "name": "Bash", "input": {"command": "fetch.sh", "description": "fetch the report"}}]}}),
        json!({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "toolu_s1", "is_error": true, "content": "exit 7: the report server said 502"}]}}),
    ]);
    // The failing one is slow, so the plain one's message is posted while its turn still runs
    let fails = routine(
        &env,
        "Sibling fails",
        &format!("Fetch the report {failed} SLEEPONCE:sib26b:2000 REPLYTAG:sibfail26b"),
    )
    .await;
    let plain = routine(
        &env,
        "Sibling plain",
        "Say hi, no tools. REPLYTAG:sibplain26b",
    )
    .await;
    let (fails_run, plain_run) = tokio::join!(run_now(&env, &fails), async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        run_now(&env, &plain).await
    });

    assert_eq!(fails_run["status"], "incomplete", "{fails_run}");
    assert_eq!(fails_run["error"], "exit 7: the report server said 502");
    assert_eq!(plain_run["status"], "ok", "{plain_run}");
    assert!(plain_run["error"].is_null(), "{plain_run}");

    // Each run's reply is its own, and says which run it is
    for (run, tag) in [
        (&fails_run, "answer sibfail26b"),
        (&plain_run, "answer sibplain26b"),
    ] {
        let id = run["message_id"].as_str().expect("the run saved a reply");
        let reply = env.state.db.get_message(id).await.unwrap().unwrap();
        assert!(reply.text.contains(tag), "{run}: {}", reply.text);
        assert_eq!(reply_of(&env, run).await["routine_run"], run["id"], "{run}");
    }

#[tokio::test]
async fn a_second_run_while_one_is_going_is_still_skipped() {
    let env = setup().await;
    let id = routine(
        &env,
        "Slow",
        "Take your time SLEEPONCE:slow26b:1500 REPLYTAG:slow26b",
    )
    .await;
    let (s, first) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(
        (s.as_u16(), first["status"].as_str()),
        (202, Some("running")),
        "{first}"
    );
    let (s, second) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s.as_u16(), 202);
    assert_eq!(
        (second["status"].as_str(), second["error"].as_str()),
        (Some("skipped"), Some("the previous run was still going"))
    );
    wait_reply(&env, &env.a, "answer slow26b").await;
}
