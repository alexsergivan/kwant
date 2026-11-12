//! Human steps: `request_human_step` pauses the turn with a card and a `human_step`
//! notification, Done or Decline resumes it with that outcome, Stop cancels it, a late answer
//! starts a new turn, and the bot's browser tools are refused while a step is open. Two turns of
//! a bot in one chat asking for the same sign-in share one step.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::mcp_runtime;
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

    // The bot's browser page (the job's own, 0.9.3) is the person's while the step is open
    if let Some(target) = s["page_key"].as_str().or(s["sandbox_target"].as_str()) {
        assert!(kwnt::server::human_steps::browser_is_held(&env.state, target).await);
    }

    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{id}/done"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn");
    let reply = wait_reply(&env, &env.a, "answer h1").await;
    assert!(reply.text.contains("TOOL[The person says the step is done"), "{}", reply.text);
    assert_eq!(step(&env, &id).await["step"]["status"], "done");
    if let Some(target) = s["page_key"].as_str().or(s["sandbox_target"].as_str()) {
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

/// A turn of bot `a` in its direct chat (the person's, or one a closed step started), as the CLI
/// sees it: its token for kwnt's self tools.
async fn turn_token(env: &Env, turn_id: &str) -> String {
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some(turn_id.into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&env.state, &bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

/// The turn asks for a sign-in at `url`, in the background as the CLI would while it waits;
/// gives the tool's text and whether it was an error.
fn ask_sign_in(env: &Env, token: &str, url: &str) -> tokio::task::JoinHandle<(String, bool)> {
    let (app, auth) = (env.app.clone(), format!("Bearer {token}"));
    let args = json!({"kind": "sign_in", "reason": "Please sign in to Instagram in the browser panel", "url": url});
    tokio::spawn(async move {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "request_human_step", "arguments": args}});
        let (code, v) = call_with(&app, "POST", "/api/mcp/self", Some(body), &[("authorization", auth.as_str())]).await;
        assert_eq!(code, StatusCode::OK, "{v}");
        (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
    })
}

async fn finished(task: tokio::task::JoinHandle<(String, bool)>) -> (String, bool) {
    tokio::time::timeout(Duration::from_secs(20), task).await.expect("the tool call never hangs").unwrap()
}

async fn sign_in_cards(env: &Env) -> usize {
    messages(env, &env.a).await.iter().filter(|m| m.text.starts_with("Sign in needed")).count()
}

/// Rigel's QA of #269: the person's turn and a turn a closed CAPTCHA step started both asked for
/// the Instagram sign-in, and the person got two identical cards.
#[tokio::test]
async fn a_second_turn_asking_for_the_same_sign_in_steps_aside_and_the_first_hears_the_answer() {
    let env = setup().await;
    let first = ask_sign_in(&env, &turn_token(&env, "turn-person").await, "https://www.instagram.com/accounts/login/");
    let s = open_step(&env, &env.a).await;
    let id = s["id"].as_str().unwrap().to_string();
    wait_for("the first turn waiting", || async { step(&env, &id).await["waiting"] == true }).await;

    // Same site, another page of it, another turn of the same bot in the same chat
    let second = ask_sign_in(&env, &turn_token(&env, "turn-resumed").await, "https://www.instagram.com/?next=x");
    let (text, is_error) = finished(second).await;
    assert!(!is_error, "{text}");
    assert!(text.contains("already asked the person for this sign-in") && text.contains("Don't ask again"), "{text}");
    let (_, open) = call(&env.app, "GET", &format!("/api/human-steps?status=open&context_id={}", env.a), None).await;
    assert_eq!(open.as_array().unwrap().len(), 1, "one step: {open}");
    assert_eq!(sign_in_cards(&env).await, 1, "one card");
    assert_eq!(step(&env, &id).await["waiting"], true, "the first turn still waits on it");

    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{id}/decline"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn");
    let (text, is_error) = finished(first).await;
    assert!(!is_error, "{text}");
    assert!(text.contains("declined the step") && text.contains("Don't ask for it again in this turn unless the person brings it up"), "{text}");
}

/// A sign-in no turn waits on any more (the asking turn's wait ran out) is picked up by the next
/// turn that asks for the same site: it waits on that step and hears the answer.
#[tokio::test]
async fn a_turn_asking_for_a_sign_in_nobody_waits_on_any_more_waits_on_that_step() {
    let env = setup().await;
    env.state.human_steps.set_wait(Duration::from_millis(300));
    let (text, _) = finished(ask_sign_in(&env, &turn_token(&env, "turn-early").await, "https://www.instagram.com/accounts/login/")).await;
    assert!(text.contains("hasn't done the step yet"), "{text}");
    let s = open_step(&env, &env.a).await;
    let id = s["id"].as_str().unwrap().to_string();
    assert_eq!(step(&env, &id).await["waiting"], false);

    env.state.human_steps.set_wait(Duration::from_secs(30));
    let later = ask_sign_in(&env, &turn_token(&env, "turn-later").await, "https://www.instagram.com/accounts/login/");
    wait_for("the later turn waiting", || async { step(&env, &id).await["waiting"] == true }).await;
    assert_eq!(sign_in_cards(&env).await, 1, "no second card");
    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{id}/done"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn");
    let (text, is_error) = finished(later).await;
    assert!(!is_error && text.contains("The person says the step is done"), "{text}");
}

#[tokio::test]
async fn a_late_decline_tells_the_new_turn_not_to_ask_again() {
    let env = setup().await;
    env.state.human_steps.set_wait(Duration::from_millis(300));
    send_direct(&env, &env.a, &format!("{} REPLYTAG:h5", step_marker("sign_in", "LATE_DECLINE log in")), json!({})).await;
    wait_reply(&env, &env.a, "answer h5").await;
    let s = open_step(&env, &env.a).await;
    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{}/decline", s["id"].as_str().unwrap()), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "new_turn");
    wait_for("the resumed turn", || async { !invocations_with("Step: LATE_DECLINE log in").is_empty() }).await;
    let prompt = invocations_with("Step: LATE_DECLINE log in").remove(0);
    assert!(prompt.contains("declined the browser step") && prompt.contains("Don't ask for it again unless the person brings it up"), "{prompt}");
}

/// Rigel's QA of #273: the dedupe is per site. While the bot's Instagram sign-in is open and
/// waited on, another turn of the same bot in the same chat asking for TikTok gets its own step
/// and card, waits on it, and neither answer reaches the other turn.
#[tokio::test]
async fn a_sign_in_for_another_site_gets_its_own_step_while_one_is_open() {
    let env = setup().await;
    let instagram = ask_sign_in(&env, &turn_token(&env, "turn-person").await, "https://www.instagram.com/accounts/login/");
    let s = open_step(&env, &env.a).await;
    let ig = s["id"].as_str().unwrap().to_string();
    wait_for("the Instagram turn waiting", || async { step(&env, &ig).await["waiting"] == true }).await;

    let tiktok = ask_sign_in(&env, &turn_token(&env, "turn-resumed").await, "https://www.tiktok.com/login");
    let open_steps = || async {
        let (_, open) = call(&env.app, "GET", &format!("/api/human-steps?status=open&context_id={}", env.a), None).await;
        open.as_array().unwrap().clone()
    };
    wait_for("a second open step", || async { open_steps().await.len() == 2 }).await;
    let tt = open_steps().await.into_iter().find(|s| s["id"] != ig.as_str()).unwrap();
    let tt_id = tt["id"].as_str().unwrap().to_string();
    assert!(tt["url"].as_str().unwrap_or("").starts_with("https://www.tiktok.com/"), "{tt}");
    wait_for("the TikTok turn waiting", || async { step(&env, &tt_id).await["waiting"] == true }).await;
    assert!(!tiktok.is_finished(), "the TikTok ask was not stepped aside as AlreadyAsked");
    assert_eq!(sign_in_cards(&env).await, 2, "one card per site");

    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{tt_id}/done"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "turn");
    let (text, is_error) = finished(tiktok).await;
    assert!(!is_error && text.contains("The person says the step is done"), "{text}");
    assert!(!text.contains("already asked"), "{text}");
    assert_eq!(step(&env, &ig).await["step"]["status"], "open", "Instagram is still open");
    assert!(!instagram.is_finished(), "the TikTok answer did not reach the Instagram turn");

    let (code, v) = call(&env.app, "POST", &format!("/api/human-steps/{ig}/decline"), None).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    let (text, is_error) = finished(instagram).await;
    assert!(!is_error && text.contains("declined the step"), "{text}");
}
