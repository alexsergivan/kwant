//! pa10b gather kit server (Sign-up, RSVP, Vote): the secret ballot on every route, the
//! host-only RSVP data, the waiting list, the Chaser's 48-hour limit, the timers and retention
//! on a fake clock, and the PIN. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa10b/server-brief.md.
//! The count's own cases (the design's sample, exhausted ballots, ties) are unit tests in
//! `src/pocket/gather.rs`.

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
        ("person", &t.main, "POST", m("/action"), Some(json!({ "name": "pick", "body": { "option": 0 } })), person()),
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

// ---- Secret ballot ----

/// Nobody (not the host, not another voter, not a stranger) can tell who voted for what, on any
/// route, while voting is open and after the count. Shown by swapping two other voters'
/// ballots in the stored state: every answer to the person, the host remote, guest A and a
/// stranger stays byte-identical, so none of them pairs a person with a ballot. A's own ballot
/// reaches A only. The event log has no vote rows and names nobody.
#[tokio::test]
async fn no_route_pairs_a_person_with_a_ballot() {
    let t = setup().await;
    let now = Utc::now();
    let (code, host_key) = make(&t, spec(now, "vote", json!({ "members": ["Ana", "Ben", "Cleo", "Dev", "Eli", "Fay"] }))).await;
    let names = ["Ana", "Ben", "Cleo", "Dev", "Eli"];
    let mut p = Vec::new();
    for n in names {
        p.push(joined(&t, &code, n).await);
    }
    // Distinct orders; Ben and Cleo differ in their 1st choice, so a leak of either shows
    let ballots = [json!([2, 0, 1]), json!([0, 1]), json!([1, 0, 3]), json!([0, 2]), json!([3, 1])];
    for ((_, k), b) in p.iter().zip(ballots.iter()) {
        ok(&t, &code, k, "vote", json!({ "ranking": b })).await;
    }
    let a_key = p[0].1.clone();
    // A sees their own ballot and the live 1st-choice bars (they voted)
    let a = view_of(&t, &code, &a_key).await;
    assert_eq!(a["me"], json!({ "voted": true, "ballot": [2, 0, 1] }));
    assert_eq!(a["live"].as_array().unwrap().len(), 4);
    // The host: who voted (to nudge), totals, never an order
    let h = host_view(&t, &code).await;
    assert_eq!(h["voters"].as_array().unwrap().len(), 5);
    assert_eq!(h["not_answered"], json!(["Fay"]));
    for row in h["voters"].as_array().unwrap() {
        let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["at", "avatar", "id", "name"], "{row}");
    }

    // Open: swap Ben's and Cleo's ballots
    let before = every_route(&t, &code, &host_key, &a_key).await;
    for (route, status, body) in &before {
        let text = body.to_string();
        assert!(!text.contains("ballot_salt") && !text.contains("\"ballots\""), "{route} ({status}): {text}");
        if !route.starts_with("guest A") || route.contains("as host") {
            assert!(!text.contains("[2,0,1]"), "{route} ({status}) shows A's ballot: {text}");
        }
    }
    swap_ballots(&t, &code, &p[1].0, &p[2].0, &p[0].0).await;
    same(&before, &every_route(&t, &code, &host_key, &a_key).await, "open");

    // Closed and counted: the same, with the result and its rounds on every view
    let r = host_act(&t, &code, "close_now", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let result = view_of(&t, &code, &a_key).await["result"].clone();
    assert_eq!(result["outcome"], "winner", "{result}");
    assert!(!result["rounds"].as_array().unwrap().is_empty());
    assert_eq!(result["ballots_counted"], 5, "a count, named so no view carries a \"ballots\" key");
    let before = every_route(&t, &code, &host_key, &a_key).await;
    for (route, status, body) in &before {
        let text = body.to_string();
        assert!(!text.contains("ballot_salt") && !text.contains("\"ballots\""), "{route} ({status}): {text}");
    }
    swap_ballots(&t, &code, &p[1].0, &p[2].0, &p[0].0).await;
    same(&before, &every_route(&t, &code, &host_key, &a_key).await, "closed");

    // The event log: no row per vote, and no row names a person
    let events = t.state.db.pocket_events(&code).await.unwrap();
    assert!(events.iter().all(|(_, kind, _)| !kind.contains("vote")), "{events:?}");
    for (player, kind, payload) in &events {
        assert!(player.is_none(), "{kind} names a player");
        for n in names {
            assert!(!payload.contains(n), "{kind} {payload}");
        }
        for (id, _) in &p {
            assert!(!payload.contains(id.as_str()), "{kind} {payload}");
        }
    }
}

#[tokio::test]
async fn a_ranked_vote_counts_in_rounds_and_a_tie_waits_for_the_host() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "vote", json!({ "show_live_count": false }))).await;
    // The design's sample: 6/5/3/1 → 6/6/3 → The Old Mill 8 to 7
    let groups: [(&[usize], usize); 5] = [(&[0, 1], 6), (&[1, 0], 5), (&[2, 0], 2), (&[2, 1], 1), (&[3, 1], 1)];
    let mut keys = Vec::new();
    for (ranking, n) in groups {
        for _ in 0..n {
            let (_, k) = joined(&t, &code, &format!("Voter {}", keys.len() + 1)).await;
            ok(&t, &code, &k, "vote", json!({ "ranking": ranking })).await;
            keys.push(k);
        }
    }
    assert_eq!(view_of(&t, &code, &keys[0]).await["live"], Value::Null, "show_live_count off");
    // A wrong ballot is refused in plain words
    assert_eq!(act(&t, &code, &keys[0], "vote", json!({ "ranking": [0, 0] })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(act(&t, &code, &keys[0], "vote", json!({ "ranking": [9] })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(act(&t, &code, &keys[0], "answer", json!({ "answer": "yes" })).await.status, StatusCode::BAD_REQUEST, "the wrong face");
    // Answers close on time though nobody watches
    pocket::sweep(&t.state, now + Duration::days(5) + Duration::minutes(1)).await;
    let v = view_of(&t, &code, &keys[0]).await;
    assert_eq!(v["phase"], "closed");
    let r = &v["result"];
    assert_eq!((r["outcome"].as_str(), r["winner"]["name"].as_str(), r["winner"]["votes"].as_u64()), (Some("winner"), Some("The Old Mill"), Some(8)));
    let rounds: Vec<Vec<u64>> = r["rounds"].as_array().unwrap().iter().map(|x| x["counts"].as_array().unwrap().iter().map(|c| c["votes"].as_u64().unwrap()).collect()).collect();
    assert_eq!(rounds, vec![vec![6, 5, 3, 1], vec![6, 6, 3], vec![8, 7]]);
    assert_eq!(r["rounds"][0]["dropped"][0]["name"], "Pizza Uno");
    assert_eq!(act(&t, &code, &keys[0], "vote", json!({ "ranking": [1] })).await.status, StatusCode::CONFLICT, "closed");
    assert_eq!(kit_state(&t, &code).await["done"], true, "the core ends a decided vote and posts the result");
    let s = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await.body;
    assert_eq!(s["summary"]["summary"], "The Old Mill wins, 8 to 7 in the last round");

    // A 2-2 one-pick vote: a final tie the host picks from (only a tied option)
    let (code, _) = make(&t, spec(now, "vote", json!({ "ballot": { "kind": "one_pick", "options": ["Fri", "Sat", "Sun"] } }))).await;
    let mut ids = Vec::new();
    for (n, pick) in [("Ana", 0), ("Ben", 0), ("Cleo", 1), ("Dev", 1)] {
        let (id, k) = joined(&t, &code, n).await;
        assert_eq!(act(&t, &code, &k, "vote", json!({ "ranking": [pick, 2] })).await.status, StatusCode::BAD_REQUEST, "one pick");
        ok(&t, &code, &k, "vote", json!({ "ranking": [pick] })).await;
        ids.push((id, k));
    }
    assert_eq!(host_act(&t, &code, "close_now", json!({})).await.status, StatusCode::OK);
    let h = host_view(&t, &code).await;
    assert_eq!((h["phase"].as_str(), h["needs_you"].as_bool()), (Some("tie"), Some(true)));
    assert_eq!(h["result"]["tied"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Fri", "Sat"]);
    assert_eq!(kit_state(&t, &code).await["done"], false, "a tie isn't final");
    // Vote again for 24 h among the tied: ballots cleared, only Fri and Sat in play
    assert_eq!(host_act(&t, &code, "revote_24h", json!({})).await.status, StatusCode::OK);
    let v = view_of(&t, &code, &ids[0].1).await;
    assert_eq!((v["phase"].as_str(), v["me"]["voted"].as_bool(), v["ballot"]["in_play"].clone()), (Some("open"), Some(false), json!([0, 1])));
    assert_eq!(act(&t, &code, &ids[0].1, "vote", json!({ "ranking": [2] })).await.status, StatusCode::BAD_REQUEST, "Sun is out");
    for (i, (_, k)) in ids.iter().enumerate() {
        ok(&t, &code, k, "vote", json!({ "ranking": [i % 2] })).await;
    }
    pocket::sweep(&t.state, Utc::now() + Duration::hours(24) + Duration::minutes(1)).await;
    assert_eq!(host_view(&t, &code).await["phase"], "tie");
    assert_eq!(host_act(&t, &code, "pick", json!({ "option": 2 })).await.status, StatusCode::BAD_REQUEST, "not a tied option");
    assert_eq!(host_act(&t, &code, "pick", json!({ "option": 1 })).await.status, StatusCode::OK);
    let h = host_view(&t, &code).await;
    assert_eq!((h["phase"].as_str(), h["result"]["outcome"].as_str(), h["result"]["winner"]["name"].as_str()), (Some("closed"), Some("picked"), Some("Sat")));
    assert_eq!(kit_state(&t, &code).await["done"], true);
}

// ---- RSVP ----

#[tokio::test]
async fn paid_ticks_food_and_plus_ones_are_the_hosts_and_show_names_off_leaves_counts() {
    let t = setup().await;
    let now = Utc::now();
    let extra = json!({ "fields": ["plus_ones", "food_needs"], "price": "€35", "members": ["Ana", "Ben", "Cleo", "Dev"] });
    let (code, host_key) = make(&t, spec(now, "rsvp", extra.clone())).await;
    let (ana, ana_k) = joined(&t, &code, "Ana").await;
    let (ben, ben_k) = joined(&t, &code, "Ben").await;
    let (_, cleo_k) = joined(&t, &code, "Cleo").await;
    ok(&t, &code, &ana_k, "answer", json!({ "answer": "yes", "plus_ones": 2, "food": ["vegan"], "food_other": "CANARYFOOD no mushrooms" })).await;
    ok(&t, &code, &ben_k, "answer", json!({ "answer": "yes" })).await;
    ok(&t, &code, &cleo_k, "answer", json!({ "answer": "maybe" })).await;
    assert_eq!(act(&t, &code, &cleo_k, "answer", json!({ "answer": "yes", "plus_ones": 9 })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(act(&t, &code, &cleo_k, "answer", json!({ "answer": "yes", "food": ["steak"] })).await.status, StatusCode::BAD_REQUEST);
    // The host ticks Ana paid; Cleo (a Maybe) can't be ticked
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": ana })).await.status, StatusCode::OK);
    let cleo = kit_state(&t, &code).await["answers"].as_object().unwrap().iter().find(|(_, a)| a["answer"] == "maybe").map(|(id, _)| id.clone()).unwrap();
    assert_eq!(host_act(&t, &code, "set_paid", json!({ "player_id": cleo })).await.status, StatusCode::CONFLICT);
    assert_eq!(act(&t, &code, &ben_k, "set_paid", json!({ "player_id": ben })).await.status, StatusCode::BAD_REQUEST, "a host action from a guest");

    let h = host_view(&t, &code).await;
    assert_eq!(h["counts"], json!({ "yes": 2, "maybe": 1, "no": 0, "plus_ones": 2, "coming": 4 }));
    assert_eq!(h["paid"]["ticked"], 1);
    assert_eq!(h["paid"]["not_yet"], json!(["Ben"]));
    assert_eq!(h["food"]["counts"]["vegan"], 1);
    assert_eq!(h["food"]["other"], json!(["CANARYFOOD no mushrooms"]));
    assert_eq!(h["not_answered"], json!(["Dev"]));
    let kinds: Vec<(&str, &str)> = h["drafts"].as_array().unwrap().iter().map(|d| (d["to"].as_str().unwrap(), d["kind"].as_str().unwrap())).collect();
    assert_eq!(kinds, [("Ben", "not_paid"), ("Cleo", "maybe"), ("Dev", "not_answered")]);
    let ana_row = h["people"].as_array().unwrap().iter().find(|p| p["name"] == "Ana").unwrap().clone();
    assert_eq!((ana_row["plus_ones"].as_u64(), ana_row["paid_at"].is_string()), (Some(2), true));

    // Guests: names and Yes/Maybe/No, counts, their own record; nobody else's food, plus-ones or tick
    let b = view_of(&t, &code, &ben_k).await;
    let text = b.to_string();
    assert!(!text.contains("CANARYFOOD") && !text.contains("[\"vegan\"]") && !text.contains("paid_at") && !text.contains("plus_ones\":2"), "{text}");
    assert_eq!(b["me"]["paid"], Value::Null, "Ben isn't ticked, so his page says nothing about paying");
    assert_eq!(b["counts"], json!({ "yes": 2, "maybe": 1, "no": 0, "coming": 4 }));
    let people: Vec<(&str, &str)> = b["people"].as_array().unwrap().iter().map(|p| (p["name"].as_str().unwrap(), p["answer"].as_str().unwrap())).collect();
    assert_eq!(people, [("Ana", "yes"), ("Ben", "yes"), ("Cleo", "maybe")]);
    for row in b["people"].as_array().unwrap() {
        let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["answer", "avatar", "id", "name"], "{row}");
    }
    let a = view_of(&t, &code, &ana_k).await;
    assert_eq!(a["me"]["food_other"], "CANARYFOOD no mushrooms");
    assert!(a["me"]["paid"]["at"].is_string(), "Ana sees her own tick");
    // Strangers and the lobby: no names, no answers
    for path in [format!("/api/pocket/{code}/info"), format!("/p/{code}")] {
        let r = send(&t.guest, "GET", &path, None, &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
        let text = r.body.to_string();
        assert!(!text.contains("CANARYFOOD") && !text.contains("Ana") && !text.contains("[\"vegan\"]"), "{path}: {text}");
    }
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(&host_key)).await;
    assert!(r.body.to_string().contains("CANARYFOOD"), "the host remote has it");

    // show_names off: guests get counts only
    let mut extra = extra;
    extra["show_names"] = json!(false);
    let (code, _) = make(&t, spec(now, "rsvp", extra)).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let (_, b) = joined(&t, &code, "Ben").await;
    ok(&t, &code, &a, "answer", json!({ "answer": "no" })).await;
    ok(&t, &code, &b, "answer", json!({ "answer": "yes" })).await;
    let v = view_of(&t, &code, &b).await;
    assert_eq!(v["people"], Value::Null);
    assert_eq!(v["counts"]["no"], 1);
    assert!(!v.to_string().contains("Ana"), "{v}");
}

// ---- Sign-up ----

#[tokio::test]
async fn a_full_slot_keeps_a_waiting_list_and_the_first_moves_in_at_once() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "signup", json!({ "fields": ["slot_note"] }))).await;
    let (_, ana) = joined(&t, &code, "Ana").await;
    let (ben_id, ben) = joined(&t, &code, "Ben").await;
    let (_, cleo) = joined(&t, &code, "Cleo").await;
    ok(&t, &code, &ana, "take", json!({ "slot": 0, "note": "Tiramisu" })).await;
    ok(&t, &code, &ben, "take", json!({ "slot": 0 })).await;
    ok(&t, &code, &cleo, "take", json!({ "slot": 0 })).await;
    let v = view_of(&t, &code, &cleo).await;
    assert_eq!(v["me"]["waiting"], json!({ "slot": 0, "position": 2 }));
    assert_eq!((v["slots"][0]["filled"].as_u64(), v["slots"][0]["waiting"].as_u64()), (Some(1), Some(2)), "the cap is never passed");
    assert_eq!(v["slots"][0]["people"][0]["note"], "Tiramisu");
    // Ana lets go: Ben (first in line) moves in at once, Cleo moves up the line
    ok(&t, &code, &ana, "let_go", json!({})).await;
    let b = view_of(&t, &code, &ben).await;
    assert_eq!((b["me"]["slot"].as_u64(), b["me"]["moved_up"]["slot"].as_u64(), b["me"]["moved_up"]["new"].as_bool()), (Some(0), Some(0), Some(true)));
    assert_eq!(view_of(&t, &code, &cleo).await["me"]["waiting"]["position"], 1);
    let h = host_view(&t, &code).await;
    assert_eq!(h["slots"][0]["people"][0]["id"], json!(ben_id));
    let d = &h["drafts"][0];
    assert_eq!((d["to"].as_str(), d["kind"].as_str()), (Some("Ben"), Some("moved_up")));
    assert!(d["text"].as_str().unwrap().contains("Desserts") && d["text"].as_str().unwrap().contains("{link}"), "{d}");
    ok(&t, &code, &ben, "seen", json!({})).await;
    assert_eq!(view_of(&t, &code, &ben).await["me"]["moved_up"]["new"], false);
    // Moving to another slot frees this one too
    ok(&t, &code, &ben, "take", json!({ "slot": 1 })).await;
    assert_eq!(view_of(&t, &code, &cleo).await["me"]["slot"], 0);
    let events = t.state.db.pocket_events(&code).await.unwrap();
    assert!(events.iter().filter(|(_, k, _)| k == "moved_up").all(|(p, _, payload)| p.is_none() && !payload.contains("pl_")), "{events:?}");

    // Without a waiting list a full slot says so
    let (code, _) = make(&t, spec(now, "signup", json!({ "waiting_list": false }))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let (_, b) = joined(&t, &code, "Ben").await;
    ok(&t, &code, &a, "take", json!({ "slot": 0 })).await;
    let r = act(&t, &code, &b, "take", json!({ "slot": 0 })).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["message"], "Desserts is full.");
    assert_eq!(act(&t, &code, &b, "take", json!({ "slot": 0, "note": "x" })).await.status, StatusCode::BAD_REQUEST, "no notes on this sign-up");
}

// ---- The Chaser ----

#[tokio::test]
async fn one_nudge_per_person_every_48_hours_and_nothing_is_sent() {
    let t = setup().await;
    let now = Utc::now();
    let (code, _) = make(&t, spec(now, "rsvp", json!({ "members": ["Ana", "Ben"], "templates": { "not_answered": "Hey {name}, {title} at {place}? {link}" } }))).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    ok(&t, &code, &a, "answer", json!({ "answer": "maybe" })).await;
    let h = host_view(&t, &code).await;
    let ben = h["drafts"].as_array().unwrap().iter().find(|d| d["to"] == "Ben").unwrap().clone();
    assert_eq!(ben["text"], "Hey Ben, Club dinner at The Old Mill? {link}");
    assert_eq!((ben["can_copy"].as_bool(), ben["player_id"].clone()), (Some(true), Value::Null), "Ben hasn't joined");
    assert_eq!(host_act(&t, &code, "copied", json!({ "to": "ben" })).await.status, StatusCode::OK, "name keys match");
    let r = host_act(&t, &code, "copied", json!({ "to": "Ben" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.body["message"].as_str().unwrap().contains("48 hours"), "{}", r.body);
    let ben = host_view(&t, &code).await["drafts"].as_array().unwrap().iter().find(|d| d["to"] == "Ben").unwrap().clone();
    assert_eq!(ben["can_copy"], false);
    let next = DateTime::parse_from_rfc3339(ben["next_copy_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    assert!(next > now + Duration::hours(47) && next < Utc::now() + Duration::hours(48) + Duration::seconds(1), "{next}");
    // Another person isn't held up
    assert_eq!(host_act(&t, &code, "copied", json!({ "to": "Ana" })).await.status, StatusCode::OK);
    // 48 hours later Ben's draft can be copied again
    let mut st = kit_state(&t, &code).await;
    st["copied"]["ben"] = json!((Utc::now() - Duration::hours(48) - Duration::minutes(1)).to_rfc3339());
    save_kit_state(&t, &code, &st, &kit_state(&t, &code).await["answers"].as_object().unwrap().keys().next().unwrap().clone()).await;
    assert_eq!(host_act(&t, &code, "copied", json!({ "to": "Ben" })).await.status, StatusCode::OK);
    // No note for someone with nothing to nudge
    assert_eq!(host_act(&t, &code, "copied", json!({ "to": "Zed" })).await.status, StatusCode::CONFLICT);
    // Nothing went out: no outbox, only counts in the event log
    for (p, kind, payload) in t.state.db.pocket_events(&code).await.unwrap() {
        assert!(p.is_none() && !payload.contains("Ben") && !payload.contains("Hey"), "{kind} {payload}");
    }
}

// ---- Times and retention ----

#[tokio::test]
async fn answers_close_unwatched_and_everything_goes_7_days_after_the_event() {
    let t = setup().await;
    let now = Utc::now();
    let s = spec(now, "rsvp", json!({}));
    let event: DateTime<Utc> = DateTime::parse_from_rfc3339(s["event_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    let close = event - Duration::days(2);
    let (code, _) = make(&t, s).await;
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &person()).await;
    let expires = r.body["pocket"]["expires_at"].as_str().map(|s| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc));
    assert_eq!(expires.map(|e| (e - (event + Duration::days(7))).num_seconds().abs() < 2), Some(true), "{}", r.body["pocket"]);
    host_view_matches_pocket(&r.body);
    let (_, a) = joined(&t, &code, "Ana").await;
    ok(&t, &code, &a, "answer", json!({ "answer": "yes" })).await;
    pocket::sweep(&t.state, close - Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["phase"], "open");
    pocket::sweep(&t.state, close + Duration::minutes(1)).await;
    let st = kit_state(&t, &code).await;
    assert_eq!((st["phase"].as_str(), st["done"].as_bool()), (Some("closed"), Some(false)), "paid ticks still work until the day");
    // Closed: no new joins, but Ana opens her page again with her name and PIN
    assert_eq!(join(&t, &code, "Ben").await.status, StatusCode::CONFLICT);
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(json!({ "name": "Ana", "pin": PIN })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    pocket::sweep(&t.state, event + Duration::minutes(1)).await;
    assert_eq!(kit_state(&t, &code).await["done"], true, "final once the event has passed");
    assert!(t.state.db.pocket_row_counts(&code).await.unwrap().iter().any(|n| *n > 0));
    assert_eq!(pocket::sweep(&t.state, event + Duration::days(7) - Duration::minutes(1)).await.deleted, 0, "not at +6 days 23:59");
    assert_eq!(pocket::sweep(&t.state, event + Duration::days(7) + Duration::minutes(1)).await.deleted, 1);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0], "players, keys and events all gone");
    // Without event_at it goes 7 days after the close
    let mut s = spec(now, "vote", json!({}));
    s.as_object_mut().unwrap().remove("event_at");
    let (code2, _) = make(&t, s).await;
    let r = send(&t.main, "GET", &format!("/api/pockets/{code2}"), None, &person()).await;
    let expires = DateTime::parse_from_rfc3339(r.body["pocket"]["expires_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    assert!((expires - (now + Duration::days(12))).num_seconds().abs() < 2);
    host_view_matches_pocket(&r.body);
}

/// The host view carries the pocket's title and deletion date, so the chat card copies it as is.
/// Checked on a live pocket's `GET /api/pockets/:code` body (a deleted one is a 404 with neither).
fn host_view_matches_pocket(body: &Value) {
    let (p, h) = (&body["pocket"], &body["host_view"]);
    assert!(p["title"].is_string(), "no live pocket: {body}");
    assert_eq!(h["title"], p["title"], "{h}");
    let at = |v: &Value| DateTime::parse_from_rfc3339(v.as_str().unwrap_or_else(|| panic!("{v}"))).unwrap();
    assert!((at(&h["expires_at"]) - at(&p["expires_at"])).num_seconds().abs() < 1, "{} vs {}", h["expires_at"], p["expires_at"]);
}

#[tokio::test]
async fn specs_the_kit_cant_run_are_refused_and_a_pin_is_required() {
    let t = setup().await;
    let now = Utc::now();
    for (extra, words) in [
        (json!({ "when": (now + Duration::days(7)).to_rfc3339() }), "event_at"),
        (json!({ "closes_at": (now - Duration::hours(1)).to_rfc3339() }), "already past"),
        (json!({ "event_at": (now + Duration::days(1)).to_rfc3339() }), "before closes_at"),
        (json!({ "face": "poll" }), "face must be"),
        (json!({ "price": "€5", "face": "vote", "ballot": { "kind": "ranked", "options": ["A", "B"] } }), "price is for an RSVP"),
    ] {
        let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "gather", "spec": spec(now, "rsvp", extra.clone()) })), &person()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{extra}: {}", r.body);
        assert!(r.body.to_string().contains(words), "{extra}: {}", r.body);
    }
    let (code, _) = make(&t, spec(now, "rsvp", json!({}))).await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": "Ana" })), &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.body["error"], "pin_required");
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), None, &Req { ip: Some(fresh_ip()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK);
    let text = r.body.to_string();
    assert!(text.contains("\"after_date\"") && text.contains("\"days\":7"), "{text}");
}
