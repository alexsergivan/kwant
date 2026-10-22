//! Who may call the API. Browsers may only call it from kwnt's own page (Origin allowlist), and
//! requests that don't come from this machine need the API token, as a bearer header or the
//! `kwnt_token` cookie the UI gets by posting the token to `/api/auth/session`.
//!
//! "From this machine" means a loopback peer, a loopback `Host` and no proxy forwarding headers.
//! The `Host` check is what keeps a DNS-rebinding page (same peer, foreign host) out.

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
use std::path::Path;

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
        let token = std::env::var("KWNT_API_TOKEN")
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| t.len() >= 16)
            .unwrap_or_else(|| load_or_create(&config.data_dir));
        Self {
            token,
            require,
            extra_origins,
            port: config.port,
        }
    }

    pub fn for_tests(token: &str, require: Require, port: u16) -> Self {
        Self {
            token: token.into(),
            require,
            extra_origins: Vec::new(),
            port,
        }
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

/// Reads the token file, or writes a new random one. Falls back to an in-memory token.
pub fn load_or_create(data_dir: &Path) -> String {
    let path = data_dir.join(TOKEN_FILE);
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if t.len() >= 16 {
            return t;
        }
    }
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

/// The link that signs a browser in once: `http://<host>:<port>/#kwnt_token=<token>`. The UI
/// posts the token to `/api/auth/session` and clears it from the address bar.
pub fn sign_in_link(host: &str, port: u16, token: &str) -> String {
    let bare = host.trim_matches(['[', ']']);
    let host = match bare.parse::<IpAddr>() {
        Ok(IpAddr::V6(_)) => format!("[{bare}]"),
        _ => bare.to_string(),
    };
    format!("http://{host}:{port}/#kwnt_token={token}")
}

/// What `kwnt serve` says about signing in, for stdout only: the link carries the token, so it
/// must never go through tracing into log files. On a loopback bind nobody needs the token,
/// unless `KWNT_REQUIRE_TOKEN=always`.
pub fn startup_notice(host: &str, port: u16, require: Require, token: &str) -> Vec<String> {
    match require {
        Require::Never => Vec::new(),
        Require::Auto if is_loopback_bind(host) => Vec::new(),
        Require::Always if is_loopback_bind(host) => vec![format!(
            "🔒 Open this link once to sign in (it signs in as you, keep it private): {}",
            sign_in_link(host, port, token)
        )],
        _ => vec![format!(
            "🔒 Requests from other machines need the API token: run `kwnt token` and open http://<this-host>:{port}/#kwnt_token=<token>"
        )],
    }
}

fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
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

/// Routes only the person may call: saved passwords and the password vault (Bitwarden), answering an approval card, Loose ends and Parallel Universe, the
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
        // Parallel Universe starts containers on this machine from a directory the person names
        ["", "api", "parallel", ..] => true,
        // Camera-to-Completed: the person's letters
        ["", "api", "documents", ..] => true,
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

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
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

fn presented_token_ok(api: &ApiToken, headers: &HeaderMap) -> bool {
    bearer(headers).is_some_and(|t| api.matches(t))
        || cookie_token(headers).is_some_and(|t| api.matches(t))
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
        Json(json!({ "error": "this needs the API token: sign in with `kwnt token`", "person_only": true })),
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

/// `POST /api/auth/session {token}`: checks the token and sets the `kwnt_token` cookie
/// (HttpOnly, SameSite=Strict), which EventSource and the browser WebSocket then send.
pub async fn create_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SessionBody>,
) -> Response {
    let token = body.token.trim();
    if !state.api_token.matches(token) {
        // A wrong guess costs a little time
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        return deny(StatusCode::UNAUTHORIZED, "wrong token");
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(token, 31_536_000, is_https(&headers)),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// `DELETE /api/auth/session`: clears the cookie.
pub async fn delete_session(headers: HeaderMap) -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie("", 0, is_https(&headers)),
    );
    resp
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
        assert!(person_only(&post, "/api/passwords/bitwarden/unlock"));
        assert!(person_only(&del, "/api/passwords/bitwarden"));
        assert!(person_only(&post, "/api/passwords/migrate"));
        assert!(person_only(&get, "/api/zombie/cases"));
        assert!(person_only(&post, "/api/zombie/sources/s1/test"));
        assert!(person_only(&get, "/api/parallel/prices"));
        assert!(person_only(&post, "/api/parallel/scenarios"));
        assert!(person_only(&post, "/api/documents"));
        assert!(person_only(&get, "/api/documents/d1"));
        assert!(person_only(&post, "/api/approvals/a1/resolve"));
        assert!(person_only(&put, "/api/spaces/s1/approvals"));
        assert!(person_only(&put, "/api/kwants/k1/approvals"));
        assert!(!person_only(&get, "/api/kwants/k1/approvals"));
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
    fn loopback_start_prints_no_token_line_unless_always() {
        let tok = "abcdefghijklmnopqrstuvwxyz012345";
        for host in ["127.0.0.1", "localhost", "::1", "[::1]"] {
            assert!(startup_notice(host, 8899, Require::Auto, tok).is_empty(), "{host}");
            assert!(startup_notice(host, 8899, Require::Never, tok).is_empty(), "{host}");
        }
        let always = startup_notice("127.0.0.1", 8899, Require::Always, tok).join("\n");
        assert!(always.contains("http://127.0.0.1:8899/#kwnt_token=abcdefghijklmnopqrstuvwxyz012345"), "{always}");
        assert!(!always.contains("kwnt token`"), "{always}");
        let v6 = startup_notice("::1", 8899, Require::Always, tok).join("\n");
        assert!(v6.contains("http://[::1]:8899/#kwnt_token="), "{v6}");
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
