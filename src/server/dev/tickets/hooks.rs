//! The three calls pickup makes into the brief turn (api.md §1.3). Signatures are Juno's; the
//! bodies here are stubs that only record a `dev_ticket_events` row. Vega's branch replaces the
//! bodies (take Vega's at merge).

use super::source::Comment;
use crate::server::AppState;

/// Which ticket, on which site, for which team (`team_id` is "" when the site has none set).
#[derive(Debug, Clone, PartialEq)]
pub struct TicketRef {
    pub site_id: String,
    pub key: String,
    pub team_id: String,
}

/// Why pickup stopped a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    TakenBack,
    Closed,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Why::TakenBack => "taken_back",
            Why::Closed => "closed",
        }
    }
}

/// The ticket's turn has come (newly matched and a slot is free, or Start on a Waiting card, or it
/// matches again after `TakenBack`). The row is `picked_up` when this is called. Resume vs fresh
/// (a brief or mission already exists) is Vega's call.
pub async fn start(state: &AppState, space_id: &str, t: TicketRef) {
    let _ = state.db.add_dev_ticket_event(space_id, &t.key, "hook", Some("start")).await;
}

/// A new comment by the ticket's reporter, posted after the Ask on Jira comment, on a row in
/// `needs_answer`. Called once per such comment, oldest first. `reply.body` is untrusted text.
pub async fn reporter_replied(state: &AppState, space_id: &str, key: &str, asked_comment_id: &str, reply: &Comment) {
    let note = format!("reporter_replied: comment {} after {asked_comment_id}", reply.id);
    let _ = state.db.add_dev_ticket_event(space_id, key, "hook", Some(&note)).await;
}

/// The person deleted the site's connection (its `dev_tickets` and `dev_ticket_events` rows are
/// already gone). Vega's body forgets the site's briefs (`brief::forget_site`, brief.md §10).
pub async fn site_deleted(_state: &AppState, _space_id: &str, _site_id: &str) {}

/// Unassigned / label removed (`TakenBack`), or Done / deleted (`Closed`). The row is already in
/// `taken_back` or `closed` when this is called.
pub async fn stopped(state: &AppState, space_id: &str, key: &str, why: Why) {
    let _ = state.db.add_dev_ticket_event(space_id, key, "hook", Some(&format!("stopped: {}", why.as_str()))).await;
}

/// What the brief turn knows about a ticket's work (api-decisions 11): the mission it started, and
/// the Matches the ticket verdict (`met`, `not_met` or `unclear`; anything else reads as `null`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkSeam {
    pub mission_id: Option<String>,
    pub matches: Option<String>,
}

/// Read by `work::rows_with_work` for every ticket row it answers, so keep it to one indexed read.
/// Vega's body reads `dev_ticket_briefs` (`mission_id`, and the verdict's word from
/// `review.matches.verdict`). The stub knows no mission.
pub async fn work_seam(_state: &AppState, _space_id: &str, _key: &str) -> WorkSeam {
    WorkSeam::default()
}

/// Vega's routes join the Jira route table here (api-decisions split: `POST tickets/:key/answer`
/// and `PUT tickets/:key/brief`), so nothing is mounted twice and `server/mod.rs` stays as is.
/// Paths are the full `/api/spaces/:id/dev/tickets/:key/…`; every handler is person-only like
/// `routes::guard`. The stub mounts nothing.
pub fn mount(r: axum::Router<AppState>) -> axum::Router<AppState> {
    r
}
