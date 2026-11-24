//! Rigel's QA probes for pa14b (the Wall's best 30), on top of Juno's own tests: a restart in the
//! middle of a pick, what still works after End, End before `closes_at`, a wall with no
//! `closes_at`, the pick's tools for a bot that isn't picking, and the plain look of the lines.

mod common;

use axum::body::Bytes;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use common::{call_tool_marker, invocations_with, setup, wait_for, wait_reply, Env};
use kwnt::pocket::{
    self,
    kits::Viewer,
    schedule::{self, Origin},
    wall, wall_pick, CreateRequest, JoinRequest,
};
use kwnt::{
    config::Config,
    db::Database,
    router::LayaRouter,
    broker::PermissionBroker,
    server::AppState,
    tools::ToolRunner,
};
use serde_json::{json, Value};

const PROPOSE: &str = "pocket_wall_propose_best";
const PHOTOS: &str = "pocket_wall_photos";

async fn on(env: &Env) {
    env.state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    env.state.db.set_setting("timezone", "UTC").await.unwrap();
}

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

fn morning_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let nine = |d: chrono::NaiveDate| d.and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap()).and_utc();
    let same = nine(t.date_naive());
    let at = if same > t { same } else { nine(t.date_naive() + Duration::days(1)) };
    at + Duration::minutes(1)
}

struct W {
    code: String,
    guest: String,
    ids: Vec<String>,
}

async fn wall(env: &Env, bot: Option<&str>, n: usize, closes: Option<DateTime<Utc>>) -> W {
    wall_titled(env, bot, n, closes, "Mia & Sam").await
}

async fn wall_titled(env: &Env, bot: Option<&str>, n: usize, closes: Option<DateTime<Utc>>, title: &str) -> W {
    let mut spec = json!({ "title": title, "per_guest_max": 100 });
    if let Some(c) = closes {
        spec["closes_at"] = json!(c);
    }
    let origin = bot.map(|b| Origin { kwant_id: b.to_string(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&env.state, CreateRequest { kit: "wall".into(), spec, origin, ..Default::default() }).await.unwrap();
    let code = s.row.code.clone();
    let (guest, _) = pocket::join(&env.state, &code, JoinRequest { name: "Zora".into(), pin: Some("4821".into()), ..Default::default() }).await.unwrap();
    let mut ids = Vec::new();
    for _ in 0..n {
        ids.push(wall::add_photo(&env.state, &code, &guest.id, Bytes::from(jpeg()), (None, false)).await.unwrap());
    }
    pocket::act(&env.state, &code, &Viewer::Host, "approve_all_from", &json!({ "guest": guest.id })).await.unwrap();
    W { code, guest: guest.id, ids }
}

/// A server that comes back up on the same data: its snapshots aren't the old server's cache.
async fn restart(env: &Env) -> AppState {
    let data = env.tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
}

async fn view_on(state: &AppState, code: &str) -> Value {
    let s = pocket::snapshot(state, code).await.unwrap().unwrap();
    pocket::view_json(&s, &Viewer::Host, Utc::now())["view"].clone()
}

async fn host(env: &Env, code: &str) -> Value {
    let s = pocket::snapshot(&env.state, code).await.unwrap().unwrap();
    pocket::view_json(&s, &Viewer::Host, Utc::now())["view"].clone()
}

async fn plant(env: &Env, bot: &str, text: &str) {
    use kwnt::db::models::{MessageExtra, NewMessage};
    let m = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(bot.to_string()),
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: text.to_string(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    env.state.db.insert_message_with(&m, &MessageExtra { thread_id: None, meta: None }).await.unwrap();
}

fn ids_of(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn turns(code: &str) -> usize {
    invocations_with(&format!("(pocket {code})")).len()
}

/// Leaves the wall's state the way a restart in the middle of a pick would: `picking`, tries used.
async fn leave_picking(env: &Env, code: &str, tries: u64) {
    let s = pocket::snapshot(&env.state, code).await.unwrap().unwrap();
    let mut st = s.kit_state.clone();
    st["picking"] = json!(true);
    st["pick_tries"] = json!(tries);
    st["bot_calls"] = json!(tries);
    env.state.db.pocket_save_state(code, &st.to_string(), &[], Utc::now()).await.unwrap();
}

#[tokio::test]
async fn qa_restart_in_the_middle_of_a_pick_asks_once_more_then_never() {
    let env = setup().await;
    on(&env).await;
    let closes = Utc::now() + Duration::hours(1);
    let w = wall(&env, Some(&env.a), 35, Some(closes)).await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": &w.ids[..30] }))).await;
    // The first try was cut off by a restart (nothing runs here for this wall)
    leave_picking(&env, &w.code, 1).await;
    let again = restart(&env).await;
    let at = morning_after(closes);
    schedule::run_once(&again, at - Duration::minutes(5)).await;
    assert_eq!(turns(&w.code), 0, "not before 09:00 even for a leftover");
    schedule::run_once(&again, at).await;
    schedule::run_once(&again, at + Duration::minutes(1)).await;
    // The restarted state isn't the one the fake CLI's tool call reaches (that is the env's own
    // server, on its port), so the proposal can't land here: what is checked is that the second
    // try STARTED (one turn, bot_calls 2) and that no third turn follows.
    wait_for("the second try's turn", || async { turns(&w.code) == 1 }).await;
    let h = view_on(&again, &w.code).await;
    assert_eq!(h["budget"]["bot_calls"], 2, "{}", h["budget"]);
    schedule::run_once(&again, at + Duration::hours(3)).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(turns(&w.code), 1, "exactly one turn for the leftover, not more");

    // Cut off twice: the third time never comes, nothing is proposed, the host can star
    let w2 = wall_titled(&env, Some(&env.a), 35, Some(closes), "Zed & Kim").await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w2.code, "ids": &w2.ids[..30] }))).await;
    leave_picking(&env, &w2.code, 2).await;
    let again = restart(&env).await;
    schedule::run_once(&again, at).await;
    schedule::run_once(&again, at + Duration::hours(1)).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(turns(&w2.code), 0, "no third try");
    let h2 = view_on(&again, &w2.code).await;
    assert_eq!(h2["picking"], false, "{h2}");
    assert!(ids_of(&h2["proposed_best"]).is_empty());
    pocket::act(&again, &w2.code, &Viewer::Host, "set_best", &json!({ "ids": &w2.ids[..2] })).await.unwrap();
    assert_eq!(view_on(&again, &w2.code).await["best_by"], "host");
    // Round 2 (QA F4): the leftover that used up both tries tells the host once
    wait_reply(&env, &env.a, "I couldn't pick the best 30 from Zed & Kim").await;
    let lines = common::messages(&env, &env.a).await;
    let said: Vec<_> = lines.iter().filter(|m| m.text.contains("I couldn't pick the best 30") && m.text.contains("Zed & Kim")).collect();
    assert_eq!(said.len(), 1, "the second wall (both tries used up) is told to the host exactly once");
    assert!(said[0].text.contains("All the photos leave this box on") || said[0].text.contains("guests don't see a best set yet"), "{}", said[0].text);
}

#[tokio::test]
async fn qa_after_end_only_set_best_works() {
    let env = setup().await;
    on(&env).await;
    let w = wall(&env, Some(&env.a), 3, Some(Utc::now() + Duration::hours(1))).await;
    pocket::end(&env.state, &w.code).await.unwrap();
    for (name, body) in [
        ("approve", json!({ "id": w.ids[0] })),
        ("hide", json!({ "id": w.ids[0] })),
        ("approve_all_from", json!({ "guest": w.guest })),
    ] {
        let e = pocket::act(&env.state, &w.code, &Viewer::Host, name, &body).await.expect_err(name);
        assert_eq!((e.status, e.code), (StatusCode::GONE, "ended"), "{name}");
    }
    // A guest can't do anything either, set_best included
    let e = pocket::act(&env.state, &w.code, &Viewer::Guest(w.guest.clone()), "set_best", &json!({ "ids": [w.ids[0]] })).await.expect_err("guest set_best");
    assert!(e.status.is_client_error(), "{} {}", e.status, e.code);
    assert_eq!(host(&env, &w.code).await["best"].as_array().map(Vec::len), Some(0));
    // The host's own set_best works, and refuses what isn't an approved photo
    let e = pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": ["ph_nope"] })).await.expect_err("unknown id");
    assert!(e.status.is_client_error());
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": [w.ids[0]] })).await.unwrap();
    assert_eq!(ids_of(&host(&env, &w.code).await["best"]), vec![w.ids[0].clone()]);
}

#[tokio::test]
async fn qa_end_before_closes_at_counts_from_the_end() {
    let env = setup().await;
    on(&env).await;
    // Closes in three days, but the host ends it now: the pick follows the End, not closes_at
    let w = wall(&env, Some(&env.a), 35, Some(Utc::now() + Duration::days(3))).await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": &w.ids[..30] }))).await;
    let ended = pocket::end(&env.state, &w.code).await.unwrap().row.ended_at.expect("ended_at");
    let at = morning_after(ended);
    schedule::run_once(&env.state, at - Duration::minutes(3)).await;
    assert_eq!(turns(&w.code), 0, "not before 09:00 the morning after End");
    schedule::run_once(&env.state, at).await;
    wait_for("the proposal", || async { ids_of(&host(&env, &w.code).await["proposed_best"]).len() == 30 }).await;
    assert_eq!(turns(&w.code), 1);
}

#[tokio::test]
async fn qa_a_wall_with_no_closes_at_never_picks_while_open() {
    let env = setup().await;
    on(&env).await;
    let w = wall(&env, Some(&env.a), 35, None).await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": &w.ids[..30] }))).await;
    for d in [1, 10, 60] {
        schedule::run_once(&env.state, Utc::now() + Duration::days(d)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(turns(&w.code), 0);
    let h = host(&env, &w.code).await;
    assert_eq!((h["picking"].clone(), h["budget"]["bot_calls"].clone()), (json!(false), json!(0)));
}

#[tokio::test]
async fn qa_the_pick_tools_refuse_a_bot_that_is_not_picking() {
    let env = setup().await;
    on(&env).await;
    let w = wall(&env, Some(&env.a), 35, Some(Utc::now() + Duration::hours(1))).await;
    for bot in [&env.a, &env.b] {
        let (c, err) = wall_pick::call(&env.state, bot, PROPOSE, &json!({ "code": w.code, "ids": &w.ids[..3] })).await;
        assert!(err, "{c}");
        assert!(c.to_string().contains("aren't picking"), "{c}");
        let (c, err) = wall_pick::call(&env.state, bot, PHOTOS, &json!({ "code": w.code })).await;
        assert!(err, "{c}");
        assert!(!c.to_string().contains("\"image\""), "no preview for a bot that isn't picking: {c}");
    }
    let (_, err) = wall_pick::call(&env.state, &env.a, PROPOSE, &json!({ "ids": ["x"] })).await;
    assert!(err, "no code");
    let h = host(&env, &w.code).await;
    assert!(ids_of(&h["proposed_best"]).is_empty() && h["picking"] == false);
}

#[tokio::test]
async fn qa_the_chat_lines_are_plain_text_without_guest_names() {
    let env = setup().await;
    on(&env).await;
    let closes = Utc::now() + Duration::hours(1);
    // 30 or fewer: the "ready" line
    let w = wall(&env, Some(&env.a), 12, Some(closes)).await;
    schedule::run_once(&env.state, morning_after(closes)).await;
    let ready = wait_reply(&env, &env.a, "closed with 12 photos").await;
    assert!(ready.text.contains("guests see them only after you say yes"), "{}", ready.text);
    assert!(ready.text.contains("All the photos leave this box on"), "says when the photos leave: {}", ready.text);
    // More than 30: the bot's pick, the "ready" line and, after the host says yes, the state-8 line
    let w2 = wall_titled(&env, Some(&env.a), 35, Some(closes), "Zed & Kim").await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w2.code, "ids": &w2.ids[..30] }))).await;
    schedule::run_once(&env.state, morning_after(closes) + Duration::minutes(1)).await;
    wait_for("the proposal", || async { ids_of(&host(&env, &w2.code).await["proposed_best"]).len() == 30 }).await;
    pocket::act(&env.state, &w2.code, &Viewer::Host, "set_best", &json!({ "ids": &w2.ids[..30] })).await.unwrap();
    let shown = wait_reply(&env, &env.a, "showed them to guests after you said yes").await;
    for l in [&ready.text, &shown.text] {
        assert!(l.chars().all(|c| c.is_ascii() && !c.is_control()), "plain text only: {l:?}");
        assert!(!l.contains("Zora") && !l.contains(&w.guest) && !l.contains(&w.ids[0]) && !l.contains(&w2.ids[0]), "{l}");
    }
    assert!(shown.text.contains("leave this box on"), "says when the photos leave: {}", shown.text);
    assert!(shown.text.contains("with 35 photos"), "{}", shown.text);
}
