//! 0.9.5 "team from words" over HTTP: `POST /api/teams/suggest {need}` drafts a roster in one
//! model call on the first ready account (the fake claude CLI), retries once when the answer is
//! unusable, keeps the drafted texts on the server, and Create builds the team from that copy
//! with one Undo. `DELETE /api/teams/suggest` stops a running draft. All names are invented.

mod common;

use axum::http::StatusCode;
use common::{call, invocations_with, plan_replies, setup, Env};
use kwnt::db::spaces::SpaceId;
use kwnt::drivers::engines::{EngineId, EngineStatus, Probe};
use serde_json::{json, Value};
use std::time::Duration;

/// Claude's own login is ready on This machine, so the draft has an account to run on.
fn claude_ready(e: &Env) {
    e.state.engines.store(vec![(EngineId::Claude, Probe { status: EngineStatus::Ready, detail: None, version: None, models: Vec::new() })]);
}

fn bot(name: &str, title: &str, lead: bool, preset: &str) -> Value {
    json!({
        "name": name, "title": title, "lead": lead, "preset": preset, "avatar": "product",
        "owns": ["orders"], "avoids": ["refunds"],
        "reason": format!("{title} because you asked for it."),
        "instructions": format!("You are {name}. You ask the person before you send anything."),
    })
}

fn draft(bots: Vec<Value>) -> String {
    json!({ "team": { "name": "Bakery Desk", "description": "Keeps the bakery's orders moving" }, "bots": bots }).to_string()
}

fn good() -> String {
    draft(vec![bot("ShopLead", "Bakery Lead", true, "custom"), bot("OrderKeeper", "Order Keeper", false, "custom"), bot("MailHelper", "Mail Helper", false, "custom")])
}

async fn suggest(e: &Env, need: &str) -> (StatusCode, Value) {
    call(&e.app, "POST", "/api/teams/suggest", Some(json!({ "need": need }))).await
}

fn space() -> SpaceId {
    SpaceId::default_space()
}

#[tokio::test]
async fn a_need_is_drafted_made_from_the_server_copy_and_undone_in_one_step() {
    let e = setup().await;
    claude_ready(&e);
    plan_replies("tfw_e2e", &[&good()]);
    let words = "I run a small bakery and drown in orders and emails. REPLYSEQ:tfw_e2e";
    let (s, sug) = suggest(&e, words).await;
    assert_eq!(s, StatusCode::OK, "{sug}");
    assert_eq!(invocations_with("REPLYSEQ:tfw_e2e").len(), 1);
    // The Suggestion shape, with the need and each row's instructions
    assert_eq!(sug["need"], words);
    assert_eq!(sug["handle"], "bakery-desk");
    assert_eq!(sug["facts"], json!([]));
    assert!(sug["stack_line"].is_null() && sug["partial"].is_null() && sug["team_id"].is_null());
    assert_eq!(sug["fallback"], false);
    assert_eq!(sug["trimmed"], 0);
    assert_eq!((sug["read"]["files"].as_u64(), sug["read"]["manifests"].as_u64()), (Some(0), Some(0)));
    let bots = sug["bots"].as_array().unwrap();
    assert_eq!(bots.len(), 3, "{sug}");
    assert_eq!((bots[0]["key"].as_str(), bots[0]["lead"].as_bool()), (Some("lead"), Some(true)));
    assert!(bots.iter().all(|b| b["preset"] == "custom" && b["owns"] == json!(["orders"])), "{sug}");
    assert!(bots.iter().all(|b| b["reason"].as_str().unwrap().ends_with("because you asked for it.")), "{sug}");
    assert!(bots.iter().all(|b| b["reason_parts"].as_array().is_some_and(|p| !p.is_empty())), "{sug}");
    assert!(bots[1]["instructions"].as_str().unwrap().starts_with("You are OrderKeeper."), "{sug}");
    // The prompt quotes the words as data, with no tools
    let prompt = &invocations_with("REPLYSEQ:tfw_e2e")[0];
    assert!(prompt.contains("<<<WORDS\nI run a small bakery"), "{prompt}");
    assert!(prompt.contains("--tools"), "{prompt}");

    // Create uses the server's copy: the instructions can't be sent in, a preset can't change
    let (s, v) = call(&e.app, "POST", "/api/teams/from-template", Some(json!({ "suggestion_id": sug["suggestion_id"], "edits": { "rows": { "orderkeeper": { "preset": "inbox" } } } }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_row")), "{v}");
    let before = e.state.db.list_kwants(&space()).await.unwrap().len();
    let (s, made) = call(&e.app, "POST", "/api/teams/from-template", Some(json!({ "suggestion_id": sug["suggestion_id"], "edits": { "rows": { "mailhelper": { "name": "Postie" } } } }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["team"]["name"], "bakery-desk");
    let made_bots = made["bots"].as_array().unwrap();
    assert_eq!(made_bots.len(), 3);
    assert_eq!(e.state.db.list_kwants(&space()).await.unwrap().len(), before + 3);
    let postie = made_bots.iter().find(|b| b["name"] == "Postie").expect("the renamed bot");
    let k = e.state.db.get_kwant(postie["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(k.soul, "You are MailHelper. You ask the person before you send anything.");
    assert_eq!(k.title, "Mail Helper");
    let lead = made_bots.iter().find(|b| b["lead"] == true).unwrap();
    let lk = e.state.db.get_kwant(lead["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert!(lk.persona_prompt.contains("ShopLead") && lk.persona_prompt.contains("Bakery Lead"), "{}", lk.persona_prompt);
    let team = e.state.db.list_teams(&space()).await.unwrap().into_iter().find(|t| t.name == "bakery-desk").expect("the team");
    assert_eq!(team.icon, "users");
    assert_eq!(team.description.as_deref(), Some("Keeps the bakery's orders moving"));
    let inst = team.instructions.clone().unwrap_or_default();
    assert!(inst.contains("I run a small bakery"), "{inst}");
    assert!(team.workspace_path.is_none() && team.repo_url.is_none());
    // No routine was drafted
    assert!(made["routine"].is_null(), "{made}");

    // Used up by the create
    let (s, v) = call(&e.app, "POST", "/api/teams/from-template", Some(json!({ "suggestion_id": sug["suggestion_id"] }))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::GONE, Some("suggestion_expired")), "{v}");

    // One Undo removes the bots and the team
    let (s, u) = call(&e.app, "POST", &format!("/api/bot-actions/{}/undo", made["action_id"].as_str().unwrap()), None).await;
    assert!(s.is_success(), "{s} {u}");
    assert_eq!(e.state.db.list_kwants(&space()).await.unwrap().len(), before);
    assert!(e.state.db.list_teams(&space()).await.unwrap().iter().all(|t| t.name != "bakery-desk"));
}

#[tokio::test]
async fn an_unusable_answer_is_retried_once_with_the_reason() {
    let e = setup().await;
    claude_ready(&e);
    plan_replies("tfw_retry", &["Sure! Here is your team: a lead and a helper.", &good()]);
    let (s, sug) = suggest(&e, "Help me answer customer emails. REPLYSEQ:tfw_retry").await;
    assert_eq!(s, StatusCode::OK, "{sug}");
    assert_eq!(sug["bots"].as_array().unwrap().len(), 3);
    let calls = invocations_with("REPLYSEQ:tfw_retry");
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().any(|c| c.contains("Your last answer couldn't be used (it wasn't one JSON object)")), "{calls:?}");
}

#[tokio::test]
async fn two_unusable_answers_are_draft_failed() {
    let e = setup().await;
    claude_ready(&e);
    plan_replies("tfw_bad", &["no json here", r#"{"bots":[{"name":"Solo","title":"Only One","instructions":"You work."}]}"#]);
    let (s, v) = suggest(&e, "Help me answer customer emails. REPLYSEQ:tfw_bad").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("draft_failed")), "{v}");
    assert_eq!(invocations_with("REPLYSEQ:tfw_bad").len(), 2, "one retry, never a third call");
    // Nothing was kept: the space is free for the next draft
    plan_replies("tfw_bad2", &[&good()]);
    let (s, v) = suggest(&e, "Help me answer customer emails. REPLYSEQ:tfw_bad2").await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn no_ready_account_is_409_and_nothing_runs() {
    let e = setup().await;
    // No engine has been found ready
    let (s, v) = suggest(&e, "Help me plan my week. REPLYSEQ:tfw_none").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("no_ready_account")), "{v}");
    assert!(invocations_with("REPLYSEQ:tfw_none").is_empty());
}

#[tokio::test]
async fn an_account_out_of_usage_is_429_with_the_account_and_when() {
    let e = setup().await;
    claude_ready(&e);
    let (s, v) = suggest(&e, "Help me plan my week. QUOTAALWAYS QUOTARESET:600 tfw_quota").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("account_limited")), "{v}");
    assert_eq!(v["account"], "This machine");
    let until = chrono::DateTime::parse_from_rfc3339(v["until"].as_str().unwrap()).unwrap();
    assert!(until > chrono::Utc::now() + chrono::Duration::minutes(5), "{v}");
    assert_eq!(invocations_with("tfw_quota").len(), 1);
    // The next ask doesn't start the CLI again while the account is out
    let (s, v) = suggest(&e, "Help me plan my week. QUOTAALWAYS QUOTARESET:600 tfw_quota").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("account_limited")), "{v}");
    assert_eq!(invocations_with("tfw_quota").len(), 1);
}

#[tokio::test]
async fn seven_bots_are_trimmed_to_the_lead_and_five_others() {
    let e = setup().await;
    claude_ready(&e);
    let mut bots: Vec<Value> = (1..=6).map(|i| bot(&format!("Helper{i}"), &format!("Helper Number {i}"), false, "custom")).collect();
    bots.insert(3, bot("ShopLead", "Bakery Lead", true, "custom"));
    plan_replies("tfw_seven", &[&draft(bots)]);
    let (s, sug) = suggest(&e, "A big team for a big shop please. REPLYSEQ:tfw_seven").await;
    assert_eq!(s, StatusCode::OK, "{sug}");
    let rows = sug["bots"].as_array().unwrap();
    assert_eq!(rows.len(), 6, "{sug}");
    assert_eq!(sug["trimmed"], 1);
    assert_eq!((rows[0]["name"].as_str(), rows[0]["lead"].as_bool()), (Some("ShopLead"), Some(true)));
    let names: Vec<&str> = rows[1..].iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Helper1", "Helper2", "Helper3", "Helper4", "Helper5"]);
    assert_eq!(rows.iter().filter(|r| r["lead"] == true).count(), 1);
}

#[tokio::test]
async fn drafted_names_that_clash_are_made_unique_and_presets_are_checked() {
    let e = setup().await;
    claude_ready(&e);
    // The space starts with a TechLead; the model also repeats a name and names a preset that doesn't exist
    let answer = draft(vec![
        bot("TechLead", "Shop Lead", true, "custom"),
        bot("Writer", "Copy Writer", false, "made_up_role"),
        bot("Writer", "Second Writer", false, "custom"),
    ]);
    plan_replies("tfw_names", &[&answer]);
    let (s, sug) = suggest(&e, "Write my product pages for me. REPLYSEQ:tfw_names").await;
    assert_eq!(s, StatusCode::OK, "{sug}");
    let rows = sug["bots"].as_array().unwrap();
    let names: Vec<String> = rows.iter().map(|r| r["name"].as_str().unwrap().to_lowercase()).collect();
    assert_ne!(names[0], "techlead", "{sug}");
    assert!(names[0].starts_with("techlead"), "{sug}");
    assert_eq!(names[1], "writer");
    assert_ne!(names[2], "writer");
    let keys: std::collections::HashSet<&str> = rows.iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(keys.len(), 3, "{sug}");
    assert_eq!(rows[1]["preset"], "custom", "an unknown preset is custom");
    // As drafted, the names are free: Create works with no edits
    let (s, made) = call(&e.app, "POST", "/api/teams/from-template", Some(json!({ "suggestion_id": sug["suggestion_id"] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
}

#[tokio::test]
async fn one_draft_per_space_and_delete_stops_it() {
    let e = setup().await;
    claude_ready(&e);
    plan_replies("tfw_busy", &[&good()]);
    let app = e.app.clone();
    let first = tokio::spawn(async move {
        call(&app, "POST", "/api/teams/suggest", Some(json!({ "need": "Slow one please. SLEEPONCE:tfw_busy:20000 REPLYSEQ:tfw_busy" }))).await
    });
    common::wait_for("the draft to start", || async { !invocations_with("SLEEPONCE:tfw_busy").is_empty() }).await;
    let (s, v) = suggest(&e, "Another one at once. REPLYSEQ:tfw_busy").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("busy")), "{v}");
    let (s, _) = call(&e.app, "DELETE", "/api/teams/suggest", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = tokio::time::timeout(Duration::from_secs(10), first).await.expect("the draft stops at once").unwrap();
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("cancelled")), "{v}");
    // Idle: still 204, and the space takes a new draft
    let (s, _) = call(&e.app, "DELETE", "/api/teams/suggest", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = suggest(&e, "Now a quick one. REPLYSEQ:tfw_busy").await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

#[tokio::test]
async fn the_words_are_checked_before_any_call() {
    let e = setup().await;
    claude_ready(&e);
    let (s, v) = suggest(&e, "   short   ").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("need_too_short")), "{v}");
    let long = format!("{} tfw_long", "a".repeat(1000));
    let (s, v) = suggest(&e, &long).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("need_too_long")), "{v}");
    let (s, _) = call(&e.app, "POST", "/api/teams/suggest", Some(json!({ "need": 42 }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&e.app, "POST", "/api/teams/suggest", Some(json!({ "need": "Ten chars and more", "team_id": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(invocations_with("tfw_long").is_empty());
}
