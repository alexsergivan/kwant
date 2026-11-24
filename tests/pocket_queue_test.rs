//! pa11b The Queue ticket kit: a QR on the counter, "Take a ticket", "You're 4th", "Call next".
//! Brief: /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa11/queue-server-brief.md,
//! API: pa11/queue-impl.md. Only the HTTP API and `pocket::sweep` are used, so these tests also
//! run (and fail) on a main without the kit: the red proof.
//!
//! No model is ever called: the fake claude (tests/common) must see no turn from any test here.

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{Duration, Local, Utc};
use common::{call, invocations_with, setup_with, Env};
use kwnt::pocket::{self, guest, listener};
use serde_json::{json, Value};
use tower::ServiceExt;

const GHOST: &str = "pocket.example.net";
const CLOSE_TEXT: &str = "Sorry, we've closed for today. Thanks for waiting, and see you next time!";

struct D {
    env: Env,
    guest: Router,
}

async fn setup() -> D {
    let env = setup_with(|s| s.config.pocket_port = Some(0)).await;
    env.state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    listener::reconcile(&env.state).await;
    let guest = guest::router(env.state.clone());
    D { env, guest }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

/// A request to the guest port from `ip` (a fresh one when `None`).
async fn send_from(d: &D, ip: Option<&str>, method: &str, uri: &str, body: Option<Value>, cookie: Option<(&str, &str)>, bearer: Option<&str>) -> Res {
    let ip = ip.map(str::to_string).unwrap_or_else(fresh_ip);
    let mut req = Request::builder().method(method).uri(uri).header("host", GHOST).header("content-type", "application/json").header("x-forwarded-for", ip);
    if let Some((k, v)) = cookie {
        req = req.header("cookie", format!("{k}={v}"));
    }
    if let Some(b) = bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    let req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let resp = d.guest.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Res { status, headers, body: serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())) }
}

async fn gsend(d: &D, method: &str, uri: &str, body: Option<Value>, cookie: Option<(&str, &str)>, bearer: Option<&str>) -> Res {
    send_from(d, None, method, uri, body, cookie, bearer).await
}

/// A queue made through the person's API: (code, host key).
async fn queue(d: &D, extra: Value) -> (String, String) {
    let mut spec = json!({ "title": "Tine's Bakery", "line": "Pick up at the side counter", "call_minutes": 3 });
    for (k, v) in extra.as_object().unwrap() {
        spec[k] = v.clone();
    }
    let (s, v) = call(&d.env.app, "POST", "/api/pockets", Some(json!({ "kit": "queue", "spec": spec }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (v["pocket"]["code"].as_str().unwrap().to_string(), v["host_key"].as_str().unwrap().to_string())
}

/// A guest who tapped "Take a ticket": joined with nothing typed. (player id, guest key).
async fn joined_from(d: &D, ip: Option<&str>, code: &str) -> (String, String) {
    let r = send_from(d, ip, "POST", &format!("/api/pocket/{code}/join"), Some(json!({})), None, None).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let cookie = guest::guest_cookie(code);
    let key = r
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| c.split(';').next()?.split_once('=').filter(|(k, _)| *k == cookie).map(|(_, v)| v.to_string()))
        .expect("the guest cookie");
    assert_eq!(r.body["me"]["name"], "", "no name is kept on the player: {}", r.body);
    (r.body["me"]["id"].as_str().unwrap().to_string(), key)
}

async fn joined(d: &D, code: &str) -> (String, String) {
    joined_from(d, None, code).await
}

async fn act_from(d: &D, ip: Option<&str>, code: &str, key: &str, name: &str, body: Value) -> Res {
    let cookie = guest::guest_cookie(code);
    send_from(d, ip, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": name, "body": body })), Some((&cookie, key)), None).await
}

async fn take(d: &D, code: &str, key: &str, body: Value) -> Res {
    act_from(d, None, code, key, "take", body).await
}

async fn state_of(d: &D, code: &str, key: &str) -> Res {
    let cookie = guest::guest_cookie(code);
    gsend(d, "GET", &format!("/api/pocket/{code}/state"), None, Some((&cookie, key)), None).await
}

async fn host_get(d: &D, code: &str, host: &str, query: &str) -> Value {
    let r = gsend(d, "GET", &format!("/api/pocket/{code}/host/state{query}"), None, None, Some(host)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

async fn host_act(d: &D, code: &str, host: &str, name: &str, body: Value) -> Res {
    gsend(d, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": name, "body": body })), None, Some(host)).await
}

async fn info(d: &D, code: &str) -> Value {
    let r = gsend(d, "GET", &format!("/api/pocket/{code}/info"), None, None, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

/// The kit state as stored.
async fn stored(d: &D, code: &str) -> String {
    d.env.state.db.pocket_get(code).await.unwrap().unwrap().kit_state
}

async fn player_ids(d: &D, code: &str) -> Vec<String> {
    d.env.state.db.pocket_players(code).await.unwrap().into_iter().map(|p| p.id).collect()
}

fn engine_turns() -> usize {
    invocations_with("").len()
}

#[tokio::test]
async fn a_guest_sees_only_their_own_ticket_and_screens_show_no_names() {
    let d = setup().await;
    let turns = engine_turns();
    let (code, host) = queue(&d, json!({ "ask_name": true })).await;
    let (a, ka) = joined(&d, &code).await;
    let (b, kb) = joined(&d, &code).await;
    let r = take(&d, &code, &ka, json!({ "name": "Mia" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["view"]["my_ticket"]["number"], 1, "{}", r.body);
    let r = take(&d, &code, &kb, json!({ "name": "Bostjan" })).await;
    assert_eq!(r.body["view"]["my_ticket"]["number"], 2);
    assert_eq!(r.body["view"]["my_ticket"]["place"], 2);
    assert_eq!(r.body["view"]["my_ticket"]["ahead"], 1);
    assert_eq!(r.body["view"]["waiting"], 2);

    // Guest A's page: their own name and ticket, nothing of B
    let sa = state_of(&d, &code, &ka).await.body.to_string();
    assert!(sa.contains("Mia") && !sa.contains("Bostjan") && !sa.contains(&b), "{sa}");
    let sb = state_of(&d, &code, &kb).await.body.to_string();
    assert!(sb.contains("Bostjan") && !sb.contains("Mia") && !sb.contains(&a), "{sb}");
    // Before taking a ticket, the big screen, and the lobby: numbers only
    let screen = host_get(&d, &code, &host, "?view=screen").await;
    let pub_ = info(&d, &code).await;
    for v in [&screen, &pub_] {
        let t = v.to_string();
        assert!(!t.contains("Mia") && !t.contains("Bostjan") && !t.contains(&a) && !t.contains(&b), "{t}");
    }
    assert_eq!(screen["view"]["next"], json!([1, 2]), "{screen}");
    assert_eq!(screen["view"]["waiting"], 2);
    assert_eq!(pub_["view"]["waiting"], 2, "{pub_}");
    assert!(pub_["view"]["my_ticket"].is_null() && pub_["view"].get("next").is_none(), "{pub_}");
    // The host sees first names, never guest keys or ids
    let hv = host_get(&d, &code, &host, "").await;
    let tickets = hv["view"]["tickets"].as_array().unwrap();
    assert_eq!(tickets.iter().map(|t| t["name"].as_str().unwrap()).collect::<Vec<_>>(), vec!["Mia", "Bostjan"], "{hv}");
    assert!(!hv["view"].to_string().contains(&a), "{hv}");
    // A name with an email shape, or with control characters, is refused
    let (_, kc) = joined(&d, &code).await;
    assert_eq!(take(&d, &code, &kc, json!({ "name": "mia@example.com" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(take(&d, &code, &kc, json!({ "name": "Mi\u{1b}a" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(take(&d, &code, &kc, json!({ "name": "x".repeat(25) })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(engine_turns(), turns, "a queue never calls a model");
}

#[tokio::test]
async fn numbers_count_from_one_call_next_serves_and_calls_and_reopen_starts_again() {
    let d = setup().await;
    let (code, host) = queue(&d, json!({})).await;
    let mut keys = Vec::new();
    for _ in 0..3 {
        let (_, k) = joined(&d, &code).await;
        assert_eq!(take(&d, &code, &k, json!({ "name": "Ignored" })).await.status, StatusCode::OK);
        keys.push(k);
    }
    assert!(!stored(&d, &code).await.contains("Ignored"), "ask_name is off: no name is kept");
    let r = host_act(&d, &code, &host, "call_next", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["view"]["now_serving"], 1, "{}", r.body);
    let mine = state_of(&d, &code, &keys[0]).await.body;
    assert_eq!(mine["view"]["my_ticket"]["state"], "called", "{mine}");
    assert!(mine["view"]["my_ticket"]["call_ends_at"].is_string(), "{mine}");
    // Served and the next called in one action
    let r = host_act(&d, &code, &host, "call_next", json!({ "served": 1 })).await;
    assert_eq!(r.body["view"]["now_serving"], 2, "{}", r.body);
    assert_eq!(r.body["view"]["summary"]["served_today"], 1);
    assert_eq!(state_of(&d, &code, &keys[0]).await.body["view"]["my_ticket"]["state"], "served");
    assert_eq!(state_of(&d, &code, &keys[2]).await.body["view"]["my_ticket"]["place"], 1);
    host_act(&d, &code, &host, "call_next", json!({ "served": 2 })).await;
    let r = host_act(&d, &code, &host, "call_next", json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("empty")), "{}", r.body);
    // Bad bodies and unknown tickets
    assert_eq!(host_act(&d, &code, &host, "serve", json!({ "ticket": "3" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(host_act(&d, &code, &host, "serve", json!({ "ticket": 99 })).await.status, StatusCode::NOT_FOUND);
    assert_eq!(host_act(&d, &code, &host, "pause", json!({ "text": "x".repeat(61) })).await.status, StatusCode::BAD_REQUEST);
    // Close, then open again: numbering restarts at 1
    let r = host_act(&d, &code, &host, "close", json!({})).await;
    assert_eq!(r.body["view"]["close_text"], CLOSE_TEXT, "{}", r.body);
    host_act(&d, &code, &host, "reopen", json!({})).await;
    let (_, k) = joined(&d, &code).await;
    let r = take(&d, &code, &k, json!({})).await;
    assert_eq!(r.body["view"]["my_ticket"]["number"], 1, "{}", r.body);
}

#[tokio::test]
async fn a_called_ticket_is_skipped_after_call_minutes_and_can_be_recalled() {
    let d = setup().await;
    let (code, host) = queue(&d, json!({ "call_minutes": 3 })).await;
    let (_, ka) = joined(&d, &code).await;
    take(&d, &code, &ka, json!({})).await;
    host_act(&d, &code, &host, "call_next", json!({})).await;
    pocket::sweep(&d.env.state, Utc::now() + Duration::minutes(2)).await;
    assert_eq!(state_of(&d, &code, &ka).await.body["view"]["my_ticket"]["state"], "called");
    pocket::sweep(&d.env.state, Utc::now() + Duration::minutes(4)).await;
    let v = state_of(&d, &code, &ka).await.body;
    assert_eq!(v["view"]["my_ticket"]["state"], "skipped", "{v}");
    let hv = host_get(&d, &code, &host, "").await;
    assert_eq!(hv["view"]["summary"]["skipped_today"], 1, "{hv}");
    assert!(hv["view"]["tickets"][0]["recall_until"].is_string(), "{hv}");
    // Inside 10 minutes: called again
    let r = host_act(&d, &code, &host, "recall", json!({ "ticket": 1 })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(state_of(&d, &code, &ka).await.body["view"]["my_ticket"]["state"], "called");
    assert_eq!(r.body["view"]["summary"]["skipped_today"], 0);
}

#[tokio::test]
async fn hours_until_closes_the_queue_with_the_default_text() {
    let d = setup().await;
    let until = (Local::now() + Duration::minutes(2)).format("%H:%M").to_string();
    let (code, host) = queue(&d, json!({ "hours": { "until": until } })).await;
    let (_, ka) = joined(&d, &code).await;
    take(&d, &code, &ka, json!({})).await;
    let hv = host_get(&d, &code, &host, "").await;
    assert_eq!(hv["view"]["hours_until"], until.as_str(), "{hv}");
    pocket::sweep(&d.env.state, Utc::now() + Duration::minutes(5)).await;
    let v = state_of(&d, &code, &ka).await.body;
    assert_eq!(v["view"]["state"], "closed", "{v}");
    assert_eq!(v["view"]["close_text"], CLOSE_TEXT);
    assert_eq!(v["view"]["my_ticket"]["closed"], true, "{v}");
    let (_, kb) = joined(&d, &code).await;
    let r = take(&d, &code, &kb, json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::GONE, Some("closed")), "{}", r.body);
    assert_eq!(r.body["close_text"], CLOSE_TEXT, "{}", r.body);
    assert_eq!(host_get(&d, &code, &host, "").await["view"]["auto_closed"], true);
}

#[tokio::test]
async fn refusals_has_ticket_paused_closed_and_full() {
    let d = setup().await;
    let (code, host) = queue(&d, json!({ "max_waiting": 10 })).await;
    let (_, ka) = joined(&d, &code).await;
    take(&d, &code, &ka, json!({})).await;
    let r = take(&d, &code, &ka, json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("has_ticket")), "{}", r.body);
    assert_eq!(r.body["ticket"]["number"], 1, "the same ticket comes back: {}", r.body);
    host_act(&d, &code, &host, "pause", json!({ "text": "Back in 10 minutes" })).await;
    let (_, kb) = joined(&d, &code).await;
    let r = take(&d, &code, &kb, json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("paused")), "{}", r.body);
    assert_eq!(r.body["pause_text"], "Back in 10 minutes");
    assert_eq!(host_act(&d, &code, &host, "call_next", json!({})).await.status, StatusCode::CONFLICT);
    host_act(&d, &code, &host, "resume", json!({})).await;
    for _ in 0..9 {
        let (_, k) = joined(&d, &code).await;
        assert_eq!(take(&d, &code, &k, json!({})).await.status, StatusCode::OK);
    }
    let r = take(&d, &code, &kb, json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("full")), "{}", r.body);
    assert_eq!(r.body["max_waiting"], 10);
    host_act(&d, &code, &host, "close", json!({ "text": "Sold out, sorry!" })).await;
    let r = take(&d, &code, &kb, json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::GONE, Some("closed")), "{}", r.body);
    assert_eq!(r.body["close_text"], "Sold out, sorry!");
}

#[tokio::test]
async fn the_fourth_take_in_an_hour_from_one_key_or_the_31st_from_one_address_is_slow_down() {
    let d = setup().await;
    let (code, _host) = queue(&d, json!({})).await;
    let (_, ka) = joined(&d, &code).await;
    for _ in 0..3 {
        assert_eq!(take(&d, &code, &ka, json!({})).await.status, StatusCode::OK);
        // While the ticket is live, another take is has_ticket, not counted
        assert_eq!(take(&d, &code, &ka, json!({})).await.status, StatusCode::CONFLICT);
        assert_eq!(act_from(&d, None, &code, &ka, "leave", json!({})).await.status, StatusCode::OK);
    }
    let r = take(&d, &code, &ka, json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("slow_down")), "{}", r.body);
    // One shop Wi-Fi: 30 customers fine, the 31st waits (joins come from other addresses so the
    // sign-in limit isn't what stops them)
    let shop = "203.0.113.9";
    for _ in 0..30 {
        let (_, k) = joined(&d, &code).await;
        let r = act_from(&d, Some(shop), &code, &k, "take", json!({})).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    }
    let (_, k) = joined(&d, &code).await;
    let r = act_from(&d, Some(shop), &code, &k, "take", json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::TOO_MANY_REQUESTS, Some("slow_down")), "{}", r.body);
    assert_eq!(act_from(&d, None, &code, &k, "take", json!({})).await.status, StatusCode::OK, "another address");
}

#[tokio::test]
async fn finished_tickets_lose_the_name_at_once_and_the_guest_at_the_sweep() {
    let d = setup().await;
    let (code, host) = queue(&d, json!({ "ask_name": true })).await;
    let names = ["Ana", "Ben", "Cvet", "Dora", "Eva"];
    let mut guests = Vec::new();
    for n in names {
        let (id, k) = joined(&d, &code).await;
        assert_eq!(take(&d, &code, &k, json!({ "name": n })).await.status, StatusCode::OK);
        guests.push((id, k));
    }
    let (idle, idle_key) = joined(&d, &code).await;
    // Ana served, Ben left, Cvet removed by the host, Dora skipped
    host_act(&d, &code, &host, "call_next", json!({})).await;
    host_act(&d, &code, &host, "serve", json!({ "ticket": 1 })).await;
    act_from(&d, None, &code, &guests[1].1, "leave", json!({})).await;
    host_act(&d, &code, &host, "remove", json!({ "ticket": 3 })).await;
    host_act(&d, &code, &host, "call_next", json!({})).await;
    host_act(&d, &code, &host, "skip", json!({ "ticket": 4 })).await;
    let st = stored(&d, &code).await;
    for n in ["Ana", "Ben", "Cvet"] {
        assert!(!st.contains(n), "{n} is gone at once: {st}");
    }
    assert!(st.contains("Dora") && st.contains("Eva"), "a skipped ticket keeps its name for the recall window: {st}");
    // The page still says what happened until the sweep
    assert_eq!(state_of(&d, &code, &guests[0].1).await.body["view"]["my_ticket"]["state"], "served");
    assert_eq!(state_of(&d, &code, &guests[2].1).await.body["view"]["my_ticket"]["state"], "left", "removed reads as left to the guest");
    pocket::sweep(&d.env.state, Utc::now() + Duration::minutes(3)).await;
    let ids = player_ids(&d, &code).await;
    for (i, (id, key)) in guests.iter().enumerate().take(3) {
        assert!(!ids.contains(id), "guest {i} is forgotten");
        assert_eq!(state_of(&d, &code, key).await.status, StatusCode::UNAUTHORIZED, "the key goes with them");
    }
    assert!(ids.contains(&guests[3].0) && ids.contains(&guests[4].0) && ids.contains(&idle));
    // 10 minutes after the skip (and after joining with no ticket), those go too
    pocket::sweep(&d.env.state, Utc::now() + Duration::minutes(11)).await;
    let ids = player_ids(&d, &code).await;
    assert!(!ids.contains(&guests[3].0) && !ids.contains(&idle), "{ids:?}");
    assert_eq!(state_of(&d, &code, &idle_key).await.status, StatusCode::UNAUTHORIZED);
    assert!(!stored(&d, &code).await.contains("Dora"));
    // Close: Eva's name goes at once, her ticket and key at the sweep
    host_act(&d, &code, &host, "close", json!({})).await;
    assert!(!stored(&d, &code).await.contains("Eva"));
    assert_eq!(state_of(&d, &code, &guests[4].1).await.body["view"]["my_ticket"]["closed"], true);
    pocket::sweep(&d.env.state, Utc::now() + Duration::minutes(3)).await;
    assert!(player_ids(&d, &code).await.is_empty());
    let st: Value = serde_json::from_str(&stored(&d, &code).await).unwrap();
    assert_eq!(st["tickets"], json!([]), "{st}");
    assert!(d.env.state.db.pocket_events(&code).await.unwrap().is_empty(), "a queue keeps no events");
    // Only counts stay for the Done row
    let (s, v) = call(&d.env.app, "GET", "/api/pockets", None).await;
    assert_eq!(s, StatusCode::OK);
    let t = v.to_string();
    for n in names {
        assert!(!t.contains(n), "{t}");
    }
    let hv = host_get(&d, &code, &host, "").await;
    assert_eq!(hv["view"]["summary"]["served_today"], 1, "{hv}");
    assert_eq!(hv["view"]["summary"]["left_today"], 2, "left and removed");
}

#[tokio::test]
async fn a_queue_works_with_every_bots_budget_used_up_and_never_needs_the_host() {
    let d = setup().await;
    let turns = engine_turns();
    let today = Local::now().format("%Y-%m-%d").to_string();
    for bot in [&d.env.a, &d.env.b] {
        d.env.state.db.set_bot_budget(bot, 1, 0.01).await.unwrap();
        d.env.state.db.add_bot_spend(&today, bot, 1000, 1.0, true).await.unwrap();
    }
    let (code, host) = queue(&d, json!({})).await;
    let (_, ka) = joined(&d, &code).await;
    assert_eq!(take(&d, &code, &ka, json!({})).await.status, StatusCode::OK);
    assert_eq!(host_act(&d, &code, &host, "call_next", json!({})).await.status, StatusCode::OK);
    assert_eq!(host_act(&d, &code, &host, "serve", json!({ "ticket": 1 })).await.status, StatusCode::OK);
    let (s, v) = call(&d.env.app, "GET", "/api/waiting", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!v.to_string().contains(&code), "nothing waits on the host: {v}");
    // Ending wipes every ticket
    let r = host_act(&d, &code, &host, "end", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(engine_turns(), turns, "no model turn");
}
