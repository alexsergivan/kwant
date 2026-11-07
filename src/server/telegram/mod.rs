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

/// Whether this card's kind may be allowed from Telegram at all (the switch is checked apart).
pub fn approvable(details: &Value) -> bool {
    TELEGRAM_APPROVABLE.contains(&ApprovalKind::of(details)) && FORCING_KEYS.iter().all(|k| details.get(*k).is_none_or(Value::is_null))
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
    let (tx, rx) = mpsc::unbounded_channel();
    let runner = Arc::new(Runner {
        space: space.to_string(),
        client: api::Client::new(token),
        cancel: CancellationToken::new(),
        status: Mutex::new(Status::Offline),
        queued: AtomicUsize::new(0),
        tx,
        bad_code_replies: Mutex::new(HashMap::new()),
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
        assert!(approvable(&json!({"source": "mcp", "tool": "create_issue"})));
        assert!(approvable(&json!({"source": "claude_builtin", "tool": "Write"})));
        assert!(approvable(&json!({"source": "mcp", "payment_reason": null})));
        for d in [
            json!({"source": "local_shell"}),
            json!({"source": "review"}),
            json!({"source": "task"}),
            json!({"source": "tamper"}),
            json!({"source": "something_new"}),
            json!({}),
            json!({"source": "mcp", "payment_reason": "pays"}),
            json!({"source": "mcp", "tier_reason": "tier"}),
            json!({"source": "claude_builtin", "prod_floor": {"needs_backup": false}}),
        ] {
            assert!(!approvable(&d), "{d}");
        }
    }
}
