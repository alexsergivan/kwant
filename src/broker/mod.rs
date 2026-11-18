use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
}

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
        let mut map = self.pending.lock().await;
        if let Some(sender) = map.remove(ticket_id) {
            sender.send(decision).is_ok()
        } else {
            false
        }
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
