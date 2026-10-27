//! Who may call the API. Browsers may only call it from kwnt's own page (Origin allowlist), and
//! requests that don't come from this machine need the API token as a bearer header, or a session
//! in the `kwnt_token` cookie, which the UI gets by posting the token or a one-time sign-in link
//! code to `/api/auth/session` (see `signin`).
//!
//! "From this machine" means a loopback peer, a loopback `Host` and no proxy forwarding headers.
//! The `Host` check is what keeps a DNS-rebinding page (same peer, foreign host) out.

use super::signin::{self, Poll, Redeem, Via};
use super::AppState;
use crate::config::Config;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

pub const COOKIE: &str = "kwnt_token";
const TOKEN_FILE: &str = "api-token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Require {
    /// Requests from other machines need the token (default).
    Auto,
    /// Every API request needs the token, local ones too.
    Always,
    /// No token at all (only for setups behind their own auth).
    Never,
}

pub struct ApiToken {
    token: String,
    pub require: Require,
    /// Extra browser origins allowed besides kwnt's own (`KWNT_ALLOWED_ORIGINS`).
    extra_origins: Vec<String>,
    port: u16,
    /// Sessions and one-time sign-in links.
    pub signin: signin::SignIn,
}

impl std::fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiToken")
            .field("require", &self.require)
            .finish_non_exhaustive()
    }
}

impl ApiToken {
    /// `KWNT_API_TOKEN`, else `<data_dir>/api-token` (created with a random token, mode 0600).
    pub fn load(config: &Config) -> Self {
        let require = match std::env::var("KWNT_REQUIRE_TOKEN")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "always" | "1" | "true" => Require::Always,
            "never" | "0" | "false" => Require::Never,
            _ => Require::Auto,
        };
        let extra_origins = std::env::var("KWNT_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|o| o.trim().trim_end_matches('/').to_string())
            .filter(|o| !o.is_empty())
            .collect();
        let token = env_token().unwrap_or_else(|| load_or_create(&config.data_dir));
        Self {
            token,
            require,
            extra_origins,
            port: config.port,
            signin: signin::SignIn::load(&config.data_dir),
        }
    }

    pub fn for_tests(token: &str, require: Require, port: u16) -> Self {
        Self {
            token: token.into(),
            require,
            extra_origins: Vec::new(),
            port,
            signin: signin::SignIn::in_memory(),
        }
    }

    /// kwnt's public address: the first `KWNT_ALLOWED_ORIGINS` entry (the HTTPS address in front
    /// of it), for sign-in links that open as they are.
    pub fn public_base(&self) -> Option<&str> {
        self.extra_origins.first().map(String::as_str)
    }

    /// The token, for `kwnt token` and the startup URL only.
    pub fn reveal(&self) -> &str {
        &self.token
    }

    pub fn matches(&self, candidate: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), candidate.as_bytes())
    }

    fn origin_allowed(&self, origin: &str, host: Option<&str>) -> bool {
        let origin = origin.trim_end_matches('/');
        let own = [self.port, 5199]
            .iter()
            .flat_map(|p| {
                [
                    format!("http://127.0.0.1:{p}"),
                    format!("http://localhost:{p}"),
                    format!("http://[::1]:{p}"),
                ]
            })
            .any(|o| o == origin);
        let same_host = host
            .is_some_and(|h| origin == format!("http://{h}") || origin == format!("https://{h}"));
        own || same_host || self.extra_origins.iter().any(|o| o == origin)
    }
}

/// `KWNT_API_TOKEN` when it is set to a usable token (16+ characters).
pub fn env_token() -> Option<String> {
    std::env::var("KWNT_API_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| t.len() >= 16)
}

/// Why the token file could not be read. `kwnt token` reports these instead of writing a new
/// token: one made by another user or for another data dir is a token the server rejects.
#[derive(Debug)]
pub enum ReadTokenError {
    Missing(PathBuf),
    Unreadable(PathBuf, std::io::Error),
    Invalid(PathBuf),
}

impl std::fmt::Display for ReadTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(p) => write!(f, "no token file at {}", p.display()),
            Self::Unreadable(p, e) => write!(f, "can't read {}: {e}", p.display()),
            Self::Invalid(p) => write!(f, "{} doesn't hold a valid token", p.display()),
        }
    }
}

impl std::error::Error for ReadTokenError {}

/// Reads `<data_dir>/api-token` without ever creating or changing it.
pub fn read_existing(data_dir: &Path) -> Result<String, ReadTokenError> {
    let path = data_dir.join(TOKEN_FILE);
    match std::fs::read_to_string(&path) {
        Ok(t) if t.trim().len() >= 16 => Ok(t.trim().to_string()),
        Ok(_) => Err(ReadTokenError::Invalid(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ReadTokenError::Missing(path)),
        Err(e) => Err(ReadTokenError::Unreadable(path, e)),
    }
}

/// Reads the token file, or writes a new random one. Falls back to an in-memory token.
/// Only the server calls this; `kwnt token` uses [`read_existing`].
pub fn load_or_create(data_dir: &Path) -> String {
    if let Ok(t) = read_existing(data_dir) {
        return t;
    }
    let path = data_dir.join(TOKEN_FILE);
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let _ = std::fs::create_dir_all(data_dir);
    if let Err(e) = write_private(&path, &token) {
        tracing::warn!(error = %e, "could not save the API token; it changes on restart");
    }
    token
}

/// Whether `kwnt serve --host <host>` listens on this machine only.
pub fn is_loopback_bind(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The link that signs a browser in: `http://<host>:<port>/#kwnt_token=<token or code>`. The UI
/// posts what follows `#kwnt_token=` to `/api/auth/session` and clears it from the address bar;
/// a fragment never reaches access logs or a `Referer`.
pub fn sign_in_link(host: &str, port: u16, token: &str) -> String {
    let bare = host.trim_matches(['[', ']']);
    let host = match bare.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{bare}]"),
        _ => bare.to_string(),
    };
    format!("http://{host}:{port}/#kwnt_token={token}")
}

/// A one-time link: on `base` (kwnt's public address) when known, else on the bound host
/// (`<this-host>` when kwnt listens on every address).
pub fn one_time_link(host: &str, port: u16, base: Option<&str>, code: &str) -> String {
    match base {
        Some(b) => format!("{}/#kwnt_token={code}", b.trim_end_matches('/')),
        None => token_command_link(host, port, code),
    }
}

/// Whether `kwnt serve` should print a sign-in link: not when nobody needs one (loopback, unless
/// `KWNT_REQUIRE_TOKEN=always`, or `never`).
pub fn startup_link_needed(host: &str, require: Require) -> bool {
    match require {
        Require::Never => false,
        Require::Auto => !is_loopback_bind(host),
        Require::Always => true,
    }
}

/// What `kwnt serve` and `kwnt signin-link` say about a one-time link, for stdout only (never
/// through tracing into log files): the link, a QR code for a phone when the address is real, and
/// how to get another.
pub fn one_time_notice(link: &str, qr: bool) -> Vec<String> {
    let minutes = signin::LINK_TTL.as_secs() / 60;
    let mut lines = vec![format!(
        "🔒 To sign in from another device, open this link. It works once, for {minutes} minutes (keep it private): {link}"
    )];
    if link.contains(THIS_HOST) {
        lines.push(format!("   Replace {THIS_HOST} with this server's address. Set KWNT_ALLOWED_ORIGINS to your HTTPS address for a ready link and a QR code."));
    } else if qr {
        let code = signin::qr_lines(link);
        if !code.is_empty() {
            lines.push("   Or scan it with your phone:".into());
            lines.extend(code);
        }
    }
    lines.push("   Used or expired? `kwnt signin-link` makes a new one (Docker: docker compose exec kwnt kwnt signin-link).".into());
    lines
}

/// Stands in for the server's address when kwnt listens on every address.
pub const THIS_HOST: &str = "<this-host>";

/// `0.0.0.0` or `::`: every address, so the bound host says nothing about how to reach it.
fn is_any_bind(host: &str) -> bool {
    host.is_empty()
        || host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_unspecified())
}

/// A sign-in link on the configured bind (`<this-host>` when kwnt listens on every address).
pub fn token_command_link(host: &str, port: u16, token: &str) -> String {
    sign_in_link(if is_any_bind(host) { THIS_HOST } else { host }, port, token)
}

pub(crate) fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_loopback_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host),
    };
    name.eq_ignore_ascii_case("localhost")
        || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Paths that never need the token: health, the login endpoints themselves, the OAuth redirect
/// (the provider's page sends the browser there) and the MCP endpoints, which carry their own
/// per-turn tokens.
fn exempt(path: &str) -> bool {
    path == "/api/health"
        || path.starts_with("/api/auth/")
        || path == "/api/mcp/oauth/callback"
        || path.starts_with("/api/mcp/permission/")
        || path.starts_with("/api/mcp/gateway")
        || path == "/api/mcp/self"
}

/// Routes only the person may call: saved passwords and the password vault (Bitwarden), changing app-wide settings, what bots wait on the person for, answering an approval card, Loose ends, Connections and Parallel Universe, the
/// approvals switch, and deleting a bot or team or undoing a bot action or import (those delete saved passwords too). A bot's turn token (or any
/// bearer that isn't the API token) is refused with 403 everywhere. From another machine they need the API token (401);
/// from this machine they don't, unless `KWNT_REQUIRE_TOKEN=always` (`person_token_needed`).
pub fn person_only(method: &Method, path: &str) -> bool {
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    match segs.as_slice() {
        ["", "api", "sandbox", _, "passwords", ..] => true,
        ["", "api", "passwords", ..] => true,
        // Loose ends read the person's mail: bots never see or change them
        ["", "api", "zombie", ..] => true,
        // Connections list saved logins and add Composio, which holds the person's app sign-ins
        ["", "api", "connections", ..] => true,
        // Parallel Universe starts containers on this machine from a directory the person names
        ["", "api", "parallel", ..] => true,
        // Camera-to-Completed: the person's letters
        ["", "api", "documents", ..] => true,
        // Setup installs software on this machine and shows what is signed in
        ["", "api", "setup", ..] => true,
        // Sign-in links, device approvals and sessions: who else may sign in as the person
        ["", "api", "signin", ..] => true,
        // Web Push: where the person's notifications go
        ["", "api", "push", ..] => true,
        // What the bots wait on the person for: their questions, across every chat
        ["", "api", "waiting", ..] => true,
        // Starting work-ahead runs now spends the person's tokens
        ["", "api", "work-ahead", ..] => true,
        // Mission Control: approving plans, verifying, rejecting, retrying and approving checks are
        // the person's calls; bots act on tasks only through their task tools
        ["", "api", "missions" | "tasks" | "checks", ..] => *method != Method::GET && *method != Method::HEAD,
        // App-wide settings, including where notifications go (`notify_webhook_url`), and the
        // sub-routes such as `/computer` (which machine the bots run on) and `/computer/window`:
        // bots change the allowed ones through their `settings_update` action, never these routes
        ["", "api", "settings", ..] => *method != Method::GET && *method != Method::HEAD,
        // Carrying on a chain that went over its token budget is the person's call
        ["", "api", "chains", _, "continue"] => *method == Method::POST,
        // Changing a choice a bot made is the person's call
        ["", "api", "messages", _, "decisions", ..] => *method != Method::GET && *method != Method::HEAD,
        // The Mac app's update: check and install (which swaps and relaunches the app) are the
        // person's; the status stays readable
        ["", "api", "app", "update", ..] => *method != Method::GET && *method != Method::HEAD,
        ["", "api", "approvals", _, "resolve"] => *method == Method::POST,
        ["", "api", "spaces" | "kwants", _, "approvals"] => *method != Method::GET && *method != Method::HEAD,
        ["", "api", "kwants" | "teams", _] => *method == Method::DELETE,
        ["", "api", "bot-actions" | "imports", _, "undo"] => *method == Method::POST,
        _ => false,
    }
}

fn header_str(headers: &HeaderMap, name: impl header::AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Whether the request comes from this machine, not through a proxy, to a loopback name.
pub fn is_local(headers: &HeaderMap, peer: Option<IpAddr>) -> bool {
    let forwarded = [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-real-ip",
    ]
    .iter()
    .any(|h| headers.contains_key(*h));
    let host_ok = header_str(headers, header::HOST).is_none_or(is_loopback_host);
    peer.is_none_or(|ip| ip.is_loopback() || ip.to_canonical().is_loopback())
        && !forwarded
        && host_ok
}

pub(super) fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| {
            c.trim()
                .strip_prefix(COOKIE)
                .and_then(|r| r.strip_prefix('='))
        })
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    header_str(headers, header::AUTHORIZATION)
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

/// The API token as a bearer, or a cookie holding a live session (or the API token itself, as
/// browsers signed in before sessions still have).
fn presented_token_ok(api: &ApiToken, headers: &HeaderMap) -> bool {
    bearer(headers).is_some_and(|t| api.matches(t))
        || cookie_token(headers).is_some_and(|t| api.matches(t) || api.signin.session_ok(t))
}

fn needs_token(api: &ApiToken, headers: &HeaderMap, peer: Option<IpAddr>) -> bool {
    match api.require {
        Require::Never => false,
        Require::Always => true,
        Require::Auto => !is_local(headers, peer),
    }
}

/// Whether a person-only route needs the API token for this request. Not from this machine:
/// the person there shouldn't be asked for a token to answer their own approval cards. A bot's
/// shell there could reach these routes with a bare `curl`, but it could read the token file just
/// as well; `KWNT_REQUIRE_TOKEN=always` asks for the token on loopback too.
fn person_token_needed(api: &ApiToken, headers: &HeaderMap, peer: Option<IpAddr>) -> bool {
    api.require != Require::Never && needs_token(api, headers, peer)
}

fn deny(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// 401 for a person-only route without the token: `{error, person_only: true}`.
/// For a handler guarding something person-only that `person_only` can't see from the path (a
/// letter's pages under `/api/attachments`): the API token must be presented where
/// `person_token_needed` says so.
#[allow(clippy::result_large_err)]
pub(super) fn require_person_token(state: &AppState, headers: &HeaderMap, peer: Option<IpAddr>) -> Result<(), Response> {
    if person_token_needed(&state.api_token, headers, peer) && !presented_token_ok(&state.api_token, headers) {
        return Err(person_token_required());
    }
    Ok(())
}

fn person_token_required() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "this needs the API token: open the sign-in link kwant printed when it started, or run `kwnt token`", "person_only": true })),
    )
        .into_response()
}

/// Origin check, CORS for allowed origins, and the token check, for every `/api` request.
pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if !path.starts_with("/api/") && path != "/api" {
        return next.run(req).await;
    }
    let api = state.api_token.clone();
    let headers = req.headers().clone();
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let host = header_str(&headers, header::HOST);
    let origin = header_str(&headers, header::ORIGIN).map(str::to_string);

    if let Some(o) = &origin {
        if !api.origin_allowed(o, host) {
            return deny(StatusCode::FORBIDDEN, "origin not allowed");
        }
    }
    let preflight = req.method() == Method::OPTIONS
        && headers.contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut resp = if preflight {
        StatusCode::NO_CONTENT.into_response()
    } else if !exempt(&path)
        && needs_token(&api, &headers, peer)
        && !presented_token_ok(&api, &headers)
    {
        deny(StatusCode::UNAUTHORIZED, "API token required")
    } else if api.require != Require::Never && person_only(req.method(), &path) {
        // A bot's turn token (or any bearer that isn't the API token) is a 403, from anywhere;
        // no token a 401 where the token is needed
        if let Err(refused) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person can do this").await {
            refused
        } else if person_token_needed(&api, &headers, peer) && !presented_token_ok(&api, &headers) {
            person_token_required()
        } else {
            next.run(req).await
        }
    } else {
        next.run(req).await
    };
    if let Some(o) = origin.and_then(|o| HeaderValue::from_str(&o).ok()) {
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
        h.insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        h.append(header::VARY, HeaderValue::from_static("Origin"));
        if preflight {
            h.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            );
            h.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("authorization, content-type"),
            );
            h.insert(
                header::ACCESS_CONTROL_MAX_AGE,
                HeaderValue::from_static("600"),
            );
        }
    }
    resp
}

/// `GET /api/auth/status`: `{auth_required, authenticated, person_auth_required,
/// person_authenticated}` for this request.
pub async fn status(State(state): State<AppState>, req: Request) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let headers = req.headers();
    let required = needs_token(&state.api_token, headers, peer);
    let has_token = presented_token_ok(&state.api_token, headers);
    let authenticated = !required || has_token;
    // Person-only actions (passwords, approvals) need it from another machine, or with `always`
    let person_required = person_token_needed(&state.api_token, headers, peer);
    Json(json!({
        "auth_required": required,
        "authenticated": authenticated,
        "person_auth_required": person_required,
        "person_authenticated": !person_required || has_token,
        // This server can let a new browser in from a signed-in device (`/api/auth/device`)
        "device_login": true,
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionBody {
    token: String,
}

fn session_cookie(value: &str, max_age: u64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("kwnt_token=; Max-Age=0; Path=/"))
}

fn is_https(headers: &HeaderMap) -> bool {
    header_str(headers, "x-forwarded-proto").is_some_and(|p| p.eq_ignore_ascii_case("https"))
}

fn peer_ip(peer: &Option<ConnectInfo<SocketAddr>>) -> Option<IpAddr> {
    peer.as_ref().map(|c| c.0.ip())
}

/// A response that signs this browser in with a new session's cookie (HttpOnly, SameSite=Strict),
/// which EventSource and the browser WebSocket then send.
pub(super) fn with_session_cookie(mut resp: Response, secret: &str, headers: &HeaderMap) -> Response {
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(secret, signin::SESSION_SECS, is_https(headers)),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// `POST /api/auth/session {token}`: `token` is the API token or a one-time sign-in link code.
/// Either starts a new session for this browser (the cookie never holds the API token). 401
/// `{error, reason}`: `wrong_token`, or `link_used_or_expired` for a link that worked once.
pub async fn create_session(
    State(state): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<SessionBody>,
) -> Response {
    let token = body.token.trim();
    let client = signin::client(&headers, peer_ip(&peer));
    let secret = if state.api_token.matches(token) {
        state.api_token.signin.create_session(Via::Token, &client)
    } else {
        match state.api_token.signin.redeem_link(token, &client) {
            Redeem::Ok(secret) => secret,
            refused => {
                // A wrong guess costs a little time
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let (msg, reason) = if refused == Redeem::Spent {
                    ("this sign-in link was already used or has expired", "link_used_or_expired")
                } else {
                    ("wrong token", "wrong_token")
                };
                return (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg, "reason": reason }))).into_response();
            }
        }
    };
    with_session_cookie(StatusCode::NO_CONTENT.into_response(), &secret, &headers)
}

/// `DELETE /api/auth/session`: ends this browser's session and clears the cookie.
pub async fn delete_session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(secret) = cookie_token(&headers) {
        if let Some(id) = state.api_token.signin.session_id(secret) {
            state.push.drop_session(&id);
        }
        state.api_token.signin.revoke_secret(secret);
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie("", 0, is_https(&headers)),
    );
    resp
}

/// `POST /api/signin/links` (person only): a one-time sign-in link for another device, live for
/// 15 minutes: `{code, url, expires_at, expires_in}`. `url` is null when kwnt doesn't know its
/// public address (no `KWNT_ALLOWED_ORIGINS`); the page then builds it from its own address.
pub async fn create_link(State(state): State<AppState>) -> Response {
    let code = state.api_token.signin.create_link(signin::LINK_TTL);
    let ttl = signin::LINK_TTL.as_secs();
    let url = state
        .api_token
        .public_base()
        .map(|b| one_time_link("", 0, Some(b), &code));
    let mut resp = Json(json!({
        "code": code,
        "url": url,
        "expires_in": ttl,
        "expires_at": chrono::Utc::now() + chrono::TimeDelta::seconds(ttl as i64),
    }))
    .into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// `POST /api/auth/device {}` (open to anyone): a new browser asks to be let in.
/// `{device_code, user_code, expires_in, interval}`; the browser keeps `device_code` to itself,
/// shows `user_code`, and polls. 429 `{error, retry_after}` (and `Retry-After`) when this address
/// asked too often or too many requests are waiting.
pub async fn device_start(
    State(state): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    let client = signin::client(&headers, peer_ip(&peer));
    match state.api_token.signin.start_device(&client) {
        Ok((device_code, user_code)) => {
            let mut resp = Json(json!({
                "device_code": device_code,
                "user_code": user_code,
                "expires_in": signin::DEVICE_TTL.as_secs(),
                "interval": signin::DEVICE_INTERVAL_SECS,
            }))
            .into_response();
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(wait) => {
            let mut resp = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": "too many sign-in requests; try again shortly", "retry_after": wait })),
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&wait.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
            resp
        }
    }
}

#[derive(Deserialize)]
pub struct DevicePollBody {
    device_code: String,
}

/// `POST /api/auth/device/poll {device_code}`: 200 `{status: "pending"}`; 200
/// `{status: "approved"}` with the session cookie (once); 403 `{status: "denied"}` (once); 410
/// `{status: "expired"}` for one that ran out, was collected, or never existed.
pub async fn device_poll(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DevicePollBody>,
) -> Response {
    match state.api_token.signin.poll_device(body.device_code.trim()) {
        Poll::Pending => Json(json!({ "status": "pending" })).into_response(),
        Poll::Approved(secret) => with_session_cookie(Json(json!({ "status": "approved" })).into_response(), &secret, &headers),
        Poll::Denied => (StatusCode::FORBIDDEN, Json(json!({ "status": "denied" }))).into_response(),
        Poll::Expired => (StatusCode::GONE, Json(json!({ "status": "expired" }))).into_response(),
    }
}

/// `GET /api/signin/requests` (person only): devices waiting to be let in, oldest first.
pub async fn device_requests(State(state): State<AppState>) -> Response {
    Json(state.api_token.signin.pending_devices()).into_response()
}

#[derive(Deserialize)]
pub struct UserCodeBody {
    user_code: String,
}

async fn decide(state: &AppState, body: UserCodeBody, approve: bool) -> Response {
    if state.api_token.signin.decide_device(&body.user_code, approve) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        deny(StatusCode::NOT_FOUND, "no waiting sign-in request with that code (it may have expired)")
    }
}

/// `POST /api/signin/requests/approve {user_code}` (person only): lets that device in. 404 when no
/// request with that code is waiting.
pub async fn device_approve(State(state): State<AppState>, Json(body): Json<UserCodeBody>) -> Response {
    decide(&state, body, true).await
}

/// `POST /api/signin/requests/deny {user_code}` (person only).
pub async fn device_deny(State(state): State<AppState>, Json(body): Json<UserCodeBody>) -> Response {
    decide(&state, body, false).await
}

/// `GET /api/signin/sessions` (person only): signed-in browsers, newest first, `current` for this one.
pub async fn sessions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let current = cookie_token(&headers).and_then(|c| state.api_token.signin.session_id(c));
    let list: Vec<_> = state
        .api_token
        .signin
        .sessions()
        .into_iter()
        .map(|s| {
            json!({
                "id": s.id,
                "browser": s.browser,
                "ip": s.ip,
                "via": s.via,
                "created_at": s.created_at,
                "last_seen": s.last_seen,
                "current": current.as_deref() == Some(s.id.as_str()),
            })
        })
        .collect();
    Json(list).into_response()
}

/// `DELETE /api/signin/sessions/:id` (person only): signs that browser out. 404 when unknown.
pub async fn revoke_session(State(state): State<AppState>, axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    // A signed-out phone stops getting notifications too
    state.push.drop_session(&id);
    if state.api_token.signin.revoke(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        deny(StatusCode::NOT_FOUND, "no such session")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn local_needs_loopback_peer_and_host_and_no_proxy() {
        let lo: Option<IpAddr> = Some("127.0.0.1".parse().unwrap());
        let remote: Option<IpAddr> = Some("203.0.113.9".parse().unwrap());
        assert!(is_local(&headers(&[("host", "127.0.0.1:3000")]), lo));
        assert!(is_local(&headers(&[("host", "localhost:3000")]), lo));
        assert!(is_local(
            &headers(&[("host", "[::1]:3000")]),
            Some("::1".parse().unwrap())
        ));
        assert!(is_local(&headers(&[]), None));
        assert!(!is_local(&headers(&[("host", "127.0.0.1:3000")]), remote));
        // DNS rebinding: loopback peer, foreign name
        assert!(!is_local(&headers(&[("host", "evil.example:3000")]), lo));
        // A reverse proxy on the same machine
        assert!(!is_local(
            &headers(&[
                ("host", "localhost:3000"),
                ("x-forwarded-for", "203.0.113.9")
            ]),
            lo
        ));
        assert!(!is_local(
            &headers(&[("host", "127.0.0.1.evil.example")]),
            lo
        ));
    }

    #[test]
    fn origins() {
        let api = ApiToken::for_tests("t".repeat(32).as_str(), Require::Auto, 3000);
        assert!(api.origin_allowed("http://localhost:3000", None));
        assert!(api.origin_allowed("http://127.0.0.1:5199/", None));
        assert!(api.origin_allowed("https://kwnt.example.com", Some("kwnt.example.com")));
        assert!(!api.origin_allowed("https://evil.example", Some("kwnt.example.com")));
        assert!(!api.origin_allowed("http://localhost:3001", Some("localhost:3000")));
        assert!(!api.origin_allowed("null", None));
    }

    #[test]
    fn tokens_and_cookies() {
        let api = ApiToken::for_tests("abcdefghijklmnopqrstuvwxyz012345", Require::Auto, 3000);
        assert!(presented_token_ok(
            &api,
            &headers(&[("authorization", "Bearer abcdefghijklmnopqrstuvwxyz012345")])
        ));
        assert!(presented_token_ok(
            &api,
            &headers(&[("cookie", "a=b; kwnt_token=abcdefghijklmnopqrstuvwxyz012345")])
        ));
        assert!(!presented_token_ok(
            &api,
            &headers(&[("cookie", "kwnt_token=abcdefghijklmnopqrstuvwxyz01234")])
        ));
        assert!(!presented_token_ok(
            &api,
            &headers(&[("authorization", "Basic abc")])
        ));
        assert!(!format!("{api:?}").contains("abcdef"));
    }

    #[test]
    fn person_only_routes() {
        let (get, post, put, del) = (Method::GET, Method::POST, Method::PUT, Method::DELETE);
        assert!(person_only(&get, "/api/sandbox/t1/passwords"));
        assert!(person_only(&post, "/api/sandbox/t1/passwords/c1/reveal"));
        assert!(person_only(&del, "/api/sandbox/t1/passwords/c1"));
        assert!(person_only(&get, "/api/passwords/store"));
        assert!(person_only(&post, "/api/messages/m1/decisions/0/change"));
        assert!(person_only(&post, "/api/work-ahead/run"));
        assert!(person_only(&post, "/api/missions"));
        assert!(person_only(&post, "/api/missions/m1/approve"));
        assert!(person_only(&post, "/api/missions/m1/run"));
        assert!(person_only(&Method::PATCH, "/api/missions/m1"));
        assert!(person_only(&post, "/api/tasks/t1/verify"));
        assert!(person_only(&post, "/api/tasks/t1/checks/run"));
        assert!(person_only(&post, "/api/checks/c1/approve"));
        assert!(person_only(&post, "/api/tasks/t1/fork"));
        assert!(person_only(&post, "/api/tasks/t1/flags/f1/resolve"));
        assert!(person_only(&post, "/api/missions/m1/graph/scan"));
        assert!(!person_only(&get, "/api/missions/m1/graph"));
        assert!(!person_only(&get, "/api/tasks/t1/flags"));
        assert!(!person_only(&get, "/api/tasks"));
        assert!(!person_only(&get, "/api/missions/insights"));
        assert!(person_only(&post, "/api/passwords/bitwarden/unlock"));
        assert!(person_only(&del, "/api/passwords/bitwarden"));
        assert!(person_only(&post, "/api/passwords/migrate"));
        assert!(person_only(&get, "/api/zombie/cases"));
        assert!(person_only(&post, "/api/zombie/sources/s1/test"));
        assert!(person_only(&get, "/api/connections"));
        assert!(person_only(&get, "/api/setup/status"));
        assert!(person_only(&post, "/api/setup/install"));
        assert!(person_only(&get, "/api/setup/jobs/setup-1"));
        assert!(person_only(&get, "/api/push/key"));
        assert!(person_only(&post, "/api/push/subscribe"));
        assert!(person_only(&post, "/api/push/unsubscribe"));
        assert!(person_only(&get, "/api/waiting"));
        assert!(person_only(&get, "/api/waiting/"));
        assert!(person_only(&post, "/api/connections/composio"));
        assert!(person_only(&get, "/api/parallel/prices"));
        assert!(person_only(&post, "/api/parallel/scenarios"));
        assert!(person_only(&post, "/api/documents"));
        assert!(person_only(&get, "/api/documents/d1"));
        assert!(person_only(&post, "/api/approvals/a1/resolve"));
        assert!(person_only(&put, "/api/spaces/s1/approvals"));
        assert!(person_only(&put, "/api/kwants/k1/approvals"));
        assert!(!person_only(&get, "/api/kwants/k1/approvals"));
        assert!(person_only(&post, "/api/settings"));
        assert!(person_only(&post, "/api/chains/m1/continue"));
        assert!(person_only(&put, "/api/settings/"));
        assert!(!person_only(&get, "/api/settings"));
        assert!(person_only(&put, "/api/settings/computer"));
        assert!(person_only(&put, "/api/settings/computer/"));
        assert!(person_only(&put, "/api/settings/computer/window"));
        assert!(!person_only(&get, "/api/settings/computer"));
        assert!(person_only(&post, "/api/app/update/check"));
        assert!(person_only(&post, "/api/app/update/install"));
        assert!(person_only(&post, "/api/app/update/install/"));
        assert!(!person_only(&get, "/api/app/update"));
        assert!(!person_only(&Method::HEAD, "/api/app/update"));
        assert!(!person_only(&get, "/api/approvals"));
        assert!(!person_only(&get, "/api/sandbox/t1/status"));
        assert!(!person_only(&get, "/api/kwants"));
        assert!(person_only(&del, "/api/kwants/k1"));
        assert!(person_only(&del, "/api/teams/t1/"));
        assert!(person_only(&post, "/api/bot-actions/b1/undo"));
        assert!(person_only(&post, "/api/imports/i1/undo"));
        assert!(!person_only(&put, "/api/kwants/k1"));
        assert!(!person_only(&get, "/api/teams/t1"));
        assert!(!person_only(&post, "/api/bot-actions/b1/confirm"));
    }

    #[test]
    fn startup_link_only_where_someone_needs_it() {
        for host in ["127.0.0.1", "localhost", "::1", "[::1]"] {
            assert!(!startup_link_needed(host, Require::Auto), "{host}");
            assert!(!startup_link_needed(host, Require::Never), "{host}");
            assert!(startup_link_needed(host, Require::Always), "{host}");
        }
        for host in ["0.0.0.0", "::", "203.0.113.9", "kwnt.example.com"] {
            assert!(startup_link_needed(host, Require::Auto), "{host}");
            assert!(!startup_link_needed(host, Require::Never), "{host}");
        }
    }

    #[test]
    fn one_time_links_and_notice() {
        let code = "kl1.abc";
        assert_eq!(one_time_link("0.0.0.0", 8799, None, code), "http://<this-host>:8799/#kwnt_token=kl1.abc");
        assert_eq!(one_time_link("::1", 8899, None, code), "http://[::1]:8899/#kwnt_token=kl1.abc");
        assert_eq!(
            one_time_link("0.0.0.0", 8799, Some("https://kwant.example.com/"), code),
            "https://kwant.example.com/#kwnt_token=kl1.abc"
        );
        // A placeholder host: no QR code (it couldn't open), and how to get a real one
        let placeholder = one_time_notice(&one_time_link("0.0.0.0", 8799, None, code), true);
        assert!(placeholder[0].ends_with("http://<this-host>:8799/#kwnt_token=kl1.abc"), "{placeholder:?}");
        assert!(placeholder[0].contains("works once, for 15 minutes"), "{placeholder:?}");
        assert!(placeholder.iter().any(|l| l.contains("KWNT_ALLOWED_ORIGINS")));
        assert!(!placeholder.iter().any(|l| l.contains('█')));
        let real = one_time_notice("https://kwant.example.com/#kwnt_token=kl1.abc", true);
        assert!(real.iter().any(|l| l.contains('█')), "QR code");
        assert!(real.last().unwrap().contains("kwnt signin-link"));
        assert!(!one_time_notice("https://kwant.example.com/#kwnt_token=kl1.abc", false).iter().any(|l| l.contains('█')));
        // The API token itself is never printed at startup any more
        assert!(!real.iter().chain(&placeholder).any(|l| l.contains("kwnt token")));
        assert_eq!(token_command_link("0.0.0.0", 8799, "tok"), "http://<this-host>:8799/#kwnt_token=tok");
        assert_eq!(token_command_link("127.0.0.1", 8899, "tok"), "http://127.0.0.1:8899/#kwnt_token=tok");
    }

    #[test]
    fn a_session_cookie_counts_as_the_token() {
        let api = ApiToken::for_tests("abcdefghijklmnopqrstuvwxyz012345", Require::Auto, 3000);
        let secret = api.signin.create_session(Via::Link, &signin::Client::default());
        assert!(presented_token_ok(&api, &headers(&[("cookie", &format!("kwnt_token={secret}"))])));
        // Only as a cookie: a bearer that isn't the API token is an agent's
        assert!(!presented_token_ok(&api, &headers(&[("authorization", &format!("Bearer {secret}"))])));
        let id = api.signin.session_id(&secret).unwrap();
        api.signin.revoke(&id);
        assert!(!presented_token_ok(&api, &headers(&[("cookie", &format!("kwnt_token={secret}"))])));
    }

    #[test]
    fn read_existing_never_creates_a_token() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(read_existing(dir.path()), Err(ReadTokenError::Missing(_))));
        assert!(!dir.path().join(TOKEN_FILE).exists());

        std::fs::write(dir.path().join(TOKEN_FILE), "short\n").unwrap();
        assert!(matches!(read_existing(dir.path()), Err(ReadTokenError::Invalid(_))));
        assert_eq!(std::fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap(), "short\n");

        let made = load_or_create(dir.path());
        assert_eq!(read_existing(dir.path()).unwrap(), made);
    }

    #[test]
    fn token_file_is_private_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let a = load_or_create(dir.path());
        assert_eq!(a.len(), 64);
        assert_eq!(load_or_create(dir.path()), a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(TOKEN_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
