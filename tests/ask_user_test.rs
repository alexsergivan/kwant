//! One-tap questions (batch 2 / C): `ask_user` pauses the turn with a question card, an answer
//! resumes it, Stop still works, a late answer starts a new turn, and the agent's other chats keep
//! running while it waits.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::server::continuity::SelfScope;
use kwnt::RuntimeEvent;
use serde_json::{json, Value};
use std::time::Duration;

fn ask_marker(question: &str, choices: &[&str]) -> String {
    call_tool_marker(
        "ask_user",
        json!({"question": question, "choices": choices, "why_you": "irreversible", "checked": "Read the release notes."}),
    )
}

/// Waits until `ctx` has an open question, and returns it.
async fn open_question(env: &Env, ctx: &str) -> Value {
    for _ in 0..300 {
        let (s, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={ctx}"), None).await;
        assert_eq!(s, StatusCode::OK);
        if let Some(q) = list.as_array().and_then(|l| l.first()) {
            return q.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no open question in {ctx}");
}

async fn question(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/questions/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    v
}

#[tokio::test]
async fn ask_pauses_the_turn_and_a_choice_resumes_it() {
    let env = setup().await;
    let mut rx = env.state.event_sender.subscribe();
    send_direct(&env, &env.a, &format!("Ship it? {} REPLYTAG:q1", ask_marker("Which branch should I ship?", &["main", "batch2"])), json!({})).await;
    let q = open_question(&env, &env.a).await;
    let id = q["id"].as_str().unwrap().to_string();
    assert_eq!(q["choices"], json!(["main", "batch2"]));
    assert_eq!(q["kwant_id"], env.a.as_str());
    assert!(question(&env, &id).await["waiting"].as_bool().unwrap(), "the turn is waiting");

    // The card is a chat message with the question in meta
    let card = messages(&env, &env.a).await.into_iter().find(|m| m.text.starts_with("Which branch")).unwrap();
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["kind"], "ask_user");
    assert_eq!(meta["question_id"], id.as_str());
    assert_eq!(meta["status"], "open");
    assert_eq!((meta["why_you"].as_str(), meta["checked"].as_str()), (Some("irreversible"), Some("Read the release notes.")));
    assert_eq!(q["why_you"], "irreversible");

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{id}/answer"), Some(json!({"choice": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn");
    let reply = wait_reply(&env, &env.a, "answer q1").await;
    assert!(reply.text.contains("TOOL[The user answered: batch2]"), "{}", reply.text);

    // Stored: answered, the answer is a user message, the card shows it
    let q = &question(&env, &id).await["question"];
    assert_eq!(q["status"], "answered");
    assert_eq!(q["answer"], "batch2");
    let msgs = messages(&env, &env.a).await;
    let ans = msgs.iter().find(|m| m.meta.as_deref().is_some_and(|x| x.contains("question_answer"))).unwrap();
    assert_eq!((ans.sender_role.as_str(), ans.text.as_str()), ("user", "batch2"));
    let card = msgs.iter().find(|m| m.id == card.id).unwrap();
    assert!(card.meta.as_deref().unwrap().contains("\"status\":\"answered\""));

    let (mut asked, mut answered, mut notified) = (false, false, false);
    while let Ok(e) = rx.try_recv() {
        match e.unscoped() {
            RuntimeEvent::QuestionUpdated { question } if question["id"] == id.as_str() => match question["status"].as_str() {
                Some("open") => asked = true,
                Some("answered") => answered = true,
                _ => {}
            },
            RuntimeEvent::Notify { kind, body, .. } if kind == "question_asked" => {
                assert!(!body.contains("branch"), "notifications don't carry the question text");
                notified = true;
            }
            _ => {}
        }
    }
    assert!(asked && answered && notified);

    // Answering twice is refused
    let (s, _) = call(&env.app, "POST", &format!("/api/questions/{id}/answer"), Some(json!({"choice": 0}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

/// The question, its card and its waiting turn appear together: polled as fast as possible,
/// the first sighting of an open question already has its card message and a live waiter.
#[tokio::test]
async fn a_question_is_never_visible_without_its_card_and_waiter() {
    let env = setup().await;
    send_direct(&env, &env.a, &format!("Go? {} REPLYTAG:r1", ask_marker("Deploy now?", &["yes", "no"])), json!({})).await;
    let q = loop {
        let (s, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={}", env.a), None).await;
        assert_eq!(s, StatusCode::OK);
        if let Some(q) = list.as_array().and_then(|l| l.first()) {
            break q.clone();
        }
        tokio::task::yield_now().await;
    };
    let id = q["id"].as_str().unwrap().to_string();
    let mid = q["message_id"].as_str().unwrap_or_else(|| panic!("open question without its card: {q}")).to_string();
    // Why the bot asked and what it checked are saved with the question, not after it
    assert_eq!((q["why_you"].as_str(), q["checked"].as_str()), (Some("irreversible"), Some("Read the release notes.")), "{q}");
    assert!(question(&env, &id).await["waiting"].as_bool().unwrap(), "open question without a waiting turn");
    assert!(messages(&env, &env.a).await.iter().any(|m| m.id == mid), "card message missing");
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{id}/answer"), Some(json!({"choice": 0}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn", "an immediate answer resumes the waiting turn");
    wait_reply(&env, &env.a, "answer r1").await;
}

#[tokio::test]
async fn the_agent_keeps_working_elsewhere_while_it_waits() {
    let env = setup().await;
    send_direct(&env, &env.a, &format!("{} REPLYTAG:w1", ask_marker("Pick a colour", &["red", "blue"])), json!({})).await;
    let q = open_question(&env, &env.a).await;

    // A thread of the same agent, and the other agent, both answer meanwhile
    let (s, thread) = call(&env.app, "POST", &format!("/api/kwants/{}/threads", env.a), Some(json!({"title": "side"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let tid = thread["id"].as_str().unwrap();
    let (s, _) = call(&env.app, "POST", "/api/chat/send", Some(json!({"prompt": "quick one REPLYTAG:w2", "sender_id": "user", "thread_id": tid}))).await;
    assert_eq!(s, StatusCode::OK);
    wait_reply(&env, tid, "answer w2").await;
    send_direct(&env, &env.b, "disk space? REPLYTAG:w3", json!({})).await;
    wait_reply(&env, &env.b, "answer w3").await;
    assert_eq!(question(&env, q["id"].as_str().unwrap()).await["question"]["status"], "open", "still waiting");

    // Free text answer
    let (s, _) = call(&env.app, "POST", &format!("/api/questions/{}/answer", q["id"].as_str().unwrap()), Some(json!({"text": "  green, actually "}))).await;
    assert_eq!(s, StatusCode::OK);
    let reply = wait_reply(&env, &env.a, "answer w1").await;
    assert!(reply.text.contains("The user answered: green, actually]"), "{}", reply.text);
}

#[tokio::test]
async fn stop_cancels_the_waiting_question() {
    let env = setup().await;
    send_direct(&env, &env.a, &format!("{} REPLYTAG:c1", ask_marker("Delete the old logs?", &["yes", "no"])), json!({})).await;
    let q = open_question(&env, &env.a).await;
    let id = q["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", "/api/chat/cancel", Some(json!({"kwant_id": env.a}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_for("question cancelled", || async { question(&env, id).await["question"]["status"] == "cancelled" }).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/questions/{id}/answer"), Some(json!({"choice": 0}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "a stopped turn's question can't be answered");
    wait_for("turn gone", || async {
        let (_, t) = call(&env.app, "GET", "/api/turns", None).await;
        t["turns"].as_array().unwrap().is_empty()
    })
    .await;
}

#[tokio::test]
async fn dismissing_lets_the_turn_carry_on() {
    let env = setup().await;
    send_direct(&env, &env.a, &format!("{} REPLYTAG:d1", ask_marker("Tabs or spaces?", &["tabs", "spaces"])), json!({})).await;
    let q = open_question(&env, &env.a).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{}/cancel", q["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "cancelled");
    let reply = wait_reply(&env, &env.a, "answer d1").await;
    assert!(reply.text.contains("dismissed the question"), "{}", reply.text);
    assert!(!reply.text.contains("TOOLERROR"));
}

#[tokio::test]
async fn a_late_answer_starts_a_new_turn() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    send_direct(&env, &env.a, &format!("{} REPLYTAG:l1", ask_marker("Which region?", &["eu", "us"])), json!({})).await;
    let reply = wait_reply(&env, &env.a, "answer l1").await;
    assert!(reply.text.contains("hasn't answered yet"), "{}", reply.text);
    let q = open_question(&env, &env.a).await;
    let id = q["id"].as_str().unwrap();
    assert_eq!(question(&env, id).await["waiting"], false);

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{id}/answer"), Some(json!({"text": "LATE_ANSWER_EU"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "new_turn");
    wait_for("the resumed turn", || async { !invocations_with("Answer: LATE_ANSWER_EU").is_empty() }).await;
    let args = invocations_with("Answer: LATE_ANSWER_EU").pop().unwrap();
    assert!(args.contains("Question: Which region?"));
    assert_eq!(invocations_with("Answer: LATE_ANSWER_EU").len(), 1, "one new turn, the tool isn't called again");
}

#[tokio::test]
async fn answers_and_tool_input_are_validated() {
    let env = setup().await;
    let (s, _) = call(&env.app, "POST", "/api/questions/nope/answer", Some(json!({"choice": 0}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "POST", "/api/questions/nope/cancel", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", "/api/questions/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", "/api/questions?status=bogus", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    send_direct(&env, &env.a, &format!("{} REPLYTAG:v1", ask_marker("Ok?", &["yes", "no"])), json!({})).await;
    let q = open_question(&env, &env.a).await;
    let url = format!("/api/questions/{}/answer", q["id"].as_str().unwrap());
    for bad in [json!({"choice": 2}), json!({"text": "   "}), json!({}), json!({"choice": 0, "text": "x"}), json!({"text": "x".repeat(2001)})] {
        let (s, _) = call(&env.app, "POST", &url, Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
    let (s, _) = call(&env.app, "POST", &url, Some(json!({"choice": 0}))).await;
    assert_eq!(s, StatusCode::OK);
    wait_reply(&env, &env.a, "answer v1").await;

    // Bad tool input never creates a question
    for (tag, args) in [
        ("v2", json!({"question": "Only one?", "choices": ["a"]})),
        ("v3", json!({"question": "Six?", "choices": ["a", "b", "c", "d", "e", "f"]})),
        ("v4", json!({"question": "", "choices": ["a", "b"]})),
    ] {
        let args = json!({"why_you": "irreversible", "checked": "x", "question": args["question"], "choices": args["choices"]});
        send_direct(&env, &env.b, &format!("{} REPLYTAG:{tag}", call_tool_marker("ask_user", args)), json!({})).await;
        let r = wait_reply(&env, &env.b, &format!("answer {tag}")).await;
        assert!(r.text.contains("TOOLERROR"), "{}", r.text);
    }
    // Without a reason only the person can answer, the bot is told to decide itself
    for (tag, args) in [
        ("v5", json!({"question": "Which colour?", "choices": ["red", "blue"]})),
        ("v6", json!({"question": "Which colour?", "choices": ["red", "blue"], "why_you": "because", "checked": "x"})),
        ("v7", json!({"question": "Which colour?", "choices": ["red", "blue"], "why_you": "irreversible", "checked": "  "})),
    ] {
        send_direct(&env, &env.b, &format!("{} REPLYTAG:{tag}", call_tool_marker("ask_user", args)), json!({})).await;
        let r = wait_reply(&env, &env.b, &format!("answer {tag}")).await;
        assert!(r.text.contains("TOOLERROR") && r.text.contains("decide it yourself"), "{}", r.text);
    }
    let (_, open) = call(&env.app, "GET", &format!("/api/questions?context_id={}", env.b), None).await;
    assert!(open.as_array().unwrap().is_empty());

    // With decide-first off, a question needs no reason
    call(&env.app, "POST", "/api/settings", Some(json!({"decide_first": false}))).await;
    send_direct(&env, &env.b, &format!("{} REPLYTAG:v8", call_tool_marker("ask_user", json!({"question": "Red?", "choices": ["yes", "no"]}))), json!({})).await;
    let q = open_question(&env, &env.b).await;
    assert_eq!(q["why_you"], Value::Null);

    // Outside a live chat turn the tool isn't offered
    env.state
        .self_tools
        .lock()
        .await
        .insert("tok".into(), SelfScope { kwant_id: env.a.clone(), kwant_name: "A".into(), ..Default::default() });
    let (_, list) = call_with(
        &env.app,
        "POST",
        "/api/mcp/self",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})),
        &[("authorization", "Bearer tok")],
    )
    .await;
    assert!(!list.to_string().contains("ask_user"));
}

#[tokio::test]
async fn every_question_route_needs_auth() {
    let env = setup().await;
    for (m, u) in [
        ("GET", "/api/questions"),
        ("GET", "/api/questions/x"),
        ("POST", "/api/questions/x/answer"),
        ("POST", "/api/questions/x/cancel"),
    ] {
        assert_eq!(call_remote(&env.app, m, u).await, StatusCode::UNAUTHORIZED, "{m} {u}");
    }
}

/// A specialist's question in a delegation thread that the person answers after the wait ran
/// out: the lead hears the question is waiting, and once answered, the specialist continues, its
/// new result is posted where the task came from, and the lead is resumed with it.
#[tokio::test]
async fn a_late_answer_in_a_delegation_reaches_the_lead() {
    let env = setup().await;
    env.state.questions.set_wait(Duration::from_millis(300));
    let task = format!("Pick a region. {} REPLYTAG:asked", ask_marker("Which region for the delegated job?", &["eu", "us"]));
    let (s, d) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({"lead_kwant_id": env.a, "specialist_kwant_id": env.b, "task": task, "title": "Region pick", "kwant_id": env.a})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    let thread = d["thread_id"].as_str().unwrap().to_string();

    // The wait runs out: the lead is told the question is waiting, not to redo the work
    wait_for("the lead's turn", || async { !invocations_with("finished the task you delegated (“Region pick”)").is_empty() }).await;
    let lead = invocations_with("finished the task you delegated (“Region pick”)").pop().unwrap();
    assert!(lead.contains("Which region for the delegated job?") && lead.contains("Don't redo this task"), "{lead}");
    let origin = messages(&env, &env.a).await;
    assert!(origin.iter().any(|m| m.text.contains("is waiting for your answer") && m.text.contains("Which region")), "{origin:?}");

    let q = open_question(&env, &thread).await;
    let id = q["id"].as_str().unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{id}/answer"), Some(json!({"text": "eu REPLYTAG:afteranswer"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "new_turn");

    // The specialist continues in the thread, the result comes back, and the lead gets it
    wait_for("the lead's turn with the late result", || async { !invocations_with("got the person's answer").is_empty() }).await;
    let lead = invocations_with("got the person's answer").pop().unwrap();
    assert!(lead.contains("“Region pick”") && lead.contains("answer afteranswer"), "{lead}");
    let origin = messages(&env, &env.a).await;
    let late = origin.iter().find(|m| m.text == "answer afteranswer").expect("the late result in the origin chat");
    let meta: Value = serde_json::from_str(late.meta.as_deref().unwrap()).unwrap();
    assert_eq!((meta["kind"].as_str(), meta["late"].as_bool()), (Some("delegation_result"), Some(true)));
    assert_eq!(invocations_with("got the person's answer").len(), 1, "the lead is resumed once");
}
