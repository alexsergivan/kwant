use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    AlwaysAllowSession,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub kwant_id: String,
    pub tool_name: String,
    pub details: Value,
}

#[derive(Clone, Debug, Default)]
pub struct PermissionBroker {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    /// (kwant_id, conversation, tool_name) the user chose "always allow" for: that bot may use
    /// that tool again in that one conversation. The fast path: each one is also written to the
    /// `always_allowed` table once a database is attached, and loaded from it after a restart.
    always_allowed: Arc<Mutex<HashSet<(String, String, String)>>>,
    /// Where "always allow" answers are kept across restarts (see [`attach_store`](Self::attach_store)).
    /// None: they last until kwnt restarts (a broker made on its own, in tests).
    store: Arc<std::sync::OnceLock<crate::db::Database>>,
    /// The stored answers were read into `always_allowed` (once per broker).
    loaded: Arc<tokio::sync::OnceCell<()>>,
    /// Calls the person allowed after their turn had stopped waiting: the resumed turn's same
    /// call runs once without a new card, until the grant lapses.
    grants: Arc<Mutex<Vec<Grant>>>,
    /// 27b: what the person declined, per bot and conversation, until they write there again.
    /// In memory only.
    pub declined: crate::mcp::declined::Declines,
    /// 29o: the person's reason on a Deny, by ticket, from [`resolve_with_reason`](Self::resolve_with_reason)
    /// until the waiting call takes it. At most [`DENY_REASONS_KEPT`]: a call whose turn ended
    /// in the same instant never takes its own, and the oldest goes first.
    deny_reasons: Arc<std::sync::Mutex<VecDeque<(String, String)>>>,
}

/// 29o: how many untaken Deny reasons the broker keeps at most.
pub const DENY_REASONS_KEPT: usize = 32;

/// How long a late Allow waits for the resumed turn to make its call again.
pub const GRANT_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
struct Grant {
    kwant_id: String,
    /// The chat the card belonged to (thread, else channel or direct chat): the grant only
    /// works there, not in another conversation of the same bot.
    conversation: String,
    tool_name: String,
    arguments: Value,
    until: std::time::Instant,
}

/// Arguments compared for a grant: claude's free-text `description` of a Bash call may change
/// between attempts, what runs may not.
fn grant_arguments(arguments: &Value) -> Value {
    let mut a = arguments.clone();
    if let Some(map) = a.as_object_mut() {
        map.remove("description");
    }
    a
}

impl PermissionBroker {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            always_allowed: Arc::new(Mutex::new(HashSet::new())),
            grants: Default::default(),
            declined: Default::default(),
            deny_reasons: Default::default(),
            store: Default::default(),
            loaded: Default::default(),
        }
    }

    /// Keeps "always allow" answers in `db` from now on, and reads the ones stored there before
    /// the first lookup. The first database attached wins. [`AppState::new`](crate::server::AppState::new)
    /// attaches its own.
    pub fn attach_store(&self, db: crate::db::Database) {
        let _ = self.store.set(db);
    }

    /// Reads the stored "always allow" answers into memory, once; how many were read (0 on the
    /// later calls). kwnt calls it at startup; every lookup and change makes sure of it first.
    pub async fn load_saved(&self) -> usize {
        let mut read = 0;
        self.loaded
            .get_or_init(|| async {
                let Some(db) = self.store.get() else { return };
                match db.all_always_allowed().await {
                    Ok(rows) => {
                        read = rows.len();
                        self.always_allowed.lock().await.extend(rows);
                    }
                    // The answers in memory still hold for this run; a later start tries again
                    Err(e) => tracing::warn!(error = %e, "could not read the stored \"always allow\" answers"),
                }
            })
            .await;
        read
    }

    /// Lets one later call of `tool_name` with these `arguments` by this kwant, in this
    /// `conversation`, run without a card (a late Allow). Lapses after [`GRANT_TTL`], or when
    /// [`drop_grant`](Self::drop_grant) is called because the resumed turn ended without it.
    pub async fn grant_once(&self, kwant_id: &str, conversation: &str, tool_name: &str, arguments: &Value) {
        let mut grants = self.grants.lock().await;
        grants.retain(|g| g.until > std::time::Instant::now());
        grants.push(Grant {
            kwant_id: kwant_id.to_string(),
            conversation: conversation.to_string(),
            tool_name: tool_name.to_string(),
            arguments: grant_arguments(arguments),
            until: std::time::Instant::now() + GRANT_TTL,
        });
    }

    /// Uses up a matching [`grant_once`](Self::grant_once); false when there is none.
    pub async fn take_grant(&self, kwant_id: &str, conversation: &str, tool_name: &str, arguments: &Value) -> bool {
        let mut grants = self.grants.lock().await;
        let now = std::time::Instant::now();
        grants.retain(|g| g.until > now);
        match Self::find_grant(&grants, kwant_id, conversation, tool_name, arguments) {
            Some(i) => {
                grants.remove(i);
                true
            }
            None => false,
        }
    }

    /// Forgets a [`grant_once`](Self::grant_once) the resumed turn didn't use, so the same call
    /// later asks again. True when there was one.
    pub async fn drop_grant(&self, kwant_id: &str, conversation: &str, tool_name: &str, arguments: &Value) -> bool {
        let mut grants = self.grants.lock().await;
        match Self::find_grant(&grants, kwant_id, conversation, tool_name, arguments) {
            Some(i) => {
                grants.remove(i);
                true
            }
            None => false,
        }
    }

    fn find_grant(grants: &[Grant], kwant_id: &str, conversation: &str, tool_name: &str, arguments: &Value) -> Option<usize> {
        let wanted = grant_arguments(arguments);
        grants.iter().position(|g| {
            g.kwant_id == kwant_id && g.conversation == conversation && g.tool_name == tool_name && g.arguments == wanted
        })
    }

    /// Registers a pending approval ticket and returns a receiver waiting for user choice.
    pub async fn request_approval(&self, request: ApprovalRequest) -> oneshot::Receiver<ApprovalDecision> {
        let (tx, rx) = oneshot::channel();
        let mut map = self.pending.lock().await;
        map.insert(request.id, tx);
        rx
    }

    /// Checks if a ticket is currently pending user resolution.
    pub async fn is_pending(&self, ticket_id: &str) -> bool {
        let map = self.pending.lock().await;
        map.contains_key(ticket_id)
    }

    /// Tickets still waiting for the user. A stored ticket not in here (kwnt restarted, the
    /// turn ended) can no longer be answered.
    pub async fn pending_ids(&self) -> Vec<String> {
        self.pending.lock().await.keys().cloned().collect()
    }

    /// Resolves an approval ticket by sending the user's decision to the waiting oneshot channel.
    pub async fn resolve(&self, ticket_id: &str, decision: ApprovalDecision) -> bool {
        self.resolve_with_reason(ticket_id, decision, None).await
    }

    /// [`resolve`](Self::resolve), with the reason the person gave for a Deny (29o): kept for
    /// the waiting call ([`take_deny_reason`](Self::take_deny_reason)) before it hears the
    /// decision. Ignored for an Allow, and dropped when no call waits (a late answer).
    pub async fn resolve_with_reason(&self, ticket_id: &str, decision: ApprovalDecision, reason: Option<String>) -> bool {
        let mut map = self.pending.lock().await;
        let Some(sender) = map.remove(ticket_id) else {
            return false;
        };
        let reason = reason.filter(|_| decision == ApprovalDecision::Deny);
        if let Some(why) = reason.clone() {
            self.keep_deny_reason(ticket_id, why);
        }
        let sent = sender.send(decision).is_ok();
        if !sent && reason.is_some() {
            self.take_deny_reason(ticket_id);
        }
        sent
    }

    fn keep_deny_reason(&self, ticket_id: &str, reason: String) {
        if let Ok(mut kept) = self.deny_reasons.lock() {
            kept.retain(|(t, _)| t != ticket_id);
            while kept.len() >= DENY_REASONS_KEPT {
                kept.pop_front();
            }
            kept.push_back((ticket_id.to_string(), reason));
        }
    }

    /// 29o: the reason the person gave when they denied `ticket_id`, once; `None` for a plain Deny.
    pub fn take_deny_reason(&self, ticket_id: &str) -> Option<String> {
        let mut kept = self.deny_reasons.lock().ok()?;
        let i = kept.iter().position(|(t, _)| t == ticket_id)?;
        kept.remove(i).map(|(_, why)| why)
    }

    /// 29o: how many Deny reasons wait for a call to take them.
    pub fn deny_reasons_held(&self) -> usize {
        self.deny_reasons.lock().map(|k| k.len()).unwrap_or(0)
    }

    /// Drops a pending ticket without a decision; its waiter sees the channel close.
    pub async fn cancel(&self, ticket_id: &str) -> bool {
        self.pending.lock().await.remove(ticket_id).is_some()
    }

    /// Remembers an "always allow" answer for this kwant and tool in one conversation (a thread,
    /// else the channel or direct chat), also after kwnt restarts. Another conversation with the
    /// same bot asks again. If it can't be stored it still holds until kwnt restarts.
    pub async fn allow_for_session(&self, kwant_id: &str, conversation: &str, tool_name: &str) {
        self.load_saved().await;
        if let Some(db) = self.store.get() {
            if let Err(e) = db.save_always_allowed(kwant_id, conversation, tool_name).await {
                tracing::warn!(kwant = %kwant_id, tool = %tool_name, error = %e, "could not store an \"always allow\": it lasts until kwnt restarts");
            }
        }
        self.always_allowed.lock().await.insert((
            kwant_id.to_string(),
            conversation.to_string(),
            tool_name.to_string(),
        ));
    }

    /// The person takes back one "always allow": the next call asks again. True when it was
    /// there, in memory or stored.
    pub async fn revoke(&self, kwant_id: &str, conversation: &str, tool_name: &str) -> Result<bool, crate::db::DbError> {
        self.load_saved().await;
        // Memory first: it stops working at once, even if the stored row can't be removed
        let in_memory = self.always_allowed.lock().await.remove(&(
            kwant_id.to_string(),
            conversation.to_string(),
            tool_name.to_string(),
        ));
        let stored = match self.store.get() {
            Some(db) => db.delete_always_allowed(kwant_id, conversation, tool_name).await?,
            None => false,
        };
        Ok(in_memory || stored)
    }

    /// The person takes back all of a bot's "always allow" answers; how many there were.
    pub async fn revoke_all(&self, kwant_id: &str) -> Result<u64, crate::db::DbError> {
        self.load_saved().await;
        let in_memory = {
            let mut set = self.always_allowed.lock().await;
            let before = set.len();
            set.retain(|(k, _, _)| k != kwant_id);
            (before - set.len()) as u64
        };
        let stored = match self.store.get() {
            Some(db) => db.delete_all_always_allowed(kwant_id).await?,
            None => 0,
        };
        Ok(in_memory.max(stored))
    }

    /// Forgets a deleted bot's "always allow" answers (its stored rows go with the bot).
    pub async fn forget_kwant(&self, kwant_id: &str) {
        self.load_saved().await;
        self.always_allowed.lock().await.retain(|(k, _, _)| k != kwant_id);
        self.grants.lock().await.retain(|g| g.kwant_id != kwant_id);
        self.declined.forget_kwant(kwant_id);
        if let Some(db) = self.store.get() {
            if let Err(e) = db.delete_all_always_allowed(kwant_id).await {
                tracing::warn!(kwant = %kwant_id, error = %e, "could not forget a deleted bot's \"always allow\" answers");
            }
        }
    }

    /// Forgets the "always allow" answers in a deleted thread or channel.
    pub async fn forget_conversation(&self, conversation: &str) {
        self.load_saved().await;
        self.always_allowed.lock().await.retain(|(_, c, _)| c != conversation);
        self.grants.lock().await.retain(|g| g.conversation != conversation);
        self.declined.forget_conversation(conversation);
        if let Some(db) = self.store.get() {
            if let Err(e) = db.delete_always_allowed_in(conversation).await {
                tracing::warn!(conversation, error = %e, "could not forget a deleted conversation's \"always allow\" answers");
            }
        }
    }

    pub async fn is_allowed_for_session(&self, kwant_id: &str, conversation: &str, tool_name: &str) -> bool {
        self.load_saved().await;
        self.always_allowed.lock().await.contains(&(
            kwant_id.to_string(),
            conversation.to_string(),
            tool_name.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(id: &str) -> ApprovalRequest {
        ApprovalRequest { id: id.into(), kwant_id: "k1".into(), tool_name: "Bash".into(), details: Value::Null }
    }

    /// 29o: a Deny's reason waits for its own call, once; an Allow, a late answer and a call that
    /// stopped waiting leave nothing behind, and untaken ones never pile up.
    #[tokio::test]
    async fn a_deny_reason_waits_for_its_own_call_only() {
        let b = PermissionBroker::new();
        let rx = b.request_approval(ask("a1")).await;
        assert!(b.resolve_with_reason("a1", ApprovalDecision::Deny, Some("use staging".into())).await);
        assert_eq!(rx.await.unwrap(), ApprovalDecision::Deny);
        assert_eq!(b.take_deny_reason("a1").as_deref(), Some("use staging"));
        assert_eq!(b.take_deny_reason("a1"), None, "taken once");

        let rx = b.request_approval(ask("a2")).await;
        assert!(b.resolve_with_reason("a2", ApprovalDecision::AllowOnce, Some("why".into())).await);
        assert_eq!(rx.await.unwrap(), ApprovalDecision::AllowOnce);
        assert_eq!(b.deny_reasons_held(), 0, "an Allow keeps no reason");

        assert!(!b.resolve_with_reason("gone", ApprovalDecision::Deny, Some("late".into())).await);
        assert_eq!(b.deny_reasons_held(), 0, "no call waits: dropped");

        let rx = b.request_approval(ask("a3")).await;
        drop(rx);
        assert!(!b.resolve_with_reason("a3", ApprovalDecision::Deny, Some("nobody hears".into())).await);
        assert_eq!(b.deny_reasons_held(), 0, "the call stopped waiting: dropped");

        for i in 0..DENY_REASONS_KEPT + 5 {
            let id = format!("n{i}");
            let _rx = b.request_approval(ask(&id)).await;
            assert!(b.resolve_with_reason(&id, ApprovalDecision::Deny, Some("x".into())).await);
        }
        assert_eq!(b.deny_reasons_held(), DENY_REASONS_KEPT);
        assert_eq!(b.take_deny_reason("n0"), None, "the oldest went first");
        assert!(b.take_deny_reason(&format!("n{}", DENY_REASONS_KEPT + 4)).is_some());
    }

    #[tokio::test]
    async fn without_a_store_an_answer_lasts_for_this_broker_only() {
        let b = PermissionBroker::new();
        b.allow_for_session("k1", "th1", "Bash").await;
        assert!(b.is_allowed_for_session("k1", "th1", "Bash").await);
        assert!(!b.is_allowed_for_session("k1", "th2", "Bash").await);
        assert!(!PermissionBroker::new().is_allowed_for_session("k1", "th1", "Bash").await);
        assert!(b.revoke("k1", "th1", "Bash").await.unwrap());
        assert!(!b.is_allowed_for_session("k1", "th1", "Bash").await);
        assert!(!b.revoke("k1", "th1", "Bash").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_all_counts_one_bots_answers_only() {
        let b = PermissionBroker::new();
        b.allow_for_session("k1", "th1", "Bash").await;
        b.allow_for_session("k1", "th2", "Write").await;
        b.allow_for_session("k2", "th1", "Bash").await;
        assert_eq!(b.revoke_all("k1").await.unwrap(), 2);
        assert!(b.is_allowed_for_session("k2", "th1", "Bash").await);
        b.forget_conversation("th1").await;
        assert!(!b.is_allowed_for_session("k2", "th1", "Bash").await);
    }
}
