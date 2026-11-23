//! The guest port's router (Pocket API v1). It answers only `GET /p/<code>` and
//! `/api/pocket/<code>/*`; everything else is 404, and so is everything while the switch is off.
//! There is no `auth::guard` here: guests have no account. A guest key (cookie `kp_<code>`) or a
//! host key (cookie `kph_<code>`, or `Authorization: Bearer`) opens one pocket only, and neither
//! is ever accepted on the main port.

use super::kits::Viewer;
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
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
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

fn host_key<'a>(code: &str, headers: &'a HeaderMap) -> Option<&'a str> {
    bearer(headers).or_else(|| cookie(headers, &host_cookie(code))).filter(|k| !k.is_empty())
}

async fn host(state: &AppState, code: &str, headers: &HeaderMap) -> PResult<Arc<Snapshot>> {
    let s = pocket(state, code).await?;
    match host_key(code, headers) {
        Some(k) if super::is_host_key(&s, k) => Ok(s),
        _ => Err(PocketError::new(StatusCode::UNAUTHORIZED, "not_host", "Only the host's remote can do this.")),
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

/// The guest page. pa5 replaces this placeholder; it has no inline script or style, as the CSP needs.
async fn page(State(state): State<AppState>, Path(code): Path<String>) -> Response {
    let (status, title, line) = match pocket(&state, &code).await {
        Ok(s) if s.row.state == crate::db::pocket::ENDED => (StatusCode::OK, s.row.title.clone(), "This pocket app has ended."),
        Ok(s) => (StatusCode::OK, s.row.title.clone(), "This pocket app's page is on its way."),
        Err(_) => (StatusCode::NOT_FOUND, "Not found".to_string(), "There's no pocket app here."),
    };
    let title = html_escape(&title);
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"referrer\" content=\"no-referrer\"><title>{title}</title></head><body><main><h1>{title}</h1><p>{line}</p></main></body></html>"
    );
    (status, Html(body)).into_response()
}

/// What anyone with the link sees before joining: no names, no guest text.
async fn info(State(state): State<AppState>, Path(code): Path<String>) -> PResult<Json<Value>> {
    let s = pocket(&state, &code).await?;
    Ok(Json(json!({ "api": super::API_VERSION, "pocket": super::pocket_json(&s) })))
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

async fn action(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Json(req): Json<ActionRequest>) -> PResult<Response> {
    pocket(&state, &code).await?;
    let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
    if !state.pocket.limits.action(&code, &p.id) {
        return Err(PocketError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", "Slow down a little."));
    }
    let viewer = Viewer::Guest(p.id);
    super::act(&state, &code, &viewer, &req.name, &req.body).await?;
    let s = pocket(&state, &code).await?;
    Ok(Json(super::view_json(&s, &viewer, Utc::now())).into_response())
}

async fn notices_seen(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap) -> PResult<StatusCode> {
    pocket(&state, &code).await?;
    let p = guest(&state, &code, &headers).await?.ok_or_else(signed_out)?;
    super::notices_seen(&state, &code, &p.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct HostSignin {
    pub key: String,
}

/// The host's remote trades the key from its link for the host cookie.
async fn host_signin(
    State(state): State<AppState>,
    Path(code): Path<String>,
    ci: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<HostSignin>,
) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    signin_limit(&state, peer(ci), &headers)?;
    if !super::is_host_key(&s, req.key.trim()) {
        return Err(PocketError::new(StatusCode::UNAUTHORIZED, "not_host", "That host link doesn't work any more."));
    }
    let body = super::view_json(&s, &Viewer::Host, Utc::now());
    Ok(signed_in(body, set_cookie(&host_cookie(&code), req.key.trim(), &code, &headers), StatusCode::OK))
}

fn host_viewer(q: &StateQuery) -> Viewer {
    if q.view.as_deref() == Some("screen") {
        Viewer::BigScreen
    } else {
        Viewer::Host
    }
}

async fn host_state(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Query(q): Query<StateQuery>) -> PResult<Response> {
    let s = host(&state, &code, &headers).await?;
    state.pocket.touch(&code);
    Ok(state_response(&s, &host_viewer(&q), q.since))
}

async fn host_action(State(state): State<AppState>, Path(code): Path<String>, headers: HeaderMap, Json(req): Json<ActionRequest>) -> PResult<Response> {
    host(&state, &code, &headers).await?;
    super::act(&state, &code, &Viewer::Host, &req.name, &req.body).await?;
    let s = pocket(&state, &code).await?;
    Ok(Json(super::view_json(&s, &Viewer::Host, Utc::now())).into_response())
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
            WsWho::Host { key, .. } => matches!(super::snapshot(state, code).await, Ok(Some(s)) if super::is_host_key(&s, key)),
        }
    }
}

/// Live state: `{"type":"state", …the state JSON}` on every change this viewer can see,
/// `{"type":"signed_out"}` when the key stops working, `{"type":"gone"}` when the pocket is
/// deleted. The host's remote and big screen pass `?view=host` / `?view=screen` with the host key.
async fn ws(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    Query(q): Query<StateQuery>,
    upgrade: WebSocketUpgrade,
) -> PResult<Response> {
    let s = pocket(&state, &code).await?;
    let who = match q.view.as_deref() {
        Some("host") | Some("screen") => {
            let key = host_key(&code, &headers).filter(|k| super::is_host_key(&s, k));
            let key = key.ok_or_else(|| PocketError::new(StatusCode::UNAUTHORIZED, "not_host", "Only the host's remote can do this."))?;
            WsWho::Host { key: key.to_string(), screen: q.view.as_deref() == Some("screen") }
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
                        let _ = socket.send(Message::Text(json!({ "type": "signed_out" }).to_string())).await;
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
