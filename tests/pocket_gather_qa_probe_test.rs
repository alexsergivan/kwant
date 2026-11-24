#![allow(warnings, clippy::all)]
//! Rigel QA probe for pa10b gather (PR #1133 head ee9e302b). Not for merge.
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

/// A gather spec closing in 5 days, the event in 7; `extra` adds or replaces fields.
fn spec(now: DateTime<Utc>, face: &str, extra: Value) -> Value {
    let mut s = json!({
        "face": face,
        "title": "Club dinner",
        "when": "Sat, 7:30 PM",
        "place": "The Old Mill",
        "host_name": "Oleks",
        "closes_at": (now + Duration::days(5)).to_rfc3339(),
        "event_at": (now + Duration::days(7)).to_rfc3339(),
    });
    match face {
        "signup" => s["slots"] = json!([{ "name": "Desserts", "cap": 1 }, { "name": "Drinks", "cap": 2 }]),
        "vote" => s["ballot"] = json!({ "kind": "ranked", "options": ["The Old Mill", "Casa Lupa", "Golden Lotus", "Pizza Uno"] }),
        _ => {}
    }
    for (k, v) in extra.as_object().unwrap() {
        s[k] = v.clone();
    }
    s
}

/// Makes a gather pocket through the person's API: (code, host key).
async fn make(t: &T, spec: Value) -> (String, String) {
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "gather", "spec": spec })), &person()).await;
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

async fn ok(t: &T, code: &str, key: &str, name: &str, body: Value) {
    let r = act(t, code, key, name, body).await;
    assert_eq!(r.status, StatusCode::OK, "{name}: {}", r.body);
}

/// A host action from the person's card on the main port.
async fn host_act(t: &T, code: &str, name: &str, body: Value) -> Res {
    send(&t.main, "POST", &format!("/api/pockets/{code}/action"), Some(json!({ "name": name, "body": body })), &person()).await
}

async fn view_of(t: &T, code: &str, key: &str) -> Value {
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(code, key)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["view"].clone()
}

async fn host_view(t: &T, code: &str) -> Value {
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["host_view"].clone()
}

async fn kit_state(t: &T, code: &str) -> Value {
    serde_json::from_str(&t.state.db.pocket_get(code).await.unwrap().unwrap().kit_state).unwrap()
}

async fn save_kit_state(t: &T, code: &str, st: &Value, any_player: &str) {
    t.state.db.pocket_save_state(code, &st.to_string(), &[], Utc::now()).await.unwrap();
    pocket::notices_seen(&t.state, code, any_player).await.unwrap();
}

/// Where a player's ballot is stored: the same salted key the kit uses.
fn ballot_key(salt: &str, player_id: &str) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, format!("{salt}:{player_id}").as_bytes());
    d.as_ref()[..12].iter().map(|x| format!("{x:02x}")).collect()
}

/// One call in `every_route`: who, which port, method, path, body, request.
type Call<'a> = (&'a str, &'a Router, &'a str, String, Option<Value>, Req<'a>);

/// Every route the person, the host remote, guest A or a stranger can call on a gather pocket,
/// with what each answered (status and steady body). Calls that would change the pocket are
/// ones that are refused, so two runs over the same state answer the same.
async fn every_route(t: &T, code: &str, host_key: &str, a_key: &str) -> Vec<(String, StatusCode, Value)> {
    let mut out = Vec::new();
    let g = |p: &str| format!("/api/pocket/{code}{p}");
    let m = |p: &str| format!("/api/pockets/{code}{p}");
    let stranger = || Req { ip: Some(fresh_ip()), ..Default::default() };
    let calls: Vec<Call> = vec![
        // The person, on the main port
        ("person", &t.main, "GET", "/api/pockets".into(), None, person()),
        ("person", &t.main, "GET", m(""), None, person()),
        ("person", &t.main, "POST", m("/action"), Some(json!({ "name": "copied", "body": { "to": "Nobody" } })), person()),
        ("person", &t.main, "POST", m("/remove"), Some(json!({ "player_id": "pl_none" })), person()),
        // The host remote, on the guest port
        ("host", &t.guest, "POST", g("/host/signin"), Some(json!({ "key": host_key })), Req { ip: Some(fresh_ip()), ..Default::default() }),
        ("host", &t.guest, "GET", g("/host/state"), None, host_req(host_key)),
        ("host", &t.guest, "GET", g("/host/state?view=screen"), None, host_req(host_key)),
        ("host", &t.guest, "POST", g("/host/action"), Some(json!({ "name": "copied", "body": { "to": "Nobody" } })), host_req(host_key)),
        // Guest A
        ("guest A", &t.guest, "GET", g("/state"), None, guest_req(code, a_key)),
        ("guest A", &t.guest, "GET", g("/info"), None, guest_req(code, a_key)),
        ("guest A", &t.guest, "GET", format!("/p/{code}"), None, guest_req(code, a_key)),
        ("guest A", &t.guest, "POST", g("/join"), Some(json!({ "name": "Ana", "pin": PIN })), guest_req(code, a_key)),
        ("guest A", &t.guest, "POST", g("/action"), Some(json!({ "name": "seen" })), guest_req(code, a_key)),
        ("guest A", &t.guest, "POST", g("/action"), Some(json!({ "name": "pick", "body": { "option": 0 } })), guest_req(code, a_key)),
        ("guest A", &t.guest, "GET", g("/host/state"), None, guest_req(code, a_key)),
        ("guest A as host", &t.guest, "GET", g("/host/state"), None, host_req(a_key)),
        // A stranger with the link
        ("stranger", &t.guest, "GET", g("/info"), None, stranger()),
        ("stranger", &t.guest, "GET", format!("/p/{code}"), None, stranger()),
        ("stranger", &t.guest, "GET", g("/state"), None, stranger()),
        ("stranger", &t.guest, "POST", g("/action"), Some(json!({ "name": "vote", "body": { "ranking": [0] } })), stranger()),
        ("stranger", &t.guest, "POST", g("/join"), Some(json!({ "name": "Eve" })), stranger()),
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

/// Swaps two players' stored ballots (everything else kept) and reloads the pocket.
async fn swap_ballots(t: &T, code: &str, x: &str, y: &str, any_player: &str) {
    let mut st = kit_state(t, code).await;
    let salt = st["ballot_salt"].as_str().unwrap().to_string();
    let (kx, ky) = (ballot_key(&salt, x), ballot_key(&salt, y));
    let (bx, by) = (st["ballots"][&kx].clone(), st["ballots"][&ky].clone());
    assert!(bx.is_array() && by.is_array() && bx != by, "{bx} {by}");
    st["ballots"][&kx] = by;
    st["ballots"][&ky] = bx;
    save_kit_state(t, code, &st, any_player).await;
}

fn same(before: &[(String, StatusCode, Value)], after: &[(String, StatusCode, Value)], when: &str) {
    assert_eq!(before.len(), after.len());
    for ((route, s1, b1), (_, s2, b2)) in before.iter().zip(after.iter()) {
        assert_eq!(s1, s2, "{when}: {route}");
        assert_eq!(b1, b2, "{when}: {route} changed when only other people's ballots did");
    }
}

// ======================= Rigel's QA probes (not for merge) =======================

fn caps_hold(v: &Value) {
    for s in v["slots"].as_array().unwrap() {
        assert!(s["filled"].as_u64().unwrap() <= s["cap"].as_u64().unwrap(), "cap passed: {s}");
        if let Some(p) = s["people"].as_array() { assert!(p.len() as u64 <= s["cap"].as_u64().unwrap(), "cap passed: {s}"); }
    }
}

async fn host_remove(t: &T, code: &str, id: &str) -> Res {
    send(&t.main, "POST", &format!("/api/pockets/{code}/remove"), Some(json!({ "player_id": id })), &person()).await
}

fn moved_up_events(events: &[(Option<String>, String, String)]) -> usize {
    events.iter().filter(|(_, k, _)| k == "moved_up").count()
}

// 1. Secret ballot in a tie and after a revote (Lin's test swaps ballots only while open and after the first close)
#[tokio::test]
async fn qa1_secret_ballot_holds_in_a_tie_and_after_a_revote() {
    let t = setup().await;
    let now = Utc::now();
    let (code, host_key) = make(&t, spec(now, "vote", json!({}))).await;
    let mut p = Vec::new();
    for n in ["Ana", "Ben", "Cleo", "Dev"] {
        p.push(joined(&t, &code, n).await);
    }
    let ballots = [json!([0, 1]), json!([0, 2]), json!([1, 0]), json!([1, 3])];
    for ((_, k), b) in p.iter().zip(ballots.iter()) {
        ok(&t, &code, k, "vote", json!({ "ranking": b })).await;
    }
    let a_key = p[0].1.clone();
    let r = host_act(&t, &code, "close_now", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let h = host_view(&t, &code).await;
    assert_eq!((h["phase"].as_str(), h["needs_you"].as_bool()), (Some("tie"), Some(true)), "{}", h["result"]);
    // During the tie: swap Ben's and Cleo's ballots (different 1st choices)
    let before = every_route(&t, &code, &host_key, &a_key).await;
    for (route, status, body) in &before {
        let text = body.to_string();
        assert!(!text.contains("ballot_salt") && !text.contains("\"ballots\""), "{route} ({status}): {text}");
    }
    swap_ballots(&t, &code, &p[1].0, &p[2].0, &p[0].0).await;
    same(&before, &every_route(&t, &code, &host_key, &a_key).await, "tie");

    // Vote again: new salt, ballots cleared, closes in 24 h, only the tied options
    let salt1 = kit_state(&t, &code).await["ballot_salt"].as_str().unwrap().to_string();
    let old_keys: Vec<String> = kit_state(&t, &code).await["ballots"].as_object().unwrap().keys().cloned().collect();
    let t0 = Utc::now();
    let r = host_act(&t, &code, "revote_24h", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let st = kit_state(&t, &code).await;
    assert_ne!(st["ballot_salt"].as_str().unwrap(), salt1, "a new salt");
    assert!(st["ballots"].as_object().unwrap().is_empty(), "ballots cleared");
    assert!(st["voted"].as_object().unwrap().is_empty(), "voted cleared");
    let c = DateTime::parse_from_rfc3339(st["closes_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    assert!(c >= t0 + Duration::hours(24) - Duration::seconds(2) && c <= Utc::now() + Duration::hours(24) + Duration::seconds(2), "closes in 24 h: {c}");
    let v = view_of(&t, &code, &a_key).await;
    assert_eq!(v["ballot"]["in_play"], json!([0, 1]));
    assert_eq!(v["me"], json!({ "voted": false, "ballot": null }));
    assert_eq!(host_view(&t, &code).await["voters"], json!([]));
    let again = [json!([0, 1]), json!([0, 1]), json!([1, 0]), json!([1, 0])];
    for ((_, k), b) in p.iter().zip(again.iter()) {
        ok(&t, &code, k, "vote", json!({ "ranking": b })).await;
    }
    let st = kit_state(&t, &code).await;
    for k in st["ballots"].as_object().unwrap().keys() {
        assert!(!old_keys.contains(k), "a ballot key from before the revote: {k}");
    }
    // Open again, after a revote
    let before = every_route(&t, &code, &host_key, &a_key).await;
    swap_ballots(&t, &code, &p[1].0, &p[2].0, &p[0].0).await;
    same(&before, &every_route(&t, &code, &host_key, &a_key).await, "revote open");
    // Second tie, then the host picks: closed
    assert_eq!(host_act(&t, &code, "close_now", json!({})).await.status, StatusCode::OK);
    assert_eq!(host_view(&t, &code).await["phase"], "tie");
    assert_eq!(host_act(&t, &code, "pick", json!({ "option": 1 })).await.status, StatusCode::OK);
    let before = every_route(&t, &code, &host_key, &a_key).await;
    for (route, status, body) in &before {
        let text = body.to_string();
        assert!(!text.contains("ballot_salt") && !text.contains("\"ballots\""), "{route} ({status}): {text}");
    }
    swap_ballots(&t, &code, &p[1].0, &p[2].0, &p[0].0).await;
    same(&before, &every_route(&t, &code, &host_key, &a_key).await, "after the pick");
    let events = t.state.db.pocket_events(&code).await.unwrap();
    for (player, kind, payload) in &events {
        assert!(player.is_none(), "{kind} names a player");
        assert!(!kind.contains("vote") || kind == "revote", "{kind}");
        for (id, _) in &p {
            assert!(!payload.contains(id.as_str()), "{kind} {payload}");
        }
    }
}

// 1b. Probe (a confirmation, by design): the host's live bars and the voter rows move together
#[tokio::test]
async fn qa1b_probe_host_live_bars_name_the_first_voters_choice_and_a_removal_moves_them() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "vote", json!({}))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let (ben_id, b) = joined(&t, &code, "Ben").await;
    ok(&t, &code, &a, "vote", json!({ "ranking": [2, 0] })).await;
    let h = host_view(&t, &code).await;
    let bars: Vec<u64> = h["live"].as_array().unwrap().iter().map(|x| x["votes"].as_u64().unwrap()).collect();
    assert_eq!((h["voters"].as_array().unwrap().len(), bars.clone()), (1, vec![0, 0, 1, 0]), "one voter, one bar: the host can read Ana's 1st choice");
    ok(&t, &code, &b, "vote", json!({ "ranking": [1] })).await;
    let bars2: Vec<u64> = host_view(&t, &code).await["live"].as_array().unwrap().iter().map(|x| x["votes"].as_u64().unwrap()).collect();
    assert_eq!(bars2, vec![0, 1, 1, 0], "the bar that moved after Ben's row appeared is Ben's");
    let r = host_remove(&t, &code, &ben_id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let bars3: Vec<u64> = host_view(&t, &code).await["live"].as_array().unwrap().iter().map(|x| x["votes"].as_u64().unwrap()).collect();
    assert_eq!(bars3, vec![0, 0, 1, 0], "removing Ben drops his bar");
}

// 3. Waiting list boundaries
#[tokio::test]
async fn qa3_waiting_list_never_passes_a_cap_and_moves_the_first_in_line_in() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "signup", json!({}))).await; // Desserts cap 1, Drinks cap 2
    let (_ana_id, ana) = joined(&t, &code, "Ana").await;
    let (_, ben) = joined(&t, &code, "Ben").await;
    let (_, cleo) = joined(&t, &code, "Cleo").await;
    let (_, dev) = joined(&t, &code, "Dev").await;
    ok(&t, &code, &ana, "take", json!({ "slot": 0 })).await;
    ok(&t, &code, &ben, "take", json!({ "slot": 0 })).await;
    ok(&t, &code, &cleo, "take", json!({ "slot": 0 })).await;
    caps_hold(&host_view(&t, &code).await);
    // Ben, in line for Desserts, takes Drinks: he leaves the line, nobody moves in (nobody left a place)
    ok(&t, &code, &ben, "take", json!({ "slot": 1 })).await;
    let c = view_of(&t, &code, &cleo).await;
    assert_eq!(c["me"]["waiting"], json!({ "slot": 0, "position": 1 }), "{}", c["me"]);
    assert_eq!(c["me"]["slot"], Value::Null);
    // Ana moves to Drinks: Desserts frees and Cleo, first in line, moves in at once
    ok(&t, &code, &ana, "take", json!({ "slot": 1 })).await;
    let c = view_of(&t, &code, &cleo).await;
    assert_eq!((c["me"]["slot"].as_u64(), c["me"]["moved_up"]["new"].as_bool(), c["me"]["moved_up"]["slot"].as_u64()), (Some(0), Some(true), Some(0)), "{}", c["me"]);
    assert_eq!(c["me"]["waiting"], Value::Null);
    let h = host_view(&t, &code).await;
    caps_hold(&h);
    assert_eq!(h["slots"][1]["people"].as_array().unwrap().len(), 2);
    // The Chaser drafts a "you're in" note for Cleo with the slot's name; copying it clears the draft
    let d = h["drafts"].as_array().unwrap().iter().find(|d| d["to"] == "Cleo").unwrap().clone();
    assert_eq!(d["kind"], "moved_up");
    assert!(d["text"].as_str().unwrap().contains("Desserts"), "{d}");
    assert_eq!(host_act(&t, &code, "copied", json!({ "to": "Cleo" })).await.status, StatusCode::OK);
    assert!(host_view(&t, &code).await["drafts"].as_array().unwrap().iter().all(|d| d["to"] != "Cleo"), "a copied moved-up note is gone");
    // Dev joins the full Drinks line (position 1), Cleo lets go (Desserts: nobody waiting), then Ben lets go: Dev moves in
    ok(&t, &code, &dev, "take", json!({ "slot": 1 })).await;
    assert_eq!(view_of(&t, &code, &dev).await["me"]["waiting"], json!({ "slot": 1, "position": 1 }));
    ok(&t, &code, &cleo, "let_go", json!({})).await;
    let d = view_of(&t, &code, &dev).await;
    assert_eq!(d["me"]["slot"], Value::Null, "Dev is still waiting");
    ok(&t, &code, &ben, "leave", json!({})).await;
    let d = view_of(&t, &code, &dev).await;
    assert_eq!((d["me"]["slot"].as_u64(), d["me"]["moved_up"]["new"].as_bool()), (Some(1), Some(true)), "{}", d["me"]);
    caps_hold(&host_view(&t, &code).await);
    // Someone in no slot and no line gets 409
    let r = act(&t, &code, &ben, "leave", json!({})).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
    // Exactly two moves-up (Cleo, Dev), none naming a person
    let events = t.state.db.pocket_events(&code).await.unwrap();
    assert_eq!(moved_up_events(&events), 2, "{events:?}");
    assert!(events.iter().all(|(p, _, payload)| p.is_none() && !payload.contains("pl_")));
}

#[tokio::test]
async fn qa3b_no_waiting_list_refuses_a_full_slot_and_keeps_the_old_place() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "signup", json!({ "waiting_list": false }))).await;
    let (_, ana) = joined(&t, &code, "Ana").await;
    let (_, ben) = joined(&t, &code, "Ben").await;
    ok(&t, &code, &ana, "take", json!({ "slot": 0 })).await;
    ok(&t, &code, &ben, "take", json!({ "slot": 1 })).await;
    let r = act(&t, &code, &ben, "take", json!({ "slot": 0 })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
    assert_eq!(view_of(&t, &code, &ben).await["me"]["slot"], 1, "Ben keeps Drinks");
    let h = host_view(&t, &code).await;
    assert_eq!(h["slots"][0]["waiting"], 0);
    // A bad slot number
    assert_eq!(act(&t, &code, &ben, "take", json!({ "slot": 9 })).await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn qa3c_host_removal_while_open_moves_the_first_in_line_in() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "signup", json!({}))).await;
    let (ana_id, ana) = joined(&t, &code, "Ana").await;
    let (_, ben) = joined(&t, &code, "Ben").await;
    ok(&t, &code, &ana, "take", json!({ "slot": 0 })).await;
    ok(&t, &code, &ben, "take", json!({ "slot": 0 })).await;
    let r = host_remove(&t, &code, &ana_id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let b = view_of(&t, &code, &ben).await;
    assert_eq!((b["me"]["slot"].as_u64(), b["me"]["moved_up"]["new"].as_bool()), (Some(0), Some(true)), "{}", b["me"]);
}

// 3d. Probe: a removal after the close (the line is frozen to guests, but the host can still remove)
#[tokio::test]
async fn qa3d_probe_host_removal_after_the_close_moves_the_line_in() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "signup", json!({}))).await;
    let (ana_id, ana) = joined(&t, &code, "Ana").await;
    let (_, ben) = joined(&t, &code, "Ben").await;
    ok(&t, &code, &ana, "take", json!({ "slot": 0 })).await;
    ok(&t, &code, &ben, "take", json!({ "slot": 0 })).await;
    pocket::sweep(&t.state, now + Duration::days(5) + Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["phase"], "closed");
    let r = host_remove(&t, &code, &ana_id).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let b = view_of(&t, &code, &ben).await;
    assert_eq!(b["me"]["slot"], 0, "Desserts is free and Ben was first in line: {}", b["me"]);
}

// 4. RSVP paid ticks and answer changes
#[tokio::test]
async fn qa4_paid_ticks_follow_the_answer_and_work_after_the_close() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "rsvp", json!({ "fields": ["plus_ones", "food_needs"], "price": "€35" }))).await;
    let (ana_id, ana) = joined(&t, &code, "Ana").await;
    let (ben_id, ben) = joined(&t, &code, "Ben").await;
    let (zed_id, _zed) = joined(&t, &code, "Zed").await;
    ok(&t, &code, &ana, "answer", json!({ "answer": "yes", "plus_ones": 2, "food": ["vegan"] })).await;
    ok(&t, &code, &ben, "answer", json!({ "answer": "no" })).await;
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": ana_id })).await.status, StatusCode::OK);
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": ben_id })).await.status, StatusCode::CONFLICT, "a No");
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": zed_id })).await.status, StatusCode::CONFLICT, "never answered");
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": "pl_none" })).await.status, StatusCode::BAD_REQUEST);
    let row = |h: &Value, name: &str| h["people"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap().clone();
    let h = host_view(&t, &code).await;
    assert!(row(&h, "Ana")["paid_at"].is_string());
    // Yes -> Maybe: the tick, plus-ones and food go
    ok(&t, &code, &ana, "answer", json!({ "answer": "maybe", "plus_ones": 2, "food": ["vegan"] })).await;
    let h = host_view(&t, &code).await;
    let a = row(&h, "Ana");
    assert_eq!((a["paid_at"].clone(), a["plus_ones"].as_u64(), a["food"].clone()), (Value::Null, Some(0), json!([])), "{a}");
    assert_eq!(h["counts"]["plus_ones"], 0);
    assert_eq!(h["paid"]["ticked"], 0);
    let g = view_of(&t, &code, &ana).await;
    assert_eq!(g["me"]["paid"], Value::Null);
    // Back to Yes: not paid again
    ok(&t, &code, &ana, "answer", json!({ "answer": "yes" })).await;
    assert_eq!(row(&host_view(&t, &code).await, "Ana")["paid_at"], Value::Null);
    // Close (answers) but not the event: the host still ticks and unticks; guests can't answer
    pocket::sweep(&t.state, now + Duration::days(5) + Duration::minutes(1)).await;
    let st = kit_state(&t, &code).await;
    assert_eq!((st["phase"].as_str(), st["done"].as_bool()), (Some("closed"), Some(false)));
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": ana_id })).await.status, StatusCode::OK, "tick after the close");
    assert!(view_of(&t, &code, &ana).await["me"]["paid"]["at"].is_string());
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": ana_id, "paid": false })).await.status, StatusCode::OK, "untick");
    assert_eq!(act(&t, &code, &ana, "answer", json!({ "answer": "no" })).await.status, StatusCode::CONFLICT);
    // No price: no ticks
    let (code2, _) = make(&t, spec(now, "rsvp", json!({}))).await;
    let (id2, k2) = joined(&t, &code2, "Ana").await;
    ok(&t, &code2, &k2, "answer", json!({ "answer": "yes" })).await;
    assert_eq!(host_act(&t, &code2, "set_paid", json!({ "player_id": id2 })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(host_view(&t, &code2).await["paid"], Value::Null);
}

// 5. Chaser kinds, placeholders and the 48-hour boundary
#[tokio::test]
async fn qa5_chaser_drafts_per_kind_keep_the_link_and_the_48_hour_edge_holds() {
    let t = setup().await;
    let now = Utc::now();
    let tpl = "N={name} T={title} W={when} P={place} H={host} S={slot} L={link}";
    let (code, _) = make(&t, spec(now, "rsvp", json!({ "price": "€35", "members": ["Ana", "Cleo", "Dev", "Eli", "Fay"], "templates": { "maybe": tpl, "not_paid": tpl, "not_answered": tpl } }))).await;
    let (_, ana) = joined(&t, &code, "Ana").await; // yes, unpaid
    let (_, cleo) = joined(&t, &code, "Cleo").await; // maybe
    let (_, _eli) = joined(&t, &code, "Eli").await; // joined, never answered
    let (fay_id, fay) = joined(&t, &code, "Fay").await; // answered no
    ok(&t, &code, &ana, "answer", json!({ "answer": "yes" })).await;
    ok(&t, &code, &cleo, "answer", json!({ "answer": "maybe" })).await;
    ok(&t, &code, &fay, "answer", json!({ "answer": "no" })).await;
    let h = host_view(&t, &code).await;
    let mut got: Vec<(String, String, bool)> = h["drafts"].as_array().unwrap().iter().map(|d| (d["to"].as_str().unwrap().to_string(), d["kind"].as_str().unwrap().to_string(), d["player_id"].is_string())).collect();
    got.sort();
    let want: Vec<(String, String, bool)> = [("Ana", "not_paid", true), ("Cleo", "maybe", true), ("Dev", "not_answered", false), ("Eli", "not_answered", true)].iter().map(|(a, b, c)| (a.to_string(), b.to_string(), *c)).collect();
    assert_eq!(got, want, "{}", h["drafts"]);
    let cleo_d = h["drafts"].as_array().unwrap().iter().find(|d| d["to"] == "Cleo").unwrap().clone();
    assert_eq!(cleo_d["text"], "N=Cleo T=Club dinner W=Sat, 7:30 PM P=The Old Mill H=Oleks S= L={link}");
    assert_eq!(h["not_answered"], json!(["Dev", "Eli"]));
    // 47 h 59 m after a copy: refused, with the time in the message and in the view
    assert_eq!(host_act(&t, &code, "copied", json!({ "to": "Dev" })).await.status, StatusCode::OK);
    let mut st = kit_state(&t, &code).await;
    st["copied"]["dev"] = json!((Utc::now() - Duration::hours(47) - Duration::minutes(59)).to_rfc3339());
    save_kit_state(&t, &code, &st, &fay_id).await;
    let r = host_act(&t, &code, "copied", json!({ "to": "DEV" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
    let msg = r.body["message"].as_str().unwrap().to_string();
    let d = host_view(&t, &code).await["drafts"].as_array().unwrap().iter().find(|d| d["to"] == "Dev").unwrap().clone();
    let next = d["next_copy_at"].as_str().unwrap().to_string();
    assert!(msg.contains("48 hours") && DateTime::parse_from_rfc3339(&next).is_ok(), "{msg} / {next}");
    assert_eq!(d["can_copy"], false);
    let n = DateTime::parse_from_rfc3339(&next).unwrap().with_timezone(&Utc);
    assert!(n > Utc::now() && n < Utc::now() + Duration::minutes(2), "{n}");
    // A joiner who is also on the list is one person (the name key), not two drafts
    let h = host_view(&t, &code).await;
    assert_eq!(h["drafts"].as_array().unwrap().iter().filter(|d| d["to"] == "Eli").count(), 1);
    // Someone who answers leaves not_answered and gets no draft for it
    let (_, dev) = joined(&t, &code, "Dev").await;
    ok(&t, &code, &dev, "answer", json!({ "answer": "yes" })).await;
    let h = host_view(&t, &code).await;
    assert_eq!(h["not_answered"], json!(["Eli"]));
    assert!(h["drafts"].as_array().unwrap().iter().all(|d| d["kind"] != "not_answered" || d["to"] == "Eli"));
}

// 6. Timers and retention for a vote with no event and no ballots
#[tokio::test]
async fn qa6_a_vote_nobody_answers_closes_unwatched_and_goes_7_days_after_the_close() {
    let t = setup().await;
    let now = Utc::now();
    let mut s = spec(now, "vote", json!({}));
    s.as_object_mut().unwrap().remove("event_at");
    let (code, _) = make(&t, s).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    pocket::sweep(&t.state, now + Duration::days(5) - Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["phase"], "open");
    pocket::sweep(&t.state, now + Duration::days(5) + Duration::minutes(1)).await;
    let st = kit_state(&t, &code).await;
    assert_eq!((st["phase"].as_str(), st["done"].as_bool()), (Some("closed"), Some(true)));
    let v = view_of(&t, &code, &a).await;
    assert_eq!(v["result"]["outcome"], "no_votes", "{}", v["result"]);
    let s = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body;
    assert_eq!(s["summary"]["summary"], "Nobody voted", "{}", s["summary"]);
    // A second sweep changes nothing
    let ver = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body["host_view"].clone();
    pocket::sweep(&t.state, now + Duration::days(5) + Duration::minutes(3)).await;
    let ver2 = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body["host_view"].clone();
    assert_eq!(steady(ver), steady(ver2));
    // Rejoin still works after the close; a new name can't join
    assert_eq!(join(&t, &code, "Ben").await.status, StatusCode::CONFLICT);
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(json!({ "name": "Ana", "pin": PIN })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // 7 days after the close, not before
    assert_eq!(pocket::sweep(&t.state, now + Duration::days(12) - Duration::minutes(1)).await.deleted, 0);
    assert_eq!(pocket::sweep(&t.state, now + Duration::days(12) + Duration::minutes(1)).await.deleted, 1);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0]);
}

// 7. Spec refusals in plain sentences, and the edges that must be accepted
#[tokio::test]
async fn qa7_bad_specs_get_plain_sentences_and_the_edges_are_accepted() {
    let t = setup().await;
    let now = Utc::now();
    let many: Vec<Value> = (0..11).map(|i| json!(format!("Option {i}"))).collect();
    let members61: Vec<Value> = (0..61).map(|i| json!(format!("Member {i}"))).collect();
    let bad: Vec<(&str, Value, &str)> = vec![
        ("rsvp", json!({ "colour": "red" }), "colour"),
        ("rsvp", json!({ "fields": ["rating"] }), "rating"),
        ("rsvp", json!({ "title": null }), "title"),
        ("rsvp", json!({ "title": "x".repeat(81) }), "title"),
        ("rsvp", json!({ "when": "x".repeat(61) }), "60"),
        ("rsvp", json!({ "when": "2026-12-12T19:30:00+01:00" }), "event_at"),
        ("rsvp", json!({ "closes_at": "next friday" }), "closes_at"),
        ("rsvp", json!({ "closes_at": null }), "closes_at"),
        ("rsvp", json!({ "show_names": "yes" }), "true or false"),
        ("rsvp", json!({ "templates": { "shout": "hi" } }), "shout"),
        ("rsvp", json!({ "templates": { "maybe": "x".repeat(301) } }), "300"),
        ("rsvp", json!({ "members": members61 }), "60"),
        ("rsvp", json!({ "slots": [{ "name": "A", "cap": 1 }] }), "sign-up"),
        ("rsvp", json!({ "ballot": { "kind": "ranked", "options": ["A", "B"] } }), "vote"),
        ("signup", json!({ "price": "€5" }), "price is for an RSVP"),
        ("signup", json!({ "slots": [] }), "slots"),
        ("signup", json!({ "slots": [{ "name": "A", "cap": 0 }] }), "cap"),
        ("signup", json!({ "slots": [{ "name": "A", "cap": 61 }] }), "cap"),
        ("signup", json!({ "slots": [{ "name": "A", "cap": 1 }, { "name": "a", "cap": 1 }] }), "called"),
        ("vote", json!({ "ballot": { "kind": "ranked", "options": ["A"] } }), "options"),
        ("vote", json!({ "ballot": { "kind": "ranked", "options": many } }), "options"),
        ("vote", json!({ "ballot": { "kind": "ranked", "options": ["A", "a"] } }), "twice"),
        ("vote", json!({ "ballot": { "kind": "stars", "options": ["A", "B"] } }), "one_pick or ranked"),
        ("vote", json!({ "fields": ["plus_ones"] }), "plus_ones"),
    ];
    for (face, extra, words) in bad {
        let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "gather", "spec": spec(now, face, extra.clone()) })), &person()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{face} {extra}: {}", r.body);
        let msg = r.body["message"].as_str().unwrap_or_else(|| panic!("no message: {}", r.body)).to_string();
        assert!(msg.contains(words), "{face} {extra}: {msg}");
        for ugly in ["serde", "expected", "invalid type", "Some(", "unwrap", "panicked", "{\""] {
            assert!(!msg.contains(ugly), "{face} {extra}: not a plain sentence: {msg}");
        }
    }
    // A PIN is required: none, or a short one
    let (code, _) = make(&t, spec(now, "rsvp", json!({}))).await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": "Ana" })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.body["error"], "pin_required");
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": "Ana", "pin": "12" })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_ne!(r.status, StatusCode::CREATED, "{}", r.body);
    // Accepted: a plain date, event_at equal to closes_at, closing in a minute, 20 slots, 10 options, 60 members
    let day = (now + Duration::days(9)).format("%Y-%m-%d").to_string();
    let slots: Vec<Value> = (0..20).map(|i| json!({ "name": format!("Slot {i}"), "cap": 60 })).collect();
    let opts: Vec<Value> = (0..10).map(|i| json!(format!("Option {i}"))).collect();
    let members60: Vec<Value> = (0..60).map(|i| json!(format!("Member {i}"))).collect();
    let good: Vec<(&str, Value)> = vec![
        ("rsvp", json!({ "closes_at": day, "event_at": day })),
        ("rsvp", json!({ "closes_at": (now + Duration::minutes(1)).to_rfc3339(), "event_at": (now + Duration::minutes(1)).to_rfc3339() })),
        ("rsvp", json!({ "when": "2026-12-12", "members": members60 })),
        ("signup", json!({ "slots": slots })),
        ("vote", json!({ "ballot": { "kind": "one_pick", "options": opts } })),
    ];
    for (face, extra) in good {
        let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "gather", "spec": spec(now, face, extra.clone()) })), &person()).await;
        assert_eq!(r.status, StatusCode::CREATED, "{face}: {}", r.body);
    }
}
