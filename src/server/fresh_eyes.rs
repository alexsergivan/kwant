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
//! - is registered here for as long as it runs, and of kwant's own tools only [`ALLOWED_TOOLS`]
//!   are offered and run during it: a tool added later (one that reads chats, notes or facts
//!   included) is off in a review until it is put on that list.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// The only kwant tools a fresh-eyes turn sees and runs (an allowlist, Rigel's QA of #456,
/// finding B): the two that record a review's verdict, and the bot browser and page source for a
/// result that is a web page. Nothing here reads a chat, a note, a memory, a fact, another task or
/// a hand-off. Any other tool, including one added later, is hidden and refused.
pub const ALLOWED_TOOLS: &[&str] = &[
    // A hand-off review's verdict (delegation.rs, lead review)
    "task_review",
    // The Swarm Auditor's challenge
    "audit_challenge",
    // Looking at a result that is a web page
    "browser_open",
    "browser_page",
    "browser_act",
    "browser_cite",
    "browser_screenshot",
    "page_source",
];

/// Whether a fresh-eyes turn may see and run the kwant tool `name`.
pub fn allowed(name: &str) -> bool {
    ALLOWED_TOOLS.contains(&name)
}

/// What a blocked tool answers.
pub const REFUSED_TEXT: &str = "This is a fresh-eyes review: judge only the brief or spec and the result in your prompt. \
     Only the tools that record your verdict and the browser are on for this turn.";

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

/// Whether a fresh-eyes turn refuses the kwant tool `name`: any tool not in [`ALLOWED_TOOLS`].
pub fn refuses(turn_id: Option<&str>, name: &str) -> bool {
    !allowed(name) && is_fresh(turn_id)
}

/// Keeps only [`ALLOWED_TOOLS`] in a `tools/list` answer for a fresh-eyes turn.
pub fn filter_tools(turn_id: Option<&str>, tools: &mut Value) {
    if !is_fresh(turn_id) {
        return;
    }
    if let Some(list) = tools.as_array_mut() {
        list.retain(|t| t["name"].as_str().is_some_and(allowed));
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
            // The truth ledger is the team's notes (Ada's review of #456, must-fix 4)
            for t in ["fact_search", "fact_get", "task_cite_fact"] {
                assert!(refuses(Some("fe-turn-1"), t), "{t}");
            }
            // An allowlist: anything not on it is off, writes and lookups alike
            for t in ["fact_propose", "fact_dispute", "memory_add", "delegate", "skill_get", "bot_list", "ask_user", "a_tool_added_next_year"] {
                assert!(refuses(Some("fe-turn-1"), t), "{t}");
            }
            assert!(!refuses(Some("fe-turn-1"), "audit_challenge"), "the review's own tools still run");
            assert!(!refuses(Some("fe-turn-1"), "task_review"));
            assert!(!refuses(Some("fe-turn-1"), "browser_page"));
            assert!(!refuses(Some("fe-turn-2"), "search_history"), "another turn is untouched");
            assert!(!refuses(None, "search_history"));
        }
        assert!(!is_fresh(Some("fe-turn-1")));
    }

    #[test]
    fn only_allowed_tools_are_in_a_fresh_turns_list() {
        let _g = Guard::new("fe-turn-list");
        let mut tools = json!([{"name": "search_history"}, {"name": "task_review"}, {"name": "memory_search"}, {"name": "fact_search"},
            {"name": "fact_get"}, {"name": "task_cite_fact"}, {"name": "audit_challenge"}, {"name": "fact_propose"},
            {"name": "chat_reader_added_later"}, {"name": "browser_page"}, {"description": "no name"}]);
        filter_tools(Some("fe-turn-list"), &mut tools);
        let names: Vec<&str> = tools.as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, ["task_review", "audit_challenge", "browser_page"], "an unknown tool is absent");
        assert_eq!(tools.as_array().unwrap().len(), 3, "a tool with no name is dropped too");
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
