//! RIGEL QA (not for merge): desk-inbox-rows-server probes. Based on pocket_booking_test helpers.
//! pa13b The front desk kit's Booking Desk: the host lists slots, a guest holds one as
//! `requested`, and only the host's `confirm` makes it `confirmed`. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa13/pa13b-server-brief.md
//!
//! Every model call here goes to the fake claude (tests/common), never a real one.

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use base64::Engine;
use chrono::{Duration, Local, Utc};
use common::{call, setup_with, Env};
#[allow(unused_imports)]
use common::invocations_with;
use kwnt::pocket::{self, guest, listener};
use serde_json::{json, Value};
use std::time::Duration as StdDuration;
use tower::ServiceExt;

const GHOST: &str = "pocket.example.net";

struct D {
    env: Env,
    guest: Router,
}

async fn setup() -> D {
    let env = setup_with(|s| s.config.pocket_port = Some(0)).await;
    env.state
        .db
        .set_setting(pocket::SETTING, "on")
        .await
        .unwrap();
    listener::reconcile(&env.state).await;
    let guest = guest::router(env.state.clone());
    D { env, guest }
}

struct Res {
    status: StatusCode,
    #[allow(dead_code)]
    headers: HeaderMap,
    body: Value,
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("203.0.{}.{}", (n >> 8) & 255, n & 255)
}

async fn gsend(
    d: &D,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: Option<(&str, &str)>,
    bearer: Option<&str>,
) -> Res {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", GHOST)
        .header("content-type", "application/json")
        .header("x-forwarded-for", fresh_ip());
    if let Some((k, v)) = cookie {
        req = req.header("cookie", format!("{k}={v}"));
    }
    if let Some(b) = bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    let req = req
        .body(
            body.map(|b| Body::from(b.to_string()))
                .unwrap_or_else(Body::empty),
        )
        .unwrap();
    let resp = d.guest.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Res {
        status,
        headers,
        body: serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())),
    }
}

/// A slot `days` from now at `hour`:00, the box's local time, as the spec writes it.
fn at(days: i64, hour: u32) -> String {
    format!(
        "{}T{hour:02}:00",
        (Local::now() + Duration::days(days)).format("%Y-%m-%d")
    )
}

/// Two morning slots ten days out, one seat each: "Morning cut" and "Noon cut".
fn two_slots() -> Value {
    json!([
        { "id": "s1", "start": at(10, 9), "minutes": 60, "label": "Morning cut" },
        { "id": "s2", "start": at(10, 12), "minutes": 60, "label": "Noon cut" },
    ])
}

/// A booking desk made through the person's API, answered by bot A: (code, host key).
async fn booking_desk(d: &D, slots: Value) -> (String, String) {
    let spec = json!({ "title": "Mia's chair", "place": "Ljubljana", "host_name": "Mia", "mode": "booking", "slots": slots, "answer_bot": d.env.a });
    let (s, v) = call(
        &d.env.app,
        "POST",
        "/api/pockets",
        Some(json!({ "kit": "desk", "spec": spec })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (
        v["pocket"]["code"].as_str().unwrap().to_string(),
        v["host_key"].as_str().unwrap().to_string(),
    )
}

async fn joined(d: &D, code: &str, name: &str) -> (String, String) {
    let r = gsend(
        d,
        "POST",
        &format!("/api/pocket/{code}/join"),
        Some(json!({ "name": name, "pin": "4821" })),
        None,
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let cookie = guest::guest_cookie(code);
    let key = r
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| {
            c.split(';')
                .next()?
                .split_once('=')
                .filter(|(k, _)| *k == cookie)
                .map(|(_, v)| v.to_string())
        })
        .expect("the guest cookie");
    (r.body["me"]["id"].as_str().unwrap().to_string(), key)
}

async fn gact(d: &D, code: &str, key: &str, name: &str, body: Value) -> Res {
    let cookie = guest::guest_cookie(code);
    gsend(
        d,
        "POST",
        &format!("/api/pocket/{code}/action"),
        Some(json!({ "name": name, "body": body })),
        Some((&cookie, key)),
        None,
    )
    .await
}

async fn say(d: &D, code: &str, key: &str, text: &str) -> Res {
    gact(d, code, key, "say", json!({ "text": text })).await
}

async fn state_of(d: &D, code: &str, key: &str) -> Value {
    let cookie = guest::guest_cookie(code);
    let r = gsend(
        d,
        "GET",
        &format!("/api/pocket/{code}/state"),
        None,
        Some((&cookie, key)),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

async fn host_get(d: &D, code: &str, host: &str, query: &str) -> Res {
    gsend(
        d,
        "GET",
        &format!("/api/pocket/{code}/host/state{query}"),
        None,
        None,
        Some(host),
    )
    .await
}

async fn host_act(d: &D, code: &str, host: &str, name: &str, body: Value) -> Res {
    gsend(
        d,
        "POST",
        &format!("/api/pocket/{code}/host/action"),
        Some(json!({ "name": name, "body": body })),
        None,
        Some(host),
    )
    .await
}

fn msgs(state: &Value) -> Vec<Value> {
    state["view"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[allow(dead_code)]
/// The guest's slot `id` in their own view.
fn slot<'a>(state: &'a Value, id: &str) -> Option<&'a Value> {
    state["view"]["slots"]
        .as_array()?
        .iter()
        .find(|s| s["id"] == id)
}

#[allow(dead_code)]
fn open(state: &Value, id: &str) -> bool {
    slot(state, id).is_some_and(|s| s["open"] == true)
}

#[allow(dead_code)]
/// The host's slot `id`.
async fn host_slot(d: &D, code: &str, host: &str, id: &str) -> Value {
    let h = host_get(d, code, host, "").await;
    assert_eq!(h.status, StatusCode::OK, "{}", h.body);
    h.body["view"]["slots"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["id"] == id).cloned())
        .unwrap_or_else(|| panic!("no host slot {id}: {}", h.body))
}

async fn wait_msg(d: &D, code: &str, key: &str, from: &str, needle: &str) -> Value {
    for _ in 0..600 {
        let s = state_of(d, code, key).await;
        if let Some(m) = msgs(&s)
            .into_iter()
            .find(|m| m["from"] == from && m["text"].as_str().unwrap_or("").contains(needle))
        {
            return m;
        }
        tokio::time::sleep(StdDuration::from_millis(100)).await;
    }
    panic!("timed out waiting for a {from} message with {needle}");
}

async fn desk_items(d: &D) -> Vec<Value> {
    let (s, v) = call(&d.env.app, "GET", "/api/waiting", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "pocket_desk")
        .cloned()
        .collect()
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}


const GUIDE: &str = "Wi-Fi: sea-view-77\nCheck-out: 10 AM";

async fn guide_desk(d: &D, extra: Value) -> (String, String) {
    let mut spec = json!({ "title": "Sea-view flat", "place": "Piran", "host_name": "Emma", "guide": GUIDE, "answer_bot": d.env.a });
    for (k, v) in extra.as_object().unwrap() {
        spec[k] = v.clone();
    }
    let (s, v) = call(&d.env.app, "POST", "/api/pockets", Some(json!({ "kit": "desk", "spec": spec }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (v["pocket"]["code"].as_str().unwrap().to_string(), v["host_key"].as_str().unwrap().to_string())
}

async fn closing_desk(d: &D, booking: bool, closes: chrono::DateTime<Utc>) -> (String, String) {
    let mut spec = json!({ "title": "Closing desk", "place": "Piran", "host_name": "Emma", "guide": GUIDE, "answer_bot": d.env.a });
    if booking {
        spec = json!({ "title": "Closing desk", "place": "Piran", "host_name": "Emma", "mode": "booking", "slots": two_slots(), "answer_bot": d.env.a });
    }
    let (s, v) = call(&d.env.app, "POST", "/api/pockets", Some(json!({ "kit": "desk", "spec": spec, "closes_at": closes.to_rfc3339() }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (v["pocket"]["code"].as_str().unwrap().to_string(), v["host_key"].as_str().unwrap().to_string())
}

async fn inbox_items(d: &D) -> Vec<Value> {
    let (s, v) = call(&d.env.app, "GET", "/api/inbox", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().clone()
}

async fn inbox_desk(d: &D) -> Vec<Value> {
    inbox_items(d).await.into_iter().filter(|i| i["kind"] == "pocket_desk").collect()
}

async fn done_rows(d: &D, code: &str) -> Vec<Value> {
    inbox_items(d).await.into_iter().filter(|i| i["id"] == json!(format!("pocket:{code}"))).collect()
}

fn mine<'a>(items: &'a [Value], guest: &str) -> Vec<&'a Value> {
    items.iter().filter(|i| i["guest_id"] == guest).collect()
}

async fn handoff_by(d: &D, code: &str, key: &str, text: &str, label: &str) {
    let r = say(d, code, key, &format!("{text} REPLYTEXT:{}", b64(label))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
}

async fn wait_rows(d: &D, n: usize) {
    common::wait_for("rows", || async { desk_items(d).await.len() == n }).await;
}

// 1. A capped guest: ask + waiting on /api/waiting and /api/inbox
#[tokio::test]
async fn rq1_capped_guest_row() {
    let d = setup().await;
    let (code, host) = guide_desk(&d, json!({ "daily_cap": 10 })).await;
    let (ben, key) = joined(&d, &code, "Ben").await;
    for i in 0..10 {
        say(&d, &code, &key, &format!("Q{i} REPLYTAG:rq1n{i}")).await;
        wait_msg(&d, &code, &key, "bot", &format!("answer rq1n{i}")).await;
        // wait idle
        for _ in 0..300 { let s = state_of(&d, &code, &key).await; if s["view"]["busy"] != json!(true) { break; } tokio::time::sleep(StdDuration::from_millis(50)).await; }
    }
    assert!(desk_items(&d).await.is_empty());
    for t in ["over 1", "over 2", "over 3"] {
        assert_eq!(say(&d, &code, &key, t).await.status, StatusCode::OK);
        tokio::time::sleep(StdDuration::from_millis(20)).await;
        if t == "over 1" { wait_rows(&d, 1).await; }
    }
    let w = desk_items(&d).await;
    assert_eq!(w.len(), 1, "{w:?}");
    println!("RQ1 /api/waiting row: {}", w[0]);
    assert_eq!((w[0]["ask"].as_str(), w[0]["text"].as_str()), (Some("daily_cap"), Some("Daily limit reached")));
    assert_eq!(w[0]["waiting"].as_u64(), Some(3), "{}", w[0]);
    assert_eq!(w[0]["id"], format!("desk:{code}:{ben}").as_str());
    let i = inbox_desk(&d).await;
    assert_eq!(i.len(), 1);
    for (k, v) in w[0].as_object().unwrap() { assert_eq!(&i[0][k], v, "{k}"); }
    // a 4th message while capped raises the count by one
    say(&d, &code, &key, "over 4").await;
    common::wait_for("count 4", || async { desk_items(&d).await[0]["waiting"].as_u64() == Some(4) }).await;
    // host reply clears
    let r = host_act(&d, &code, &host, "reply", json!({ "guest": ben, "text": "Sorry!" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert!(desk_items(&d).await.is_empty());
    assert!(inbox_desk(&d).await.is_empty());
}

// 2. booking request + hand-off on a booking desk
#[tokio::test]
async fn rq2_booking_and_handoff_two_rows() {
    let d = setup().await;
    let (code, host) = booking_desk(&d, two_slots()).await;
    let start_s1 = at(10, 9);
    let start_s2 = at(10, 12);
    // Ann: hand-off first, then a booking request
    let (ann, ann_key) = joined(&d, &code, "Ann").await;
    handoff_by(&d, &code, &ann_key, "How much is a men's cut?", "[[person]] Price question").await;
    wait_rows(&d, 1).await;
    assert_eq!(gact(&d, &code, &ann_key, "request", json!({ "slot": "s1" })).await.status, StatusCode::OK);
    wait_rows(&d, 2).await;
    let w = desk_items(&d).await;
    let r = mine(&w, &ann);
    assert_eq!(r.len(), 2, "{w:?}");
    println!("RQ2 rows: {}", serde_json::to_string(&r).unwrap());
    let b = r.iter().find(|i| i["ask"] == "booking").expect("booking row");
    let h = r.iter().find(|i| i["ask"] == "handoff").expect("handoff row");
    assert_eq!(b["id"], format!("desk:{code}:{ann}@{start_s1}").as_str());
    assert_eq!(h["id"], format!("desk:{code}:{ann}").as_str());
    assert_eq!(h["text"], "Price question");
    assert_eq!(b["waiting"], 0); assert_eq!(h["waiting"], 0);
    assert_eq!(b["slot"], "s1");
    let ii = inbox_desk(&d).await;
    assert_eq!(ii.len(), 2);
    let mut ids: Vec<&str> = ii.iter().map(|i| i["id"].as_str().unwrap()).collect(); ids.sort(); ids.dedup();
    assert_eq!(ids.len(), 2, "distinct inbox ids");
    // confirm leaves the hand-off
    assert_eq!(host_act(&d, &code, &host, "confirm", json!({ "guest": ann, "slot": "s1" })).await.status, StatusCode::OK);
    let w = desk_items(&d).await;
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!((w[0]["ask"].as_str(), w[0]["id"].as_str()), (Some("handoff"), Some(format!("desk:{code}:{ann}").as_str())));

    // Ben: booking first, then hand-off; decline leaves the hand-off
    let (ben, ben_key) = joined(&d, &code, "Ben").await;
    assert_eq!(gact(&d, &code, &ben_key, "request", json!({ "slot": "s2" })).await.status, StatusCode::OK);
    wait_rows(&d, 2).await;
    handoff_by(&d, &code, &ben_key, "Do you do beards?", "[[person]] Beard question").await;
    wait_rows(&d, 3).await;
    let w = desk_items(&d).await;
    let r = mine(&w, &ben);
    assert_eq!(r.len(), 2, "{w:?}");
    assert!(r.iter().any(|i| i["ask"] == "booking" && i["id"] == format!("desk:{code}:{ben}@{start_s2}").as_str()));
    assert_eq!(host_act(&d, &code, &host, "decline", json!({ "guest": ben, "slot": "s2" })).await.status, StatusCode::OK);
    let w = desk_items(&d).await;
    let r = mine(&w, &ben);
    assert_eq!((r.len(), r[0]["ask"].as_str()), (1, Some("handoff")), "{w:?}");

    // Cy: request + hand-off; reply leaves the request; then Di: resolve leaves the request
    let (_cy, _) = (0, 0);
    // free slot s1 again? s1 is confirmed to Ann; s2 free after decline
    let (cy, cy_key) = joined(&d, &code, "Cy").await;
    handoff_by(&d, &code, &cy_key, "Can I bring my son?", "[[person]] Bring a child").await;
    assert_eq!(gact(&d, &code, &cy_key, "request", json!({ "slot": "s2" })).await.status, StatusCode::OK);
    common::wait_for("Cy two rows", || async { mine(&desk_items(&d).await, &cy).len() == 2 }).await;
    let r = host_act(&d, &code, &host, "reply", json!({ "guest": cy, "text": "Sure, bring him." })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let w = desk_items(&d).await;
    let r = mine(&w, &cy);
    assert_eq!((r.len(), r[0]["ask"].as_str()), (1, Some("booking")), "reply must leave the request: {w:?}");
    // resolve on a guest with the same shape: re-make hand-off for Cy then resolve
    handoff_by(&d, &code, &cy_key, "And parking?", "[[person]] Parking").await;
    common::wait_for("Cy two rows again", || async { mine(&desk_items(&d).await, &cy).len() == 2 }).await;
    let r = host_act(&d, &code, &host, "resolve", json!({ "guest": cy })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let w = desk_items(&d).await;
    let r = mine(&w, &cy);
    assert_eq!((r.len(), r[0]["ask"].as_str()), (1, Some("booking")), "resolve must leave the request: {w:?}");
    // the host page's guest row still says waiting while the request is open
    let h = host_get(&d, &code, &host, "").await;
    assert_eq!(h.status, StatusCode::OK);
}

// 3. Request label variants; the guest never sees the word
#[tokio::test]
async fn rq3_request_label_variants() {
    let d = setup().await;
    let (code, host) = guide_desk(&d, json!({})).await;
    let cases: [(&str, &str, &str); 8] = [
        ("[[person]] Request: Late check-out: 2 PM", "request", "Late check-out: 2 PM"),
        ("[[person]] request:Early check-in", "request", "Early check-in"),
        ("[[person]] REQUEST:   Extra guest", "request", "Extra guest"),
        ("[[person]] Request:", "request", "Bell asked you to reply"),
        ("[[person]] Requested late check-out", "handoff", "Requested late check-out"),
        ("[[person]] Late check-out: Request: 2 PM", "handoff", "Late check-out: Request: 2 PM"),
        ("[[person]] Request - late check-out", "handoff", "Request - late check-out"),
        ("[[person]] Price of breakfast", "handoff", "Price of breakfast"),
    ];
    let (_, rd) = call(&d.env.app, "GET", &format!("/api/pockets/{code}"), None).await;
    let bot = rd["summary"]["answer_bot"]["name"].as_str().unwrap().to_string();
    for (n, (label, ask, text)) in cases.iter().enumerate() {
        let (g, key) = joined(&d, &code, &format!("G{n}")).await;
        handoff_by(&d, &code, &key, "hello there", label).await;
        common::wait_for("row", || async { !mine(&desk_items(&d).await, &g).is_empty() }).await;
        let w = desk_items(&d).await;
        let r = mine(&w, &g)[0].clone();
        let expect_text = text.replace("Bell", &bot);
        println!("RQ3 {label:?} -> ask={} text={}", r["ask"], r["text"]);
        assert_eq!((r["ask"].as_str().unwrap(), r["text"].as_str().unwrap()), (*ask, expect_text.as_str()), "{label}");
        let s = state_of(&d, &code, &key).await.to_string();
        assert!(!s.contains("Request:") && !s.to_lowercase().contains("request:"), "guest sees the label: {s}");
        // host page too
        let h = host_get(&d, &code, &host, "").await.body;
        let g_row = h["view"]["guests"].as_array().unwrap().iter().find(|x| x["id"] == g.as_str()).cloned().unwrap();
        println!("   host page handoff_reason={}", g_row["handoff_reason"]);
        let _ = host_act(&d, &code, &host, "resolve", json!({ "guest": g })).await;
    }
    // quoting: the row quote is the guest's own words
    // host-away: a deleted bot is never a request
    let (g, key) = joined(&d, &code, "Away").await;
    let (s, v) = call(&d.env.app, "DELETE", &format!("/api/kwants/{}", d.env.a), None).await;
    assert!(s.is_success(), "{s} {v}");
    say(&d, &code, &key, "Request: please let me stay late").await;
    common::wait_for("away row", || async { !mine(&desk_items(&d).await, &g).is_empty() }).await;
    let w = desk_items(&d).await;
    let r = mine(&w, &g)[0].clone();
    println!("RQ3 away row: ask={} text={}", r["ask"], r["text"]);
    assert_eq!(r["ask"], "handoff");
}

// 3b. a booking desk's Request:
#[tokio::test]
async fn rq3b_booking_desk_request() {
    let d = setup().await;
    let (code, _host) = booking_desk(&d, two_slots()).await;
    let (g, key) = joined(&d, &code, "Dog").await;
    handoff_by(&d, &code, &key, "Can I bring my dog?", "[[person]] Request: Bring my dog").await;
    wait_rows(&d, 1).await;
    let w = desk_items(&d).await;
    assert_eq!((w[0]["ask"].as_str(), w[0]["text"].as_str(), w[0]["mode"].as_str(), w[0]["guest_id"].as_str()), (Some("request"), Some("Bring my dog"), Some("booking"), Some(g.as_str())));
    assert!(!state_of(&d, &code, &key).await.to_string().contains("Request:"));
}

// 4. End via card / host page / closing time
async fn two_waiting_one_answered(d: &D, code: &str, host: &str, booking: bool) -> usize {
    // Ann hand-off, Bea request label, Cy answered, Dan idle, (booking) Eve booking request
    let (_ann, ak) = joined(d, code, "Ann").await;
    handoff_by(d, code, &ak, "late?", "[[person]] Late check-out").await;
    let (_bea, bk) = joined(d, code, "Bea").await;
    handoff_by(d, code, &bk, "early?", "[[person]] Request: Early check-in").await;
    let (cy, ck) = joined(d, code, "Cy").await;
    handoff_by(d, code, &ck, "dog?", "[[person]] Dog").await;
    let (_dan, _) = joined(d, code, "Dan").await;
    let mut n = 2;
    common::wait_for("3 rows", || async { desk_items(d).await.len() == 3 }).await;
    assert_eq!(host_act(d, code, host, "reply", json!({ "guest": cy, "text": "ok" })).await.status, StatusCode::OK);
    if booking {
        let (_e, ek) = joined(d, code, "Eve").await;
        assert_eq!(gact(d, code, &ek, "request", json!({ "slot": "s1" })).await.status, StatusCode::OK);
        n += 1;
    }
    common::wait_for("rows", || async { desk_items(d).await.len() == n }).await;
    n
}

#[tokio::test]
async fn rq4a_end_via_card_and_via_host_page_and_second_end() {
    for via in ["card", "host"] {
        let d = setup().await;
        let (code, host) = booking_desk(&d, two_slots()).await;
        let n = two_waiting_one_answered(&d, &code, &host, true).await;
        assert_eq!(n, 3);
        let before = done_rows(&d, &code).await;
        assert!(before.is_empty(), "no done row while open: {before:?}");
        if via == "card" {
            let (s, v) = call(&d.env.app, "POST", &format!("/api/pockets/{code}/end"), None).await;
            assert_eq!(s, StatusCode::OK, "{v}");
        } else {
            let r = gsend(&d, "POST", &format!("/api/pocket/{code}/host/end"), None, None, Some(&host)).await;
            assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        }
        pocket::schedule::run_once(&d.env.state, Utc::now()).await;
        assert!(desk_items(&d).await.is_empty());
        let rows = done_rows(&d, &code).await;
        println!("RQ4a via {via}: {}", serde_json::to_string(&rows).unwrap());
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["title"], "Closing desk: Ended with 3 unanswered".replace("Closing desk", "Mia's chair").as_str());
        assert_eq!(rows[0]["ended_unanswered"], 3);
        // second End (both routes), a run_once, a sweep at +1 day
        call(&d.env.app, "POST", &format!("/api/pockets/{code}/end"), None).await;
        let _ = gsend(&d, "POST", &format!("/api/pocket/{code}/host/end"), None, None, Some(&host)).await;
        pocket::schedule::run_once(&d.env.state, Utc::now() + Duration::days(1)).await;
        let sw = pocket::sweep(&d.env.state, Utc::now() + Duration::days(1)).await;
        println!("   sweep +1d: {sw:?}");
        let rows = done_rows(&d, &code).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["ended_unanswered"], 3, "{rows:?}");
        // sweep at +8 days: guest data forgotten (7 days idle); N must not change while the row is listed
        let sw = pocket::sweep(&d.env.state, Utc::now() + Duration::days(8)).await;
        let rows = done_rows(&d, &code).await;
        println!("   sweep +8d: {sw:?} rows: {}", serde_json::to_string(&rows).unwrap());
        if let Some(r) = rows.first() { assert_eq!(r["ended_unanswered"], 3, "{rows:?}"); }
    }
}

#[tokio::test]
async fn rq4b_end_at_closing_time_exactly_one_row() {
    let d = setup().await;
    let closes = Utc::now() + Duration::hours(1);
    let (code, host) = closing_desk(&d, true, closes).await;
    let n = two_waiting_one_answered(&d, &code, &host, true).await;
    assert_eq!(n, 3);
    // before closing: nothing
    pocket::schedule::run_once(&d.env.state, Utc::now() + Duration::minutes(10)).await;
    assert!(done_rows(&d, &code).await.is_empty());
    let ran = pocket::schedule::run_once(&d.env.state, Utc::now() + Duration::hours(2)).await;
    println!("RQ4b ran ended={}", ran.ended);
    let rows = done_rows(&d, &code).await;
    println!("RQ4b {}", serde_json::to_string(&rows).unwrap());
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["ended_unanswered"], 3);
    assert_eq!(rows[0]["title"], "Closing desk: Ended with 3 unanswered");
    // sweep path at a later time does not change N
    pocket::sweep(&d.env.state, Utc::now() + Duration::hours(3)).await;
    pocket::schedule::run_once(&d.env.state, Utc::now() + Duration::hours(3)).await;
    let rows = done_rows(&d, &code).await;
    assert_eq!((rows.len(), rows[0]["ended_unanswered"].clone()), (1, json!(3)));
    // and the sweep (not run_once) as the closer
    let d2 = setup().await;
    let (code2, host2) = closing_desk(&d2, false, Utc::now() + Duration::hours(1)).await;
    let _ = two_waiting_one_answered(&d2, &code2, &host2, false).await;
    let sw = pocket::sweep(&d2.env.state, Utc::now() + Duration::hours(2)).await;
    println!("RQ4b sweep closer: {sw:?}");
    pocket::schedule::run_once(&d2.env.state, Utc::now() + Duration::hours(2)).await;
    let rows = done_rows(&d2, &code2).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["ended_unanswered"], 2, "{rows:?}");
}

// 5. nobody waiting: plain row; also: all answered before End
#[tokio::test]
async fn rq5_nobody_waiting_keeps_old_row() {
    let d = setup().await;
    let (code, host) = guide_desk(&d, json!({})).await;
    let (g, key) = joined(&d, &code, "Ann").await;
    handoff_by(&d, &code, &key, "late?", "[[person]] Late").await;
    wait_rows(&d, 1).await;
    assert_eq!(host_act(&d, &code, &host, "resolve", json!({ "guest": g })).await.status, StatusCode::OK);
    let r = gsend(&d, "POST", &format!("/api/pocket/{code}/host/end"), None, None, Some(&host)).await;
    assert_eq!(r.status, StatusCode::OK);
    pocket::schedule::run_once(&d.env.state, Utc::now()).await;
    let rows = done_rows(&d, &code).await;
    println!("RQ5 {}", serde_json::to_string(&rows).unwrap());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["title"], "Sea-view flat has ended");
    assert!(rows[0].get("ended_unanswered").is_none());
    // an empty desk too
    let (code, _h) = guide_desk(&d, json!({})).await;
    call(&d.env.app, "POST", &format!("/api/pockets/{code}/end"), None).await;
    pocket::schedule::run_once(&d.env.state, Utc::now()).await;
    let rows = done_rows(&d, &code).await;
    println!("RQ5 empty desk rows: {}", serde_json::to_string(&rows).unwrap());
}

// 6. every desk row has `waiting`
#[tokio::test]
async fn rq6_every_row_has_waiting_field() {
    let d = setup().await;
    let (code, _h) = booking_desk(&d, two_slots()).await;
    let (_g, key) = joined(&d, &code, "Ann").await;
    handoff_by(&d, &code, &key, "q", "[[person]] Q").await;
    assert_eq!(gact(&d, &code, &key, "request", json!({ "slot": "s1" })).await.status, StatusCode::OK);
    common::wait_for("2", || async { desk_items(&d).await.len() == 2 }).await;
    for i in desk_items(&d).await { assert!(i.get("waiting").is_some_and(|w| w.is_u64()), "{i}"); }
}
