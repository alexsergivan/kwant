//! "While you were away" (batch 3): `GET /api/away` lists what's waiting on the user, what
//! finished and what failed or got stuck; `POST /api/away/seen` records that the user looked,
//! which moves the default `since` and resets the unseen counts for the sidebar badge.
//!
//! Summaries are one line of at most [`SUMMARY_CHARS`] with anything key-like masked: results
//! and errors can carry provider text.

use super::spaces::Scope;
use super::AppState;
use crate::db::away::{AwayRow, LAST_SEEN_KEY};
use crate::db::spaces::SpaceId;
use crate::profiles::classify::{mask_credentials, redact_detail};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Most items per list.
pub const MAX_ITEMS: i64 = 50;
/// A delegation still running this long after it started counts as stuck.
pub const STUCK_AFTER_MINUTES: i64 = 30;
/// How far back the page looks when the user has never marked it seen.
pub const DEFAULT_LOOKBACK_DAYS: i64 = 7;
pub const SUMMARY_CHARS: usize = 200;
pub(super) const TITLE_CHARS: usize = 120;
/// Argument names whose values are never shown in an approval's summary.
const SECRET_ARG_HINTS: [&str; 6] = ["key", "token", "secret", "password", "auth", "cookie"];

/// Argument names that hold a file or folder path. The card shows these in full, so the person
/// can see which file a bot wants (a path with digits would otherwise look key-like); only a
/// password inside a URL is still masked.
const PATH_ARGS: [&str; 6] = ["file_path", "path", "notebook_path", "cwd", "directory", "dir"];

fn is_path_arg(name: &str) -> bool {
    PATH_ARGS.contains(&name.to_ascii_lowercase().as_str())
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn internal() -> Response {
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

pub(super) fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Whether `word` looks like a key or token, by the same rule provider errors are masked with.
fn looks_secret(word: &str) -> bool {
    !word.is_empty() && redact_detail(word, usize::MAX) == "[redacted]"
}

/// One line for a card: the first non-empty line without markdown markers, at most
/// [`SUMMARY_CHARS`], with free-text credentials (`password=…`) and any word that has a key-like
/// part (`"sk-…"`) masked.
pub fn summarize(text: &str) -> String {
    let line = text
        .lines()
        .map(|l| l.trim().trim_start_matches(['#', '>', '*', '-', ' ']).trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let joined = mask_words(line);
    if joined.chars().count() > SUMMARY_CHARS {
        let cut: String = joined.chars().take(SUMMARY_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        joined
    }
}

/// `text` with free-text credentials and any word that has a key-like part masked.
pub(super) fn mask_words(text: &str) -> String {
    let line = mask_credentials(text);
    let masked: Vec<&str> = line
        .split_whitespace()
        .map(|w| {
            if w.split(|c: char| "\"'`=:,;()[]{}<>".contains(c)).any(looks_secret) {
                "[redacted]"
            } else {
                w
            }
        })
        .collect();
    masked.join(" ")
}

/// An approval's stored details in the live `approval_required` event's shape. The tool's
/// `arguments` get the summary's masking (sensitive-named fields, credentials, key-like words),
/// as the feed never carries secrets; the rest (source, tool, ids, cwd) is kwnt's own.
pub(super) fn masked_details(details: Value) -> Value {
    let mut details = details;
    if let Some(args) = details.get_mut("arguments") {
        *args = masked_arguments(args);
    }
    details
}

fn masked_arguments(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let lower = k.to_ascii_lowercase();
                    let shown = match (SECRET_ARG_HINTS.iter().any(|h| lower.contains(h)), v) {
                        (true, _) => json!("[redacted]"),
                        (false, Value::String(path)) if is_path_arg(k) => json!(mask_credentials(path)),
                        (false, _) => masked_arguments(v),
                    };
                    (k.clone(), shown)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(masked_arguments).collect()),
        // Untouched strings keep their line breaks; masked ones come back on one line
        Value::String(s) => {
            let masked = mask_words(s);
            match masked == s.split_whitespace().collect::<Vec<_>>().join(" ") {
                true => v.clone(),
                false => json!(masked),
            }
        }
        other => other.clone(),
    }
}

fn clip_title(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TITLE_CHARS {
        return line;
    }
    let cut: String = line.chars().take(TITLE_CHARS - 1).collect();
    format!("{cut}…")
}

/// An approval's stored details as `name: value` pairs, sensitive argument names masked.
pub(super) fn approval_summary(parameters: &str) -> String {
    let details: Value = serde_json::from_str(parameters).unwrap_or(Value::Null);
    let mut parts = Vec::new();
    if let Some(args) = details["arguments"].as_object() {
        for (k, v) in args {
            let lower = k.to_ascii_lowercase();
            let shown = match v {
                _ if SECRET_ARG_HINTS.iter().any(|h| lower.contains(h)) => "[redacted]".to_string(),
                Value::String(path) if is_path_arg(k) => mask_credentials(path).split_whitespace().collect::<Vec<_>>().join(" "),
                Value::String(s) => mask_words(s),
                other => mask_words(&other.to_string()),
            };
            parts.push(format!("{k}: {shown}"));
        }
    }
    if parts.is_empty() {
        return "Wants your approval to use this tool".into();
    }
    clip_summary(&parts.join(", "))
}

/// [`summarize`]'s length cut, for text that is already masked.
fn clip_summary(line: &str) -> String {
    let line = line.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    if line.chars().count() > SUMMARY_CHARS {
        let cut: String = line.chars().take(SUMMARY_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_string()
    }
}

fn approval_title(tool_name: &str, parameters: &str) -> String {
    let details: Value = serde_json::from_str(parameters).unwrap_or(Value::Null);
    match (details["tool"].as_str(), details["server"].as_str()) {
        (Some(tool), Some(server)) => clip_title(&format!("Use {tool} ({server})")),
        _ => clip_title(&format!("Use {tool_name}")),
    }
}

fn fallback_summary(row: &AwayRow) -> &'static str {
    match (row.kind.as_str(), row.status.as_str()) {
        (_, "stuck") => "Still running with no result yet",
        (_, "completed" | "ok") => "Finished",
        (_, "interrupted") => "Stopped before it finished",
        _ => "Failed",
    }
}

pub(super) fn item(row: AwayRow) -> Value {
    let (title, summary) = match row.kind.as_str() {
        "approval" => {
            let params = row.body.as_deref().unwrap_or("");
            (approval_title(&row.title, params), approval_summary(params))
        }
        "question" => {
            let choices: Vec<String> = row
                .choices
                .as_deref()
                .and_then(|c| serde_json::from_str(c).ok())
                .unwrap_or_default();
            let summary = if choices.is_empty() {
                "Waiting for your answer".to_string()
            } else {
                summarize(&format!("Choices: {}", choices.join(" · ")))
            };
            (clip_title(&row.title), summary)
        }
        _ => {
            // A usage-limit line is not a summary of anything (also when an older run saved one)
            let summary = row
                .body
                .as_deref()
                .filter(|b| !crate::profiles::classify::is_limit_line(b))
                .map(summarize)
                .filter(|s| !s.is_empty());
            (clip_title(&row.title), summary.unwrap_or_else(|| fallback_summary(&row).into()))
        }
    };
    let mut v = json!({
        "kind": row.kind,
        "id": row.id,
        "kwant_id": row.kwant_id,
        "kwant_name": row.kwant_name.unwrap_or_else(|| "A bot".into()),
        "title": title,
        "summary": summary,
        "status": row.status,
        "timestamp": rfc3339(row.at),
        "link": {
            "thread_id": row.thread_id,
            "channel_id": row.channel_id,
            "direct_kwant_id": row.direct_kwant_id,
        },
    });
    if row.kind == "approval" {
        // The live `approval_required` event's fields, so an approval opened from here renders
        // like one that arrived over SSE; secret values are masked, as everywhere in the feed.
        v["tool_name"] = json!(row.title);
        let details: Value = row.body.as_deref().and_then(|b| serde_json::from_str(b).ok()).unwrap_or(Value::Null);
        v["details"] = masked_details(details);
    }
    if row.kind == "question" {
        v["choices"] = row
            .choices
            .as_deref()
            .and_then(|c| serde_json::from_str(c).ok())
            .unwrap_or_else(|| json!([]));
    }
    v
}

/// The space's last-seen marker. Default keeps the marker from before spaces.
fn seen_key(space: &SpaceId) -> String {
    match space.is_default() {
        true => LAST_SEEN_KEY.to_string(),
        false => format!("{LAST_SEEN_KEY}:{space}"),
    }
}

pub(super) async fn last_seen(state: &AppState, space: &SpaceId) -> Result<Option<DateTime<Utc>>, crate::db::DbError> {
    Ok(state
        .db
        .get_setting(&seen_key(space))
        .await?
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|t| t.with_timezone(&Utc)))
}

/// The space's unread badge: what waits on the user plus what finished or failed since they
/// last looked, as `unseen.total` of `GET /api/away?space=` counts it.
pub async fn unread_total(state: &AppState, space: &SpaceId) -> Result<i64, crate::db::DbError> {
    let now = Utc::now();
    let since = last_seen(state, space).await?.unwrap_or(now - Duration::days(DEFAULT_LOOKBACK_DAYS));
    let watch = crate::server::missions::watch::waiting_items(state, space).await.len() as i64;
    let waiting = state.db.away_count_open_questions(space).await? + waiting_approvals(state, space).await?.len() as i64 + watch;
    let (finished, failed) = state.db.away_counts(space, since, now - Duration::minutes(STUCK_AFTER_MINUTES)).await?;
    Ok(waiting + finished + failed)
}

/// The space's approvals the broker still waits on, each linked to the chat its turn runs in.
pub(super) async fn waiting_approvals(state: &AppState, space: &SpaceId) -> Result<Vec<AwayRow>, crate::db::DbError> {
    Ok(approvals_by_space(state, Some(space)).await?.remove(space).unwrap_or_default())
}

/// The waiting approvals of every space (`only`: just that one), in one pass: each card's space
/// is looked up once, and only the cards that are kept are linked to their chat.
pub(super) async fn approvals_by_space(state: &AppState, only: Option<&SpaceId>) -> Result<HashMap<SpaceId, Vec<AwayRow>>, crate::db::DbError> {
    let mut spaces: HashMap<String, SpaceId> = HashMap::new();
    let mut by_space: HashMap<SpaceId, Vec<AwayRow>> = HashMap::new();
    let live = state.broker.pending_ids().await;
    let turns = state.turns.active_turns();
    for mut row in state.db.away_pending_approvals(&live).await? {
        let of = space_of(state, &mut spaces, &row.kwant_id).await?;
        if only.is_some_and(|s| s != &of) {
            continue;
        }
        // turn_id isn't in the row; look it up by ticket
        let turn_id = state.db.get_approval(&row.id).await?.map(|a| a.turn_id);
        let turn = turns.iter().find(|t| Some(&t.turn_id) == turn_id.as_ref());
        match turn {
            Some(t) if t.thread_id.is_some() => row.thread_id = t.thread_id.clone(),
            Some(t) => match state.db.get_channel(&t.context_id).await?.is_some() {
                true => row.channel_id = Some(t.context_id.clone()),
                false => row.direct_kwant_id = Some(t.context_id.clone()),
            },
            None => row.direct_kwant_id = Some(row.kwant_id.clone()),
        }
        by_space.entry(of).or_default().push(row);
    }
    // Cards whose turn stopped waiting: still answerable, and a late Allow resumes the work
    for row in state.db.away_waiting_approvals().await? {
        let of = space_of(state, &mut spaces, &row.kwant_id).await?;
        if only.is_some_and(|s| s != &of) {
            continue;
        }
        let rows = by_space.entry(of).or_default();
        if !rows.iter().any(|r| r.id == row.id) {
            rows.push(row);
        }
    }
    Ok(by_space)
}

/// A bot's space, read once per call ([`approvals_by_space`]).
async fn space_of(state: &AppState, cache: &mut HashMap<String, SpaceId>, kwant_id: &str) -> Result<SpaceId, crate::db::DbError> {
    if let Some(s) = cache.get(kwant_id) {
        return Ok(s.clone());
    }
    let of = state.db.space_of_kwant(kwant_id).await?.unwrap_or_else(SpaceId::default_space);
    cache.insert(kwant_id.to_string(), of.clone());
    Ok(of)
}

/// The space's open stopped items, as Needs-you cards: `title` is what stopped and what resumes
/// it, `message_id` the chat notice that carries Retry or Continue, `retry` set when Retry is
/// `POST /api/waiting/:id/retry` rather than the chat's own.
pub(super) async fn stopped_items(state: &AppState, space: &SpaceId) -> Vec<Value> {
    close_gone_places(state, space).await;
    let rows = state.db.list_waiting(Some(space)).await.unwrap_or_default();
    let mut out = Vec::new();
    for w in rows.into_iter().filter(|w| w.kind.as_deref() == Some(crate::db::waiting::STOPPED)) {
        let name = state.db.get_kwant(&w.kwant_id).await.ok().flatten().map(|k| k.name);
        out.push(json!({
            "kind": "stopped",
            "id": w.id,
            "kwant_id": w.kwant_id,
            "kwant_name": name.unwrap_or_else(|| "A bot".into()),
            "title": super::stopped::title(&w.question, TITLE_CHARS),
            "summary": w.question,
            "status": "open",
            "timestamp": rfc3339(w.created_at),
            "message_id": w.message_id,
            "retry": w.retry.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok()),
            "link": {
                "thread_id": w.thread_id,
                "channel_id": w.channel_id,
                "direct_kwant_id": w.direct_kwant_id,
            },
        }));
    }
    // A sign-in, code or CAPTCHA step the person hasn't done: the bot's work waits on it (0.9.5,
    // #9), so it is in Needs you too, not only a card in its chat
    for s in state.db.list_human_steps(Some("open"), None, Some(space)).await.unwrap_or_default() {
        let name = state.db.get_kwant(&s.kwant_id).await.ok().flatten().map(|k| k.name);
        out.push(json!({
            "kind": "stopped",
            "id": s.id,
            "human_step_id": s.id,
            "kwant_id": s.kwant_id,
            "kwant_name": name.unwrap_or_else(|| "A bot".into()),
            "title": super::stopped::title(&s.reason, TITLE_CHARS),
            "summary": s.reason,
            "status": "open",
            "timestamp": rfc3339(s.created_at),
            "message_id": s.message_id,
            "retry": Value::Null,
            "link": {
                "thread_id": s.thread_id,
                "channel_id": s.channel_id,
                "direct_kwant_id": s.direct_kwant_id,
            },
        }));
    }
    out
}

/// 22i: an item whose bot, channel or thread is gone has nowhere to open, and the client fell back
/// to the bot's direct chat, where nothing says what is needed. Close those, live, before listing.
async fn close_gone_places(state: &AppState, space: &SpaceId) {
    for w in state.db.waiting_places_gone(Some(space)).await.unwrap_or_default() {
        if let Ok(Some(closed)) = state.db.claim_waiting(&w.id).await {
            tracing::info!(waiting_id = %closed.id, kwant_id = %closed.kwant_id, "closed a Needs-you item whose chat is gone");
            super::waiting::closed(state, std::slice::from_ref(&closed), super::waiting::REASON_PLACE_GONE);
        }
    }
}

/// 22i: what the person does about an item, so its row can say it in one word:
/// `retry` (rerun what stopped or failed), `continue` (a chain paused at its cap), `ask_again`
/// (an approval that expired), `answer` (a bot's question), `decide` (an approval, a watch item or
/// work that is stuck), `browser` (a sign-in, code or CAPTCHA step). `None` for what only needs a
/// look (finished work).
pub(super) fn action_of(v: &Value) -> Option<&'static str> {
    let kind = v["kind"].as_str().unwrap_or("");
    let status = v["status"].as_str().unwrap_or("");
    Some(match kind {
        "approval" | "watch" => "decide",
        "question" => "answer",
        "stopped" if v.get("human_step_id").is_some_and(|h| !h.is_null()) => "browser",
        "stopped" => match v["retry"]["type"].as_str() {
            Some("approval") => "ask_again",
            Some("continue") => "continue",
            _ => "retry",
        },
        // A failed hand-off reruns only through its board task (`task_id`); without one there is
        // no Retry to offer: the person asks again in its thread (a later reply closes the row) or
        // dismisses it
        "delegation" => match status {
            "stuck" => "decide",
            "failed" | "interrupted" if v["task_id"].is_string() => "retry",
            _ => return None,
        },
        "routine_run" => match status {
            "stuck" => "decide",
            "error" => "retry",
            _ => return None,
        },
        _ => return None,
    })
}

/// Whether the person may clear the item with `POST /api/waiting/:id/dismiss`: a stop, a failed or
/// stuck hand-off or routine run. A question, approval, watch item or browser step has its own
/// answer or decline, which tells the bot; dismissing it would leave the bot waiting.
pub(super) fn dismissable(v: &Value) -> bool {
    let kind = v["kind"].as_str().unwrap_or("");
    let status = v["status"].as_str().unwrap_or("");
    match kind {
        "stopped" => v.get("human_step_id").is_none_or(|h| h.is_null()),
        "delegation" | "routine_run" => matches!(status, "stuck" | "failed" | "interrupted" | "error"),
        _ => false,
    }
}

/// 22i: a failed or interrupted hand-off gets `task_id` when the board has a failed task for it,
/// which `POST /api/tasks/:id/retry` reruns. Without it the row's `action` is `null`.
async fn add_hand_off_task(db: &crate::db::Database, v: &mut Value) {
    if v["kind"] != "delegation" || !matches!(v["status"].as_str(), Some("failed" | "interrupted")) {
        return;
    }
    let Some(id) = v["id"].as_str() else { return };
    if let Ok(Some(task_id)) = db.retryable_hand_off_task(id).await {
        v["task_id"] = json!(task_id);
    }
}

/// Adds `action`, `dismissable`, `task_id` on a failed hand-off with a board task and, when the item's thread belongs to another bot,
/// `link.thread_kwant_id`: the client looks a thread up under its owner, and finding it under the
/// item's bot failed, falling back to the direct chat (one way to Quant's case).
async fn annotate(state: &AppState, items: &mut [Value]) {
    let mut owners: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    for v in items.iter_mut() {
        add_hand_off_task(&state.db, v).await;
        v["action"] = json!(action_of(v));
        v["dismissable"] = json!(dismissable(v));
        let Some(thread_id) = v["link"]["thread_id"].as_str().map(str::to_string) else { continue };
        if !owners.contains_key(&thread_id) {
            let owner = state.db.get_thread(&thread_id).await.ok().flatten().map(|t| t.kwant_id);
            owners.insert(thread_id.clone(), owner);
        }
        if let Some(Some(owner)) = owners.get(&thread_id) {
            if v["kwant_id"].as_str() != Some(owner.as_str()) {
                v["link"]["thread_kwant_id"] = json!(owner);
            }
        }
    }
}

/// What waits on the person in a space, as `GET /api/away` lists it under `waiting`: open
/// questions, approvals, what a mission watcher left (P1) and work that stopped and waits to be
/// resumed (0.9.5), newest first and cut to [`MAX_ITEMS`], with how many there are in all.
/// `GET /api/glance` reads the same list, so the island and the nudge agree.
pub(super) async fn waiting(state: &AppState, space: &SpaceId) -> Result<(Vec<Value>, i64), crate::db::DbError> {
    let approvals = waiting_approvals(state, space).await?;
    let watch = crate::server::missions::watch::waiting_items(state, space).await;
    waiting_with(state, space, approvals, watch).await
}

/// [`waiting`] with the space's approvals and watch items already read (the glance reads them
/// for every space in one pass).
pub(super) async fn waiting_with(state: &AppState, space: &SpaceId, approvals: Vec<AwayRow>, watch: Vec<Value>) -> Result<(Vec<Value>, i64), crate::db::DbError> {
    let mut rows = state.db.away_open_questions(space, MAX_ITEMS).await?;
    let count = state.db.away_count_open_questions(space).await? + approvals.len() as i64;
    rows.extend(approvals);
    rows.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
    rows.truncate(MAX_ITEMS as usize);
    let stopped = stopped_items(state, space).await;
    let count = count + watch.len() as i64 + stopped.len() as i64;
    let mut waiting: Vec<Value> = rows.into_iter().map(item).collect();
    waiting.extend(watch);
    waiting.extend(stopped);
    waiting.sort_by(|a, b| b["timestamp"].as_str().unwrap_or("").cmp(a["timestamp"].as_str().unwrap_or("")));
    waiting.truncate(MAX_ITEMS as usize);
    annotate(state, &mut waiting).await;
    Ok((waiting, count))
}

#[derive(Debug, Deserialize)]
pub struct AwayQuery {
    pub since: Option<String>,
}

/// `GET /api/away?since=<RFC 3339>`: `since` defaults to the last-seen marker, else 7 days ago.
/// Waiting items and stuck delegations are listed whatever `since` is: they still need you.
pub async fn get(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<AwayQuery>) -> Response {
    let now = Utc::now();
    let Ok(seen) = last_seen(&state, &space).await else {
        return internal();
    };
    let default_since = seen.unwrap_or(now - Duration::days(DEFAULT_LOOKBACK_DAYS));
    let since = match q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => default_since,
        Some(s) => match DateTime::parse_from_rfc3339(s) {
            Ok(t) => t.with_timezone(&Utc),
            Err(_) => return error(StatusCode::BAD_REQUEST, "since must be an RFC 3339 time, e.g. 2026-10-17T09:00:00Z"),
        },
    };
    let stuck_before = now - Duration::minutes(STUCK_AFTER_MINUTES);

    let result = async {
        let (waiting, waiting_count) = waiting(&state, &space).await?;
        let finished = state.db.away_finished(&space, since, MAX_ITEMS).await?;
        let failed = state.db.away_failed(&space, since, stuck_before, MAX_ITEMS).await?;
        // The badge counts from the marker, whatever this page asked for
        let (unseen_finished, unseen_failed) = state.db.away_counts(&space, default_since, stuck_before).await?;
        Ok::<_, crate::db::DbError>((waiting, finished, failed, waiting_count, unseen_finished, unseen_failed))
    }
    .await;
    let Ok((waiting, finished, failed, waiting_count, unseen_finished, unseen_failed)) = result else {
        return internal();
    };
    let mut finished: Vec<Value> = finished.into_iter().map(item).collect();
    let mut failed: Vec<Value> = failed.into_iter().map(item).collect();
    annotate(&state, &mut finished).await;
    annotate(&state, &mut failed).await;
    Json(json!({
        "since": rfc3339(since),
        "last_seen": seen.map(rfc3339),
        "now": rfc3339(now),
        "stuck_after_minutes": STUCK_AFTER_MINUTES,
        "waiting": waiting,
        "finished": finished,
        "failed": failed,
        "unseen": {
            "waiting": waiting_count,
            "finished": unseen_finished,
            "failed": unseen_failed,
            "total": waiting_count + unseen_finished + unseen_failed,
        },
    }))
    .into_response()
}

/// `POST /api/away/seen?space=`: the user has looked at that space; returns the stored marker.
pub async fn seen(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let now = Utc::now();
    if state.db.set_setting(&seen_key(&space), &rfc3339(now)).await.is_err() {
        return internal();
    }
    Json(json!({ "last_seen": rfc3339(now) })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 22i: every row says what the person does, and only stops and failed or stuck work can be
    /// dismissed (the rest has an answer that tells the bot).
    #[test]
    fn items_say_what_is_needed_and_whether_they_can_be_dismissed() {
        let cases = [
            (json!({"kind": "approval", "status": "pending"}), Some("decide"), false),
            (json!({"kind": "watch", "status": "open"}), Some("decide"), false),
            (json!({"kind": "question", "status": "open"}), Some("answer"), false),
            (json!({"kind": "stopped", "status": "open", "human_step_id": "h1", "retry": null}), Some("browser"), false),
            (json!({"kind": "stopped", "status": "open", "retry": null}), Some("retry"), true),
            (json!({"kind": "stopped", "status": "open", "retry": {"type": "continue"}}), Some("continue"), true),
            (json!({"kind": "stopped", "status": "open", "retry": {"type": "approval", "approval_id": "a1"}}), Some("ask_again"), true),
            (json!({"kind": "delegation", "status": "failed", "task_id": "t1"}), Some("retry"), true),
            (json!({"kind": "delegation", "status": "interrupted", "task_id": "t1"}), Some("retry"), true),
            (json!({"kind": "delegation", "status": "failed"}), None, true),
            (json!({"kind": "delegation", "status": "interrupted"}), None, true),
            (json!({"kind": "delegation", "status": "stuck"}), Some("decide"), true),
            (json!({"kind": "routine_run", "status": "error"}), Some("retry"), true),
            (json!({"kind": "delegation", "status": "completed"}), None, false),
            (json!({"kind": "routine_run", "status": "success"}), None, false),
        ];
        for (v, action, can_dismiss) in cases {
            assert_eq!(action_of(&v), action, "{v}");
            assert_eq!(dismissable(&v), can_dismiss, "{v}");
        }
    }

    /// 22i: a failed hand-off offers Retry only with a failed board task to rerun, and carries its
    /// `task_id`; with the board off (no task) or a task that isn't failed it offers no Retry, so
    /// the row never has a button with nowhere to go. It stays dismissable either way.
    #[tokio::test]
    async fn a_failed_hand_off_offers_retry_only_with_a_board_task_to_rerun() {
        use crate::db::missions::NewTask;
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::db::Database::init(tmp.path()).await.unwrap();
        let row = |id: &str| json!({"kind": "delegation", "id": id, "status": "failed", "link": {"thread_id": "t"}});
        // No board task (the board is off by default)
        let mut v = row("d1");
        add_hand_off_task(&db, &mut v).await;
        assert!(v.get("task_id").is_none(), "{v}");
        assert_eq!(action_of(&v), None);
        assert!(dismissable(&v));
        // A failed board task for it: Retry reruns that task
        let task = |state: &str, source: &str| NewTask {
            title: "Hand-off".into(),
            state: state.into(),
            priority: 1,
            max_attempts: 3,
            source_kind: "delegation".into(),
            source_id: Some(source.into()),
            created_by: "kwant".into(),
            ..Default::default()
        };
        let t = db.create_task(&SpaceId::default_space(), &task("failed", "d1")).await.unwrap();
        let mut v = row("d1");
        add_hand_off_task(&db, &mut v).await;
        assert_eq!(v["task_id"], json!(t.id));
        assert_eq!(action_of(&v), Some("retry"));
        // A task that isn't failed can't be retried there, so no Retry either
        db.create_task(&SpaceId::default_space(), &task("blocked", "d2")).await.unwrap();
        let mut v = row("d2");
        add_hand_off_task(&db, &mut v).await;
        assert!(v.get("task_id").is_none(), "{v}");
        assert_eq!(action_of(&v), None);
        // Other rows are left alone
        let mut v = json!({"kind": "routine_run", "id": "d1", "status": "error"});
        add_hand_off_task(&db, &mut v).await;
        assert!(v.get("task_id").is_none());
    }

    #[test]
    fn summaries_are_one_short_masked_line() {
        assert_eq!(summarize("\n\n## Done\nmore"), "Done");
        let s = summarize(r#"401 from provider: {"key":"sk-ant-api03-abcdefghijklmnop1234"} key=xai-ABCDEFGH12345678abcd"#);
        assert!(!s.contains("sk-ant") && !s.contains("xai-"), "{s}");
        assert!(s.starts_with("401 from provider:"));
        let long = summarize(&"word ".repeat(100));
        assert_eq!(long.chars().count(), SUMMARY_CHARS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn approval_summaries_hide_sensitive_arguments() {
        let p = r#"{"tool":"send","server":"mail","arguments":{"to":"a@b.c","api_token":"plain-but-hidden"}}"#;
        assert_eq!(approval_title("mcp__mail__send", p), "Use send (mail)");
        let s = approval_summary(p);
        assert!(s.contains("to: a@b.c") && !s.contains("plain-but-hidden"), "{s}");
    }

    #[test]
    fn approval_cards_show_the_file_path_but_mask_values() {
        let p = r#"{"tool":"Read","arguments":{"file_path":"/tmp/rg4/proj/.saved_passwords.json","content":"key sk-ant-api03-abcdefghijklmnop1234","password":"DUMMY-PW-1"}}"#;
        let s = approval_summary(p);
        assert!(s.contains("file_path: /tmp/rg4/proj/.saved_passwords.json"), "{s}");
        assert!(!s.contains("sk-ant") && !s.contains("DUMMY-PW-1"), "{s}");
        let d = masked_details(serde_json::from_str(p).unwrap());
        assert_eq!(d["arguments"]["file_path"], "/tmp/rg4/proj/.saved_passwords.json");
        assert_eq!(d["arguments"]["password"], "[redacted]");
        assert!(!d["arguments"]["content"].as_str().unwrap().contains("sk-ant"));
        let url = masked_details(json!({"arguments": {"path": "https://bob:hunter2@host/x"}}));
        assert_eq!(url["arguments"]["path"], "https://bob:[redacted]@host/x");
    }

    #[test]
    fn a_usage_limit_line_is_never_the_summary() {
        let row = |body: &str| AwayRow {
            kind: "task".into(),
            id: "t1".into(),
            kwant_id: "k1".into(),
            kwant_name: Some("Architect".into()),
            title: "Plan the release".into(),
            status: "completed".into(),
            body: Some(body.into()),
            at: chrono::Utc::now(),
            thread_id: None,
            channel_id: None,
            direct_kwant_id: None,
            choices: None,
        };
        let limit = item(row("You've hit your session limit · resets 3am (Europe/Berlin)"));
        assert_eq!(limit["summary"], "Finished");
        let real = item(row("Plan is ready: three steps."));
        assert_eq!(real["summary"], "Plan is ready: three steps.");
    }

    #[test]
    fn summaries_mask_url_passwords_and_bearer_tokens() {
        let s = summarize("failed: postgres://bob:hunter2@db:5432/x with Authorization: Bearer abc123");
        assert_eq!(s, "failed: postgres://bob:[redacted]@db:5432/x with Authorization: Bearer [redacted]");
    }
}
