use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
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
}

impl PermissionBroker {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
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

    /// Resolves an approval ticket by sending the user's decision to the waiting oneshot channel.
    pub async fn resolve(&self, ticket_id: &str, decision: ApprovalDecision) -> bool {
        let mut map = self.pending.lock().await;
        if let Some(sender) = map.remove(ticket_id) {
            sender.send(decision).is_ok()
        } else {
            false
        }
    }
}
