//! quiz-open-without-id (Mira's UX pass on a real pub quiz, Minor row a): on a Custom API or API
//! model the host says "open it" in the turn after the Ready card, and that turn has no qd_ id:
//! the history carries only the earlier reply's text. `pocket_update`, `pocket_dry_run`,
//! `pocket_open` and `pocket_close` now take `id` as optional:
//!
//! - one quiz the tool can act on: it is used;
//! - several: a list to pick from, and nothing changes;
//! - none: says so;
//! - never another bot's quiz (or another space's);
//! - a given `id` works exactly as before.
//!
//! Plus one scripted Custom API bot, end to end: "make us a pub quiz…", then a new turn "open it".
//! Its own test binary: a Pocket core registers once per process.

mod common;
mod quizmaster_support;

use async_trait::async_trait;
use axum::{extract::State, http::StatusCode, routing::get, routing::post, Json, Router};
use kwnt::server::{
    quizmaster_tools::{self, register_pocket_core, PocketCore, PocketOrigin},
    AppState,
};
use quizmaster_support::{card, ready_friday_quiz, setup, space, start_pages, tool, turn, write_friday_quiz, Env};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Opens and closes nothing real: hands out codes PQ1, PQ2… and counts calls.
#[derive(Default)]
struct FakeCore {
    opens: AtomicUsize,
    closes: AtomicUsize,
}

#[async_trait]
impl PocketCore for FakeCore {
    async fn open(&self, _state: &AppState, _kit: &str, _spec: &Value, _origin: &PocketOrigin) -> Result<String, String> {
        Ok(format!("PQ{}", self.opens.fetch_add(1, Ordering::SeqCst) + 1))
    }

    async fn close(&self, _state: &AppState, _code: &str) -> Result<(), String> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn core() -> &'static Arc<FakeCore> {
    static CORE: OnceLock<Arc<FakeCore>> = OnceLock::new();
    CORE.get_or_init(|| {
        let c = Arc::new(FakeCore::default());
        assert!(register_pocket_core(c.clone()), "the first core registers");
        c
    })
}

async fn state_of(e: &Env, id: &str) -> String {
    e.state.db.quiz_draft_get(&space(), id).await.unwrap().unwrap().state
}

#[tokio::test]
async fn with_one_quiz_open_update_dry_run_and_close_need_no_id() {
    core();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-one").await;
    let (id, _) = ready_friday_quiz(&e, &token, &pages_url).await;

    // Move it, dry-run it: the one quiz is used (blank and null count as left out)
    let (text, err) = tool(&e, &token, "pocket_update", json!({"when": "2026-11-27T21:00:00+00:00"})).await;
    assert!(!err, "{text}");
    assert_eq!((card(&text)["id"].clone(), card(&text)["when"].clone()), (json!(id), json!("2026-11-27T21:00:00+00:00")), "{text}");
    let (text, err) = tool(&e, &token, "pocket_dry_run", json!({"id": "  ", "players": 40})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["id"], json!(id));

    // "open it"
    let (text, err) = tool(&e, &token, "pocket_open", json!({})).await;
    assert!(!err, "{text}");
    let c = card(&text);
    assert_eq!((c["id"].clone(), c["state"].clone()), (json!(id), json!("open")), "{text}");
    assert!(c["pocket"].as_str().unwrap().starts_with("PQ"), "{text}");
    assert_eq!(state_of(&e, &id).await, "open");
    // Again: the open quiz is the only candidate, and it says it's open
    let (why, err) = tool(&e, &token, "pocket_open", json!({"id": null})).await;
    assert!(err && why.contains("already open"), "{why}");

    // "close it"
    let (text, err) = tool(&e, &token, "pocket_close", json!({})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["state"], json!("closed"), "{text}");
    // Nothing left to act on
    let (why, err) = tool(&e, &token, "pocket_close", json!({})).await;
    assert!(err && why.contains("no open quiz to close"), "{why}");
    for (name, verb) in [("pocket_open", "open"), ("pocket_update", "update"), ("pocket_dry_run", "dry-run")] {
        let (why, err) = tool(&e, &token, name, json!({})).await;
        assert!(err && why.contains(&format!("no quiz draft to {verb}")) && why.contains("pocket_create"), "{name}: {why}");
    }
}

#[tokio::test]
async fn with_two_quizzes_and_no_id_a_list_comes_back_and_nothing_changes() {
    core();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-two").await;
    let (first, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    let (second, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    let before = e.state.db.quiz_draft_get(&space(), &first).await.unwrap();

    for name in ["pocket_open", "pocket_update", "pocket_dry_run"] {
        let (why, err) = tool(&e, &token, name, json!({"when": "2026-11-28T21:00:00+00:00"})).await;
        assert!(err, "{name}: {why}");
        assert!(why.contains("You have 2 quizzes") && why.contains("nothing changed") && why.contains("Call again with `id`."), "{name}: {why}");
        for id in [&first, &second] {
            assert!(why.contains(&format!("{id} · Friday Pub Quiz · Ready · updated ")), "{name}: {why}");
        }
    }
    assert_eq!(state_of(&e, &first).await, "ready");
    assert_eq!(state_of(&e, &second).await, "ready");
    assert_eq!(e.state.db.quiz_draft_get(&space(), &first).await.unwrap(), before, "the first quiz is untouched");

    // Picking from the list works as it always did
    let (text, err) = tool(&e, &token, "pocket_open", json!({"id": second})).await;
    assert!(!err, "{text}");
    assert_eq!(state_of(&e, &second).await, "open");
    // One open, one Ready: close has a single candidate, open still lists both
    let (why, err) = tool(&e, &token, "pocket_open", json!({})).await;
    assert!(err && why.contains("You have 2 quizzes"), "{why}");
    let (text, err) = tool(&e, &token, "pocket_close", json!({})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["id"], json!(second), "{text}");
    assert_eq!(state_of(&e, &first).await, "ready", "close never picks a quiz that isn't open");
}

#[tokio::test]
async fn another_bots_quiz_is_never_picked() {
    core();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let mine = turn(&e, &e.bot, "t-mine").await;
    let theirs = turn(&e, &e.other, "t-theirs").await;
    let (their_id, _) = ready_friday_quiz(&e, &theirs, &pages_url).await;

    // No quiz of my own: theirs isn't a candidate
    for name in ["pocket_open", "pocket_update", "pocket_dry_run", "pocket_close"] {
        let (why, err) = tool(&e, &mine, name, json!({})).await;
        assert!(err && why.contains("You have no") && !why.contains(&their_id), "{name}: {why}");
    }
    assert_eq!(state_of(&e, &their_id).await, "ready");

    // One of mine and one of theirs: mine is the only one
    let (my_id, _) = ready_friday_quiz(&e, &mine, &pages_url).await;
    let (text, err) = tool(&e, &mine, "pocket_open", json!({})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["id"], json!(my_id));
    assert_eq!(state_of(&e, &their_id).await, "ready");
}

#[tokio::test]
async fn a_given_id_works_as_before() {
    core();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-given").await;
    let other = turn(&e, &e.other, "t-given-other").await;
    let (a, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    let (b, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    let (theirs, _) = ready_friday_quiz(&e, &other, &pages_url).await;

    for (name, args, says) in [
        ("pocket_open", json!({"id": theirs}), "That quiz belongs to another bot."),
        ("pocket_close", json!({"id": theirs}), "That quiz belongs to another bot."),
        ("pocket_open", json!({"id": "qd_nope"}), "There's no quiz \"qd_nope\" in your space."),
        ("pocket_open", json!({"id": 7}), "`id` is required"),
        ("pocket_open", json!({"id": "x".repeat(65)}), "`id` is required"),
        ("pocket_close", json!({"id": a}), "This quiz isn't open."),
    ] {
        let (why, err) = tool(&e, &token, name, args.clone()).await;
        assert!(err, "{name} {args}: {why}");
        assert_eq!(why, says, "{name} {args}");
    }
    let (text, err) = tool(&e, &token, "pocket_open", json!({"id": format!(" {a} ")})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["id"], json!(a));
    assert_eq!((state_of(&e, &a).await, state_of(&e, &b).await, state_of(&e, &theirs).await), ("open".into(), "ready".into(), "ready".into()));
}

#[test]
fn the_four_tools_take_id_as_optional() {
    let tools = quizmaster_tools::tools();
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert!(t["inputSchema"].get("required").is_none(), "{name}: {t}");
        if name != "pocket_create" {
            assert!(t["inputSchema"]["properties"]["id"]["description"].as_str().unwrap().contains("Leave it out"), "{name}");
        }
    }
}

// ---- a Custom API bot, end to end ------------------------------------------------------------

type Seen = Arc<Mutex<Vec<Value>>>;

fn say(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}], "usage": {"total_tokens": 9}})
}

fn call_tool(name: &str, args: Value) -> Value {
    json!({
        "choices": [{"message": {"role": "assistant", "content": "", "tool_calls": [{
            "id": format!("call_{}", args.to_string().len()), "type": "function",
            "function": {"name": name, "arguments": args.to_string()},
        }]}}],
        "usage": {"total_tokens": 9},
    })
}

fn content(m: &Value) -> String {
    match &m["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

const READY_REPLY: &str = "Your Friday Pub Quiz is Ready: every answer checked, dry run passed. Say \"open it\" when guests should join.";

/// Calls the offered tool whose name ends in `tool` (kwant prefixes it with its gateway's name),
/// the way a real model must: by name when the turn lists it in full, else through
/// `kwnt_call_tool`. A turn with many tools lists only kwant's core ones in full and names the
/// rest, Quizmaster's pocket tools among them, in `kwnt_find_tools`' description.
fn use_tool(body: &Value, tool: &str, args: Value) -> Value {
    let tools: Vec<&Value> = body["tools"].as_array().into_iter().flatten().map(|t| &t["function"]).collect();
    let ours = |n: &str| n == tool || n.ends_with(&format!("__{tool}"));
    if let Some(name) = tools.iter().filter_map(|f| f["name"].as_str()).find(|n| ours(n)) {
        return call_tool(name, args);
    }
    let named = tools
        .iter()
        .find(|f| f["name"] == "kwnt_find_tools")
        .and_then(|f| f["description"].as_str())
        .and_then(|d| d.split("Not listed in full: ").nth(1))
        .and_then(|list| list.trim_end_matches('.').split(", ").map(str::trim).find(|n| ours(n)))
        .unwrap_or_else(|| panic!("{tool} isn't offered: {}", body["tools"]))
        .to_string();
    call_tool("kwnt_call_tool", json!({"name": named, "arguments": args}))
}

/// What the scripted host's model does. Turn 1 ("make us a pub quiz"): the template, then the
/// whole quiz, then a reply that, like a real one, doesn't repeat the qd_ id. Turn 2 ("open it"):
/// `pocket_open` with no id (it has none), then the result.
fn script(body: &Value, pages: &str) -> Value {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let last = messages.last().cloned().unwrap_or_default();
    let asked = messages.iter().rev().find(|m| m["role"] == "user").map(content).unwrap_or_default();
    if last["role"] == "tool" {
        let result = content(&last);
        if result.contains("Template (") {
            return use_tool(body, "pocket_create", json!({"spec": write_friday_quiz(&result, pages)}));
        }
        if asked.contains("open it") {
            return say(&format!("Done. {}", result.lines().next().unwrap_or("")));
        }
        return say(READY_REPLY);
    }
    if asked.contains("open it") {
        return use_tool(body, "pocket_open", json!({}));
    }
    if asked.contains("make us a pub quiz") {
        return use_tool(body, "pocket_create", json!({}));
    }
    say("ok")
}

async fn fake_model(pages: String) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/v1/models", get(|| async { Json(json!({"data": [{"id": "qwen-7b"}]})) }))
        .route(
            "/v1/chat/completions",
            post(move |State(seen): State<Seen>, Json(body): Json<Value>| {
                let pages = pages.clone();
                async move {
                    seen.lock().unwrap().push(body.clone());
                    Json(script(&body, &pages))
                }
            }),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (root, seen)
}

#[tokio::test]
async fn a_custom_api_host_says_open_it_in_the_next_turn_and_the_quiz_opens() {
    core();
    let (pages_url, _) = start_pages().await;
    let env = common::setup().await;
    env.state.db.set_setting(quizmaster_tools::SETTING, "on").await.unwrap();
    let (root, seen) = fake_model(pages_url).await;

    let (s, sp) = common::call(&env.app, "POST", "/api/spaces", Some(json!({"name": "Pub", "icon": "🍺", "this_machine_enabled": false}))).await;
    assert_eq!(s, StatusCode::CREATED, "{sp}");
    let sp = sp["id"].as_str().unwrap().to_string();
    let (s, acct) = common::call(
        &env.app,
        "POST",
        &format!("/api/profiles?space={sp}"),
        Some(json!({"engine": "custom", "kind": "custom_api", "name": "Local 7B", "protocol": "openai-chat", "base_url": format!("{root}/v1"), "api_key": "sk-custom-quiz-0123456789", "model": "qwen-7b"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    let (s, bot) = common::call(
        &env.app,
        "POST",
        &format!("/api/kwants?space={sp}"),
        Some(json!({"name": "Quizmaster", "title": "Pub quizzes", "persona_prompt": "", "avatar_sprite": "s", "driver_type": "api", "preferred_model": "custom"})),
    )
    .await;
    assert!(s.is_success(), "{s} {bot}");
    let kid = bot["id"].as_str().unwrap().to_string();

    // Turn 1: the quiz is made and Ready
    common::send_direct(&env, &kid, "make us a pub quiz for Friday", json!({})).await;
    common::wait_reply(&env, &kid, "is Ready").await;
    let drafts = env.state.db.quiz_drafts_of_kwant(&sp, &kid, &["ready"], 5).await.unwrap();
    assert_eq!(drafts.len(), 1, "one Ready quiz");
    let id = drafts[0].id.clone();

    // Turn 2: "open it", with no qd_ id anywhere in what the model was sent
    common::send_direct(&env, &kid, "open it", json!({})).await;
    let reply = common::wait_reply(&env, &kid, "Done.").await;
    let opened_with = {
        let seen = seen.lock().unwrap();
        let req = seen.iter().find(|b| b["messages"].as_array().and_then(|m| m.last()).is_some_and(|m| m["role"] == "user" && content(m).contains("open it"))).cloned();
        req.expect("the open-it turn reached the model")
    };
    let sent: String = opened_with["messages"].as_array().unwrap().iter().map(content).collect::<Vec<_>>().join("\n");
    assert!(sent.contains(READY_REPLY), "turn 1's reply is in the history");
    assert!(!sent.contains(&id), "the qd_ id isn't in the history, which is the bug's premise: {sent}");

    let row = env.state.db.quiz_draft_get(&sp, &id).await.unwrap().unwrap();
    assert_eq!(row.state, "open", "{}", reply.text);
    assert!(row.pocket_code.as_deref().is_some_and(|c| c.starts_with("PQ")));
    assert!(reply.text.contains(&id) && !reply.text.contains("`id` is required"), "{}", reply.text);
}
