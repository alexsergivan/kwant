//! 15c: a turn stops sending a call that keeps failing the same way.
//!
//! [`McpToolSession`](super::session::McpToolSession) holds one guard per turn. After a call,
//! [`RepeatGuard::after`] counts identical failures (same tool, same arguments, same error text);
//! before a call, [`RepeatGuard::check`] refuses a call that already failed [`REPEAT_LIMIT`]
//! times the same way, so the turn tries another way instead of spending its call cap on it.
//! Successful calls (polling a status) and calls whose error changes never stop. Only `ok` and
//! `error` outcomes count: a denied, unanswered or rehearsed call was the person's decision.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Identical failures after which the same call is no longer sent.
pub const REPEAT_LIMIT: u32 = 3;
/// Calls the guard remembers per turn; the least recently failed one is dropped first.
pub const MAX_KEYS: usize = 64;
/// How much of the repeated error the stop text quotes.
const QUOTE_MAX_CHARS: usize = 200;

struct Entry {
    key: [u8; 32],
    error: String,
    count: u32,
}

/// One turn's failing calls, oldest first. Interior mutability: calls may run concurrently.
#[derive(Default)]
pub struct RepeatGuard {
    entries: Mutex<Vec<Entry>>,
}

/// What the guard says before a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// The text the model gets instead of the call's result.
    pub text: String,
    /// How many identical failures came before ([`REPEAT_LIMIT`]).
    pub count: u32,
}

impl RepeatGuard {
    /// The same call is the same key: tool name plus the arguments' JSON. serde_json here has
    /// no `preserve_order`, so object keys serialize sorted and key order doesn't matter.
    /// Hashed, so 64 large argument objects don't stay in memory.
    pub fn key(name: &str, arguments: &Value) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(name.as_bytes());
        h.update([0u8]);
        h.update(arguments.to_string().as_bytes());
        h.finalize().into()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Some` when this call already failed [`REPEAT_LIMIT`] times in a row with the same error:
    /// don't send it.
    pub fn check(&self, key: &[u8; 32]) -> Option<Stopped> {
        let entries = self.lock();
        let e = entries.iter().find(|e| &e.key == key)?;
        (e.count >= REPEAT_LIMIT).then(|| Stopped { text: stop_text(e.count, &e.error), count: e.count })
    }

    /// Records a call's outcome: `error` with the same (redacted) text as last time counts one
    /// more, a different text starts again at 1, `ok` forgets the call; anything else (denied,
    /// unanswered, rehearsed) leaves the count as it is.
    pub fn after(&self, key: &[u8; 32], status: &str, text: &str) {
        let mut entries = self.lock();
        let at = entries.iter().position(|e| &e.key == key);
        match status {
            "ok" => {
                if let Some(i) = at {
                    entries.remove(i);
                }
            }
            "error" => {
                let text = text.trim();
                let mut entry = match at {
                    Some(i) => entries.remove(i),
                    None => Entry { key: *key, error: String::new(), count: 0 },
                };
                if entry.count > 0 && entry.error == text {
                    entry.count += 1;
                } else {
                    entry.error = text.to_string();
                    entry.count = 1;
                }
                if entries.len() >= MAX_KEYS {
                    entries.remove(0);
                }
                entries.push(entry);
            }
            _ => {}
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// What the model reads instead of a stopped call's result. No connection words of its own, so
/// tool_failures doesn't read it as a connection failure (the quoted error may have some; a
/// stopped call is skipped there by its `repeat_stopped` field).
pub fn stop_text(count: u32, error: &str) -> String {
    let error = super::truncate_text(error.trim(), QUOTE_MAX_CHARS);
    format!(
        "kwnt didn't send this call: the same call (same tool, same arguments) already failed {count} times in this \
         turn with the same error: \"{error}\". Change the arguments or try another way. If only the person can fix \
         it, end your reply with \"Blocked: <what is blocked>. <what would unblock it>\"."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fail(g: &RepeatGuard, k: &[u8; 32], times: u32, error: &str) {
        for _ in 0..times {
            assert!(g.check(k).is_none(), "stopped too early");
            g.after(k, "error", error);
        }
    }

    #[test]
    fn three_same_failures_stop_the_fourth() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__gh__create_issue", &json!({"repo": "a/b", "title": "x"}));
        fail(&g, &k, 3, "422 Validation Failed: title too short");
        let s = g.check(&k).expect("4th call stopped");
        assert_eq!(s.count, 3);
        assert!(s.text.starts_with("kwnt didn't send this call: the same call (same tool, same arguments) already failed 3 times"));
        assert!(s.text.contains("\"422 Validation Failed: title too short\""));
        assert!(s.text.ends_with("end your reply with \"Blocked: <what is blocked>. <what would unblock it>\"."));
        // Still stopped on a 5th try: the stop itself isn't recorded as a new outcome
        assert!(g.check(&k).is_some());
    }

    #[test]
    fn a_different_error_text_starts_again() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__a__b", &json!({}));
        fail(&g, &k, 2, "boom");
        g.after(&k, "error", "boom, but different");
        assert!(g.check(&k).is_none());
        g.after(&k, "error", "boom, but different");
        assert!(g.check(&k).is_none());
        g.after(&k, "error", "boom, but different");
        assert!(g.check(&k).is_some());
    }

    #[test]
    fn an_ok_forgets_the_call() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__a__status", &json!({"id": 7}));
        fail(&g, &k, 2, "not ready");
        g.after(&k, "ok", "ready");
        fail(&g, &k, 2, "not ready");
        assert!(g.check(&k).is_none());
        // Polling that keeps working never stops
        for _ in 0..20 {
            g.after(&k, "ok", "running");
        }
        assert!(g.check(&k).is_none());
    }

    #[test]
    fn different_arguments_are_a_different_call() {
        let g = RepeatGuard::default();
        let a = RepeatGuard::key("mcp__a__get", &json!({"id": 1}));
        let b = RepeatGuard::key("mcp__a__get", &json!({"id": 2}));
        let c = RepeatGuard::key("mcp__a__other", &json!({"id": 1}));
        fail(&g, &a, 3, "not found");
        assert!(g.check(&a).is_some());
        assert!(g.check(&b).is_none());
        assert!(g.check(&c).is_none());
    }

    #[test]
    fn key_order_in_arguments_does_not_matter() {
        let x: Value = serde_json::from_str(r#"{"b": 1, "a": {"y": 2, "x": 3}}"#).unwrap();
        let y: Value = serde_json::from_str(r#"{"a": {"x": 3, "y": 2}, "b": 1}"#).unwrap();
        assert_eq!(RepeatGuard::key("t", &x), RepeatGuard::key("t", &y));
        assert_ne!(RepeatGuard::key("t", &x), RepeatGuard::key("u", &x));
    }

    #[test]
    fn denied_unanswered_and_rehearsed_do_not_count() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__a__b", &json!({}));
        for status in ["denied", "unanswered", "rehearsed", "denied", "denied"] {
            g.after(&k, status, "The person declined this tool call.");
        }
        assert!(g.check(&k).is_none());
        // and don't reset a count either
        fail(&g, &k, 2, "boom");
        g.after(&k, "denied", "no");
        g.after(&k, "error", "boom");
        assert!(g.check(&k).is_some());
    }

    #[test]
    fn at_most_64_calls_are_remembered_oldest_dropped() {
        let g = RepeatGuard::default();
        let keys: Vec<_> = (0..65).map(|i| RepeatGuard::key("mcp__a__b", &json!({ "i": i }))).collect();
        fail(&g, &keys[0], 3, "boom");
        for k in &keys[1..] {
            g.after(k, "error", "boom");
        }
        assert_eq!(g.len(), MAX_KEYS);
        // The oldest one was dropped, so it is no longer stopped
        assert!(g.check(&keys[0]).is_none());
        fail(&g, &keys[64], 2, "boom");
        assert!(g.check(&keys[64]).is_some());
        assert_eq!(g.len(), MAX_KEYS);
    }

    #[test]
    fn a_long_error_is_cut_to_200_characters() {
        let long = "e".repeat(500);
        let t = stop_text(3, &long);
        assert!(t.contains(&format!("\"{}… [truncated]\"", "e".repeat(200))));
        assert!(!t.contains(&"e".repeat(201)));
    }
}
