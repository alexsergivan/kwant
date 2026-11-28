//! pa12 L2 (autonomy/pocket-apps/pa12/L2/brief.md, plan.md section 1): the `made` kit's
//! collections. Each read rule (`everyone`, `mine`, `host`, `after_reveal`) gives each viewer (four
//! guests, the big screen, the host) only what the rule allows, the same on every way out: polling,
//! the WebSocket, `info`, the page's boot and the person's main-port read. The leak test puts a
//! unique marker in every private row and in `secrets`, then string-searches every response each
//! other viewer gets.

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
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const LOCAL: &str = "127.0.0.1:50000";
const GHOST: &str = "pocket.example.net";
const SECRET: &str = "secret-marker-7f3a";

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

async fn host_put(t: &T, code: &str, host: &str, coll: &str, values: Value) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": "put", "body": { "collection": coll, "values": values } })), &host_req(host)).await
}

async fn take_back(t: &T, code: &str, key: &str, id: &str) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "take_back", "body": { "collection": "shelf", "id": id } })), &guest_req(code, key)).await
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

fn spec() -> Value {
    json!({
        "format": "made/1",
        "title": "Book swap",
        "retention": "after_end",
        "reveal_at": "2099-12-18T19:00:00Z",
        "words": { "intro": "Bring one book." },
        "collections": {
            "shelf": { "read": "everyone", "write": "guest_many", "max_per_guest": 2, "fields": { "title": { "type": "text", "max": 60 } } },
            "guesses": { "read": "mine", "write": "guest_one", "fields": { "text": { "type": "text", "max": 60 } } },
            "notes": { "read": "host", "write": "guest_one", "fields": { "line": { "type": "text", "max": 60 } } },
            "picks": { "read": "after_reveal", "write": "guest_one", "fields": { "text": { "type": "text", "max": 60 }, "book": { "type": "row", "of": "shelf" } } },
            "tally": { "read": "everyone", "write": "server", "fields": { "n": { "type": "int", "min": 0, "max": 99 } } }
        },
        "secrets": { "answer": SECRET, "list": [SECRET] }
    })
}

/// The rows' `values` of one collection in a view, or `None` when it has no `rows` (a count, or left out).
fn rows<'a>(view: &'a Value, coll: &str) -> Option<&'a Vec<Value>> {
    view["view"]["collections"][coll]["rows"].as_array()
}

fn texts(view: &Value, coll: &str, field: &str) -> Vec<String> {
    let mut v: Vec<String> = rows(view, coll).map(|r| r.iter().filter_map(|r| r["values"][field].as_str().map(str::to_string)).collect()).unwrap_or_default();
    v.sort();
    v
}

#[tokio::test]
async fn each_read_rule_gives_each_viewer_its_own_part_and_nothing_private_leaks() {
    let t = setup(true).await;
    let addr = t.state.pocket.listener.addr().expect("the guest listener runs");
    let (code, host) = made(&t, spec()).await;
    let mut guests = Vec::new();
    for (i, name) in ["Ana", "Ben", "Cleo", "Dev"].iter().enumerate() {
        let (id, key) = joined(&t, &code, name).await;
        for (coll, values) in [
            ("shelf", json!({ "title": format!("shelf-g{i}") })),
            ("guesses", json!({ "text": format!("mine-g{i}-a1") })),
            ("notes", json!({ "line": format!("host-g{i}-b2") })),
        ] {
            let r = put(&t, &code, &key, coll, values).await;
            assert_eq!(r.status, StatusCode::OK, "{coll}: {}", r.body);
        }
        guests.push((id, key));
    }
    // A pick points at a shelf row (everyone can see the shelf)
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &guests[0].1)).await;
    let shelf_ids: Vec<String> = rows(&r.body, "shelf").unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(shelf_ids.len(), 4);
    for (i, (_, key)) in guests.iter().enumerate() {
        let r = put(&t, &code, key, "picks", json!({ "text": format!("pick-g{i}-c3"), "book": shelf_ids[(i + 1) % 4] })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    }
    // The host's own row in a host-read collection, put from the remote
    let r = host_put(&t, &code, &host, "notes", json!({ "line": "host-own-d4" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);

    let mine = |i: usize| format!("mine-g{i}-a1");
    let note = |i: usize| format!("host-g{i}-b2");
    let pick = |i: usize| format!("pick-g{i}-c3");
    let shelf: Vec<String> = (0..4).map(|i| format!("shelf-g{i}")).collect();
    let all_notes: Vec<String> = (0..4).map(note).chain(["host-own-d4".to_string()]).collect();

    // Every response each viewer gets, labelled; the leak search runs on all of them before the
    // per-viewer checks, so a leak fails on the search itself
    let mut seen: Vec<(String, Vec<(String, Value)>)> = Vec::new();
    let mut polls = Vec::new();
    for (i, (_, key)) in guests.iter().enumerate() {
        let poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, key)).await;
        assert_eq!(poll.status, StatusCode::OK);
        let ws = first_state(ws_connect(addr, &code, Some((guest::guest_cookie(&code), key.clone())), None, None).await).await;
        let page = send(&t.guest, "GET", &format!("/p/{code}"), None, &guest_req(&code, key)).await;
        assert_eq!(page.status, StatusCode::OK);
        let html = page.body.as_str().unwrap().to_string();
        polls.push((poll.body.clone(), ws.clone(), boot_of(&html)));
        seen.push((format!("guest {i}"), vec![("poll".into(), poll.body), ("ws".into(), ws), ("boot".into(), Value::String(html))]));
    }
    let screen = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state?view=screen"), None, &host_req(&host)).await;
    assert_eq!(screen.status, StatusCode::OK, "{}", screen.body);
    let screen_ws = first_state(ws_connect(addr, &code, None, Some(host.as_str()), Some("screen")).await).await;
    seen.push(("screen".into(), vec![("poll".into(), screen.body.clone()), ("ws".into(), screen_ws.clone())]));
    let hosted = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(&host)).await;
    assert_eq!(hosted.status, StatusCode::OK, "{}", hosted.body);
    let host_ws = first_state(ws_connect(addr, &code, None, Some(host.as_str()), Some("host")).await).await;
    let main_read = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &local()).await;
    assert_eq!(main_read.status, StatusCode::OK, "{}", main_read.body);
    let main_list = send(&t.main, "GET", "/api/pockets", None, &local()).await;
    seen.push(("host".into(), vec![("poll".into(), hosted.body.clone()), ("ws".into(), host_ws.clone()), ("main read".into(), main_read.body.clone()), ("main list".into(), main_list.body)]));
    // Before joining: info and the page's boot
    let info = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), None, &Req::default()).await;
    assert_eq!(info.status, StatusCode::OK, "{}", info.body);
    let page = send(&t.guest, "GET", &format!("/p/{code}"), None, &Req::default()).await;
    let public = Value::String(page.body.as_str().unwrap_or_default().to_string());
    seen.push(("nobody".into(), vec![("info".into(), info.body.clone()), ("boot".into(), public)]));

    // The leak test: what no viewer but the owner (and the host, for host rows) may ever see
    for (who, responses) in &seen {
        let mut banned: Vec<String> = vec![SECRET.to_string()];
        for i in 0..4 {
            if who != &format!("guest {i}") {
                banned.push(mine(i));
                banned.push(pick(i));
                if who != "host" {
                    banned.push(note(i));
                }
            }
        }
        if who != "host" {
            banned.push("host-own-d4".into());
        }
        for (way, v) in responses {
            let text = match v {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            for b in &banned {
                assert!(!text.contains(b.as_str()), "{who} {way} leaks {b:?}: {text}");
            }
        }
    }

    // Each guest: the same part on every way out, and exactly what each rule gives them
    for (i, (v, ws, boot)) in polls.iter().enumerate() {
        assert_eq!(ws["view"], v["view"], "guest {i}: ws and polling agree");
        if let Some(boot) = boot {
            assert_eq!(boot["view"], v["view"], "guest {i}: the page boot is the same view");
        }
        assert_eq!(v["me"]["id"], guests[i].0.as_str());
        // everyone: all rows
        assert_eq!(texts(v, "shelf", "title"), shelf, "guest {i} sees the whole shelf");
        // mine: own row only, count of own
        assert_eq!(texts(v, "guesses", "text"), vec![mine(i)]);
        assert_eq!(v["view"]["collections"]["guesses"]["count"], 1);
        assert_eq!(rows(v, "guesses").unwrap()[0]["mine"], true);
        // host: own row only
        assert_eq!(texts(v, "notes", "line"), vec![note(i)]);
        assert_eq!(v["view"]["collections"]["notes"]["count"], 1);
        // after_reveal before the reveal: own row, the total as the count
        assert_eq!(texts(v, "picks", "text"), vec![pick(i)]);
        assert_eq!(v["view"]["collections"]["picks"]["count"], 4);
        // server: everyone, no rows yet
        assert_eq!(v["view"]["collections"]["tally"], json!({ "rows": [], "count": 0, "locked": false }));
        assert_eq!(v["view"]["title"], "Book swap");
        assert_eq!(v["view"]["words"]["intro"], "Bring one book.");
        assert_eq!(v["view"]["revealed"], json!({}));
        assert_eq!(v["view"]["players"].as_array().unwrap().len(), 4);
    }
    // The big screen: counts for mine/after_reveal, nothing at all of a host collection
    assert_eq!(screen_ws["view"], screen.body["view"]);
    let c = &screen.body["view"]["collections"];
    assert_eq!(texts(&screen.body, "shelf", "title"), shelf);
    assert_eq!(c["guesses"], json!({ "count": 4, "locked": false }));
    assert_eq!(c["picks"], json!({ "count": 4, "locked": false }));
    assert!(c.get("notes").is_none(), "the screen doesn't even get a count of a host collection: {c}");
    // The host: every host-read row, counts for mine/after_reveal
    assert_eq!(host_ws["view"], hosted.body["view"]);
    let c = &hosted.body["view"]["collections"];
    let mut want_notes = all_notes.clone();
    want_notes.sort();
    assert_eq!(texts(&hosted.body, "notes", "line"), want_notes);
    assert_eq!(c["guesses"], json!({ "count": 4, "locked": false }));
    assert_eq!(c["picks"], json!({ "count": 4, "locked": false }));
    let own: Vec<&Value> = rows(&hosted.body, "notes").unwrap().iter().filter(|r| r["mine"] == true).collect();
    assert_eq!((own.len(), &own[0]["values"]["line"]), (1, &json!("host-own-d4")));
    // The person's main-port read: the summary's counts, and the host's view
    assert_eq!(main_read.body["summary"]["counts"], json!({ "shelf": 4, "guesses": 4, "notes": 5, "picks": 4, "tally": 0 }));
    assert_eq!(main_read.body["host_view"]["collections"], hosted.body["view"]["collections"]);
    // Before joining: the lobby counts the everyone collections only
    assert!(info.body.to_string().contains(r#""counts":{"shelf":4,"tally":0}"#), "{}", info.body);
}

#[tokio::test]
async fn after_the_reveal_everyone_sees_every_after_reveal_row() {
    let t = setup(true).await;
    let mut s = spec();
    s["reveal_at"] = json!("2020-01-01T00:00:00Z");
    let (code, host) = made(&t, s).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let (_, b) = joined(&t, &code, "Ben").await;
    assert_eq!(put(&t, &code, &a, "shelf", json!({ "title": "Emma" })).await.status, StatusCode::OK);
    let poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &b)).await;
    let id = rows(&poll.body, "shelf").unwrap()[0]["id"].as_str().unwrap().to_string();
    assert_eq!(put(&t, &code, &a, "picks", json!({ "text": "ana-pick", "book": id })).await.status, StatusCode::OK);
    let poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &b)).await;
    assert_eq!(texts(&poll.body, "picks", "text"), vec!["ana-pick"]);
    assert_eq!(rows(&poll.body, "picks").unwrap()[0]["mine"], false);
    let screen = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state?view=screen"), None, &host_req(&host)).await;
    assert_eq!(texts(&screen.body, "picks", "text"), vec!["ana-pick"]);
    // mine stays mine after the reveal
    assert_eq!(put(&t, &code, &a, "guesses", json!({ "text": "ana-guess" })).await.status, StatusCode::OK);
    let poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &b)).await;
    assert!(!poll.body.to_string().contains("ana-guess"));
}

#[tokio::test]
async fn write_rules_overwrite_cap_refuse_server_and_check_row_refs() {
    let t = setup(true).await;
    let (code, host) = made(&t, spec()).await;
    let (_, a) = joined(&t, &code, "Ana").await;
    let (_, b) = joined(&t, &code, "Ben").await;
    // Each guest may send 10 actions a second (limits::ACTIONS_PER_SEC); Ana's writes stay at 8,
    // and the row-field checks below go to Cy so no guest is ever slowed down mid-test.
    let (_, c) = joined(&t, &code, "Cy").await;
    let state = |key: String| {
        let t = &t;
        let code = code.clone();
        async move { send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &key)).await.body }
    };
    // guest_one: the second put overwrites the first, same row id
    put(&t, &code, &a, "guesses", json!({ "text": "first" })).await;
    let before = rows(&state(a.clone()).await, "guesses").unwrap()[0]["id"].clone();
    let r = put(&t, &code, &a, "guesses", json!({ "text": "second" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let v = state(a.clone()).await;
    assert_eq!((rows(&v, "guesses").unwrap().len(), rows(&v, "guesses").unwrap()[0]["id"].clone()), (1, before));
    assert_eq!(texts(&v, "guesses", "text"), vec!["second"]);
    // guest_many: max_per_guest 2, per guest
    for title in ["one", "two"] {
        assert_eq!(put(&t, &code, &a, "shelf", json!({ "title": title })).await.status, StatusCode::OK);
    }
    let r = put(&t, &code, &a, "shelf", json!({ "title": "three" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("not_now")), "{}", r.body);
    assert_eq!(r.body["message"], "You can add at most 2 to \"shelf\". Take one back first.");
    assert_eq!(put(&t, &code, &b, "shelf", json!({ "title": "bens" })).await.status, StatusCode::OK, "the cap is per guest");
    // take_back frees a place; another guest's row answers like a missing one
    let mine_id = rows(&state(a.clone()).await, "shelf").unwrap().iter().find(|r| r["mine"] == true).unwrap()["id"].as_str().unwrap().to_string();
    let theirs = take_back(&t, &code, &b, &mine_id).await;
    let missing = take_back(&t, &code, &b, "r_0000000000000000").await;
    assert_eq!((theirs.status, &theirs.body), (missing.status, &missing.body));
    assert_eq!(take_back(&t, &code, &a, &mine_id).await.status, StatusCode::OK);
    assert_eq!(put(&t, &code, &a, "shelf", json!({ "title": "three" })).await.status, StatusCode::OK);
    // server: nobody puts, not even the host
    for r in [put(&t, &code, &a, "tally", json!({ "n": 1 })).await, host_put(&t, &code, &host, "tally", json!({ "n": 1 })).await] {
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("not_now")), "{}", r.body);
    }
    // the host can't take_back (not a host action)
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": "take_back", "body": { "collection": "shelf", "id": mine_id } })), &host_req(&host)).await;
    assert!(r.status.is_client_error(), "{}", r.body);
    // a row field: a missing id refuses
    let r = put(&t, &code, &c, "picks", json!({ "text": "x", "book": "r_0000000000000000" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_action")), "{}", r.body);
    assert_eq!(r.body["message"], "book: there's no such row in \"shelf\".");
    // unknown and missing fields, and wrong types. mine_id was taken back above, so these point at
    // a live row Cy can see (Ana's new "three"): fields are checked in name order, so with a dead
    // row "book" would be refused before "text" is looked at.
    let live_id = rows(&state(c.clone()).await, "shelf").unwrap().iter().find(|r| r["values"]["title"] == "three").unwrap()["id"].as_str().unwrap().to_string();
    assert_ne!(live_id, mine_id);
    for (values, want) in [
        (json!({ "text": "x", "book": live_id, "extra": 1 }), "\"picks\" has no field called \"extra\"."),
        (json!({ "text": "x" }), "book: fill this in."),
        (json!({ "text": 5, "book": live_id }), "text: send a text."),
    ] {
        let r = put(&t, &code, &c, "picks", values).await;
        assert_eq!((r.status, r.body["message"].as_str()), (StatusCode::BAD_REQUEST, Some(want)), "{}", r.body);
    }
}

#[tokio::test]
async fn spec_mistakes_are_refused_with_one_plain_sentence() {
    let t = setup(true).await;
    let with = |path: &[&str], v: Value| {
        let mut s = spec();
        let mut at = &mut s;
        for p in &path[..path.len() - 1] {
            at = &mut at[*p];
        }
        at[path[path.len() - 1]] = v;
        s
    };
    for bad in [
        json!("not an object"),
        with(&["format"], json!("made/2")),
        with(&["title"], json!("")),
        with(&["surprise"], json!(1)),
        with(&["collections", "shelf", "read"], json!("friends")),
        with(&["collections", "shelf", "write"], json!("anyone")),
        with(&["collections", "picks", "fields", "book", "of"], json!("nowhere")),
        with(&["collections", "shelf", "fields", "title", "type"], json!("html")),
        with(&["collections"], json!({})),
        with(&["reveal_at"], json!("next tuesday")),
    ] {
        let r = make(&t, bad.clone()).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_spec")), "{bad}: {}", r.body);
        let m = r.body["message"].as_str().unwrap();
        assert!(m.ends_with('.') && !m.contains('\n') && m.len() < 200, "{m:?}");
        for jargon in ["invalid type", "expected", "line ", "column", "serde", "Error(", "unwrap", "{"] {
            assert!(!m.contains(jargon), "{m:?} reads like a parser error");
        }
    }
}

#[tokio::test]
async fn with_pocket_make_off_a_made_pocket_cant_be_made() {
    let t = setup(false).await;
    let r = make(&t, spec()).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::NOT_FOUND, Some("pocket_apps_off")), "{}", r.body);
}
