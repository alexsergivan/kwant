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

/// Most items per list.
pub const MAX_ITEMS: i64 = 50;
/// A delegation still running this long after it started counts as stuck.
pub const STUCK_AFTER_MINUTES: i64 = 30;
/// How far back the page looks when the user has never marked it seen.
pub const DEFAULT_LOOKBACK_DAYS: i64 = 7;
pub const SUMMARY_CHARS: usize = 200;
const TITLE_CHARS: usize = 120;
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

fn rfc3339(t: DateTime<Utc>) -> String {
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
fn mask_words(text: &str) -> String {
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
fn approval_summary(parameters: &str) -> String {
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

fn item(row: AwayRow) -> Value {
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
            let summary = row.body.as_deref().map(summarize).filter(|s| !s.is_empty());
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

async fn last_seen(state: &AppState, space: &SpaceId) -> Result<Option<DateTime<Utc>>, crate::db::DbError> {
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
    let waiting = state.db.away_count_open_questions(space).await? + waiting_approvals(state, space).await?.len() as i64;
    let (finished, failed) = state.db.away_counts(space, since, now - Duration::minutes(STUCK_AFTER_MINUTES)).await?;
    Ok(waiting + finished + failed)
}

/// The space's approvals the broker still waits on, each linked to the chat its turn runs in.
async fn waiting_approvals(state: &AppState, space: &SpaceId) -> Result<Vec<AwayRow>, crate::db::DbError> {
    let live = state.broker.pending_ids().await;
    let mut rows = state.db.away_pending_approvals(&live).await?;
    let mut mine = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        let of = state.db.space_of_kwant(&row.kwant_id).await?.unwrap_or_else(SpaceId::default_space);
        if &of == space {
            mine.push(row);
        }
    }
    let mut rows = mine;
    let turns = state.turns.active_turns();
    // turn_id isn't in the row; look it up by ticket
    for row in &mut rows {
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
    }
    Ok(rows)
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
        let mut waiting = state.db.away_open_questions(&space, MAX_ITEMS).await?;
        let approvals = waiting_approvals(&state, &space).await?;
        let waiting_count = state.db.away_count_open_questions(&space).await? + approvals.len() as i64;
        waiting.extend(approvals);
        waiting.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
        waiting.truncate(MAX_ITEMS as usize);
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
    Json(json!({
        "since": rfc3339(since),
        "last_seen": seen.map(rfc3339),
        "now": rfc3339(now),
        "stuck_after_minutes": STUCK_AFTER_MINUTES,
        "waiting": waiting.into_iter().map(item).collect::<Vec<_>>(),
        "finished": finished.into_iter().map(item).collect::<Vec<_>>(),
        "failed": failed.into_iter().map(item).collect::<Vec<_>>(),
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
    fn summaries_mask_url_passwords_and_bearer_tokens() {
        let s = summarize("failed: postgres://bob:hunter2@db:5432/x with Authorization: Bearer abc123");
        assert_eq!(s, "failed: postgres://bob:[redacted]@db:5432/x with Authorization: Bearer [redacted]");
    }
}
