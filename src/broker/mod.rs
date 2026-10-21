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
    /// that tool again in that one conversation, until kwnt restarts.
    always_allowed: Arc<Mutex<HashSet<(String, String, String)>>>,
    /// Calls the person allowed after their turn had stopped waiting: the resumed turn's same
    /// call runs once without a new card, until the grant lapses.
    grants: Arc<Mutex<Vec<Grant>>>,
}

/// How long a late Allow waits for the resumed turn to make its call again.
pub const GRANT_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
struct Grant {
    kwant_id: String,
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
        }
    }

    /// Lets one later call of `tool_name` with these `arguments` by this kwant run without a
    /// card (a late Allow). Lapses after [`GRANT_TTL`].
    pub async fn grant_once(&self, kwant_id: &str, tool_name: &str, arguments: &Value) {
        let mut grants = self.grants.lock().await;
        grants.retain(|g| g.until > std::time::Instant::now());
        grants.push(Grant {
            kwant_id: kwant_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: grant_arguments(arguments),
            until: std::time::Instant::now() + GRANT_TTL,
        });
    }

    /// Uses up a matching [`grant_once`](Self::grant_once); false when there is none.
    pub async fn take_grant(&self, kwant_id: &str, tool_name: &str, arguments: &Value) -> bool {
        let mut grants = self.grants.lock().await;
        let now = std::time::Instant::now();
        grants.retain(|g| g.until > now);
        let wanted = grant_arguments(arguments);
        match grants.iter().position(|g| g.kwant_id == kwant_id && g.tool_name == tool_name && g.arguments == wanted) {
            Some(i) => {
                grants.remove(i);
                true
            }
            None => false,
        }
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
    /// else the channel or direct chat). Another conversation with the same bot asks again.
    pub async fn allow_for_session(&self, kwant_id: &str, conversation: &str, tool_name: &str) {
        self.always_allowed.lock().await.insert((
            kwant_id.to_string(),
            conversation.to_string(),
            tool_name.to_string(),
        ));
    }

    /// Forgets a deleted bot's "always allow" answers.
    pub async fn forget_kwant(&self, kwant_id: &str) {
        self.always_allowed.lock().await.retain(|(k, _, _)| k != kwant_id);
        self.grants.lock().await.retain(|g| g.kwant_id != kwant_id);
    }

    pub async fn is_allowed_for_session(&self, kwant_id: &str, conversation: &str, tool_name: &str) -> bool {
        self.always_allowed.lock().await.contains(&(
            kwant_id.to_string(),
            conversation.to_string(),
            tool_name.to_string(),
        ))
    }
}
