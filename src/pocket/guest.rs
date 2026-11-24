//! The guest port's router (Pocket API v1). It answers only `GET /p/<code>`, the page's own files
//! under `GET /p/_a/<file>`, and `/api/pocket/<code>/*`; everything else is 404, and so is
//! everything while the switch is off.
//! There is no `auth::guard` here: guests have no account. A guest key (cookie `kp_<code>`) or a
//! host key (cookie `kph_<code>`, or `Authorization: Bearer`) opens one pocket only, and neither
//! is ever accepted on the main port. A host or screen link (pa1b, Pocket API v1.1) signs in
//! through `host/signin {token}` and gets a session key in the same `kph_<code>` cookie; a
//! screen session may only read `view=screen`.

use super::kits::{KitCtx, Viewer};
use super::{keys, PResult, PocketError, Snapshot};
use crate::db::pocket::PlayerRow;
use crate::server::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

/// The page's Content-Security-Policy (pa5's page must fit it: no inline script or style, no
/// third-party anything).
pub const CSP: &str =
    "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'";
/// Request bodies on the guest port: names, PINs and small actions only.
pub const BODY_LIMIT: usize = 16 * 1024;
/// Guest keys outlive any pocket's retention; the key dies with the pocket anyway.
const COOKIE_MAX_AGE: i64 = 90 * 24 * 3600;
const WS_PING: Duration = Duration::from_secs(25);
const WS_MAX_MESSAGE: usize = 4 * 1024;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/p/:code", get(page))
        .route("/p/_a/:file", get(page_file))
        .route("/api/pocket/:code/info", get(info))
        .route("/api/pocket/:code/join", post(join))
        .route("/api/pocket/:code/rejoin", post(rejoin))
        .route("/api/pocket/:code/state", get(guest_state))
        .route("/api/pocket/:code/action", post(action))
        .route("/api/pocket/:code/notices/seen", post(notices_seen))
        .route("/api/pocket/:code/ws", get(ws))
        .route("/api/pocket/:code/host/signin", post(host_signin))
        .route("/api/pocket/:code/host/state", get(host_state))
        .route("/api/pocket/:code/host/action", post(host_action))
        .route("/api/pocket/:code/host/remove", post(host_remove))
        .route("/api/pocket/:code/host/reset-pin", post(host_reset_pin))
        .route("/api/pocket/:code/host/end", post(host_end))
        // pa14: the Wall's photos. Only the upload takes a big body; every other route keeps 16 KB
        .route("/api/pocket/:code/photo", post(photo_upload).layer(DefaultBodyLimit::max(super::wall::UPLOAD_LIMIT)))
        .route("/api/pocket/:code/photo/:id", get(photo_get))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(axum::middleware::from_fn_with_state(state.clone(), gate))
        .with_state(state)
}

async fn not_found() -> Response {
    PocketError::not_found().into_response()
}

/// Off means 404 for everything; a foreign Origin on a write or a WebSocket is refused; a
/// method a path doesn't take is 404 too; every answer gets the safety headers.
async fn gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut res = if !state.pocket.on.load(Ordering::SeqCst) {
        PocketError::not_found().into_response()
    } else if (req.method() != Method::GET || req.headers().contains_key(header::UPGRADE)) && !same_origin(req.headers()) {
        PocketError::new(StatusCode::FORBIDDEN, "bad_origin", "This request came from another site.").into_response()
    } else {
        let res = next.run(req).await;
        if res.status() == StatusCode::METHOD_NOT_ALLOWED {
            PocketError::not_found().into_response()
        } else {
            res
        }
    };
    let h = res.headers_mut();
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    // The page (no-cache) and its hashed files (immutable) set their own; everything else is no-store
    h.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    res
}

/// No Origin (curl, a test) is fine: only browsers carry cookies across sites, and they send it.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let Some(origin_host) = origin.split_once("://").map(|(_, rest)| rest.trim_end_matches('/')) else { return false };
    let hosts = [headers.get("x-forwarded-host"), headers.get(header::HOST)];
    hosts.iter().flatten().filter_map(|v| v.to_str().ok()).any(|h| h.eq_ignore_ascii_case(origin_host))
}

/// The guest's address, for the in-memory limiter only (never stored or logged). Behind a local
/// tunnel the peer is loopback, so the tunnel's header names the guest.
fn client_ip(peer: Option<SocketAddr>, headers: &HeaderMap) -> IpAddr {
    let peer_ip = peer.map(|p| p.ip()).unwrap_or(IpAddr::from([127, 0, 0, 1]));
    if !peer_ip.is_loopback() {
        return peer_ip;
    }
    let forwarded = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .or_else(|| headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()));
    forwarded.and_then(|v| v.trim().parse().ok()).unwrap_or(peer_ip)
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

pub fn guest_cookie(code: &str) -> String {
    format!("kp_{code}")
}

pub fn host_cookie(code: &str) -> String {
    format!("kph_{code}")
}

fn set_cookie(name: &str, value: &str, code: &str, headers: &HeaderMap) -> HeaderValue {
    let secure = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).is_some_and(|p| p.eq_ignore_ascii_case("https"));
    let v = format!(
        "{name}={value}; Path=/api/pocket/{code}; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Lax{}",
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&v).unwrap_or_else(|_| HeaderValue::from_static(""))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim)
}

async fn pocket(state: &AppState, code: &str) -> PResult<Arc<Snapshot>> {
    if !keys::valid_code(code) {
        return Err(PocketError::not_found());
    }
    super::snapshot(state, code).await?.ok_or_else(PocketError::not_found)
}

fn signed_out() -> PocketError {
    PocketError::new(StatusCode::UNAUTHORIZED, "signed_out", "Join first, or open your page again with your name.")
}

/// The guest signed in with this pocket's cookie, if any.
async fn guest(state: &AppState, code: &str, headers: &HeaderMap) -> PResult<Option<PlayerRow>> {
    match cookie(headers, &guest_cookie(code)) {
        Some(key) if !key.is_empty() => super::guest_for_key(state, code, key).await,
        _ => Ok(None),
    }
}

/// The host key or a link session key, from the bearer or the `kph_<code>` cookie.
fn host_credential<'a>(code: &str, headers: &'a HeaderMap) -> Option<&'a str> {
    bearer(headers).or_else(|| cookie(headers, &host_cookie(code))).filter(|k| !k.is_empty())
}

fn not_host() -> PocketError {
    PocketError::new(StatusCode::UNAUTHORIZED, "not_host", "Only the host's remote can do this.")
}

fn screen_only() -> PocketError {
    PocketError::new(StatusCode::FORBIDDEN, "screen_only", "This screen can only show the game.")
}

/// The pocket and what this caller's host key or link session may do there.
async fn host_as(state: &AppState, code: &str, headers: &HeaderMap) -> PResult<(Arc<Snapshot>, super::HostRole)> {
    let s = pocket(state, code).await?;
    let Some(key) = host_credential(code, headers) else { return Err(not_host()) };
    let role = super::host_role(state, &s, key).await?.ok_or_else(not_host)?;
    Ok((s, role))
}

/// For every host write: the host key or a host link; a screen gets 403 `screen_only`.
async fn host(state: &AppState, code: &str, headers: &HeaderMap) -> PResult<Arc<Snapshot>> {
    match host_as(state, code, headers).await? {
        (s, super::HostRole::Host) => Ok(s),
        (_, super::HostRole::Screen) => Err(screen_only()),
    }
}

fn signin_limit(state: &AppState, peer: Option<SocketAddr>, headers: &HeaderMap) -> PResult<()> {
    if state.pocket.limits.signin(client_ip(peer, headers)) {
        Ok(())
    } else {
        Err(PocketError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", "Too many tries from here. Wait a moment."))
    }
}

fn peer(ci: Option<ConnectInfo<SocketAddr>>) -> Option<SocketAddr> {
    ci.map(|ConnectInfo(a)| a)
}

fn html_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '&' => "&amp;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// The guest page (pa5), from the build's `frontend/dist/pocket/`. The same file for every code.
const PAGE_DIR: &str = "pocket/";
const PAGE_INDEX: &str = "index.html";
/// The page's boot slot: the server swaps `null` for this visitor's first state.
const BOOT_SLOT: &str = r#"<script type="application/json" id="pocket-boot">null</script>"#;
const BOOT_OPEN: &str = r#"<script type="application/json" id="pocket-boot">"#;

fn page_asset(name: &str) -> Option<Cow<'static, [u8]>> {
    crate::embedded::Assets::get(&format!("{PAGE_DIR}{name}")).map(|f| f.data)
}

/// True when this build carries pa5's page (otherwise `/p/<code>` serves the placeholder).
pub fn has_page() -> bool {
    page_asset(PAGE_INDEX).is_some()
}

/// JSON that is safe inside `<script type="application/json">`: nothing in it can close the
/// element or start a comment, whatever a title or a name says.
pub fn boot_json(v: &Value) -> String {
    let text = serde_json::to_string(v).unwrap_or_else(|_| "null".into());
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

/// The page with its boot slot filled. A page without the exact slot is served as built.
pub fn fill_boot(index: &str, boot: &Value) -> String {
    match index.find(BOOT_SLOT) {
        Some(at) => {
            let mut out = String::with_capacity(index.len() + 512);
            out.push_str(&index[..at]);
            out.push_str(BOOT_OPEN);
            out.push_str(&boot_json(boot));
            out.push_str("</script>");
            out.push_str(&index[at + BOOT_SLOT.len()..]);
            out
        }
        None => index.to_string(),
    }
}

fn html_response(status: StatusCode, body: String) -> Response {
    let mut res = (status, Html(body)).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    // Always asks again, so a new build's page (and its new file names) reaches phones at once
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}

/// What this visitor's page starts with: their own state with a seat, else the public facts.
/// (The seat cookie's Path is `/api/pocket/<code>`, so browsers don't send it here: in practice
/// the boot is the public facts and the page asks `state` for the seat.)
async fn boot_state(state: &AppState, s: &Snapshot, code: &str, headers: &HeaderMap) -> Value {
    match guest(state, code, headers).await {
        Ok(Some(p)) => super::view_json(s, &Viewer::Guest(p.id), Utc::now()),
        _ => public_json(s),
    }
}

/// `GET /p/<code>`: pa5's page with the boot filled in, or a placeholder when this build has no
/// page. 404 for a malformed or unknown code.
async fn page(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap) -> Response {
    let s = match pocket(&state, &code).await {
        Ok(s) => s,
        Err(_) => return html_response(StatusCode::NOT_FOUND, placeholder("Not found", "There's no pocket app here.")),
    };
    if let Some(index) = page_asset(PAGE_INDEX) {
        let boot = boot_state(&state, &s, &code, &headers).await;
        return html_response(StatusCode::OK, fill_boot(&String::from_utf8_lossy(&index), &boot));
    }
    let line = if s.row.state == crate::db::pocket::ENDED { "This pocket app has ended." } else { "This pocket app's page is on its way." };
    html_response(StatusCode::OK, placeholder(&s.row.title, line))
}

/// The page before pa5 is in the build. No inline script or style, as the CSP needs.
fn placeholder(title: &str, line: &str) -> String {
    let title = html_escape(title);
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"referrer\" content=\"no-referrer\"><title>{title}</title></head><body><main><h1>{title}</h1><p>{line}</p></main></body></html>"
    )
}

/// The content type for one of the page's files, by extension. Anything else isn't served.
pub fn page_file_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "webp" => "image/webp",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// A flat file name from the page's folder: no folders, no way up, never the page itself.
pub fn page_file_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != PAGE_INDEX
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && !name.contains("..")
        && page_file_type(name).is_some()
}

/// One of the page's files, through `lookup` (the build's files; tests pass their own).
pub fn page_file_response(name: &str, lookup: impl Fn(&str) -> Option<Cow<'static, [u8]>>) -> Response {
    let (true, Some(mime)) = (page_file_name_ok(name), page_file_type(name)) else {
        return PocketError::not_found().into_response();
    };
    let Some(data) = lookup(name) else {
        return PocketError::not_found().into_response();
    };
    let mut res = (StatusCode::OK, data.into_owned()).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    // Names carry a content hash, so a file never changes under its name
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    res
}

/// `GET /p/_a/<file>`: the page's hashed scripts, styles and fonts. Can't meet a pocket code:
/// codes never contain `_`.
async fn page_file(Path(file): Path<String>) -> Response {
    page_file_response(&file, page_asset)
}

/// The public facts anyone with the link sees before joining (`info`, and the page's boot
/// without a seat): `{api, pocket, view}`, where `view` is the kit's `lobby` (or `null`).
/// No names, no guest text.
pub fn public_json(s: &Snapshot) -> Value {
    let view = s.kit().map_or(Value::Null, |k| k.lobby(&KitCtx::new(&s.spec, &s.players, Utc::now()), &s.kit_state));
    json!({ "api": super::API_VERSION, "pocket": super::pocket_json(s), "view": view })
}

/// What anyone with the link sees before joining: no names, no guest text.
async fn info(State(state): State<AppState>, Path(code): Path<String>) -> PResult<Json<Value>> {
    let s = pocket(&state, &code).await?;
    Ok(Json(public_json(&s)))
}

fn signed_in(state_json: Value, cookie: HeaderValue, status: StatusCode) -> Response {
    let mut res = (status, Json(state_json)).into_response();
    res.headers_mut().append(header::SET_COOKIE, cookie);
    res
}

async fn join(
    State(state): State<AppState>,
    Path(code): Path<String>,
    ci: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<super::JoinRequest>,
) -> PResult<Response> {
    pocket(&state, &code).await?;
    // Already joined on this phone: the same player, no second seat
    if let Some(p) = guest(&state, &code, &headers).await? {
        let s = pocket(&state, &code).await?;
        return Ok((StatusCode::OK, Json(super::view_json(&s, &Viewer::Guest(p.id), Utc::now()))).into_response());
    }
    signin_limit(&state, peer(ci), &headers)?;
    let (p, key) = super::join(&state, &code, req).await?;
    let s = pocket(&state, &code).await?;
    let body = super::view_json(&s, &Viewer::Guest(p.id), Utc::now());
    Ok(signed_in(body, set_cookie(&guest_cookie(&code), &key, &code, &headers), StatusCode::CREATED))
}

async fn rejoin(
    State(state): State<AppState>,
    Path(code): Path<String>,
    ci: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<super::RejoinRequest>,
) -> PResult<Response> {
    pocket(&state, &code).await?;
    signin_limit(&state, peer(ci), &headers)?;
    let (p, key, notices) = super::rejoin(&state, &code, req).await?;
    let s = pocket(&state, &code).await?;
    let mut body = super::view_json(&s, &Viewer::Guest(p.id), Utc::now());
    body["notices"] = Value::Array(notices);
    Ok(signed_in(body, set_cookie(&guest_cookie(&code), &key, &code, &headers), StatusCode::OK))
}

#[derive(Debug, Default, Deserialize)]
pub struct StateQuery {
    /// The version the page has: an unchanged pocket answers 204 with no body.
    #[serde(default)]
    pub since: Option<i64>,
    /// `screen` on the host's state: the big screen's view.
    #[serde(default)]
    pub view: Option<String>,
}

fn state_response(s: &Snapshot, viewer: &Viewer, since: Option<i64>) -> Response {
    if since == Some(s.row.version) {
        return StatusCode::NO_CONTENT.into_response();
    }
    Json(super::view_json(s, viewer, Utc::now())).into_response()
}

/// Polling (every 2 s for pages without WebSocket): the same JSON the WebSocket pushes.
async fn guest_state(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Query(q): Query<StateQuery>) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
    state.pocket.touch(&code);
    Ok(state_response(&s, &Viewer::Guest(p.id), q.since))
}

#[derive(Debug, Default, Deserialize)]
pub struct ActionRequest {
    pub name: String,
    #[serde(default)]
    pub body: Value,
}

async fn action(
    State(state): State<AppState>,
    Path(code): Path<String>,
    ci: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<ActionRequest>,
) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
    if !state.pocket.limits.action(&code, &p.id) {
        return Err(PocketError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", "Slow down a little."));
    }
    // pa11b: a queue ticket is 3 an hour per guest key and 30 per address. A guest who already
    // holds one gets the kit's has_ticket answer instead, and only a take that worked counts
    let take_ip = (s.row.kit == super::queue::ID && req.name == "take" && !super::queue::has_live_ticket(&s.kit_state, &p.id)).then(|| client_ip(peer(ci), &headers));
    if let Some(ip) = take_ip {
        if !state.pocket.limits.may_take(&code, &p.id, ip) {
            return Err(PocketError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", "This phone has taken 3 tickets in the last hour. Wait a bit, or ask at the counter."));
        }
    }
    let viewer = Viewer::Guest(p.id.clone());
    super::act(&state, &code, &viewer, &req.name, &req.body).await?;
    if let Some(ip) = take_ip {
        state.pocket.limits.took(&code, &p.id, ip);
    }
    let s = pocket(&state, &code).await?;
    Ok(Json(super::view_json(&s, &viewer, Utc::now())).into_response())
}

async fn notices_seen(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap) -> PResult<StatusCode> {
    pocket(&state, &code).await?;
    let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
    super::notices_seen(&state, &code, &p.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `{key}` (the host key, v1) or `{token}` (a one-time host or screen link, v1.1).
#[derive(Debug, Default, Deserialize)]
pub struct HostSignin {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

/// The host's remote trades the key from its link for the host cookie; a host or screen link
/// trades its one-time token for a session key with that role, in the same cookie.
async fn host_signin(
    State(state): State<AppState>,
    Path(code): Path<String>,
    ci: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<HostSignin>,
) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    signin_limit(&state, peer(ci), &headers)?;
    let token = req.token.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let key = req.key.as_deref().map(str::trim).filter(|k| !k.is_empty());
    let (viewer, cookie_value) = match (token, key) {
        (Some(token), _) => {
            let (role, session) = super::spend_link(&state, &code, token, Utc::now()).await?;
            (if role == super::HostRole::Screen { Viewer::BigScreen } else { Viewer::Host }, session)
        }
        (None, Some(key)) if super::is_host_key(&s, key) => (Viewer::Host, key.to_string()),
        (None, Some(_)) => return Err(PocketError::new(StatusCode::UNAUTHORIZED, "not_host", "That host link doesn't work any more.")),
        (None, None) => return Err(PocketError::new(StatusCode::BAD_REQUEST, "no_key", "Open the link from the card again.")),
    };
    let s = pocket(&state, &code).await?;
    let body = super::view_json(&s, &viewer, Utc::now());
    Ok(signed_in(body, set_cookie(&host_cookie(&code), &cookie_value, &code, &headers), StatusCode::OK))
}

fn host_viewer(q: &StateQuery) -> Viewer {
    if q.view.as_deref() == Some("screen") {
        Viewer::BigScreen
    } else {
        Viewer::Host
    }
}

/// The remote's state (`view=host`, the default) or the big screen's (`view=screen`). A screen
/// session reads only `view=screen`.
async fn host_state(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Query(q): Query<StateQuery>) -> PResult<Response> {
    let (s, role) = host_as(&state, &code, &headers).await?;
    // pa13a: one part of the host's view (a front desk guest's conversation), host only
    if let Some(key) = q.view.as_deref().filter(|v| v.starts_with("guest:")) {
        if role == super::HostRole::Screen {
            return Err(screen_only());
        }
        state.pocket.touch(&code);
        return Ok(Json(super::host_detail_json(&s, key, Utc::now())?).into_response());
    }
    let viewer = host_viewer(&q);
    if role == super::HostRole::Screen && viewer != Viewer::BigScreen {
        return Err(screen_only());
    }
    state.pocket.touch(&code);
    Ok(state_response(&s, &viewer, q.since))
}

async fn host_action(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Json(req): Json<ActionRequest>) -> PResult<Response> {
    host(&state, &code, &headers).await?;
    super::act(&state, &code, &Viewer::Host, &req.name, &req.body).await?;
    let s = pocket(&state, &code).await?;
    Ok(Json(super::view_json(&s, &Viewer::Host, Utc::now())).into_response())
}

/// pa14: one photo from a guest, the raw file as the body (`Content-Type` is ignored: the type
/// comes from the file's first bytes) and the caption in `X-Caption` (URI-encoded). 201 with the
/// guest's view and `photo: {id}`. Refused before anything is stored when it's over the wall's
/// per-photo size (413 `too_big`).
async fn photo_upload(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, body: axum::body::Body) -> PResult<Response> {
    use http_body_util::BodyExt;
    let s = pocket(&state, &code).await?;
    let caps = super::wall::wall_caps(&s)?;
    let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
    if !state.pocket.limits.photo(&code, &p.id) {
        // One photo every 3 s after a burst of 10: the page waits this long and sends again
        let mut res = PocketError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", "Slow down a little: wait a few seconds before the next photo.").into_response();
        res.headers_mut().insert(header::RETRY_AFTER, axum::http::HeaderValue::from_static("3"));
        return Ok(res);
    }
    let caption = super::wall::header_caption(&s.spec, headers.get("x-caption").map(|v| v.as_bytes()))?;
    let max = caps.photo_bytes.min(super::wall::UPLOAD_LIMIT as u64);
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > max) {
        return Err(super::wall::too_big(max));
    }
    // A slot first (pa14 QA F2): the body is read only once one is free, so 20 guests at once
    // never hold 20 photos in memory. A busy minute ends in the same `slow_down` as the rate.
    let slot = tokio::time::timeout(std::time::Duration::from_secs(super::wall::UPLOAD_WAIT_SECS), super::wall::UPLOADS.acquire()).await;
    let Ok(Ok(_slot)) = slot else {
        let mut res = PocketError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", "Lots of photos are going up right now: wait a few seconds and send it again.").into_response();
        res.headers_mut().insert(header::RETRY_AFTER, axum::http::HeaderValue::from_static("3"));
        return Ok(res);
    };
    let bytes = match http_body_util::Limited::new(body, max as usize).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => return Err(super::wall::too_big(max)),
        Err(_) => return Err(PocketError::new(StatusCode::BAD_REQUEST, "broken_photo", "That photo didn't arrive whole. Try sending it again.")),
    };
    let id = super::wall::add_photo(&state, &code, &p.id, bytes, caption).await?;
    let s = pocket(&state, &code).await?;
    let mut v = super::view_json(&s, &Viewer::Guest(p.id), Utc::now());
    v["photo"] = json!({ "id": id });
    Ok((StatusCode::CREATED, Json(v)).into_response())
}

/// pa14: one photo's bytes. The host key or a host or screen link (cookie or bearer) first, else
/// the guest cookie; the Wall decides what each may see (404 for anything else).
async fn photo_get(State(state): State<AppState>, Path((code, id)): Path<(String, String)>, headers: HeaderMap) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    super::wall::wall_caps(&s)?;
    let host = match host_credential(&code, &headers) {
        Some(key) => super::host_role(&state, &s, key).await?,
        None => None,
    };
    let viewer = match host {
        Some(super::HostRole::Host) => Viewer::Host,
        Some(super::HostRole::Screen) => Viewer::BigScreen,
        None => Viewer::Guest(guest(&state, &code, &headers).await?.ok_or_else(signed_out)?.id),
    };
    super::wall::photo_response(&state, &s, &id, &viewer, &headers).await
}

#[derive(Debug, Deserialize)]
pub struct PlayerRef {
    pub player_id: String,
}

async fn host_remove(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Json(req): Json<PlayerRef>) -> PResult<Response> {
    host(&state, &code, &headers).await?;
    super::remove_player(&state, &code, &req.player_id).await?;
    let s = pocket(&state, &code).await?;
    Ok(Json(super::view_json(&s, &Viewer::Host, Utc::now())).into_response())
}

/// The one-time reset code, shown once. The answer names nothing else about the player.
async fn host_reset_pin(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Json(req): Json<PlayerRef>) -> PResult<Json<Value>> {
    host(&state, &code, &headers).await?;
    let (reset, expires_at) = super::reset_pin(&state, &code, &req.player_id).await?;
    Ok(Json(json!({ "code": reset, "expires_at": expires_at })))
}

async fn host_end(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap) -> PResult<Json<Value>> {
    host(&state, &code, &headers).await?;
    let s = super::end(&state, &code).await?;
    Ok(Json(super::view_json(&s, &Viewer::Host, Utc::now())))
}

/// Who a WebSocket is for, checked again whenever keys may have changed.
#[derive(Clone)]
enum WsWho {
    Guest { key: String, id: String },
    Host { key: String, screen: bool },
}

impl WsWho {
    fn viewer(&self) -> Viewer {
        match self {
            WsWho::Guest { id, .. } => Viewer::Guest(id.clone()),
            WsWho::Host { screen: true, .. } => Viewer::BigScreen,
            WsWho::Host { .. } => Viewer::Host,
        }
    }

    async fn still_valid(&self, state: &AppState, code: &str) -> bool {
        match self {
            WsWho::Guest { key, id } => matches!(super::guest_for_key(state, code, key).await, Ok(Some(p)) if &p.id == id),
            WsWho::Host { key, .. } => match super::snapshot(state, code).await {
                Ok(Some(s)) => matches!(super::host_role(state, &s, key).await, Ok(Some(_))),
                _ => false,
            },
        }
    }

    /// The host removed this guest (not a PIN reset or a rotated host key).
    async fn removed(&self, state: &AppState, code: &str) -> bool {
        match self {
            WsWho::Guest { id, .. } => matches!(state.db.pocket_player(code, id).await, Ok(Some(p)) if p.removed_at.is_some()),
            WsWho::Host { .. } => false,
        }
    }
}

/// Live state: `{"type":"state", …the state JSON}` on every change this viewer can see,
/// `{"type":"removed"}` when the host removed this guest, `{"type":"signed_out"}` when the key
/// otherwise stops working (PIN reset, host key rotated), `{"type":"gone"}` when the pocket is
/// deleted. The host's remote and big screen pass `?view=host` / `?view=screen` with the host key
/// or a link session; a screen session gets 403 `screen_only` for `view=host`.
async fn ws(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    Query(q): Query<StateQuery>,
    upgrade: WebSocketUpgrade,
) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    let who = match q.view.as_deref() {
        Some(view @ ("host" | "screen")) => {
            let key = host_credential(&code, &headers).ok_or_else(not_host)?;
            let role = super::host_role(&state, &s, key).await?.ok_or_else(not_host)?;
            if role == super::HostRole::Screen && view != "screen" {
                return Err(screen_only());
            }
            WsWho::Host { key: key.to_string(), screen: view == "screen" }
        }
        _ => {
            let key = cookie(&headers, &guest_cookie(&code)).unwrap_or("").to_string();
            let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
            WsWho::Guest { key, id: p.id }
        }
    };
    Ok(upgrade.max_message_size(WS_MAX_MESSAGE).on_upgrade(move |socket| serve_ws(socket, state, code, who)))
}

fn ws_state(s: &Snapshot, viewer: &Viewer) -> Value {
    let mut v = super::view_json(s, viewer, Utc::now());
    v["type"] = json!("state");
    v
}

async fn serve_ws(mut socket: WebSocket, state: AppState, code: String, who: WsWho) {
    let mut rx = state.pocket.subscribe(&code);
    let viewer = who.viewer();
    let mut last = Value::Null;
    let mut ping = tokio::time::interval(WS_PING);
    ping.tick().await;
    let mut send_state = true;
    loop {
        if send_state {
            send_state = false;
            match super::snapshot(&state, &code).await {
                Ok(Some(s)) => {
                    let mut v = ws_state(&s, &viewer);
                    // The version alone changing (another guest's private step) isn't news here
                    let mut cmp = v.clone();
                    cmp["version"] = Value::Null;
                    if cmp != last {
                        last = cmp;
                        if socket.send(Message::Text(v.take().to_string())).await.is_err() {
                            return;
                        }
                    }
                }
                Ok(None) => {
                    let _ = socket.send(Message::Text(json!({ "type": "gone" }).to_string())).await;
                    return;
                }
                Err(_) => {}
            }
        }
        tokio::select! {
            sig = rx.recv() => match sig {
                Ok(super::Signal::Changed(_)) | Err(RecvError::Lagged(_)) => send_state = true,
                Ok(super::Signal::Revoked(_)) => {
                    if !who.still_valid(&state, &code).await {
                        // A removed guest gets its own frame, so the page shows "removed" at once
                        let kind = if who.removed(&state, &code).await { "removed" } else { "signed_out" };
                        let _ = socket.send(Message::Text(json!({ "type": kind }).to_string())).await;
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                }
                Ok(super::Signal::Deleted) | Err(RecvError::Closed) => {
                    let _ = socket.send(Message::Text(json!({ "type": "gone" }).to_string())).await;
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Vec::new())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => match msg {
                // Pages act over POST; anything they send here is ignored
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn files(name: &str) -> Option<Cow<'static, [u8]>> {
        match name {
            "g-3f9a1c.js" => Some(Cow::Borrowed(b"console.log(1)")),
            "g-77aa01.css" => Some(Cow::Borrowed(b"body{}")),
            "inter-guest-0b1c.woff2" => Some(Cow::Borrowed(b"wOF2")),
            "index.html" => Some(Cow::Borrowed(b"<!doctype html>")),
            _ => None,
        }
    }

    #[tokio::test]
    async fn hashed_files_are_immutable_with_their_type() {
        for (name, mime) in [
            ("g-3f9a1c.js", "application/javascript; charset=utf-8"),
            ("g-77aa01.css", "text/css; charset=utf-8"),
            ("inter-guest-0b1c.woff2", "font/woff2"),
        ] {
            let res = page_file_response(name, files);
            assert_eq!(res.status(), StatusCode::OK, "{name}");
            assert_eq!(res.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
            assert_eq!(res.headers()[header::CONTENT_TYPE], mime);
            assert!(!to_bytes(res.into_body(), usize::MAX).await.unwrap().is_empty());
        }
    }

    #[test]
    fn the_page_itself_and_anything_outside_the_folder_are_404() {
        // `..%2Fx` reaches the handler decoded as `../x`
        for name in ["index.html", "../x", "..", "a/g-3f9a1c.js", "a\\g-3f9a1c.js", "..\\x.js", ".hidden.js", "g-0000.js", "x.exe", "noext", ""] {
            let res = page_file_response(name, files);
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{name:?}");
            assert!(res.headers().get(header::CACHE_CONTROL).is_none(), "{name:?}: the gate adds no-store");
        }
    }

    #[test]
    fn the_boot_json_cannot_close_its_script() {
        let page = r#"<html><body><div id="app"></div><script type="application/json" id="pocket-boot">null</script><script src="/p/_a/g-1.js"></script></body></html>"#;
        let evil = json!({ "pocket": { "title": "</script><script>alert(1)</script><!-- & \u{2028}" } });
        let out = fill_boot(page, &evil);
        let open = r#"<script type="application/json" id="pocket-boot">"#;
        let start = out.find(open).unwrap() + open.len();
        let end = start + out[start..].find("</script>").unwrap();
        let slot = &out[start..end];
        assert!(!slot.contains('<') && !slot.contains('>') && !slot.contains('&') && !slot.contains('\u{2028}'), "{slot}");
        assert_eq!(serde_json::from_str::<Value>(slot).unwrap(), evil, "the page reads back the same JSON");
        assert!(out.ends_with(r#"<script src="/p/_a/g-1.js"></script></body></html>"#));
        assert_eq!(out.matches("</script>").count(), 2);
        // No slot: the page goes out as built
        assert_eq!(fill_boot("<html></html>", &evil), "<html></html>");
    }
}
