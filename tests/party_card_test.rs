//! party-card-and-results (autonomy/party-card-and-results/brief.md): the chat card for the
//! party pockets #1319 makes from chat. A bot's reply after `gather_create` (RSVP, sign-up, vote)
//! or `wall_create` carries a pocket block with a `gather` or `wall` part, in the shape the card
//! (Pocket.tsx validGather / validWall) draws; the ended post carries the same part and no
//! top-level `results`. An ended vote's chat line and Inbox title name the winner ("Party theme
//! has ended. Winter wonderland wins with 2 of 3."), a server-counted result is never "Not
//! checked", and an open Photo Wall with waiting photos is one Needs-you row that Approve clears.
//!
//! With `KWANT_PARTY_FIXTURES=<path>` the first test writes the four reply blocks it made to
//! `<path>` (frontend/src/components/results/fixtures/party-cards.json is that file, checked by
//! party-cards.test.ts against the card's own validator).

mod quizmaster_support;

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::{Datelike, Duration, Local, NaiveDate, Utc, Weekday};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, Message, MessageExtra, NewKwant, NewMessage},
        spaces::SpaceId,
        Database,
    },
    mcp::SELF_SERVER_NAME,
    pocket::{
        self,
        kits::Viewer,
        listener,
        schedule::{self, Origin},
        wall, CreateRequest, JoinRequest,
    },
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        continuity::TurnOrigin,
        create_router, AppState,
    },
    tools::ToolRunner,
};
use quizmaster_support::{setup as chat_setup, tool, turn_in, Env};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const GUEST_BASE: &str = "https://party.example.com";
const TOKEN: &str = "test-api-token-0123456789";
const PIN: &str = "4821";
const THEMES: [&str; 5] = ["Winter wonderland", "Ugly sweaters", "Casino night", "Roaring twenties", "Masquerade"];

// ---- The reply card (through the bot's own tools, as #1319 makes them) ----

/// The first Saturday at least 14 days ahead, and its `when` line.
fn party_day() -> (NaiveDate, String) {
    let mut d = Local::now().date_naive() + Duration::days(14);
    while d.weekday() != Weekday::Sat {
        d += Duration::days(1);
    }
    (d, d.format("%a, %b %-d, 6 PM").to_string())
}

fn at(d: NaiveDate, hm: &str) -> String {
    format!("{}T{hm}:00+01:00", d.format("%Y-%m-%d"))
}

fn rsvp() -> Value {
    let (d, when) = party_day();
    json!({"face": "rsvp", "title": "Office holiday party", "when": when, "place": "The Old Mill", "host_name": "Nina",
           "event_at": at(d, "18:00"), "price": "EUR 15", "fields": ["plus_ones", "food_needs"], "show_names": false})
}

fn potluck() -> Value {
    let (d, _) = party_day();
    json!({"face": "signup", "title": "Potluck", "event_at": at(d, "18:00"),
           "slots": [{"name": "Mains", "cap": 3}, {"name": "Desserts", "cap": 4}, {"name": "Drinks", "cap": 2}], "fields": ["slot_note"]})
}

fn theme_vote() -> Value {
    let (d, _) = party_day();
    json!({"face": "vote", "title": "Party theme", "closes_at": at(d - Duration::days(7), "18:00"),
           "ballot": {"kind": "ranked", "options": THEMES}, "members": ["Ana", "Ben", "Cleo"]})
}

fn photo_wall() -> Value {
    let (d, _) = party_day();
    json!({"title": "Office holiday party", "host_name": "Nina", "event_at": at(d, "18:00"), "closes_at": at(d + Duration::days(1), "12:00")})
}

fn code_after<'a>(text: &'a str, head: &str) -> &'a str {
    let rest = text.strip_prefix(head).unwrap_or_else(|| panic!("not made by {head}: {text}"));
    &rest[..rest.find(']').unwrap()]
}

/// Makes `args` with `tool_name` as the bot in its own direct chat: (code, the tool's text).
async fn make(e: &Env, tool_name: &str, args: Value) -> (String, String) {
    let origin = TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    let token = turn_in(e, &e.bot, &format!("t-{tool_name}-{}", rand_id()), origin).await;
    let (text, err) = tool(e, &token, tool_name, args).await;
    assert!(!err, "{text}");
    let head = if tool_name == "wall_create" { "Pocket app · Photo wall [" } else { "Pocket app · Gather [" };
    (code_after(&text, head).to_string(), text)
}

fn rand_id() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}

/// The bot's final reply in its own chat after the turn's one tool call.
async fn reply(e: &Env, tool_name: &str, args: Value, result: &str) -> Message {
    let call = json!({"id": format!("call-{tool_name}"), "name": format!("{SELF_SERVER_NAME}__{tool_name}"), "server": SELF_SERVER_NAME,
                      "tool": tool_name, "arguments": args, "status": "ok", "result": result});
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(e.bot.id.clone()),
        sender_role: "kwant".into(),
        sender_id: e.bot.name.clone(),
        text: "It's open: send the link to everyone.".into(),
        reasoning_content: None,
        tool_calls: Some(json!([call])),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

fn pocket_block(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

/// Whether `v` has `key` at any depth.
fn has_key(v: &Value, key: &str) -> bool {
    match v {
        Value::Object(o) => o.contains_key(key) || o.values().any(|x| has_key(x, key)),
        Value::Array(a) => a.iter().any(|x| has_key(x, key)),
        _ => false,
    }
}

/// What every party card holds, whatever its face: one kit part, no quiz or santa parts.
fn common_card(b: &Value, kit: &str, state: &str) {
    assert_eq!(b["kit"], kit, "{b}");
    assert_eq!(b["state"], state, "{b}");
    for gone in ["results", "santa", "desk", "live", "rounds"] {
        assert!(b.get(gone).is_none_or(Value::is_null), "{gone} in {b}");
    }
    let other = if kit == "gather" { "wall" } else { "gather" };
    assert!(b.get(other).is_none_or(Value::is_null), "{other} in {b}");
}

/// The gather card's must-haves (validGather): face, phase, title, when it goes.
fn gather_card(b: &Value, face: &str, phase: &str) -> Value {
    let g = b["gather"].clone();
    assert_eq!((g["face"].as_str(), g["phase"].as_str()), (Some(face), Some(phase)), "{g}");
    assert!(g["title"].as_str().is_some_and(|t| !t.trim().is_empty()), "{g}");
    assert!(g["expires_at"].is_string(), "{g}");
    assert!(g.get("join").is_none_or(|j| j.is_null() || j.is_object()), "{g}");
    g
}

/// The wall card's must-haves (validWall): counts that add up, the space used and the cap, and
/// none of the keys only the host's own screens carry.
fn wall_card(b: &Value, state: &str) -> Value {
    let w = b["wall"].clone();
    assert_eq!(w["state"], state, "{w}");
    let c = &w["counts"];
    let n = |k: &str| c[k].as_u64().unwrap_or_else(|| panic!("counts.{k} in {w}"));
    assert_eq!(n("total"), n("waiting") + n("approved") + n("hidden"), "{w}");
    assert!(w["used_mb"].as_f64().is_some(), "{w}");
    assert!(w["pocket_cap_mb"].as_f64().is_some_and(|m| m > 0.0), "{w}");
    for k in ["by", "bytes", "guest", "guest_id", "player_id", "players", "queue", "wall", "photos", "pin"] {
        assert!(!has_key(&w, k), "{k} in {w}");
    }
    w
}

#[tokio::test]
async fn each_party_reply_carries_a_card_the_chat_can_draw() {
    let e = chat_setup(true).await;
    e.state.db.set_setting(pocket::address::PUBLIC_URL_SETTING, GUEST_BASE).await.unwrap();
    let mut made = serde_json::Map::new();
    for (name, tool_name, args) in [("rsvp", "gather_create", rsvp()), ("signup", "gather_create", potluck()), ("vote", "gather_create", theme_vote()), ("wall", "wall_create", photo_wall())] {
        let (code, text) = make(&e, tool_name, args.clone()).await;
        let m = reply(&e, tool_name, args, &text).await;
        let b = pocket_block(&m).unwrap_or_else(|| panic!("{name}: no card in {:?}", m.results));
        assert_eq!(b["code"], json!(code), "{name}: {b}");
        if name == "wall" {
            common_card(&b, "wall", "live");
            let w = wall_card(&b, "open");
            assert_eq!(w["counts"]["total"], 0, "{w}");
        } else {
            common_card(&b, "gather", "live");
            let g = gather_card(&b, name, "open");
            match name {
                "rsvp" => assert!(g["counts"].is_object(), "{g}"),
                "signup" => {
                    let slots: Vec<(&str, u64)> = g["slots"].as_array().unwrap().iter().map(|s| (s["name"].as_str().unwrap(), s["cap"].as_u64().unwrap())).collect();
                    assert_eq!(slots, [("Mains", 3), ("Desserts", 4), ("Drinks", 2)], "{g}");
                }
                _ => {
                    let options: Vec<&str> = g["ballot"]["options"].as_array().unwrap().iter().map(|o| o["name"].as_str().unwrap()).collect();
                    assert_eq!(options, THEMES, "{g}");
                    assert_eq!(g["voted"], 0, "{g}");
                }
            }
        }
        made.insert(name.into(), b);
    }
    if let Ok(path) = std::env::var("KWANT_PARTY_FIXTURES") {
        std::fs::write(path, serde_json::to_string_pretty(&Value::Object(made)).unwrap() + "\n").unwrap();
    }
}

#[tokio::test]
async fn another_bots_reply_never_carries_this_bots_party_card() {
    let e = chat_setup(true).await;
    let (_, text) = make(&e, "gather_create", rsvp()).await;
    let call = json!({"id": "c", "name": format!("{SELF_SERVER_NAME}__gather_create"), "server": SELF_SERVER_NAME,
                      "tool": "gather_create", "arguments": rsvp(), "status": "ok", "result": text});
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(e.other.id.clone()),
        sender_role: "kwant".into(),
        sender_id: e.other.name.clone(),
        text: "Here it is.".into(),
        reasoning_content: None,
        tool_calls: Some(json!([call])),
        tokens_used: 0,
    };
    let m = e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap();
    assert_eq!(pocket_block(&m), None, "{:?}", m.results);
}

// ---- The ended post, the Inbox title and the wall's Needs-you row (the person's API) ----

struct T {
    main: Router,
    state: AppState,
    _tmp: TempDir,
}

async fn setup() -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    state.db.set_setting("timezone", "UTC").await.unwrap();
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), state, _tmp: tmp }
}

async fn bot(t: &T, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Party host".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    t.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

/// The person on the main port.
async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn create(t: &T, kit: &str, spec: Value, k: &Kwant) -> String {
    let origin = Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: kit.into(), spec, origin, ..Default::default() }).await.unwrap();
    s.row.code.clone()
}

async fn join(t: &T, code: &str, name: &str) -> String {
    let (g, _) = pocket::join(&t.state, code, JoinRequest { name: name.into(), avatar: None, team: None, pin: Some(PIN.into()) }).await.unwrap();
    g.id
}

/// The results post pa7 left in the bot's chat.
async fn results_post(t: &T, k: &Kwant, code: &str) -> Message {
    t.state
        .db
        .list_messages(&k.id, 100)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).is_some_and(|v| v["reason"] == "results" && v["code"] == json!(code)))
        .expect("the results post")
}

async fn inbox(t: &T) -> Value {
    let (status, body) = send(&t.main, "GET", "/api/inbox", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn of_kind(body: &Value, kind: &str) -> Vec<Value> {
    body["items"].as_array().unwrap().iter().filter(|i| i["kind"] == kind).cloned().collect()
}

#[tokio::test]
async fn an_ended_vote_names_its_winner_in_the_chat_line_the_card_and_the_inbox() {
    let t = setup().await;
    let k = bot(&t, "Party").await;
    let now = Utc::now();
    let closes = now + Duration::days(5);
    let spec = json!({"face": "vote", "title": "Party theme", "host_name": "Nina", "closes_at": closes.to_rfc3339(), "event_at": (now + Duration::days(7)).to_rfc3339(),
                      "ballot": {"kind": "ranked", "options": THEMES}});
    let code = create(&t, "gather", spec, &k).await;
    for (name, ranking) in [("Ana", json!([0])), ("Ben", json!([0, 1])), ("Cleo", json!([1]))] {
        let id = join(&t, &code, name).await;
        pocket::act(&t.state, &code, &Viewer::Guest(id), "vote", &json!({ "ranking": ranking })).await.unwrap();
    }
    let mut rx = t.state.event_sender.subscribe();
    pocket::sweep(&t.state, closes + Duration::minutes(1)).await;
    schedule::run_once(&t.state, closes + Duration::minutes(2)).await;
    let post = results_post(&t, &k, &code).await;
    assert_eq!(post.text, "Party theme has ended. Winter wonderland wins with 2 of 3.");
    // The phone's notice says the same
    let mut ends = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let kwnt::drivers::events::RuntimeEvent::Notify { title, body, .. } = ev {
            if title.ends_with("has ended") {
                ends.push((title, body));
            }
        }
    }
    assert_eq!(ends, [("Party theme has ended".to_string(), "Winter wonderland wins with 2 of 3. The results are in your chat.".to_string())]);
    // The ended card: the vote's own part, closed, with its result; no quiz results
    let b = pocket_block(&post).expect("the ended card");
    common_card(&b, "gather", "ended");
    let g = gather_card(&b, "vote", "closed");
    assert_eq!(g["result"]["winner"]["name"], "Winter wonderland", "{g}");
    assert!(g.get("join").is_none_or(Value::is_null), "no join once closed: {g}");
    for name in ["Ana", "Ben", "Cleo"] {
        assert!(!b.to_string().contains(name), "{name} in the posted card: {b}");
    }
    // The Inbox's Done row: "wins" for a vote, and a plain Done, never "Not checked"
    let body = inbox(&t).await;
    let items = of_kind(&body, "pocket");
    assert_eq!(items.len(), 1, "{body}");
    assert_eq!(items[0]["title"], "Party theme: Winter wonderland wins");
    assert!(items[0]["checked"].is_null(), "{}", items[0]);
}

#[tokio::test]
async fn an_ended_rsvp_says_how_many_people_came_and_its_card_is_closed() {
    let t = setup().await;
    let k = bot(&t, "Party").await;
    let now = Utc::now();
    let spec = json!({"face": "rsvp", "title": "Office holiday party", "host_name": "Nina", "event_at": (now + Duration::days(7)).to_rfc3339()});
    let code = create(&t, "gather", spec, &k).await;
    for name in ["Ana", "Ben"] {
        let id = join(&t, &code, name).await;
        pocket::act(&t.state, &code, &Viewer::Guest(id), "answer", &json!({ "answer": "yes" })).await.unwrap();
    }
    pocket::end(&t.state, &code).await.unwrap();
    schedule::run_once(&t.state, now + Duration::minutes(1)).await;
    let post = results_post(&t, &k, &code).await;
    assert_eq!(post.text, "Office holiday party has ended. 2 people took part.");
    let b = pocket_block(&post).expect("the ended card");
    common_card(&b, "gather", "ended");
    let g = gather_card(&b, "rsvp", "closed");
    assert_eq!(g["counts"]["coming"], 2, "{g}");
    for name in ["Ana", "Ben"] {
        assert!(!b.to_string().contains(name), "{name} in the posted card: {b}");
    }
}

/// Rigel's QA finding 2: an ended sign-up's card keeps its slots and counts and loses who took
/// them and what they wrote.
#[tokio::test]
async fn an_ended_signup_card_keeps_its_counts_and_no_name_or_note() {
    let t = setup().await;
    let k = bot(&t, "Party").await;
    let now = Utc::now();
    let spec = json!({"face": "signup", "title": "Party potluck", "host_name": "Nina", "event_at": (now + Duration::days(7)).to_rfc3339(),
                      "slots": [{"name": "Mains", "cap": 1}, {"name": "Desserts", "cap": 4}], "fields": ["slot_note"], "waiting_list": true});
    let code = create(&t, "gather", spec, &k).await;
    for (name, slot, note) in [("Ana", 0, "Lasagne from Zofia"), ("Ben", 1, "Lemon tart"), ("Cleo", 0, "Quiche")] {
        let id = join(&t, &code, name).await;
        pocket::act(&t.state, &code, &Viewer::Guest(id), "take", &json!({ "slot": slot, "note": note })).await.unwrap();
    }
    pocket::end(&t.state, &code).await.unwrap();
    schedule::run_once(&t.state, now + Duration::minutes(1)).await;
    let post = results_post(&t, &k, &code).await;
    assert_eq!(post.text, "Party potluck has ended. 3 people took part.");
    let b = pocket_block(&post).expect("the ended card");
    common_card(&b, "gather", "ended");
    let g = gather_card(&b, "signup", "closed");
    let slots = g["slots"].as_array().unwrap();
    let shape: Vec<(&str, u64, u64)> = slots.iter().map(|s| (s["name"].as_str().unwrap(), s["filled"].as_u64().unwrap(), s["waiting"].as_u64().unwrap())).collect();
    assert_eq!(shape, [("Mains", 1, 1), ("Desserts", 1, 0)], "the counts stay: {g}");
    for s in slots {
        assert_eq!((&s["people"], &s["waiting_people"]), (&json!([]), &json!([])), "{s}");
    }
    for gone in ["Ana", "Ben", "Cleo", "Lasagne", "Zofia", "Lemon tart", "Quiche"] {
        assert!(!b.to_string().contains(gone), "{gone} in the posted card: {b}");
    }
}

/// The message as the chat loads it, with its stored card.
async fn served(db: &Database, id: &str) -> Message {
    let mut v = vec![db.get_message(id).await.unwrap().unwrap()];
    db.attach_results(&mut v).await;
    v.remove(0)
}

/// Rigel's QA finding 1: the RSVP, sign-up or vote card under the bot's reply is the host's view
/// as of the reply, guest names and words included. The host keeps seeing them until the
/// pocket's "Names and answers are deleted on …" date; once that passes, the stored card (and
/// the chat's live copy) holds no guest name, voter, unpaid list or note.
#[tokio::test]
async fn a_reply_card_loses_its_guests_names_and_notes_when_the_pocket_expires() {
    let e = chat_setup(true).await;
    e.state.db.set_setting(pocket::address::PUBLIC_URL_SETTING, GUEST_BASE).await.unwrap();
    for (face, args) in [("rsvp", rsvp()), ("signup", potluck()), ("vote", theme_vote())] {
        let (code, text) = make(&e, "gather_create", args.clone()).await;
        for (name, words) in [("Ana", "Zofia's lemon tart"), ("Ben", "Quiche for Marek")] {
            let (g, _) = pocket::join(&e.state, &code, JoinRequest { name: name.into(), avatar: None, team: None, pin: Some(PIN.into()) }).await.unwrap();
            let (action, body) = match face {
                "rsvp" => ("answer", json!({ "answer": "yes", "food_other": words })),
                "signup" => ("take", json!({ "slot": 0, "note": words })),
                _ => ("vote", json!({ "ranking": [0] })),
            };
            pocket::act(&e.state, &code, &Viewer::Guest(g.id), action, &body).await.unwrap_or_else(|err| panic!("{face} {action}: {err:?}"));
        }
        let m = reply(&e, "gather_create", args, &text).await;
        let names: &[&str] = match face {
            "vote" => &["Ana", "Ben", "Cleo"],
            _ => &["Ana", "Ben", "Zofia", "Marek"],
        };
        let before = pocket_block(&m).unwrap_or_else(|| panic!("{face}: no card in {:?}", m.results));
        assert!(before.to_string().contains("Ana"), "{face}: the host sees names until the date: {before}");
        let expires = pocket::snapshot(&e.state, &code).await.unwrap().unwrap().row.expires_at.unwrap_or_else(|| panic!("{face}: a deletion date"));
        // A run before the date keeps them
        schedule::run_once(&e.state, expires - Duration::minutes(5)).await;
        let kept = pocket_block(&served(&e.state.db, &m.id).await).expect("still drawn");
        assert!(kept.to_string().contains("Ana"), "{face}: names stay before the date: {kept}");
        // The run past the date blanks the stored card and tells the open chat
        let mut rx = e.state.event_sender.subscribe();
        let ran = schedule::run_once(&e.state, expires + Duration::minutes(1)).await;
        assert!(ran.names_deleted >= 1, "{face}: {ran:?}");
        let after = served(&e.state.db, &m.id).await;
        let b = pocket_block(&after).unwrap_or_else(|| panic!("{face}: the card still draws, without names: {:?}", after.results));
        common_card(&b, "gather", "live");
        let g = gather_card(&b, face, "open");
        for k in ["people", "voters", "not_answered", "drafts", "moved_up"] {
            assert!(g.get(k).is_none_or(|v| v.is_null() || v == &json!([])), "{face}: {k} in {g}");
        }
        assert!(g["paid"].get("not_yet").is_none_or(|v| v == &json!([])), "{face}: {g}");
        for name in names {
            assert!(!b.to_string().contains(name), "{face}: {name} in the stored reply card: {b}");
        }
        let mut live = None;
        while let Ok(ev) = rx.try_recv() {
            if let kwnt::drivers::events::RuntimeEvent::MessageAdded { message } = ev {
                if message["id"] == json!(m.id) {
                    live = Some(message);
                }
            }
        }
        let live = live.unwrap_or_else(|| panic!("{face}: the open chat hears of it"));
        for name in names {
            assert!(!live["results"].to_string().contains(name), "{face}: {name} in the chat's copy: {live}");
        }
        // Once: a later run finds nothing left to clear for this message
        let again = schedule::run_once(&e.state, expires + Duration::minutes(2)).await;
        assert_eq!(again.names_deleted, 0, "{face}: {again:?}");
    }
}

/// A JPEG header with no metadata, enough for the wall to take it.
fn jpeg() -> Vec<u8> {
    let seg = |marker: u8, payload: &[u8]| {
        let mut s = vec![0xFF, marker];
        s.extend(((payload.len() + 2) as u16).to_be_bytes());
        s.extend(payload);
        s
    };
    let mut j = vec![0xFF, 0xD8];
    j.extend(seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    j.extend(seg(0xC0, &[8, 0x01, 0xE0, 0x02, 0x80, 1, 1, 0x11, 0]));
    j.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
    j.extend([0x12, 0x34, 0xFF, 0x00, 0x56]);
    j.extend([0xFF, 0xD9]);
    j
}

async fn add_photo(t: &T, code: &str, guest: &str) -> String {
    wall::add_photo(&t.state, code, guest, Bytes::from(jpeg()), (None, false)).await.unwrap()
}

#[tokio::test]
async fn waiting_photos_are_one_needs_you_row_that_approve_and_hide_clear() {
    let t = setup().await;
    let k = bot(&t, "Party").await;
    let spec = json!({"title": "Office holiday party", "host_name": "Nina", "closes_at": (Utc::now() + Duration::days(1)).to_rfc3339()});
    let code = create(&t, "wall", spec, &k).await;
    assert!(of_kind(&inbox(&t).await, "pocket_wall").is_empty(), "nothing waits yet");
    let cleo = join(&t, &code, "Cleo Marsh").await;
    let first = add_photo(&t, &code, &cleo).await;
    // One photo: one row, singular
    let body = inbox(&t).await;
    let rows = of_kind(&body, "pocket_wall");
    assert_eq!(rows.len(), 1, "{body}");
    let row = &rows[0];
    assert_eq!(row["title"], "1 photo waiting on Office holiday party");
    assert_eq!((row["group"].as_str(), row["kwant_id"].as_str(), row["pocket_code"].as_str()), (Some("waiting_on_you"), Some(k.id.as_str()), Some(code.as_str())), "{row}");
    assert_eq!(row["next_photo"], json!({"id": first, "by_name": "Cleo Marsh"}));
    assert_eq!(row["link"]["direct_kwant_id"], json!(k.id), "the row opens the chat with the card");
    // Approve from the row clears it
    let (status, out) = send(&t.main, "POST", &format!("/api/pockets/{code}/action"), Some(json!({"name": "approve", "body": {"ids": [first]}}))).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(of_kind(&inbox(&t).await, "pocket_wall").is_empty(), "approved: nothing waits");

    // Three waiting from two guests: still one row, plural, the oldest first
    let dev = join(&t, &code, "Dev").await;
    let a = add_photo(&t, &code, &dev).await;
    let b = add_photo(&t, &code, &cleo).await;
    let c = add_photo(&t, &code, &dev).await;
    let rows = of_kind(&inbox(&t).await, "pocket_wall");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["title"], "3 photos waiting on Office holiday party");
    assert_eq!(rows[0]["next_photo"]["id"], json!(a));
    // Hide takes it off the list too; the next one comes up
    let (status, out) = send(&t.main, "POST", &format!("/api/pockets/{code}/action"), Some(json!({"name": "hide", "body": {"ids": [a]}}))).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let rows = of_kind(&inbox(&t).await, "pocket_wall");
    assert_eq!(rows[0]["title"], "2 photos waiting on Office holiday party");
    assert_eq!(rows[0]["next_photo"]["id"], json!(b));
    let (status, _) = send(&t.main, "POST", &format!("/api/pockets/{code}/action"), Some(json!({"name": "approve", "body": {"ids": [b, c]}}))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(of_kind(&inbox(&t).await, "pocket_wall").is_empty());

    // With the wall switched off, no row even while a photo waits
    add_photo(&t, &code, &cleo).await;
    assert_eq!(of_kind(&inbox(&t).await, "pocket_wall").len(), 1);
    t.state.db.set_setting(pocket::SETTING, "gather").await.unwrap();
    assert!(of_kind(&inbox(&t).await, "pocket_wall").is_empty(), "the wall kit is off");
    t.state.db.set_setting(pocket::SETTING, "on").await.unwrap();

    // An ended wall waits for nobody; its post says how many photos it kept, on a closed card
    pocket::end(&t.state, &code).await.unwrap();
    assert!(of_kind(&inbox(&t).await, "pocket_wall").is_empty(), "ended");
    schedule::run_once(&t.state, Utc::now() + Duration::minutes(1)).await;
    let post = results_post(&t, &k, &code).await;
    assert_eq!(post.text, "Office holiday party has ended. 3 photos on the wall.");
    let card = pocket_block(&post).expect("the ended card");
    common_card(&card, "wall", "ended");
    let w = wall_card(&card, "closed");
    assert_eq!(w["counts"]["approved"], 3, "{w}");
}

#[tokio::test]
async fn another_spaces_wall_is_not_this_spaces_row() {
    let t = setup().await;
    let space = t.state.db.create_space(&kwnt::db::spaces::NewSpace { name: "Elsewhere".into(), icon: "E".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let input = NewKwant {
        name: "Away".into(),
        title: "Party host".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    let space = SpaceId::new(space.id);
    let away = t.state.db.create_kwant(&space, &input).await.unwrap();
    let code = create(&t, "wall", json!({"title": "Their party", "closes_at": (Utc::now() + Duration::days(1)).to_rfc3339()}), &away).await;
    let g = join(&t, &code, "Zora").await;
    add_photo(&t, &code, &g).await;
    assert!(of_kind(&inbox(&t).await, "pocket_wall").is_empty(), "the default space's Inbox");
    let rows = kwnt::server::inbox_pockets::wall_needs_you(&t.state, &space).await.unwrap();
    assert_eq!(rows.len(), 1, "its own space's");
}
