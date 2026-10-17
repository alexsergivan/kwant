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
    /// (kwant_id, tool_name) pairs the user chose "always allow" for, until kwnt restarts.
    always_allowed: Arc<Mutex<HashSet<(String, String)>>>,
}

impl PermissionBroker {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            always_allowed: Arc::new(Mutex::new(HashSet::new())),
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

    /// Remembers an "always allow" answer for this kwant and tool for the rest of the session.
    pub async fn allow_for_session(&self, kwant_id: &str, tool_name: &str) {
        self.always_allowed.lock().await.insert((kwant_id.to_string(), tool_name.to_string()));
    }

    pub async fn is_allowed_for_session(&self, kwant_id: &str, tool_name: &str) -> bool {
        self.always_allowed.lock().await.contains(&(kwant_id.to_string(), tool_name.to_string()))
    }
}
