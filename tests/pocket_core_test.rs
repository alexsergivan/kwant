//! pa1 Pocket core: the guest port, keys, PINs, limits, the sweeper and the switch. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa1/brief.md ("Done when").

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
    pocket::{self, guest, keys, listener, CreateRequest},
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

async fn setup(on: bool) -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    if on {
        state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    }
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), guest: guest::router(state.clone()), state, _tmp: tmp }
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Res {
    /// The value a `Set-Cookie` gives `name`.
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
    ip: Option<&'a str>,
    origin: Option<&'a str>,
    peer: Option<&'a str>,
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>, r: &Req<'_>) -> Res {
    // The main port is reached as this computer (loopback peer and name) or from afar; the guest
    // port through its public name
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
    if let Some(o) = r.origin {
        req = req.header("origin", o);
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

fn guest_req(code: &str, key: &str) -> Req<'static> {
    Req { cookies: vec![(guest::guest_cookie(code), key.to_string())], ..Default::default() }
}

fn host_req(key: &str) -> Req<'_> {
    Req { bearer: Some(key), ..Default::default() }
}

/// A pocket made through the person's API on the main port: (code, host key).
async fn make(t: &T, spec: Value) -> (String, String) {
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "test", "spec": spec })), &Req { peer: Some(LOCAL), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (r.body["pocket"]["code"].as_str().unwrap().to_string(), r.body["host_key"].as_str().unwrap().to_string())
}

static NEXT_IP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// Each guest from its own address, so the per-address sign-in limit isn't what's tested.
fn fresh_ip() -> String {
    let n = NEXT_IP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("198.51.{}.{}", (n >> 8) & 255, n & 255)
}

async fn join(t: &T, code: &str, body: Value) -> Res {
    let ip = fresh_ip();
    send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(body), &Req { ip: Some(ip.as_str()), ..Default::default() }).await
}

/// Joins and returns (player id, guest key).
async fn joined(t: &T, code: &str, name: &str) -> (String, String) {
    let r = join(t, code, json!({ "name": name })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    (r.body["me"]["id"].as_str().unwrap().to_string(), r.cookie(&guest::guest_cookie(code)).unwrap())
}

async fn rejoin(t: &T, code: &str, body: Value) -> Res {
    let ip = fresh_ip();
    send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(body), &Req { ip: Some(ip.as_str()), ..Default::default() }).await
}

async fn guest_state(t: &T, code: &str, key: &str) -> Res {
    send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(code, key)).await
}

// ---- Every main-port route refuses pocket keys ----

/// Every route path the main router registers, collected from the source at test time so routes
/// added later are walked too: `.route("…")` and `"/api/…"` literals, parameters filled in.
fn main_routes() -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("rs") || p.starts_with(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pocket/guest.rs")) {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            for marker in [".route(\"", "\"/api/"] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(marker) {
                    // Start at the path's leading slash
                    let start = if marker.starts_with('"') { i + 1 } else { i + marker.len() };
                    let tail = &rest[start..];
                    let end = tail.find('"').unwrap_or(0);
                    let lit = &tail[..end];
                    rest = &rest[start + end.max(1)..];
                    if !lit.starts_with('/') || lit.contains(char::is_whitespace) || lit.contains('\\') {
                        continue;
                    }
                    let lit = lit.split('?').next().unwrap_or(lit);
                    let path: Vec<String> = lit
                        .split('/')
                        .map(|s| if s.starts_with(':') || s.starts_with('*') || s.contains('{') { "x".to_string() } else { s.to_string() })
                        .collect();
                    let path = path.join("/");
                    if path.chars().all(|c| c.is_ascii_alphanumeric() || "/-_.~".contains(c)) {
                        out.insert(path);
                    }
                }
            }
        }
    }
    out.into_iter().collect()
}

#[tokio::test]
async fn main_port_routes_never_take_a_pocket_key() {
    let t = setup(true).await;
    let (code, host_key) = make(&t, json!({ "title": "Walk" })).await;
    let (_, guest_key) = joined(&t, &code, "Mia").await;
    let routes = main_routes();
    assert!(routes.len() > 100, "found only {} routes", routes.len());
    for p in ["/api/pockets", "/api/pockets/x", "/api/pockets/x/end", "/api/pockets/x/host-key", "/api/pockets/x/link", "/api/pockets/x/tunnel", "/api/health"] {
        assert!(routes.iter().any(|r| r == p), "the walk misses {p}");
    }
    let creds: Vec<(&str, Req)> = vec![
        ("guest cookie", Req { cookies: vec![(guest::guest_cookie(&code), guest_key.clone())], ..Default::default() }),
        ("host cookie", Req { cookies: vec![(guest::host_cookie(&code), host_key.clone())], ..Default::default() }),
        ("guest key as kwnt_token", Req { cookies: vec![("kwnt_token".into(), guest_key.clone())], ..Default::default() }),
        ("host key as kwnt_token", Req { cookies: vec![("kwnt_token".into(), host_key.clone())], ..Default::default() }),
        ("guest key as bearer", Req { bearer: Some(guest_key.as_str()), ..Default::default() }),
        ("host key as bearer", Req { bearer: Some(host_key.as_str()), ..Default::default() }),
    ];
    let mut walked = 0;
    for route in &routes {
        let route = route.replace("/pockets/x", &format!("/pockets/{code}"));
        for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
            let body = (method != "GET" && method != "DELETE").then(|| json!({}));
            let base = send(&t.main, method, &route, body.clone(), &Req { peer: Some(REMOTE), ..Default::default() }).await;
            if base.status.is_success() {
                // Public without any key (health, sign-in, the app's pages): a key changes nothing
                continue;
            }
            for (label, c) in &creds {
                let r = send(&t.main, method, &route, body.clone(), &Req { peer: Some(REMOTE), ..c.clone() }).await;
                assert!(!r.status.is_success(), "{method} {route} with a {label} answered {}", r.status);
                walked += 1;
            }
        }
    }
    assert!(walked > 500, "walked only {walked}");
    // From this computer the person needs no token at all; a pocket key there changes nothing
    for path in ["/api/pockets".to_string(), format!("/api/pockets/{code}"), "/api/auth/status".to_string()] {
        let plain = send(&t.main, "GET", &path, None, &Req { peer: Some(LOCAL), ..Default::default() }).await;
        for key in [&guest_key, &host_key] {
            let r = send(&t.main, "GET", &path, None, &Req { peer: Some(LOCAL), bearer: Some(key.as_str()), ..Default::default() }).await;
            assert!(r.status == plain.status || !r.status.is_success(), "GET {path} from loopback with a pocket bearer: {}", r.status);
        }
    }
    // The pocket is untouched by all of that
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(&host_key)).await;
    assert_eq!(r.status, StatusCode::OK, "the host key still works on the guest port");
}

#[tokio::test]
async fn the_person_api_lists_reads_ends_and_deletes() {
    let t = setup(true).await;
    let local = Req { peer: Some(LOCAL), ..Default::default() };
    let (code, _) = make(&t, json!({ "title": "Quiz night", "days": 7 })).await;
    joined(&t, &code, "Mia").await;
    let r = send(&t.main, "GET", "/api/pockets", None, &local).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["enabled"], true);
    assert_eq!(r.body["pockets"][0]["code"], code.as_str());
    assert_eq!(r.body["pockets"][0]["players"], 1);
    assert_eq!(r.body["pockets"][0]["path"], format!("/p/{code}"));
    assert!(!r.body.to_string().contains("Mia"), "the list names no guest");
    // Remote callers need the person's token
    let r = send(&t.main, "GET", "/api/pockets", None, &Req { peer: Some(REMOTE), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(&t.main, "GET", "/api/pockets", None, &Req { peer: Some(REMOTE), bearer: Some(TOKEN), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::OK);
    // Bad requests
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "nope", "spec": {} })), &local).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_kit")));
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "test", "spec": {} })), &local).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_spec")));
    // Read, end
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &local).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["summary"]["round"], 0);
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/end"), None, &local).await;
    assert_eq!(r.body["pocket"]["state"], "ended");
    let ended = chrono::DateTime::parse_from_rfc3339(r.body["pocket"]["ended_at"].as_str().unwrap()).unwrap();
    let expires = chrono::DateTime::parse_from_rfc3339(r.body["pocket"]["expires_at"].as_str().unwrap()).unwrap();
    assert_eq!(expires - ended, Duration::days(7));
    let r = join(&t, &code, json!({ "name": "Late" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::GONE, Some("ended")));
    // Delete is final
    let r = send(&t.main, "DELETE", &format!("/api/pockets/{code}"), None, &local).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap(), [0, 0, 0, 0]);
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &local).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

// ---- The guest port ----

#[tokio::test]
async fn the_guest_port_answers_only_its_routes_and_one_pocket_per_key() {
    let t = setup(true).await;
    let (a, host_a) = make(&t, json!({ "title": "A <b>" })).await;
    let (b, host_b) = make(&t, json!({ "title": "B" })).await;
    let (_, key_a) = joined(&t, &a, "Mia").await;
    let none = Req::default();
    let r = send(&t.guest, "GET", &format!("/p/{a}"), None, &none).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers["content-security-policy"], guest::CSP);
    let html = r.body.as_str().unwrap();
    if guest::has_page() {
        assert!(html.contains(r#"id="pocket-boot">{"#) && !html.contains("A <b>"), "{html}");
    } else {
        assert!(html.contains("A &lt;b&gt;") && !html.contains("<script") && !html.contains("style="), "{html}");
    }
    for (method, path) in [
        ("GET", "/".to_string()),
        ("GET", "/index.html".into()),
        ("GET", "/api/health".into()),
        ("GET", "/api/pockets".into()),
        ("GET", "/api/auth/status".into()),
        ("GET", "/p/zzzzzz".into()),
        ("GET", "/p/NOTACODE".into()),
        ("GET", "/p/_a".into()),
        ("GET", "/p/_a/".into()),
        ("GET", "/p/_a/index.html".into()),
        ("GET", "/p/_a/..%2Fx".into()),
        ("GET", "/p/_a/..%2F..%2Findex.html".into()),
        ("GET", "/p/_a/g-nosuchfile.js".into()),
        ("POST", "/p/_a/g-nosuchfile.js".into()),
        ("GET", format!("/p/{a}/x")),
        ("GET", format!("/api/pocket/{a}")),
        ("GET", format!("/api/pocket/{a}/nope")),
        ("DELETE", format!("/api/pocket/{a}/join")),
        ("GET", format!("/api/pocket/{a}/join")),
        ("GET", "/api/pocket/zzzzzz/info".into()),
        ("POST", "/api/pockets".into()),
    ] {
        let r = send(&t.guest, method, &path, None, &none).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(r.headers["content-security-policy"], guest::CSP);
    }
    // A's guest key is nobody in B, under either pocket's cookie name
    assert_eq!(guest_state(&t, &a, &key_a).await.status, StatusCode::OK);
    assert_eq!(guest_state(&t, &b, &key_a).await.status, StatusCode::UNAUTHORIZED);
    let r = send(&t.guest, "GET", &format!("/api/pocket/{b}/state"), None, &Req { cookies: vec![(guest::guest_cookie(&a), key_a.clone())], ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(&t.guest, "POST", &format!("/api/pocket/{b}/action"), Some(json!({ "name": "tap" })), &guest_req(&b, &key_a)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // A's host key is nobody in B, and a guest key is no host
    for (code, key) in [(&b, &host_a), (&a, &host_b), (&a, &key_a)] {
        let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(key)).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
        let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": "advance" })), &host_req(key)).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // The host's remote trades its link key for the host cookie
    let r = send(&t.guest, "POST", &format!("/api/pocket/{a}/host/signin"), Some(json!({ "key": host_a })), &none).await;
    assert_eq!(r.status, StatusCode::OK);
    let host_cookie = r.cookie(&guest::host_cookie(&a)).unwrap();
    let set = r.headers["set-cookie"].to_str().unwrap();
    assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains(&format!("Path=/api/pocket/{a}")), "{set}");
    let r = send(&t.guest, "GET", &format!("/api/pocket/{a}/host/state"), None, &Req { cookies: vec![(guest::host_cookie(&a), host_cookie)], ..Default::default() }).await;
    assert_eq!((r.status, r.body["viewer"].as_str()), (StatusCode::OK, Some("host")));
    // Another site's page can't write
    let r = send(&t.guest, "POST", &format!("/api/pocket/{a}/join"), Some(json!({ "name": "Evil" })), &Req { origin: Some("https://evil.example"), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let origin = format!("https://{GHOST}");
    let r = send(&t.guest, "POST", &format!("/api/pocket/{a}/join"), Some(json!({ "name": "Ok" })), &Req { origin: Some(origin.as_str()), ip: Some("198.18.0.1"), ..Default::default() }).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
}

#[tokio::test]
async fn join_and_rejoin_with_the_cookie_and_the_name_filter() {
    let t = setup(true).await;
    let (code, _) = make(&t, json!({ "title": "Join" })).await;
    let r = join(&t, &code, json!({ "name": "  Mia ", "avatar": "fox" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    assert_eq!(r.body["me"]["name"], "Mia");
    assert_eq!(r.body["api"], "pocket/1");
    let set = r.headers["set-cookie"].to_str().unwrap().to_string();
    assert!(set.starts_with(&format!("kp_{code}=")) && set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains(&format!("Path=/api/pocket/{code}")), "{set}");
    let key = r.cookie(&guest::guest_cookie(&code)).unwrap();
    assert_eq!(key.len(), 43, "32 random bytes");
    let id = r.body["me"]["id"].as_str().unwrap().to_string();
    // Only the hash is kept
    assert!(t.state.db.pocket_key_player(&code, &key).await.unwrap().is_none());
    assert!(t.state.db.pocket_key_player(&code, &keys::hash_key(&key)).await.unwrap().is_some());
    let r = guest_state(&t, &code, &key).await;
    assert_eq!((r.status, r.body["me"]["id"].as_str()), (StatusCode::OK, Some(id.as_str())));
    // Joining again from the same phone is the same player
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/join"), Some(json!({ "name": "Someone else" })), &guest_req(&code, &key)).await;
    assert_eq!((r.status, r.body["me"]["id"].as_str()), (StatusCode::OK, Some(id.as_str())));
    assert_eq!(r.body["pocket"]["players"], 1);
    // The name is taken (any case); rejoining by name gives a second key, both work
    let r = join(&t, &code, json!({ "name": "MIA" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("name_taken")));
    let r = rejoin(&t, &code, json!({ "name": "mia" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let key2 = r.cookie(&guest::guest_cookie(&code)).unwrap();
    assert_ne!(key, key2);
    assert_eq!(guest_state(&t, &code, &key).await.status, StatusCode::OK);
    assert_eq!(guest_state(&t, &code, &key2).await.body["me"]["id"], id.as_str());
    let r = rejoin(&t, &code, json!({ "name": "Nobody" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::NOT_FOUND, Some("no_such_name")));
    // No cookie, no state
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &Req::default()).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("signed_out")));
    // The name filter
    for (name, err) in [("", "name_empty"), ("   ", "name_empty"), ("abcdefghijklmnopqrstu", "name_too_long"), ("fuckface", "name_not_allowed"), ("www.spam.io", "name_not_allowed"), ("sh1t", "name_not_allowed")] {
        let r = join(&t, &code, json!({ "name": name })).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some(err)), "{name:?}");
    }
    let r = join(&t, &code, json!({ "name": "Ok", "team": "big dick energy" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_team")));
    let r = join(&t, &code, json!({ "name": "Ok", "avatar": "<img>" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_avatar")));
    assert_eq!(join(&t, &code, json!({ "name": "Scunthorpe" })).await.status, StatusCode::CREATED);
}

#[tokio::test]
async fn the_host_removes_a_player() {
    let t = setup(true).await;
    let (code, host) = make(&t, json!({ "title": "Remove" })).await;
    let (id, key) = joined(&t, &code, "Spammer").await;
    joined(&t, &code, "Mia").await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/remove"), Some(json!({ "player_id": id })), &host_req(&host)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["pocket"]["players"], 1);
    assert_eq!(guest_state(&t, &code, &key).await.status, StatusCode::UNAUTHORIZED);
    let r = rejoin(&t, &code, json!({ "name": "Spammer" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::FORBIDDEN, Some("removed")));
    let r = join(&t, &code, json!({ "name": "spammer" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::FORBIDDEN, Some("removed")));
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/remove"), Some(json!({ "player_id": id })), &host_req(&host)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // A guest can't remove anyone
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/remove"), Some(json!({ "player_id": id })), &guest_req(&code, &key)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sixty_players_then_full() {
    let t = setup(true).await;
    let (code, _) = make(&t, json!({ "title": "Full", "max_players": 500 })).await;
    for i in 0..60 {
        joined(&t, &code, &format!("Player {i}")).await;
    }
    let r = join(&t, &code, json!({ "name": "Player 61" })).await;
    assert_eq!((r.status, r.body["error"].as_str(), r.body["max_players"].as_u64()), (StatusCode::CONFLICT, Some("full"), Some(60)), "a kit can't go past 60");
    let (small, _) = make(&t, json!({ "title": "Small", "max_players": 2 })).await;
    joined(&t, &small, "A").await;
    joined(&t, &small, "B").await;
    assert_eq!(join(&t, &small, json!({ "name": "C" })).await.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn signins_and_actions_are_limited() {
    let t = setup(true).await;
    let (code, _) = make(&t, json!({ "title": "Limits" })).await;
    // One address: a burst of 30 sign-in tries, then 429 (rejoin of a missing name costs a try too)
    let mut statuses = Vec::new();
    for _ in 0..32 {
        statuses.push(send(&t.guest, "POST", &format!("/api/pocket/{code}/rejoin"), Some(json!({ "name": "x" })), &Req { ip: Some("192.0.2.7"), ..Default::default() }).await.status);
    }
    assert!(statuses[..30].iter().all(|s| *s == StatusCode::NOT_FOUND), "{statuses:?}");
    assert_eq!(statuses[31], StatusCode::TOO_MANY_REQUESTS);
    // Actions: 10 a second per guest
    let (_, key) = joined(&t, &code, "Tapper").await;
    let mut statuses = Vec::new();
    for _ in 0..25 {
        statuses.push(send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "tap" })), &guest_req(&code, &key)).await.status);
    }
    assert!(statuses[..10].iter().all(|s| *s == StatusCode::OK), "{statuses:?}");
    assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS), "{statuses:?}");
    // Named actions only, and only the ones for that side
    let (_, key2) = joined(&t, &code, "Other").await;
    for name in ["advance", "nope", ""] {
        let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": name })), &guest_req(&code, &key2)).await;
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_action")), "{name:?}");
    }
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "set_secret", "body": { "secret": "x".repeat(51) } })), &guest_req(&code, &key2)).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_action")));
    // Big bodies stop at 16 KB
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "tap", "body": { "pad": "x".repeat(20_000) } })), &guest_req(&code, &key2)).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
}

// ---- PINs ----

#[tokio::test]
async fn pin_lock_after_five_hard_lock_after_ten_and_the_reset_code() {
    let t = setup(true).await;
    let (code, host) = make(&t, json!({ "title": "Santa", "pin": true })).await;
    let r = join(&t, &code, json!({ "name": "Mia" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("pin_required")));
    let r = join(&t, &code, json!({ "name": "Mia", "pin": "12345" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = join(&t, &code, json!({ "name": "Mia", "pin": "4821" })).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let id = r.body["me"]["id"].as_str().unwrap().to_string();
    let first_key = r.cookie(&guest::guest_cookie(&code)).unwrap();
    let row = t.state.db.pocket_player(&code, &id).await.unwrap().unwrap();
    assert!(row.pin_hash.as_deref().is_some_and(|h| h.len() == 64 && !h.contains("4821")), "only an argon2 hash is kept");
    // 4 wrong tries: the page is told how many are left
    for left in [4, 3, 2, 1] {
        let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "0000" })).await;
        assert_eq!((r.status, r.body["error"].as_str(), r.body["tries_left"].as_i64()), (StatusCode::UNAUTHORIZED, Some("wrong_pin"), Some(left)));
    }
    // The signed-in phone hears about it
    let r = guest_state(&t, &code, &first_key).await;
    assert_eq!(r.body["me"]["notices"][0]["kind"], "wrong_pin");
    // The 5th: a 10-minute wait, and even the right PIN waits
    let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "0000" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::LOCKED, Some("locked")));
    let until = chrono::DateTime::parse_from_rfc3339(r.body["until"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    assert!((until - Utc::now() - Duration::minutes(10)).num_seconds().abs() < 30, "{until}");
    let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "4821" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::LOCKED, Some("locked")));
    // 10 minutes later (the wait moved into the past)
    let mut c = after(&t, &code, &id).await;
    c.locked_until = Some(Utc::now() - Duration::seconds(1));
    t.state.db.pocket_set_pin_counters(&code, &id, &c).await.unwrap();
    for left in [4, 3, 2, 1] {
        let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "1111" })).await;
        assert_eq!((r.status, r.body["tries_left"].as_i64()), (StatusCode::UNAUTHORIZED, Some(left)));
    }
    // The 10th wrong try in total: locked until the host makes a reset code
    let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "1111" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::LOCKED, Some("hard_locked")));
    let mut c = after(&t, &code, &id).await;
    c.locked_until = None;
    t.state.db.pocket_set_pin_counters(&code, &id, &c).await.unwrap();
    let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "4821" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::LOCKED, Some("hard_locked")), "waiting doesn't open a hard lock");
    // The host's reset: a code once, 24 hours, nothing else about the player
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/reset-pin"), Some(json!({ "player_id": id })), &host_req(&host)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body.as_object().unwrap().len(), 2, "only the code and its expiry: {}", r.body);
    let reset = r.body["code"].as_str().unwrap().to_string();
    assert_eq!(reset.len(), 4);
    let expires = chrono::DateTime::parse_from_rfc3339(r.body["expires_at"].as_str().unwrap()).unwrap().with_timezone(&Utc);
    assert!((expires - Utc::now() - Duration::hours(24)).num_seconds().abs() < 30);
    // The old PIN and every key of the player stop working
    assert_eq!(guest_state(&t, &code, &first_key).await.status, StatusCode::UNAUTHORIZED);
    let r = rejoin(&t, &code, json!({ "name": "Mia", "pin": "4821" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("reset_pending")));
    // A wrong code counts as a wrong try
    let wrong = if reset == "AAAA" { "BBBB" } else { "AAAA" };
    let r = rejoin(&t, &code, json!({ "name": "Mia", "reset_code": wrong, "new_pin": "2468" })).await;
    assert_eq!((r.status, r.body["tries_left"].as_i64()), (StatusCode::UNAUTHORIZED, Some(4)));
    let r = rejoin(&t, &code, json!({ "name": "Mia", "reset_code": reset, "new_pin": "12" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("pin_required")));
    let r = rejoin(&t, &code, json!({ "name": "Mia", "reset_code": reset.to_lowercase(), "new_pin": "2468" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let kinds: Vec<&str> = r.body["notices"].as_array().unwrap().iter().filter_map(|n| n["kind"].as_str()).collect();
    assert_eq!(kinds, ["wrong_pin", "pin_reset"], "the next sign-in says someone tried a wrong PIN, and that the host reset it");
    let new_key = r.cookie(&guest::guest_cookie(&code)).unwrap();
    assert_eq!(guest_state(&t, &code, &new_key).await.body["me"]["notices"], json!([]));
    // Single use
    let r = rejoin(&t, &code, json!({ "name": "Mia", "reset_code": reset, "new_pin": "1357" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("no_reset")));
    assert_eq!(rejoin(&t, &code, json!({ "name": "Mia", "pin": "4821" })).await.status, StatusCode::UNAUTHORIZED, "the old PIN is gone");
    assert_eq!(rejoin(&t, &code, json!({ "name": "Mia", "pin": "2468" })).await.status, StatusCode::OK);
    // An unused code expires after 24 hours
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/host/reset-pin"), Some(json!({ "player_id": id })), &host_req(&host)).await;
    let reset = r.body["code"].as_str().unwrap().to_string();
    let salt = t.state.db.pocket_player(&code, &id).await.unwrap().unwrap().pin_salt.unwrap();
    let issued = Utc::now() - Duration::hours(25);
    t.state.db.pocket_pin_reset(&code, &id, &keys::hash_secret(&reset, &salt), issued + Duration::hours(24), issued).await.unwrap();
    let r = rejoin(&t, &code, json!({ "name": "Mia", "reset_code": reset, "new_pin": "1357" })).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::GONE, Some("reset_expired")));
    // A pocket without PINs has no resets
    let (plain, plain_host) = make(&t, json!({ "title": "Plain" })).await;
    let (pid, _) = joined(&t, &plain, "Jo").await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{plain}/host/reset-pin"), Some(json!({ "player_id": pid })), &host_req(&plain_host)).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
}

async fn after(t: &T, code: &str, id: &str) -> kwnt::db::pocket::PinCounters {
    let p = t.state.db.pocket_player(code, id).await.unwrap().unwrap();
    kwnt::db::pocket::PinCounters { wrong_tries: p.wrong_tries, total_wrong: p.total_wrong, locked_until: p.locked_until, pin_locked: p.pin_locked, last_wrong_at: p.last_wrong_at }
}

#[tokio::test]
async fn seen_notices_stop_showing_but_keep_the_counters() {
    let t = setup(true).await;
    let (code, _) = make(&t, json!({ "title": "Santa", "pin": true })).await;
    let r = join(&t, &code, json!({ "name": "Mia", "pin": "4821" })).await;
    let key = r.cookie(&guest::guest_cookie(&code)).unwrap();
    let id = r.body["me"]["id"].as_str().unwrap().to_string();
    for _ in 0..3 {
        rejoin(&t, &code, json!({ "name": "Mia", "pin": "0000" })).await;
    }
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/notices/seen"), None, &guest_req(&code, &key)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(guest_state(&t, &code, &key).await.body["me"]["notices"], json!([]));
    let c = after(&t, &code, &id).await;
    assert_eq!((c.wrong_tries, c.total_wrong), (3, 3), "seeing a notice gives a guesser no fresh tries");
}

// ---- Views: polling and WebSocket, and nothing private leaks ----

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

async fn next_state(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a message within 5 s").unwrap().unwrap();
        if let Message::Text(t) = msg {
            let mut v: Value = serde_json::from_str(&t).unwrap();
            if let Some(o) = v.as_object_mut() {
                if let Some(ty) = o.remove("type") {
                    assert!(ty == "state" || o.is_empty(), "{t}");
                }
            }
            return v;
        }
    }
}

/// The next text frame as sent (with its `type`), skipping pings.
async fn next_frame(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a message within 5 s").unwrap().unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

#[tokio::test]
async fn polling_and_websocket_deliver_the_same_state() {
    let t = setup(true).await;
    let addr = t.state.pocket.listener.addr().expect("the guest listener runs");
    let (code, host) = make(&t, json!({ "title": "Live" })).await;
    let (_, key) = joined(&t, &code, "Mia").await;
    let mut ws = ws_connect(addr, &code, Some((guest::guest_cookie(&code), key.clone())), None, None).await;
    let first = next_state(&mut ws).await;
    assert_eq!(first, guest_state(&t, &code, &key).await.body);
    // A tap, then the host's advance: each arrives over ws exactly as polling shows it
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "tap" })), &guest_req(&code, &key)).await;
    assert_eq!(r.status, StatusCode::OK);
    let pushed = next_state(&mut ws).await;
    assert_eq!(pushed["view"]["me"]["taps"], 1);
    assert_eq!(pushed, guest_state(&t, &code, &key).await.body);
    send(&t.guest, "POST", &format!("/api/pocket/{code}/host/action"), Some(json!({ "name": "advance" })), &host_req(&host)).await;
    let pushed = next_state(&mut ws).await;
    assert_eq!(pushed["view"]["round"], 1);
    let polled = guest_state(&t, &code, &key).await.body;
    assert_eq!(pushed, polled);
    // Polling with the version it has: 204, no body
    let v = polled["version"].as_i64().unwrap();
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/state?since={v}"), None, &guest_req(&code, &key)).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    // The real listener answers the same (and only its routes)
    let http = reqwest::Client::new();
    let r = http.get(format!("http://{addr}/api/pocket/{code}/state")).header("cookie", format!("kp_{code}={key}")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), polled);
    assert_eq!(http.get(format!("http://{addr}/api/health")).send().await.unwrap().status(), 404);
    // A WebSocket from another site, or without a key, doesn't open
    let mut req = format!("ws://{addr}/api/pocket/{code}/ws").into_client_request().unwrap();
    req.headers_mut().insert("origin", "https://evil.example".parse().unwrap());
    req.headers_mut().insert("cookie", format!("kp_{code}={key}").parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
    let mut req = format!("ws://{addr}/api/pocket/{code}/ws").into_client_request().unwrap();
    req.headers_mut().insert("origin", format!("http://{addr}").parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err());
    // Removed by the host: the socket says removed (not signed_out), so the page shows it at once
    let id = polled["me"]["id"].as_str().unwrap().to_string();
    send(&t.guest, "POST", &format!("/api/pocket/{code}/host/remove"), Some(json!({ "player_id": id })), &host_req(&host)).await;
    assert_eq!(next_frame(&mut ws).await, json!({ "type": "removed" }), "removed carries nothing but its type");
    // A PIN reset isn't a removal: that guest's socket says signed_out
    let (pin_code, pin_host) = make(&t, json!({ "title": "Santa", "pin": true })).await;
    let r = join(&t, &pin_code, json!({ "name": "Ola", "pin": "4821" })).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let (ola, ola_key) = (r.body["me"]["id"].as_str().unwrap().to_string(), r.cookie(&guest::guest_cookie(&pin_code)).unwrap());
    let mut ows = ws_connect(addr, &pin_code, Some((guest::guest_cookie(&pin_code), ola_key)), None, None).await;
    next_state(&mut ows).await;
    let r = send(&t.guest, "POST", &format!("/api/pocket/{pin_code}/host/reset-pin"), Some(json!({ "player_id": ola })), &host_req(&pin_host)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(next_frame(&mut ows).await, json!({ "type": "signed_out" }));
    // Deleted: a host socket hears gone
    let mut hws = ws_connect(addr, &code, None, Some(host.as_str()), Some("host")).await;
    let hstate = next_state(&mut hws).await;
    assert_eq!(hstate["viewer"], "host");
    let _ = ws.send(Message::Close(None)).await;
    pocket::delete(&t.state, &code).await.unwrap();
    let mut gone = false;
    while let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_secs(5), hws.next()).await {
        if let Message::Text(txt) = m {
            gone |= txt.contains("\"gone\"");
        }
    }
    assert!(gone);
}

#[tokio::test]
async fn no_private_field_reaches_another_guest_the_host_or_the_screen() {
    let t = setup(true).await;
    let addr = t.state.pocket.listener.addr().unwrap();
    let (code, host) = make(&t, json!({ "title": "Secrets" })).await;
    let (_, key_a) = joined(&t, &code, "Alice").await;
    let (_, key_b) = joined(&t, &code, "Bob").await;
    let mut ws_b = ws_connect(addr, &code, Some((guest::guest_cookie(&code), key_b.clone())), None, None).await;
    let mut ws_screen = ws_connect(addr, &code, None, Some(host.as_str()), Some("screen")).await;
    next_state(&mut ws_b).await;
    next_state(&mut ws_screen).await;
    const SECRET: &str = "pear-tree-42";
    let r = send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "set_secret", "body": { "secret": SECRET } })), &guest_req(&code, &key_a)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["view"]["me"]["secret"], SECRET, "the owner sees it");
    // A tap so every socket has something to send after the secret was set
    send(&t.guest, "POST", &format!("/api/pocket/{code}/action"), Some(json!({ "name": "tap" })), &guest_req(&code, &key_a)).await;
    let b_ws = next_state(&mut ws_b).await;
    let screen_ws = next_state(&mut ws_screen).await;
    let b_poll = guest_state(&t, &code, &key_b).await.body;
    let host_poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state"), None, &host_req(&host)).await.body;
    let screen_poll = send(&t.guest, "GET", &format!("/api/pocket/{code}/host/state?view=screen"), None, &host_req(&host)).await.body;
    let info = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), None, &Req::default()).await.body;
    let main_read = send(&t.main, "GET", &format!("/api/pockets/{code}"), None, &Req { peer: Some(LOCAL), ..Default::default() }).await.body;
    let main_list = send(&t.main, "GET", "/api/pockets", None, &Req { peer: Some(LOCAL), ..Default::default() }).await.body;
    for (label, v) in [("B over ws", &b_ws), ("B polling", &b_poll), ("screen over ws", &screen_ws), ("host", &host_poll), ("screen", &screen_poll), ("info", &info), ("main read", &main_read), ("main list", &main_list)] {
        assert!(!v.to_string().contains(SECRET), "{label} leaks the secret: {v}");
    }
    assert_eq!(b_ws["view"]["total_taps"], 1, "B's socket did get the change");
    assert_eq!(host_poll["view"]["secrets_set"], 1, "the host sees a count");
    assert!(!info.to_string().contains("Alice"), "the join page names nobody");
    // The page's PIN hash, salt and keys never go out
    for v in [&b_poll, &host_poll] {
        let s = v.to_string();
        assert!(!s.contains("pin_hash") && !s.contains("salt") && !s.contains("key"), "{s}");
    }
}

// ---- The sweeper and the switch ----

async fn create(t: &T, spec: Value) -> String {
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "test".into(), spec, ..Default::default() }).await.unwrap();
    s.row.code.clone()
}

#[tokio::test]
async fn the_sweeper_deletes_by_each_retention_kind() {
    let t = setup(true).await;
    let now = Utc::now();
    // after_end: 7 days after the end
    let after_end = create(&t, json!({ "title": "Quiz", "retention": "after_end", "days": 7 })).await;
    let (_, key) = joined(&t, &after_end, "Mia").await;
    send(&t.guest, "POST", &format!("/api/pocket/{after_end}/action"), Some(json!({ "name": "tap" })), &guest_req(&after_end, &key)).await;
    // after_date: 7 days after the reveal, ended or not
    let reveal = now + Duration::days(3);
    let after_date = create(&t, json!({ "title": "Santa", "retention": "after_date", "days": 7, "reveal_at": reveal.to_rfc3339() })).await;
    joined(&t, &after_date, "Jo").await;
    // at_end: nothing kept after the end
    let at_end = create(&t, json!({ "title": "Class", "retention": "at_end" })).await;
    joined(&t, &at_end, "Kid").await;
    // closes_at: the sweeper ends it, then its retention runs
    let (closing, _) = pocket::create(&t.state, CreateRequest { kit: "test".into(), spec: json!({ "title": "Vote", "days": 1 }), closes_at: Some(now + Duration::hours(1)), ..Default::default() }).await.unwrap();
    let closing = closing.row.code.clone();

    assert_eq!(pocket::sweep(&t.state, now).await, pocket::Swept::default(), "nothing is due yet");
    pocket::end(&t.state, &after_end).await.unwrap();
    pocket::end(&t.state, &at_end).await.unwrap();
    assert_eq!(t.state.db.pocket_row_counts(&after_end).await.unwrap(), [1, 1, 1, 1]);

    let swept = pocket::sweep(&t.state, Utc::now() + Duration::seconds(1)).await;
    assert_eq!(swept.deleted, 1, "at_end goes at the next sweep");
    assert_eq!(t.state.db.pocket_row_counts(&at_end).await.unwrap(), [0, 0, 0, 0]);

    let swept = pocket::sweep(&t.state, now + Duration::hours(2)).await;
    assert_eq!((swept.ended, swept.deleted), (1, 0), "past closing time it ends");
    assert_eq!(t.state.db.pocket_get(&closing).await.unwrap().unwrap().state, "ended");

    assert_eq!(pocket::sweep(&t.state, now + Duration::hours(23)).await.deleted, 0);
    assert_eq!(pocket::sweep(&t.state, now + Duration::days(2)).await.deleted, 1, "the closed vote, 1 day after it ended");
    assert_eq!(t.state.db.pocket_row_counts(&closing).await.unwrap()[0], 0);
    assert_eq!(pocket::sweep(&t.state, now + Duration::days(6)).await.deleted, 0);
    assert_eq!(t.state.db.pocket_row_counts(&after_end).await.unwrap(), [1, 1, 1, 1]);
    let swept = pocket::sweep(&t.state, now + Duration::days(7) + Duration::minutes(1)).await;
    assert_eq!(swept.deleted, 1, "after_end, 7 days after it ended");
    assert_eq!(t.state.db.pocket_row_counts(&after_end).await.unwrap(), [0, 0, 0, 0], "players, keys and events go too");
    assert_eq!(guest_state(&t, &after_end, &key).await.status, StatusCode::NOT_FOUND);

    assert_eq!(pocket::sweep(&t.state, reveal + Duration::days(7) - Duration::minutes(1)).await.deleted, 0);
    assert_eq!(t.state.db.pocket_get(&after_date).await.unwrap().unwrap().state, "open");
    assert_eq!(pocket::sweep(&t.state, reveal + Duration::days(7) + Duration::minutes(1)).await.deleted, 1, "reveal + 7 days, though still open");
    assert_eq!(t.state.db.pocket_row_counts(&after_date).await.unwrap(), [0, 0, 0, 0]);
}

#[tokio::test]
async fn switched_off_nothing_starts_and_nothing_answers() {
    let t = setup(false).await;
    let local = Req { peer: Some(LOCAL), ..Default::default() };
    assert!(t.state.pocket.listener.addr().is_none(), "no guest listener");
    assert!(!listener::reconcile(&t.state).await);
    assert!(t.state.pocket.listener.addr().is_none());
    // The list is empty; everything else on the main port is 404
    let r = send(&t.main, "GET", "/api/pockets", None, &local).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, json!({ "enabled": false, "pocket_port": null, "kits": [], "pockets": [] }));
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": "test", "spec": { "title": "x" } })), &local).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::NOT_FOUND, Some("pocket_apps_off")));
    // A pocket left from before: the sweeper leaves it alone and the guest router answers nothing
    let code = create(&t, json!({ "title": "Old", "retention": "at_end" })).await;
    pocket::end(&t.state, &code).await.unwrap();
    assert_eq!(pocket::sweep(&t.state, Utc::now() + Duration::days(30)).await, pocket::Swept::default());
    assert_eq!(t.state.db.pocket_row_counts(&code).await.unwrap()[0], 1);
    for path in [format!("/p/{code}"), format!("/api/pocket/{code}/info")] {
        assert_eq!(send(&t.guest, "GET", &path, None, &Req::default()).await.status, StatusCode::NOT_FOUND, "{path}");
    }
    for path in [format!("/api/pockets/{code}"), format!("/api/pockets/{code}/end")] {
        let method = if path.ends_with("end") { "POST" } else { "GET" };
        assert_eq!(send(&t.main, method, &path, None, &local).await.status, StatusCode::NOT_FOUND);
    }
    // On: the listener starts and answers; off again: it stops
    t.state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    assert!(listener::reconcile(&t.state).await);
    let addr = t.state.pocket.listener.addr().expect("started");
    let http = reqwest::Client::new();
    assert_eq!(http.get(format!("http://{addr}/p/{code}")).send().await.unwrap().status(), 200);
    t.state.db.set_setting(pocket::SETTING, "off").await.unwrap();
    assert!(!listener::reconcile(&t.state).await);
    assert!(t.state.pocket.listener.addr().is_none());
    let mut refused = false;
    for _ in 0..50 {
        match http.get(format!("http://{addr}/p/{code}")).send().await {
            Err(_) => {
                refused = true;
                break;
            }
            Ok(r) => assert_eq!(r.status(), 404, "a request still in flight finds the switch off"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(refused, "the port closes");
}

/// pa5's page: the same page for every code, never cached stale; its files are hashed and
/// immutable (unit tests in `guest.rs` serve a fake build); the boot carries only public facts.
#[tokio::test]
async fn the_page_is_no_cache_and_its_boot_is_public() {
    let t = setup(true).await;
    let (code, _) = make(&t, json!({ "title": "Quiz </script><script>x()</script>" })).await;
    let (_, key) = joined(&t, &code, "Mia").await;
    for r in [send(&t.guest, "GET", &format!("/p/{code}"), None, &Req::default()).await, send(&t.guest, "GET", &format!("/p/{code}"), None, &guest_req(&code, &key)).await] {
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.headers["content-security-policy"], guest::CSP);
        assert_eq!(r.headers["cache-control"], "no-cache");
        assert_eq!(r.headers["content-type"], "text/html; charset=utf-8");
        let html = r.body.as_str().unwrap();
        assert!(!html.contains("<script>x()"), "{html}");
        if guest::has_page() {
            let open = r#"<script type="application/json" id="pocket-boot">"#;
            let start = html.find(open).unwrap() + open.len();
            let boot: Value = serde_json::from_str(&html[start..start + html[start..].find("</script>").unwrap()]).unwrap();
            assert_eq!(boot["pocket"]["code"], code.as_str());
        }
    }
    // An unknown code is 404 (and still no-cache, so a later pocket with it isn't hidden)
    let r = send(&t.guest, "GET", "/p/zzzzzz", None, &Req::default()).await;
    assert_eq!((r.status, r.headers["cache-control"].to_str().unwrap()), (StatusCode::NOT_FOUND, "no-cache"));
    // API answers keep no-store
    let r = send(&t.guest, "GET", &format!("/api/pocket/{code}/info"), None, &Req::default()).await;
    assert_eq!(r.headers["cache-control"], "no-store");
    assert_eq!(r.body["pocket"]["code"], code.as_str());
    // The kit's pre-join facts ride along as `view` (the test kit: round and a head count, no names)
    assert_eq!(r.body["view"], json!({ "round": 0, "players": 1 }));
    assert!(!r.body.to_string().contains("Mia"), "{}", r.body);
    if guest::has_page() {
        let r = send(&t.guest, "GET", &format!("/p/{code}"), None, &Req::default()).await;
        let html = r.body.as_str().unwrap();
        let open = r#"<script type="application/json" id="pocket-boot">"#;
        let start = html.find(open).unwrap() + open.len();
        let boot: Value = serde_json::from_str(&html[start..start + html[start..].find("</script>").unwrap()]).unwrap();
        assert_eq!(boot["view"], json!({ "round": 0, "players": 1 }), "the boot is the info JSON");
    }
    // Off: the files are 404 like everything else
    t.state.db.set_setting(pocket::SETTING, "off").await.unwrap();
    assert!(!listener::reconcile(&t.state).await);
    for path in [format!("/p/{code}"), "/p/_a/g-nosuchfile.js".to_string()] {
        assert_eq!(send(&t.guest, "GET", &path, None, &Req::default()).await.status, StatusCode::NOT_FOUND, "{path}");
    }
}
