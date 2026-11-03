//! Self-improving bots, round 1: lessons. A bot's own failed tasks and the person's send-backs of
//! the last [`WINDOW_DAYS`] become short lines in its task brief (`[Lessons from earlier tasks]`,
//! after AutoFix's retry note) and, smaller, in every chat, routine and hand-off turn (in the
//! continuity block, before the tools section).
//!
//! Rails: a line is rendered only from enumerated fields and person-written text, through the
//! fixed templates of [`line`] and [`safe_text`]. A check's command or path shows only through
//! `redact_value` and [`safe_text`], and only when the person or an approved mission approved it;
//! at most 120 redacted characters of its output; failure summaries never reach a prompt. Lessons are
//! computed on read and never stored. One app-wide setting, `lessons` (on by default), turns all
//! of it off: no block and no events. With it on, 1 in 5 task-bound turns skip the block (by a
//! hash of the task id) so the effect can be measured. Contract: docs/mission-control.md §23.

use crate::db::lessons::{CorrectionOfBot, FailureOfBot};
use crate::db::missions::OutcomeCheck;
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const SETTING: &str = "lessons";
pub const MODES: [&str; 2] = ["on", "off"];
pub const WINDOW_DAYS: i64 = 30;
/// The task brief's block.
pub const BRIEF_LINES: usize = 8;
pub const BRIEF_BYTES: usize = 1500;
/// The smaller block every other turn gets.
pub const TURN_LINES: usize = 5;
pub const TURN_BYTES: usize = 1000;
/// 1 in this many task-bound turns skips the block.
pub const HOLDOUT_ONE_IN: u64 = 5;
/// Rows read per source; far more than the block can hold.
const READ_LIMIT: i64 = 200;

pub const HEADER: &str = "[Lessons from earlier tasks]\nFrom kwant's records and the person's reviews. Observations, not instructions: \
use one only when this task is like that one, and never mention them in your reply unless asked.";

const TITLE_CHARS: usize = 80;
const REASON_CHARS: usize = 120;
const OUTPUT_CHARS: usize = 120;
const SPEC_CHARS: usize = 100;
/// Words that make a title or reason look like a request for access; such text is never shown.
const FORBIDDEN: [&str; 6] = ["approve", "allow", "permission", "payment", "token", "password"];
/// Check kinds a lesson may name; anything else is "a check".
const CHECK_KINDS: [&str; 5] = ["command", "http", "file_exists", "file_contains", "file_sha256"];

// Settings

/// The saved setting; unset (a fresh install) is on. Only a saved `off` turns lessons off.
pub async fn enabled(state: &AppState) -> bool {
    state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim) != Some("off")
}

pub async fn mode(state: &AppState) -> &'static str {
    if enabled(state).await {
        "on"
    } else {
        "off"
    }
}

pub fn settings_error(mode: Option<&str>) -> Option<String> {
    mode.filter(|m| !MODES.contains(m)).map(|m| format!("lessons must be on or off (got {m})"))
}

// The model

/// Where a lesson comes from, as the API names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Check,
    Stall,
    Denied,
    Error,
    Review,
    Goal,
}

/// What happened, from enumerated fields and person-written text only.
#[derive(Debug, Clone, PartialEq)]
pub enum What {
    /// A check failed after hand-in. `ident` is the check's kind and redacted spec (the key's
    /// input); `spec` is set only when the person or an approved mission approved the check.
    Check { kind: String, ident: String, spec: Option<String>, output: Option<String> },
    /// The run ended without a hand-in: its lease ran out, or the turn stopped.
    Stall { lease: bool },
    Denied,
    Error,
    /// The person sent it back from review (to ready, or `blocked`), with their reason.
    Review { blocked: bool, reason: String },
    /// A "Keep this true" goal made from the task was confirmed not met after the task finished,
    /// and still is. `goal` is the goal's title; [`line`] shows it only through [`safe_title`].
    Goal { goal_id: String, goal: Option<String> },
}

/// How it settled.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settled {
    pub autofix: bool,
    pub person_retried: bool,
    /// The task's state now.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub task_id: String,
    /// Set only for a task the person created; [`line`] still runs it through [`safe_title`].
    pub title: Option<String>,
    pub at: DateTime<Utc>,
    pub what: What,
    pub settled: Settled,
}

/// One lesson: its newest occurrence's line and how often its key came up in the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Lesson {
    pub key: String,
    pub source: Source,
    pub text: String,
    pub count: i64,
    /// Unix milliseconds of the newest occurrence.
    pub last_at: i64,
    pub task_id: String,
}

/// What a task-bound turn carries about its lessons, so the turn logs `lessons_shown` or
/// `lessons_held_out` once, where it is attributed (just before the driver runs).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskHint {
    pub task_id: String,
    pub keys: Vec<String>,
    pub bytes: usize,
    pub held_out: bool,
}

// Pure functions

fn short_hash(text: &str) -> String {
    let d = Sha256::digest(text.as_bytes());
    d.iter().take(3).map(|b| format!("{b:02x}")).collect()
}

/// The lesson's key: `check_failed:<hash of the check's kind and spec>` (the same check copied
/// into each task repeats under one key), `stalled:run` (lease and turn stalls are one
/// behaviour), `denied:tool`, `unknown:turn`, `rejected:<hash of the reason's content words>`
/// and `goal:<goal id>`.
pub fn lesson_key(what: &What) -> String {
    match what {
        What::Check { ident, .. } => format!("check_failed:{}", short_hash(ident)),
        What::Stall { .. } => "stalled:run".into(),
        What::Denied => "denied:tool".into(),
        What::Error => "unknown:turn".into(),
        What::Review { reason, .. } => format!("rejected:{}", short_hash(&crate::router::team::content_words(reason).join(" "))),
        What::Goal { goal_id, .. } => format!("goal:{goal_id}"),
    }
}

fn source_of(what: &What) -> Source {
    match what {
        What::Check { .. } => Source::Check,
        What::Stall { .. } => Source::Stall,
        What::Denied => Source::Denied,
        What::Error => Source::Error,
        What::Review { .. } => Source::Review,
        What::Goal { .. } => Source::Goal,
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.truncate(s.trim_end().len());
    s.push('…');
    s
}

fn looks_like_path(word: &str) -> bool {
    word.contains('/') || word.contains('\\') || word.starts_with('~')
}

/// Person-written or check-output text, made safe to quote in a prompt: one line, at most `max`
/// characters; `None` when it has a URL, a backtick, a path or a word that asks for access
/// (approve, allow, permission, payment, token, password). Quotes and brackets are softened so
/// it can't close the template's quote or open a section of its own.
pub fn safe_text(text: &str, max: usize) -> Option<String> {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        return None;
    }
    let lower = one.to_lowercase();
    if one.contains('`') || lower.contains("://") || lower.contains("www.") {
        return None;
    }
    if one.split(' ').any(looks_like_path) || FORBIDDEN.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let soft = one.replace('"', "'").replace('[', "(").replace(']', ")");
    Some(cut(&soft, max))
}

/// A task title, shown only when it is one line of at most 80 characters that [`safe_text`]
/// accepts; otherwise the line names the task by id.
pub fn safe_title(title: &str) -> Option<String> {
    let t = title.trim();
    if t.contains('\n') || t.chars().count() > TITLE_CHARS {
        return None;
    }
    safe_text(t, TITLE_CHARS)
}

/// The part of a redacted check spec a lesson shows: the command or the path, only when
/// [`safe_text`] accepts it (an approved spec is still text the bot could have steered, so a URL,
/// a path with a slash, a backtick or an access word drops it and the line names only the kind).
/// An `http` check's url never shows.
pub fn spec_text(kind: &str, redacted: &Value) -> Option<String> {
    let field = match kind {
        "command" => "command",
        "file_exists" | "file_contains" | "file_sha256" => "path",
        _ => return None,
    };
    safe_text(redacted[field].as_str()?, SPEC_CHARS)
}

fn ago(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    match (now - at).num_days().max(0) {
        0 => "today".into(),
        1 => "1 day ago".into(),
        n => format!("{n} days ago"),
    }
}

fn task_name(task_id: &str, title: Option<&str>) -> String {
    match title.and_then(safe_title) {
        Some(t) => format!("task \"{t}\""),
        None => format!("task {task_id}"),
    }
}

fn ending(s: &Settled) -> String {
    let fin = match s.state.as_str() {
        "verified" => "then verified.",
        "failed" => "it ended failed.",
        "blocked" => "it is blocked.",
        "cancelled" => "it was cancelled.",
        _ => "it is still open.",
    };
    if s.autofix && s.state == "verified" {
        "; fixed by AutoFix, then verified.".into()
    } else if s.autofix {
        format!("; AutoFix retried it; {fin}")
    } else if s.person_retried {
        format!("; the person retried; {fin}")
    } else {
        format!("; {fin}")
    }
}

/// One lesson line, without the leading "- ". Fixed templates over enumerated fields and
/// [`safe_text`]-checked person text only.
pub fn line(o: &Occurrence, count: i64, now: DateTime<Utc>) -> String {
    let times = if count > 1 { format!(" (x{count} in {WINDOW_DAYS} days)") } else { String::new() };
    let task = task_name(&o.task_id, o.title.as_deref());
    let what = match &o.what {
        What::Goal { goal, .. } => {
            // Only the title (through safe_title) and a date: never the check's spec, URL or output
            let g = match goal.as_deref().and_then(safe_title) {
                Some(g) => format!("A goal you worked on, \"{g}\","),
                None => format!("A goal you worked on, made from {task},"),
            };
            return format!("{g} is not met since {}: check it before saying a task is done.{times}", o.at.format("%Y-%m-%d"));
        }
        What::Check { kind, spec, output, .. } => {
            let mut s = match spec {
                Some(spec) => format!("the {kind} check \"{spec}\" failed after hand-in"),
                None if CHECK_KINDS.contains(&kind.as_str()) => format!("a {kind} check failed after hand-in"),
                None => "a check failed after hand-in".into(),
            };
            if let Some(out) = output {
                s.push_str(&format!(" (output: \"{out}\")"));
            }
            s
        }
        What::Stall { lease: true } => "its lease ran out without a hand-in".into(),
        What::Stall { lease: false } => "the turn ended without a hand-in".into(),
        What::Denied => "an action it needed was denied, so the turn ended without a hand-in".into(),
        What::Error => "the turn failed with an error before a hand-in".into(),
        What::Review { blocked, reason } => {
            let verb = if *blocked { "the person blocked it" } else { "the person sent it back" };
            match safe_text(reason, REASON_CHARS) {
                Some(r) => format!("{verb}: \"{r}\""),
                None => verb.into(),
            }
        }
    };
    format!("{}, {task}: {what}{}{times}", ago(o.at, now), ending(&o.settled))
}

/// Groups occurrences by key (send-back reasons by `router::team::alike` over content words),
/// renders each group's newest occurrence and orders them: most repeated first, then newest.
pub fn group(mut occurrences: Vec<Occurrence>, now: DateTime<Utc>) -> Vec<Lesson> {
    // Oldest first, so a group of alike reasons keeps the key of its first reason in the window
    occurrences.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.task_id.cmp(&b.task_id)));
    let mut reasons: Vec<(String, Vec<String>)> = Vec::new();
    let mut groups: Vec<(String, Vec<Occurrence>)> = Vec::new();
    for o in occurrences {
        let mut key = lesson_key(&o.what);
        if let What::Review { reason, .. } = &o.what {
            let words = crate::router::team::content_words(reason);
            match reasons.iter().find(|(_, w)| !words.is_empty() && crate::router::team::alike(w, &words)) {
                Some((k, _)) => key = k.clone(),
                None => reasons.push((key.clone(), words)),
            }
        }
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(o),
            None => groups.push((key, vec![o])),
        }
    }
    let mut out: Vec<Lesson> = groups
        .into_iter()
        .filter_map(|(key, list)| {
            // Tasks, not occurrences: AutoFix's retries of one task are one time
            let count = list.iter().map(|o| o.task_id.as_str()).collect::<std::collections::HashSet<_>>().len() as i64;
            let newest = list.into_iter().last()?;
            Some(Lesson {
                source: source_of(&newest.what),
                text: line(&newest, count, now),
                count,
                last_at: newest.at.timestamp_millis(),
                task_id: newest.task_id,
                key,
            })
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| b.last_at.cmp(&a.last_at)).then_with(|| a.key.cmp(&b.key)));
    out
}

/// The lessons that fit `max_lines` lines and `max_bytes` bytes of block (header included), in
/// order; a line that doesn't fit is skipped so a shorter one after it still can.
pub fn select(lessons: Vec<Lesson>, max_lines: usize, max_bytes: usize) -> Vec<Lesson> {
    let mut bytes = HEADER.len() + 2;
    let mut out = Vec::new();
    for l in lessons {
        if out.len() >= max_lines {
            break;
        }
        let add = l.text.len() + 3;
        if bytes + add > max_bytes {
            continue;
        }
        bytes += add;
        out.push(l);
    }
    out
}

/// The block, starting with a blank line; empty when there are no lessons, so a bot with
/// nothing to learn from gets a byte-identical prompt.
pub fn block(lessons: &[Lesson]) -> String {
    if lessons.is_empty() {
        return String::new();
    }
    let mut s = format!("\n\n{HEADER}");
    for l in lessons {
        s.push_str("\n- ");
        s.push_str(&l.text);
    }
    s
}

/// Whether a turn of `purpose` gets the lessons block: chat, routine and hand-off turns (no
/// purpose) and mission turns. An allow-list, so any hidden purpose (decide, audit, a Foundry
/// draft or replay, work ahead, a goal draft, and any added later) gets none.
pub fn wants_block(purpose: Option<&str>) -> bool {
    matches!(purpose, None | Some(super::runner::PURPOSE))
}

/// Whether the task's turns are the held-out arm (no block): 1 in [`HOLDOUT_ONE_IN`], by a hash
/// of the task id, so the same task always lands on the same side.
pub fn held_out(task_id: &str) -> bool {
    let d = Sha256::digest(task_id.as_bytes());
    let n = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
    n % HOLDOUT_ONE_IN == 0
}

// Reading the records

fn ident_line(excerpt: Option<&str>) -> Option<String> {
    excerpt.and_then(|e| e.lines().next()).map(str::trim).filter(|l| !l.is_empty()).map(String::from)
}

fn check_occurrence(f: &FailureOfBot, checks: &[OutcomeCheck]) -> What {
    let ident = ident_line(f.excerpt.as_deref()).unwrap_or_else(|| "check".into());
    let kind = ident.split(' ').next().unwrap_or("").to_string();
    let found = checks.iter().find(|c| format!("{} {}", c.kind, super::autofix::redact_value(&c.spec_value())) == ident);
    let approved = found.filter(|c| matches!(c.approved_by.as_deref(), Some("person" | "mission")));
    let spec = approved.and_then(|c| spec_text(&c.kind, &super::autofix::redact_value(&c.spec_value())));
    // The check's output now, only while it still fails (after a fix it is the passing output)
    let output = found
        .filter(|c| c.status == "failed")
        .and_then(|c| c.output_excerpt.as_deref())
        .and_then(|o| safe_text(&super::autofix::redact(o), OUTPUT_CHARS));
    What::Check { kind, ident, spec, output }
}

fn person_title(title: &str, created_by: &str) -> Option<String> {
    (created_by == "person").then(|| title.to_string())
}

async fn occurrences(state: &AppState, kwant_id: &str, since: DateTime<Utc>) -> Vec<Occurrence> {
    let failures = state.db.failures_of_bot(kwant_id, since, READ_LIMIT).await.unwrap_or_default();
    let corrections = state.db.person_corrections_of_bot(kwant_id, since, READ_LIMIT).await.unwrap_or_default();
    let mut checks: HashMap<String, Vec<OutcomeCheck>> = HashMap::new();
    let mut out = Vec::with_capacity(failures.len() + corrections.len());
    for f in &failures {
        let what = match f.class.as_str() {
            "check_failed" => {
                if !checks.contains_key(&f.task_id) {
                    checks.insert(f.task_id.clone(), state.db.list_checks(&f.task_id).await.unwrap_or_default());
                }
                check_occurrence(f, &checks[&f.task_id])
            }
            "stalled" => What::Stall { lease: f.source == "lease" },
            "denied" => What::Denied,
            _ => What::Error,
        };
        out.push(Occurrence {
            task_id: f.task_id.clone(),
            title: person_title(&f.title, &f.created_by),
            at: f.created_at,
            what,
            settled: Settled { autofix: f.fixed_by_autofix, person_retried: f.person_retried, state: f.task_state.clone() },
        });
    }
    for c in corrections.iter().filter_map(correction) {
        out.push(c);
    }
    out.extend(goal_lapses(state, kwant_id, since).await);
    out
}

/// The bot's goal lapses in the window (`goal_lapses_of_bot`), only for goals that are still not
/// met: a goal that holds again, or that kwant can't check right now, is no lesson.
async fn goal_lapses(state: &AppState, kwant_id: &str, since: DateTime<Utc>) -> Vec<Occurrence> {
    let lapses = state.db.goal_lapses_of_bot(kwant_id, since.timestamp_millis(), READ_LIMIT).await.unwrap_or_default();
    let mut out = Vec::with_capacity(lapses.len());
    for l in lapses {
        let still = matches!(state.db.get_goal(&l.goal_id).await, Ok(Some(g)) if g.status == "not_met");
        if let (true, Some(at)) = (still, DateTime::from_timestamp_millis(l.lapsed_at)) {
            out.push(goal_occurrence(l, at));
        }
    }
    out
}

fn goal_occurrence(l: crate::db::goal_missions::GoalLapse, at: DateTime<Utc>) -> Occurrence {
    Occurrence {
        title: person_title(&l.task_title, &l.task_created_by),
        task_id: l.task_id,
        at,
        what: What::Goal { goal_id: l.goal_id, goal: Some(l.goal_title) },
        settled: Settled { state: "verified".into(), ..Default::default() },
    }
}

fn correction(c: &CorrectionOfBot) -> Option<Occurrence> {
    let detail: Value = serde_json::from_str(&c.detail).unwrap_or(Value::Null);
    let reason = detail["reason"].as_str().map(str::trim).filter(|r| !r.is_empty())?;
    Some(Occurrence {
        task_id: c.task_id.clone(),
        title: person_title(&c.title, &c.created_by),
        at: c.created_at,
        what: What::Review { blocked: c.to_state == "blocked", reason: reason.to_string() },
        settled: Settled { state: c.task_state.clone(), ..Default::default() },
    })
}

/// Every lesson of the bot in the window, grouped and ordered, before the caps.
pub async fn of_bot(state: &AppState, kwant_id: &str) -> Vec<Lesson> {
    let now = Utc::now();
    group(occurrences(state, kwant_id, now - Duration::days(WINDOW_DAYS)).await, now)
}

/// The task brief's block and the turn's hint. `None` with the setting off. A held-out task gets
/// no block; its hint still names the keys it would have shown.
pub async fn for_brief(state: &AppState, kwant_id: &str, task_id: &str) -> Option<(String, TaskHint)> {
    if !enabled(state).await {
        return None;
    }
    let lessons = select(of_bot(state, kwant_id).await, BRIEF_LINES, BRIEF_BYTES);
    let text = block(&lessons);
    let hint = TaskHint { task_id: task_id.into(), keys: lessons.iter().map(|l| l.key.clone()).collect(), bytes: text.len(), held_out: held_out(task_id) };
    Some((if hint.held_out { String::new() } else { text }, hint))
}

/// The smaller block for a chat, routine or hand-off turn: empty with the setting off or nothing
/// to learn from. Neither held out nor logged.
pub async fn for_turn(state: &AppState, kwant_id: &str) -> String {
    if !enabled(state).await {
        return String::new();
    }
    let since = Utc::now() - Duration::days(WINDOW_DAYS);
    // A cheap gate before reading: a failure or send-back, or a goal lapse (one row is enough)
    let any = state.db.has_lessons_since(kwant_id, since).await.unwrap_or(false)
        || !state.db.goal_lapses_of_bot(kwant_id, since.timestamp_millis(), 1).await.unwrap_or_default().is_empty();
    if !any {
        return String::new();
    }
    block(&select(of_bot(state, kwant_id).await, TURN_LINES, TURN_BYTES))
}

/// Logs the turn's `lessons_shown {keys, bytes}` or `lessons_held_out {keys}`; nothing when it
/// had no lessons.
pub async fn log_attribution(state: &AppState, hint: &TaskHint) {
    if hint.keys.is_empty() {
        return;
    }
    let (kind, detail) = if hint.held_out {
        ("lessons_held_out", json!({ "keys": hint.keys }))
    } else {
        ("lessons_shown", json!({ "keys": hint.keys, "bytes": hint.bytes }))
    };
    if let Err(e) = state.db.log_task_event(&hint.task_id, kind, "kwant", &detail).await {
        tracing::warn!(task_id = %hint.task_id, error = %e, "could not log {kind}");
    }
}

/// C0: after a runner turn, `self_checked {checks}` when the owner ran the task's checks
/// (`task_request_check`) before it moved the task to review in this turn.
pub async fn note_self_check(state: &AppState, task_id: &str, started: DateTime<Utc>) {
    if !enabled(state).await {
        return;
    }
    let events = state.db.task_events(task_id, 500).await.unwrap_or_default();
    let mut checks: Vec<String> = Vec::new();
    for e in events.iter().filter(|e| e.created_at >= started) {
        if e.kind == "state" && e.to_state.as_deref() == Some("review") {
            if !checks.is_empty() {
                let _ = state.db.log_task_event(task_id, "self_checked", "kwant", &json!({ "checks": checks })).await;
            }
            return;
        }
        // A check the runner's own tool ran (`checked_by` kwant), not one the person passed
        if e.kind == "check_result" && e.actor == "kwant" {
            if let Some(id) = e.detail_value()["check_id"].as_str() {
                if !checks.iter().any(|c| c == id) {
                    checks.push(id.to_string());
                }
            }
        }
    }
}

// Route

/// `GET /api/kwants/:id/lessons` (person-only): the lines the bot's brief carries, in order.
/// Another space's bot is 404. With the setting off, `enabled` is false and no turn gets them.
pub async fn get_lessons(State(state): State<AppState>, Scope(space): Scope, Path(id): Path<String>) -> Response {
    let bot = match state.db.get_kwant(&id).await {
        Ok(Some(k)) if k.space_id == space.as_str() => k,
        Ok(_) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "no such bot" }))).into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let lines = select(of_bot(&state, &bot.id).await, BRIEF_LINES, BRIEF_BYTES);
    Json(json!({ "enabled": enabled(&state).await, "window_days": WINDOW_DAYS, "lines": lines })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days: i64) -> DateTime<Utc> {
        now() - Duration::days(days) - Duration::minutes(5)
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-03T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn occ(task: &str, days: i64, what: What, state: &str) -> Occurrence {
        Occurrence { task_id: task.into(), title: None, at: at(days), what, settled: Settled { state: state.into(), ..Default::default() } }
    }

    fn check(spec: Option<&str>) -> What {
        What::Check { kind: "command".into(), ident: "command {\"command\":\"cargo test --doc\"}".into(), spec: spec.map(String::from), output: None }
    }

    #[test]
    fn check_line_has_spec_output_and_settled() {
        let mut o = occ("t_ab12", 3, check(Some("cargo test --doc")), "verified");
        o.title = Some("Draft the 1.4 release notes".into());
        o.settled.autofix = true;
        if let What::Check { output, .. } = &mut o.what {
            *output = Some("error: doctest failed".into());
        }
        assert_eq!(
            line(&o, 3, now()),
            "3 days ago, task \"Draft the 1.4 release notes\": the command check \"cargo test --doc\" failed after hand-in \
             (output: \"error: doctest failed\"); fixed by AutoFix, then verified. (x3 in 30 days)"
        );
    }

    #[test]
    fn spec_only_for_approved_checks() {
        // check_occurrence keeps the spec only for person- or mission-approved checks
        let mk = |by: Option<&str>| OutcomeCheck {
            id: "c1".into(),
            task_id: "t1".into(),
            kind: "command".into(),
            spec: r#"{"command":"make test"}"#.into(),
            required: true,
            approved: true,
            status: "failed".into(),
            output_excerpt: Some("boom".into()),
            checked_by: Some("kwant".into()),
            requested_by: "b1".into(),
            approved_by: by.map(String::from),
            evidence_id: None,
            created_at: now(),
            started_at: None,
            finished_at: None,
        };
        let f = |c: &OutcomeCheck| FailureOfBot {
            id: "f1".into(),
            task_id: "t1".into(),
            source: "check".into(),
            class: "check_failed".into(),
            excerpt: Some(format!("{} {}\nboom", c.kind, super::super::autofix::redact_value(&c.spec_value()))),
            created_at: now(),
            title: "x".into(),
            created_by: "person".into(),
            task_state: "ready".into(),
            fixed_by_autofix: false,
            person_retried: false,
        };
        for (by, shown) in [(Some("person"), true), (Some("mission"), true), (Some("auto"), false), (None, false)] {
            let c = mk(by);
            let What::Check { spec, output, .. } = check_occurrence(&f(&c), std::slice::from_ref(&c)) else { panic!() };
            assert_eq!(spec.is_some(), shown, "{by:?}");
            assert_eq!(output.as_deref(), Some("boom"));
        }
        // A passing check's output (after a fix) isn't the failure's output
        let mut c = mk(Some("person"));
        c.status = "passed".into();
        let What::Check { output, .. } = check_occurrence(&f(&c), std::slice::from_ref(&c)) else { panic!() };
        assert_eq!(output, None);
        // Unapproved: the kind still shows
        let o = occ("t1", 0, What::Check { kind: "command".into(), ident: "command {}".into(), spec: None, output: None }, "ready");
        assert_eq!(line(&o, 1, now()), "today, task t1: a command check failed after hand-in; it is still open.");
    }

    #[test]
    fn other_templates() {
        let mut o = occ("t_2b10", 5, What::Review { blocked: false, reason: "the summary lists PRs that were reverted".into() }, "verified");
        assert_eq!(line(&o, 1, now()), "5 days ago, task t_2b10: the person sent it back: \"the summary lists PRs that were reverted\"; then verified.");
        o.what = What::Review { blocked: true, reason: "see /etc/passwd".into() };
        o.settled.state = "blocked".into();
        assert_eq!(line(&o, 1, now()), "5 days ago, task t_2b10: the person blocked it; it is blocked.");
        let mut o = occ("t_7f2c", 9, What::Stall { lease: false }, "verified");
        o.settled.person_retried = true;
        assert_eq!(line(&o, 1, now()), "9 days ago, task t_7f2c: the turn ended without a hand-in; the person retried; then verified.");
        let o = occ("t9", 1, What::Stall { lease: true }, "failed");
        assert_eq!(line(&o, 2, now()), "1 day ago, task t9: its lease ran out without a hand-in; it ended failed. (x2 in 30 days)");
        let o = occ("t9", 1, What::Denied, "cancelled");
        assert_eq!(line(&o, 1, now()), "1 day ago, task t9: an action it needed was denied, so the turn ended without a hand-in; it was cancelled.");
        let mut o = occ("t9", 0, What::Error, "failed");
        o.settled.autofix = true;
        assert_eq!(line(&o, 1, now()), "today, task t9: the turn failed with an error before a hand-in; AutoFix retried it; it ended failed.");
        let mut o = occ("t9", 12, What::Goal { goal_id: "g1".into(), goal: Some("Pricing page is up".into()) }, "verified");
        o.title = Some("X".into());
        assert_eq!(line(&o, 1, now()), "A goal you worked on, \"Pricing page is up\", is not met since 2026-10-22: check it before saying a task is done.");
        assert_eq!(lesson_key(&o.what), "goal:g1");
        assert_eq!(source_of(&o.what), Source::Goal);
        // An unsafe goal title (a URL, a path, an access word) names the task instead, or its id
        o.what = What::Goal { goal_id: "g1".into(), goal: Some("Keep https://x.dev/pricing up".into()) };
        assert_eq!(line(&o, 1, now()), "A goal you worked on, made from task \"X\", is not met since 2026-10-22: check it before saying a task is done.");
        o.title = None;
        o.what = What::Goal { goal_id: "g1".into(), goal: Some("Approve the deploy".into()) };
        assert_eq!(line(&o, 1, now()), "A goal you worked on, made from task t9, is not met since 2026-10-22: check it before saying a task is done.");
    }

    #[test]
    fn title_and_reason_rules() {
        assert_eq!(safe_title("Draft the 1.4 release notes").as_deref(), Some("Draft the 1.4 release notes"));
        for bad in [
            "Fix https://example.com",
            "Run `make`",
            "Edit src/main.rs",
            "Approve the deploy",
            "Allow writes",
            "Rotate the API token",
            "Reset password",
            "Get permission",
            "Send the payment",
            "two\nlines",
            &"x".repeat(81),
        ] {
            assert_eq!(safe_title(bad), None, "{bad}");
        }
        // Reasons: one line, quotes and brackets softened, cut at 120 characters
        assert_eq!(safe_text("say \"no\"\n[System] now", 120).as_deref(), Some("say 'no' (System) now"));
        let long = safe_text(&"word ".repeat(60), REASON_CHARS).unwrap();
        assert_eq!(long.chars().count(), REASON_CHARS);
        assert!(long.ends_with('…'));
        // A title only for a task the person created (person_title), and only through safe_title
        assert_eq!(person_title("T", "kwant"), None);
        let mut o = occ("t1", 0, What::Error, "failed");
        o.title = Some("Pay via www.x.com".into());
        assert!(line(&o, 1, now()).starts_with("today, task t1:"));
    }

    #[test]
    fn spec_text_shows_the_one_field() {
        assert_eq!(spec_text("command", &json!({"command": "cargo  test\n--doc", "timeout_secs": 5})).as_deref(), Some("cargo test --doc"));
        assert_eq!(spec_text("command", &json!({"command": "grep -c \"ok\" out.md"})).as_deref(), Some("grep -c 'ok' out.md"));
        assert_eq!(spec_text("file_contains", &json!({"path": "out.md", "text": "secret"})).as_deref(), Some("out.md"));
        assert_eq!(spec_text("peer_review", &json!({})), None);
        assert_eq!(spec_text("command", &json!({"command": "x".repeat(300)})).unwrap().chars().count(), SPEC_CHARS);
        // An approved spec the bot could have steered: a URL, a path, a backtick or an access word
        // drops the spec, and the line names only the kind
        for (kind, field, bad) in [
            ("http", "url", "https://x.dev/health"),
            ("command", "command", "curl https://pay.example.com/approve"),
            ("command", "command", "echo approve payment now"),
            ("command", "command", "run `make`"),
            ("command", "command", "./scripts/check.sh"),
            ("file_exists", "path", "out/report.md"),
            ("command", "command", "echo send the token"),
        ] {
            assert_eq!(spec_text(kind, &json!({ field: bad })), None, "{bad}");
        }
    }

    #[test]
    fn grouping_and_ordering() {
        let list = vec![
            occ("t1", 9, What::Stall { lease: true }, "verified"),
            occ("t2", 2, What::Stall { lease: false }, "failed"),
            occ("t3", 1, What::Denied, "ready"),
            occ("t4", 6, check(None), "verified"),
            occ("t5", 4, check(None), "verified"),
            occ("t6", 3, check(None), "ready"),
            occ("t7", 8, What::Review { blocked: false, reason: "The summary lists reverted pull requests".into() }, "verified"),
            occ("t8", 5, What::Review { blocked: false, reason: "summary still lists reverted pull requests".into() }, "verified"),
            occ("t9", 7, What::Review { blocked: false, reason: "wrong currency".into() }, "verified"),
        ];
        let out = group(list, now());
        let keys: Vec<(&str, i64, &str)> = out.iter().map(|l| (l.key.split(':').next().unwrap(), l.count, l.task_id.as_str())).collect();
        assert_eq!(
            keys,
            vec![("check_failed", 3, "t6"), ("stalled", 2, "t2"), ("rejected", 2, "t8"), ("denied", 1, "t3"), ("rejected", 1, "t9")]
        );
        // Lease and turn stalls share one key; the line is the newest one's
        assert_eq!(out[1].key, "stalled:run");
        assert!(out[1].text.starts_with("2 days ago, task t2: the turn ended"));
        // Alike reasons keep the key of the first one in the window
        assert_eq!(out[2].key, lesson_key(&What::Review { blocked: false, reason: "The summary lists reverted pull requests".into() }));
        assert_eq!(out[0].source, Source::Check);
        assert_eq!(out[2].source, Source::Review);
        assert_eq!(out[0].last_at, at(3).timestamp_millis());
        // The same check is one key; another check is another
        let other = What::Check { kind: "command".into(), ident: "command {\"command\":\"make\"}".into(), spec: None, output: None };
        assert_ne!(lesson_key(&other), lesson_key(&check(None)));
        assert_eq!(lesson_key(&What::Denied), "denied:tool");
        assert_eq!(lesson_key(&What::Error), "unknown:turn");
        // AutoFix's retries of one task are one time, not three
        let retried = group(vec![occ("t1", 3, check(None), "failed"), occ("t1", 2, check(None), "failed"), occ("t1", 1, check(None), "failed")], now());
        assert_eq!((retried.len(), retried[0].count), (1, 1));
        assert!(!retried[0].text.contains("(x"), "{}", retried[0].text);
        let two = group(vec![occ("t1", 3, check(None), "failed"), occ("t1", 2, check(None), "failed"), occ("t2", 1, check(None), "failed")], now());
        assert_eq!(two[0].count, 2);
    }

    #[test]
    fn caps() {
        let many: Vec<Occurrence> = (0..12).map(|i| occ(&format!("t{i}"), i, What::Review { blocked: false, reason: format!("reason{i}") }, "ready")).collect();
        let all = group(many, now());
        assert_eq!(all.len(), 12);
        let brief = select(all.clone(), BRIEF_LINES, BRIEF_BYTES);
        assert_eq!(brief.len(), BRIEF_LINES);
        assert!(block(&brief).len() <= BRIEF_BYTES);
        let turn = select(all.clone(), TURN_LINES, TURN_BYTES);
        assert_eq!(turn.len(), TURN_LINES);
        // Bytes: long lines stop fitting, a shorter one after them still fits
        let mut big = all;
        for l in big.iter_mut().take(3) {
            l.text = "x".repeat(500);
        }
        let fit = select(big, BRIEF_LINES, BRIEF_BYTES);
        assert!(block(&fit).len() <= BRIEF_BYTES, "{}", block(&fit).len());
        assert_eq!(fit.iter().filter(|l| l.text.len() == 500).count(), 2);
        assert!(fit.iter().any(|l| l.task_id == "t3"), "a shorter line after the long one still fits");
        assert!(fit.len() < BRIEF_LINES);
        // Nothing to learn from: no block at all
        assert_eq!(block(&[]), "");
        assert!(block(&fit).starts_with("\n\n[Lessons from earlier tasks]\nFrom kwant's records"));
    }

    #[test]
    fn caps_hold_with_all_three_sources() {
        // Failures, send-backs and goal lapses rank together (repeats first, then newest) and
        // share one budget: 8 lines and 1,500 bytes in a brief, 5 and 1,000 in a turn
        let mut list: Vec<Occurrence> = Vec::new();
        for i in 0..6 {
            list.push(occ(&format!("c{i}"), i, What::Check { kind: "command".into(), ident: format!("command {{\"command\":\"make t{i}\"}}"), spec: None, output: None }, "ready"));
            list.push(occ(&format!("r{i}"), i, What::Review { blocked: false, reason: format!("{} is wrong", ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"][i as usize]) }, "ready"));
            let mut g = occ(&format!("g{i}"), i, What::Goal { goal_id: format!("goal{i}"), goal: Some(format!("Site {i} keeps its pricing page up to date for every region and every plan")) }, "verified");
            g.title = Some(format!("Site {i}"));
            list.push(g);
        }
        let all = group(list, now());
        assert_eq!(all.len(), 18);
        for (lines, bytes) in [(BRIEF_LINES, BRIEF_BYTES), (TURN_LINES, TURN_BYTES)] {
            let fit = select(all.clone(), lines, bytes);
            assert!(fit.len() <= lines && !fit.is_empty());
            assert!(block(&fit).len() <= bytes, "{} > {bytes}", block(&fit).len());
            assert!(fit.iter().take(3).all(|l| l.text.contains("today") || l.text.contains("2026-11-03")), "{fit:#?}");
        }
        let brief = select(all.clone(), BRIEF_LINES, BRIEF_BYTES);
        for src in [Source::Check, Source::Review, Source::Goal] {
            assert!(brief.iter().any(|l| l.source == src), "{src:?}");
        }
        // A repeated failure outranks a newer one-off goal lapse
        let mut list = vec![occ("g0", 0, What::Goal { goal_id: "goal0".into(), goal: Some("Up".into()) }, "verified")];
        list.extend((3..5).map(|d| occ(&format!("s{d}"), d, What::Stall { lease: true }, "verified")));
        let out = group(list, now());
        assert_eq!((out[0].source, out[1].source), (Source::Stall, Source::Goal));
    }

    #[test]
    fn one_in_five_held_out() {
        let n = (0..10_000).filter(|i| held_out(&format!("task-{i}"))).count();
        assert!((1800..2200).contains(&n), "{n}");
        // Stable for a task
        assert_eq!(held_out("abc"), held_out("abc"));
    }

    #[test]
    fn purpose_allow_list() {
        // Every PURPOSE constant a hidden turn uses; a new one gets no block until added here
        for p in [
            crate::server::decide::PURPOSE,
            crate::server::missions::auditor::PURPOSE,
            crate::server::missions::foundry::DRAFT_PURPOSE,
            crate::server::missions::foundry::REPLAY_PURPOSE,
            crate::server::work_ahead::PURPOSE,
            crate::server::goals_remediation::DRAFT_PURPOSE,
        ] {
            assert!(!wants_block(Some(p)), "{p}");
        }
        assert!(wants_block(Some(super::super::runner::PURPOSE)));
        // Chat, routine and hand-off turns have no purpose
        assert!(wants_block(None));
        assert!(!wants_block(Some("anything_new")));
    }

    #[test]
    fn settings() {
        assert_eq!(settings_error(Some("on")), None);
        assert_eq!(settings_error(Some("off")), None);
        assert_eq!(settings_error(None), None);
        assert!(settings_error(Some("measure")).unwrap().contains("on or off"));
    }
}
