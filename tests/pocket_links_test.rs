//! pa1b host and screen links (Pocket API v1.1): the remote and the big screen sign in with
//! one-time tokens and never sign each other out; a screen only watches. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa1b/brief.md ("Done when").

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use futures::{SinkExt, StreamExt};
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
const REMOTE: &str = "203.0.113.9:50000";
const GHOST: &str = "pocket.example.net";

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
            let (k, v) = c.split(';').next()?.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    }

    fn error(&self) -> (StatusCode, Option<&str>) {
        (self.status, self.body["error"].as_str())
    }
}

#[derive(Default, Clone)]
struct Req<'a> {
    cookies: Vec<(String, String)>,
    bearer: Option<&'a str>,
    ip: Option<&'a str>,
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
    if let Some(ip) = r.ip {
        req = req.header("x-forwarded-for", ip);
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

fn local() -> Req<'static> {
    Req { peer: Some(LOCAL), ..Default::default() }
}

/// The `kph_<code>` cookie, as a browser that signed in sends it.
fn host_cookie(code: &str, key: &str) -> Req<'static> {
    Req { cookies: vec![(guest::host_cookie(code), key.to_string())], ..Default::default() }
}

async fn make(t: &T) -> (String, String) {
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "test", "spec": { "title": "Quiz night" } })), &local()).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (r.body["pocket"]["code"].as_str().unwrap().to_string(), r.body["host_key"].as_str().unwrap().to_string())
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

/// `POST /api/pockets/:code/link` from the person's card.
async fn mint(t: &T, code: &str, role: &str) -> Res {
    send(&t.main, "POST", &format!("/api/pockets/{code}/link"), Some(json!({ "role": role })), &local()).await
}

async fn token(t: &T, code: &str, role: &str) -> String {
    let r = mint(t, code, role).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["token"].as_str().unwrap().to_string()
}

async fn signin(t: &T, code: &str, body: Value) -> Res {
    let ip = fresh_ip();
    send(&t.guest, "POST", &format!("/api/pocket/{code}/host/signin"), Some(body), &Req { ip: Some(ip.as_str()), ..Default::default() }).await
}

/// Signs in with a fresh link of `role`; the session key from the cookie.
async fn linked(t: &T, code: &str, role: &str) -> String {
    let tok = token(t, code, role).await;
    let r = signin(t, code, json!({ "token": tok })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["viewer"], role);
    let session = r.cookie(&guest::host_cookie(code)).expect("the sign-in sets the host cookie");
    assert_ne!(session, tok, "the cookie holds a new session key, not the spent token");
    session
}

async fn host_state(t: &T, code: &str, key: &str, view: Option<&str>) -> Res {
    let q = view.map(|v| format!("?view={v}")).unwrap_or_default();
    send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state{q}"), None, &host_cookie(code, key)).await
}

async fn host_action(t: &T, code: &str, key: &str, name: &str) -> Res {
    send(&t.guest, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": name })), &host_cookie(code, key)).await
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A host socket with the `kph_<code>` cookie, or the HTTP status it was refused with.
async fn ws_try(addr: SocketAddr, code: &str, key: &str, view: &str) -> Result<Ws, StatusCode> {
    let mut req = format!("ws://{addr}/api/pocket/{code}/ws?view={view}").into_client_request().unwrap();
    req.headers_mut().insert("origin", format!("http://{addr}").parse().unwrap());
    req.headers_mut().insert("cookie", format!("{}={key}", guest::host_cookie(code)).parse().unwrap());
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => Err(res.status()),
        Err(e) => panic!("ws: {e}"),
    }
}

/// The next text frame (with its `type`), skipping pings.
async fn next_frame(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a message within 5 s").unwrap().unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

// ---- The bug: a second device signed the first one out ----

#[tokio::test]
async fn a_screen_link_after_the_remote_signed_in_leaves_the_remote_working() {
    let t = setup().await;
    let addr = t.state.pocket.listener.addr().unwrap();
    let (code, host_key) = make(&t).await;
    // The remote signed in with the host key (v1) and holds a live socket
    let r = signin(&t, &code, json!({ "key": host_key })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let remote = r.cookie(&guest::host_cookie(&code)).unwrap();
    let mut remote_ws = ws_try(addr, &code, &remote, "host").await.unwrap();
    assert_eq!(next_frame(&mut remote_ws).await["type"], "state");
    // A second remote from a host link (v1.1)
    let remote2 = linked(&t, &code, "host").await;
    // "Big screen" on the card: a screen link, then the TV signs in
    let screen = linked(&t, &code, "screen").await;
    let mut screen_ws = ws_try(addr, &code, &screen, "screen").await.unwrap();
    assert_eq!(next_frame(&mut screen_ws).await["viewer"], "screen");
    // "Open the remote" again after a reload: one more host link; the TV stays too
    let remote3 = linked(&t, &code, "host").await;
    for key in [&host_key, &remote, &remote2, &remote3] {
        assert_eq!(host_state(&t, &code, key, None).await.status, StatusCode::OK, "every remote is still signed in");
    }
    assert_eq!(host_state(&t, &code, &screen, Some("screen")).await.status, StatusCode::OK, "the screen is still signed in");
    // The first remote still runs the game, and its socket and the screen's hear the change
    let r = host_action(&t, &code, &remote, "advance").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let f = next_frame(&mut remote_ws).await;
    assert_eq!((f["type"].as_str(), f["view"]["round"].as_i64()), (Some("state"), Some(1)), "no signed_out frame: {f}");
    let f = next_frame(&mut screen_ws).await;
    assert_eq!((f["type"].as_str(), f["viewer"].as_str()), (Some("state"), Some("screen")), "{f}");
    let _ = remote_ws.send(Message::Close(None)).await;
    let _ = screen_ws.send(Message::Close(None)).await;
}

// ---- Roles ----

#[tokio::test]
async fn a_screen_session_only_watches() {
    let t = setup().await;
    let addr = t.state.pocket.listener.addr().unwrap();
    let (code, _) = make(&t).await;
    let screen = linked(&t, &code, "screen").await;
    let r = host_state(&t, &code, &screen, Some("screen")).await;
    assert_eq!((r.status, r.body["viewer"].as_str()), (StatusCode::OK, Some("screen")));
    for view in [None, Some("host")] {
        let r = host_state(&t, &code, &screen, view).await;
        assert_eq!(r.error(), (StatusCode::FORBIDDEN, Some("screen_only")), "view {view:?}");
        assert_eq!(r.body["message"], "This screen can only show the game.");
    }
    let writes: [(&str, Value); 4] = [
        ("action", json!({ "name": "advance" })),
        ("remove", json!({ "player_id": "pl_nobody" })),
        ("reset-pin", json!({ "player_id": "pl_nobody" })),
        ("end", json!({})),
    ];
    for (path, body) in writes {
        let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/{path}"), Some(body.clone()), &host_cookie(&code, &screen)).await;
        assert_eq!(r.error(), (StatusCode::FORBIDDEN, Some("screen_only")), "host/{path}");
        // As a bearer too
        let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/{path}"), Some(body), &Req { bearer: Some(screen.as_str()), ..Default::default() }).await;
        assert_eq!(r.error(), (StatusCode::FORBIDDEN, Some("screen_only")), "host/{path} with a bearer");
    }
    assert_eq!(ws_try(addr, &code, &screen, "host").await.err(), Some(StatusCode::FORBIDDEN), "ws?view=host");
    let mut ws = ws_try(addr, &code, &screen, "screen").await.expect("ws?view=screen");
    assert_eq!(next_frame(&mut ws).await["viewer"], "screen");
    let _ = ws.send(Message::Close(None)).await;
    // Nothing the screen tried changed the pocket
    let s = t.state.db.pocket_get(&code).await.unwrap().unwrap();
    assert_eq!((s.state.as_str(), s.version), ("open", 1));
}

#[tokio::test]
async fn a_host_link_runs_the_game() {
    let t = setup().await;
    let addr = t.state.pocket.listener.addr().unwrap();
    let (code, _) = make(&t).await;
    let remote = linked(&t, &code, "host").await;
    let r = host_state(&t, &code, &remote, None).await;
    assert_eq!((r.status, r.body["viewer"].as_str()), (StatusCode::OK, Some("host")));
    assert_eq!(host_state(&t, &code, &remote, Some("screen")).await.status, StatusCode::OK, "a remote may show the screen view too");
    let r = host_action(&t, &code, &remote, "advance").await;
    assert_eq!((r.status, r.body["view"]["round"].as_i64()), (StatusCode::OK, Some(1)), "{}", r.body);
    // start/next/pause are the quiz kit's names: the link passes them to the kit, which answers
    // for itself (the test kit doesn't know them), never 401/403
    for name in ["start", "next", "pause"] {
        let r = host_action(&t, &code, &remote, name).await;
        assert!(r.status != StatusCode::UNAUTHORIZED && r.status != StatusCode::FORBIDDEN, "{name}: {} {}", r.status, r.body);
    }
    let mut ws = ws_try(addr, &code, &remote, "host").await.expect("ws?view=host");
    assert_eq!(next_frame(&mut ws).await["viewer"], "host");
    let _ = ws.send(Message::Close(None)).await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/end"), Some(json!({})), &host_cookie(&code, &remote)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
}

// ---- Tokens ----

#[tokio::test]
async fn a_token_works_once_and_not_after_ten_minutes() {
    let t = setup().await;
    let (code, _) = make(&t).await;
    let r = mint(&t, &code, "screen").await;
    assert_eq!((r.status, r.body["role"].as_str()), (StatusCode::OK, Some("screen")));
    let tok = r.body["token"].as_str().unwrap().to_string();
    assert_eq!(tok.len(), 43);
    let expires = chrono::DateTime::parse_from_rfc3339(r.body["expires_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    let left = expires - Utc::now();
    assert!(left <= Duration::minutes(10) && left > Duration::minutes(9), "{left}");
    // Once
    assert_eq!(signin(&t, &code, json!({ "token": tok })).await.status, StatusCode::OK);
    let r = signin(&t, &code, json!({ "token": tok })).await;
    assert_eq!(r.error(), (StatusCode::UNAUTHORIZED, Some("link_used_or_expired")));
    assert_eq!(r.body["message"], "This link was already used or is too old. Open a new one from the card.");
    // 10 minutes
    let now = Utc::now();
    let (late, _) = pocket::mint_link(&t.state, &code, pocket::HostRole::Host, now).await.unwrap();
    let err = pocket::spend_link(&t.state, &code, &late, now + Duration::minutes(10)).await.unwrap_err();
    assert_eq!(err.code, "link_used_or_expired");
    let (fresh, _) = pocket::mint_link(&t.state, &code, pocket::HostRole::Host, now).await.unwrap();
    let (role, _) = pocket::spend_link(&t.state, &code, &fresh, now + Duration::minutes(9)).await.unwrap();
    assert_eq!(role, pocket::HostRole::Host);
    // The host key is not a token, and a made-up token is nobody
    for body in [json!({ "token": "x".repeat(43) }), json!({ "token": keys_like() })] {
        assert_eq!(signin(&t, &code, body).await.error(), (StatusCode::UNAUTHORIZED, Some("link_used_or_expired")));
    }
}

fn keys_like() -> String {
    kwnt::pocket::keys::new_key()
}

#[tokio::test]
async fn a_token_from_one_pocket_is_refused_on_another() {
    let t = setup().await;
    let (a, _) = make(&t).await;
    let (b, _) = make(&t).await;
    let tok = token(&t, &a, "host").await;
    let r = signin(&t, &b, json!({ "token": tok })).await;
    assert_eq!(r.error(), (StatusCode::UNAUTHORIZED, Some("link_used_or_expired")));
    // The failed try didn't spend it on A
    let r = signin(&t, &a, json!({ "token": tok })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let session = r.cookie(&guest::host_cookie(&a)).unwrap();
    // A's session opens nothing on B
    assert_eq!(host_state(&t, &b, &session, None).await.error(), (StatusCode::UNAUTHORIZED, Some("not_host")));
    let r = send(&t.guest, "GET", &format!("/api/pocket/{b}/host/state"), None, &Req { bearer: Some(session.as_str()), ..Default::default() }).await;
    assert_eq!(r.error(), (StatusCode::UNAUTHORIZED, Some("not_host")));
}

#[tokio::test]
async fn bad_requests_and_who_may_mint() {
    let t = setup().await;
    let (code, host_key) = make(&t).await;
    for role in ["admin", "", "HOST"] {
        assert_eq!(mint(&t, &code, role).await.error(), (StatusCode::BAD_REQUEST, Some("bad_role")), "{role:?}");
    }
    assert_eq!(mint(&t, "zzzzzz", "host").await.status, StatusCode::NOT_FOUND);
    assert_eq!(signin(&t, &code, json!({})).await.error(), (StatusCode::BAD_REQUEST, Some("no_key")));
    // v1 still works, and a wrong key is still not_host
    assert_eq!(signin(&t, &code, json!({ "key": host_key })).await.status, StatusCode::OK);
    assert_eq!(signin(&t, &code, json!({ "key": "nope" })).await.error(), (StatusCode::UNAUTHORIZED, Some("not_host")));
    // Person only: from afar without the person's token, or with a pocket key, nobody mints
    let uri = format!("/api/pockets/{code}/link");
    let body = Some(json!({ "role": "host" }));
    let r = send(&t.main, "POST", &uri, body.clone(), &Req { peer: Some(REMOTE), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(&t.main, "POST", &uri, body.clone(), &Req { peer: Some(REMOTE), bearer: Some(host_key.as_str()), ..Default::default() }).await;
    assert!(!r.status.is_success(), "{}", r.status);
    let r = send(&t.main, "POST", &uri, body.clone(), &Req { peer: Some(REMOTE), bearer: Some(TOKEN), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    // The guest port has no minting route
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/link"), body, &Req { bearer: Some(host_key.as_str()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn token_signins_share_the_signin_limit() {
    let t = setup().await;
    let (code, _) = make(&t).await;
    let ip = "192.0.2.77";
    let mut statuses = Vec::new();
    for _ in 0..40 {
        let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/signin"), Some(json!({ "token": "nope" })), &Req { ip: Some(ip), ..Default::default() }).await;
        statuses.push(r.status);
    }
    assert!(statuses[..30].iter().all(|s| *s == StatusCode::UNAUTHORIZED), "{statuses:?}");
    assert!(statuses[30..].contains(&StatusCode::TOO_MANY_REQUESTS), "{statuses:?}");
}

// ---- Rotation, limits, retention, privacy ----

#[tokio::test]
async fn rotating_the_host_key_drops_host_links_and_keeps_the_screen() {
    let t = setup().await;
    let addr = t.state.pocket.listener.addr().unwrap();
    let (code, _) = make(&t).await;
    let remote = linked(&t, &code, "host").await;
    let screen = linked(&t, &code, "screen").await;
    let unspent_host = token(&t, &code, "host").await;
    let unspent_screen = token(&t, &code, "screen").await;
    let mut remote_ws = ws_try(addr, &code, &remote, "host").await.unwrap();
    let mut screen_ws = ws_try(addr, &code, &screen, "screen").await.unwrap();
    next_frame(&mut remote_ws).await;
    next_frame(&mut screen_ws).await;
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/host-key"), None, &local()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let new_key = r.body["host_key"].as_str().unwrap().to_string();
    assert_eq!(next_frame(&mut remote_ws).await, json!({ "type": "signed_out" }));
    assert_eq!(host_state(&t, &code, &remote, None).await.error(), (StatusCode::UNAUTHORIZED, Some("not_host")));
    assert_eq!(signin(&t, &code, json!({ "token": unspent_host })).await.error(), (StatusCode::UNAUTHORIZED, Some("link_used_or_expired")));
    // The screen stays signed in, its socket stays open, and its unspent link still works
    assert_eq!(host_state(&t, &code, &screen, Some("screen")).await.status, StatusCode::OK);
    assert_eq!(host_action(&t, &code, &new_key, "advance").await.status, StatusCode::OK);
    assert_eq!(next_frame(&mut screen_ws).await["type"], "state");
    assert_eq!(signin(&t, &code, json!({ "token": unspent_screen })).await.status, StatusCode::OK);
    let _ = screen_ws.send(Message::Close(None)).await;
}

#[tokio::test]
async fn the_newest_five_sessions_per_role_stay() {
    let t = setup().await;
    let (code, _) = make(&t).await;
    let mut screens = Vec::new();
    for _ in 0..6 {
        screens.push(linked(&t, &code, "screen").await);
    }
    let remote = linked(&t, &code, "host").await;
    assert_eq!(host_state(&t, &code, &screens[0], Some("screen")).await.status, StatusCode::UNAUTHORIZED, "the oldest screen went");
    for s in &screens[1..] {
        assert_eq!(host_state(&t, &code, s, Some("screen")).await.status, StatusCode::OK);
    }
    assert_eq!(host_state(&t, &code, &remote, None).await.status, StatusCode::OK, "screens never push out a remote");
}

#[tokio::test]
async fn deleting_the_pocket_deletes_its_links() {
    let t = setup().await;
    let (code, _) = make(&t).await;
    linked(&t, &code, "screen").await;
    token(&t, &code, "host").await;
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [1, 0, 2, 0], "a session and a token count as keys");
    let r = send(&t.main, "DELETE", &format!("/api/pockets/{code}"), None, &local()).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0]);
}

#[tokio::test]
async fn a_screen_session_sees_nothing_private() {
    let t = setup().await;
    let (code, _) = make(&t).await;
    let ip = fresh_ip();
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": "Alice" })), &Req { ip: Some(ip.as_str()), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let alice = r.cookie(&guest::guest_cookie(&code)).unwrap();
    const SECRET: &str = "pear-tree-42";
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "set_secret", "body": { "secret": SECRET } })), &Req { cookies: vec![(guest::guest_cookie(&code), alice)], ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK);
    let tok = token(&t, &code, "screen").await;
    let r = signin(&t, &code, json!({ "token": tok })).await;
    let session = r.cookie(&guest::host_cookie(&code)).unwrap();
    let poll = host_state(&t, &code, &session, Some("screen")).await.body;
    for (label, v) in [("sign-in", &r.body), ("poll", &poll)] {
        let s = v.to_string();
        // "key" is checked on field names only: a random code or player id can spell "key" by chance.
        assert!(!s.contains(SECRET) && !s.contains("pin_hash") && !s.contains("salt") && !field_names(v).iter().any(|n| n.contains("key")) && !s.contains(&tok), "{label}: {s}");
    }
}

/// Every object field name in `v`, at any depth.
fn field_names(v: &Value) -> Vec<String> {
    match v {
        Value::Object(m) => m.iter().flat_map(|(k, v)| std::iter::once(k.clone()).chain(field_names(v))).collect(),
        Value::Array(a) => a.iter().flat_map(field_names).collect(),
        _ => vec![],
    }
}
