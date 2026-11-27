//! pocket-switch-santa-alone (autonomy/pocket-switch-santa-alone/brief.md): the `pocket_apps`
//! setting also takes a kit list, so Secret Santa can go on alone while the Quiz waits for pa8.
//! `on` is every kit, `off`/empty/unknown names are off, and `santa` lets Santa through and keeps
//! the Quiz's routes, tools and card answering `pocket_apps_off` (and the reverse with `quiz`).
//! The guest port listens while any kit is on. A pocket made before the switch narrowed stays
//! readable by the person.

mod quizmaster_support;

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use chrono::{Datelike, NaiveDate, Utc};
use kwnt::db::models::{Kwant, Message, MessageExtra, NewMessage};
use kwnt::mcp::SELF_SERVER_NAME;
use kwnt::server::continuity::TurnOrigin;
use quizmaster_support::{setup, tool, tool_names, turn, turn_in, Env};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;

async fn switch(e: &Env, value: &str) {
    e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
}

/// The person on this computer, on the main port.
async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = e.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// The next Dec 18 at least a week away (the kit refuses past dates).
fn gift_day() -> NaiveDate {
    let today = Utc::now().date_naive();
    let this_year = NaiveDate::from_ymd_opt(today.year(), 12, 18).unwrap();
    if this_year - today >= chrono::Duration::days(7) {
        this_year
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, 12, 18).unwrap()
    }
}

fn santa_spec() -> Value {
    let d = gift_day().format("%Y-%m-%d").to_string();
    json!({
        "title": "Team Secret Santa",
        "gift_on": d,
        "reveal": true,
        "reveal_at": format!("{d}T19:00:00+00:00"),
        "people": ["Anna", "Ben", "Chloe", "Dev"],
        "keep_apart": [["Anna", "Ben"]],
    })
}

/// `POST /api/pockets` for `kit`: (status, error code).
async fn make_through_api(e: &Env, kit: &str, spec: Value) -> (StatusCode, Value) {
    person(e, "POST", "/api/pockets", Some(json!({ "kit": kit, "spec": spec }))).await
}

fn error(b: &Value) -> Option<&str> {
    b["error"].as_str()
}

/// The Quiz's gate passed when the core reaches the spec check (an empty spec is a bad spec).
async fn quiz_create_status(e: &Env) -> (StatusCode, Option<String>) {
    let (s, b) = make_through_api(e, "quiz", json!({})).await;
    (s, error(&b).map(str::to_string))
}

/// A turn's record of one of its own tool calls, as the self server writes it.
fn call(tool: &str, args: Value, status: &str, result: &str) -> Value {
    json!({"id": format!("call-{tool}"), "name": format!("{SELF_SERVER_NAME}__{tool}"), "server": SELF_SERVER_NAME, "tool": tool,
           "arguments": args, "status": status, "result": result})
}

/// `bot`'s final reply in its own chat, with the turn's tool calls.
async fn reply_as(e: &Env, bot: &Kwant, calls: Vec<Value>) -> Message {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(bot.id.clone()),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "Your Secret Santa is open.".into(),
        reasoning_content: None,
        tool_calls: Some(Value::Array(calls)),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

fn pocket_block(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

/// Makes a Santa through the bot's `santa_create`: (is_error, text).
async fn santa_by_tool(e: &Env) -> (bool, String) {
    e.state.db.set_setting("timezone", "Europe/London").await.unwrap();
    let origin = TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    let token = turn_in(e, &e.bot, "t-switch-santa", origin).await;
    let (text, err) = tool(e, &token, "santa_create", json!({"spec": santa_spec()})).await;
    (err, text)
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

#[tokio::test]
async fn the_switch_reads_on_off_and_kit_lists() {
    let e = setup(false).await;
    let all: Vec<String> = kwnt::pocket::kits::kit_ids().into_iter().map(str::to_string).collect();
    let cases: [(&str, Vec<String>); 9] = [
        ("on", all.clone()),
        ("off", vec![]),
        ("", vec![]),
        ("santa", vec!["santa".into()]),
        ("quiz", vec!["quiz".into()]),
        ("santa,quiz", vec!["quiz".into(), "santa".into()]),
        (" Santa ", vec!["santa".into()]),
        ("santa,bogus", vec!["santa".into()]),
        ("bogus", vec![]),
    ];
    for (value, want) in cases {
        switch(&e, value).await;
        let (status, body) = person(&e, "GET", "/api/pockets", None).await;
        assert_eq!(status, StatusCode::OK, "{value:?}: {body}");
        assert_eq!(sorted(strings(&body["kits"])), sorted(want.clone()), "{value:?}: {body}");
        assert_eq!(body["enabled"], json!(!want.is_empty()), "{value:?}: {body}");
        assert_eq!(kwnt::pocket::enabled(&e.state.db).await, !want.is_empty(), "{value:?}");
    }
}

#[tokio::test]
async fn santa_alone_makes_a_santa_and_refuses_the_quiz() {
    let e = setup(false).await;
    switch(&e, "santa").await;
    let (status, body) = make_through_api(&e, "santa", santa_spec()).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // The Quiz (and every other kit) answers as if pocket apps were off
    assert_eq!(quiz_create_status(&e).await, (StatusCode::NOT_FOUND, Some("pocket_apps_off".into())));
    for kit in ["desk", "wall", "gather", "queue"] {
        let (s, b) = make_through_api(&e, kit, json!({})).await;
        assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{kit}: {b}");
    }
    // An unknown kit still says so
    let (s, b) = make_through_api(&e, "nope", json!({})).await;
    assert_eq!((s, error(&b)), (StatusCode::BAD_REQUEST, Some("unknown_kit")), "{b}");
    // The quiz drafts API is off
    let (s, b) = person(&e, "GET", "/api/quiz-drafts/qd_whatever1", None).await;
    assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{b}");
}

#[tokio::test]
async fn santa_alone_offers_santa_create_only_and_attaches_its_card() {
    let e = setup(false).await;
    switch(&e, "santa").await;
    let token = turn(&e, &e.bot, "t-switch-tools").await;
    let names = tool_names(&e, &token).await;
    assert!(names.iter().any(|n| n == "santa_create"), "{names:?}");
    for t in ["pocket_create", "pocket_update", "pocket_dry_run", "pocket_open", "pocket_close"] {
        assert!(!names.iter().any(|n| n == t), "{t} offered: {names:?}");
    }
    // A Quizmaster tool called anyway refuses
    let (text, err) = tool(&e, &token, "pocket_create", json!({"spec": {}})).await;
    assert!(err && text.contains("off"), "{text}");
    // santa_create makes the Santa, and its reply gets the Santa card
    let (err, text) = santa_by_tool(&e).await;
    assert!(!err, "{text}");
    let bot = e.bot.clone();
    let m = reply_as(&e, &bot, vec![call("santa_create", json!({"spec": santa_spec()}), "ok", &text)]).await;
    let b = pocket_block(&m).unwrap_or_else(|| panic!("no Santa card: {:?}", m.results));
    assert_eq!(b["kit"], "santa", "{b}");
}

#[tokio::test]
async fn quiz_alone_is_the_reverse() {
    let e = setup(false).await;
    switch(&e, "quiz").await;
    let (s, b) = make_through_api(&e, "santa", santa_spec()).await;
    assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{b}");
    assert_eq!(quiz_create_status(&e).await, (StatusCode::BAD_REQUEST, Some("bad_spec".into())), "the Quiz's gate is open");
    let (s, b) = person(&e, "GET", "/api/quiz-drafts/qd_whatever1", None).await;
    assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("not_found")), "{b}");
    let token = turn(&e, &e.bot, "t-switch-quiz").await;
    let names = tool_names(&e, &token).await;
    assert!(names.iter().any(|n| n == "pocket_create"), "{names:?}");
    assert!(!names.iter().any(|n| n == "santa_create"), "{names:?}");
    let (err, text) = santa_by_tool(&e).await;
    assert!(err && text.contains("off"), "{text}");
    let bot = e.bot.clone();
    let m = reply_as(&e, &bot, vec![call("santa_create", json!({"spec": santa_spec()}), "ok", "Pocket app · Secret Santa [abcdefgh]")]).await;
    assert_eq!(pocket_block(&m), None, "{:?}", m.results);
}

#[tokio::test]
async fn on_and_off_work_as_before() {
    let e = setup(false).await;
    switch(&e, "on").await;
    let (s, b) = make_through_api(&e, "santa", santa_spec()).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(quiz_create_status(&e).await, (StatusCode::BAD_REQUEST, Some("bad_spec".into())));
    let token = turn(&e, &e.bot, "t-switch-on").await;
    let names = tool_names(&e, &token).await;
    assert!(names.iter().any(|n| n == "pocket_create") && names.iter().any(|n| n == "santa_create"), "{names:?}");
    let (err, text) = santa_by_tool(&e).await;
    assert!(!err, "{text}");

    switch(&e, "off").await;
    let (s, b) = make_through_api(&e, "santa", santa_spec()).await;
    assert_eq!((s, error(&b)), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{b}");
    assert_eq!(quiz_create_status(&e).await, (StatusCode::NOT_FOUND, Some("pocket_apps_off".into())));
    let token = turn(&e, &e.bot, "t-switch-off").await;
    let names = tool_names(&e, &token).await;
    assert!(!names.iter().any(|n| n == "pocket_create" || n == "santa_create"), "{names:?}");
}

#[tokio::test]
async fn a_pocket_made_before_the_switch_narrowed_stays_readable() {
    let e = setup(false).await;
    switch(&e, "on").await;
    let (s, b) = make_through_api(&e, "santa", santa_spec()).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let code = b["pocket"]["code"].as_str().unwrap().to_string();
    switch(&e, "quiz").await;
    let (s, b) = person(&e, "GET", &format!("/api/pockets/{code}"), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = person(&e, "GET", "/api/pockets", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(b["pockets"].as_array().unwrap().iter().any(|p| p["code"] == json!(code)), "{b}");
    // Nothing is deleted when the switch goes off; it reads again once a kit is on
    switch(&e, "off").await;
    assert_eq!(person(&e, "GET", &format!("/api/pockets/{code}"), None).await.0, StatusCode::NOT_FOUND);
    switch(&e, "santa").await;
    assert_eq!(person(&e, "GET", &format!("/api/pockets/{code}"), None).await.0, StatusCode::OK);
}

/// The guest port listens while any kit is on (its own port 0, never the main port + 1).
#[tokio::test]
async fn santa_alone_keeps_the_guest_port_listening() {
    use kwnt::{broker::PermissionBroker, config::Config, db::Database, pocket::listener, router::LayaRouter, server::AppState, tools::ToolRunner};
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    for (value, on) in [("santa", true), ("bogus", false), ("quiz", true), ("off", false), ("on", true), ("", false)] {
        state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        assert_eq!(listener::reconcile(&state).await, on, "{value:?}");
        assert_eq!(state.pocket.listener.addr().is_some(), on, "{value:?}");
    }
}
