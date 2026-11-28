//! pa12 L5 (autonomy/pocket-apps/pa12/L5/brief.md, security-review.md sections 3 and 5): versions
//! of a bot-made app and the app document. The document route answers 200 only for a live version
//! framed by an iframe, with section 3's headers byte for byte and CSP hashes that match the bytes
//! served; every other guest answer keeps today's CSP and X-Frame-Options. Going live, rollback,
//! the additive rule, the 10-version cap, delete and the person-only main-port routes.

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

async fn ws_connect(addr: SocketAddr, code: &str, cookie: Option<(String, String)>) -> Ws {
    let mut req = format!("ws://{addr}/api/pocket/{code}/ws").into_client_request().unwrap();
    req.headers_mut().insert("origin", format!("http://{addr}").parse().unwrap());
    if let Some((k, v)) = cookie {
        req.headers_mut().insert("cookie", format!("{k}={v}").parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.expect("ws connects").0
}

async fn next_text(ws: &mut Ws) -> String {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a message within 5 s").unwrap().unwrap();
        if let Message::Text(t) = msg {
            return t.to_string();
        }
    }
}

use kwnt::pocket::made::versions::{self, Draft, Screen};
use sha2::{Digest, Sha256};

async fn raw(app: &Router, method: &str, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri).header("host", GHOST);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = if method == "POST" { Body::from("{}") } else { Body::empty() };
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    (status, headers, to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec())
}

const IFRAME: (&str, &str) = ("sec-fetch-dest", "iframe");

async fn doc(t: &T, code: &str, version: i64, surface: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    raw(&t.guest, "GET", &format!("/p/{code}/app/{version}/{surface}"), &[IFRAME]).await
}

fn vspec() -> Value {
    json!({
        "format": "made/1",
        "title": "Book swap",
        "retention": "after_end",
        "collections": {
            "shelf": { "read": "everyone", "write": "guest_many", "max_per_guest": 2, "fields": { "title": { "type": "text", "max": 60 } } },
            "notes": { "read": "host", "write": "guest_one", "fields": { "line": { "type": "text", "max": 60 } } }
        }
    })
}

fn screen(tag: &str, surface: &str) -> Screen {
    Screen {
        html: format!("<main class=\"stack\"><h1 id=\"t\">{tag} {surface}</h1><ul id=\"shelf\"></ul></main>"),
        css: format!("#t{{color:#123}} /* {tag} {surface} */"),
        js: format!("kwant.onState(function(s){{document.getElementById(\"t\").textContent=\"{tag}\"}});kwant.act(\"put\",{{collection:\"shelf\",values:{{title:\"{tag}\"}}}});"),
    }
}

async fn draft(t: &T, code: &str, spec: Value, tag: &str) -> Result<i64, kwnt::pocket::PocketError> {
    versions::draft(&t.state, code, Draft { spec, phone: Some(screen(tag, "phone")), screen: Some(screen(tag, "screen")), made_by: Some("k_maker".into()) }).await
}

async fn pass(t: &T, code: &str, v: i64) {
    versions::set_check(&t.state, code, v, versions::CHECKING, None).await.unwrap();
    versions::set_check(&t.state, code, v, versions::PASSED, Some("K2 lint: clean")).await.unwrap();
}

/// A version that went live: draft, check, pass, live.
async fn live(t: &T, code: &str, spec: Value, tag: &str) -> i64 {
    let v = draft(t, code, spec, tag).await.unwrap();
    pass(t, code, v).await;
    versions::go_live(&t.state, code, v).await.unwrap();
    v
}

fn b64(s: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(s))
}

/// The inline bodies between `open` and `close`, in order.
fn bodies<'a>(doc: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = doc;
    while let Some(i) = rest.find(open) {
        let after = &rest[i + open.len()..];
        let j = after.find(close).expect("closed");
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

/// security-review.md section 3's policy, written out here (not taken from the server).
fn section3_csp(prelude: &str, js: &str, pocket_css: &str, css: &str) -> String {
    format!(
        "sandbox allow-scripts; default-src 'none'; script-src 'sha256-{prelude}' 'sha256-{js}'; style-src 'sha256-{pocket_css}' 'sha256-{css}'; img-src data: blob:; font-src data:; connect-src 'none'; media-src 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'; require-trusted-types-for 'script'"
    )
}

const PERMISSIONS: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=(), hid=(), publickey-credentials-get=(), publickey-credentials-create=(), clipboard-read=(), display-capture=(), screen-wake-lock=(), web-share=(), fullscreen=()";

/// Checks one served document: section 3's headers byte for byte (and nothing else but a length),
/// the body order, and every inline body's sha256 against the CSP.
fn check_document(headers: &HeaderMap, bytes: &[u8], surface: &str, tag: &str) {
    let doc = std::str::from_utf8(bytes).unwrap();
    let scripts = bodies(doc, "<script>", "</script>");
    let styles = bodies(doc, "<style>", "</style>");
    assert_eq!(scripts.len(), 2, "prelude and app script");
    assert_eq!(styles.len(), 2, "pocket.css and app style");
    assert_eq!(scripts[0], versions::PRELUDE);
    assert_eq!(styles[0], versions::POCKET_CSS);
    let s = screen(tag, surface);
    assert_eq!((scripts[1], styles[1]), (s.js.as_str(), s.css.as_str()));
    // <html data-surface>, prelude, pocket.css, app style, markup, app script
    let pos = |p: &str| doc.find(p).unwrap_or_else(|| panic!("{p} in the document"));
    assert!(doc.starts_with(&format!("<!doctype html><html data-surface=\"{surface}\">")));
    assert!(pos(versions::PRELUDE) < pos(versions::POCKET_CSS));
    assert!(pos(versions::POCKET_CSS) < pos(&s.css) && pos(&s.css) < pos(&s.html) && pos(&s.html) < pos(&s.js));
    let want = [
        ("content-type", "text/html; charset=utf-8".to_string()),
        ("cache-control", "private, no-cache".to_string()),
        ("content-security-policy", section3_csp(&b64(scripts[0].as_bytes()), &b64(scripts[1].as_bytes()), &b64(styles[0].as_bytes()), &b64(styles[1].as_bytes()))),
        ("permissions-policy", PERMISSIONS.to_string()),
        ("x-content-type-options", "nosniff".to_string()),
        ("referrer-policy", "no-referrer".to_string()),
        ("x-dns-prefetch-control", "off".to_string()),
        ("cross-origin-resource-policy", "same-origin".to_string()),
    ];
    for (k, v) in &want {
        let got: Vec<&str> = headers.get_all(*k).iter().map(|v| v.to_str().unwrap()).collect();
        assert_eq!(got, vec![v.as_str()], "{k}");
    }
    let extra: Vec<&str> = headers.keys().map(|k| k.as_str()).filter(|k| *k != "content-length" && !want.iter().any(|(w, _)| w == k)).collect();
    assert!(extra.is_empty(), "no other header (no x-frame-options): {extra:?}");
    assert!(!headers.get("content-security-policy").unwrap().to_str().unwrap().replace("frame-ancestors 'self'", "").contains("'self'"));
}

fn is_guest_404(status: StatusCode, headers: &HeaderMap, bytes: &[u8]) -> bool {
    status == StatusCode::NOT_FOUND
        && serde_json::from_slice::<Value>(bytes).ok() == Some(json!({ "error": "not_found", "message": "There's no pocket app here." }))
        && headers.get("content-security-policy").map(|v| v.as_bytes()) == Some(guest::CSP.as_bytes())
        && headers.get("x-frame-options").map(|v| v.as_bytes()) == Some(b"DENY".as_slice())
}

fn view_app_version(r: &Res) -> Value {
    r.body["view"]["app"]["version"].clone()
}

#[tokio::test]
async fn the_live_document_has_section_3_headers_and_hashes_that_match_its_bytes() {
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    // Each version, made live in turn, is served exactly as stored, with hashes over those bytes
    for tag in ["first", "second", "third"] {
        let v = live(&t, &code, vspec(), tag).await;
        for surface in ["phone", "screen"] {
            let (status, headers, bytes) = doc(&t, &code, v, surface).await;
            assert_eq!(status, StatusCode::OK, "{tag} {surface}");
            check_document(&headers, &bytes, surface, tag);
            let row = t.state.db.pocket_app_version(&code, v).await.unwrap().unwrap();
            let stored = if surface == "phone" { row.phone_doc } else { row.screen_doc };
            assert_eq!(stored.as_deref().map(str::as_bytes), Some(bytes.as_slice()), "served byte for byte as stored");
        }
    }
    // pocket.css is the pinned build
    assert_eq!(b64(versions::POCKET_CSS.as_bytes()), versions::POCKET_CSS_SHA256);
}

#[tokio::test]
async fn only_an_iframe_gets_the_live_version_everything_else_is_the_guest_404() {
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    let v1 = live(&t, &code, vspec(), "one").await;
    let v2 = live(&t, &code, vspec(), "two").await;
    let drafted = draft(&t, &code, vspec(), "draft").await.unwrap();
    let failed = draft(&t, &code, vspec(), "failed").await.unwrap();
    versions::set_check(&t.state, &code, failed, versions::FAILED, Some("K2 lint: fetch")).await.unwrap();
    let ok = doc(&t, &code, v2, "phone").await;
    assert_eq!(ok.0, StatusCode::OK);
    // Fetch metadata: only exactly `iframe`
    for dest in [None, Some("document"), Some("empty"), Some(""), Some("frame"), Some("object"), Some("embed"), Some("script"), Some("IFRAME"), Some("iframe ")] {
        let h: Vec<(&str, &str)> = dest.map(|d| vec![("sec-fetch-dest", d)]).unwrap_or_default();
        let (s, hd, b) = raw(&t.guest, "GET", &format!("/p/{code}/app/{v2}/phone"), &h).await;
        assert!(is_guest_404(s, &hd, &b), "dest {dest:?}: {s}");
    }
    // Not live, not a surface, not a version, not a pocket
    let paths = [
        format!("/p/{code}/app/{v1}/phone"),
        format!("/p/{code}/app/{drafted}/phone"),
        format!("/p/{code}/app/{failed}/screen"),
        format!("/p/{code}/app/99/phone"),
        format!("/p/{code}/app/0/phone"),
        format!("/p/{code}/app/0{v2}/phone"),
        format!("/p/{code}/app/+{v2}/phone"),
        format!("/p/{code}/app/-{v2}/phone"),
        format!("/p/{code}/app/{v2}/tablet"),
        format!("/p/{code}/app/{v2}/Phone"),
        format!("/p/{code}/app/{v2}/phone/x"),
        format!("/p/{code}/app/{v2}"),
        format!("/p/{code}/app/{v2}/"),
        format!("/p/abcdef/app/{v2}/phone"),
        format!("/p/_a/app/{v2}/phone"),
    ];
    for p in &paths {
        let (s, hd, b) = raw(&t.guest, "GET", p, &[IFRAME]).await;
        assert!(is_guest_404(s, &hd, &b), "{p}: {s} {}", String::from_utf8_lossy(&b));
    }
    // A write to the route is the guest 404 too (same origin, so the gate lets it through)
    let (s, hd, b) = raw(&t.guest, "POST", &format!("/p/{code}/app/{v2}/phone"), &[IFRAME, ("origin", "http://pocket.example.net")]).await;
    assert!(is_guest_404(s, &hd, &b), "POST: {s}");
    // "Make your own" off hides the made pocket's document like the pocket itself
    t.state.db.set_setting(pocket::MAKE_SETTING, "off").await.unwrap();
    let (s, hd, b) = doc(&t, &code, v2, "phone").await;
    assert!(is_guest_404(s, &hd, &b), "make off: {s}");
}

#[tokio::test]
async fn every_other_guest_answer_keeps_todays_csp_and_frame_denial() {
    let t = setup(true).await;
    let (code, host) = made(&t, vspec()).await;
    live(&t, &code, vspec(), "one").await;
    let (_, key) = joined(&t, &code, "Ann").await;
    let cookie = format!("{}={key}", guest::guest_cookie(&code));
    let same = ("origin", "http://pocket.example.net");
    let ws = [("connection", "upgrade"), ("upgrade", "websocket"), ("sec-websocket-version", "13"), ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")];
    let auth = format!("Bearer {host}");
    let cases: Vec<(&str, String, Vec<(&str, &str)>)> = vec![
        ("GET", format!("/p/{code}"), vec![]),
        ("GET", format!("/p/{code}"), vec![IFRAME]),
        ("GET", "/p/_a/nope.js".into(), vec![]),
        ("GET", "/p/_a/index.html".into(), vec![IFRAME]),
        ("GET", format!("/api/pocket/{code}/info"), vec![]),
        ("GET", format!("/api/pocket/{code}/state"), vec![("cookie", cookie.as_str())]),
        ("POST", format!("/api/pocket/{code}/action"), vec![("cookie", cookie.as_str()), same, ("content-type", "application/json")]),
        ("GET", format!("/api/pocket/{code}/ws"), [&ws[..], &[same, ("cookie", cookie.as_str())]].concat()),
        ("GET", "/nothing/here".into(), vec![]),
        ("GET", format!("/api/pocket/{code}/host/state"), vec![("authorization", auth.as_str())]),
    ];
    for (method, uri, h) in &cases {
        let (s, hd, _) = raw(&t.guest, method, uri, h).await;
        assert_eq!(hd.get_all("content-security-policy").iter().map(|v| v.as_bytes()).collect::<Vec<_>>(), vec![guest::CSP.as_bytes()], "{method} {uri} {s}");
        assert_eq!(hd.get("x-frame-options").map(|v| v.as_bytes()), Some(b"DENY".as_slice()), "{method} {uri} {s}");
        assert_eq!(hd.get("x-content-type-options").map(|v| v.as_bytes()), Some(b"nosniff".as_slice()));
        assert_eq!(hd.get("referrer-policy").map(|v| v.as_bytes()), Some(b"no-referrer".as_slice()));
        assert!(hd.get("permissions-policy").is_none() && hd.get("cross-origin-resource-policy").is_none(), "{method} {uri}");
    }
}

#[tokio::test]
async fn a_null_origin_write_or_websocket_is_refused() {
    // G6: the sandboxed frame's requests carry `Origin: null`
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    live(&t, &code, vspec(), "one").await;
    let (_, key) = joined(&t, &code, "Ann").await;
    let cookie = format!("{}={key}", guest::guest_cookie(&code));
    let null = ("origin", "null");
    let (s, hd, b) = raw(&t.guest, "POST", &format!("/api/pocket/{code}/action"), &[null, ("cookie", cookie.as_str()), ("content-type", "application/json")]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(serde_json::from_slice::<Value>(&b).unwrap()["error"], "bad_origin");
    assert_eq!(hd.get("x-frame-options").map(|v| v.as_bytes()), Some(b"DENY".as_slice()));
    let ws = [("connection", "upgrade"), ("upgrade", "websocket"), ("sec-websocket-version", "13"), ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")];
    let (s, _, b) = raw(&t.guest, "GET", &format!("/api/pocket/{code}/ws"), &[&ws[..], &[null, ("cookie", cookie.as_str())]].concat()).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(serde_json::from_slice::<Value>(&b).unwrap()["error"], "bad_origin");
    // And over a real socket
    let addr = t.state.pocket.listener.addr().expect("the guest listener runs");
    let mut req = format!("ws://{addr}/api/pocket/{code}/ws").into_client_request().unwrap();
    req.headers_mut().insert("origin", "null".parse().unwrap());
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    assert!(tokio_tungstenite::connect_async(req).await.is_err(), "Origin: null never upgrades");
}

#[tokio::test]
async fn live_pointer_rollback_and_the_state_machine() {
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    let (_, key) = joined(&t, &code, "Ann").await;
    let state = || send(&t.guest, "GET", &format!("/api/pocket/{code}/state"), None, &guest_req(&code, &key));
    assert_eq!(view_app_version(&state().await), Value::Null, "no version is live yet");
    // A draft or a failed version can't go live
    let v1 = draft(&t, &code, vspec(), "one").await.unwrap();
    let e = versions::go_live(&t.state, &code, v1).await.unwrap_err();
    assert_eq!((e.status, e.code), (StatusCode::CONFLICT, "not_passed"));
    versions::set_check(&t.state, &code, v1, versions::CHECKING, None).await.unwrap();
    assert_eq!(versions::go_live(&t.state, &code, v1).await.unwrap_err().code, "not_passed");
    versions::set_check(&t.state, &code, v1, versions::PASSED, Some("ok")).await.unwrap();
    // A check that's over can't be moved again
    assert_eq!(versions::set_check(&t.state, &code, v1, versions::FAILED, None).await.unwrap_err().code, "version_moved_on");
    assert_eq!(versions::set_check(&t.state, &code, v1, versions::LIVE, None).await.unwrap_err().code, "bad_version_state");
    assert_eq!(versions::go_live(&t.state, &code, 42).await.unwrap_err().code, "no_such_version");
    versions::go_live(&t.state, &code, v1).await.unwrap();
    assert_eq!(view_app_version(&state().await), json!(v1));
    let failed = draft(&t, &code, vspec(), "bad").await.unwrap();
    versions::set_check(&t.state, &code, failed, versions::FAILED, Some("K2 lint: eval")).await.unwrap();
    assert_eq!(versions::go_live(&t.state, &code, failed).await.unwrap_err().code, "not_passed");

    let mut spec2 = vspec();
    spec2["title"] = json!("Book swap 2");
    let v2 = live(&t, &code, spec2, "two").await;
    let r = state().await;
    assert_eq!(view_app_version(&r), json!(v2));
    assert_eq!(r.body["pocket"]["title"], "Book swap 2", "pocket_apps.spec and title follow the live version");
    assert_eq!(doc(&t, &code, v1, "phone").await.0, StatusCode::NOT_FOUND);
    assert_eq!(doc(&t, &code, v2, "phone").await.0, StatusCode::OK);

    // The person's list and one-tap rollback on the main port
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}/app/versions"), None, &local()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["live"], json!(v2));
    let states: Vec<(i64, String)> = r.body["versions"].as_array().unwrap().iter().map(|v| (v["version"].as_i64().unwrap(), v["state"].as_str().unwrap().to_string())).collect();
    assert_eq!(states, vec![(v2, "live".into()), (failed, "failed".into()), (v1, "retired".into())]);
    assert_eq!(r.body["versions"][0]["made_by"], "k_maker");
    assert_eq!(r.body["versions"][1]["report"], "K2 lint: eval");

    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/app/rollback"), None, &local()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["live"], json!(v1), "the newest earlier version that passed (not the failed one)");
    assert_eq!(doc(&t, &code, v2, "phone").await.0, StatusCode::NOT_FOUND, "the rolled-back version is gone");
    let (s, h, b) = doc(&t, &code, v1, "phone").await;
    assert_eq!(s, StatusCode::OK);
    check_document(&h, &b, "phone", "one");
    let r = state().await;
    assert_eq!(view_app_version(&r), json!(v1));
    assert_eq!(r.body["pocket"]["title"], "Book swap");
    let v2_row = r_list(&t, &code).await.into_iter().find(|v| v.0 == v2).unwrap();
    assert_eq!(v2_row, (v2, "failed".to_string(), Some(versions::ROLLED_BACK.to_string())));
    // The rolled-back one never comes back, and there's nothing older
    assert_eq!(versions::go_live(&t.state, &code, v2).await.unwrap_err().code, "not_passed");
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/app/rollback"), None, &local()).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("nothing_to_roll_back")));
    // A later version rolls back to v1 again, never to the rolled-back v2
    let v4 = live(&t, &code, vspec(), "four").await;
    assert_eq!(view_app_version(&state().await), json!(v4));
    assert_eq!(versions::rollback(&t.state, &code).await.unwrap(), v1);
    assert_eq!(view_app_version(&state().await), json!(v1));
}

async fn r_list(t: &T, code: &str) -> Vec<(i64, String, Option<String>)> {
    t.state.db.pocket_app_versions(code).await.unwrap().into_iter().map(|v| (v.version, v.state, v.report)).collect()
}

#[tokio::test]
async fn the_websocket_names_the_new_live_version() {
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    let (_, key) = joined(&t, &code, "Ann").await;
    let v1 = live(&t, &code, vspec(), "one").await;
    let addr = t.state.pocket.listener.addr().expect("the guest listener runs");
    let mut ws = ws_connect(addr, &code, Some((guest::guest_cookie(&code), key.clone()))).await;
    let first: Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
    assert_eq!(first["view"]["app"]["version"], json!(v1));
    let v2 = live(&t, &code, vspec(), "two").await;
    let next: Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
    assert_eq!(next["view"]["app"]["version"], json!(v2));
}

#[tokio::test]
async fn the_additive_rule_refuses_removing_or_retyping_a_collection_with_rows_once_guests_are_in() {
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    let mut no_shelf = vspec();
    no_shelf["collections"].as_object_mut().unwrap().remove("shelf");
    // Before any guest, anything goes; this one passes its check while the pocket is empty
    let early = draft(&t, &code, no_shelf.clone(), "early").await.unwrap();
    pass(&t, &code, early).await;
    let (_, key) = joined(&t, &code, "Ann").await;
    let r = put(&t, &code, &key, "shelf", json!({ "title": "Emma" })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let refused = |e: kwnt::pocket::PocketError| {
        assert_eq!((e.status, e.code), (StatusCode::CONFLICT, "not_additive"));
        assert!(e.message.ends_with("start a new pocket for that."), "{}", e.message);
    };
    refused(draft(&t, &code, no_shelf.clone(), "gone").await.unwrap_err());
    // Going live checks again: guests came after the draft
    refused(versions::go_live(&t.state, &code, early).await.unwrap_err());
    let mut retyped = vspec();
    retyped["collections"]["shelf"]["fields"]["title"] = json!({ "type": "int", "min": 0, "max": 9 });
    refused(draft(&t, &code, retyped, "retyped").await.unwrap_err());
    let mut dropped = vspec();
    dropped["collections"]["shelf"]["fields"] = json!({ "name": { "type": "text", "max": 60 } });
    refused(draft(&t, &code, dropped, "dropped").await.unwrap_err());
    let mut wider = vspec();
    wider["collections"]["shelf"]["read"] = json!("host");
    refused(draft(&t, &code, wider, "read").await.unwrap_err());
    // Adding a field and a collection, and dropping the empty `notes`, is fine
    let mut added = vspec();
    added["collections"]["shelf"]["fields"]["author"] = json!({ "type": "text", "max": 60 });
    added["collections"].as_object_mut().unwrap().remove("notes");
    added["collections"]["likes"] = json!({ "read": "everyone", "write": "guest_one", "fields": { "book": { "type": "row", "of": "shelf" } } });
    let v = live(&t, &code, added, "added").await;
    assert_eq!(doc(&t, &code, v, "screen").await.0, StatusCode::OK);
    // A bad spec or a part that would end its element early is refused at the draft
    let e = draft(&t, &code, json!({ "format": "made/1" }), "x").await.unwrap_err();
    assert_eq!((e.status, e.code), (StatusCode::BAD_REQUEST, "bad_spec"));
    let bad = Draft { spec: vspec(), phone: Some(Screen { html: String::new(), css: String::new(), js: "x='</script>'".into() }), screen: None, made_by: None };
    let e = versions::draft(&t.state, &code, bad).await.unwrap_err();
    assert_eq!((e.status, e.code), (StatusCode::BAD_REQUEST, "bad_screen"));
}

#[tokio::test]
async fn ten_versions_at_most_the_live_one_kept_and_delete_removes_them() {
    let t = setup(true).await;
    let (code, _) = made(&t, vspec()).await;
    let v1 = live(&t, &code, vspec(), "one").await;
    for i in 0..12 {
        draft(&t, &code, vspec(), &format!("d{i}")).await.unwrap();
    }
    let list = t.state.db.pocket_app_versions(&code).await.unwrap();
    assert_eq!(list.len(), versions::MAX_APP_VERSIONS as usize);
    let nums: Vec<i64> = list.iter().map(|v| v.version).collect();
    assert_eq!(nums, (5..=13).rev().chain([v1]).collect::<Vec<_>>(), "the oldest drafts go, the live one stays");
    assert_eq!(doc(&t, &code, v1, "phone").await.0, StatusCode::OK);
    // New numbers never reuse a dropped one
    assert_eq!(draft(&t, &code, vspec(), "next").await.unwrap(), 14);
    let r = send(&t.main, "DELETE", &format!("/api/pockets/{code}"), None, &local()).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert!(t.state.db.pocket_app_versions(&code).await.unwrap().is_empty());
    assert!(t.state.db.pocket_app_version(&code, v1).await.unwrap().is_none());
    let (s, h, b) = doc(&t, &code, v1, "phone").await;
    assert!(is_guest_404(s, &h, &b));
}

#[tokio::test]
async fn two_guests_get_byte_identical_documents() {
    let t = setup(true).await;
    let (code, host) = made(&t, vspec()).await;
    let v = live(&t, &code, vspec(), "one").await;
    let (_, a) = joined(&t, &code, "Ann").await;
    let (_, b) = joined(&t, &code, "Bob").await;
    let get = |cookie: String| {
        let uri = format!("/p/{code}/app/{v}/phone");
        let t = &t;
        async move { raw(&t.guest, "GET", &uri, &[IFRAME, ("cookie", cookie.as_str())]).await }
    };
    let ra = get(format!("{}={a}", guest::guest_cookie(&code))).await;
    let rb = get(format!("{}={b}", guest::guest_cookie(&code))).await;
    let rh = get(format!("{}={host}", guest::host_cookie(&code))).await;
    assert_eq!(ra.0, StatusCode::OK);
    assert_eq!(ra.2, rb.2);
    assert_eq!(ra.2, rh.2);
    assert_eq!(ra.1, rb.1);
    assert!(!String::from_utf8_lossy(&ra.2).contains("Ann") && !String::from_utf8_lossy(&ra.2).contains("Bob"), "no guest data in the document");
}

#[test]
fn the_prelude_buffers_the_last_state_acts_by_message_and_has_no_timer() {
    let p = versions::PRELUDE;
    assert!(p.lines().count() <= 15, "about 15 lines");
    // (a) a late onState gets the held state at once
    assert!(p.contains("fns.push(fn);if(has)fn(last)"));
    // (b) only the parent's {type:"state"} is taken, kept, and given to every listener once
    assert!(p.contains("e.source!==P||!d||d.type!==\"state\")return;last=d;has=true;"));
    // (c) act posts {type:"act", name, body} to the parent
    assert!(p.contains("P.postMessage({type:\"act\",name:String(name),body:"));
    // WebRTC is gone before app code; no timer, nothing re-posted, no network
    assert!(p.contains("\"RTCPeerConnection\",\"webkitRTCPeerConnection\",\"mozRTCPeerConnection\",\"RTCDataChannel\"") && p.contains("delete w[k]"));
    for banned in ["setTimeout", "setInterval", "requestAnimationFrame", "requestIdleCallback", "fetch(", "XMLHttpRequest", "WebSocket", "</script", "<!--"] {
        assert!(!p.contains(banned), "{banned}");
    }
    assert_eq!(p.matches("postMessage").count(), 2, "act and the violation report only");
}

#[tokio::test]
async fn the_main_port_routes_are_person_only_and_made_only() {
    let t = setup(true).await;
    let (code, host) = made(&t, vspec()).await;
    for (method, path) in [("GET", "versions"), ("POST", "rollback")] {
        let uri = format!("/api/pockets/{code}/app/{path}");
        let r = send(&t.main, method, &uri, None, &Req { bearer: Some("a-bot-turn-token-0123456789"), local: true, ..Default::default() }).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "a bot's token: {method} {uri}");
        let r = send(&t.main, method, &uri, None, &Req { bearer: Some(host.as_str()), local: true, ..Default::default() }).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "a host key never works here: {method} {uri}");
        let r = send(&t.main, method, &uri, None, &Req { bearer: Some(TOKEN), ..Default::default() }).await;
        assert!(r.status != StatusCode::FORBIDDEN || r.body["error"] != "person_only", "the person's token: {method} {uri} {}", r.body);
    }
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}/app/versions"), None, &local()).await;
    assert_eq!((r.status, &r.body), (StatusCode::OK, &json!({ "live": null, "versions": [] })));
    // The guest port has no such route
    let r = send(&t.guest, "GET", &format!("/api/pockets/{code}/app/versions"), None, &host_req(&host)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
