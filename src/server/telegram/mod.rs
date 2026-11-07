//! Telegram (next-077 spec §G): the person talks to their kwant space from Telegram through a bot
//! they make in BotFather. One bot per space, paired with one private chat. Off by default.
//!
//! - The token lives in the vault ([`VAULT_OWNER`]), never in the database, logs, events or a
//!   bot's context. Only [`api::Client`] puts it in a URL.
//! - Inbound text from the paired chat goes to #general as the person's message
//!   (`meta.source = "telegram"`); anything from another chat, a group, a channel or a forward is
//!   ignored (Q-G1).
//! - Outbound: bot replies in #general to a message sent from Telegram (A19), and the needs-you
//!   kinds the person switched on (A20). Questions get their choices as buttons.
//! - Approvals are link-only unless the person switched "Allow approvals" on AND the call is a
//!   kind on [`TELEGRAM_APPROVABLE`] with no forcing reason (Q-G6). There is no Deny (A22).
//!
//! Long polling only (getUpdates): no webhook, no inbound port.

pub mod api;
pub mod bridge;
pub mod pairing;
pub mod routes;
pub mod text;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::AppState;
use crate::db::telegram as db;
use crate::drivers::events::RuntimeEvent;
use crate::sandbox::credentials::{CredentialStore, StoreError};

/// The vault owner the bot tokens are saved under (username: the space id).
pub const VAULT_OWNER: &str = "kwnt-telegram";
/// The vault origin of a token. Always the real API, whatever base a test build talks to.
pub const VAULT_ORIGIN: &str = api::TELEGRAM_API;

/// What a Telegram approval card is about, from its `details.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKind {
    /// A tool on an MCP server or a Composio app (`source: "mcp"`).
    McpTool,
    /// One of claude's own tools: Bash, Write, Edit, … (`source: "claude_builtin"`).
    BuiltinTool,
    /// A shell command on the computer (`source: "local_shell"`).
    LocalShell,
    /// Anything else: reviews, tasks, tamper checks, and every kind added later.
    Other,
}

/// The kinds "Allow once" may be pressed for in Telegram (Q-G6), when the person switched Allow
/// approvals on. Everything else is link-only: payments, human steps, Take over, production and
/// any kind added later.
pub const TELEGRAM_APPROVABLE: &[ApprovalKind] = &[ApprovalKind::McpTool, ApprovalKind::BuiltinTool];

/// Keys on a card's details that make it link-only whatever its kind: the payment floor, a
/// tier-forced ask and the production floor.
pub const FORCING_KEYS: &[&str] = &["payment_reason", "tier_reason", "prod_floor"];

impl ApprovalKind {
    pub fn of(details: &Value) -> Self {
        match details["source"].as_str() {
            Some("mcp") => ApprovalKind::McpTool,
            Some("claude_builtin") => ApprovalKind::BuiltinTool,
            Some("local_shell") => ApprovalKind::LocalShell,
            _ => ApprovalKind::Other,
        }
    }
}

/// Claude's own tools that may be allowed once from Telegram: the file tools, and only inside the
/// bot's own folder. Bash is a shell on the computer, as `local_shell` is, and every other or later
/// tool is link-only (Lin's M2).
pub const BUILTIN_APPROVABLE: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Read", "Glob", "Grep"];

/// Whether this card may be allowed from Telegram at all, as far as its details tell (the switch,
/// and the space's production marks, are checked apart).
pub fn approvable(details: &Value) -> bool {
    link_only_reason(details).is_none()
}

/// Why a card is link-only in Telegram, from its details alone; `None` when Allow once may show.
/// Product copy is Mira's.
pub fn link_only_reason(details: &Value) -> Option<&'static str> {
    let set = |k: &str| details.get(k).is_some_and(|v| !v.is_null());
    if set("prod_floor") {
        return Some("This is a production connection.");
    }
    if set("payment_reason") {
        return Some("This one involves a payment.");
    }
    if set("tier_reason") {
        return Some("This bot's tier needs you to approve it.");
    }
    // A turn that read outside text: an injected request is likeliest there
    if details["tainted"] == true {
        return Some("This bot read text from outside in this turn.");
    }
    // A turn kwant started on its own (loose ends, a mission): nobody asked for this step
    if details["always_ask"] == true {
        return Some("kwant started this turn on its own, not from your message.");
    }
    let kind = ApprovalKind::of(details);
    if !TELEGRAM_APPROVABLE.contains(&kind) {
        return Some(if kind == ApprovalKind::LocalShell { "This is a command on your computer." } else { "This kind of step can't be approved from Telegram." });
    }
    match kind {
        ApprovalKind::BuiltinTool => {
            let tool = details["tool"].as_str().unwrap_or_default();
            if tool == "Bash" {
                return Some("This is a command on your computer.");
            }
            if !BUILTIN_APPROVABLE.contains(&tool) {
                return Some("This kind of step can't be approved from Telegram.");
            }
            // Only an explicit "inside the folder" counts
            if details["outside_folder"] != false {
                return Some("This changes files outside the bot's folder.");
            }
            None
        }
        ApprovalKind::McpTool if details["server"].as_str().is_none_or(|s| s.eq_ignore_ascii_case(crate::mcp::SELF_SERVER_NAME)) => {
            Some("This is one of kwant's own tools.")
        }
        // A phone shows only the wrapper's name, not the actions or code it runs (Ada, N-T2)
        ApprovalKind::McpTool if runs_other_actions(details["tool"].as_str().unwrap_or_default()) => {
            Some("This step runs other actions inside it.")
        }
        _ => None,
    }
}

/// Parts of an MCP tool name (any case) that mark a tool running other actions or code inside it,
/// such as COMPOSIO_MULTI_EXECUTE_TOOL, COMPOSIO_REMOTE_WORKBENCH and COMPOSIO_REMOTE_BASH_TOOL.
pub const RUNS_OTHER_ACTIONS: &[&str] = &["MULTI_EXECUTE", "WORKBENCH", "BASH", "SHELL"];

fn runs_other_actions(tool: &str) -> bool {
    let tool = tool.to_ascii_uppercase();
    RUNS_OTHER_ACTIONS.iter().any(|part| tool.contains(part))
}

/// [`link_only_reason`], plus the space's production marks: a call through a connection the person
/// marked production is link-only in every space, Dev mode or not (Q-G6, Lin's M2).
pub async fn link_only_in(state: &AppState, space: &str, details: &Value) -> Option<&'static str> {
    if let Some(why) = link_only_reason(details) {
        return Some(why);
    }
    if ApprovalKind::of(details) != ApprovalKind::McpTool {
        return None;
    }
    let marked = match state.db.prod_connections(space).await {
        Ok(m) => m,
        // Can't tell: link-only
        Err(_) => return Some("This kind of step can't be approved from Telegram."),
    };
    if marked.is_empty() {
        return None;
    }
    let servers = match state.db.list_mcp_servers(&crate::db::spaces::SpaceId::new(space)).await {
        Ok(s) => s,
        Err(_) => return Some("This kind of step can't be approved from Telegram."),
    };
    // `mcp:<id>` marks become server names, `composio:<slug>` marks the toolkit slug
    let names = crate::server::dev::floor::prod_names(&marked, servers.iter().map(|s| (s.id.as_str(), s.name.as_str())));
    let server = details["server"].as_str().unwrap_or_default();
    let tool = details["tool"].as_str().unwrap_or_default();
    crate::mcp::prod_floor::production_connection(server, tool, &details["arguments"], &names).map(|_| "This is a production connection.")
}

/// The text Telegram shows (design G, "Telegram copy"). Product copy is Mira's.
pub mod copy {
    pub const UNPAIRED: &str = "Unpaired. Pair again from kwant › Connections.";
    pub const BAD_CODE: &str = "That code didn't work. Make a new one in kwant.";
    pub const ALREADY_ANSWERED: &str = "Already answered in kwant.";
    pub const TEST: &str = "Test from kwant. If you see this, alerts reach you.";
    pub const TEXT_ONLY: &str = "Only text works here for now. Send files in kwant.";
    pub const NO_GENERAL: &str = "This space has no #general, so your message wasn't sent. Open kwant to add one.";
    pub const APPROVE_IN_KWANT: &str = "Approve it in kwant.";
    pub const NO_LONGER_WAITING: &str = "No longer waiting";
    pub const OPEN_IN_KWANT: &str = "Open in kwant";
    pub const ALLOW_ONCE: &str = "Allow once";
    pub const NEEDS_BACKUP: &str = "This step needs a fresh backup first. Approve it in kwant.";
    pub const NOT_SENT: &str = "kwant couldn't take that message. Try again, or open kwant.";
    pub const SIGN_INS_IN_KWANT: &str = "Sign-ins happen in kwant, never here.";

    pub fn paired(space: &str) -> String {
        format!("<b>Paired with kwant · {}.</b> Anything you send here goes to #general. Write @Name to reach one bot. Send /stop to unpair.", super::text::escape(space))
    }

    pub fn no_such_bot(name: &str, space: &str) -> String {
        format!("There's no bot called {} in {}. Your message went to #general.", super::text::escape(name), super::text::escape(space))
    }
}

/// Whether the poller is reading updates right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Listening,
    Offline,
    Throttled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Listening => "listening",
            Status::Offline => "offline",
            Status::Throttled => "throttled",
        }
    }
}

/// One thing to send, in order, paced.
#[derive(Debug, Clone)]
pub enum Out {
    /// A message in parts; buttons go on the last part.
    Send {
        parts: Vec<String>,
        markup: Option<Value>,
        reply_to: Option<i64>,
        /// The ask this message carries (its last part's id is kept on the ask).
        ask_id: Option<String>,
        /// The kwant message this is, so a Telegram reply to it can reply to that message.
        message_id: Option<String>,
    },
    /// New text (and buttons, or none) on a message already sent.
    Edit { message_id: i64, html: String, markup: Option<Value> },
}

/// A space's live Telegram connection.
pub struct Runner {
    pub space: String,
    pub client: api::Client,
    cancel: CancellationToken,
    status: Mutex<Status>,
    queued: AtomicUsize,
    tx: mpsc::UnboundedSender<(i64, Out)>,
    /// When each chat last got "That code didn't work" (once a minute per chat, Q-G1).
    bad_code_replies: Mutex<HashMap<i64, Instant>>,
    /// Wrong codes per Telegram account: how many, since when (in-memory; a restart forgets them,
    /// the space-wide count stays in the database).
    bad_codes: Mutex<HashMap<i64, (i64, Instant)>>,
}

impl Runner {
    pub fn status(&self) -> Status {
        *self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets the status; true when it changed.
    fn set_status(&self, s: Status) -> bool {
        let mut cur = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *cur != s;
        *cur = s;
        changed
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::SeqCst)
    }

    /// Queues `out` for `chat_id`.
    pub fn enqueue(&self, chat_id: i64, out: Out) {
        if self.tx.send((chat_id, out)).is_ok() {
            self.queued.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Whether this sender's codes are ignored: [`pairing::MAX_FAILURES`] wrong ones within
    /// [`pairing::FAIL_WINDOW_MINUTES`], counted since the last new code.
    fn sender_locked(&self, from_id: i64) -> bool {
        let mut m = self.bad_codes.lock().unwrap_or_else(|e| e.into_inner());
        prune_bad_codes(&mut m);
        m.get(&from_id).is_some_and(|(n, _)| *n >= pairing::MAX_FAILURES)
    }

    /// One more wrong code from this sender.
    fn sender_failed(&self, from_id: i64) {
        let mut m = self.bad_codes.lock().unwrap_or_else(|e| e.into_inner());
        prune_bad_codes(&mut m);
        let e = m.entry(from_id).or_insert((0, Instant::now()));
        e.0 += 1;
    }

    /// A new code: every sender starts again, as the space's lock does.
    pub fn forget_bad_codes(&self) {
        self.bad_codes.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Whether this chat may get the wrong-code reply now (and notes that it did).
    fn may_reply_bad_code(&self, chat_id: i64) -> bool {
        let mut m = self.bad_code_replies.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        m.retain(|_, t| now.duration_since(*t) < Duration::from_secs(60));
        if m.contains_key(&chat_id) {
            return false;
        }
        m.insert(chat_id, now);
        true
    }
}

/// Drops the per-sender counts older than the window. The window is real time: the in-memory
/// count can't be moved by the test clock, and doesn't need to be.
fn prune_bad_codes(m: &mut HashMap<i64, (i64, Instant)>) {
    let window = Duration::from_secs(pairing::FAIL_WINDOW_MINUTES as u64 * 60);
    m.retain(|_, (_, since)| since.elapsed() < window);
}

type Key = (PathBuf, String);

static RUNNERS: LazyLock<Mutex<HashMap<Key, Arc<Runner>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Data directories whose event listener runs (one per server; tests run several in a process).
static LISTENERS: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
/// Sends in the last second, across every bot (Telegram allows about 30 a second).
static GLOBAL_SENDS: LazyLock<tokio::sync::Mutex<VecDeque<Instant>>> = LazyLock::new(|| tokio::sync::Mutex::new(VecDeque::new()));
const GLOBAL_PER_SECOND: usize = 30;
/// At least this long between two sends to one chat.
const CHAT_GAP: Duration = Duration::from_millis(1000);

fn key(state: &AppState, space: &str) -> Key {
    (state.config.data_dir.clone(), space.to_string())
}

pub fn runner(state: &AppState, space: &str) -> Option<Arc<Runner>> {
    RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).get(&key(state, space)).cloned()
}

/// `(status, queued)` for the GET; a space with no runner is offline.
pub fn status_of(state: &AppState, space: &str) -> (Status, usize) {
    match runner(state, space) {
        Some(r) => (r.status(), r.queued()),
        None => (Status::Offline, 0),
    }
}

pub fn emit_status(state: &AppState, space: &str) {
    let _ = state.event_sender.send(RuntimeEvent::TelegramStatus { space_id: space.to_string() });
}

// --- The vault -----------------------------------------------------------------------------

pub async fn load_token(state: &AppState, space: &str) -> Result<Option<Zeroizing<String>>, StoreError> {
    let store: &dyn CredentialStore = state.credentials.as_ref();
    let Some(meta) = store.list(VAULT_OWNER).await?.into_iter().find(|m| m.origin == VAULT_ORIGIN && m.username == space) else {
        return Ok(None);
    };
    Ok(store.get(VAULT_OWNER, &meta.id).await?.map(|c| Zeroizing::new(c.password.clone())))
}

pub async fn save_token(state: &AppState, space: &str, token: &str) -> Result<(), StoreError> {
    let store: &dyn CredentialStore = state.credentials.as_ref();
    store.save(VAULT_OWNER, VAULT_ORIGIN, space, token).await.map(|_| ())
}

pub async fn forget_token(state: &AppState, space: &str) -> Result<(), StoreError> {
    let store: &dyn CredentialStore = state.credentials.as_ref();
    for m in store.list(VAULT_OWNER).await?.into_iter().filter(|m| m.username == space) {
        store.delete(VAULT_OWNER, &m.id).await?;
    }
    Ok(())
}

// --- Starting and stopping -----------------------------------------------------------------

/// Starts Telegram with the server: the event listener, and a runner for every space with a
/// token that Telegram hasn't rejected.
pub fn spawn(state: AppState, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        ensure_running(&state).await;
        shutdown.cancelled().await;
        stop_all(&state);
    })
}

/// Idempotent: the listener and any missing runners (tests call this; they don't run
/// `spawn_background`).
pub async fn ensure_running(state: &AppState) {
    let fresh = LISTENERS.lock().unwrap_or_else(|e| e.into_inner()).insert(state.config.data_dir.clone());
    if fresh {
        bridge::spawn_listener(state.clone());
    }
    for link in state.db.telegram_links().await.unwrap_or_default() {
        if link.state == db::REJECTED || runner(state, &link.space_id).is_some() {
            continue;
        }
        match load_token(state, &link.space_id).await {
            Ok(Some(token)) => start(state, &link.space_id, token),
            Ok(None) => tracing::warn!(space = %link.space_id, "telegram: no token in the vault for a saved bot"),
            Err(StoreError::Locked) => tracing::info!(space = %link.space_id, "telegram: the vault is locked; not listening until it is unlocked"),
            Err(e) => tracing::warn!(space = %link.space_id, "telegram: could not read the token: {e}"),
        }
    }
}

/// (Re)starts the space's runner with `token`.
pub fn start(state: &AppState, space: &str, token: Zeroizing<String>) {
    stop(state, space);
    let client = match api::Client::new(token) {
        Ok(c) => c,
        Err(_) => {
            tracing::warn!(space, "telegram: couldn't build a client that never follows redirects; not listening");
            return;
        }
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let runner = Arc::new(Runner {
        space: space.to_string(),
        client,
        cancel: CancellationToken::new(),
        status: Mutex::new(Status::Offline),
        queued: AtomicUsize::new(0),
        tx,
        bad_code_replies: Mutex::new(HashMap::new()),
        bad_codes: Mutex::new(HashMap::new()),
    });
    RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).insert(key(state, space), runner.clone());
    tokio::spawn(bridge::poll_loop(state.clone(), runner.clone()));
    tokio::spawn(send_loop(state.clone(), runner.clone(), rx));
    tokio::spawn(bridge::tick_loop(state.clone(), runner));
}

pub fn stop(state: &AppState, space: &str) {
    if let Some(r) = RUNNERS.lock().unwrap_or_else(|e| e.into_inner()).remove(&key(state, space)) {
        r.cancel.cancel();
    }
}

fn stop_all(state: &AppState) {
    let mut m = RUNNERS.lock().unwrap_or_else(|e| e.into_inner());
    m.retain(|(dir, _), r| {
        if *dir == state.config.data_dir {
            r.cancel.cancel();
            false
        } else {
            true
        }
    });
}

// --- Sending -------------------------------------------------------------------------------

async fn global_slot() {
    loop {
        let wait = {
            let mut q = GLOBAL_SENDS.lock().await;
            let now = Instant::now();
            while q.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) {
                q.pop_front();
            }
            if q.len() < GLOBAL_PER_SECOND {
                q.push_back(now);
                return;
            }
            Duration::from_secs(1).saturating_sub(now.duration_since(*q.front().expect("a full queue has a front")))
        };
        tokio::time::sleep(wait).await;
    }
}

/// Sends the queue in order: one chat message a second at most, 30 a second across bots, and
/// Telegram's `retry_after` honoured on a 429 (test plan G-N).
async fn send_loop(state: AppState, runner: Arc<Runner>, mut rx: mpsc::UnboundedReceiver<(i64, Out)>) {
    let mut last: Option<Instant> = None;
    loop {
        let item = tokio::select! {
            _ = runner.cancel.cancelled() => return,
            i = rx.recv() => i,
        };
        let Some((chat_id, out)) = item else { return };
        match out {
            Out::Send { parts, markup, reply_to, ask_id, message_id } => {
                let n = parts.len();
                for (i, part) in parts.iter().enumerate() {
                    let buttons = if i + 1 == n { markup.as_ref() } else { None };
                    pace(&mut last).await;
                    let sent = send_one(&state, &runner, chat_id, part, buttons, if i == 0 { reply_to } else { None }).await;
                    let Some(tg_id) = sent else { break };
                    if let Some(mid) = &message_id {
                        let _ = state.db.telegram_map_message(&runner.space, chat_id, tg_id, mid).await;
                    }
                    if i + 1 == n {
                        if let Some(a) = &ask_id {
                            let _ = state.db.telegram_ask_sent(a, tg_id).await;
                        }
                    }
                }
            }
            Out::Edit { message_id, html, markup } => {
                pace(&mut last).await;
                with_retries(&state, &runner, || runner.client.edit_message(chat_id, message_id, &html, markup.as_ref())).await;
            }
        }
        runner.queued.fetch_sub(1, Ordering::SeqCst);
        let _ = state.db.telegram_touch(&runner.space).await;
    }
}

async fn pace(last: &mut Option<Instant>) {
    if let Some(t) = *last {
        let since = t.elapsed();
        if since < CHAT_GAP {
            tokio::time::sleep(CHAT_GAP - since).await;
        }
    }
    global_slot().await;
    *last = Some(Instant::now());
}

/// One part; HTML Telegram can't parse goes again as plain text.
async fn send_one(state: &AppState, runner: &Runner, chat_id: i64, html: &str, markup: Option<&Value>, reply_to: Option<i64>) -> Option<i64> {
    match with_retries(state, runner, || runner.client.send_message(chat_id, html, markup, reply_to)).await {
        Some(Ok(id)) => Some(id),
        Some(Err(api::TgError::BadRequest(_))) => {
            let plain = text::unescape(&text::strip_tags(html));
            match with_retries(state, runner, || runner.client.send_plain(chat_id, &plain, markup)).await {
                Some(Ok(id)) => Some(id),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Runs `call`, waiting out 429s and short outages (a few tries). `None` when it never went
/// through for a reason the caller can't fix; `Some(Err(BadRequest))` so the caller can fall back.
async fn with_retries<T, F, Fut>(state: &AppState, runner: &Runner, mut call: F) -> Option<Result<T, api::TgError>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, api::TgError>>,
{
    let mut backoff = 1u64;
    for _ in 0..5 {
        match call().await {
            Ok(v) => {
                if runner.status() == Status::Throttled && runner.set_status(Status::Listening) {
                    emit_status(state, &runner.space);
                }
                return Some(Ok(v));
            }
            Err(api::TgError::TooMany { retry_after }) => {
                if runner.set_status(Status::Throttled) {
                    emit_status(state, &runner.space);
                }
                tokio::select! {
                    _ = runner.cancel.cancelled() => return None,
                    _ = tokio::time::sleep(Duration::from_secs(retry_after.min(60))) => {}
                }
            }
            Err(api::TgError::Network) | Err(api::TgError::Server(_)) => {
                tokio::select! {
                    _ = runner.cancel.cancelled() => return None,
                    _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                }
                backoff = (backoff * 2).min(30);
            }
            Err(api::TgError::Forbidden) => {
                let _ = state.db.telegram_set_error(&runner.space, Some("blocked")).await;
                emit_status(state, &runner.space);
                return None;
            }
            Err(api::TgError::Unauthorized) => {
                bridge::rejected(state, runner).await;
                return None;
            }
            Err(e @ api::TgError::BadRequest(_)) => return Some(Err(e)),
            Err(e) => {
                tracing::info!(space = %runner.space, "telegram: send failed: {e}");
                return None;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_listed_kinds_without_a_forcing_reason_are_approvable() {
        assert!(approvable(&json!({"source": "mcp", "server": "files", "tool": "create_issue"})));
        assert!(approvable(&json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false})));
        assert!(approvable(&json!({"source": "claude_builtin", "tool": "Edit", "outside_folder": false})));
        assert!(approvable(&json!({"source": "mcp", "server": "files", "payment_reason": null})));
        for d in [
            json!({"source": "local_shell"}),
            json!({"source": "review"}),
            json!({"source": "task"}),
            json!({"source": "tamper"}),
            json!({"source": "something_new"}),
            json!({}),
            json!({"source": "mcp", "server": "files", "payment_reason": "pays"}),
            json!({"source": "mcp", "server": "files", "tier_reason": "tier"}),
            json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false, "prod_floor": {"needs_backup": false}}),
        ] {
            assert!(!approvable(&d), "{d}");
        }
    }

    // Lin's M2: what is as risky as a shell, or likeliest to be an injected request, stays link-only
    #[test]
    fn bash_outside_writes_kwants_own_tools_and_tainted_turns_are_link_only() {
        let cases = [
            (json!({"source": "claude_builtin", "tool": "Bash", "outside_folder": false}), "This is a command on your computer."),
            (json!({"source": "claude_builtin", "tool": "Write", "outside_folder": true}), "This changes files outside the bot's folder."),
            (json!({"source": "claude_builtin", "tool": "Edit"}), "This changes files outside the bot's folder."),
            (json!({"source": "claude_builtin", "tool": "WebFetch", "outside_folder": false}), "This kind of step can't be approved from Telegram."),
            (json!({"source": "claude_builtin", "tool": "SomeNewTool", "outside_folder": false}), "This kind of step can't be approved from Telegram."),
            (json!({"source": "mcp", "server": "kwnt_tools", "tool": "browser_click"}), "This is one of kwant's own tools."),
            (json!({"source": "mcp", "tool": "no_server"}), "This is one of kwant's own tools."),
            (json!({"source": "mcp", "server": "files", "tool": "x", "tainted": true}), "This bot read text from outside in this turn."),
            (json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false, "tainted": true}), "This bot read text from outside in this turn."),
            (json!({"source": "mcp", "server": "files", "tool": "x", "always_ask": true}), "kwant started this turn on its own, not from your message."),
            (json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false, "always_ask": true}), "kwant started this turn on its own, not from your message."),
            // Both: the outside text is the sharper reason
            (json!({"source": "mcp", "server": "files", "tool": "x", "always_ask": true, "tainted": true}), "This bot read text from outside in this turn."),
        ];
        for (d, why) in cases {
            assert_eq!(link_only_reason(&d), Some(why), "{d}");
            assert!(!approvable(&d), "{d}");
        }
    }

    // Ada's N-T2: a tool that runs other actions or code inside it is link-only; a phone shows
    // only the wrapper's name
    #[test]
    fn tools_that_run_other_actions_inside_them_are_link_only() {
        for tool in ["COMPOSIO_MULTI_EXECUTE_TOOL", "COMPOSIO_REMOTE_WORKBENCH", "COMPOSIO_REMOTE_BASH_TOOL", "composio_multi_execute_tool", "run_shell"] {
            let d = json!({"source": "mcp", "server": "composio", "tool": tool});
            assert_eq!(link_only_reason(&d), Some("This step runs other actions inside it."), "{d}");
            assert!(!approvable(&d), "{d}");
        }
        // kwant's own tools keep their earlier reason
        let own = json!({"source": "mcp", "server": "kwnt_tools", "tool": "shell_thing"});
        assert_eq!(link_only_reason(&own), Some("This is one of kwant's own tools."));
        // An ordinary tool still gets Allow once
        let send = json!({"source": "mcp", "server": "composio", "tool": "GMAIL_SEND_EMAIL"});
        assert_eq!(link_only_reason(&send), None);
        assert!(approvable(&send));
    }
}
