//! `GET /api/tasks/receipts.csv` (roadmap Next 7): a space's closed tasks as one CSV an auditor
//! can take away. One row per task: what it was, who verified it, how it was checked, the
//! verifier's latest verdict, its evidence and the tokens it spent. Read-only, space-scoped and
//! open like `GET /api/tasks`; no table of its own.

use super::routes::{bad, internal};
use crate::db::goals::CLOSED_TASK_STATES;
use crate::db::receipts::{ReceiptEvidence, ReceiptParts, ReceiptTask};
use crate::db::verifier::TaskVerdict;
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Query, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// At most this many rows; past it the response says `X-Kwant-Truncated: true`.
pub const MAX_ROWS: usize = 10_000;
/// The default window: tasks finished in the last 30 days (UTC), today included.
pub const DEFAULT_DAYS: i64 = 30;
/// How many tasks one verdict query covers.
const PAGE: usize = 500;

pub const HEADER: [&str; 17] = [
    "task_id",
    "title",
    "state",
    "mission",
    "owner_bot",
    "source",
    "created_at",
    "finished_at",
    "verified_by",
    "how_checked",
    "checks_passed",
    "checks_total",
    "verdict",
    "verdict_summary",
    "couldnt_verify",
    "evidence",
    "tokens",
];

#[derive(Deserialize)]
pub struct ReceiptQuery {
    from: Option<String>,
    to: Option<String>,
    state: Option<String>,
}

/// One CSV cell (RFC 4180), with the formula guard: a cell that starts with `=`, `+`, `-`, `@`,
/// a tab or a CR gets a leading `'`, so a spreadsheet shows it as text instead of running it
/// (titles and labels come from bots and the web).
pub fn cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// One CSV line, CRLF-terminated.
pub fn line<S: AsRef<str>>(cells: &[S]) -> String {
    let mut out = cells.iter().map(|c| cell(c.as_ref())).collect::<Vec<_>>().join(",");
    out.push_str("\r\n");
    out
}

fn iso(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn date(s: Option<&str>, name: &str) -> Result<Option<NaiveDate>, String> {
    match s.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Some).map_err(|_| format!("{name} must be a date like 2026-11-13")),
    }
}

/// `(states, from, to)` from the query: `from` and `to` are whole UTC days, both included.
type Window = (Vec<&'static str>, NaiveDate, NaiveDate);

pub fn window(q: &ReceiptQuery, today: NaiveDate) -> Result<Window, String> {
    let states: Vec<&'static str> = match q.state.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("closed") {
        "closed" => CLOSED_TASK_STATES.to_vec(),
        s => match CLOSED_TASK_STATES.iter().find(|c| **c == s) {
            Some(c) => vec![*c],
            None => return Err("unknown state: use verified, failed, cancelled or closed".into()),
        },
    };
    let to = date(q.to.as_deref(), "to")?.unwrap_or(today);
    let from = date(q.from.as_deref(), "from")?.unwrap_or(to - Duration::days(DEFAULT_DAYS));
    if from > to {
        return Err("from is after to".into());
    }
    Ok((states, from, to))
}

fn start_of(d: NaiveDate) -> DateTime<Utc> {
    d.and_hms_opt(0, 0, 0).expect("midnight").and_utc()
}

/// Who verified it, in words: the person, kwant's own checks, or the bot's name.
fn verifier_name(actor: &str, names: &HashMap<String, String>) -> String {
    match actor {
        "person" => "person".into(),
        "kwant" => "kwant check".into(),
        id => names.get(id).cloned().unwrap_or_else(|| id.to_string()),
    }
}

fn couldnt_verify(v: &Value) -> String {
    let items = v.as_array().map(Vec::as_slice).unwrap_or_default();
    items
        .iter()
        .filter_map(|i| match i {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => {
                let what = o.get("what").and_then(Value::as_str).unwrap_or("").trim();
                let why = o.get("why").and_then(Value::as_str).unwrap_or("").trim();
                match (what.is_empty(), why.is_empty()) {
                    (true, true) => None,
                    (false, true) => Some(what.to_string()),
                    (true, false) => Some(why.to_string()),
                    (false, false) => Some(format!("{what} ({why})")),
                }
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn evidence(items: Option<&Vec<ReceiptEvidence>>) -> String {
    items
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            let at = e.uri.as_deref().or(e.path.as_deref()).map(str::trim).filter(|s| !s.is_empty());
            let sha = e.sha256.as_deref().map(|s| s.chars().take(12).collect::<String>()).filter(|s| !s.is_empty());
            let inner: Vec<String> = at.map(str::to_string).into_iter().chain(sha).collect();
            if inner.is_empty() {
                e.label.clone()
            } else {
                format!("{} ({})", e.label, inner.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// One receipt row, in [`HEADER`] order.
pub fn row(t: &ReceiptTask, parts: &ReceiptParts, verdict: Option<&TaskVerdict>) -> Vec<String> {
    let checks = parts.checks.get(&t.id).map(Vec::as_slice).unwrap_or_default();
    let mut kinds: Vec<&str> = Vec::new();
    for (kind, _) in checks {
        if !kinds.contains(&kind.as_str()) {
            kinds.push(kind);
        }
    }
    let passed = checks.iter().filter(|(_, status)| status == "passed").count();
    let verdict_text = verdict.map(|v| match &v.overturned_to {
        Some(to) => format!("{} (overturned: {to})", v.verdict),
        None => v.verdict.clone(),
    });
    vec![
        t.id.clone(),
        t.title.clone(),
        t.state.clone(),
        t.mission_id.as_ref().map(|m| parts.mission_titles.get(m).cloned().unwrap_or_default()).unwrap_or_default(),
        t.owner_kwant_id.as_ref().map(|id| parts.bot_names.get(id).cloned().unwrap_or_else(|| id.clone())).unwrap_or_default(),
        t.source_kind.clone(),
        iso(&t.created_at),
        t.finished_at.as_ref().map(iso).unwrap_or_default(),
        parts.verified_by.get(&t.id).map(|a| verifier_name(a, &parts.bot_names)).unwrap_or_default(),
        kinds.join("; "),
        passed.to_string(),
        checks.len().to_string(),
        verdict_text.unwrap_or_default(),
        verdict.map(|v| v.summary.clone()).unwrap_or_default(),
        verdict.map(|v| couldnt_verify(&v.couldnt_verify)).unwrap_or_default(),
        evidence(parts.evidence.get(&t.id)),
        t.spent_tokens.to_string(),
    ]
}

/// A space id as it may sit in a file name.
fn file_part(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(64).collect()
}

pub async fn receipts_csv(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ReceiptQuery>) -> Response {
    let (states, from, to) = match window(&q, Utc::now().date_naive()) {
        Ok(w) => w,
        Err(msg) => return bad(msg),
    };
    let mut tasks = match state.db.receipt_tasks(&space, &states, start_of(from), start_of(to + Duration::days(1)), MAX_ROWS as i64 + 1).await {
        Ok(t) => t,
        Err(e) => return internal(e),
    };
    let truncated = tasks.len() > MAX_ROWS;
    tasks.truncate(MAX_ROWS);
    let parts = match state.db.receipt_parts(&tasks).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    let mut verdicts: HashMap<String, TaskVerdict> = HashMap::new();
    for page in tasks.chunks(PAGE) {
        let ids: Vec<String> = page.iter().map(|t| t.id.clone()).collect();
        match state.db.latest_verdicts(&ids).await {
            Ok(v) => verdicts.extend(v),
            Err(e) => return internal(e),
        }
    }
    let mut body = line(&HEADER);
    for t in &tasks {
        body.push_str(&line(&row(t, &parts, verdicts.get(&t.id))));
    }
    let name = format!("attachment; filename=\"kwant-receipts-{}-{}.csv\"", file_part(space.as_str()), to.format("%Y-%m-%d"));
    let mut resp = body.into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&name) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    if truncated {
        h.insert("x-kwant-truncated", HeaderValue::from_static("true"));
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(from: Option<&str>, to: Option<&str>, state: Option<&str>) -> ReceiptQuery {
        ReceiptQuery { from: from.map(Into::into), to: to.map(Into::into), state: state.map(Into::into) }
    }

    #[test]
    fn cells_are_quoted_per_rfc_4180() {
        assert_eq!(cell("plain"), "plain");
        assert_eq!(cell(""), "");
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("two\nlines"), "\"two\nlines\"");
        assert_eq!(cell("cr\rhere"), "\"cr\rhere\"");
        assert_eq!(line(&["a", "b,c", ""]), "a,\"b,c\",\r\n");
    }

    #[test]
    fn formula_cells_get_a_leading_quote() {
        assert_eq!(cell("=HYPERLINK(\"http://x\")"), "\"'=HYPERLINK(\"\"http://x\"\")\"");
        assert_eq!(cell("=1+1"), "'=1+1");
        assert_eq!(cell("+1"), "'+1");
        assert_eq!(cell("-2"), "'-2");
        assert_eq!(cell("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(cell("\tx"), "'\tx");
        assert_eq!(cell("\rx"), "\"'\rx\"");
        // Only the first character counts
        assert_eq!(cell("a=b"), "a=b");
        assert_eq!(cell("2026-11-13T00:00:00Z"), "2026-11-13T00:00:00Z");
    }

    #[test]
    fn the_window_defaults_to_30_days_of_closed_tasks() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        let (states, from, to) = window(&q(None, None, None), today).unwrap();
        assert_eq!(states, CLOSED_TASK_STATES.to_vec());
        assert_eq!((from, to), (NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(), today));
        let (states, _, _) = window(&q(None, None, Some("failed")), today).unwrap();
        assert_eq!(states, vec!["failed"]);
        assert!(window(&q(None, None, Some("review")), today).unwrap_err().contains("unknown state"));
        assert!(window(&q(Some("13/11/2026"), None, None), today).unwrap_err().contains("from must be a date"));
        assert!(window(&q(None, Some("2026-13-01"), None), today).unwrap_err().contains("to must be a date"));
        assert_eq!(window(&q(Some("2026-11-14"), Some("2026-11-13"), None), today).unwrap_err(), "from is after to");
    }

    #[test]
    fn couldnt_verify_items_read_as_what_and_why() {
        let v = serde_json::json!([{"what": "the login page", "why": "no browser"}, "a plain one", {"what": "", "why": ""}]);
        assert_eq!(couldnt_verify(&v), "the login page (no browser); a plain one");
        assert_eq!(couldnt_verify(&Value::Null), "");
    }

    #[test]
    fn file_names_keep_only_safe_characters() {
        assert_eq!(file_part("spc_default"), "spc_default");
        assert_eq!(file_part("a\"b/../c\r\n"), "abc");
    }
}
