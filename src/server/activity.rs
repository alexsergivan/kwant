//! A running turn's live activity (see [`crate::drivers::activity`]): the turn's events feed its
//! log, and every changed step goes out as a scoped `activity` event. Turns are registered by id
//! so code that only knows the turn id (approval cards, human steps) can add a waiting step.

use crate::drivers::activity::{ActivityLog, Step};
use crate::drivers::events::{RuntimeEvent, TurnTag};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::broadcast;

/// The activity of one running turn.
pub struct TurnActivity {
    log: Mutex<ActivityLog>,
    sender: broadcast::Sender<RuntimeEvent>,
    tag: TurnTag,
}

fn turns() -> &'static Mutex<HashMap<String, Weak<TurnActivity>>> {
    static TURNS: OnceLock<Mutex<HashMap<String, Weak<TurnActivity>>>> = OnceLock::new();
    TURNS.get_or_init(Default::default)
}

impl TurnActivity {
    /// Starts the turn's log and registers it under its turn id until it is dropped.
    pub fn start(sender: broadcast::Sender<RuntimeEvent>, tag: TurnTag) -> Arc<Self> {
        let this = Arc::new(Self { log: Mutex::new(ActivityLog::default()), sender, tag });
        let mut all = turns().lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|_, w| w.strong_count() > 0);
        all.insert(this.tag.turn_id.clone(), Arc::downgrade(&this));
        this
    }

    /// The running turn with this id, if any.
    pub fn find(turn_id: &str) -> Option<Arc<Self>> {
        turns().lock().unwrap_or_else(|e| e.into_inner()).get(turn_id).and_then(Weak::upgrade)
    }

    fn with_log<T>(&self, f: impl FnOnce(&mut ActivityLog) -> T) -> T {
        f(&mut self.log.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn send(&self, steps: impl IntoIterator<Item = Step>) {
        for step in steps {
            let _ = self.sender.send(RuntimeEvent::Scoped {
                tag: self.tag.clone(),
                event: Box::new(RuntimeEvent::Activity { step: step.to_json() }),
            });
        }
    }

    /// Masks these values on every step from now on (the turn's MCP secrets).
    pub fn add_secrets(&self, secrets: Vec<String>) {
        self.with_log(|log| log.add_secrets(secrets));
    }

    /// Masks the turn's secrets, credentials and home folders in text saved for others to read.
    pub fn redact(&self, text: &str) -> String {
        crate::drivers::activity::hide_home_paths(&self.with_log(|log| log.redact(text)))
    }

    pub fn observe(&self, event: &RuntimeEvent) {
        let steps = self.with_log(|log| log.observe(event));
        self.send(steps);
    }

    pub fn note(&self, title: &str) {
        let step = self.with_log(|log| log.note(title));
        self.send(step);
    }

    pub fn wait(&self, kind: &'static str, ref_id: &str, title: &str, tool: Option<&str>) {
        let steps = self.with_log(|log| log.wait(kind, ref_id, title, tool));
        self.send(steps);
    }

    pub fn settle(&self, ref_id: &str, status: &'static str) {
        let step = self.with_log(|log| log.settle(ref_id, status));
        self.send(step);
    }

    /// Ends the turn's open steps and returns what to save on the reply as `meta.activity`
    /// (`None` when nothing happened).
    pub fn finish(&self) -> Option<Value> {
        let (steps, saved) = self.with_log(|log| {
            let steps = log.finish();
            (steps, (!log.is_empty()).then(|| log.to_json()))
        });
        self.send(steps);
        saved
    }
}

/// An approval card for `turn_id` is waiting on the person.
pub fn approval_waiting(turn_id: &str, ticket: &str, tool_name: &str) {
    if let Some(turn) = TurnActivity::find(turn_id) {
        let (title, _) = crate::drivers::activity::summarize(tool_name, &Value::Null);
        turn.wait("approval", ticket, &format!("Waiting for your approval: {title}"), Some(tool_name));
    }
}

/// An approval card for `turn_id` was answered ("ok" allowed, "declined") or stopped waiting.
pub fn approval_settled(turn_id: &str, ticket: &str, status: &'static str) {
    if let Some(turn) = TurnActivity::find(turn_id) {
        turn.settle(ticket, status);
    }
}

/// Adds `meta.activity` to a reply's meta.
pub fn with_activity(meta: Option<Value>, activity: Option<Value>) -> Option<Value> {
    let Some(activity) = activity else { return meta };
    let mut meta = meta.unwrap_or_else(|| serde_json::json!({}));
    if meta.is_object() {
        meta["activity"] = activity;
    }
    Some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steps_go_out_scoped_and_approvals_find_their_turn() {
        let (tx, mut rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-activity-test".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        approval_waiting("turn-activity-test", "ap", "Bash");
        approval_settled("turn-activity-test", "ap", "ok");
        approval_waiting("another-turn", "ap2", "Bash");
        let wire: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).map(|e| e.to_wire()).collect();
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[0]["type"], "activity");
        assert_eq!(wire[0]["turn_id"], "turn-activity-test");
        assert_eq!(wire[0]["data"]["step"]["title"], "Running `ls`");
        assert_eq!(wire[1]["data"]["step"]["status"], "waiting");
        assert_eq!(wire[2]["data"]["step"]["status"], "ok");
        let saved = turn.finish().unwrap();
        assert_eq!(saved["steps"][0]["status"], "stopped");
        drop(turn);
        assert!(TurnActivity::find("turn-activity-test").is_none());
        assert_eq!(with_activity(None, Some(json!({"steps": []}))).unwrap()["activity"]["steps"], json!([]));
    }
}
