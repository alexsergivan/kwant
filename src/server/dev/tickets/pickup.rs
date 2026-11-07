//! The pickup poller (jira-tickets J4, J5, J13, J14): every site's `poll_secs` (5 minutes by
//! default), or right away when a route asks ([`poll_soon`]), one search per site finds the
//! tickets handed over to kwant, and each tracked ticket is moved by what Jira says now.
//!
//! - **New** match: a `dev_tickets` row, `waiting` while the first poll after connecting runs
//!   (backfill: those wait for the person's Start, J5) and `picked_up` after it.
//! - **Still** matching: facts refreshed; a `taken_back` or `failed` ticket that matches again is
//!   `picked_up` again.
//! - **Missing** from the search: read on its own (search lags edits on Cloud, so missing alone
//!   proves nothing). Deleted or Done: `closed`. No longer matching the built rule: `taken_back`
//!   (with who did it, from the changelog). With Advanced JQL there is no local rule, so it takes
//!   [`CUSTOM_MISSES`] polls in a row.
//! - **Questions**: for a ticket in `needs_answer` with an Ask on Jira comment, each newer comment
//!   by the reporter (else the assignee) goes to `hooks::reporter_replied`, once.
//! - **Slots** (J13): at most [`SLOTS`] tickets are being read or worked at once per space; the
//!   queue is priority, then Jira's created time, then key. `hooks::start` is called once per turn.
//!
//! Nothing here runs unless the space has a Jira site that is on, and Dev mode is on. Errors are
//! kept on the site row for the status line: a 429 waits for its Retry-After, a 401 says the token
//! stopped working, and a site unreachable for [`UNREACHABLE_FAIL_HOURS`] fails its live tickets.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::hooks::{self, TicketRef, Why};
use super::source::{Issue, SourceError, TicketSource};
use super::store;
use crate::db::dev_jira::{self, JiraSiteRow, PollMark, TicketFacts, TicketRow};
use crate::server::dev::atlassian::{self, check, client::AtlassianClient, jira::MAX_SEARCH};
use crate::server::AppState;

/// Tickets read or worked at once per space (J13).
pub const SLOTS: usize = 3;
/// Tracked tickets missing from a search that one poll reads on their own.
pub const MAX_MISSING_CHECKS: usize = 20;
/// Polls in a row an Advanced-JQL ticket must be missing before it counts as taken back.
pub const CUSTOM_MISSES: i64 = 2;
/// After this long unreachable, the live tickets fail (the card says why).
pub const UNREACHABLE_FAIL_HOURS: i64 = 24;
/// How often the loop looks at which site is due.
pub const TICK: StdDuration = StdDuration::from_secs(30);
/// The fewest and most seconds between two polls of one site.
pub const MIN_POLL_SECS: i64 = 60;
pub const MAX_POLL_SECS: i64 = 86_400;
/// Comment threads read per poll (tickets waiting on a Jira answer).
pub const MAX_ANSWER_CHECKS: usize = 10;

#[derive(Default)]
struct Shared {
    soon: HashSet<String>,
    running: HashSet<String>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn wake() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

/// Polls `space_id`'s site on the loop's next turn (after a save, Start, or turn-on).
pub fn poll_soon(space_id: &str) {
    shared().lock().unwrap().soon.insert(space_id.to_string());
    wake().notify_one();
}

/// What one poll did (the `POST …/dev/jira/poll` answer and the tests' view).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PollReport {
    /// Tickets the search returned.
    pub matched: usize,
    /// New rows (`picked_up`, or `waiting` on the first poll).
    pub new: usize,
    pub waiting: usize,
    /// `taken_back` / `failed` rows picked up again.
    pub resumed: usize,
    pub taken_back: usize,
    pub closed: usize,
    pub failed: usize,
    /// `hooks::start` calls.
    pub started: usize,
    /// `hooks::reporter_replied` calls.
    pub replies: usize,
    /// Why nothing ran: `no_site`, `off`, `dev_mode_off`, `backing_off`, `busy`.
    pub skipped: Option<&'static str>,
    /// The error kept on the site row (shown on the status line).
    pub error: Option<String>,
}

/// Starts the loop. Each tick polls the sites that are due or were asked for.
pub fn spawn(state: AppState, shutdown: CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = wake().notified() => {}
                _ = tokio::time::sleep(TICK) => {}
            }
            let soon = std::mem::take(&mut shared().lock().unwrap().soon);
            let sites = match state.db.dev_jira_sites_on().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("jira: could not list sites: {e}");
                    continue;
                }
            };
            let now = Utc::now();
            for site in sites.into_iter().filter(|s| soon.contains(&s.space_id) || due(s, now)) {
                if shutdown.is_cancelled() {
                    return;
                }
                poll_site(&state, &site.space_id).await;
            }
        }
    });
}

/// Whether a site's next poll is due.
pub fn due(site: &JiraSiteRow, now: DateTime<Utc>) -> bool {
    let every = Duration::seconds(site.poll_secs.clamp(MIN_POLL_SECS, MAX_POLL_SECS));
    site.polled_at.is_none_or(|t| t + every <= now) && site.backoff_until.is_none_or(|t| t <= now)
}

/// Polls one space's site now (unless it's off, backing off, or already being polled).
pub async fn poll_site(state: &AppState, space_id: &str) -> PollReport {
    let mut report = PollReport::default();
    let site = match state.db.dev_jira_site(space_id).await {
        Ok(Some(s)) => s,
        _ => {
            report.skipped = Some("no_site");
            return report;
        }
    };
    if !site.on_ {
        report.skipped = Some("off");
        return report;
    }
    if !matches!(state.db.dev_mode(space_id).await, Ok(Some(m)) if m.on) {
        report.skipped = Some("dev_mode_off");
        return report;
    }
    if site.backoff_until.is_some_and(|t| t > Utc::now()) {
        report.skipped = Some("backing_off");
        report.error = site.poll_error.clone();
        return report;
    }
    if !shared().lock().unwrap().running.insert(space_id.to_string()) {
        report.skipped = Some("busy");
        return report;
    }
    let mark = run(state, &site, &mut report).await;
    shared().lock().unwrap().running.remove(space_id);
    // A source error means Jira couldn't be read: fail the live tickets after a day of that.
    if mark.unreachable {
        if let Some(since) = site.unreachable_since {
            if Utc::now() - since >= Duration::hours(UNREACHABLE_FAIL_HOURS) {
                report.failed += fail_live(state, space_id).await;
            }
        }
    }
    report.error = mark.error.clone();
    if let Err(e) = state.db.mark_dev_jira_polled(space_id, &mark).await {
        tracing::warn!("jira: could not save the poll for a space: {e}");
    }
    report
}

/// The `PollMark` for an error that stopped a poll. Never a URL or header (see `SourceError`).
pub fn mark_for(e: &SourceError, now: DateTime<Utc>) -> PollMark {
    match e {
        SourceError::RateLimited { retry_after } => {
            let wait = retry_after.unwrap_or(StdDuration::from_secs(60));
            let until = now + Duration::from_std(wait).unwrap_or(Duration::seconds(60));
            PollMark { error: Some(format!("Jira asked kwant to slow down; next check at {} UTC", until.format("%H:%M"))), backoff_until: Some(until), ..Default::default() }
        }
        SourceError::Unauthorized => PollMark { error: Some("Jira stopped accepting the token: paste a new one".into()), token_rejected: true, ..Default::default() },
        SourceError::Forbidden => PollMark { error: Some("the token can no longer read these projects".into()), ..Default::default() },
        SourceError::Unavailable(m) => PollMark { error: Some(format!("couldn't reach Jira: {m}")), unreachable: true, ..Default::default() },
        other => PollMark { error: Some(other.to_string()), ..Default::default() },
    }
}

async fn fail_live(state: &AppState, space_id: &str) -> usize {
    let live = [dev_jira::PICKED_UP, dev_jira::READING, dev_jira::NEEDS_ANSWER, dev_jira::BRIEF_READY, dev_jira::WORKING];
    let mut n = 0;
    for t in state.db.dev_tickets_in(space_id, &live).await.unwrap_or_default() {
        let note = format!("Jira unreachable for {UNREACHABLE_FAIL_HOURS} hours");
        if matches!(store::set_state(state, space_id, &t.key, dev_jira::FAILED, Some(&note)).await, Ok(true)) {
            n += 1;
        }
    }
    n
}

/// A search hit or a single read as the row's facts.
pub fn facts_of(issue_id: Option<String>, i: &Issue) -> TicketFacts {
    TicketFacts {
        key: i.key.clone(),
        issue_id: issue_id.filter(|s| !s.is_empty()),
        project: i.project_key.clone(),
        summary: i.summary.clone(),
        issue_type: i.issue_type.clone(),
        status: i.status.clone(),
        status_category: i.status_category.as_str().to_string(),
        priority: i.priority.clone(),
        priority_rank: i.priority_rank.map(i64::from),
        assignee_id: i.assignee.as_ref().map(|p| p.id.clone()),
        assignee_name: i.assignee.as_ref().map(|p| p.display_name.clone()),
        labels: i.labels.clone(),
        reporter_id: i.reporter.as_ref().map(|p| p.id.clone()),
        reporter_name: i.reporter.as_ref().map(|p| p.display_name.clone()),
        url: i.url.clone(),
        jira_created: i.created,
        jira_updated: i.updated,
    }
}

fn tref(site: &JiraSiteRow, key: &str) -> TicketRef {
    TicketRef { site_id: site.id.clone(), key: key.to_string(), team_id: site.team_id.clone().unwrap_or_default() }
}

/// How the row says it was handed over: the built rule's choice, or `jql` for Advanced JQL.
fn handed_over_by(site: &JiraSiteRow) -> &str {
    if site.jql.as_deref().is_some_and(|j| !j.trim().is_empty()) {
        "jql"
    } else {
        &site.handover
    }
}

async fn run(state: &AppState, site: &JiraSiteRow, report: &mut PollReport) -> PollMark {
    let space = site.space_id.as_str();
    let client = match atlassian::client_for(state, site).await {
        Ok(c) => c,
        Err(e) => return mark_for(&e, Utc::now()),
    };
    let projects = site.project_list();
    let jql = match check::pickup_jql(&projects, Some(&site.handover), site.assignee_id.as_deref(), Some(&site.label), site.jql.as_deref()) {
        Ok(j) => j,
        Err(m) => return PollMark { error: Some(format!("the pickup rule is invalid: {m}")), ..Default::default() },
    };
    let rows = state.db.dev_tickets(space).await.unwrap_or_default();
    let tracked = |key: &str| rows.iter().find(|r| r.key == key);
    // Read-after-write: ask Cloud to reflect the tickets kwant holds (≤ 50).
    let reconcile: Vec<String> = rows.iter().filter(|r| dev_jira::ACTIVE.contains(&r.state.as_str())).filter_map(|r| r.issue_id.clone()).take(50).collect();
    let hits = match client.search(&jql, &reconcile, MAX_SEARCH).await {
        Ok(h) => h,
        Err(e) => return mark_for(&e, Utc::now()),
    };
    report.matched = hits.len();
    let seen: HashSet<String> = hits.iter().map(|(_, i)| i.key.clone()).collect();

    for (id, issue) in &hits {
        let facts = facts_of(Some(id.clone()), issue);
        match tracked(&issue.key) {
            None => {
                let first = if site.backfill_done { dev_jira::PICKED_UP } else { dev_jira::WAITING };
                if let Ok(true) = state.db.insert_dev_ticket(space, &site.id, &facts, first, site.team_id.as_deref(), Some(handed_over_by(site))).await {
                    report.new += 1;
                    if first == dev_jira::WAITING {
                        report.waiting += 1;
                    }
                    store::notify(state, space, &issue.key, first);
                }
            }
            Some(row) => {
                let _ = state.db.update_dev_ticket_facts(space, &facts).await;
                if row.misses != 0 {
                    let _ = state.db.reset_dev_ticket_misses(space, &issue.key).await;
                }
                if matches!(row.state.as_str(), dev_jira::TAKEN_BACK | dev_jira::FAILED) {
                    if let Ok(true) = store::set_state(state, space, &issue.key, dev_jira::PICKED_UP, Some("handed over again")).await {
                        report.resumed += 1;
                    }
                }
            }
        }
    }
    if !site.backfill_done {
        let _ = state.db.set_dev_jira_backfill_done(space).await;
    }

    // Tracked tickets the search didn't return: read each on its own before moving it.
    let rule = if handed_over_by(site) == "jql" {
        None
    } else {
        let who = site.assignee_key.as_deref().or(site.assignee_id.as_deref());
        check::rule_of(&projects, Some(&site.handover), who, Some(&site.label)).ok()
    };
    let missing: Vec<&TicketRow> = rows
        .iter()
        .filter(|r| !seen.contains(&r.key))
        .filter(|r| dev_jira::ACTIVE.contains(&r.state.as_str()) && !matches!(r.state.as_str(), dev_jira::TAKEN_BACK | dev_jira::FAILED))
        .take(MAX_MISSING_CHECKS)
        .collect();
    for row in missing {
        let issue = match client.issue_unscoped(&row.key).await {
            Ok(i) => i,
            Err(SourceError::NotFound) => {
                if let Ok(true) = store::set_state(state, space, &row.key, dev_jira::CLOSED, Some("deleted")).await {
                    report.closed += 1;
                    hooks::stopped(state, space, &row.key, Why::Closed).await;
                }
                continue;
            }
            Err(e @ (SourceError::RateLimited { .. } | SourceError::Unauthorized | SourceError::Unavailable(_))) => return mark_for(&e, Utc::now()),
            Err(_) => continue,
        };
        let _ = state.db.update_dev_ticket_facts(space, &facts_of(row.issue_id.clone(), &issue)).await;
        if issue.status_category == super::source::StatusCategory::Done {
            if let Ok(true) = store::set_state(state, space, &row.key, dev_jira::CLOSED, Some(&format!("done: {}", issue.status))).await {
                report.closed += 1;
                hooks::stopped(state, space, &row.key, Why::Closed).await;
            }
            continue;
        }
        let gone = match &rule {
            Some(rule) => {
                let who = issue.assignee.as_ref().map(|p| p.id.as_str());
                if rule.matches(&issue.project_key, who, &issue.labels) {
                    // Still handed over: the search lagged. Keep it.
                    false
                } else {
                    true
                }
            }
            None => state.db.bump_dev_ticket_misses(space, &row.key).await.unwrap_or(0) >= CUSTOM_MISSES,
        };
        if gone {
            let note = if rule.as_ref().is_some_and(|r| r.handover == atlassian::jql::Handover::Assignee) { "unassigned" } else if rule.is_some() { "label removed" } else { "no longer matches the JQL" };
            if let Ok(true) = store::set_state(state, space, &row.key, dev_jira::TAKEN_BACK, Some(note)).await {
                report.taken_back += 1;
                if let Ok(Some(name)) = client.last_changed_by(&row.key, &["assignee", "labels"]).await {
                    let _ = state.db.set_dev_ticket_taken_back_by(space, &row.key, &name).await;
                    store::notify(state, space, &row.key, dev_jira::TAKEN_BACK);
                }
                hooks::stopped(state, space, &row.key, Why::TakenBack).await;
            }
        }
    }

    if let Err(e) = answers(state, site, &client, report).await {
        return mark_for(&e, Utc::now());
    }
    start_queued(state, site, report).await;
    PollMark::default()
}

/// A comment id's order: Jira's ids are numbers that grow.
fn id_num(id: &str) -> Option<u64> {
    id.trim().parse().ok()
}

/// Replies to Ask on Jira comments, oldest first, each once.
async fn answers(state: &AppState, site: &JiraSiteRow, client: &AtlassianClient, report: &mut PollReport) -> Result<(), SourceError> {
    let space = site.space_id.as_str();
    let waiting = state.db.dev_tickets_in(space, &[dev_jira::NEEDS_ANSWER]).await.unwrap_or_default();
    for row in waiting.iter().filter(|r| r.asked_comment_id.is_some()).take(MAX_ANSWER_CHECKS) {
        let Some(asked) = row.asked_comment_id.as_deref().and_then(id_num) else { continue };
        let after = row.last_reply_id.as_deref().and_then(id_num).unwrap_or(0).max(asked);
        // Whose answer counts: the reporter, else the assignee (whoever the comment mentioned),
        // never kwant's own user.
        let Some(who) = row.reporter_id.as_deref().or(row.assignee_id.as_deref()).filter(|w| Some(*w) != site.me_id.as_deref()) else { continue };
        let comments = match client.comments(&row.key).await {
            Ok(c) => c,
            Err(e @ (SourceError::RateLimited { .. } | SourceError::Unauthorized | SourceError::Unavailable(_))) => return Err(e),
            Err(_) => continue,
        };
        for c in comments.iter().filter(|c| c.author.as_ref().is_some_and(|a| a.id == who) && id_num(&c.id).is_some_and(|n| n > after)) {
            hooks::reporter_replied(state, space, &row.key, &asked.to_string(), c).await;
            let _ = state.db.set_dev_ticket_last_reply(space, &row.key, &c.id).await;
            report.replies += 1;
        }
    }
    Ok(())
}

/// The J13 queue: priority (unknown last), then Jira's created time, then key.
pub fn queue_order(a: &TicketRow, b: &TicketRow) -> std::cmp::Ordering {
    let rank = |t: &TicketRow| t.priority_rank.unwrap_or(i64::MAX);
    let created = |t: &TicketRow| t.jira_created.unwrap_or(t.first_seen_at);
    rank(a).cmp(&rank(b)).then_with(|| created(a).cmp(&created(b))).then_with(|| a.key.cmp(&b.key))
}

/// Starts queued tickets while a slot is free (the Start button); answers how many started.
pub async fn fill_slots(state: &AppState, site: &JiraSiteRow) -> usize {
    let mut report = PollReport::default();
    start_queued(state, site, &mut report).await;
    report.started
}

/// Starts queued tickets while a slot is free.
async fn start_queued(state: &AppState, site: &JiraSiteRow, report: &mut PollReport) {
    let space = site.space_id.as_str();
    let rows = state.db.dev_tickets(space).await.unwrap_or_default();
    let busy = rows.iter().filter(|r| dev_jira::SLOT_STATES.contains(&r.state.as_str()) || (r.state == dev_jira::PICKED_UP && r.start_called_at.is_some())).count();
    let mut queue: Vec<&TicketRow> = rows.iter().filter(|r| r.state == dev_jira::PICKED_UP && r.start_called_at.is_none()).collect();
    queue.sort_by(|a, b| queue_order(a, b));
    for t in queue.into_iter().take(SLOTS.saturating_sub(busy)) {
        if let Ok(true) = state.db.claim_dev_ticket_start(space, &t.key).await {
            hooks::start(state, space, tref(site, &t.key)).await;
            report.started += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, rank: Option<i64>, created_min: i64) -> TicketRow {
        let t0 = DateTime::parse_from_rfc3339("2026-11-07T09:00:00Z").unwrap().with_timezone(&Utc);
        TicketRow {
            space_id: "s".into(),
            key: key.into(),
            site_id: "jira_1".into(),
            issue_id: None,
            project: "TRAIL".into(),
            summary: String::new(),
            issue_type: String::new(),
            status: String::new(),
            status_category: "todo".into(),
            priority: None,
            priority_rank: rank,
            assignee_id: None,
            assignee_name: None,
            labels: "[]".into(),
            reporter_id: None,
            reporter_name: None,
            url: String::new(),
            state: "picked_up".into(),
            team_id: None,
            jira_created: Some(t0 + Duration::minutes(created_min)),
            jira_updated: None,
            handed_over_by: None,
            handed_over_at: None,
            asked_comment_id: None,
            last_reply_id: None,
            start_called_at: None,
            misses: 0,
            taken_back_at: None,
            taken_back_by: None,
            closed_resolution: None,
            waiting_since_connect: false,
            work_comment_at: None,
            pr_comment_at: None,
            first_seen_at: t0,
            updated_at: t0,
        }
    }

    #[test]
    fn the_queue_is_priority_then_created_then_key() {
        let mut v = [row("TRAIL-3", None, 0), row("TRAIL-2", Some(3), 5), row("TRAIL-1", Some(3), 5), row("TRAIL-9", Some(1), 9), row("TRAIL-4", Some(3), 1)];
        v.sort_by(queue_order);
        let keys: Vec<&str> = v.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["TRAIL-9", "TRAIL-4", "TRAIL-1", "TRAIL-2", "TRAIL-3"]);
    }

    #[test]
    fn errors_become_a_status_line_and_a_back_off() {
        let now = Utc::now();
        let m = mark_for(&SourceError::RateLimited { retry_after: Some(StdDuration::from_secs(120)) }, now);
        assert_eq!(m.backoff_until, Some(now + Duration::seconds(120)));
        assert!(m.error.unwrap().starts_with("Jira asked kwant to slow down"));
        assert_eq!(mark_for(&SourceError::RateLimited { retry_after: None }, now).backoff_until, Some(now + Duration::seconds(60)));
        assert!(mark_for(&SourceError::Unauthorized, now).token_rejected);
        assert!(mark_for(&SourceError::Unavailable("timed out".into()), now).unreachable);
        assert!(!mark_for(&SourceError::Forbidden, now).unreachable);
    }

    #[test]
    fn a_site_is_due_after_its_interval_and_its_back_off() {
        let now = Utc::now();
        let mut s = JiraSiteRow::blank("s", "jira_1");
        assert!(due(&s, now));
        s.polled_at = Some(now - Duration::seconds(299));
        s.poll_secs = 300;
        assert!(!due(&s, now));
        s.polled_at = Some(now - Duration::seconds(300));
        assert!(due(&s, now));
        s.poll_secs = 1; // clamped to a minute
        s.polled_at = Some(now - Duration::seconds(30));
        assert!(!due(&s, now));
        s.polled_at = None;
        s.backoff_until = Some(now + Duration::seconds(5));
        assert!(!due(&s, now));
    }
}
