//! 0.9.5 "Runs never stop silently": when a bot's work in a person's chat stops without finishing
//! and nothing will pick it up by itself, the person gets exactly one ask. It goes in the chat (the
//! notice the stop already posts, which carries Retry or Continue) and in Needs you, as a
//! `waiting_on_person` item of kind `stopped` whose line says what stopped, why, and the one action
//! that resumes it. The item closes when the person writes in that chat (Retry resends their
//! message), when a later reply of that bot there finishes, when its Continue or Retry runs, or
//! when the stop resolves itself.
//!
//! Stops that already resume by themselves get no item: a turn parked behind a usage limit (it
//! retries at the reset), a stall's one automatic retry, the delivery check's one continuation.
//! Hand-offs, routine runs and mission tasks have their own Inbox and board entries, so their
//! turns add none here (see [`for_turn`]).

use super::AppState;
use crate::db::waiting::WaitingItem;
use serde_json::{json, Value};

/// Longest line a stopped item keeps (it is shown on one row).
pub const MAX_LINE_CHARS: usize = 200;

/// The chat a stop happened in.
#[derive(Debug, Clone, Default)]
pub struct Place {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
}

impl Place {
    /// `kwant`'s place in a turn's chat; a thread keeps only its thread id (as messages do).
    pub(super) fn of_turn(scope: &super::routes::TurnScope, kwant_id: &str) -> Self {
        Self {
            kwant_id: kwant_id.to_string(),
            channel_id: scope.channel_id.clone(),
            direct_kwant_id: scope.kwant_id.clone(),
            thread_id: scope.thread_id.clone(),
        }
    }
}

/// What resumes a stop when it isn't the chat's own Retry.
#[derive(Debug, Clone, PartialEq)]
pub enum Resume {
    /// Retry resends the person's last message in that chat (the notice's Retry button).
    ResendMessage,
    /// The notice's Continue button (a chain paused at its budget or hand-off cap).
    Continue,
    /// `POST /api/approvals/:id/retry` asks again for an approval that expired unanswered.
    Approval { approval_id: String },
}

impl Resume {
    fn json(&self) -> Option<String> {
        match self {
            Resume::ResendMessage => None,
            Resume::Continue => Some(json!({"type": "continue"}).to_string()),
            Resume::Approval { approval_id } => Some(json!({"type": "approval", "approval_id": approval_id}).to_string()),
        }
    }
}

/// One short line: the text with a leading "⚠️ [Bot] " cut, its first non-empty line, clipped.
pub fn short_line(text: &str) -> String {
    let t = text.trim_start();
    let t = t.strip_prefix("⚠️").map(str::trim_start).unwrap_or(t);
    let t = match t.strip_prefix('[').and_then(|r| r.split_once("] ")) {
        Some((name, rest)) if !name.contains('\n') && name.chars().count() <= 80 => rest,
        _ => t,
    };
    let first = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let clipped: String = first.chars().take(MAX_LINE_CHARS).collect();
    if clipped.chars().count() < first.chars().count() {
        format!("{}…", clipped.trim_end())
    } else {
        clipped
    }
}

/// The Needs-you line for a reply that stopped: what stopped (in the bot's notice's own words, so
/// the why is there) and what resumes it.
pub fn reply_line(notice: &str) -> String {
    let what = short_line(notice);
    // The notice's own "send it again" advice is replaced by the one action below, not repeated
    let what = what.trim_end().strip_suffix(OWN_RETRY_ADVICE).unwrap_or(what.trim_end()).trim_end();
    let what = what.trim_end_matches('.');
    if what.is_empty() {
        "The reply stopped before it finished. Retry in the chat sends your message again.".into()
    } else {
        format!("{what}. Retry in the chat sends your message again.")
    }
}

/// The advice a stall notice ends with (`turns::stall_stop_line`), which the item's action replaces.
const OWN_RETRY_ADVICE: &str = "Send your message again to retry.";

/// An item's one-row title, at most `max` characters: when it's too long, what stopped is clipped
/// and its last sentence (the action that resumes it) is kept whole.
pub fn title(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    if let Some(i) = line.trim_end_matches('.').rfind(". ") {
        let action = &line[i + 2..];
        let room = max.saturating_sub(action.chars().count() + 2);
        if room >= 24 {
            let what: String = line[..i].chars().take(room).collect();
            return format!("{}… {action}", what.trim_end());
        }
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Records the ask (clients refresh Needs you on the notice's message, posted with it).
pub async fn ask(state: &AppState, place: &Place, message_id: Option<&str>, line: &str, resume: Resume) -> Option<WaitingItem> {
    let line: String = line.chars().take(MAX_LINE_CHARS + 120).collect();
    let item = state
        .db
        .add_stopped(
            &place.kwant_id,
            place.channel_id.as_deref(),
            place.direct_kwant_id.as_deref(),
            place.thread_id.as_deref(),
            message_id,
            &line,
            resume.json().as_deref(),
        )
        .await
        .ok()?;
    tracing::info!(waiting_id = %item.id, kwant_id = %place.kwant_id, "a bot's work stopped: asked the person to resume it");
    Some(item)
}

/// A reply of the bot in that chat finished: whatever stopped there before has moved on.
pub async fn recovered(state: &AppState, place: &Place) {
    let _ = state
        .db
        .close_stopped(&place.kwant_id, place.channel_id.as_deref(), place.direct_kwant_id.as_deref(), place.thread_id.as_deref())
        .await;
}

/// Whether a turn that stopped gets a Needs-you item of its own: only a person's chat (depth 0)
/// whose stop notice was saved. A hand-off, a routine run, a mission task, a discussion round, a
/// replay or an unsaved view is reported where it belongs (Inbox, board, its caller).
pub(super) fn for_turn(depth: i64, quiet_failure: bool, unsaved: bool, discussion: bool, rehearse: bool, routine: bool) -> bool {
    depth == 0 && !quiet_failure && !unsaved && !discussion && !rehearse && !routine
}

/// The meta a gave-up reply is saved with when it has none of its own: marks it as a failure the
/// chat offers Retry on. `None` for a turn that resumes by itself (parked behind a limit).
pub fn gave_up_meta(meta: Option<Value>, resumes_itself: bool) -> Option<Value> {
    match (meta, resumes_itself) {
        (Some(m), _) => Some(m),
        (None, true) => None,
        (None, false) => Some(json!({"kind": "turn_failed", "gave_up": true})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_short_and_drop_the_bot_tag() {
        assert_eq!(short_line("⚠️ [Ben] Couldn't finish this reply: boom\nmore"), "Couldn't finish this reply: boom");
        assert_eq!(short_line("\n  plain line  \n"), "plain line");
        let long = short_line(&format!("⚠️ [B] {}", "x".repeat(400)));
        assert!(long.ends_with('…') && long.chars().count() == MAX_LINE_CHARS + 1, "{long}");
        // A bracket that isn't a bot tag stays
        assert_eq!(short_line("[draft without close"), "[draft without close");
    }

    #[test]
    fn reply_lines_say_what_resumes_it() {
        assert_eq!(
            reply_line("⚠️ [Ben] The model sent nothing for 300 s, so this reply was stopped."),
            "The model sent nothing for 300 s, so this reply was stopped. Retry in the chat sends your message again."
        );
        assert!(reply_line("").starts_with("The reply stopped before it finished."));
        // A stall notice's own advice isn't repeated next to the action
        assert_eq!(
            reply_line(&super::super::turns::stall_stop_line("Ben", 2, true)),
            "The model sent nothing for 2 s, twice (it was retried once), so this reply was stopped. Retry in the chat sends your message again."
        );
    }

    #[test]
    fn a_long_title_keeps_the_action_that_resumes_it() {
        let action = "Retry in the chat sends your message again.";
        let long = format!("Couldn't finish this reply: {}. {action}", "the driver failed with a long error ".repeat(5));
        let t = title(&long, 120);
        assert!(t.chars().count() <= 120 && t.ends_with(&format!("… {action}")) && t.starts_with("Couldn't finish this reply:"), "{t}");
        assert_eq!(title("  Short.   Retry here. ", 120), "Short. Retry here.");
        // No sentence to keep: plain clip
        let one = "x".repeat(300);
        assert_eq!(title(&one, 120).chars().count(), 120);
        assert!(title(&one, 120).ends_with('…'));
    }

    #[test]
    fn only_a_persons_own_chat_gets_an_item() {
        assert!(for_turn(0, false, false, false, false, false));
        assert!(!for_turn(1, false, false, false, false, false), "hand-offs are in the Inbox");
        assert!(!for_turn(0, true, false, false, false, false), "quiet failures: the caller reports");
        assert!(!for_turn(0, false, true, false, false, false));
        assert!(!for_turn(0, false, false, true, false, false));
        assert!(!for_turn(0, false, false, false, true, false));
        assert!(!for_turn(0, false, false, false, false, true), "routine runs are in the Inbox");
    }

    #[test]
    fn a_parked_turn_gets_no_retry_meta() {
        assert_eq!(gave_up_meta(None, true), None);
        assert_eq!(gave_up_meta(None, false), Some(json!({"kind": "turn_failed", "gave_up": true})));
        let stall = json!({"kind": "turn_stalled"});
        assert_eq!(gave_up_meta(Some(stall.clone()), false), Some(stall));
    }

    #[test]
    fn resume_kinds_serialize() {
        assert_eq!(Resume::ResendMessage.json(), None);
        let a = Resume::Approval { approval_id: "ap1".into() }.json().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&a).unwrap(), json!({"type": "approval", "approval_id": "ap1"}));
    }
}
