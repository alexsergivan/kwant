//! Human steps: `request_human_step` pauses the turn with a card and a `human_step`
//! notification, Done or Decline resumes it with that outcome, Stop cancels it, a late answer
//! starts a new turn, and the bot's browser tools are refused while a step is open.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::RuntimeEvent;
use serde_json::{json, Value};
use std::time::Duration;

fn step_marker(kind: &str, reason: &str) -> String {
    call_tool_marker("request_human_step", json!({"kind": kind, "reason": reason, "url": "https://me:pw@example.com/login?token=abc"}))
}

/// Waits until `ctx` has an open step, and returns it.
async fn open_step(env: &Env, ctx: &str) -> Value {
    for _ in 0..300 {
        let (s, list) = call(&env.app, "GET", &format!("/api/human-steps?status=open&context_id={ctx}"), None).await;
        assert_eq!(s, StatusCode::OK);
        if let Some(step) = list.as_array().and_then(|l| l.first()) {
            return step.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no open human step in {ctx}");
}

async fn step(env: &Env, id: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/human-steps/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    v
}

#[tokio::test]
async fn done_resumes_the_waiting_turn() {
    let env = setup().await;
    let mut rx = env.state.event_sender.subscribe();
    send_direct(&env, &env.a, &format!("Open the PR {} REPLYTAG:h1", step_marker("sign_in", "Log in to Example")), json!({})).await;
    let s = open_step(&env, &env.a).await;
    let id = s["id"].as_str().unwrap().to_string();
    assert_eq!(s["kind"], "sign_in");
    assert_eq!(s["source"], "bot");
    assert_eq!(s["url"], "https://example.com/login", "no password or query kept");
    assert!(step(&env, &id).await["waiting"].as_bool().unwrap());

    let card = messages(&env, &env.a).await.into_iter().find(|m| m.text.starts_with("Sign in needed")).unwrap();
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!((meta["kind"].as_str(), meta["human_step_id"].as_str(), meta["status"].as_str()), (Some("human_step"), Some(id.as_str()), Some("open")));
    assert!(!card.text.contains("pw") && !card.text.contains("token"));

    // The bot's browser is the person's while the step is open
    if let Some(target) = s["sandbox_target"].as_str() {
        assert!(kwnt::server::human_steps::browser_is_held(&env.state, target).await);
    }

    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{id}/done"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn");
    let reply = wait_reply(&env, &env.a, "answer h1").await;
    assert!(reply.text.contains("TOOL[The person says the step is done"), "{}", reply.text);
    assert_eq!(step(&env, &id).await["step"]["status"], "done");
    if let Some(target) = s["sandbox_target"].as_str() {
        assert!(!kwnt::server::human_steps::browser_is_held(&env.state, target).await, "handed back");
    }
    let card = messages(&env, &env.a).await.into_iter().find(|m| m.id == card.id).unwrap();
    assert!(card.meta.as_deref().unwrap().contains("\"status\":\"done\""));

    let (mut opened, mut closed, mut notified, mut waiting, mut settled) = (false, false, false, false, false);
    while let Ok(e) = rx.try_recv() {
        match e.unscoped() {
            RuntimeEvent::HumanStepUpdated { step } if step["id"] == id.as_str() => match step["status"].as_str() {
                Some("open") => opened = true,
                Some("done") => closed = true,
                _ => {}
            },
            RuntimeEvent::Notify { kind, body, .. } if kind == "human_step" => {
                assert!(!body.contains("Example") && !body.contains("example.com"), "no reason or URL in the notification");
                notified = true;
            }
            RuntimeEvent::Activity { step } if step["ref_id"] == id.as_str() => {
                assert_eq!(step["kind"], "human_step");
                match step["status"].as_str() {
                    Some("waiting") => waiting = true,
                    Some("done") => settled = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    assert!(opened && closed && notified, "opened {opened} closed {closed} notified {notified}");
    assert!(waiting && settled, "activity waiting {waiting} settled {settled}");

    let (code, _) = call(&env.app, "POST", &format!("/api/human-steps/{id}/decline"), None).await;
    assert_eq!(code, StatusCode::CONFLICT, "closed once only");
}

#[tokio::test]
async fn decline_tells_the_turn_to_carry_on_without_it() {
    let env = setup().await;
    send_direct(&env, &env.a, &format!("{} REPLYTAG:h2", step_marker("captcha", "Solve the check")), json!({})).await;
    let s = open_step(&env, &env.a).await;
    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{}/decline", s["id"].as_str().unwrap()), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["step"]["status"], "declined");
    let reply = wait_reply(&env, &env.a, "answer h2").await;
    assert!(reply.text.contains("declined the step"), "{}", reply.text);
    assert!(!reply.text.contains("TOOLERROR"));
}

#[tokio::test]
async fn stop_cancels_the_step() {
    let env = setup().await;
    send_direct(&env, &env.a, &format!("{} REPLYTAG:h3", step_marker("other", "Approve the 2FA prompt")), json!({})).await;
    let s = open_step(&env, &env.a).await;
    let id = s["id"].as_str().unwrap();
    let (code, v) = call(&env.app, "POST", "/api/chat/cancel", Some(json!({"kwant_id": env.a}))).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    wait_for("step cancelled", || async { step(&env, id).await["step"]["status"] == "cancelled" }).await;
    let (code, _) = call(&env.app, "POST", &format!("/api/human-steps/{id}/done"), None).await;
    assert_eq!(code, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_late_done_starts_a_new_turn() {
    let env = setup().await;
    env.state.human_steps.set_wait(Duration::from_millis(300));
    send_direct(&env, &env.a, &format!("{} REPLYTAG:h4", step_marker("sign_in", "LATE_STEP log in")), json!({})).await;
    let reply = wait_reply(&env, &env.a, "answer h4").await;
    assert!(reply.text.contains("hasn't done the step yet"), "{}", reply.text);
    let s = open_step(&env, &env.a).await;
    let id = s["id"].as_str().unwrap();
    assert_eq!(step(&env, id).await["waiting"], false);
    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{id}/done"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "new_turn");
    wait_for("the resumed turn", || async { !invocations_with("Step: LATE_STEP log in").is_empty() }).await;
    assert_eq!(invocations_with("Step: LATE_STEP log in").len(), 1);
}

#[tokio::test]
async fn input_and_routes_are_checked() {
    let env = setup().await;
    for (m, u) in [("GET", "/api/human-steps/nope"), ("POST", "/api/human-steps/nope/done"), ("POST", "/api/human-steps/nope/decline")] {
        let (s, _) = call(&env.app, m, u, None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} {u}");
    }
    let (s, _) = call(&env.app, "GET", "/api/human-steps?status=bogus", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    for (tag, args) in [
        ("v1", json!({"kind": "login", "reason": "x"})),
        ("v2", json!({"kind": "sign_in", "reason": " "})),
        ("v3", json!({"kind": "other", "reason": "x", "url": "file:///etc/passwd"})),
    ] {
        send_direct(&env, &env.b, &format!("{} REPLYTAG:{tag}", call_tool_marker("request_human_step", args)), json!({})).await;
        let r = wait_reply(&env, &env.b, &format!("answer {tag}")).await;
        assert!(r.text.contains("TOOLERROR"), "{}", r.text);
    }
    let (_, list) = call(&env.app, "GET", &format!("/api/human-steps?context_id={}", env.b), None).await;
    assert!(list.as_array().unwrap().is_empty());
    for (m, u) in [
        ("GET", "/api/human-steps"),
        ("GET", "/api/human-steps/x"),
        ("POST", "/api/human-steps/x/done"),
        ("POST", "/api/human-steps/x/decline"),
    ] {
        assert_eq!(call_remote(&env.app, m, u).await, StatusCode::UNAUTHORIZED, "{m} {u}");
    }
}
