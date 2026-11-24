//! pa14b the Wall's best 30: the morning after the wall closes, one scheduled pick. More than 30
//! approved photos: the wall's bot gets one turn and answers with `pocket_wall_propose_best`; 30
//! or fewer: all of them are proposed without a bot call. The host's own pick is never
//! overridden, guests never see the proposal, and a failed turn leaves no proposal. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa14/pa14b-brief.md, API in
//! pa14b-impl.md next to it. The state rules alone are unit tests in `src/pocket/wall.rs`, the
//! times, prompt, lines and previews in `src/pocket/wall_pick.rs`.
//!
//! The fake claude answers the pick's turn: a marker planted in the bot's direct chat (its turn
//! reads the chat's history) makes it call the propose tool, or fail.

mod common;

use axum::body::Bytes;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use common::{call, call_tool_marker, invocations_with, setup, wait_for, wait_reply, Env};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{MessageExtra, NewMessage},
        Database,
    },
    pocket::{
        self,
        kits::{KitCtx, Viewer},
        schedule::{self, Origin},
        wall, CreateRequest, JoinRequest,
    },
    router::LayaRouter,
    server::AppState,
    tools::ToolRunner,
};
use serde_json::{json, Value};

const PROPOSE: &str = "pocket_wall_propose_best";

async fn on(env: &Env) {
    env.state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    // The person's zone: UTC, so the pick runs at 09:00 UTC
    env.state.db.set_setting("timezone", "UTC").await.unwrap();
}

/// The server started again on the same data: nothing in memory survives.
async fn restart(env: &Env) -> AppState {
    let data = env.tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
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

/// The first 09:00 UTC after `t`, plus a minute.
fn morning_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let nine = |d: chrono::NaiveDate| d.and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap()).and_utc();
    let same = nine(t.date_naive());
    let at = if same > t { same } else { nine(t.date_naive() + Duration::days(1)) };
    at + Duration::minutes(1)
}

struct Wall {
    code: String,
    guest: String,
    /// Approved photo ids, oldest first.
    ids: Vec<String>,
    closes: DateTime<Utc>,
}

/// A wall from `bot`'s direct chat (or none) with `n` approved photos from one guest; uploads
/// stop in an hour.
async fn wall_with(env: &Env, bot: Option<&str>, n: usize) -> Wall {
    let closes = Utc::now() + Duration::hours(1);
    let spec = json!({ "title": "Mia & Sam", "closes_at": closes, "per_guest_max": 100 });
    let origin = bot.map(|b| Origin { kwant_id: b.to_string(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&env.state, CreateRequest { kit: "wall".into(), spec, origin, ..Default::default() }).await.unwrap();
    let code = s.row.code.clone();
    let (guest, _) = pocket::join(&env.state, &code, JoinRequest { name: "Zora".into(), pin: Some("4821".into()), ..Default::default() }).await.unwrap();
    let mut ids = Vec::new();
    for _ in 0..n {
        ids.push(wall::add_photo(&env.state, &code, &guest.id, Bytes::from(jpeg()), (None, false)).await.unwrap());
    }
    pocket::act(&env.state, &code, &Viewer::Host, "approve_all_from", &json!({ "guest": guest.id })).await.unwrap();
    Wall { code, guest: guest.id, ids, closes }
}

async fn view(state: &AppState, code: &str, viewer: &Viewer) -> Value {
    let s = pocket::snapshot(state, code).await.unwrap().unwrap();
    pocket::view_json(&s, viewer, Utc::now())["view"].clone()
}

async fn host(env: &Env, code: &str) -> Value {
    view(&env.state, code, &Viewer::Host).await
}

async fn lobby(env: &Env, code: &str) -> Value {
    let s = pocket::snapshot(&env.state, code).await.unwrap().unwrap();
    let ctx = KitCtx::new(&s.spec, &s.players, Utc::now());
    s.kit().unwrap().lobby(&ctx, &s.kit_state)
}

/// Plants a line in the bot's direct chat for its next turn to read (no turn starts).
async fn plant(env: &Env, bot: &str, text: &str) {
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

/// The pick's turns for this wall so far (its prompt names the pocket's code).
fn turns(code: &str) -> usize {
    invocations_with(&format!("(pocket {code})")).len()
}

/// Every key of a view, anywhere in it.
fn has_key(v: &Value, key: &str) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| k == key || has_key(x, key)),
        Value::Array(a) => a.iter().any(|x| has_key(x, key)),
        _ => false,
    }
}

#[tokio::test]
async fn more_than_30_the_bot_picks_once_and_the_host_says_yes() {
    let env = setup().await;
    on(&env).await;
    let w = wall_with(&env, Some(&env.a), 35).await;
    let pick: Vec<String> = w.ids[5..].to_vec();
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": pick }))).await;

    // Not the evening it closes, and not before 09:00 the morning after
    let at = morning_after(w.closes);
    pocket::sweep(&env.state, w.closes + Duration::minutes(1)).await;
    schedule::run_once(&env.state, w.closes + Duration::minutes(1)).await;
    schedule::run_once(&env.state, at - Duration::minutes(2)).await;
    assert_eq!(turns(&w.code), 0, "not due yet");
    assert!(host(&env, &w.code).await["proposed_best"].as_array().is_some_and(Vec::is_empty));

    schedule::run_once(&env.state, at).await;
    // The tick again while it runs, End and the sweep: still the one turn
    schedule::run_once(&env.state, at + Duration::minutes(1)).await;
    let _ = pocket::end(&env.state, &w.code).await;
    pocket::sweep(&env.state, at + Duration::minutes(1)).await;
    wait_for("the proposal", || async { ids_of(&host(&env, &w.code).await["proposed_best"]).len() == 30 }).await;
    let h = host(&env, &w.code).await;
    assert_eq!(ids_of(&h["proposed_best"]), pick);
    assert_eq!(h["picking"], false);
    assert_eq!(h["best_by"], Value::Null, "nothing is shown until the host says yes");
    assert_eq!(h["budget"]["bot_calls"], 1);
    assert!(h["budget"]["line"].as_str().unwrap().starts_with("1 bot call so far"), "{}", h["budget"]);
    let ready = wait_reply(&env, &env.a, "guests see them only after you say yes").await;
    assert!(ready.text.starts_with("I picked the best 30 photos from Mia & Sam. Look them over on the card"), "{}", ready.text);
    assert!(ready.text.contains("All the photos leave this box on"), "says when the photos leave: {}", ready.text);
    assert!(!ready.text.contains("Zora") && !ready.text.contains(&w.guest), "no guest names: {}", ready.text);

    // Guests, the lobby and the big screen never see the proposal
    for (who, v) in [
        ("guest", view(&env.state, &w.code, &Viewer::Guest(w.guest.clone())).await),
        ("screen", view(&env.state, &w.code, &Viewer::BigScreen).await),
        ("lobby", lobby(&env, &w.code).await),
    ] {
        for key in ["proposed_best", "picking", "budget"] {
            assert!(!has_key(&v, key), "{who} has {key}: {v}");
        }
    }

    // Again, and after a restart: never a second turn
    schedule::run_once(&env.state, at + Duration::hours(1)).await;
    let again = restart(&env).await;
    schedule::run_once(&again, at + Duration::hours(2)).await;
    schedule::run_once(&again, at + Duration::days(1)).await;
    assert_eq!(turns(&w.code), 1, "one turn per pocket");

    // The host says yes to the 30 unchanged (after End): the bot's pick, said once in the chat
    let mut yes = pick.clone();
    yes.reverse();
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": yes })).await.unwrap();
    assert_eq!(host(&env, &w.code).await["best_by"], "snap");
    let guest = view(&env.state, &w.code, &Viewer::Guest(w.guest.clone())).await;
    assert_eq!(guest["best_by"], "snap");
    assert_eq!(guest["wall"].as_array().unwrap().iter().filter(|p| p["best"] == true).count(), 30);
    let shown = wait_reply(&env, &env.a, "showed them to guests after you said yes").await;
    assert!(shown.text.contains("I picked the best 30"), "{}", shown.text);
    assert!(shown.text.contains("with 35 photos"), "{}", shown.text);
    assert!(shown.text.contains("leave this box on"), "says when the photos leave: {}", shown.text);
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": pick })).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let lines = common::messages(&env, &env.a).await;
    assert_eq!(lines.iter().filter(|m| m.text.contains("showed them to guests")).count(), 1, "said once");

    // One swap: the host's pick
    let mut swapped = pick.clone();
    swapped[0] = w.ids[0].clone();
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": swapped })).await.unwrap();
    assert_eq!(host(&env, &w.code).await["best_by"], "host");
    assert!(!has_key(&view(&env.state, &w.code, &Viewer::Guest(w.guest.clone())).await, "best_by"));
}

#[tokio::test]
async fn thirty_or_fewer_are_all_proposed_without_a_bot_call() {
    let env = setup().await;
    on(&env).await;
    let w = wall_with(&env, Some(&env.a), 12).await;
    plant(&env, &env.a, "FAILTURN").await;
    schedule::run_once(&env.state, morning_after(w.closes)).await;
    let h = host(&env, &w.code).await;
    assert_eq!(ids_of(&h["proposed_best"]), w.ids, "all 12");
    assert_eq!(h["picking"], false);
    assert_eq!(h["budget"]["bot_calls"], 0);
    let line = wait_reply(&env, &env.a, "guests see them only after you say yes").await;
    assert!(line.text.starts_with("Mia & Sam closed with 12 photos, so all 12 are in the best."), "{}", line.text);
    assert!(line.text.contains("All the photos leave this box on"), "says when the photos leave: {}", line.text);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(turns(&w.code), 0, "no bot call");
    // The host says yes to all 12 unchanged: still the host's, since no bot picked anything
    // (copy check at 3032d327: no holo sticker, no state-8 line)
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": w.ids })).await.unwrap();
    assert_eq!(host(&env, &w.code).await["best_by"], "host");
    let guest = view(&env.state, &w.code, &Viewer::Guest(w.guest.clone())).await;
    assert!(!has_key(&guest, "best_by"), "{guest}");
    assert_eq!(guest["wall"].as_array().unwrap().iter().filter(|p| p["best"] == true).count(), 12);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let lines = common::messages(&env, &env.a).await;
    assert!(!lines.iter().any(|m| m.text.contains("showed them to guests")), "no state-8 line");
    // The host drops one: the host's pick too
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": &w.ids[1..] })).await.unwrap();
    assert_eq!(host(&env, &w.code).await["best_by"], "host");
}

#[tokio::test]
async fn the_hosts_own_pick_is_never_overridden() {
    let env = setup().await;
    on(&env).await;
    let w = wall_with(&env, Some(&env.a), 35).await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": &w.ids[..30] }))).await;
    let mine = &w.ids[..3];
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": mine })).await.unwrap();
    let at = morning_after(w.closes);
    schedule::run_once(&env.state, at).await;
    schedule::run_once(&restart(&env).await, at + Duration::hours(1)).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(turns(&w.code), 0, "no pick after a hand pick");
    let h = host(&env, &w.code).await;
    assert_eq!(ids_of(&h["best"]), mine);
    assert_eq!(h["best_by"], "host");
    assert!(h["proposed_best"].as_array().is_some_and(Vec::is_empty), "{}", h["proposed_best"]);
}

/// The turn ends without a usable proposal: no proposal, not picking, the host stars by hand,
/// and no second turn (not on the next tick, not after a restart).
async fn fails(env: &Env, w: &Wall) {
    let at = morning_after(w.closes);
    schedule::run_once(&env.state, at).await;
    wait_for("the turn", || async { turns(&w.code) == 1 }).await;
    let line = wait_reply(env, &env.a, "I couldn't pick the best 30 from Mia & Sam").await;
    assert!(line.text.contains("You can star the best ones yourself on the card."), "{}", line.text);
    assert!(line.text.contains("All the photos leave this box on"), "says when the photos leave: {}", line.text);
    let h = host(env, &w.code).await;
    assert_eq!(h["picking"], false);
    assert!(h["proposed_best"].as_array().is_some_and(Vec::is_empty), "{}", h["proposed_best"]);
    schedule::run_once(&env.state, at + Duration::minutes(1)).await;
    schedule::run_once(&restart(env).await, at + Duration::hours(1)).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(turns(&w.code), 1, "not retried");
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": &w.ids[..2] })).await.unwrap();
    assert_eq!(host(env, &w.code).await["best_by"], "host");
}

#[tokio::test]
async fn a_failed_turn_leaves_no_proposal() {
    let env = setup().await;
    on(&env).await;
    let w = wall_with(&env, Some(&env.a), 31).await;
    plant(&env, &env.a, "FAILTURN").await;
    fails(&env, &w).await;
}

#[tokio::test]
async fn bad_ids_are_refused_and_leave_no_proposal() {
    let env = setup().await;
    on(&env).await;
    let other = wall_with(&env, None, 1).await;
    let w = wall_with(&env, Some(&env.a), 35).await;
    // 31 ids, one of them another wall's: refused, so the turn ends with nothing (each kind of
    // bad list alone is a unit test in wall.rs)
    let mut ids: Vec<String> = w.ids[..30].to_vec();
    ids.push(other.ids[0].clone());
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": ids }))).await;
    fails(&env, &w).await;
    // The other wall closed too, so the same tick gave it its own pick (1 approved: all of it,
    // no bot call). The refused list put nothing there: only its own photo, nothing starred.
    let o = host(&env, &other.code).await;
    assert_eq!(ids_of(&o["proposed_best"]), other.ids, "nothing reached the other wall: {o}");
    assert!(ids_of(&o["best"]).is_empty(), "{o}");
    assert_eq!(o["budget"]["bot_calls"], 0, "{o}");
}

/// A restart cut the pick off `tries` times: `picking` is left over and nothing runs here.
async fn leave_picking(env: &Env, code: &str, tries: u64) {
    let s = pocket::snapshot(&env.state, code).await.unwrap().unwrap();
    let mut st = s.kit_state.clone();
    st["picking"] = json!(true);
    st["pick_tries"] = json!(tries);
    st["bot_calls"] = json!(tries);
    env.state.db.pocket_save_state(code, &st.to_string(), &[], Utc::now()).await.unwrap();
}

/// QA F4 at 3032d327: cut off twice, no third try, and the host hears it didn't work (once).
#[tokio::test]
async fn tries_used_up_says_it_couldnt_pick() {
    let env = setup().await;
    on(&env).await;
    let w = wall_with(&env, Some(&env.a), 35).await;
    plant(&env, &env.a, &call_tool_marker(PROPOSE, json!({ "code": w.code, "ids": &w.ids[..30] }))).await;
    leave_picking(&env, &w.code, 2).await;
    // The server that comes back up (the leftover was written under the old one's cache)
    let again = restart(&env).await;
    let at = morning_after(w.closes);
    schedule::run_once(&again, at).await;
    let line = wait_reply(&env, &env.a, "I couldn't pick the best 30 from Mia & Sam").await;
    assert!(line.text.contains("All the photos leave this box on"), "{}", line.text);
    schedule::run_once(&again, at + Duration::hours(1)).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(turns(&w.code), 0, "no third try");
    let lines = common::messages(&env, &env.a).await;
    assert_eq!(lines.iter().filter(|m| m.text.contains("I couldn't pick the best 30")).count(), 1, "said once");
    let h = view(&again, &w.code, &Viewer::Host).await;
    assert_eq!(h["picking"], false, "{h}");
    assert!(ids_of(&h["proposed_best"]).is_empty());
}

#[tokio::test]
async fn a_deleted_bot_means_no_pick() {
    let env = setup().await;
    on(&env).await;
    let w = wall_with(&env, Some(&env.b), 35).await;
    let (s, v) = call(&env.app, "DELETE", &format!("/api/kwants/{}", env.b), None).await;
    assert!(s.is_success(), "{s} {v}");
    schedule::run_once(&env.state, morning_after(w.closes)).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(turns(&w.code), 0);
    let h = host(&env, &w.code).await;
    assert_eq!(h["picking"], false);
    assert!(h["proposed_best"].as_array().is_some_and(Vec::is_empty), "{}", h["proposed_best"]);
    pocket::act(&env.state, &w.code, &Viewer::Host, "set_best", &json!({ "ids": &w.ids[..2] })).await.unwrap();
    assert_eq!(host(&env, &w.code).await["best_by"], "host");
}
