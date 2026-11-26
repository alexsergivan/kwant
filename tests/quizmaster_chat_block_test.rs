//! pa3c: the Quizmaster's chat card carries its draft id and "yours"
//! (autonomy/pocket-apps/pa3c/brief.md). A bot reply whose turn called a Quizmaster tool gets the
//! Smart Results `pocket` block, built from the stored draft (never from the call's text): `draft`
//! is the quiz_drafts id and `yours` the person's own questions, in writing → ready, and `draft` on
//! open (live) and closed (ended) too. A draft id from another space or another bot, a call that
//! failed, or the switch off: no pocket block.

mod quizmaster_support;

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use kwnt::db::models::{Kwant, Message, MessageExtra, NewMessage};
use kwnt::db::quiz_drafts::QuizDraftRow;
use kwnt::mcp::SELF_SERVER_NAME;
use quizmaster_support::{card, ready_friday_quiz, setup, space, start_pages, tool, turn, Env};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;

/// A turn's record of one of its own tool calls, as the self server writes it.
fn call(tool: &str, args: Value, status: &str, result: &str) -> Value {
    json!({"id": format!("call-{tool}"), "name": format!("{SELF_SERVER_NAME}__{tool}"), "server": SELF_SERVER_NAME, "tool": tool,
           "arguments": args, "status": status, "result": result})
}

/// The bot's final reply in its direct chat, with the turn's tool calls.
async fn reply_as(e: &Env, bot: &Kwant, direct: &str, calls: Vec<Value>) -> Message {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(direct.into()),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "Your Friday Pub Quiz is ready.".into(),
        reasoning_content: None,
        tool_calls: Some(Value::Array(calls)),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

async fn reply(e: &Env, calls: Vec<Value>) -> Message {
    let bot = e.bot.clone();
    reply_as(e, &bot, &bot.id, calls).await
}

/// The reply's pocket block, or `None`.
fn pocket(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

/// The same reply as the chat reads it back (stored, then served).
async fn served(e: &Env, m: &Message) -> Option<Value> {
    let mut list = vec![m.clone()];
    list[0].results = None;
    e.state.db.attach_results(&mut list).await;
    pocket(&list[0])
}

async fn person_patch(e: &Env, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder().method("PATCH").uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(Body::from(body.to_string())).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = e.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    (status, serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap_or(Value::Null))
}

async fn ready(e: &Env) -> (String, String) {
    let (pages_url, _pages) = start_pages().await;
    let token = turn(e, &e.bot, "t-pa3c").await;
    let (id, _) = ready_friday_quiz(e, &token, &pages_url).await;
    (id, token)
}

async fn row(e: &Env, id: &str) -> QuizDraftRow {
    e.state.db.quiz_draft_get(&space(), id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_ready_draft_reply_carries_its_draft_id_and_zero_yours() {
    let e = setup(true).await;
    let (id, token) = ready(&e).await;
    // pocket_create's result is the card text, "Pocket app · Quiz [<id>] …": the id is read from
    // it, and the block from the stored row. pocket_dry_run answers with the same card text.
    let (text, err) = tool(&e, &token, "pocket_dry_run", json!({"id": id})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["id"], json!(id));
    let m = reply(&e, vec![call("pocket_create", json!({"spec": {}}), "ok", &text)]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block on the reply: {:?}", m.results));
    assert_eq!(b["state"], json!("ready"), "{b}");
    assert_eq!(b["draft"], json!(id), "{b}");
    assert_eq!(b["yours"], json!(0), "{b}");
    assert_eq!(b["kit"], json!("quiz"));
    assert_eq!(b["name"], json!("Friday Pub Quiz"));
    assert_eq!(b["source"], json!({"kind": "slot"}));
    assert_eq!(b["provenance"]["kind"], json!("kwant_data"));
    assert_eq!(b["dry_run"]["passed"], json!(true), "ready means the dry run passed: {b}");
    assert_eq!(b["checks"]["matched"], json!(4), "{b}");
    assert_eq!(b["checks"]["total"], json!(4), "{b}");
    assert!(b["results"].is_null());
    // The chat reads back the same block
    assert_eq!(served(&e, &m).await, Some(b));

    // An update call names the draft in its arguments
    let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "ok", "card: {}")]).await;
    assert_eq!(pocket(&m).unwrap()["draft"], json!(id));
}

/// A call as the Claude CLI engine records it: let through on its permission path ("allowed"),
/// with no `result` (the CLI keeps the tool's answer to itself).
fn claude_call(tool: &str, args: Value) -> Value {
    json!({"id": format!("toolu-{tool}"), "name": format!("mcp__{SELF_SERVER_NAME}__{tool}"), "server": SELF_SERVER_NAME, "tool": tool,
           "arguments": args, "status": "allowed"})
}

/// The person's message in the bot's direct chat: the ask a turn answers.
async fn ask(e: &Env) {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(e.bot.id.clone()),
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: "Make us a pub quiz for Friday 8 PM".into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap();
}

/// quiz-card-and-results (Mira's ux-pass-0952 #2): on the Claude engine the Quizmaster said the
/// quiz was ready and no card came, because its calls are "allowed" with no answer. The card comes
/// from the draft the turn made (a create alone) or named (a later call's `id`).
#[tokio::test]
async fn a_claude_engine_turn_gets_the_ready_card() {
    let e = setup(true).await;
    ask(&e).await;
    let (id, _) = ready(&e).await;
    // What the bot sent (the record keeps the arguments; the stored draft is what the card shows)
    let spec = json!({"version": 1, "title": "Friday Pub Quiz"});

    // pocket_create alone: no id anywhere in the record, so the draft this turn made
    let m = reply(&e, vec![claude_call("pocket_create", json!({"spec": spec}))]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("the Ready card under a Claude reply: {:?}", m.results));
    assert_eq!((b["state"].clone(), b["draft"].clone(), b["name"].clone()), (json!("ready"), json!(id), json!("Friday Pub Quiz")), "{b}");
    assert_eq!(b["dry_run"]["passed"], json!(true), "{b}");
    assert_eq!(b["checks"]["matched"], json!(4), "{b}");
    assert_eq!(served(&e, &m).await, Some(b));

    // A later call names the draft
    let m = reply(&e, vec![claude_call("pocket_create", json!({"spec": spec})), claude_call("pocket_dry_run", json!({"id": id}))]).await;
    assert_eq!(pocket(&m).expect("a pocket block")["draft"], json!(id));

    // Asking for the template makes no draft; a denied call ran nothing
    assert_eq!(pocket(&reply(&e, vec![claude_call("pocket_create", json!({}))]).await), None);
    let mut denied = claude_call("pocket_dry_run", json!({"id": id}));
    denied["status"] = json!("denied");
    assert_eq!(pocket(&reply(&e, vec![denied]).await), None);

    // A draft made before the newest ask isn't this turn's (message times keep whole seconds)
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    ask(&e).await;
    assert_eq!(pocket(&reply(&e, vec![claude_call("pocket_create", json!({"spec": spec}))]).await), None, "an older draft");
    // Named by id, it is still the bot's own draft
    assert_eq!(pocket(&reply(&e, vec![claude_call("pocket_update", json!({"id": id}))]).await).expect("a pocket block")["draft"], json!(id));
}

#[tokio::test]
async fn after_the_persons_edit_yours_is_one() {
    let e = setup(true).await;
    let (id, _) = ready(&e).await;
    let base = row(&e, &id).await.updated_at;
    let (status, body) =
        person_patch(&e, &format!("/api/quiz-drafts/{id}/questions/2"), json!({"accepted": ["Zidane", "Zizou", "ZZ"], "base": base})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["card"]["yours"], json!(1));
    assert!(row(&e, &id).await.own_keys.len() > 2, "own_keys has the edited question");

    let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "ok", "")]).await;
    let b = pocket(&m).expect("a pocket block");
    assert_eq!((b["draft"].clone(), b["yours"].clone()), (json!(id), json!(1)), "{b}");
    assert_eq!(b["state"], json!("ready"));
}

#[tokio::test]
async fn writing_checking_and_stopped_drafts_carry_the_draft_id() {
    let e = setup(true).await;
    let (id, _) = ready(&e).await;
    for (state, want) in [("writing", "writing"), ("checking", "checking"), ("dry_run_stopped", "dry_run_stopped")] {
        let mut r = row(&e, &id).await;
        r.state = state.into();
        if state == "dry_run_stopped" {
            r.dry_run = Some(json!({"ok": false, "summary": "The dry run stopped at round 1.", "problems": ["Q3 has no answer"]}).to_string());
        }
        e.state.db.quiz_draft_save(&r).await.unwrap();
        let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "ok", "")]).await;
        let b = pocket(&m).unwrap_or_else(|| panic!("{state}: a pocket block: {:?}", m.results));
        assert_eq!(b["state"], json!(want), "{b}");
        assert_eq!(b["draft"], json!(id), "{state}: {b}");
        assert_eq!(b["yours"], json!(0), "{state}: {b}");
        if state == "dry_run_stopped" {
            assert_eq!(b["dry_run"]["passed"], json!(false), "{b}");
            assert_eq!(b["dry_run"]["what_happened"], json!("Q3 has no answer"), "{b}");
        }
    }
}

#[tokio::test]
async fn a_draft_still_being_written_comes_back_as_an_error_card_and_still_gets_its_block() {
    // The Quizmaster answers a draft in writing with its card as an error, so the bot fixes it
    let e = setup(true).await;
    let (id, _) = ready(&e).await;
    let mut r = row(&e, &id).await;
    r.state = "writing".into();
    e.state.db.quiz_draft_save(&r).await.unwrap();
    let card_text = format!("Pocket app · Quiz [{id}] \"Friday Pub Quiz\" · Writing\ncard: {{}}");
    let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "error", &card_text)]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block: {:?}", m.results));
    assert_eq!((b["state"].clone(), b["draft"].clone(), b["yours"].clone()), (json!("writing"), json!(id), json!(0)), "{b}");
    // Another failure doesn't count
    let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "error", "Couldn't save the quiz.")]).await;
    assert_eq!(pocket(&m), None);
}

#[tokio::test]
async fn open_and_closed_blocks_carry_the_draft_id() {
    let e = setup(true).await;
    let (id, _) = ready(&e).await;
    for (state, want) in [("open", "live"), ("closed", "ended")] {
        let mut r = row(&e, &id).await;
        r.state = state.into();
        r.pocket_code = Some("PQ7XK2".into());
        e.state.db.quiz_draft_save(&r).await.unwrap();
        let tool_name = if state == "open" { "pocket_open" } else { "pocket_close" };
        let m = reply(&e, vec![call(tool_name, json!({"id": id}), "ok", "")]).await;
        let b = pocket(&m).unwrap_or_else(|| panic!("{state}: a pocket block: {:?}", m.results));
        assert_eq!(b["state"], json!(want), "{b}");
        assert_eq!(b["draft"], json!(id), "{state}: {b}");
        assert_eq!(b["code"], json!("PQ7XK2"), "the pocket's own code once it is open: {b}");
        assert!(b["results"].is_null());
    }
}

#[tokio::test]
async fn another_spaces_or_bots_draft_id_is_never_emitted() {
    let e = setup(true).await;
    let (id, _) = ready(&e).await;
    let mine = row(&e, &id).await;

    // The same bot id, but a draft stored in another space
    let elsewhere = QuizDraftRow { id: "qd_elsewhere01".into(), space_id: "space-other".into(), ..mine.clone() };
    e.state.db.quiz_draft_save(&elsewhere).await.unwrap();
    let m = reply(&e, vec![call("pocket_update", json!({"id": "qd_elsewhere01"}), "ok", "")]).await;
    assert_eq!(pocket(&m), None, "another space's draft");
    assert!(!serde_json::to_string(&m.results).unwrap().contains("qd_elsewhere01"));

    // A draft of another bot in this space
    let theirs = QuizDraftRow { id: "qd_theirs00001".into(), kwant_id: e.other.id.clone(), ..mine.clone() };
    e.state.db.quiz_draft_save(&theirs).await.unwrap();
    let m = reply(&e, vec![call("pocket_update", json!({"id": "qd_theirs00001"}), "ok", "")]).await;
    assert_eq!(pocket(&m), None, "another bot's draft");

    // The other bot's reply naming this bot's draft
    let other = e.other.clone();
    let m = reply_as(&e, &other, &other.id, vec![call("pocket_update", json!({"id": id}), "ok", "")]).await;
    assert_eq!(pocket(&m), None, "a reply by another bot");

    // pocket_create's result text claiming another space's draft
    let m = reply(&e, vec![call("pocket_create", json!({}), "ok", "Pocket app · Quiz [qd_elsewhere01] \"x\" · Ready\ncard: {}")]).await;
    assert_eq!(pocket(&m), None, "a forged id in a result");

    // No such draft, a failed call, another server's tool of the same name
    let m = reply(&e, vec![call("pocket_update", json!({"id": "qd_nope"}), "ok", "")]).await;
    assert_eq!(pocket(&m), None);
    let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "error", "Couldn't save the quiz.")]).await;
    assert_eq!(pocket(&m), None, "a call that failed");
    let mut foreign = call("pocket_update", json!({"id": id}), "ok", "");
    foreign["server"] = json!("someones_mcp");
    let m = reply(&e, vec![foreign]).await;
    assert_eq!(pocket(&m), None, "not kwant's own tool");
}

#[tokio::test]
async fn the_switch_off_emits_nothing_new() {
    let e = setup(true).await;
    let (id, _) = ready(&e).await;
    e.state.db.set_setting(kwnt::server::quizmaster_tools::SETTING, "off").await.unwrap();
    let m = reply(&e, vec![call("pocket_update", json!({"id": id}), "ok", "")]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);
    // A reply with no Quizmaster call never gets one either
    e.state.db.set_setting(kwnt::server::quizmaster_tools::SETTING, "on").await.unwrap();
    let m = reply(&e, vec![]).await;
    assert_eq!(pocket(&m), None);
}
