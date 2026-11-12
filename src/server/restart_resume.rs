//! 0.9.5 "Runs never stop silently": a reply that a kwnt restart cut off continues by itself,
//! once. A running turn records its tool steps as they start and finish
//! ([`crate::db::continuity::RunningTurn::steps`]); at the next start the turn runs again with
//! a note saying what was cut off. A step that could have changed something outside (sent,
//! posted, pushed, paid) is never re-run blindly: the bot is told it was cut off mid-call, checks
//! whether it took effect, and only then retries; if it can't tell, it asks the person once. A
//! step that only reads may simply run again.
//!
//! Bounded: the continuation is itself marked ([`crate::db::continuity::RunningTurn::resumed`]),
//! so a second restart asks the person instead (the notice with Retry and a Needs-you item). A
//! turn with a card or a browser step still waiting on the person isn't continued: answering
//! that resumes the work. The exact in-flight call isn't resumed (that needs a durable record per
//! call); the model makes it again or not, after checking.

use crate::db::continuity::RunningTurn;
use crate::drivers::RuntimeEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Steps kept per turn (the newest).
pub const MAX_STEPS: usize = 12;
/// A step's arguments are kept this short.
const MAX_ARGS_CHARS: usize = 200;

/// One tool step of a running turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub name: String,
    /// Its arguments as JSON, clipped.
    #[serde(default)]
    pub args: String,
    /// It only reads (see [`read_only`]).
    pub read_only: bool,
    /// It finished before the turn ended.
    pub done: bool,
}

/// A turn's steps as they happen.
#[derive(Debug, Default)]
pub struct Steps(Vec<Step>);

impl Steps {
    /// Notes a step starting or finishing; true when the record changed.
    pub fn saw(&mut self, e: &RuntimeEvent) -> bool {
        match e {
            RuntimeEvent::ToolCallPending { id, name, args } => {
                let args = clip(&args.to_string(), MAX_ARGS_CHARS);
                self.0.push(Step { id: id.clone(), read_only: read_only(name), name: name.clone(), args, done: false });
                if self.0.len() > MAX_STEPS {
                    self.0.remove(0);
                }
                true
            }
            RuntimeEvent::ToolCallCompleted { id, .. } => match self.0.iter_mut().rev().find(|s| &s.id == id && !s.done) {
                Some(s) => {
                    s.done = true;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    pub fn json(&self) -> String {
        serde_json::to_string(&self.0).unwrap_or_else(|_| "[]".into())
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// Tool names (after any `mcp__<server>__` prefix) that only read.
const READ_ONLY_TOOLS: &[&str] = &[
    "read", "grep", "glob", "ls", "websearch", "webfetch", "notebookread", "todowrite", "todoread", "bashoutput",
    "view_image", "web_search",
];
/// Name prefixes and parts that only read (a browser look, a lookup).
const READ_ONLY_PREFIXES: &[&str] = &["get_", "list_", "read_", "search_", "find_", "view_", "describe_", "fetch_"];
const READ_ONLY_PARTS: &[&str] = &["snapshot", "screenshot", "get_text", "get_url", "get_title", "wait_for", "tab_list"];

/// Whether a step only reads, so running it again is harmless. Anything not known to read (a
/// shell command, a click, a send) counts as able to change something.
pub fn read_only(name: &str) -> bool {
    let tool = name.rsplit("__").next().unwrap_or(name);
    let lower = tool.to_ascii_lowercase();
    if crate::server::continuity::READ_ONLY_SELF_TOOLS.contains(&tool) || READ_ONLY_TOOLS.contains(&lower.as_str()) {
        return true;
    }
    // A browser tool's own name follows its prefix (agent_browser_snapshot, browser_get_text)
    let bare = lower.trim_start_matches("agent_").trim_start_matches("browser_");
    READ_ONLY_PREFIXES.iter().any(|p| bare.starts_with(p)) || READ_ONLY_PARTS.iter().any(|p| bare.contains(p))
}

fn steps_of(t: &RunningTurn) -> Vec<Step> {
    t.steps.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
}

fn step_line(s: &Step) -> String {
    if s.args.is_empty() || s.args == "{}" || s.args == "null" {
        format!("`{}`", s.name)
    } else {
        format!("`{}` with {}", s.name, s.args)
    }
}

/// The prompt that continues `t` after a restart; `None` when it isn't continued (no message
/// recorded, or it was already a continuation).
pub fn prompt(t: &RunningTurn) -> Option<String> {
    let original = t.prompt.as_deref().filter(|p| !p.trim().is_empty())?;
    if t.resumed == Some(true) {
        return None;
    }
    let steps = steps_of(t);
    let mut note = String::from(
        "[kwnt restarted while you were working on the message below, which cut your turn off; kwnt is continuing it once, now. \
         What you did before the restart isn't in this conversation.",
    );
    let done_acting: Vec<String> = steps.iter().filter(|s| s.done && !s.read_only).map(step_line).collect();
    if !done_acting.is_empty() {
        note.push_str(&format!(
            "\n- These steps finished before the restart and may have changed something: {}. Don't do them again; check their result instead if you need it.",
            done_acting.join("; ")
        ));
    }
    for s in steps.iter().filter(|s| !s.done) {
        if s.read_only {
            note.push_str(&format!(
                "\n- The step {} was cut off mid-call. It only reads, so run it again if you still need it.",
                step_line(s)
            ));
        } else {
            note.push_str(&format!(
                "\n- The step {} was cut off mid-call, so it may or may not have taken effect. Check first whether it took effect \
                 (look for the sent message, the post, the push, the payment or the changed file). Run it again only if it clearly \
                 didn't. If you can't tell, don't run it again: ask the person once, in one short question, whether to.",
                step_line(s)
            ));
        }
    }
    note.push_str("\nThen finish the person's request.]");
    Some(format!("{note}\n\n{original}"))
}

/// The step that was cut off mid-call, for the chat's note.
pub fn cut_off_step(t: &RunningTurn) -> Option<String> {
    steps_of(t).into_iter().rev().find(|s| !s.done).map(|s| s.name)
}

/// The chat's note when the reply continues by itself.
pub fn note_text(t: &RunningTurn) -> String {
    match cut_off_step(t) {
        Some(step) => format!(
            "kwant restarted while {} was replying (in the middle of `{step}`), so {} is continuing it now and checks first whether that step already took effect.",
            t.kwant_name, t.kwant_name
        ),
        None => format!("kwant restarted while {} was replying, so {} is continuing it now.", t.kwant_name, t.kwant_name),
    }
}

/// The meta that note is saved with.
pub fn note_meta(t: &RunningTurn) -> Value {
    serde_json::json!({
        "kind": "turn_resumed_after_restart",
        "kwant_id": t.kwant_id,
        "kwant_name": t.kwant_name,
        "turn_id": t.turn_id,
        "started_at": t.started_at,
        "cut_off_step": cut_off_step(t),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn(steps: &[Step], resumed: Option<bool>) -> RunningTurn {
        RunningTurn {
            turn_id: "t1".into(),
            kwant_id: "k1".into(),
            kwant_name: "Ben".into(),
            prompt: Some("post the launch note".into()),
            steps: Some(serde_json::to_string(steps).unwrap()),
            resumed,
            ..Default::default()
        }
    }

    fn step(name: &str, done: bool) -> Step {
        Step { id: format!("id-{name}"), name: name.into(), args: r#"{"x":1}"#.into(), read_only: read_only(name), done }
    }

    #[test]
    fn reads_are_told_apart_from_steps_that_act() {
        for name in ["Read", "Grep", "WebFetch", "mcp__browser__agent_browser_snapshot", "mcp__kwnt_tools__memory_search", "mcp__x__list_issues", "mcp__browser__agent_browser_get_text", "mcp__gh__get_file"] {
            assert!(read_only(name), "{name}");
        }
        for name in ["Bash", "Write", "Edit", "mcp__browser__agent_browser_click", "mcp__slack__send_message", "mcp__gh__create_pull_request", "mcp__stripe__charge", "shell", "apply_patch", "mcp__browser__agent_browser_open", "Task"] {
            assert!(!read_only(name), "{name}");
        }
    }

    #[test]
    fn steps_are_recorded_as_they_start_and_finish() {
        let mut s = Steps::default();
        assert!(s.saw(&RuntimeEvent::ToolCallPending { id: "a".into(), name: "Bash".into(), args: json!({"command": "x".repeat(500)}) }));
        assert!(!s.saw(&RuntimeEvent::Progress));
        assert!(s.saw(&RuntimeEvent::ToolCallCompleted { id: "a".into(), result: json!({}) }));
        assert!(!s.saw(&RuntimeEvent::ToolCallCompleted { id: "zz".into(), result: json!({}) }));
        let v: Vec<Step> = serde_json::from_str(&s.json()).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].done && !v[0].read_only);
        assert!(v[0].args.chars().count() <= MAX_ARGS_CHARS + 1, "args are clipped");
        for i in 0..20 {
            s.saw(&RuntimeEvent::ToolCallPending { id: i.to_string(), name: "Read".into(), args: json!({}) });
        }
        let v: Vec<Step> = serde_json::from_str(&s.json()).unwrap();
        assert_eq!(v.len(), MAX_STEPS, "only the newest are kept");
        assert_eq!(v.last().unwrap().id, "19");
    }

    #[test]
    fn a_step_that_acts_is_checked_before_it_runs_again() {
        let p = prompt(&turn(&[step("Read", true), step("mcp__slack__send_message", false)], None)).unwrap();
        assert!(p.contains("`mcp__slack__send_message` with {\"x\":1} was cut off mid-call"), "{p}");
        assert!(p.contains("Check first whether it took effect"), "{p}");
        assert!(p.contains("If you can't tell, don't run it again: ask the person once"), "{p}");
        assert!(!p.contains("Don't do them again"), "a finished read isn't listed: {p}");
        assert!(p.ends_with("\n\npost the launch note"), "{p}");
    }

    #[test]
    fn a_read_is_simply_run_again_and_finished_actions_are_not_repeated() {
        let p = prompt(&turn(&[step("Bash", true), step("mcp__browser__agent_browser_snapshot", false)], None)).unwrap();
        assert!(p.contains("It only reads, so run it again if you still need it."), "{p}");
        assert!(!p.contains("Check first whether it took effect"), "{p}");
        assert!(p.contains("These steps finished before the restart and may have changed something: `Bash`"), "{p}");
    }

    #[test]
    fn a_continuation_is_not_continued_again_and_an_old_row_is_not_continued() {
        assert!(prompt(&turn(&[], Some(true))).is_none());
        assert!(prompt(&RunningTurn { prompt: None, ..turn(&[], None) }).is_none());
        // No steps at all: still continued, with no step lines
        let p = prompt(&turn(&[], None)).unwrap();
        assert!(!p.contains("mid-call"), "{p}");
    }

    #[test]
    fn the_chat_note_names_the_step_that_was_cut_off() {
        let t = turn(&[step("mcp__slack__send_message", false)], None);
        assert!(note_text(&t).contains("in the middle of `mcp__slack__send_message`"), "{}", note_text(&t));
        assert_eq!(note_meta(&t)["cut_off_step"], "mcp__slack__send_message");
        assert_eq!(note_text(&turn(&[], None)), "kwant restarted while Ben was replying, so Ben is continuing it now.");
    }
}
