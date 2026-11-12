//! 0.9.5: a connection failing says so in the chat when it happens. When one of a bot's
//! connections or tool servers fails during a turn (its sign-in expired or was refused, a
//! Composio app needs connecting again, the server didn't start or stopped responding), kwnt
//! posts one plain line in the chat the turn runs in, with a link to fix it:
//! "Gmail stopped working for Ben: its sign-in expired or was refused. Reconnect".
//!
//! Rules:
//! - one line per connection per turn, never one per tool call;
//! - nothing for a failure a retry through the same connection fixed: a failure waits
//!   [`GRACE`] for a retry (a second failure, or the turn ending, posts it at once);
//! - a server that didn't start posts at once (nothing can retry it in this turn);
//! - when only the person can fix it (a sign-in or a reconnect), the bot shows as "needs you"
//!   (`bot_status`'s `needs_sign_in`) and the line is listed in `GET /api/waiting` until a later
//!   call through that connection works, or [`NEEDS_YOU_DAYS`] pass.
//!
//! No new store: a failure is the activity step's `failed` status
//! ([`crate::drivers::activity::failed_reason`]), a server that didn't start is the problem
//! `McpToolSession::open` reports (API driver, codex gateway) or claude's own init line, and the
//! line is an ordinary message with `meta.kind = "tool_failure"`, posted like a tool note's.

use super::AppState;
use crate::db::models::{McpServer, Message, MessageExtra, NewMessage};
use crate::drivers::events::RuntimeEvent;
use chrono::{DateTime, Duration as Span, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub const KIND: &str = crate::db::tool_failures::KIND;
pub use crate::mcp::UNAVAILABLE_PREFIX;
/// How long a failure waits for a retry through the same connection before its line is posted.
pub const GRACE: Duration = Duration::from_secs(20);
/// How long an unfixed sign-in keeps the bot in "needs you".
pub const NEEDS_YOU_DAYS: i64 = 7;
/// Longest reason kept with a line.
const MAX_DETAIL: usize = 300;
/// Most failures one turn keeps track of (a runaway turn can't grow this without bound).
const MAX_KEYS: usize = 32;

/// Why a connection failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// Its sign-in expired or was refused (401/403, an expired or invalid token).
    SignIn,
    /// A Composio app has no active connected account: it needs connecting again.
    Reconnect,
    /// The server didn't start (or couldn't be reached) when the turn began.
    NotStarted,
    /// The server stopped responding mid-turn.
    Down,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Cause::SignIn => "sign_in",
            Cause::Reconnect => "reconnect",
            Cause::NotStarted => "not_started",
            Cause::Down => "down",
        }
    }

    /// Only the person can fix it (sign in, connect again).
    pub fn needs_you(self) -> bool {
        matches!(self, Cause::SignIn | Cause::Reconnect)
    }

    fn fix_label(self) -> &'static str {
        if self.needs_you() {
            "Reconnect"
        } else {
            "Check it"
        }
    }
}

/// One of the turn's servers, as the classifier needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub composio: bool,
}

impl Server {
    pub fn of(s: &McpServer) -> Self {
        Self { id: s.id.clone(), name: s.name.clone(), composio: super::connections::is_composio(s) }
    }
}

/// The connection a tool call went through: one key per connection, a name for the person and
/// the server to fix it in (none for kwant's own tools).
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    /// `mcp:<server id>`, `composio:<toolkit>` or `kwnt`.
    pub key: String,
    pub label: String,
    pub server_id: Option<String>,
}

/// A failed connection: what the line says.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub conn: Connection,
    pub cause: Cause,
    /// The tool that met it (`server · tool`), when a call did.
    pub tool: Option<String>,
    /// The reason it gave, redacted and cut.
    pub detail: Option<String>,
}

fn is_own(server: &str) -> bool {
    server == crate::mcp::SELF_SERVER_NAME || server == crate::mcp::RESERVED_SERVER_NAME
}

/// `Gmail` from `gmail`, `GMAIL`.
fn title(word: &str) -> String {
    let lower = word.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The Composio app a call used: the prefix of its tool slug (`GMAIL_SEND_EMAIL` → `gmail`), or
/// of the first slug a meta tool was given (`COMPOSIO_MULTI_EXECUTE_TOOL` with `tools[0].tool_slug`).
fn composio_toolkit(tool: &str, args: &Value) -> Option<String> {
    let slug = if tool.to_ascii_uppercase().starts_with("COMPOSIO_") {
        args["tools"][0]["tool_slug"]
            .as_str()
            .or_else(|| args["tool_slug"].as_str())
            .or_else(|| args["toolkit"].as_str())
            .or_else(|| args["toolkits"][0].as_str())?
    } else {
        tool
    };
    let kit = slug.split('_').next().unwrap_or("").trim().to_lowercase();
    (kit.len() >= 2 && kit.len() <= 40 && kit.chars().all(|c| c.is_ascii_alphanumeric()) && kit != "composio").then_some(kit)
}

/// The connection behind a tool name (`mcp__<server>__<tool>`), or `None` for a tool that isn't a
/// connection's (an engine's own tool, an unknown server).
pub fn connection_of(name: &str, args: &Value, servers: &[Server]) -> Option<Connection> {
    let (server, tool) = crate::mcp::split_tool_name(name)?;
    if is_own(server) {
        return Some(Connection { key: "kwnt".into(), label: "kwant's own tools".into(), server_id: None });
    }
    let s = servers.iter().find(|s| s.name == server)?;
    if s.composio {
        if let Some(kit) = composio_toolkit(tool, args) {
            return Some(Connection { key: format!("composio:{kit}"), label: title(&kit), server_id: Some(s.id.clone()) });
        }
    }
    Some(Connection { key: format!("mcp:{}", s.id), label: s.name.clone(), server_id: Some(s.id.clone()) })
}

/// `code` appears in `text` as a number of its own (`401`, not `14012`).
fn has_code(text: &str, code: &str) -> bool {
    text.match_indices(code).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + code.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

const SIGN_IN_WORDS: [&str; 14] = [
    "unauthorized",
    "unauthenticated",
    "forbidden",
    "invalid_grant",
    "invalid_token",
    "invalid token",
    "token expired",
    "token has expired",
    "expired token",
    "authentication required",
    "authentication failed",
    "login required",
    "re-authenticate",
    "reauthenticate",
];

const RECONNECT_WORDS: [&str; 8] = [
    "connected account not found",
    "connected account is not active",
    "connected account expired",
    "connected account is expired",
    "connected account is inactive",
    "no connected account",
    "no active connection",
    "needs to be reconnected",
];

const DOWN_WORDS: [&str; 11] = [
    "could not start",
    "failed to start",
    "connection refused",
    "connection reset",
    "connection closed",
    "transport closed",
    "failed to connect",
    "error sending request",
    "bad gateway",
    "service unavailable",
    "gateway timeout",
];

fn sign_in_text(lower: &str) -> bool {
    has_code(lower, "401") || has_code(lower, "403") || SIGN_IN_WORDS.iter().any(|w| lower.contains(w))
}

fn down_text(lower: &str) -> bool {
    has_code(lower, "502") || has_code(lower, "503") || has_code(lower, "504") || DOWN_WORDS.iter().any(|w| lower.contains(w))
}

/// What a failed call's reason says about its connection, or `None` when it's the tool's own
/// error (bad arguments, not found, a rule it refused): those aren't the connection's.
pub fn cause_of(conn: &Connection, why: &str) -> Option<Cause> {
    let lower = why.to_lowercase();
    // The MCP client couldn't talk to the server at all (`McpToolSession::call`)
    let transport = lower.starts_with("tool call failed:");
    if conn.key == "kwnt" {
        // kwant's own tools reach the web for the bot; only their own transport failing counts
        return (transport || lower.contains("transport closed") || lower.contains("connection closed")).then_some(Cause::Down);
    }
    let composio = conn.key.starts_with("composio:");
    if composio && RECONNECT_WORDS.iter().any(|w| lower.contains(w)) {
        return Some(Cause::Reconnect);
    }
    if sign_in_text(&lower) {
        return Some(if composio { Cause::Reconnect } else { Cause::SignIn });
    }
    if transport || down_text(&lower) {
        return Some(Cause::Down);
    }
    None
}

/// A failed call → the connection failure it is, if any.
pub fn classify(name: &str, args: &Value, why: &str, servers: &[Server]) -> Option<Failure> {
    let conn = connection_of(name, args, servers)?;
    let cause = cause_of(&conn, why)?;
    Some(Failure {
        conn,
        cause,
        tool: Some(cut(&super::tool_notes::tool_name(name), 80)),
        detail: Some(cut(why.trim(), MAX_DETAIL)).filter(|d| !d.is_empty()),
    })
}

/// A server the turn couldn't reach at the start (`"<server>: <why>"`), as a failure.
pub fn not_started(problem: &str, servers: &[Server]) -> Option<Failure> {
    let (server, why) = problem.split_once(": ").unwrap_or((problem, ""));
    let server = server.trim();
    let conn = if is_own(server) {
        Connection { key: "kwnt".into(), label: "kwant's own tools".into(), server_id: None }
    } else {
        let s = servers.iter().find(|s| s.name == server)?;
        Connection { key: format!("mcp:{}", s.id), label: s.name.clone(), server_id: Some(s.id.clone()) }
    };
    let lower = why.to_lowercase();
    let cause = if conn.server_id.is_some() && sign_in_text(&lower) { Cause::SignIn } else { Cause::NotStarted };
    Some(Failure { conn, cause, tool: None, detail: Some(cut(why.trim(), MAX_DETAIL)).filter(|d| !d.is_empty()) })
}

fn cut(s: &str, max: usize) -> String {
    super::tool_notes::cut(&super::tool_notes::clean(s, false), max)
}

/// The line's words, without the fix link.
pub fn line(f: &Failure, bot: &str) -> String {
    let c = &f.conn.label;
    match f.cause {
        Cause::SignIn => {
            format!("{c} stopped working for {bot}: its sign-in expired or was refused.")
        }
        Cause::Reconnect => {
            format!("{c} stopped working for {bot}: it needs to be connected again.")
        }
        Cause::NotStarted => format!("{c} didn't start for {bot}, so its tools weren't available."),
        Cause::Down => format!("{c} stopped responding for {bot}."),
    }
}

/// The saved message's text: the line plus where to fix it, for clients that show text only.
pub fn text(f: &Failure, bot: &str) -> String {
    let mut out = line(f, bot);
    if f.conn.server_id.is_some() {
        let verb = if f.cause.needs_you() { "Reconnect it" } else { "Check it" };
        out.push_str(&format!(" {verb} in Settings › Connections."));
    }
    out
}

/// The saved message's `meta`.
pub fn meta(f: &Failure, kwant_id: &str, bot: &str, turn_id: &str) -> Value {
    let fix = f
        .conn
        .server_id
        .as_ref()
        .map(|id| json!({"label": f.cause.fix_label(), "opens": "connections", "connection_id": format!("mcp:{id}")}));
    json!({
        "kind": KIND,
        "kwant_id": kwant_id,
        "turn_id": turn_id,
        "connection": f.conn.label,
        "key": f.conn.key,
        "server_id": f.conn.server_id,
        "cause": f.cause.as_str(),
        "tool": f.tool,
        "detail": f.detail,
        "line": line(f, bot),
        "needs_you": f.cause.needs_you(),
        "fix": fix,
        "resolved_at": null,
    })
}

// ---------------------------------------------------------------------------------------------
// One turn's failures: which to post, which a retry fixed

/// The turn's connection failures. A failure waits (pending) for a retry; a success through the
/// same connection drops it; a second failure, its grace running out or the turn ending posts
/// it. A connection is posted at most once per turn.
#[derive(Debug, Default)]
pub struct Watch {
    pending: HashMap<String, (Failure, u64)>,
    posted: HashSet<String>,
    next_gen: u64,
}

/// What [`Watch::failed`] asks for.
#[derive(Debug, PartialEq)]
pub enum Next {
    /// Nothing: already posted, or too many connections this turn.
    Nothing,
    /// Post this now.
    Post(Failure),
    /// Wait for a retry; post when [`Watch::due`] with this generation still finds it.
    Wait(u64),
}

impl Watch {
    pub fn failed(&mut self, f: Failure) -> Next {
        let key = f.conn.key.clone();
        if self.posted.contains(&key) {
            return Next::Nothing;
        }
        if self.pending.remove(&key).is_some() || f.cause == Cause::NotStarted {
            self.posted.insert(key);
            return Next::Post(f);
        }
        if self.pending.len() + self.posted.len() >= MAX_KEYS {
            return Next::Nothing;
        }
        self.next_gen += 1;
        self.pending.insert(key, (f, self.next_gen));
        Next::Wait(self.next_gen)
    }

    /// A call through `key` worked: a failure still waiting is dropped (the retry fixed it).
    pub fn succeeded(&mut self, key: &str) -> bool {
        self.pending.remove(key).is_some()
    }

    /// The grace of the failure `gen` ran out: post it if it still waits.
    pub fn due(&mut self, key: &str, gen: u64) -> Option<Failure> {
        match self.pending.get(key) {
            Some((_, g)) if *g == gen => {
                let (f, _) = self.pending.remove(key)?;
                self.posted.insert(key.to_string());
                Some(f)
            }
            _ => None,
        }
    }

    /// The turn ended: everything still waiting is posted.
    pub fn finish(&mut self) -> Vec<Failure> {
        let mut out: Vec<(Failure, u64)> = self.pending.drain().map(|(_, v)| v).collect();
        out.sort_by_key(|(_, g)| *g);
        for (f, _) in &out {
            self.posted.insert(f.conn.key.clone());
        }
        out.into_iter().map(|(f, _)| f).collect()
    }

    pub fn was_posted(&self, key: &str) -> bool {
        self.posted.contains(key)
    }
}

/// Where the turn's line goes.
#[derive(Debug, Clone, Default)]
pub struct Place {
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// The chat's id as bot status names it (`TurnTag::context_id`).
    pub context_id: String,
}

#[derive(Default)]
struct Inner {
    watch: Watch,
    /// Calls still running: id → (tool name, arguments).
    calls: HashMap<String, (String, Value)>,
    /// Connections a success was already seen for this turn (open lines resolved once).
    worked: HashSet<String>,
    finished: bool,
}

/// One turn's watch, found by turn id (the codex gateway reports its calls to it).
pub struct TurnWatch {
    state: AppState,
    place: Place,
    kwant_id: String,
    kwant_name: String,
    turn_id: String,
    servers: Vec<Server>,
    secrets: Vec<String>,
    inner: Mutex<Inner>,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<TurnWatch>>> {
    static WATCHES: OnceLock<Mutex<HashMap<String, Arc<TurnWatch>>>> = OnceLock::new();
    WATCHES.get_or_init(Default::default)
}

/// Removes the turn's watch when the turn's run ends, however it ends; failures still waiting
/// are posted in the background (the normal end posts them first, see [`TurnWatch::finish`]).
pub struct WatchGuard(pub Arc<TurnWatch>);

impl Drop for WatchGuard {
    fn drop(&mut self) {
        let w = self.0.clone();
        registry().lock().unwrap_or_else(|e| e.into_inner()).remove(&w.turn_id);
        let left = {
            let mut inner = w.lock();
            inner.finished = true;
            inner.watch.finish()
        };
        if !left.is_empty() {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    for f in left {
                        w.post(f).await;
                    }
                });
            }
        }
    }
}

impl TurnWatch {
    /// Starts watching a turn that has MCP servers.
    pub fn start(
        state: &AppState,
        place: Place,
        kwant_id: &str,
        kwant_name: &str,
        turn_id: &str,
        servers: &[McpServer],
        secrets: Vec<String>,
    ) -> WatchGuard {
        let watch = Arc::new(Self {
            state: state.clone(),
            place,
            kwant_id: kwant_id.to_string(),
            kwant_name: kwant_name.to_string(),
            turn_id: turn_id.to_string(),
            servers: servers.iter().map(Server::of).collect(),
            secrets,
            inner: Mutex::new(Inner::default()),
        });
        registry().lock().unwrap_or_else(|e| e.into_inner()).insert(turn_id.to_string(), watch.clone());
        WatchGuard(watch)
    }

    pub fn find(turn_id: &str) -> Option<Arc<TurnWatch>> {
        registry().lock().unwrap_or_else(|e| e.into_inner()).get(turn_id).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A driver event of the turn. Calls through kwnt's gateway (`mcp__kwnt__…`) are skipped:
    /// the gateway reports those itself, with the real server's name ([`TurnWatch::call_done`]).
    pub async fn observe(self: &Arc<Self>, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::ToolCallPending { id, name, args } => {
                let Some((server, _)) = crate::mcp::split_tool_name(name) else {
                    return;
                };
                if server == crate::mcp::RESERVED_SERVER_NAME {
                    return;
                }
                let mut inner = self.lock();
                if inner.calls.len() < 256 {
                    inner.calls.insert(id.clone(), (name.clone(), args.clone()));
                }
            }
            RuntimeEvent::ToolCallCompleted { id, result } => {
                let Some((name, args)) = self.lock().calls.remove(id) else {
                    return;
                };
                let status = result["status"].as_str().unwrap_or("");
                // A card the person declined, or a call the turn's end cut off, isn't the connection's
                if matches!(status, "denied" | "cancelled" | "unanswered") {
                    return;
                }
                let why = crate::drivers::activity::failed_reason(result);
                self.call_done(&name, &args, why.map(|w| w.unwrap_or(""))).await;
            }
            RuntimeEvent::ThinkingDelta(t) => {
                for problem in t.lines().filter_map(|l| l.trim().strip_prefix(UNAVAILABLE_PREFIX)) {
                    self.server_problem(problem).await;
                }
            }
            _ => {}
        }
    }

    /// A tool call finished: `failed` is `None` when it worked, else its reason.
    pub async fn call_done(self: &Arc<Self>, name: &str, args: &Value, failed: Option<&str>) {
        match failed {
            None => {
                let Some(conn) = connection_of(name, args, &self.servers) else {
                    return;
                };
                let (first, dropped) = {
                    let mut inner = self.lock();
                    let dropped = inner.watch.succeeded(&conn.key);
                    (inner.worked.insert(conn.key.clone()), dropped)
                };
                if dropped {
                    tracing::info!(kwant = %self.kwant_name, key = %conn.key, "a retry fixed a failed connection; no line");
                }
                if first {
                    resolve(&self.state, &self.kwant_id, &conn.key).await;
                }
            }
            Some(why) => {
                let why = crate::drivers::activity::redact(why, &self.secrets);
                if let Some(f) = classify(name, args, &why, &self.servers) {
                    self.failed(f).await;
                }
            }
        }
    }

    /// A server the turn couldn't reach (`"<server>: <why>"`).
    pub async fn server_problem(self: &Arc<Self>, problem: &str) {
        let problem = crate::drivers::activity::redact(problem, &self.secrets);
        if let Some(f) = not_started(&problem, &self.servers) {
            self.failed(f).await;
        }
    }

    async fn failed(self: &Arc<Self>, f: Failure) {
        let key = f.conn.key.clone();
        let next = {
            let mut inner = self.lock();
            if inner.finished {
                return;
            }
            inner.watch.failed(f)
        };
        match next {
            Next::Nothing => {}
            Next::Post(f) => self.post(f).await,
            Next::Wait(gen) => {
                let me = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(GRACE).await;
                    let due = me.lock().watch.due(&key, gen);
                    if let Some(f) = due {
                        me.post(f).await;
                    }
                });
            }
        }
    }

    /// The turn ended: failures still waiting for a retry are posted now (before the reply).
    pub async fn finish(&self) {
        let left = {
            let mut inner = self.lock();
            inner.finished = true;
            inner.watch.finish()
        };
        for f in left {
            self.post(f).await;
        }
    }

    async fn post(&self, f: Failure) {
        post_line(&self.state, &self.place, &self.kwant_id, &self.kwant_name, &self.turn_id, &f).await;
    }
}

/// Posts the line in the turn's chat, tells Connections, and marks the bot "needs you" when
/// only the person can fix it.
pub async fn post_line(state: &AppState, place: &Place, kwant_id: &str, bot: &str, turn_id: &str, f: &Failure) -> Option<Message> {
    // A connection's own server: Connections shows it as signed out or failing
    if let (Some(id), false) = (&f.conn.server_id, f.conn.key.starts_with("composio:")) {
        super::connections::record_mcp_check(id, &json!({"ok": false, "auth_required": f.cause == Cause::SignIn, "error": f.detail}));
    }
    if f.cause.needs_you() {
        let mark = super::bot_status::SignInMark {
            at: Utc::now(),
            key: f.conn.key.clone(),
            connection: f.conn.label.clone(),
            context_id: place.context_id.clone(),
            thread_id: place.thread_id.clone(),
        };
        state.turns.board(|b| b.mark_sign_in(kwant_id, mark));
    }
    tracing::info!(kwant = %bot, key = %f.conn.key, cause = f.cause.as_str(), "a connection failed; posting a line");
    let message = NewMessage {
        channel_id: place.channel_id.clone(),
        direct_kwant_id: place.direct_kwant_id.clone(),
        sender_role: "kwant".into(),
        sender_id: bot.to_string(),
        text: text(f, bot),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = MessageExtra { thread_id: place.thread_id.clone(), meta: Some(meta(f, kwant_id, bot, turn_id)) };
    match state.db.insert_message_with(&message, &extra).await {
        Ok(saved) => {
            if let Some(t) = &place.thread_id {
                let _ = state.db.touch_thread(t).await;
            }
            let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
            Some(saved)
        }
        Err(e) => {
            tracing::warn!(error = %e, "tool failure: could not post the chat line");
            None
        }
    }
}

fn meta_of(m: &Message) -> Option<Value> {
    m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).filter(|v| v["kind"] == KIND)
}

/// A call through `key` worked: the bot's open lines for it are resolved (out of "Needs you")
/// and its sign-in mark is cleared.
pub async fn resolve(state: &AppState, kwant_id: &str, key: &str) {
    let marked = state.turns.board(|b| b.clear_sign_in(kwant_id, key));
    let since = Utc::now() - Span::days(NEEDS_YOU_DAYS);
    let open: Vec<(String, Value)> = match state.db.list_tool_failure_lines().await {
        Ok(lines) => lines
            .iter()
            .filter(|m| m.created_at.is_some_and(|at| at >= since))
            .filter_map(|m| meta_of(m).map(|v| (m.id.clone(), v)))
            .filter(|(_, v)| v["kwant_id"] == kwant_id && v["key"] == key && v["needs_you"] == true && v["resolved_at"].is_null())
            .collect(),
        Err(_) => return,
    };
    if open.is_empty() && !marked {
        return;
    }
    let now = Utc::now();
    for (id, mut v) in open {
        v["resolved_at"] = json!(now);
        let _ = state.db.set_message_meta(&id, &v).await;
    }
    tracing::info!(kwant = %kwant_id, %key, "a connection works again; its line is resolved");
}

/// "Needs you" items: open sign-in lines of the space's bots from the last [`NEEDS_YOU_DAYS`],
/// one per bot and connection (the newest), as `(asked_at, item)` for `GET /api/waiting`.
pub async fn needs_you(state: &AppState, kwants: &[crate::db::models::Kwant]) -> Vec<(DateTime<Utc>, Value)> {
    let Ok(lines) = state.db.list_tool_failure_lines().await else {
        return Vec::new();
    };
    needs_you_items(&lines, kwants, Utc::now())
}

pub fn needs_you_items(lines: &[Message], kwants: &[crate::db::models::Kwant], now: DateTime<Utc>) -> Vec<(DateTime<Utc>, Value)> {
    let since = now - Span::days(NEEDS_YOU_DAYS);
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    // Newest first, so the first line of a bot and connection is the one kept
    for m in lines {
        let Some(at) = m.created_at.filter(|at| *at >= since) else {
            continue;
        };
        let Some(v) = meta_of(m) else { continue };
        if v["needs_you"] != true || !v["resolved_at"].is_null() {
            continue;
        }
        let (Some(kwant_id), Some(key)) = (v["kwant_id"].as_str(), v["key"].as_str()) else {
            continue;
        };
        let Some(k) = kwants.iter().find(|k| k.id == kwant_id) else {
            continue;
        };
        if !seen.insert((kwant_id.to_string(), key.to_string())) {
            continue;
        }
        out.push((
            at,
            json!({
                "kind": "connection",
                "kwant_id": kwant_id,
                "kwant_name": k.name,
                "channel_id": m.channel_id,
                "direct_kwant_id": m.direct_kwant_id,
                "thread_id": m.thread_id,
                "message_id": m.id,
                "connection": v["connection"],
                "cause": v["cause"],
                "fix": v["fix"],
                "text": v["line"],
                "asked_at": at,
                "follow_up_at": null,
                "reminded_at": null,
            }),
        ));
    }
    out
}

#[cfg(test)]
mod tests;
