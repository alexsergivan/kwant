//! Work ahead while the person is away: when a chat with recent work has been quiet for
//! [`QUIET_HOURS`], its lead gets one hidden turn to do the reversible next steps (finish the
//! checks it left open, write drafts, research, prepare the next deliverable, schedule a
//! follow-up). It never sends, pays, contacts anyone, deletes or signs in; those are drafted and
//! offered. A reply of `[NOTHING]` posts nothing. Otherwise one message, "While you were away I …",
//! lands in the chat with `meta.kind = "work_ahead"` and a `work_ahead` notification.
//!
//! Caps: `work_ahead_daily_cap` runs per space per day (default 3), one run per chat per day,
//! and none again in a chat until the person writes there. A run that uses more than
//! [`MAX_RUN_TOKENS`] fresh tokens uses up the space's runs for the day. The scheduler looks
//! every [`SCAN_EVERY`]; `POST /api/work-ahead/run` (person-only) looks now.

use super::continuity::{notify, Notification};
use super::decide::{work_ahead_cap, work_ahead_enabled};
use super::routes::{scope_for, turn_once, TurnOpts};
use super::AppState;
use crate::db::models::{Kwant, Message};
use crate::db::spaces::SpaceId;
use crate::db::work_ahead::WorkAheadRun;
use crate::drivers::events::RuntimeEvent;
use axum::{extract::State, response::IntoResponse, response::Response, Json};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};

/// Hours a chat must be quiet before its lead works ahead.
pub const QUIET_HOURS: i64 = 2;
/// A chat whose last message is older than this isn't worked on: the work went cold.
pub const STALE_HOURS: i64 = 48;
/// Fresh tokens one run may use; a run over it ends the space's runs for the day.
pub const MAX_RUN_TOKENS: u64 = 60_000;
/// The `purpose` of the run's turn in `GET /api/turns` (`turns[]`).
pub const PURPOSE: &str = "work_ahead";
/// What a run replies when nothing is worth doing.
pub const NOTHING: &str = "[NOTHING]";
/// How often the scheduler looks for quiet chats, in seconds.
pub const SCAN_EVERY: i64 = 10 * 60;
/// Chats looked at per scan.
const CHATS_SCANNED: i64 = 50;
/// Messages of a chat looked at.
const RECENT_MESSAGES: i64 = 20;
/// Longest message a run posts.
const MAX_MESSAGE_CHARS: usize = 3000;

static LAST_SCAN: AtomicI64 = AtomicI64::new(0);
static SCANNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A quiet chat and who works ahead in it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub context_id: String,
    pub channel_id: Option<String>,
    pub thread_id: Option<String>,
    pub lead: Kwant,
    pub space: SpaceId,
    pub quiet_hours: i64,
}

/// `YYYY-MM-DD` of `now`.
pub fn day_of(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// Whether a chat's recent messages (oldest first) make it worth working ahead in at `now`: the
/// person wrote there, a bot answered, the last message is between [`QUIET_HOURS`] and
/// [`STALE_HOURS`] old, and nothing since the person's last message was a work-ahead message.
pub fn quiet_enough(messages: &[Message], now: DateTime<Utc>) -> Option<i64> {
    let last = messages.last()?;
    let at = last.created_at?;
    let quiet = now - at;
    if quiet < Duration::hours(QUIET_HOURS) || quiet > Duration::hours(STALE_HOURS) {
        return None;
    }
    let person = messages.iter().rposition(|m| m.sender_role == "user")?;
    let after = &messages[person + 1..];
    if !after.iter().any(|m| m.sender_role == "kwant") || after.iter().any(is_work_ahead) {
        return None;
    }
    Some(quiet.num_hours())
}

fn is_work_ahead(m: &Message) -> bool {
    m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).is_some_and(|v| v["kind"] == "work_ahead")
}

/// The chat's lead: a channel's lead, a thread's or direct chat's bot.
async fn lead_of(state: &AppState, thread: Option<&str>, channel: Option<&str>, direct: Option<&str>) -> Option<(Kwant, Option<String>, Option<String>)> {
    if let Some(t) = thread {
        let thread = state.db.get_thread(t).await.ok().flatten()?;
        // Delegation threads are bots' work for each other, not the person's chats
        if thread.kind != "chat" || thread.archived {
            return None;
        }
        let k = state.db.get_kwant(&thread.kwant_id).await.ok().flatten()?;
        return Some((k, None, Some(t.to_string())));
    }
    if let Some(c) = channel {
        let channel = state.db.get_channel(c).await.ok().flatten()?;
        let k = state.db.get_kwant(&channel.lead_kwant_id).await.ok().flatten()?;
        return Some((k, Some(c.to_string()), None));
    }
    let k = state.db.get_kwant(direct?).await.ok().flatten()?;
    Some((k, None, None))
}

/// Quiet chats at `now` that may get a run, newest first, within each space's daily cap.
pub async fn candidates(state: &AppState, now: DateTime<Utc>) -> Vec<Candidate> {
    let day = day_of(now);
    let cap = work_ahead_cap(state).await as i64;
    let mut used: std::collections::HashMap<String, i64> = Default::default();
    let mut out = Vec::new();
    for (thread, channel, direct) in state.db.recent_chats(CHATS_SCANNED).await.unwrap_or_default() {
        let Some(context_id) = thread.clone().or_else(|| channel.clone()).or_else(|| direct.clone()) else { continue };
        if state.turns.has_job(&context_id) {
            continue;
        }
        let messages = state.db.list_messages(&context_id, RECENT_MESSAGES).await.unwrap_or_default();
        let Some(quiet_hours) = quiet_enough(&messages, now) else { continue };
        if state.db.last_work_ahead_run(&context_id).await.ok().flatten().is_some_and(|r| r.day == day) {
            continue;
        }
        let Some((lead, channel_id, thread_id)) = lead_of(state, thread.as_deref(), channel.as_deref(), direct.as_deref()).await else {
            continue;
        };
        let space = SpaceId::new(&lead.space_id);
        if state.db.space_archived(&space).await.unwrap_or(true) {
            continue;
        }
        let n = match used.get(space.as_str()) {
            Some(n) => *n,
            None => match state.db.work_ahead_runs_on(&space, &day).await {
                // A run over the token cap uses up the day
                Ok((_, most)) if most > MAX_RUN_TOKENS as i64 => i64::MAX,
                Ok((n, _)) => n,
                Err(_) => i64::MAX,
            },
        };
        if n >= cap {
            continue;
        }
        used.insert(space.as_str().to_string(), n.saturating_add(1));
        out.push(Candidate { context_id, channel_id, thread_id, lead, space, quiet_hours });
    }
    out
}

/// The run's prompt.
pub fn prompt(quiet_hours: i64) -> String {
    format!(
        "[Work ahead] The person hasn't been in this chat for about {quiet_hours} h. Nobody asked for this turn; the person \
         sees only your final reply, and only when you did something useful.\n\
         Look at the open items in this chat: checks you said weren't done, drafts not written, next steps you promised, \
         what you wait on. Do the ones that can be undone, now: run the checks, write the drafts, research, prepare the \
         next deliverable, schedule a follow-up with `follow_up_create`. Decide reversible choices yourself, using \
         [How the person usually decides] if you have it.\n\
         Never send, publish, post, pay, buy, delete, sign in or contact anyone for this: draft it and offer it. Don't \
         @mention teammates and don't ask the person anything.\n\
         If nothing is worth doing, reply exactly {NOTHING} and nothing else. Otherwise reply in at most 8 short lines: \
         start with \"While you were away I\", say what you did and where it is, and end with one line \"Next I expect \
         you'll want …\" that predicts the person's next ask from this chat."
    )
}

/// The message a run posts, or `None` for [`NOTHING`] or an empty reply.
pub fn message_text(reply: &str) -> Option<String> {
    let t = reply.trim();
    if t.is_empty() || (t.contains(NOTHING) && t.len() <= NOTHING.len() + 40) {
        return None;
    }
    let t = t.replace(NOTHING, "");
    let t = t.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_MESSAGE_CHARS).collect())
}

/// Runs one work-ahead turn for `c` under its started `row` and returns the posted message, if any.
pub async fn run(state: &AppState, c: Candidate, row: WorkAheadRun) -> Option<Message> {
    let day = row.day.clone();
    let job = state.turns.start_job([c.context_id.clone()]);
    let scope = match scope_for(state, &c.lead, c.channel_id.clone(), c.thread_id.clone(), &job, 0).await {
        Ok(s) => s,
        Err(_) => {
            let _ = state.db.finish_work_ahead_run(&row.id, "failed", None, 0).await;
            return None;
        }
    };
    let mut opts = TurnOpts::default();
    (opts.unsaved, opts.quiet_failure, opts.purpose) = (true, true, Some(PURPOSE.into()));
    let reply = turn_once(&scope, &c.lead, prompt(c.quiet_hours), opts).await;
    let tokens = state.turns.job_tokens(&job.id);
    if tokens > MAX_RUN_TOKENS {
        tracing::warn!(space = %c.space.as_str(), tokens, "a work-ahead run went over its token cap; no more runs in this space today");
    }
    let Ok(reply) = reply else {
        let _ = state.db.finish_work_ahead_run(&row.id, "failed", None, tokens).await;
        return None;
    };
    // The post's own choices come off its text into meta.decisions, as in any other reply
    let (reply, decisions) = super::decide::take_decisions(&reply);
    let Some(text) = message_text(&reply) else {
        let _ = state.db.finish_work_ahead_run(&row.id, "nothing", None, tokens).await;
        return None;
    };
    let meta = json!({"kind": "work_ahead", "date": day, "run_id": row.id});
    let meta = super::decide::with_decisions(Some(meta), &decisions);
    let saved = scope.save(&scope.message("kwant", &c.lead.name, text), meta).await;
    if let (Some(m), false) = (&saved, decisions.is_empty()) {
        super::decide::note_bot_decisions(state, &c.lead, m).await;
    }
    let _ = state.db.finish_work_ahead_run(&row.id, if saved.is_some() { "posted" } else { "failed" }, saved.as_ref().map(|m| m.id.as_str()), tokens).await;
    let saved = saved?;
    let place = (scope.channel_id.clone(), scope.kwant_id.clone(), scope.thread_id.clone());
    super::missions::listener::on_work_ahead_posted(state, &c.lead, place, &row.id, &saved.text).await;
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    notify(
        state,
        Notification {
            kind: "work_ahead",
            kwant_id: c.lead.id.clone(),
            kwant_name: c.lead.name.clone(),
            channel_id: c.channel_id.clone(),
            title: format!("{} worked ahead while you were away", c.lead.name),
            body: "Open the chat to see what's ready.".into(),
        },
    )
    .await;
    tracing::info!(run_id = %row.id, kwant_id = %c.lead.id, "posted a work-ahead message");
    Some(saved)
}

/// Starts a run in every candidate chat at `now` (in the background) and returns them. With
/// `force` the scan doesn't wait for [`SCAN_EVERY`] since the last one.
pub async fn scan(state: &AppState, now: DateTime<Utc>, force: bool) -> Vec<Candidate> {
    if !work_ahead_enabled(state).await {
        return Vec::new();
    }
    let _guard = SCANNING.lock().await;
    let last = LAST_SCAN.load(Ordering::Relaxed);
    if !force && now.timestamp() - last < SCAN_EVERY {
        return Vec::new();
    }
    LAST_SCAN.store(now.timestamp(), Ordering::Relaxed);
    let found = candidates(state, now).await;
    let mut started = Vec::new();
    for c in found {
        // The row is there before the run starts, so the next scan doesn't pick the chat again
        let Ok(row) = state.db.start_work_ahead_run(&c.space, &c.context_id, &c.lead.id, &day_of(now)).await else { continue };
        started.push(c.clone());
        let state = state.clone();
        tokio::spawn(async move {
            run(&state, c, row).await;
        });
    }
    started
}

/// `POST /api/work-ahead/run` (person-only): looks for quiet chats now instead of at the next
/// scan, still within every cap. Returns the chats where a run started.
pub async fn run_now(State(state): State<AppState>) -> Response {
    let started = scan(&state, Utc::now(), true).await;
    let rows: Vec<Value> = started
        .iter()
        .map(|c| json!({"context_id": c.context_id, "kwant_id": c.lead.id, "quiet_hours": c.quiet_hours}))
        .collect();
    Json(json!({"enabled": work_ahead_enabled(&state).await, "started": rows})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, hours_ago: i64, meta: Option<&str>) -> Message {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            channel_id: Some("c".into()),
            direct_kwant_id: None,
            sender_role: role.into(),
            sender_id: "x".into(),
            text: "t".into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
            created_at: Some(Utc::now() - Duration::hours(hours_ago)),
            thread_id: None,
            meta: meta.map(str::to_string),
        }
    }

    #[test]
    fn only_a_quiet_chat_with_a_bot_reply_after_the_person_qualifies() {
        // A minute ahead, so "6 h ago" counts as 6 whole hours
        let now = Utc::now() + Duration::minutes(1);
        assert_eq!(quiet_enough(&[msg("user", 7, None), msg("kwant", 6, None)], now), Some(6));
        assert_eq!(quiet_enough(&[msg("user", 2, None), msg("kwant", 1, None)], now), None, "not quiet yet");
        assert_eq!(quiet_enough(&[msg("user", 80, None), msg("kwant", 72, None)], now), None, "gone cold");
        assert_eq!(quiet_enough(&[msg("user", 7, None)], now), None, "no bot answered");
        assert_eq!(quiet_enough(&[msg("kwant", 7, None)], now), None, "the person never wrote");
        let wa = r#"{"kind":"work_ahead"}"#;
        assert_eq!(quiet_enough(&[msg("user", 30, None), msg("kwant", 29, None), msg("kwant", 5, Some(wa))], now), None, "once per person message");
        assert_eq!(quiet_enough(&[msg("kwant", 30, Some(wa)), msg("user", 7, None), msg("kwant", 6, None)], now), Some(6));
    }

    #[test]
    fn nothing_posts_nothing() {
        assert_eq!(message_text("[NOTHING]"), None);
        assert_eq!(message_text("  [NOTHING]\n"), None);
        assert_eq!(message_text("[NOTHING] — all open items need the person."), None);
        assert_eq!(message_text(""), None);
        assert_eq!(message_text("While you were away I ran the link check.").as_deref(), Some("While you were away I ran the link check."));
    }

    #[test]
    fn the_prompt_forbids_outside_steps_and_asks_for_a_prediction() {
        let p = prompt(6);
        for words in ["Never send", "pay", "sign in", NOTHING, "While you were away I", "Next I expect", "about 6 h"] {
            assert!(p.contains(words), "{words}");
        }
    }
}
