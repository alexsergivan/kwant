//! pa12 L4 (autonomy/pocket-apps/pa12/L4/brief.md, plan.md section 2): the made kit's server
//! actions. The key test is a full dry run over HTTP and the WebSocket in which every view each
//! viewer gets (four guests, the big screen, the host, the person's main-port read, `info` and
//! the page's boot) is serialized at every step and string-searched for every secret
//! (normalised, at least 3 characters) until that secret is revealed; a secret that is never
//! revealed is searched for to the end. The other tests drive the kit directly: a guest-sent
//! `points`/`score`/`answer_key` is refused, the draw keeps `apart` pairs and never gives anyone
//! themselves, a timer locks through `tick`, turns rotate and skip, `reveal_at` flips the
//! reveal, `step` walks `steps[]`, `adjust` is capped, and unknown actions stay `unknown_action`.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use futures::StreamExt;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    pocket::{
        self, guest,
        kits::{Kit, KitCtx, KitError, Player, Viewer},
        listener,
        made::Made,
        toolkit::normalise,
    },
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
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const LOCAL: &str = "127.0.0.1:50000";
const GHOST: &str = "pocket.example.net";

struct T {
    main: Router,
    guest: Router,
    state: AppState,
    _tmp: TempDir,
}

/// `pocket_apps=on`, and `pocket_make` as given.
async fn setup(make: bool) -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    if make {
        state.db.set_setting(pocket::MAKE_SETTING, "on").await.unwrap();
    }
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), guest: guest::router(state.clone()), state, _tmp: tmp }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

#[derive(Default, Clone)]
struct Req<'a> {
    cookie: Option<(String, String)>,
    bearer: Option<&'a str>,
    ip: Option<String>,
    local: bool,
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>, r: &Req<'_>) -> Res {
    let host = if r.local { "127.0.0.1:8799" } else { GHOST };
    let mut req = Request::builder().method(method).uri(uri).header("host", host).header("content-type", "application/json");
    if let Some((k, v)) = &r.cookie {
        req = req.header("cookie", format!("{k}={v}"));
    }
    if let Some(b) = r.bearer {
        req = req.header("authorization", format!("Bearer {b}"));
    }
    if let Some(ip) = &r.ip {
        req = req.header("x-forwarded-for", ip.as_str());
    }
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    if r.local {
        req.extensions_mut().insert(ConnectInfo(LOCAL.parse::<SocketAddr>().unwrap()));
    }
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Res { status, headers, body: serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())) }
}

fn local() -> Req<'static> {
    Req { local: true, ..Default::default() }
}

fn guest_req(code: &str, key: &str) -> Req<'static> {
    Req { cookie: Some((guest::guest_cookie(code), key.to_string())), ..Default::default() }
}

fn host_req(key: &str) -> Req<'_> {
    Req { bearer: Some(key), ..Default::default() }
}

async fn make(t: &T, spec: Value) -> Res {
    send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "made", "spec": spec })), &local()).await
}

/// A made pocket: (code, host key).
async fn made(t: &T, spec: Value) -> (String, String) {
    let r = make(t, spec).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (r.body["pocket"]["code"].as_str().unwrap().to_string(), r.body["host_key"].as_str().unwrap().to_string())
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// (player id, guest key); each guest from its own address.
async fn joined(t: &T, code: &str, name: &str) -> (String, String) {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let ip = format!("198.51.{}.{}", (n >> 8) & 255, n & 255);
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": name })), &Req { ip: Some(ip), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let cookie = guest::guest_cookie(code);
    let key = r.headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find_map(|c| c.split(';').next()?.strip_prefix(&format!("{cookie}=")).map(str::to_string));
    (r.body["me"]["id"].as_str().unwrap().to_string(), key.unwrap())
}

async fn put(t: &T, code: &str, key: &str, coll: &str, values: Value) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "put", "body": { "collection": coll, "values": values } })), &guest_req(code, key)).await
}




type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_connect(addr: SocketAddr, code: &str, cookie: Option<(String, String)>, bearer: Option<&str>, view: Option<&str>) -> Ws {
    let q = view.map(|v| format!("?view={v}")).unwrap_or_default();
    let mut req = format!("ws://{addr}/api/pocket/{code}/ws{q}").into_client_request().unwrap();
    req.headers_mut().insert("origin", format!("http://{addr}").parse().unwrap());
    if let Some((k, v)) = cookie {
        req.headers_mut().insert("cookie", format!("{k}={v}").parse().unwrap());
    }
    if let Some(b) = bearer {
        req.headers_mut().insert("authorization", format!("Bearer {b}").parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.expect("ws connects").0
}

/// The first state a fresh socket sends (the current one), without its `type`.
async fn first_state(mut ws: Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a message within 5 s").unwrap().unwrap();
        if let Message::Text(t) = msg {
            let mut v: Value = serde_json::from_str(&t).unwrap();
            if let Some(o) = v.as_object_mut() {
                o.remove("type");
            }
            return v;
        }
    }
}


/// The boot JSON in the page, when this build has the page (else `None`: the placeholder page
/// embeds nothing but the title, and is string-searched all the same).
fn boot_of(html: &str) -> Option<Value> {
    let start = html.find(r#"id="pocket-boot">"#)? + r#"id="pocket-boot">"#.len();
    let end = html[start..].find("</script>")? + start;
    serde_json::from_str(&html[start..end]).ok()
}


async fn act(t: &T, code: &str, key: &str, name: &str, body: Value) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": name, "body": body })), &guest_req(code, key)).await
}

async fn host_act(t: &T, code: &str, host: &str, name: &str, body: Value) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": name, "body": body })), &host_req(host)).await
}

/// The secrets of [`spec`]: the answers group (an item with an accepted spelling, and a plain one)
/// and a group no reveal ever names.
const ANSWER_0: &str = "Call me Ishmael";
const ACCEPT_0: &str = "Ishmael-only";
const ANSWER_1: &str = "Whale-road sorrow";
const HIDDEN: &str = "never shown ghost-name";

fn spec() -> Value {
    json!({
        "format": "made/1",
        "title": "Opening lines",
        "retention": "after_end",
        "words": { "intro": "Guess the first line." },
        "collections": {
            "shelf": { "read": "everyone", "write": "guest_many", "max_per_guest": 2, "fields": { "title": { "type": "text", "max": 60 } } },
            "guesses": { "read": "mine", "write": "server", "fields": { "text": { "type": "text", "max": 100 }, "q": { "type": "int", "min": 0, "max": 1 } } },
            "picks": { "read": "after_reveal", "write": "guest_one", "fields": { "text": { "type": "text", "max": 60 } } },
            "notes": { "read": "host", "write": "guest_one", "fields": { "line": { "type": "text", "max": 60 } } }
        },
        "secrets": {
            "answers": [ { "text": ANSWER_0, "accept": [ACCEPT_0] }, ANSWER_1 ],
            "hidden": [ HIDDEN ]
        },
        "actions": {
            "answer": { "collection": "guesses", "against": "secrets.answers", "match": "text", "points": 2 },
            "vote": { "collection": "shelf", "count": "one_pick" },
            "draw": { "apart": [["Ana", "Ben"]] },
            "timer": { "seconds": 30 },
            "turns": { "order": "join" },
            "reveal": { "collections": ["picks"], "secrets": ["answers"] }
        },
        "steps": [ { "say": "Round 1" }, { "open": "guesses", "timer": 60 }, { "lock": "guesses" }, { "reveal": true } ]
    })
}

/// A copy of a view without the viewer's own rows (`"mine": true`): a guest's own guess may be
/// right, and seeing what they typed is no leak.
fn without_own_rows(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().filter(|x| x.get("mine") != Some(&Value::Bool(true))).map(without_own_rows).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), without_own_rows(x))).collect()),
        x => x.clone(),
    }
}

/// Every response each viewer gets right now: (viewer, way, response).
async fn every_view(t: &T, code: &str, host: &str, guests: &[(String, String)]) -> Vec<(String, String, Value)> {
    let addr = t.state.pocket.listener.addr().expect("the guest listener runs");
    let mut out = Vec::new();
    for (i, (_, key)) in guests.iter().enumerate() {
        let who = format!("guest {i}");
        let poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(code, key)).await;
        assert_eq!(poll.status, StatusCode::OK, "{}", poll.body);
        let ws = first_state(ws_connect(addr, code, Some((guest::guest_cookie(code), key.clone())), None, None).await).await;
        assert_eq!(ws["view"]["revealed"], poll.body["view"]["revealed"], "{who}: ws and polling agree");
        let page = send(&t.guest, "GET", &format!("/p/{code}"), None, &guest_req(code, key)).await;
        let html = page.body.as_str().unwrap_or_default().to_string();
        let boot = boot_of(&html).unwrap_or(Value::String(html));
        out.push((who.clone(), "poll".into(), poll.body));
        out.push((who.clone(), "ws".into(), ws));
        out.push((who, "boot".into(), boot));
    }
    let screen = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state?view=screen"), None, &host_req(host)).await;
    assert_eq!(screen.status, StatusCode::OK, "{}", screen.body);
    let screen_ws = first_state(ws_connect(addr, code, None, Some(host), Some("screen")).await).await;
    out.push(("screen".into(), "poll".into(), screen.body));
    out.push(("screen".into(), "ws".into(), screen_ws));
    let hosted = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(host)).await;
    assert_eq!(hosted.status, StatusCode::OK, "{}", hosted.body);
    let host_ws = first_state(ws_connect(addr, code, None, Some(host), Some("host")).await).await;
    let main_read = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &local()).await;
    assert_eq!(main_read.status, StatusCode::OK, "{}", main_read.body);
    let main_list = send(&t.main, "GET", "/api/pockets", None, &local()).await;
    out.push(("host".into(), "poll".into(), hosted.body));
    out.push(("host".into(), "ws".into(), host_ws));
    out.push(("host".into(), "main read".into(), main_read.body));
    out.push(("host".into(), "main list".into(), main_list.body));
    let info = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), None, &Req::default()).await;
    assert_eq!(info.status, StatusCode::OK, "{}", info.body);
    let page = send(&t.guest, "GET", &format!("/p/{code}"), None, &Req::default()).await;
    let html = page.body.as_str().unwrap_or_default().to_string();
    out.push(("nobody".into(), "info".into(), info.body));
    out.push(("nobody".into(), "boot".into(), boot_of(&html).unwrap_or(Value::String(html))));
    out
}

/// The leak search: no response may hold a banned secret, normalised (at least 3 characters).
fn assert_no_secret(step: &str, views: &[(String, String, Value)], banned: &[&str]) {
    let banned: Vec<String> = banned.iter().map(|s| normalise(s)).filter(|s| s.chars().count() >= 3).collect();
    assert!(!banned.is_empty());
    for (who, way, v) in views {
        let text = match without_own_rows(v) {
            Value::String(s) => s,
            v => v.to_string(),
        };
        let hay = normalise(&text);
        for b in &banned {
            assert!(!hay.contains(b.as_str()), "{step}: {who} {way} leaks the secret {b:?}: {text}");
        }
    }
}

fn view_of<'a>(views: &'a [(String, String, Value)], who: &str) -> &'a Value {
    &views.iter().find(|(w, way, _)| w == who && way == "poll").expect("a poll").2["view"]
}

#[tokio::test]
async fn a_full_dry_run_shows_no_secret_to_any_viewer_before_its_reveal() {
    let t = setup(true).await;
    let (code, host) = made(&t, spec()).await;
    let mut guests = Vec::new();
    for name in ["Ana", "Ben", "Cleo", "Dev"] {
        guests.push(joined(&t, &code, name).await);
    }
    let all = [ANSWER_0, ACCEPT_0, ANSWER_1, HIDDEN];
    let mut checkpoints = 0;
    macro_rules! check {
        ($step:expr) => {{
            let v = every_view(&t, &code, &host, &guests).await;
            assert_no_secret($step, &v, &all);
            checkpoints += 1;
            v
        }};
    }
    check!("joined");
    // Step 1: say
    assert_eq!(host_act(&t, &code, &host, "step", json!({})).await.status, StatusCode::OK);
    let v = check!("step 1");
    assert_eq!(view_of(&v, "guest 0")["step"], json!({ "at": 0, "of": 4, "say": "Round 1" }));
    // Guesses before the collection opens are fine (nothing locked it yet); step 2 opens it with a timer
    assert_eq!(host_act(&t, &code, &host, "step", json!({})).await.status, StatusCode::OK);
    let v = check!("step 2");
    let timer = &view_of(&v, "guest 1")["timer"];
    assert_eq!((timer["collection"].as_str(), timer["seconds_left"].as_i64().is_some_and(|s| (55..=60).contains(&s))), (Some("guesses"), true), "{timer}");
    // Answers: Ana right with other punctuation and case, Cleo right on item 1, Ben and Dev wrong
    for (g, q, text) in [(0, 0, "call me, ISHMAEL!"), (1, 0, "moby"), (2, 1, "whale road sorrow"), (3, 0, "ahab"), (1, 1, "the sea")] {
        let r = act(&t, &code, &guests[g].1, "answer", json!({ "q": q, "text": text })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    }
    let v = check!("answered");
    // Before the reveal nobody's score counts a guess, and the guesser sees only their own rows
    assert_eq!(view_of(&v, "guest 0")["scores"], json!([{ "id": guests[0].0, "points": 0 }, { "id": guests[1].0, "points": 0 }, { "id": guests[2].0, "points": 0 }, { "id": guests[3].0, "points": 0 }]));
    assert_eq!(view_of(&v, "guest 1")["collections"]["guesses"]["count"], 2);
    assert_eq!(view_of(&v, "host")["collections"]["guesses"], json!({ "count": 5, "locked": false }));
    assert_eq!(view_of(&v, "guest 0")["revealed"], json!({}));
    // Shelf and a vote, picks, notes
    for (i, (_, key)) in guests.iter().enumerate() {
        assert_eq!(put(&t, &code, key, "shelf", json!({ "title": format!("Book {i}") })).await.status, StatusCode::OK);
        assert_eq!(put(&t, &code, key, "picks", json!({ "text": format!("pick {i}") })).await.status, StatusCode::OK);
        assert_eq!(put(&t, &code, key, "notes", json!({ "line": format!("note {i}") })).await.status, StatusCode::OK);
    }
    let shelf: Vec<String> = view_of(&check!("shelf"), "guest 0")["collections"]["shelf"]["rows"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    for (g, pick) in [(0, 1), (1, 1), (2, 3), (3, 1)] {
        let r = act(&t, &code, &guests[g].1, "vote", json!({ "pick": shelf[pick] })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    }
    let v = check!("voted");
    let vote = &view_of(&v, "guest 2")["vote"];
    assert_eq!((&vote["mine"], &vote["voters"], &vote["tally"]["outcome"]), (&json!([shelf[3]]), &json!(4), &json!({ "kind": "winner", "id": shelf[1] })));
    assert_eq!(view_of(&v, "host")["vote"]["mine"], Value::Null, "the host sees no ballot");
    // The draw, a turn and an adjustment
    assert_eq!(host_act(&t, &code, &host, "draw", json!({})).await.status, StatusCode::OK);
    assert_eq!(host_act(&t, &code, &host, "turn_next", json!({})).await.status, StatusCode::OK);
    assert_eq!(host_act(&t, &code, &host, "adjust", json!({ "player": guests[3].0, "points": 3 })).await.status, StatusCode::OK);
    let v = check!("drawn");
    let mut drawn = Vec::new();
    for (i, (id, _)) in guests.iter().enumerate() {
        let d = &view_of(&v, &format!("guest {i}"))["draw"];
        let to = d["mine"]["id"].as_str().expect("each guest sees whom they drew").to_string();
        assert_ne!(&to, id, "nobody draws themselves");
        drawn.push(to);
    }
    assert!(drawn[0] != guests[1].0 && drawn[1] != guests[0].0, "Ana and Ben are kept apart");
    for who in ["host", "screen"] {
        assert_eq!(view_of(&v, who)["draw"], json!({ "done": true, "count": 4 }), "{who} sees no match");
    }
    assert_eq!(view_of(&v, "screen")["turn"]["current"]["id"], guests[0].0.as_str());
    // Step 3 locks the guesses; a late answer is refused
    assert_eq!(host_act(&t, &code, &host, "step", json!({})).await.status, StatusCode::OK);
    assert_eq!(act(&t, &code, &guests[3].1, "answer", json!({ "q": 0, "text": "late" })).await.status, StatusCode::CONFLICT);
    let v = check!("step 3");
    assert_eq!(view_of(&v, "guest 0")["collections"]["guesses"]["locked"], true);
    assert_eq!(view_of(&v, "guest 0")["timer"], Value::Null, "the lock ended the timer");
    assert!(checkpoints >= 8);

    // Step 4: the full reveal. The answers are out; the hidden group never is
    assert_eq!(host_act(&t, &code, &host, "step", json!({})).await.status, StatusCode::OK);
    let v = every_view(&t, &code, &host, &guests).await;
    assert_no_secret("revealed", &v, &[HIDDEN]);
    let g = view_of(&v, "guest 3");
    assert_eq!(g["revealed"], json!({ "answers": [ANSWER_0, ANSWER_1] }));
    assert_eq!(g["collections"]["picks"]["count"], 4);
    assert_eq!(g["collections"]["picks"]["rows"].as_array().unwrap().len(), 4, "after the reveal everyone sees every pick");
    let points: Vec<i64> = g["scores"].as_array().unwrap().iter().map(|s| s["points"].as_i64().unwrap()).collect();
    assert_eq!(points, [2, 0, 2, 3], "Ana and Cleo score 2 each; Dev has the host's 3");
    assert_eq!(act(&t, &code, &guests[3].1, "answer", json!({ "q": 1, "text": "x" })).await.status, StatusCode::CONFLICT);
    assert_eq!(host_act(&t, &code, &host, "step", json!({})).await.status, StatusCode::CONFLICT, "that was the last step");
}

#[tokio::test]
async fn a_guest_sent_score_is_refused_and_unknown_actions_stay_unknown() {
    let t = setup(true).await;
    let (code, host) = made(&t, spec()).await;
    let (_, key) = joined(&t, &code, "Ana").await;
    // Ana spends 6 actions and Ben 9: each stays under the per-guest burst of 10 (limits.rs)
    let (_, key2) = joined(&t, &code, "Ben").await;
    for body in [
        json!({ "text": ANSWER_0, "points": 5 }),
        json!({ "text": "x", "score": 99 }),
        json!({ "text": "x", "answer_key": ANSWER_0 }),
        json!({ "text": "x", "correct": true }),
    ] {
        let r = act(&t, &code, &key, "answer", body.clone()).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_action")), "{body}: {}", r.body);
        assert!(r.body.to_string().contains("isn't one of them"), "{}", r.body);
    }
    // Nothing was stored, and a put into the answer collection is refused (only answer writes it)
    let r = put(&t, &code, &key, "guesses", json!({ "text": ANSWER_0, "q": 0 })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
    let s = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &key)).await;
    assert_eq!(s.body["view"]["collections"]["guesses"]["count"], 0);
    let r = act(&t, &code, &key, "vote", json!({ "pick": "r_x", "points": 3 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // Host actions from a guest, guest actions from the host, and made-up names: unknown_action
    for name in ["adjust", "reveal", "draw", "step", "lock", "timer_start", "turn_next", "delete_everything", "score"] {
        let r = act(&t, &code, &key2, name, json!({})).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_action")), "guest {name}: {}", r.body);
    }
    for name in ["answer", "vote", "take_back", "score", "eval"] {
        let r = host_act(&t, &code, &host, name, json!({})).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_action")), "host {name}: {}", r.body);
    }
}

// ---- the kit, driven directly ----

fn players(names: &[&str]) -> Vec<Player> {
    let t0 = chrono::Utc::now();
    names
        .iter()
        .enumerate()
        .map(|(i, n)| Player { id: format!("p{i}"), name: n.to_string(), avatar: "cat".into(), team: None, joined_at: t0 + chrono::Duration::seconds(i as i64) })
        .collect()
}

fn guest(id: &str) -> Viewer {
    Viewer::Guest(id.to_string())
}

#[test]
fn the_draw_never_gives_anyone_themselves_keeps_pairs_apart_and_shows_each_guest_only_theirs() {
    let spec = spec();
    assert_eq!(Made.validate_spec(&spec), Ok(()));
    let ps = players(&["Ana", "Ben", "Cleo", "Dev", "Eve"]);
    for _ in 0..200 {
        let mut st = Made.init(&spec);
        let mut ctx = KitCtx::new(&spec, &ps, chrono::Utc::now());
        Made.act(&mut ctx, &mut st, &Viewer::Host, "draw", &json!({})).unwrap();
        let mut got = Vec::new();
        for p in &ps {
            let v = Made.view(&ctx, &st, &guest(&p.id));
            let to = v["draw"]["mine"]["id"].as_str().unwrap().to_string();
            assert_ne!(to, p.id);
            got.push(to);
            // nobody else's match is in this guest's view
            assert_eq!(v["draw"].as_object().unwrap().len(), 2, "{}", v["draw"]);
        }
        assert!(got[0] != "p1" && got[1] != "p0", "Ana and Ben apart: {got:?}");
        let mut sorted = got.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "everyone is drawn exactly once: {got:?}");
        for v in [Viewer::Host, Viewer::BigScreen] {
            assert_eq!(Made.view(&ctx, &st, &v)["draw"], json!({ "done": true, "count": 5 }));
        }
        assert!(matches!(Made.act(&mut ctx, &mut st, &Viewer::Host, "draw", &json!({})), Err(KitError::NotNow(_))), "once");
        assert!(Made.may_remove(&ctx, &st, "p2").is_err(), "removing would change another match");
    }
    // A late joiner goes into the loop: one other match changes, nobody gets themselves
    let six = players(&["Ana", "Ben", "Cleo", "Dev", "Eve", "Finn"]);
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &six[..5], chrono::Utc::now());
    Made.act(&mut ctx, &mut st, &Viewer::Host, "draw", &json!({})).unwrap();
    let before: Vec<Value> = six[..5].iter().map(|p| Made.view(&ctx, &st, &guest(&p.id))["draw"]["mine"]["id"].clone()).collect();
    let mut ctx = KitCtx::new(&spec, &six, chrono::Utc::now());
    Made.on_join(&mut ctx, &mut st, &six[5]);
    let after: Vec<Value> = six.iter().map(|p| Made.view(&ctx, &st, &guest(&p.id))["draw"]["mine"]["id"].clone()).collect();
    assert_ne!(after[5], json!("p5"));
    assert!(after[5].is_string(), "Finn drew someone");
    assert_eq!((0..5).filter(|i| before[*i] != after[*i]).count(), 1, "exactly one match changed: {before:?} {after:?}");
    // Too few players, and pairs that leave no loop, are plain refusals
    let three = players(&["Ana", "Ben"]);
    let mut ctx = KitCtx::new(&spec, &three, chrono::Utc::now());
    assert!(matches!(Made.act(&mut ctx, &mut Made.init(&spec), &Viewer::Host, "draw", &json!({})), Err(KitError::NotNow(m)) if m.contains("at least 3")));
    let mut tight = spec.clone();
    tight["actions"]["draw"]["apart"] = json!([["Ana", "Ben"], ["Ana", "Cleo"]]);
    let abc = players(&["Ana", "Ben", "Cleo"]);
    let mut ctx = KitCtx::new(&tight, &abc, chrono::Utc::now());
    assert!(matches!(Made.act(&mut ctx, &mut Made.init(&tight), &Viewer::Host, "draw", &json!({})), Err(KitError::NotNow(m)) if m.contains("Ana")));
}

#[test]
fn a_timer_locks_its_collection_through_tick_and_stop_or_unlock_undo_it() {
    let spec = spec();
    let ps = players(&["Ana", "Ben", "Cleo"]);
    let t0 = chrono::Utc::now();
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, t0);
    for bad in [json!({ "collection": "guesses", "seconds": 4 }), json!({ "collection": "nope" }), json!({})] {
        assert!(matches!(Made.act(&mut ctx, &mut st, &Viewer::Host, "timer_start", &bad), Err(KitError::BadInput(_))), "{bad}");
    }
    // The config's 30 seconds by default
    Made.act(&mut ctx, &mut st, &Viewer::Host, "timer_start", &json!({ "collection": "guesses" })).unwrap();
    assert_eq!(Made.view(&ctx, &st, &guest("p0"))["timer"]["seconds_left"], 30);
    let at = |s: i64| KitCtx::new(&spec, &ps, t0 + chrono::Duration::seconds(s));
    assert!(!Made.tick(&mut at(29), &mut st));
    Made.act(&mut at(29), &mut st, &guest("p0"), "answer", &json!({ "q": 0, "text": "in time" })).unwrap();
    // A poll after the end shows it locked even before the tick, and an answer then is refused
    assert_eq!(Made.view(&at(31), &st, &guest("p0"))["collections"]["guesses"]["locked"], true);
    assert!(matches!(Made.act(&mut at(31), &mut st, &guest("p1"), "answer", &json!({ "q": 0, "text": "late" })), Err(KitError::NotNow(_))));
    assert!(Made.tick(&mut at(31), &mut st) && !Made.tick(&mut at(32), &mut st), "the tick locks it once");
    assert_eq!(st["locked"]["guesses"], true);
    assert_eq!(st["timer"], Value::Null);
    Made.act(&mut at(40), &mut st, &Viewer::Host, "unlock", &json!({ "collection": "guesses" })).unwrap();
    Made.act(&mut at(40), &mut st, &guest("p1"), "answer", &json!({ "q": 0, "text": "after unlock" })).unwrap();
    // timer_stop: no lock
    Made.act(&mut at(41), &mut st, &Viewer::Host, "timer_start", &json!({ "vote": true, "seconds": 5 })).unwrap();
    Made.act(&mut at(42), &mut st, &Viewer::Host, "timer_stop", &json!({})).unwrap();
    assert!(!Made.tick(&mut at(60), &mut st));
    assert!(matches!(Made.act(&mut at(60), &mut st, &Viewer::Host, "timer_stop", &json!({})), Err(KitError::NotNow(_))));
    // lock on a collection stops guests' put and take_back, not the host's put
    Made.act(&mut at(61), &mut st, &guest("p0"), "put", &json!({ "collection": "shelf", "values": { "title": "Emma" } })).unwrap();
    let id = st["rows"]["shelf"][0]["id"].as_str().unwrap().to_string();
    Made.act(&mut at(62), &mut st, &Viewer::Host, "lock", &json!({ "collection": "shelf" })).unwrap();
    assert!(matches!(Made.act(&mut at(63), &mut st, &guest("p0"), "put", &json!({ "collection": "shelf", "values": { "title": "Ulysses" } })), Err(KitError::NotNow(m)) if m.contains("closed")));
    assert!(matches!(Made.act(&mut at(63), &mut st, &guest("p0"), "take_back", &json!({ "collection": "shelf", "id": id })), Err(KitError::NotNow(_))));
    Made.act(&mut at(63), &mut st, &Viewer::Host, "put", &json!({ "collection": "shelf", "values": { "title": "Host's" } })).unwrap();
    // lock on the vote closes voting
    Made.act(&mut at(64), &mut st, &Viewer::Host, "lock", &json!({ "vote": true })).unwrap();
    assert!(matches!(Made.act(&mut at(65), &mut st, &guest("p0"), "vote", &json!({ "pick": id })), Err(KitError::NotNow(m)) if m.contains("Voting has closed")));
    // A closed vote freezes its options too: taking back the winning option would change the tally
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, t0);
    for (p, t) in [("p0", "Emma"), ("p1", "Dune")] {
        Made.act(&mut ctx, &mut st, &guest(p), "put", &json!({ "collection": "shelf", "values": { "title": t } })).unwrap();
    }
    let emma = st["rows"]["shelf"][0]["id"].as_str().unwrap().to_string();
    for p in ["p0", "p1", "p2"] {
        Made.act(&mut ctx, &mut st, &guest(p), "vote", &json!({ "pick": emma })).unwrap();
    }
    Made.act(&mut ctx, &mut st, &Viewer::Host, "lock", &json!({ "vote": true })).unwrap();
    let tally = Made.view(&ctx, &st, &Viewer::BigScreen)["vote"]["tally"].clone();
    assert_eq!(tally["outcome"], json!({ "kind": "winner", "id": emma }), "{tally}");
    assert!(matches!(Made.act(&mut ctx, &mut st, &guest("p0"), "take_back", &json!({ "collection": "shelf", "id": emma })), Err(KitError::NotNow(m)) if m == "Voting has closed."));
    assert!(matches!(Made.act(&mut ctx, &mut st, &guest("p2"), "put", &json!({ "collection": "shelf", "values": { "title": "Late" } })), Err(KitError::NotNow(m)) if m == "Voting has closed."));
    assert_eq!(Made.view(&ctx, &st, &Viewer::BigScreen)["vote"]["tally"], tally, "the tally is unchanged");
    assert_eq!(Made.view(&ctx, &st, &guest("p0"))["collections"]["shelf"]["locked"], true, "the options show closed");
    // The same through a vote timer that ran out, before its tick
    Made.act(&mut ctx, &mut st, &Viewer::Host, "unlock", &json!({ "vote": true })).unwrap();
    Made.act(&mut ctx, &mut st, &Viewer::Host, "timer_start", &json!({ "vote": true, "seconds": 5 })).unwrap();
    assert!(matches!(Made.act(&mut at(6), &mut st, &guest("p0"), "take_back", &json!({ "collection": "shelf", "id": emma })), Err(KitError::NotNow(m)) if m == "Voting has closed."));
    // Stopping or replacing a timer that ran out (before its tick) keeps the lock every poll showed
    assert!(matches!(Made.act(&mut at(6), &mut st, &Viewer::Host, "timer_stop", &json!({})), Err(KitError::NotNow(_))), "it had ended");
    assert_eq!((st["vote_locked"].clone(), st["timer"].clone()), (json!(true), Value::Null), "stopping an ended timer locks, never reopens");
    let mut st2 = Made.init(&spec);
    Made.act(&mut ctx, &mut st2, &Viewer::Host, "timer_start", &json!({ "collection": "guesses", "seconds": 5 })).unwrap();
    Made.act(&mut at(6), &mut st2, &Viewer::Host, "timer_start", &json!({ "vote": true, "seconds": 30 })).unwrap();
    assert_eq!((st2["locked"]["guesses"].clone(), st2["timer"]["vote"].clone()), (json!(true), json!(true)), "the ended guesses timer locked before the vote's replaced it");
}

/// Sends a turn action as the host: (whose turn, round, who was skipped) on the big screen.
fn turn(ctx: &mut KitCtx, st: &mut Value, name: &str) -> (String, u64, Option<String>) {
    Made.act(ctx, st, &Viewer::Host, name, &json!({})).unwrap();
    let t = Made.view(ctx, st, &Viewer::BigScreen)["turn"].clone();
    (t["current"]["name"].as_str().unwrap().to_string(), t["round"].as_u64().unwrap(), t["skipped"]["name"].as_str().map(str::to_string))
}

#[test]
fn turns_rotate_in_join_order_skip_and_follow_who_is_in() {
    let spec = spec();
    let ps = players(&["Ana", "Ben", "Cleo"]);
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, chrono::Utc::now());
    assert_eq!(Made.view(&ctx, &st, &Viewer::BigScreen)["turn"], Value::Null);
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Ana".into(), 1, None));
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Ben".into(), 1, None));
    // skip: Cleo would be next, so she's passed over and it wraps to Ana in round 2
    assert_eq!(turn(&mut ctx, &mut st, "turn_skip"), ("Ana".into(), 2, Some("Cleo".into())));
    assert_eq!(Made.view(&ctx, &st, &guest("p2"))["turn"]["next"]["name"], "Ben");
    // Dev joins: the end of the order. Ben leaves: he drops out
    let four = players(&["Ana", "Ben", "Cleo", "Dev"]);
    let mut ctx = KitCtx::new(&spec, &four, chrono::Utc::now());
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Ben".into(), 2, None));
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Cleo".into(), 2, None));
    let no_ben: Vec<Player> = four.iter().filter(|p| p.name != "Ben").cloned().collect();
    let mut ctx = KitCtx::new(&spec, &no_ben, chrono::Utc::now());
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Dev".into(), 2, None));
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Ana".into(), 3, None));
    assert_eq!(turn(&mut ctx, &mut st, "turn_next"), ("Cleo".into(), 3, None));
    // Shuffled: everyone once per round
    let mut shuffled = spec.clone();
    shuffled["actions"]["turns"]["order"] = json!("shuffled");
    let mut st = Made.init(&shuffled);
    let mut ctx = KitCtx::new(&shuffled, &four, chrono::Utc::now());
    let mut seen: Vec<String> = (0..4).map(|_| turn(&mut ctx, &mut st, "turn_next").0).collect();
    seen.sort();
    assert_eq!(seen, ["Ana", "Ben", "Cleo", "Dev"]);
    let empty: Vec<Player> = vec![];
    let mut ctx = KitCtx::new(&spec, &empty, chrono::Utc::now());
    assert!(matches!(Made.act(&mut ctx, &mut Made.init(&spec), &Viewer::Host, "turn_next", &json!({})), Err(KitError::NotNow(_))));
}

#[test]
fn reveal_at_flips_the_reveal_publishes_its_secrets_and_scores_follow() {
    let reveal_at = chrono::Utc::now() + chrono::Duration::days(3);
    let mut spec = spec();
    spec["reveal_at"] = json!(reveal_at.to_rfc3339());
    assert_eq!(Made.validate_spec(&spec), Ok(()));
    let ps = players(&["Ana", "Ben", "Cleo"]);
    let now = chrono::Utc::now();
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, now);
    Made.act(&mut ctx, &mut st, &guest("p0"), "answer", &json!({ "q": 0, "text": "ishmael only" })).unwrap();
    Made.act(&mut ctx, &mut st, &guest("p1"), "answer", &json!({ "q": 1, "text": "Whale road, sorrow" })).unwrap();
    Made.act(&mut ctx, &mut st, &guest("p2"), "answer", &json!({ "q": 1, "text": "whale" })).unwrap();
    assert!(!Made.tick(&mut ctx, &mut st));
    for v in [guest("p0"), guest("p1"), Viewer::Host, Viewer::BigScreen] {
        let out = normalise(&without_own_rows(&Made.view(&ctx, &st, &v)).to_string());
        for s in [ANSWER_0, ACCEPT_0, ANSWER_1, HIDDEN] {
            assert!(!out.contains(&normalise(s)), "{v:?} sees {s:?} before reveal_at: {out}");
        }
    }
    let mut later = KitCtx::new(&spec, &ps, reveal_at + chrono::Duration::seconds(1));
    // A poll before the tick already shows it
    assert_eq!(Made.view(&later, &st, &guest("p2"))["revealed"]["answers"], json!([ANSWER_0, ANSWER_1]));
    assert!(Made.tick(&mut later, &mut st) && !Made.tick(&mut later, &mut st), "once");
    assert_eq!((&st["revealed"], &st["locked"]["guesses"]), (&json!(true), &json!(true)));
    let v = Made.view(&later, &st, &Viewer::BigScreen);
    assert_eq!(v["revealed"], json!({ "answers": [ANSWER_0, ANSWER_1] }), "the hidden group stays hidden");
    assert_eq!(v["scores"], json!([{ "id": "p0", "points": 2 }, { "id": "p1", "points": 2 }, { "id": "p2", "points": 0 }]), "an accepted spelling counts; a near miss doesn't");
    assert!(matches!(Made.act(&mut later, &mut st, &guest("p2"), "answer", &json!({ "q": 1, "text": "x" })), Err(KitError::NotNow(m)) if m.contains("out")));
    assert!(matches!(Made.act(&mut later, &mut st, &Viewer::Host, "reveal", &json!({})), Err(KitError::NotNow(_))), "already revealed");
    // A partial reveal names groups and doesn't flip the after_reveal rows
    let spec = self::spec();
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, now);
    Made.act(&mut ctx, &mut st, &Viewer::Host, "reveal", &json!({ "secrets": ["hidden"] })).unwrap();
    assert_eq!((Made.view(&ctx, &st, &guest("p0"))["revealed"].clone(), st["revealed"].clone()), (json!({ "hidden": [HIDDEN] }), json!(false)));
    assert!(matches!(Made.act(&mut ctx, &mut st, &Viewer::Host, "reveal", &json!({ "secrets": ["nope"] })), Err(KitError::BadInput(_))));
    assert!(matches!(Made.act(&mut ctx, &mut st, &Viewer::Host, "reveal", &json!({ "points": 1 })), Err(KitError::BadInput(_))));
}

#[test]
fn step_walks_the_steps_and_adjust_is_capped() {
    let spec = spec();
    let ps = players(&["Ana", "Ben", "Cleo"]);
    let now = chrono::Utc::now();
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, now);
    assert_eq!(Made.view(&ctx, &st, &guest("p0"))["step"], json!({ "at": null, "of": 4, "say": null }));
    let walk: Vec<(Value, Value, Value)> = (0..4)
        .map(|_| {
            Made.act(&mut ctx, &mut st, &Viewer::Host, "step", &json!({})).unwrap();
            let v = Made.view(&ctx, &st, &guest("p0"));
            (v["step"]["at"].clone(), v["collections"]["guesses"]["locked"].clone(), v["timer"]["collection"].clone())
        })
        .collect();
    assert_eq!(walk, [(json!(0), json!(false), Value::Null), (json!(1), json!(false), json!("guesses")), (json!(2), json!(true), Value::Null), (json!(3), json!(true), Value::Null)]);
    assert_eq!(st["revealed"], true);
    assert!(matches!(Made.act(&mut ctx, &mut st, &Viewer::Host, "step", &json!({})), Err(KitError::NotNow(m)) if m.contains("last step")));
    assert!(matches!(Made.act(&mut ctx, &mut st, &Viewer::Host, "step", &json!({ "to": 9 })), Err(KitError::BadInput(_))));
    // adjust: 1..=100 either way, a real player, never 0
    for (body, ok) in [
        (json!({ "player": "p1", "points": 100 }), true),
        (json!({ "player": "p1", "points": -100 }), true),
        (json!({ "player": "p1", "points": 7 }), true),
        (json!({ "player": "p1", "points": 101 }), false),
        (json!({ "player": "p1", "points": -101 }), false),
        (json!({ "player": "p1", "points": -9223372036854775808_i64 }), false),
        (json!({ "player": "p1", "points": 9223372036854775807_i64 }), false),
        (json!({ "player": "p1", "points": 0 }), false),
        (json!({ "player": "p1", "points": 1.5 }), false),
        (json!({ "player": "nobody", "points": 1 }), false),
        (json!({ "player": "p1", "points": 1, "why": "x" }), false),
    ] {
        let r = Made.act(&mut ctx, &mut st, &Viewer::Host, "adjust", &body);
        assert_eq!(r.is_ok(), ok, "{body}: {r:?}");
    }
    let scores = Made.view(&ctx, &st, &Viewer::Host)["scores"].clone();
    assert_eq!(scores[1], json!({ "id": "p1", "points": 7 }));
    assert_eq!(kwnt::pocket::made::actions::MAX_ADJUST, kwnt::kits::quiz::engine::MAX_ADJUST, "the quiz's cap");
}

#[test]
fn a_ranked_vote_counts_on_the_box_and_nobody_sees_anyone_elses_ballot() {
    let mut spec = spec();
    spec["actions"]["vote"] = json!({ "collection": "shelf", "count": "ranked", "tally": "after_reveal" });
    assert_eq!(Made.validate_spec(&spec), Ok(()));
    let ps = players(&["Ana", "Ben", "Cleo", "Dev", "Eve"]);
    let mut st = Made.init(&spec);
    let mut ctx = KitCtx::new(&spec, &ps, chrono::Utc::now());
    for (p, t) in [("p0", "A"), ("p1", "B"), ("p2", "C")] {
        Made.act(&mut ctx, &mut st, &guest(p), "put", &json!({ "collection": "shelf", "values": { "title": t } })).unwrap();
    }
    let ids: Vec<String> = st["rows"]["shelf"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    // First choices: A 2, B 2, C 1. Nobody has a majority, C drops, Eve's second choice is B
    for (p, r) in [("p0", &[0, 1][..]), ("p1", &[0]), ("p2", &[1]), ("p3", &[1, 0]), ("p4", &[2, 1])] {
        let ranking: Vec<&str> = r.iter().map(|i| ids[*i].as_str()).collect();
        Made.act(&mut ctx, &mut st, &guest(p), "vote", &json!({ "ranking": ranking })).unwrap();
    }
    for bad in [json!({ "pick": ids[0] }), json!({ "ranking": [ids[0], ids[0]] }), json!({ "ranking": ["r_nope"] }), json!({ "ranking": [] }), json!({ "ranking": [ids[0]], "weight": 9 })] {
        assert!(matches!(Made.act(&mut ctx, &mut st, &guest("p0"), "vote", &bad), Err(KitError::BadInput(_))), "{bad}");
    }
    let v = Made.view(&ctx, &st, &guest("p3"));
    assert_eq!((v["vote"]["mine"].clone(), v["vote"]["voters"].clone(), v["vote"]["tally"].clone()), (json!([ids[1], ids[0]]), json!(5), Value::Null), "the tally waits for the reveal");
    for viewer in [Viewer::Host, Viewer::BigScreen, guest("p0")] {
        let out = Made.view(&ctx, &st, &viewer);
        assert!(!out.to_string().contains("ballots"));
        assert_ne!(out["vote"]["mine"], json!([ids[1], ids[0]]), "{viewer:?} sees Dev's ballot");
    }
    Made.act(&mut ctx, &mut st, &Viewer::Host, "reveal", &json!({})).unwrap();
    let v = Made.view(&ctx, &st, &Viewer::BigScreen);
    assert_eq!((v["vote"]["tally"]["outcome"].clone(), v["vote"]["tally"]["rounds"].clone()), (json!({ "kind": "winner", "id": ids[1] }), json!(2)), "{}", v["vote"]);
    // A removed player's ballot goes
    Made.on_remove(&mut ctx, &mut st, "p4");
    assert!(st["ballots"].get("p4").is_none());
}

#[test]
fn the_spec_checks_actions_secrets_and_steps_in_plain_sentences() {
    let base = spec();
    let with = |path: &[&str], v: Value| {
        let mut s = base.clone();
        let mut at = &mut s;
        for p in &path[..path.len() - 1] {
            at = &mut at[*p];
        }
        at[path[path.len() - 1]] = v;
        s
    };
    let cases = [
        (with(&["actions", "score"], json!({})), "actions has \"score\", which isn't an action."),
        (with(&["actions", "answer", "collection"], json!("shelf")), "needs write \"server\""),
        (with(&["actions", "answer", "against"], json!("secrets.nope")), "there's no secret group called \"nope\""),
        (with(&["actions", "answer", "points"], json!(0)), "points must be a whole number from 1 to 100."),
        (with(&["actions", "answer", "match"], json!("judge")), "match is \"text\""),
        (with(&["collections", "guesses", "read"], json!("everyone")), "needs read \"mine\""),
        (with(&["collections", "guesses", "read"], json!("host")), "needs read \"mine\""),
        (with(&["collections", "guesses", "fields", "q", "max"], json!(5)), "field \"q\" needs min 0 and max 1."),
        (with(&["actions", "vote", "collection"], json!("picks")), "needs read \"everyone\""),
        (with(&["actions", "vote", "count"], json!("borda")), "count must be one_pick or ranked."),
        (with(&["actions", "draw", "apart"], json!([["Ana"]])), "each apart entry is a pair of names"),
        (with(&["actions", "timer", "seconds"], json!(2)), "seconds must be a whole number from 5 to 3600."),
        (with(&["actions", "turns", "order"], json!("age")), "order must be join or shuffled."),
        (with(&["actions", "reveal", "collections"], json!(["shelf"])), "\"shelf\" isn't an after_reveal collection."),
        (with(&["secrets", "answers"], json!([3])), "secrets.answers must be a text or"),
        (with(&["secrets", "Bad"], json!("x")), "Secret group \"Bad\": a name is lower-case"),
        (with(&["steps"], json!([{ "jump": 3 }])), "Step 1 doesn't take \"jump\"."),
        (with(&["steps"], json!([{ "timer": 30 }])), "Step 1: a timer goes with open"),
        (with(&["steps"], json!([{ "open": "nope" }])), "Step 1: there's no collection called \"nope\" to open."),
        (with(&["steps"], json!([{ "reveal": ["nope"] }])), "Step 1: there's no secret group called"),
        (with(&["actions", "reveal", "secrets"], json!(["hidden"])), "actions.reveal: secrets must include \"answers\", the group answer is played against."),
    ];
    for (spec, want) in cases {
        let got = Made.validate_spec(&spec).expect_err(want);
        assert!(got.contains(want), "want {want:?}, got {got:?}");
        assert!(got.ends_with('.') && !got.contains('\n'), "one plain sentence: {got:?}");
        for s in [ANSWER_0, ANSWER_1, HIDDEN] {
            assert!(!got.contains(s), "an error never repeats a secret: {got}");
        }
    }
}
