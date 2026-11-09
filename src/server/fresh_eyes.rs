//! B1, "Checked with fresh eyes": a reviewer turn sees only the brief (or a written spec) and the
//! result, never the author's conversation. See
//! `/home/maus/shared/ship-software-safely/release-080/fresh-eyes-audit.md` for the places a
//! reviewer runs and the contract the UI reads.
//!
//! A turn started with `TurnOpts::fresh_eyes`:
//! - gets no chat history (`TurnInput::history` is empty) and no driver reads the conversation, its
//!   saved summary or cached prefix back from the store (`TurnInput::fresh_eyes`);
//! - gets none of the reviewer's own notes or lessons in its prompt, only the person's pinned team
//!   and channel instructions;
//! - is registered here for as long as it runs, and kwant's own tools that read other chats, notes,
//!   tasks or hand-offs refuse during it ([`BLOCKED_TOOLS`]).

use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// kwant's own tools a fresh-eyes turn neither sees nor runs: each reads something besides the
/// brief and the result (chats, the reviewer's notes, other tasks' notes, hand-off results,
/// attachments of chats).
pub const BLOCKED_TOOLS: &[&str] =
    &["search_history", "memory_search", "task_list", "delegation_status", "list_attachments", "read_attachment"];

/// What a blocked tool answers.
pub const REFUSED_TEXT: &str = "This is a fresh-eyes review: judge only the brief or spec and the result in your prompt. \
     Tools that read chats, notes, other tasks or hand-offs are off for this turn.";

/// The parts a fresh-eyes reviewer is shown, in the order the label lists them.
pub const SAW_BRIEF: &str = "brief";
pub const SAW_SPEC: &str = "spec";
pub const SAW_RESULT: &str = "result";
pub const SAW_DIFF: &str = "diff";

static FRESH: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Marks a turn as fresh-eyes while it is alive; dropping it ends the mark.
pub struct Guard(String);

impl Guard {
    pub fn new(turn_id: &str) -> Self {
        FRESH.lock().unwrap_or_else(|e| e.into_inner()).insert(turn_id.to_string());
        Guard(turn_id.to_string())
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        FRESH.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// Whether `turn_id` is a running fresh-eyes turn.
pub fn is_fresh(turn_id: Option<&str>) -> bool {
    turn_id.is_some_and(|id| FRESH.lock().unwrap_or_else(|e| e.into_inner()).contains(id))
}

/// Whether a fresh-eyes turn refuses the kwant tool `name`.
pub fn refuses(turn_id: Option<&str>, name: &str) -> bool {
    BLOCKED_TOOLS.contains(&name) && is_fresh(turn_id)
}

/// Drops the blocked tools from a `tools/list` answer for a fresh-eyes turn.
pub fn filter_tools(turn_id: Option<&str>, tools: &mut Value) {
    if !is_fresh(turn_id) {
        return;
    }
    if let Some(list) = tools.as_array_mut() {
        list.retain(|t| !t["name"].as_str().is_some_and(|n| BLOCKED_TOOLS.contains(&n)));
    }
}

/// The "Checked with fresh eyes" label stored on a review card or an audit (contract:
/// fresh-eyes-audit.md §3): `checked` is true only when the reviewer turn ran fresh-eyes and
/// answered; `saw` lists what it was shown; `spec_source` is `written`, `acceptance`, `brief`,
/// `ticket`, `pr` (only the author's own PR text or commit message) or `none`; `reviewer` is the reviewing bot's id (never the author's).
pub fn label(checked: bool, saw: &[&str], spec_source: &str, reviewer: Option<&str>) -> Value {
    json!({
        "checked": checked,
        "saw": saw,
        "spec_source": spec_source,
        "reviewer_kwant_id": reviewer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guard_marks_the_turn_only_while_it_lives() {
        assert!(!is_fresh(Some("fe-turn-1")));
        {
            let _g = Guard::new("fe-turn-1");
            assert!(is_fresh(Some("fe-turn-1")));
            assert!(refuses(Some("fe-turn-1"), "search_history"));
            assert!(refuses(Some("fe-turn-1"), "memory_search"));
            assert!(!refuses(Some("fe-turn-1"), "audit_challenge"), "the review's own tools still run");
            assert!(!refuses(Some("fe-turn-2"), "search_history"), "another turn is untouched");
            assert!(!refuses(None, "search_history"));
        }
        assert!(!is_fresh(Some("fe-turn-1")));
    }

    #[test]
    fn blocked_tools_are_hidden_from_a_fresh_turns_list() {
        let _g = Guard::new("fe-turn-list");
        let mut tools = json!([{"name": "search_history"}, {"name": "task_review"}, {"name": "memory_search"}, {"name": "audit_challenge"}]);
        filter_tools(Some("fe-turn-list"), &mut tools);
        let names: Vec<&str> = tools.as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["task_review", "audit_challenge"]);
        let mut other = json!([{"name": "search_history"}]);
        filter_tools(Some("not-fresh"), &mut other);
        assert_eq!(other.as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_label_names_what_the_reviewer_saw() {
        let l = label(true, &[SAW_SPEC, SAW_RESULT], "written", Some("rev"));
        assert_eq!(l, json!({"checked": true, "saw": ["spec", "result"], "spec_source": "written", "reviewer_kwant_id": "rev"}));
    }
}
