//! Ticket row states with their side effects: [`set_state`] wraps the one DB writer
//! (`db.set_dev_ticket_state`) to send `dev_ticket {space_id, key, state}` and, when the person
//! ticked it (J9), to post the one work-started comment. [`on_card`] moves a ticket to `in_review`
//! or `landed` when Dev mode writes a card for a PR whose branch or title carries its key (J11).

use crate::db::dev_jira::{self, TicketRow};
use crate::db::dev_mode::DevCardRow;
use crate::db::DbError;
use crate::drivers::events::RuntimeEvent;
use crate::server::dev::atlassian;
use crate::server::AppState;

/// The first Jira key in a PR's head branch (`TRAIL-101-receipt-link`), else in its title
/// (`TRAIL-101: Add a receipt link`). A key is `[A-Z][A-Z0-9_]+-[0-9]+` standing alone: not
/// inside a longer word, so `XTRAIL-1` and `TRAIL-1x` aren't `TRAIL-1`.
pub fn key_in(branch: Option<&str>, title: &str) -> Option<String> {
    keys_in(branch, title).into_iter().next()
}

/// Every key-shaped word in the branch, then the title, without repeats. `UTF-8` is key-shaped
/// too, so callers keep the first one that is a ticket in the space.
pub fn keys_in(branch: Option<&str>, title: &str) -> Vec<String> {
    let mut out = Vec::new();
    for k in branch.map(all_keys).unwrap_or_default().into_iter().chain(all_keys(title)) {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

fn all_keys(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_uppercase() && (i == 0 || !word(b[i - 1])) {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j].is_ascii_digit() || b[j] == b'_') {
                j += 1;
            }
            if j - i >= 2 && j < b.len() && b[j] == b'-' {
                let mut k = j + 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 && k - j - 1 <= 12 && (k == b.len() || !word(b[k])) {
                    let key = &s[i..k];
                    if super::source::valid_key(key) && out.len() < 10 {
                        out.push(key.to_string());
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Moves a ticket and sends `dev_ticket`. Ok(false): not allowed from its current state (or no such
/// ticket); nothing is sent. Vega calls this rather than the DB fn, so the stream hears every move.
pub async fn set_state(state: &AppState, space_id: &str, key: &str, to: &str, detail: Option<&str>) -> Result<bool, DbError> {
    let moved = state.db.set_dev_ticket_state_with(space_id, key, to, detail).await?;
    if moved {
        notify(state, space_id, key, to);
        if to == dev_jira::WORKING {
            progress(state, space_id, key, Progress::Work).await;
        }
    }
    Ok(moved)
}

/// The person's Stop, Skip, Close or Retry: the DB move, then `dev_ticket`. Ok(None): not from
/// where the ticket is (or no such ticket); nothing is sent. Never writes to Jira.
pub async fn person_move(state: &AppState, space_id: &str, key: &str, mv: dev_jira::PersonMove) -> Result<Option<String>, DbError> {
    let from = state.db.person_move_dev_ticket(space_id, key, mv).await?;
    if from.is_some() {
        notify(state, space_id, key, mv.to());
    }
    Ok(from)
}

/// Sends `dev_ticket {space_id, key, state}`.
pub fn notify(state: &AppState, space_id: &str, key: &str, to: &str) {
    let _ = state.event_sender.send(RuntimeEvent::DevTicket { space_id: space_id.into(), key: key.into(), state: to.into() });
}

/// J9's two comments.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Work,
    Pr(Option<String>),
}

/// The J9 text. Plain words, no mention, no bot's text: only kwant's own sentence and a PR link.
pub fn progress_text(p: &Progress) -> String {
    match p {
        Progress::Work => "kwant started work on this ticket. The pull request will carry the ticket key.".into(),
        Progress::Pr(Some(url)) => format!("A pull request for this ticket is ready for review: {url}"),
        Progress::Pr(None) => "A pull request for this ticket is ready for review.".into(),
    }
}

/// Posts one J9 comment when the site's setting is on, once per ticket per kind (claimed in the
/// row first, so a retry or a second card never posts twice). A failure is logged on the ticket,
/// never retried.
pub async fn progress(state: &AppState, space_id: &str, key: &str, p: Progress) {
    let Ok(Some(site)) = state.db.dev_jira_site(space_id).await else { return };
    if !site.comment_progress || !site.on_ {
        return;
    }
    let which = if p == Progress::Work { "work" } else { "pr" };
    if !matches!(state.db.claim_dev_ticket_comment(space_id, key, which).await, Ok(true)) {
        return;
    }
    let note = match atlassian::writer_for(state, &site).await {
        Ok(w) => match w.add_comment(key, &progress_text(&p)).await {
            Ok(c) => format!("posted the {which} comment {}", c.id),
            Err(e) => format!("couldn't post the {which} comment: {e}"),
        },
        Err(e) => format!("couldn't post the {which} comment: {e}"),
    };
    let _ = state.db.add_dev_ticket_event(space_id, key, "comment", Some(&note)).await;
}

/// The key a Dev mode card carries, when it is a live review or a landed card. Cheap: no DB.
pub fn card_keys(row: &DevCardRow) -> Option<(Vec<String>, Option<String>)> {
    let live = match row.kind.as_str() {
        "review" => !matches!(row.status.as_str(), "stale" | "failed"),
        "landed" => true,
        _ => false,
    };
    if !live {
        return None;
    }
    let facts: serde_json::Value = serde_json::from_str(&row.facts).ok()?;
    let title = facts.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let keys = keys_in(facts.get("head_branch").and_then(|v| v.as_str()), title);
    if keys.is_empty() {
        return None;
    }
    let pr_url = facts.pointer("/links/pr").and_then(|v| v.as_str()).filter(|u| u.starts_with("https://")).map(str::to_string);
    Some((keys, pr_url))
}

/// Dev mode wrote or changed a card whose PR carries `keys`; the first that is a ticket here counts: a review moves the ticket to
/// `in_review` (and posts the PR-ready comment when the person ticked J9), a landed card to
/// `landed`. Nothing happens unless this space has a Jira site and a row for that key.
pub async fn on_card(state: AppState, row: DevCardRow, keys: Vec<String>, pr_url: Option<String>) {
    let Ok(Some(_site)) = state.db.dev_jira_site(&row.space_id).await else { return };
    let mut found = None;
    for k in &keys {
        if let Ok(Some(t)) = state.db.dev_ticket(&row.space_id, k).await {
            found = Some(t);
            break;
        }
    }
    let Some(ticket) = found else { return };
    let to = if row.kind == "landed" { dev_jira::LANDED } else { dev_jira::IN_REVIEW };
    if let Ok(true) = set_state(&state, &row.space_id, &ticket.key, to, Some(&format!("card {}", row.id))).await {
        if to == dev_jira::IN_REVIEW {
            progress(&state, &row.space_id, &ticket.key, Progress::Pr(pr_url)).await;
        }
    }
}

/// The site row's `team_id` for a ticket, as the hooks want it.
pub fn team_of(t: &TicketRow) -> String {
    t.team_id.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reading_checklist_event_has_the_agreed_wire_shape() {
        // api.md 1.7: Vega's `tools::progress` is the only sender; this branch only defines the event.
        let ev = RuntimeEvent::DevTicketProgress { space_id: "s".into(), key: "K-1".into(), step: "links".into(), state: "skipped".into(), detail: "none".into() };
        let v = serde_json::to_value(ev).unwrap();
        assert_eq!(v["type"], "dev_ticket_progress");
        assert_eq!(v["data"], serde_json::json!({"space_id": "s", "key": "K-1", "step": "links", "state": "skipped", "detail": "none"}));
    }

    #[test]
    fn keys_come_from_the_branch_then_the_title() {
        assert_eq!(key_in(Some("TRAIL-101-receipt-link"), "x"), Some("TRAIL-101".into()));
        assert_eq!(key_in(Some("feature/receipt"), "TRAIL-102: Orders export"), Some("TRAIL-102".into()));
        assert_eq!(key_in(Some("fix-TRAIL-7"), ""), Some("TRAIL-7".into()));
        assert_eq!(key_in(None, "[TRAIL-9] thing"), Some("TRAIL-9".into()));
        assert_eq!(key_in(Some("A_B2-3"), ""), Some("A_B2-3".into()));
    }

    #[test]
    fn a_key_stands_alone() {
        assert_eq!(key_in(Some("XtRAIL-1"), "trail-1 and T-1 and TRAIL-01"), None);
        assert_eq!(key_in(None, "TRAIL-1x"), None);
        assert_eq!(key_in(None, "aTRAIL-1"), None);
        assert_eq!(key_in(None, "TRAIL-"), None);
        assert_eq!(key_in(None, "TRAIL-1234567890123"), None);
        assert_eq!(keys_in(Some("TRAIL-12-utf"), "Fix UTF-8 in TRAIL-12"), vec!["TRAIL-12".to_string(), "UTF-8".to_string()]);
    }

    #[test]
    fn progress_comments_are_kwants_own_words() {
        assert!(progress_text(&Progress::Work).starts_with("kwant started work"));
        assert_eq!(progress_text(&Progress::Pr(Some("https://github.com/o/r/pull/9".into()))), "A pull request for this ticket is ready for review: https://github.com/o/r/pull/9");
    }
}
