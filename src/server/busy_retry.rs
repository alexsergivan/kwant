//! 0.9.5 no-silent-stops: a browser step that failed because the VM browser was busy is retried
//! by kwnt itself, a bounded number of times, instead of the reply ending on it.
//!
//! The browser recovery work (#544) retries a busy browser for about 45 s inside each call; when
//! it still doesn't answer, the call fails with HTTP 503 and
//! `{"code": "BROWSER_BUSY", "retryable": true, "is_running": true}`. That is temporary, not a
//! give-up: when a reply ends with its last browser step failed that way, kwnt parks the turn and
//! runs it again after a cooldown, told to retry that same step ([`MAX_RETRIES`] times per
//! message, [`cooldown`] apart). After that the person is asked once (a stopped item).
//!
//! Matched on the `code` only, never on the wording, and only from what kwnt's own browser
//! handlers answered (`continuity::call_browser_tool`), never from the bot's text or tool output
//! it could echo. A step that may reach outside (a click on "Send", "Pay"…, see
//! [`super::restart_resume::outward`]) is never run again by itself: the person is asked.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The `code` of a browser call that failed because the browser is there but overloaded.
// TODO(#544): use `crate::sandbox::service::BROWSER_BUSY_CODE` once #544 is on main.
pub const BROWSER_BUSY_CODE: &str = "BROWSER_BUSY";

/// How many times kwnt runs a reply again by itself after it ended on a busy browser step.
pub const MAX_RETRIES: u8 = 2;

/// Parked-turn reasons, one per retry, so the count survives a restart and a new message from
/// the person starts again from zero.
pub const REASON_1: &str = "browser_busy_1";
pub const REASON_2: &str = "browser_busy_2";

/// The header the retry's prompt starts with (stripped again if that retry ends busy too).
const PROMPT_HEAD: &str = "[kwnt: the browser was busy";

/// Longest step arguments kept, in characters (as the restart record does).
const MAX_ARGS_CHARS: usize = 200;

/// Whether a browser handler's answer means "busy, retry the same step": its `code` is
/// [`BROWSER_BUSY_CODE`] (and it doesn't say `retryable: false`). The text is never looked at.
pub fn is_busy(body: &Value) -> bool {
    body.get("code").and_then(Value::as_str) == Some(BROWSER_BUSY_CODE) && body.get("retryable") != Some(&Value::Bool(false))
}

/// The browser step that last failed busy in a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub tool: String,
    pub args: String,
}

fn busy_steps() -> &'static Mutex<HashMap<String, Step>> {
    static STEPS: OnceLock<Mutex<HashMap<String, Step>>> = OnceLock::new();
    STEPS.get_or_init(Default::default)
}

/// Records how a turn's browser step ended: busy keeps it as the turn's last busy step, anything
/// else (the browser answered) clears it, so only a turn that *ended* on a busy step is retried.
pub fn note(turn_id: &str, tool: &str, args: &Value, busy: bool) {
    if turn_id.is_empty() {
        return;
    }
    let mut steps = busy_steps().lock().unwrap_or_else(|e| e.into_inner());
    if busy {
        let args: String = args.to_string().chars().take(MAX_ARGS_CHARS).collect();
        steps.insert(turn_id.to_string(), Step { tool: tool.to_string(), args });
    } else {
        steps.remove(turn_id);
    }
}

/// The turn's last browser step, if it failed busy and nothing answered after it. Forgets it.
pub fn take(turn_id: &str) -> Option<Step> {
    busy_steps().lock().unwrap_or_else(|e| e.into_inner()).remove(turn_id)
}

/// How long the `attempt`th retry (1-based) waits: 1 min, then 2 min.
pub fn cooldown(attempt: u8) -> Duration {
    Duration::from_secs(60 * u64::from(attempt.max(1)))
}

/// The parked reason for the `attempt`th retry.
pub fn reason(attempt: u8) -> &'static str {
    if attempt <= 1 { REASON_1 } else { REASON_2 }
}

/// How many busy retries a parked turn's reason stands for, if it is one.
pub fn retries_of(reason: &str) -> Option<u8> {
    match reason {
        REASON_1 => Some(1),
        REASON_2 => Some(2),
        _ => None,
    }
}

/// What happens to a reply that ended on a busy browser step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Run it again by itself: the `attempt`th retry, after `after`.
    Retry { attempt: u8, after: Duration },
    /// The retries are used up: ask the person once.
    Exhausted,
    /// The step may reach outside: never run again by itself, ask the person.
    Outward,
}

/// The rule: retried by itself up to [`MAX_RETRIES`] times (`done` already happened for this
/// message), except a step that may reach outside.
pub fn decide(step: &Step, done: u8) -> Exit {
    if super::restart_resume::outward(&step.tool, &step.args) {
        return Exit::Outward;
    }
    if done >= MAX_RETRIES {
        return Exit::Exhausted;
    }
    let attempt = done + 1;
    Exit::Retry { attempt, after: cooldown(attempt) }
}

fn step_line(step: &Step) -> String {
    if step.args.is_empty() || step.args == "{}" || step.args == "null" {
        format!("`{}`", step.tool)
    } else {
        format!("`{}` with {}", step.tool, step.args)
    }
}

/// The retry's prompt: the same step again, then the original request. A request that is
/// already a busy retry keeps only its original part.
pub fn prompt(step: &Step, original: &str) -> String {
    let original = match original.strip_prefix(PROMPT_HEAD) {
        Some(rest) => rest.split_once("]\n\n").map_or(original, |(_, o)| o),
        None => original,
    };
    format!(
        "{PROMPT_HEAD} (overloaded, not gone) when you ran {step}, so that step didn't happen. \
         Run that same step again now, then carry on with the request below. If the browser is \
         still busy, say so in one line]\n\n{original}",
        step = step_line(step),
    )
}

/// The chat note when a retry is set.
pub fn retry_note(name: &str, step: &Step, attempt: u8, at: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "The browser was busy when {name} ran `{tool}` (it is overloaded, not gone), so {name} tries that same step again at {at} UTC (retry {attempt} of {MAX_RETRIES}).",
        tool = step.tool,
        at = at.format("%H:%M"),
    )
}

/// The one ask (chat note and Needs-you line) when kwnt doesn't retry by itself.
pub fn ask_line(name: &str, step: &Step, exit: &Exit) -> String {
    match exit {
        Exit::Outward => format!(
            "{name}'s browser step `{tool}` failed because the browser was busy, and it may reach outside (a send, a post, a payment), \
             so kwant won't run it again by itself. Check whether it went through, then tell {name} in the chat whether to do it again.",
            tool = step.tool,
        ),
        _ => format!(
            "{name} couldn't finish: the browser stayed busy after {MAX_RETRIES} automatic retries of `{tool}`. Retry in the chat sends your message again.",
            tool = step.tool,
        ),
    }
}

/// Called when a reply in a person's own chat ended with its last browser step failed busy:
/// parks a retry of it, or asks the person once (see [`decide`]). `done` is how many busy
/// retries this message already had.
pub(super) async fn after_turn(
    scope: &super::routes::TurnScope,
    kwant: &crate::db::models::Kwant,
    original: &str,
    step: &Step,
    done: u8,
) {
    let state = &scope.state;
    let exit = decide(step, done);
    if let Exit::Retry { attempt, after } = &exit {
        let at = chrono::Utc::now() + chrono::Duration::from_std(*after).unwrap_or_else(|_| chrono::Duration::seconds(60));
        let parked = state
            .db
            .park_turn(&crate::db::parked::NewParkedTurn {
                kwant_id: kwant.id.clone(),
                channel_id: scope.channel_id.clone(),
                direct_kwant_id: scope.kwant_id.clone(),
                thread_id: scope.thread_id.clone(),
                delegation_id: None,
                depth: scope.delegation_depth,
                prompt: prompt(step, original),
                retry_at: at,
                reason: reason(*attempt),
            })
            .await;
        match parked {
            Ok(Some(_)) => {
                let note = scope.message("system", "kwnt", retry_note(&kwant.name, step, *attempt, at));
                scope.save(&note, Some(serde_json::json!({"kind": "browser_busy_retry", "attempt": attempt}))).await;
                tracing::info!(kwant = %kwant.id, attempt, tool = %step.tool, "the browser was busy: retrying the step by itself");
                state.scheduler_wake.notify_one();
            }
            // Another retry already waits in this chat (a usage limit): that one runs it again
            Ok(None) => {}
            Err(e) => tracing::warn!(kwant = %kwant.id, "couldn't park a busy-browser retry: {e}"),
        }
        return;
    }
    let line = ask_line(&kwant.name, step, &exit);
    let note = scope.message("system", "kwnt", line.clone());
    let saved = scope.save(&note, Some(serde_json::json!({"kind": "browser_busy_stopped", "outward": exit == Exit::Outward}))).await;
    let place = super::stopped::Place::of_turn(scope, &kwant.id);
    super::stopped::ask(state, &place, saved.as_ref().map(|m| m.id.as_str()), &line, super::stopped::Resume::ResendMessage).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn act(args: Value) -> Step {
        Step { tool: "browser_act".into(), args: args.to_string() }
    }

    #[test]
    fn busy_is_matched_on_the_code_never_the_text() {
        // #544's 503 body
        let busy = json!({"error": "The browser isn't responding right now (it is overloaded, not gone). This is temporary: retry the same step in a minute.",
            "code": "BROWSER_BUSY", "retryable": true, "is_running": true});
        assert!(is_busy(&busy));
        assert!(is_busy(&json!({"error": "anything else entirely", "code": "BROWSER_BUSY", "retryable": true})), "any wording");
        // The same wording without the code is not busy (a bot or a page could echo it)
        assert!(!is_busy(&json!({"error": busy["error"], "retryable": true})));
        assert!(!is_busy(&json!(busy["error"])));
        // Truly unreachable (Docker), pages full, or an explicit opt-out
        assert!(!is_busy(&json!({"error": "Sandbox browser is unavailable. Check that Docker is running, then restart the browser.", "is_running": false})));
        assert!(!is_busy(&json!({"error": "pages full", "code": "PAGES_FULL"})));
        assert!(!is_busy(&json!({"code": "BROWSER_BUSY", "retryable": false})));
        assert!(!is_busy(&json!({"code": "browser_busy"})), "the code is exact");
    }

    #[test]
    fn the_code_matches_the_browser_recovery_constant() {
        // TODO(#544): assert_eq!(BROWSER_BUSY_CODE, crate::sandbox::service::BROWSER_BUSY_CODE)
        assert_eq!(BROWSER_BUSY_CODE, "BROWSER_BUSY");
    }

    #[test]
    fn a_busy_step_is_retried_by_itself_twice_then_the_person_is_asked() {
        let step = act(json!({"action": "scroll", "delta_y": 400}));
        assert_eq!(decide(&step, 0), Exit::Retry { attempt: 1, after: Duration::from_secs(60) });
        assert_eq!(decide(&step, 1), Exit::Retry { attempt: 2, after: Duration::from_secs(120) });
        assert_eq!(decide(&step, 2), Exit::Exhausted);
        assert_eq!(decide(&step, 9), Exit::Exhausted);
        // the count rides on the parked reason, so it survives a restart
        assert_eq!(retries_of(reason(1)), Some(1));
        assert_eq!(retries_of(reason(2)), Some(2));
        assert_eq!(retries_of("usage_limit"), None);
        assert_eq!(retries_of("restart"), None);
        let reads = Step { tool: "browser_page".into(), args: "{}".into() };
        assert!(matches!(decide(&reads, 0), Exit::Retry { .. }));
    }

    #[test]
    fn a_busy_step_that_may_reach_outside_is_never_run_again_by_itself() {
        // a click whose arguments name a "Send" button, or a tool named for a payment
        assert_eq!(decide(&act(json!({"action": "type", "ref": "e4", "text": "Send"})), 0), Exit::Outward);
        let pay = Step { tool: "pay_invoice".into(), args: "{}".into() };
        assert_eq!(decide(&pay, 0), Exit::Outward);
        // ...even with retries left, and before the cap
        assert_eq!(decide(&pay, 1), Exit::Outward);
        assert!(ask_line("Ben", &pay, &Exit::Outward).contains("won't run it again by itself"));
        assert!(ask_line("Ben", &pay, &Exit::Exhausted).contains("Retry in the chat"));
    }

    #[test]
    fn only_a_turn_that_ended_on_a_busy_step_is_retried() {
        let turn = format!("t-{}", uuid::Uuid::new_v4());
        note(&turn, "browser_open", &json!({"url": "https://example.com"}), true);
        // the browser answered a later step: nothing to retry
        note(&turn, "browser_page", &json!({}), false);
        assert_eq!(take(&turn), None);
        note(&turn, "browser_page", &json!({}), false);
        note(&turn, "browser_act", &json!({"action": "click", "ref": "e9"}), true);
        let step = take(&turn).expect("ended busy");
        assert_eq!(step.tool, "browser_act");
        assert!(step.args.contains("e9"));
        assert_eq!(take(&turn), None, "taken once");
        // a turn without an id records nothing
        note("", "browser_act", &json!({}), true);
        assert_eq!(take(""), None);
        // long arguments are clipped
        note(&turn, "browser_act", &json!({"text": "x".repeat(1000)}), true);
        assert_eq!(take(&turn).unwrap().args.chars().count(), MAX_ARGS_CHARS);
    }

    #[test]
    fn the_retry_prompt_names_the_step_and_keeps_one_copy_of_the_request() {
        let step = act(json!({"action": "click", "ref": "e9"}));
        let first = prompt(&step, "Find the price on example.com");
        assert!(first.starts_with(PROMPT_HEAD));
        assert!(first.contains("`browser_act` with {\"action\":\"click\",\"ref\":\"e9\"}"));
        assert!(first.ends_with("\n\nFind the price on example.com"));
        let second = prompt(&Step { tool: "browser_open".into(), args: "{}".into() }, &first);
        assert_eq!(second.matches(PROMPT_HEAD).count(), 1);
        assert!(second.contains("`browser_open`"));
        assert!(second.ends_with("\n\nFind the price on example.com"));
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-12T22:30:00Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(
            retry_note("Ben", &step, 1, at),
            "The browser was busy when Ben ran `browser_act` (it is overloaded, not gone), so Ben tries that same step again at 22:30 UTC (retry 1 of 2)."
        );
    }
}
