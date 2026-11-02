//! `verification`: a mission task's own outcome checks, attached to its "Verified: ..." / "Failed:
//! ..." report line (`crate::server::missions::task_chat_line`). Every row, label and count comes
//! from the stored checks, never from the reply's text, so the block is `kwant_data` provenance
//! and needs no reply-containment check. It is a snapshot taken when the report is posted; each
//! check carries the time it finished.

use sqlx::SqliteConnection;

use crate::db::missions::OutcomeCheck;
use crate::db::models::MessageExtra;
use crate::server::missions::autofix::{redact, redact_value};

use super::{Block, Check, Provenance, Source, Verification, VerificationTarget};

const MAX_CHECKS: usize = 20;
const MAX_TEXT: usize = 300;
const MAX_LABEL: usize = 120;

/// The verification block for a message whose `meta` is a task report
/// (`{"kind": "mission", "task_id": ...}`) and whose task has 1 to 20 outcome checks; `None` for
/// every other message (including a plain mission-chat line). A database error just means no
/// block.
pub async fn candidate(conn: &mut SqliteConnection, extra: &MessageExtra) -> Option<Block> {
    let meta = extra.meta.as_ref()?;
    if meta.get("kind").and_then(|v| v.as_str()) != Some("mission") {
        return None;
    }
    let task_id = meta.get("task_id").and_then(|v| v.as_str())?;
    let title: String = match sqlx::query_scalar("SELECT title FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&mut *conn).await {
        Ok(Some(t)) => t,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!("smart results: couldn't read task {task_id} for its verification: {e}");
            return None;
        }
    };
    let rows = match sqlx::query_as::<_, OutcomeCheck>("SELECT * FROM outcome_checks WHERE task_id = ? ORDER BY created_at, rowid")
        .bind(task_id)
        .fetch_all(&mut *conn)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("smart results: couldn't read the checks of task {task_id}: {e}");
            return None;
        }
    };
    from_rows(task_id, &title, &rows)
}

/// The block for `rows`, or `None` when there are none or more than 20.
pub(super) fn from_rows(task_id: &str, title: &str, rows: &[OutcomeCheck]) -> Option<Block> {
    if rows.is_empty() || rows.len() > MAX_CHECKS {
        return None;
    }
    let checks: Vec<Check> = rows
        .iter()
        .map(|c| Check {
            id: c.id.clone(),
            label: label(c),
            kind: c.kind.clone(),
            status: status(&c.status).into(),
            excerpt: c.output_excerpt.as_deref().map(|e| safe(&redact(e), MAX_TEXT)).filter(|e| !e.trim().is_empty()),
            checked_at: c.finished_at.map(|t| t.to_rfc3339()),
        })
        .collect();
    let passed = checks.iter().filter(|c| c.status == "passed").count();
    Some(Block::Verification(Verification {
        id: "b0".into(),
        title: None,
        source: Source::slot(),
        provenance: Provenance::kwant_data(format!("task:{task_id}")),
        target: VerificationTarget { kind: "task".into(), id: task_id.into(), title: safe(title, MAX_TEXT) },
        summary: format!("{passed} of {} checks passed", checks.len()),
        checks,
    }))
}

/// A check's stored status in the contract's four words: anything that hasn't finished (waiting
/// for approval, queued, running) is `pending`, and is never counted as passed.
fn status(stored: &str) -> &'static str {
    match stored {
        "passed" => "passed",
        "failed" => "failed",
        "error" => "error",
        _ => "pending",
    }
}

/// "command cargo test", "http https://...", "file_exists path": the check's kind and its main
/// field, redacted.
fn label(c: &OutcomeCheck) -> String {
    let spec = redact_value(&c.spec_value());
    let main = ["command", "url", "path"].iter().find_map(|k| spec.get(k).and_then(|v| v.as_str())).unwrap_or("");
    safe(format!("{} {}", c.kind, main).trim(), MAX_LABEL)
}

/// `text` cut to `max` characters (with an ellipsis), angle brackets turned into look-alike quote
/// marks so a hostile excerpt can never read as markup, and control characters dropped.
pub(super) fn safe(text: &str, max: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '<' => '\u{2039}',
            '>' => '\u{203a}',
            c if c.is_control() && c != '\n' => ' ',
            c => c,
        })
        .collect();
    if cleaned.chars().count() <= max {
        return cleaned;
    }
    let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}
