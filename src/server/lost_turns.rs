//! The safety net for turns nobody runs any more (stuck-getting-started, 0.9.51).
//!
//! A team chat said "Quant is getting started · 1:25:45" with a Stop button while nothing ran. The
//! cause was the app's live stream losing `turn_ended` (see [`super::sse::sse_handler`]), but the
//! same picture follows from any turn the server keeps listing after its work is gone, or whose
//! engine hangs without a word. Every [`SWEEP_EVERY`] this sweep ends:
//!
//! - **orphans**: a listed turn no job holds any more (its work is gone), after [`ORPHAN_GRACE`].
//!   Ended quietly: the app clears its working status, and no line is posted, since nothing was cut.
//! - **silent turns**: a running turn with no event for longer than [`silence_limit`] while no
//!   tool runs and nothing waits for the person (an approval, a question, a human step, a held
//!   hand-off). Its job is cancelled and a plain line goes in its chat ([`silent_line`]); if it is
//!   still listed at the next sweep, it is ended by force.
//!
//! Engines that report only at the end (`live_steps == Some(false)`) are never judged by silence:
//! they are quiet by design and the turn's own watchdog doesn't cover them either.

use super::bot_status::TurnSignals;
use super::turns::ActiveTurn;
use super::AppState;
use crate::db::models::{MessageExtra, NewMessage};
use crate::drivers::events::RuntimeEvent;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// How often the sweep looks.
pub const SWEEP_EVERY: Duration = Duration::from_secs(60);
/// How long a listed turn may go without a job before it counts as an orphan (a turn is listed
/// after its job holds it and unlisted before the job goes, so this only absorbs clock jitter).
pub const ORPHAN_GRACE: Duration = Duration::from_secs(120);
/// The shortest silence the sweep acts on, whatever the stall limit says.
pub const MIN_SILENCE: Duration = Duration::from_secs(5 * 60);
/// The silence the sweep acts on when the stall watchdog is turned off (`turn_stall_secs = 0`).
pub const SILENCE_WITHOUT_WATCHDOG: Duration = Duration::from_secs(30 * 60);

/// The `meta.kind` of the line a silent turn leaves.
pub const LINE_KIND: &str = "turn_lost";

/// Why a listed turn is ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lost {
    /// No job holds it: nothing runs it.
    Orphan,
    /// Running, but nothing happened for longer than the silence limit.
    Silent,
}

/// What the sweep knows about one listed turn besides the turn itself.
#[derive(Debug, Clone, Default)]
pub struct Seen {
    /// A job still holds the turn's id.
    pub has_job: bool,
    pub signals: Option<TurnSignals>,
    /// A step waits for the person (an approval card or a human step), from the turn's activity.
    pub waiting_on_person: bool,
    /// A tool step still runs.
    pub running_tool: bool,
    /// The turn holds its hand-offs' reports (a lead waiting for its team).
    pub holding: bool,
}

/// The silence a running turn may keep before the sweep ends it: three stall limits (the turn's
/// own watchdog retries once and then stops it, so it gets its go first), at least
/// [`MIN_SILENCE`]. `None` (the watchdog is off): [`SILENCE_WITHOUT_WATCHDOG`].
pub fn silence_limit(stall: Option<Duration>) -> Duration {
    match stall {
        Some(s) => (s * 3).max(MIN_SILENCE),
        None => SILENCE_WITHOUT_WATCHDOG,
    }
}

/// Whether `turn` is lost at `now`, from what the server holds in memory. A [`Lost::Silent`] verdict
/// is checked against the database (approvals, human steps, questions) before anything is ended.
pub fn judge(turn: &ActiveTurn, seen: &Seen, now: DateTime<Utc>, silence: Duration) -> Option<Lost> {
    let age = |t: DateTime<Utc>| (now - t).to_std().unwrap_or_default();
    if !seen.has_job {
        return (age(turn.since) > ORPHAN_GRACE).then_some(Lost::Orphan);
    }
    if turn.state != "running" || seen.waiting_on_person || seen.running_tool || seen.holding {
        return None;
    }
    let signals = seen.signals.clone().unwrap_or_default();
    if signals.live_steps == Some(false) {
        return None;
    }
    let last = signals.last_step_at.map_or(turn.since, |t| t.max(turn.since));
    (age(last) > silence).then_some(Lost::Silent)
}

/// The line a silent turn leaves in its chat.
pub fn silent_line(kwant_name: &str, silence: Duration) -> String {
    let mins = (silence.as_secs() / 60).max(1);
    format!("{kwant_name} got stuck for {mins} minutes, so the work was stopped. Ask again if you still need it.")
}

/// Starts the sweep; it stops with `shutdown`.
pub fn spawn(state: AppState, shutdown: CancellationToken) {
    tokio::spawn(async move {
        let mut cancelled = HashSet::new();
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tick.tick() => {}
            }
            let silence = silence_limit(super::routes::stall_limit(&state.db).await);
            sweep(&state, &mut cancelled, Utc::now(), silence).await;
        }
    });
}

/// One look at every listed turn: ends orphans, stops silent turns (see the module docs).
/// `cancelled` carries the silent turns cancelled by an earlier sweep. Returns what it ended.
pub async fn sweep(state: &AppState, cancelled: &mut HashSet<String>, now: DateTime<Utc>, silence: Duration) -> Vec<(String, Lost)> {
    let listed = state.turns.active_turns();
    cancelled.retain(|id| listed.iter().any(|t| &t.turn_id == id));
    let mut out = Vec::new();
    for turn in &listed {
        // Cancelled a sweep ago and still listed: its work didn't wind down, so the listing goes
        if cancelled.remove(&turn.turn_id) {
            force_end(state, turn).await;
            out.push((turn.turn_id.clone(), Lost::Silent));
            continue;
        }
        let seen = seen(state, turn);
        match judge(turn, &seen, now, silence) {
            None => {}
            Some(Lost::Orphan) => {
                tracing::warn!(turn_id = %turn.turn_id, kwant_id = %turn.kwant_id, "a listed turn has no job; ending it");
                force_end(state, turn).await;
                out.push((turn.turn_id.clone(), Lost::Orphan));
            }
            Some(Lost::Silent) => {
                if waits_on_person(state, &turn.turn_id).await {
                    continue;
                }
                tracing::warn!(turn_id = %turn.turn_id, kwant_id = %turn.kwant_id, secs = silence.as_secs(), "a running turn went silent; stopping it");
                post_line(state, turn, silence).await;
                if state.turns.cancel(Some(&turn.turn_id)) == 0 {
                    force_end(state, turn).await;
                } else {
                    cancelled.insert(turn.turn_id.clone());
                }
                out.push((turn.turn_id.clone(), Lost::Silent));
            }
        }
    }
    out
}

/// Stop in a chat: ends the listed turns `matches` picks that no job holds. Stop cancels jobs, so
/// a turn without one would keep its chat "working" (or "getting started") after the person
/// pressed Stop. No grace here: the person asked for the chat to stop. Returns how many.
pub async fn end_orphans(state: &AppState, matches: impl Fn(&ActiveTurn) -> bool) -> usize {
    let mut n = 0;
    for turn in state.turns.active_turns() {
        if matches(&turn) && !state.turns.has_job(&turn.turn_id) {
            force_end(state, &turn).await;
            n += 1;
        }
    }
    n
}

fn seen(state: &AppState, turn: &ActiveTurn) -> Seen {
    let waits = super::activity::turn_waits(&turn.turn_id);
    Seen {
        has_job: state.turns.has_job(&turn.turn_id),
        signals: state.turns.board(|b| b.signals.get(&turn.turn_id).cloned()),
        waiting_on_person: waits.waiting.is_some(),
        running_tool: waits.running_tool.is_some(),
        holding: state.turns.holding(&turn.turn_id),
    }
}

/// Whether the database says the turn waits for the person: an approval card, an open human step
/// or an open question. A read that fails counts as waiting, so nothing is ended on a guess.
async fn waits_on_person(state: &AppState, turn_id: &str) -> bool {
    let db = &state.db;
    db.turn_has_waiting_approval(turn_id).await.unwrap_or(true)
        || !db.open_human_steps_for_turn(turn_id).await.is_ok_and(|s| s.is_empty())
        || db.turn_has_open_question(turn_id).await.unwrap_or(true)
}

/// Unlists the turn as its own end would: the registry, the restart record and `turn_ended`, so
/// every app clears the chat's working status.
async fn force_end(state: &AppState, turn: &ActiveTurn) {
    let _ = state.turns.end_hold(&turn.turn_id);
    state.turns.end_turn(&turn.turn_id);
    let _ = state.db.remove_running_turn(&turn.turn_id).await;
    let _ = state.event_sender.send(RuntimeEvent::TurnEnded {
        kwant_id: turn.kwant_id.clone(),
        context_id: Some(turn.context_id.clone()),
        outcome: "failed".into(),
        turn_id: Some(turn.turn_id.clone()),
        thread_id: turn.thread_id.clone(),
        quiet: true,
        reply: None,
    });
}

/// Posts [`silent_line`] where the turn's reply would have gone (its restart record says where),
/// and asks the person to resume it in Needs you when the record says the turn asks (a person's
/// own chat; hand-offs, routine runs and missions don't). The sweep posts once per turn: a turn it
/// cancelled is ended by force at the next sweep, never judged again, so one item per stop.
/// A turn without a record (it never got that far) gets no line; its working status still clears.
async fn post_line(state: &AppState, turn: &ActiveTurn, silence: Duration) {
    let Ok(Some(rt)) = state.db.running_turn(&turn.turn_id).await else {
        tracing::info!(turn_id = %turn.turn_id, "a silent turn has no record of its chat; ending it without a line");
        return;
    };
    let text = silent_line(&rt.kwant_name, silence);
    let msg = NewMessage {
        channel_id: rt.channel_id.clone(),
        direct_kwant_id: rt.direct_kwant_id.clone(),
        sender_role: "system".into(),
        sender_id: "kwnt".into(),
        text: text.clone(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let meta = json!({
        "kind": LINE_KIND,
        "kwant_id": rt.kwant_id,
        "kwant_name": rt.kwant_name,
        "turn_id": rt.turn_id,
        "silent_secs": silence.as_secs(),
    });
    match state.db.insert_message_with(&msg, &MessageExtra { thread_id: rt.thread_id.clone(), meta: Some(meta) }).await {
        Ok(saved) => {
            if let Some(tid) = &rt.thread_id {
                let _ = state.db.touch_thread(tid).await;
            }
            let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
            // …and in Needs you, for a person's own chat, as a Stop or a restart does
            // (silent-stop-asks). The line's own words: it has no Retry, the person asks again.
            if super::routes::restart_asks(state, &rt).await {
                let place = super::stopped::Place {
                    kwant_id: rt.kwant_id.clone(),
                    channel_id: rt.channel_id.clone(),
                    direct_kwant_id: rt.direct_kwant_id.clone(),
                    thread_id: rt.thread_id.clone(),
                };
                super::stopped::ask(state, &place, Some(saved.id.as_str()), &text, super::stopped::Resume::ResendMessage).await;
            }
        }
        Err(e) => tracing::warn!(error = %e, turn_id = %turn.turn_id, "could not post the line of a silent turn"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as Span;

    fn turn(state: &str, since: DateTime<Utc>) -> ActiveTurn {
        ActiveTurn {
            turn_id: "t1".into(),
            kwant_id: "quant".into(),
            context_id: "team-chat".into(),
            thread_id: None,
            state: state.into(),
            since,
            purpose: Some("work_ahead".into()),
            audit: None,
        }
    }

    fn live(last: Option<DateTime<Utc>>) -> Seen {
        Seen {
            has_job: true,
            signals: Some(TurnSignals { last_step_at: last, live_steps: Some(true), ..Default::default() }),
            ..Default::default()
        }
    }

    const SIX_MIN: Duration = Duration::from_secs(360);

    #[test]
    fn the_silence_limit_is_three_stall_limits_and_never_short() {
        assert_eq!(silence_limit(Some(Duration::from_secs(120))), SIX_MIN);
        assert_eq!(silence_limit(Some(Duration::from_secs(30))), MIN_SILENCE);
        assert_eq!(silence_limit(None), SILENCE_WITHOUT_WATCHDOG);
    }

    #[test]
    fn a_turn_no_job_holds_is_an_orphan_after_the_grace() {
        let now = Utc::now();
        let gone = Seen { has_job: false, ..Default::default() };
        assert_eq!(judge(&turn("running", now - Span::seconds(30)), &gone, now, SIX_MIN), None);
        assert_eq!(judge(&turn("running", now - Span::minutes(85)), &gone, now, SIX_MIN), Some(Lost::Orphan));
        // Queued too: nothing will ever take its slot
        assert_eq!(judge(&turn("queued", now - Span::minutes(5)), &gone, now, SIX_MIN), Some(Lost::Orphan));
    }

    #[test]
    fn a_running_turn_silent_past_the_limit_is_lost() {
        let now = Utc::now();
        let t = turn("running", now - Span::minutes(90));
        assert_eq!(judge(&t, &live(Some(now - Span::minutes(85))), now, SIX_MIN), Some(Lost::Silent));
        // Never a step: silence counts from its start
        assert_eq!(judge(&t, &live(None), now, SIX_MIN), Some(Lost::Silent));
        assert_eq!(judge(&t, &Seen { has_job: true, ..Default::default() }, now, SIX_MIN), Some(Lost::Silent));
        // A recent step keeps it
        assert_eq!(judge(&t, &live(Some(now - Span::minutes(2))), now, SIX_MIN), None);
        // A fresh turn keeps it even without a step
        assert_eq!(judge(&turn("running", now - Span::minutes(2)), &live(None), now, SIX_MIN), None);
    }

    #[test]
    fn a_turn_that_waits_on_the_person_or_a_tool_is_never_silent() {
        let now = Utc::now();
        let t = turn("running", now - Span::hours(3));
        let quiet = live(Some(now - Span::hours(3)));
        for seen in [
            Seen { waiting_on_person: true, ..quiet.clone() },
            Seen { running_tool: true, ..quiet.clone() },
            Seen { holding: true, ..quiet.clone() },
        ] {
            assert_eq!(judge(&t, &seen, now, SIX_MIN), None, "{seen:?}");
        }
        // Waiting for its slot is the line's business, not silence
        assert_eq!(judge(&turn("queued", now - Span::hours(3)), &quiet, now, SIX_MIN), None);
    }

    #[test]
    fn an_engine_that_reports_only_at_the_end_is_not_judged_by_silence() {
        let now = Utc::now();
        let seen = Seen {
            has_job: true,
            signals: Some(TurnSignals { live_steps: Some(false), ..Default::default() }),
            ..Default::default()
        };
        assert_eq!(judge(&turn("running", now - Span::hours(3)), &seen, now, SIX_MIN), None);
    }

    #[test]
    fn the_line_names_the_bot_and_the_wait_in_minutes() {
        assert_eq!(
            silent_line("Quant", SIX_MIN),
            "Quant got stuck for 6 minutes, so the work was stopped. Ask again if you still need it."
        );
    }
}
