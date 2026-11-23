//! pa10a Secret Santa server: privacy on every route, Santa questions, the late joiner, the
//! timers (draw, reveal) and retention on a fake clock, and PIN guessing on a Santa pocket.
//! Brief: /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa10a/server-brief.md.
//! The draw's own cases (500 seeded draws per size, refusals) are unit tests in
//! `src/pocket/santa.rs`.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    pocket::{self, guest, listener},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const LOCAL: &str = "127.0.0.1:50000";
const REMOTE: &str = "203.0.113.9:50000";
const GHOST: &str = "pocket.example.net";
const PIN: &str = "4821";

struct T {
    main: Router,
    guest: Router,
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
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), guest: guest::router(state.clone()), state, _tmp: tmp }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Res {
    fn cookie(&self, name: &str) -> Option<String> {
        self.headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find_map(|c| {
            let first = c.split(';').next()?;
            let (k, v) = first.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    }
}

#[derive(Default, Clone)]
struct Req<'a> {
    cookies: Vec<(String, String)>,
    bearer: Option<&'a str>,
    ip: Option<String>,
    peer: Option<&'a str>,
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>, r: &Req<'_>) -> Res {
    let host = match r.peer {
        Some(LOCAL) => "127.0.0.1:8799",
        Some(_) => "kwnt.example.com",
        None => GHOST,
    };
    let mut req = Request::builder().method(method).uri(uri).header("host", host).header("content-type", "application/json");
    if !r.cookies.is_empty() {
        req = req.header("cookie", r.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "));
    }
    if let Some(b) = r.bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    if let Some(ip) = &r.ip {
        req = req.header("x-forwarded-for", ip.as_str());
    }
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    if let Some(p) = r.peer {
        req.extensions_mut().insert(ConnectInfo(p.parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Res { status, headers, body: serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())) }
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

fn person() -> Req<'static> {
    Req { peer: Some(LOCAL), ..Default::default() }
}

fn guest_req(code: &str, key: &str) -> Req<'static> {
    Req { cookies: vec![(guest::guest_cookie(code), key.to_string())], ..Default::default() }
}

fn host_req(key: &str) -> Req<'_> {
    Req { bearer: Some(key), ..Default::default() }
}

/// A Santa spec 10 days out: gifts on that day, the reveal at 18:00 UTC on it.
fn spec(now: DateTime<Utc>, extra: Value) -> Value {
    let day = (now + Duration::days(10)).date_naive();
    let mut s = json!({
        "title": "Family Secret Santa",
        "gift_on": day.format("%Y-%m-%d").to_string(),
        "reveal_at": format!("{}T18:00:00Z", day.format("%Y-%m-%d")),
        "budget": "€25",
        "host_name": "Oleks",
        "keep_apart": [["Mia", "Sam"]],
    });
    for (k, v) in extra.as_object().unwrap() {
        s[k] = v.clone();
    }
    s
}

/// Makes a Santa pocket through the person's API: (code, host key).
async fn make(t: &T, spec: Value) -> (String, String) {
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "santa", "spec": spec })), &person()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (r.body["pocket"]["code"].as_str().unwrap().to_string(), r.body["host_key"].as_str().unwrap().to_string())
}

async fn join(t: &T, code: &str, name: &str) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": name, "pin": PIN })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await
}

/// Joins and returns (player id, guest key).
async fn joined(t: &T, code: &str, name: &str) -> (String, String) {
    let r = join(t, code, name).await;
    assert_eq!(r.status, StatusCode::CREATED, "{name}: {}", r.body);
    (r.body["me"]["id"].as_str().unwrap().to_string(), r.cookie(&guest::guest_cookie(code)).unwrap())
}

async fn act(t: &T, code: &str, key: &str, name: &str, body: Value) -> Res {
    // Under the 10-a-second action limit, so that limit isn't what's tested
    tokio::time::sleep(std::time::Duration::from_millis(110)).await;
    send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": name, "body": body })), &guest_req(code, key)).await
}

/// A host action from the person's card on the main port.
async fn host_act(t: &T, code: &str, name: &str, body: Value) -> Res {
    send(&t.main, "POST", &format!("/api/pockets/{code}/action"), Some(json!({ "name": name, "body": body })), &person()).await
}

async fn state_of(t: &T, code: &str, key: &str) -> Value {
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(code, key)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body
}

async fn kit_state(t: &T, code: &str) -> Value {
    serde_json::from_str(&t.state.db.pocket_get(code).await.unwrap().unwrap().kit_state).unwrap()
}

/// The loop as stored: player ids, each gives to the next.
async fn ring(t: &T, code: &str) -> Vec<String> {
    kit_state(t, code).await["ring"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn receiver(ring: &[String], giver: &str) -> String {
    let k = ring.iter().position(|p| p == giver).unwrap();
    ring[(k + 1) % ring.len()].clone()
}

fn santa_of(ring: &[String], me: &str) -> String {
    let k = ring.iter().position(|p| p == me).unwrap();
    ring[(k + ring.len() - 1) % ring.len()].clone()
}

/// Replaces the stored loop (everything else kept) and reloads the pocket.
async fn set_ring(t: &T, code: &str, ring: &[String], any_player: &str) {
    let mut st = kit_state(t, code).await;
    st["ring"] = json!(ring);
    t.state.db.pocket_save_state(code, &st.to_string(), &[], Utc::now()).await.unwrap();
    pocket::notices_seen(&t.state, code, any_player).await.unwrap();
}

/// A response with the parts that change on every save (the version) taken out, also from a
/// page's boot JSON.
fn steady(mut v: Value) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("version");
                m.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            Value::String(s) if s.contains("\"version\":") => {
                let mut out = String::new();
                let mut rest = s.as_str();
                while let Some(i) = rest.find("\"version\":") {
                    let (head, tail) = rest.split_at(i + 10);
                    out.push_str(head);
                    rest = tail.trim_start_matches(|c: char| c.is_ascii_digit());
                }
                out.push_str(rest);
                *s = out;
            }
            _ => {}
        }
    }
    strip(&mut v);
    v
}

/// One call in `every_route`: who, which port, method, path, body, request.
type Call<'a> = (&'a str, &'a Router, &'a str, String, Option<Value>, Req<'a>);

/// Every route a host, guest A or a stranger can call on a drawn Santa pocket, with what each
/// answered (status and steady body). `a` is A's guest key.
async fn every_route(t: &T, code: &str, host_key: &str, a_key: &str) -> Vec<(String, StatusCode, Value)> {
    let mut out = Vec::new();
    let g = |p: &str| format!("/api/pocket/{code}{p}");
    let m = |p: &str| format!("/api/pockets/{code}{p}");
    let stranger = || Req { ip: Some(fresh_ip()), ..Default::default() };
    let calls: Vec<Call> = vec![
        // The person, on the main port
        ("person", &t.main, "GET", "/api/pockets".into(), None, person()),
        ("person", &t.main, "GET", m(""), None, person()),
        ("person", &t.main, "POST", m("/action"), Some(json!({ "name": "remove_late", "body": { "name": "Nobody" } })), person()),
        ("person", &t.main, "POST", m("/action"), Some(json!({ "name": "draw_now" })), person()),
        ("person", &t.main, "POST", m("/remove"), Some(json!({ "player_id": "pl_none" })), person()),
        // The host remote, on the guest port
        ("host", &t.guest, "POST", g("/host/signin"), Some(json!({ "key": host_key })), Req { ip: Some(fresh_ip()), ..Default::default() }),
        ("host", &t.guest, "GET", g("/host/state"), None, host_req(host_key)),
        ("host", &t.guest, "GET", g("/host/state?view=screen"), None, host_req(host_key)),
        ("host", &t.guest, "POST", g("/host/action"), Some(json!({ "name": "remove_late", "body": { "name": "Nobody" } })), host_req(host_key)),
        ("host", &t.guest, "POST", g("/host/action"), Some(json!({ "name": "add_pair", "body": { "a": "Mia", "b": "Ana" } })), host_req(host_key)),
        // Guest A
        ("guest A", &t.guest, "GET", g("/state"), None, guest_req(code, a_key)),
        ("guest A", &t.guest, "GET", g("/info"), None, guest_req(code, a_key)),
        ("guest A", &t.guest, "GET", format!("/p/{code}"), None, guest_req(code, a_key)),
        ("guest A", &t.guest, "POST", g("/join"), Some(json!({ "name": "Ana", "pin": PIN })), guest_req(code, a_key)),
        ("guest A", &t.guest, "POST", g("/action"), Some(json!({ "name": "seen_asks" })), guest_req(code, a_key)),
        ("guest A", &t.guest, "POST", g("/action"), Some(json!({ "name": "draw_now" })), guest_req(code, a_key)),
        ("guest A", &t.guest, "GET", g("/host/state"), None, guest_req(code, a_key)),
        ("guest A as host", &t.guest, "GET", g("/host/state"), None, host_req(a_key)),
        // A stranger with the link
        ("stranger", &t.guest, "GET", g("/info"), None, stranger()),
        ("stranger", &t.guest, "GET", format!("/p/{code}"), None, stranger()),
        ("stranger", &t.guest, "GET", g("/state"), None, stranger()),
        ("stranger", &t.guest, "POST", g("/action"), Some(json!({ "name": "open_match" })), stranger()),
        ("stranger", &t.guest, "POST", g("/join"), Some(json!({ "name": "Eve", "pin": PIN })), stranger()),
        ("stranger", &t.guest, "POST", g("/rejoin"), Some(json!({ "name": "Nobody", "pin": "0000" })), stranger()),
        ("stranger", &t.guest, "GET", g("/host/state"), None, stranger()),
        ("stranger", &t.main, "GET", m(""), None, Req { peer: Some(REMOTE), ..Default::default() }),
        ("stranger", &t.main, "GET", "/api/pockets".into(), None, Req { peer: Some(REMOTE), ..Default::default() }),
    ];
    for (who, app, method, path, body, req) in calls {
        let r = send(app, method, &path, body, &req).await;
        out.push((format!("{who} {method} {path}"), r.status, steady(r.body)));
    }
    out
}

// ---- Privacy ----

/// Nobody but a giver learns their receiver, and the host learns no match and no question's
/// words, on any route. Shown two ways: (1) canary questions and answers appear only to the two
/// people of their pair; (2) the loop is rewritten among the people other than A, A's receiver
/// and A's Santa, and every answer to the host, to A and to a stranger stays exactly the same,
/// so none of them carries anything about the others' matches. The event log names nobody.
#[tokio::test]
async fn no_route_shows_another_persons_match_or_a_question() {
    let t = setup().await;
    let now = Utc::now();
    let (code, host_key) = make(&t, spec(now, json!({}))).await;
    let names = ["Ana", "Ben", "Cleo", "Dev", "Eli", "Fay"];
    let mut keys = Vec::new();
    for n in names {
        keys.push(joined(&t, &code, n).await);
    }
    let r = host_act(&t, &code, "draw_now", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let loop_ = ring(&t, &code).await;
    assert_eq!(loop_.len(), 6);
    let id_of = |i: usize| keys[i].0.clone();
    let key_of = |id: &str| keys.iter().find(|(p, _)| p == id).unwrap().1.clone();
    let name_of = |id: &str| names[keys.iter().position(|(p, _)| p == id).unwrap()].to_string();
    for (_, k) in &keys {
        assert_eq!(act(&t, &code, k, "open_match", json!({})).await.status, StatusCode::OK);
    }
    let a = id_of(0);
    let a_key = keys[0].1.clone();
    let (a_receiver, a_santa) = (receiver(&loop_, &a), santa_of(&loop_, &a));
    // A asks their receiver; A's Santa asks A, and A answers; two others ask each other
    assert_eq!(act(&t, &code, &a_key, "ask", json!({ "text": "CANARYQA shoe size?" })).await.status, StatusCode::OK);
    assert_eq!(act(&t, &code, &key_of(&a_santa), "ask", json!({ "text": "CANARYQS favourite colour?" })).await.status, StatusCode::OK);
    let received = state_of(&t, &code, &a_key).await["view"]["asks"]["received"][0].clone();
    assert_eq!(received["q"], "CANARYQS favourite colour?");
    assert_eq!(act(&t, &code, &a_key, "answer", json!({ "id": received["id"], "text": "CANARYANS green" })).await.status, StatusCode::OK);
    let others: Vec<String> = loop_.iter().filter(|p| **p != a && **p != a_receiver && **p != a_santa).cloned().collect();
    assert_eq!(others.len(), 3);
    assert_eq!(act(&t, &code, &key_of(&others[0]), "ask", json!({ "text": "CANARYQX secret?" })).await.status, StatusCode::OK);
    // The asker's own name never reaches the receiver
    let a_recv_view = state_of(&t, &code, &key_of(&a_receiver)).await;
    let their_asks = a_recv_view["view"]["asks"]["received"].to_string();
    assert!(their_asks.contains("CANARYQA") && !their_asks.contains(&a) && !their_asks.contains("Ana"), "{their_asks}");
    // A has read their question, so A's page is the same in both rounds below
    assert_eq!(act(&t, &code, &a_key, "seen_asks", json!({})).await.status, StatusCode::OK);

    let before = every_route(&t, &code, &host_key, &a_key).await;
    // (1) Canaries: the host and strangers never see a question or answer; A sees only their pair's
    for (route, status, body) in &before {
        let text = body.to_string();
        if !route.starts_with("guest A") || route.contains("as host") {
            for canary in ["CANARYQA", "CANARYQS", "CANARYANS", "CANARYQX"] {
                assert!(!text.contains(canary), "{route} ({status}) shows {canary}: {text}");
            }
            for field in ["\"match\":{", "\"my_santa\":{", "\"reveal_loop\":[", "\"receiver\"", "\"giver\"", "\"ring\""] {
                assert!(!text.contains(field), "{route} ({status}) has {field}: {text}");
            }
        } else {
            assert!(!text.contains("CANARYQX"), "{route}: A sees another pair's question");
            assert!(!text.contains("\"ring\"") && !text.contains("\"reveal_loop\":[") && !text.contains("\"my_santa\":{"), "{route}: {text}");
        }
        // Nobody's answer names A's Santa as A's Santa (only the reveal does)
        if route.starts_with("guest A") && *status == StatusCode::OK && body["view"]["match"].is_object() {
            assert_eq!(body["view"]["match"]["name"], json!(name_of(&a_receiver)), "{route}");
        }
    }
    assert!(before.iter().any(|(r, s, b)| *r == format!("guest A GET /api/pocket/{code}/state") && *s == StatusCode::OK && b.to_string().contains("CANARYQA")));

    // (2) Rewrite the loop among the three others; A's pair and A's Santa stay
    let mut new_loop = loop_.clone();
    let spots: Vec<usize> = (0..6).filter(|&k| others.contains(&loop_[k])).collect();
    let rotated: Vec<String> = spots.iter().map(|&k| loop_[k].clone()).cycle().skip(1).take(3).collect();
    for (k, id) in spots.iter().zip(rotated) {
        new_loop[*k] = id;
    }
    assert_ne!(new_loop, loop_);
    let changed = loop_.iter().filter(|g| receiver(&loop_, g) != receiver(&new_loop, g)).count();
    assert!(changed >= 2, "other people's matches really changed ({changed})");
    assert_eq!((receiver(&new_loop, &a), santa_of(&new_loop, &a)), (a_receiver.clone(), a_santa.clone()));
    set_ring(&t, &code, &new_loop, &a).await;
    let after = every_route(&t, &code, &host_key, &a_key).await;
    for ((route, s1, b1), (_, s2, b2)) in before.iter().zip(after.iter()) {
        assert_eq!(s1, s2, "{route}");
        assert_eq!(b1, b2, "{route} changed when only other people's matches did");
    }

    // The event log: counts only, never a player or a pair
    let events = t.state.db.pocket_events(&code).await.unwrap();
    assert!(!events.is_empty());
    for (player, kind, payload) in &events {
        assert!(player.is_none(), "{kind} names a player");
        for n in names {
            assert!(!payload.contains(n), "{kind} {payload}");
        }
        for id in &loop_ {
            assert!(!payload.contains(id.as_str()), "{kind} {payload}");
        }
        assert!(!payload.contains("CANARY"), "{kind} {payload}");
    }
}

#[tokio::test]
async fn the_host_sees_ticks_and_counts_and_the_reveal_shows_everyone_the_loop() {
    let t = setup().await;
    let (code, host_key) = make(&t, spec(Utc::now(), json!({ "people": ["Ana", "Ben", "Cleo", "Dev"] }))).await;
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await;
    assert_eq!(r.body["host_view"]["waiting_for"], json!(["Ana", "Ben", "Cleo", "Dev"]));
    assert_eq!(r.body["join"]["quick_tunnel"], false);
    assert_eq!(r.body["join"]["wifi_note"], "this_computer_only", "the guest port listens on 127.0.0.1 here");
    let mut ids = Vec::new();
    for n in ["Ana", "Ben", "Cleo"] {
        ids.push(joined(&t, &code, n).await);
    }
    assert_eq!(kit_state(&t, &code).await["phase"], "collecting");
    // The 4th of the 4 expected names draws at once
    ids.push(joined(&t, &code, "Dev").await);
    let host = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(&host_key)).await.body;
    assert_eq!(host["view"]["phase"], "drawn");
    assert_eq!(host["view"]["waiting_for"], json!([]));
    act(&t, &code, &ids[1].1, "open_match", json!({})).await;
    assert_eq!(act(&t, &code, &ids[1].1, "gift_ready", json!({})).await.status, StatusCode::OK);
    assert_eq!(act(&t, &code, &ids[2].1, "gift_ready", json!({})).await.status, StatusCode::CONFLICT, "open your match first");
    let host = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body;
    let people = host["host_view"]["people"].as_array().unwrap();
    let ben = people.iter().find(|p| p["name"] == "Ben").unwrap();
    assert_eq!((ben["looked"].as_bool(), ben["gift_ready"].as_bool()), (Some(true), Some(true)));
    assert_eq!(host["host_view"]["counts"], json!({ "joined": 4, "looked": 1, "gifts_ready": 1, "questions_asked": 0, "questions_answered": 0 }));
    assert_eq!(host["summary"]["looked"], 1);
    // The reveal: everyone sees the whole loop and their own Santa
    let r = host_act(&t, &code, "reveal_now", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let loop_ = ring(&t, &code).await;
    let v = state_of(&t, &code, &ids[0].1).await["view"].clone();
    assert_eq!(v["phase"], "revealed");
    assert_eq!(v["reveal_loop"].as_array().unwrap().len(), 4);
    assert_eq!(v["my_santa"]["id"], json!(santa_of(&loop_, &ids[0].0)));
    // Still nothing about matches for the host after the reveal (the card stays counts only)
    let host = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body.to_string();
    assert!(!host.contains("reveal_loop\":[") && !host.contains("\"receiver\""), "{host}");
    assert_eq!(act(&t, &code, &ids[0].1, "ask", json!({ "text": "late?" })).await.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn keep_apart_pairs_that_cant_be_drawn_refuse_with_a_reason() {
    let t = setup().await;
    let (code, _) = make(&t, spec(Utc::now(), json!({}))).await;
    for n in ["Mia", "Sam", "Ana"] {
        joined(&t, &code, n).await;
    }
    let r = host_act(&t, &code, "draw_now", json!({})).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("not_now")));
    assert!(r.body["message"].as_str().unwrap().contains("no one to buy for"), "{}", r.body);
    let host = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body;
    assert_eq!(host["host_view"]["draw"]["can_draw"], false);
    assert_eq!(host["host_view"]["draw"]["blocked"]["code"], "no_one_left");
    assert_eq!(host["host_view"]["needs_you"], true);
    // Removing the pair (or one more person) makes it drawable
    let r = host_act(&t, &code, "remove_pair", json!({ "a": "sam", "b": "MIA" })).await;
    assert_eq!(r.body["host_view"]["draw"]["can_draw"], true, "{}", r.body);
    assert_eq!(host_act(&t, &code, "draw_now", json!({})).await.status, StatusCode::OK);
    assert_eq!(host_act(&t, &code, "draw_now", json!({})).await.status, StatusCode::CONFLICT, "only once");
    // Pairs can't change after the draw, and a bad spec is refused at creation
    assert_eq!(host_act(&t, &code, "add_pair", json!({ "a": "Mia", "b": "Ana" })).await.status, StatusCode::CONFLICT);
    for bad in [json!({ "title": "x" }), json!({ "title": "x", "gift_on": "2026-12-18" }), json!({ "title": "x", "gift_on": "soon", "reveal": false }), json!({ "title": "x", "gift_on": "2026-12-18", "reveal": false, "keep_apart": [["Mia", "Mia"]] })] {
        let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "santa", "spec": bad })), &person()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}: {}", r.body);
    }
}

// ---- Santa questions ----

#[tokio::test]
async fn questions_are_capped_cleaned_and_limited_per_day() {
    let t = setup().await;
    let (code, _) = make(&t, spec(Utc::now(), json!({ "keep_apart": [] }))).await;
    let mut ids = Vec::new();
    for n in ["Ana", "Ben", "Cleo", "Dev"] {
        ids.push(joined(&t, &code, n).await);
    }
    // A player removed before the draw can't act at all
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/remove"), Some(json!({ "player_id": ids[3].0 })), &person()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(act(&t, &code, &ids[3].1, "ask", json!({ "text": "hi" })).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(host_act(&t, &code, "draw_now", json!({})).await.status, StatusCode::OK);
    // After the draw nobody can be removed (it would change someone else's match)
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/remove"), Some(json!({ "player_id": ids[2].0 })), &person()).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let k = ids[0].1.clone();
    act(&t, &code, &k, "open_match", json!({})).await;
    let r = act(&t, &code, &k, "ask", json!({ "text": "é".repeat(201) })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_action")), "the 201st character");
    let r = act(&t, &code, &k, "ask", json!({ "text": "Like this? https://shop.example/x www.spam.io amazon.com/dp/1 ok" })).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["view"]["asks"]["sent"][0]["q"], "Like this? ok", "links are stripped");
    assert_eq!(act(&t, &code, &k, "ask", json!({ "text": "https://only.link" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(act(&t, &code, &k, "ask", json!({ "text": "you sh1t" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(act(&t, &code, &k, "ask", json!({ "text": "é".repeat(200) })).await.status, StatusCode::OK, "200 is fine");
    for i in 3..=10 {
        let r = act(&t, &code, &k, "ask", json!({ "text": format!("question {i}") })).await;
        assert_eq!(r.status, StatusCode::OK, "#{i}: {}", r.body);
    }
    let r = act(&t, &code, &k, "ask", json!({ "text": "the 11th" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "the 11th of the day: {}", r.body);
    assert_eq!(state_of(&t, &code, &k).await["view"]["asks"]["left_today"], 0);
    // Only the asker's receiver answers, once; another guest can't answer it
    let loop_ = ring(&t, &code).await;
    let rk = ids.iter().find(|(p, _)| *p == receiver(&loop_, &ids[0].0)).unwrap().1.clone();
    let other = ids[..3].iter().find(|(p, _)| *p != ids[0].0 && *p != receiver(&loop_, &ids[0].0)).unwrap().1.clone();
    let q = state_of(&t, &code, &rk).await["view"]["asks"]["received"][0]["id"].clone();
    assert_eq!(act(&t, &code, &other, "answer", json!({ "id": q, "text": "x" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(act(&t, &code, &rk, "answer", json!({ "id": q, "text": "Size 38" })).await.status, StatusCode::OK);
    assert_eq!(act(&t, &code, &rk, "answer", json!({ "id": q, "text": "again" })).await.status, StatusCode::CONFLICT);
    let sent = state_of(&t, &code, &k).await["view"]["asks"]["sent"][0].clone();
    assert_eq!((sent["a"].as_str(), sent["new_answer"].as_bool()), (Some("Size 38"), Some(true)));
    // Unknown actions and host actions from a guest are refused
    assert_eq!(act(&t, &code, &k, "draw_now", json!({})).await.status, StatusCode::BAD_REQUEST);
}

// ---- Late joiners ----

#[tokio::test]
async fn a_late_joiner_redraws_while_nobody_looked_and_changes_one_match_after() {
    let t = setup().await;
    let (code, _) = make(&t, spec(Utc::now(), json!({ "keep_apart": [] }))).await;
    let mut ids = Vec::new();
    for n in ["Ana", "Ben", "Cleo", "Dev"] {
        ids.push(joined(&t, &code, n).await);
    }
    assert_eq!(host_act(&t, &code, "draw_now", json!({})).await.status, StatusCode::OK);
    // Only names the host let in may join after the draw
    let r = join(&t, &code, "Zoe").await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("not_now")), "{}", r.body);
    assert_eq!(host_act(&t, &code, "add_late", json!({ "name": "Zoe" })).await.status, StatusCode::OK);
    let host = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body;
    assert_eq!(host["host_view"]["late_allowed"], json!(["Zoe"]));
    // Nobody has looked: a fresh draw with all 5
    let zoe = joined(&t, &code, "Zoe").await;
    let loop5 = ring(&t, &code).await;
    assert_eq!(loop5.len(), 5);
    assert!(loop5.contains(&zoe.0));
    // Now people look, and Yan joins late: exactly one match changes, to Yan
    for (_, k) in &ids {
        assert_eq!(act(&t, &code, k, "open_match", json!({})).await.status, StatusCode::OK);
    }
    assert_eq!(host_act(&t, &code, "add_late", json!({ "name": "Yan", "keep_apart": "Zoe" })).await.status, StatusCode::OK);
    let yan = joined(&t, &code, "Yan").await;
    let loop6 = ring(&t, &code).await;
    assert_eq!(loop6.len(), 6);
    let changed: Vec<&String> = loop5.iter().filter(|g| receiver(&loop5, g) != receiver(&loop6, g)).collect();
    assert_eq!(changed.len(), 1, "exactly one existing match changes: {loop5:?} -> {loop6:?}");
    assert_eq!(receiver(&loop6, changed[0]), yan.0);
    assert_ne!(receiver(&loop6, &yan.0), zoe.0, "kept apart");
    assert_ne!(santa_of(&loop6, &yan.0), zoe.0, "kept apart");
    // The changed giver is told and opens again; nobody else's page changes
    let ck = [ids.clone(), vec![zoe.clone()]].concat().into_iter().find(|(p, _)| p == changed[0]).unwrap().1;
    let v = state_of(&t, &code, &ck).await["view"].clone();
    assert_eq!(v["notices"][0]["kind"], "match_changed");
    assert!(v["match"].is_null() && v["can_open"] == true);
    let v = act(&t, &code, &ck, "open_match", json!({})).await.body["view"].clone();
    assert_eq!(v["match"]["id"], json!(yan.0));
    assert_eq!(v["notices"], json!([]));
    for (p, k) in &ids {
        if p != changed[0] {
            let v = state_of(&t, &code, k).await["view"].clone();
            assert_eq!(v["match"]["id"], json!(receiver(&loop5, p)), "{p} keeps their match");
            assert_eq!(v["notices"], json!([]));
        }
    }
}

/// What the host sees: the card's view and summary (main port) and the host page's view.
async fn host_sees(t: &T, code: &str, host_key: &str) -> Value {
    let card = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body;
    let page = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(host_key)).await.body;
    steady(json!({ "host_view": card["host_view"], "summary": card["summary"], "page": page["view"] }))
}

#[tokio::test]
async fn a_late_joiner_leaves_the_host_view_byte_identical() {
    // F2 (pa10a QA): placing a late joiner after everyone looked used to drop the changed
    // giver's ticks, which told the host (who typed the name) who buys for the late joiner
    let t = setup().await;
    let (code, host_key) = make(&t, spec(Utc::now(), json!({ "keep_apart": [] }))).await;
    let mut ids = Vec::new();
    for n in ["Ana", "Ben", "Cleo", "Dev", "Eli"] {
        ids.push(joined(&t, &code, n).await);
    }
    assert_eq!(host_act(&t, &code, "draw_now", json!({})).await.status, StatusCode::OK);
    // Everyone looked, ticked the gift and asked a question, so whoever the giver is has it all
    for (_, k) in &ids {
        for (name, body) in [("open_match", json!({})), ("gift_ready", json!({})), ("ask", json!({ "text": "Size?" }))] {
            assert_eq!(act(&t, &code, k, name, body).await.status, StatusCode::OK, "{name}");
        }
    }
    assert_eq!(host_act(&t, &code, "add_late", json!({ "name": "Yan" })).await.status, StatusCode::OK);
    let loop5 = ring(&t, &code).await;
    let before = host_sees(&t, &code, &host_key).await;
    let yan = joined(&t, &code, "Yan").await;
    let loop6 = ring(&t, &code).await;
    let giver = santa_of(&loop6, &yan.0);
    assert_eq!(receiver(&loop6, &yan.0), receiver(&loop5, &giver), "Yan sits between one giver and their old receiver");
    let mut after = host_sees(&t, &code, &host_key).await;
    // Take out exactly what one more person adds (their own unticked row and the joined count,
    // and the late name is used up); every other byte must be as before
    for part in ["host_view", "page"] {
        let v = &mut after[part];
        let people = v["people"].as_array_mut().unwrap();
        let k = people.iter().position(|p| p["id"] == json!(yan.0)).unwrap();
        let row = people.remove(k);
        assert_eq!((row["looked"].as_bool(), row["gift_ready"].as_bool()), (Some(false), Some(false)), "{row}");
        assert_eq!(v["late_allowed"], json!([]));
        v["late_allowed"] = json!(["Yan"]);
        v["joined"] = json!(v["joined"].as_i64().unwrap() - 1);
        v["counts"]["joined"] = json!(v["counts"]["joined"].as_i64().unwrap() - 1);
    }
    after["summary"]["joined"] = json!(after["summary"]["joined"].as_i64().unwrap() - 1);
    assert_eq!(after.to_string(), before.to_string(), "the host's view is byte-identical apart from the new person");
    // The giver's own page says their match changed; opening it and ticking again changes
    // nothing the host sees either
    let gk = ids.iter().find(|(p, _)| *p == giver).unwrap().1.clone();
    let v = state_of(&t, &code, &gk).await["view"].clone();
    assert_eq!((v["notices"][0]["kind"].as_str(), v["gift_ready"].as_bool(), v["match"].is_null()), (Some("match_changed"), Some(false), true), "{v}");
    assert_eq!(act(&t, &code, &gk, "ask", json!({ "text": "Colour?" })).await.status, StatusCode::CONFLICT, "open the new match first");
    let still = host_sees(&t, &code, &host_key).await;
    let v = act(&t, &code, &gk, "open_match", json!({})).await.body["view"].clone();
    assert_eq!((v["match"]["id"].as_str(), v["gift_ready"].as_bool(), v["asks"]["sent"].clone()), (Some(yan.0.as_str()), Some(false), json!([])));
    assert_eq!(act(&t, &code, &gk, "gift_ready", json!({})).await.status, StatusCode::OK);
    assert_eq!(host_sees(&t, &code, &host_key).await, still, "reopening and re-ticking don't show on the host's side");
}

#[tokio::test]
async fn past_dates_and_a_reveal_long_before_the_gift_day_are_refused_at_creation() {
    // F3 (pa10a QA): everything goes 7 days after the reveal (else the gift day), so a past
    // date deleted the pocket within a minute or showed every match right after the draw
    let t = setup().await;
    let now = Utc::now();
    let day = |d: i64| (now + Duration::days(d)).format("%Y-%m-%d").to_string();
    let at = |d: Duration| (now + d).to_rfc3339();
    let empty = send(&t.main, "GET", "/api/pockets", None, &person()).await.body;
    for (spec, says) in [
        (json!({ "title": "x", "gift_on": day(-400), "reveal_at": format!("{}T18:00:00Z", day(-400)) }), "gift_on is already past"),
        (json!({ "title": "x", "gift_on": day(-3), "reveal": false }), "gift_on is already past"),
        (json!({ "title": "x", "gift_on": day(10), "reveal_at": at(-Duration::hours(1)) }), "reveal_at is already past"),
        (json!({ "title": "x", "gift_on": day(10), "reveal_at": at(Duration::days(3)) }), "reveal_at is more than a day before the gift day"),
    ] {
        let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "santa", "spec": spec })), &person()).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_spec")), "{spec}: {}", r.body);
        assert!(r.body["message"].as_str().unwrap_or_default().starts_with(says), "{spec}: {}", r.body);
    }
    assert_eq!(send(&t.main, "GET", "/api/pockets", None, &person()).await.body, empty, "nothing was made");
    // A normal spec, and one revealed the evening before the gift day, are fine
    make(&t, spec(now, json!({}))).await;
    make(&t, json!({ "title": "x", "gift_on": day(10), "reveal_at": format!("{}T19:00:00Z", day(9)) })).await;
}

// ---- Timers and retention (fake clock) ----

#[tokio::test]
async fn draw_at_and_reveal_at_run_unwatched_and_everything_goes_at_reveal_plus_7_days() {
    let t = setup().await;
    let now = Utc::now();
    let draw_at = now + Duration::days(1);
    let (code, _) = make(&t, spec(now, json!({ "keep_apart": [], "draw_at": draw_at.to_rfc3339() }))).await;
    let reveal: DateTime<Utc> = DateTime::parse_from_rfc3339(spec(now, json!({}))["reveal_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await;
    assert_eq!(r.body["pocket"]["expires_at"].as_str().map(|s| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)), Some(reveal + Duration::days(7)));
    let mut ids = Vec::new();
    for n in ["Ana", "Ben", "Cleo"] {
        ids.push(joined(&t, &code, n).await);
    }
    assert_eq!(pocket::sweep(&t.state, now + Duration::hours(23)).await, pocket::Swept::default());
    assert_eq!(kit_state(&t, &code).await["phase"], "collecting");
    pocket::sweep(&t.state, draw_at + Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["phase"], "drawn", "draw_at draws though nobody watches");
    act(&t, &code, &ids[0].1, "open_match", json!({})).await;
    act(&t, &code, &ids[0].1, "ask", json!({ "text": "Size?" })).await;
    act(&t, &code, &ids[1].1, "set_wish", json!({ "text": "Warm socks" })).await;
    pocket::sweep(&t.state, reveal - Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["phase"], "drawn");
    pocket::sweep(&t.state, reveal + Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["phase"], "revealed", "the reveal runs on time");
    assert_eq!(t.state.db.pocket_get(&code).await.unwrap().unwrap().state, "open", "still open for a week of thank-yous");
    let counts = t.state.db.pocket_row_counts(&code).await.unwrap();
    assert!(counts.iter().all(|n| *n > 0), "{counts:?}");
    assert_eq!(pocket::sweep(&t.state, reveal + Duration::days(7) - Duration::minutes(1)).await.deleted, 0, "not at +6 days 23:59");
    assert_eq!(pocket::sweep(&t.state, reveal + Duration::days(7) + Duration::minutes(1)).await.deleted, 1);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0], "players, keys, events, questions and wishes all gone");
    assert_eq!(send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &ids[0].1)).await.status, StatusCode::NOT_FOUND);
    // With the reveal skipped, it never shows and the data still goes on time
    let (code2, _) = make(&t, spec(now, json!({ "keep_apart": [] }))).await;
    for n in ["Ana", "Ben", "Cleo"] {
        joined(&t, &code2, n).await;
    }
    host_act(&t, &code2, "draw_now", json!({})).await;
    assert_eq!(host_act(&t, &code2, "skip_reveal", json!({})).await.status, StatusCode::OK);
    pocket::sweep(&t.state, reveal + Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code2).await["phase"], "drawn");
    assert_eq!(pocket::sweep(&t.state, reveal + Duration::days(7) + Duration::minutes(1)).await.deleted, 1);
}

// ---- PIN guessing ----

async fn rejoin(t: &T, code: &str, name: &str, pin: &str) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(json!({ "name": name, "pin": pin })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await
}

#[tokio::test]
async fn santa_needs_a_pin_and_ten_wrong_guesses_lock_the_name() {
    let t = setup().await;
    let (code, _) = make(&t, spec(Utc::now(), json!({ "keep_apart": [] }))).await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": "Ana" })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.body["error"], "pin_required");
    let (id, _) = joined(&t, &code, "Ana").await;
    let mut ids = vec![id.clone()];
    for n in ["Ben", "Cleo"] {
        ids.push(joined(&t, &code, n).await.0);
    }
    host_act(&t, &code, "draw_now", json!({})).await;
    for _ in 0..4 {
        assert_eq!(rejoin(&t, &code, "Ana", "0000").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(rejoin(&t, &code, "Ana", "0000").await.body["error"], "locked", "the 5th waits 10 minutes");
    assert_eq!(rejoin(&t, &code, "Ana", PIN).await.body["error"], "locked", "even the right PIN waits");
    let p = t.state.db.pocket_player(&code, &id).await.unwrap().unwrap();
    let c = kwnt::db::pocket::PinCounters { wrong_tries: p.wrong_tries, total_wrong: p.total_wrong, locked_until: Some(Utc::now() - Duration::seconds(1)), pin_locked: p.pin_locked, last_wrong_at: p.last_wrong_at };
    t.state.db.pocket_set_pin_counters(&code, &id, &c).await.unwrap();
    for _ in 0..4 {
        assert_eq!(rejoin(&t, &code, "Ana", "1111").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(rejoin(&t, &code, "Ana", "1111").await.body["error"], "hard_locked");
    assert_eq!(rejoin(&t, &code, "Ana", PIN).await.body["error"], "hard_locked");
    // The person's reset from the card: the code and its expiry, nothing else
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/reset-pin"), Some(json!({ "player_id": id })), &person()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body.as_object().unwrap().len(), 2, "{}", r.body);
    let reset = r.body["code"].as_str().unwrap();
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(json!({ "name": "Ana", "reset_code": reset, "new_pin": "2468" })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // A guessed name of someone else gets nothing about them: the wrong-PIN fields only (its
    // message says "don't match", so the word itself isn't the test), and no player's id or
    // other name
    let r = rejoin(&t, &code, "Ben", "9999").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let mut keys: Vec<&String> = r.body.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["error", "message", "tries_left"], "{}", r.body);
    assert_eq!(r.body["error"], "wrong_pin");
    let text = r.body.to_string();
    assert!(ids.iter().all(|i| !text.contains(i.as_str())) && !text.contains("Ana") && !text.contains("Cleo"), "{text}");
}
