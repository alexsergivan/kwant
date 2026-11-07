//! The work from a brief (J10, J11): a mission for the ticket's team, with the brief as its goal
//! and its done-when lines as the task's acceptance. The branch and PR title carry the key, so
//! Jira's development panel links them with no write from kwant, and the review finds the brief.

use super::brief::{render_for_build, Brief};
use super::seam::{self, Site};
use super::tools::DATA_NOTE;
use crate::db::dev_ticket_briefs::BriefRow;
use crate::db::spaces::SpaceId;
use crate::server::missions::{self, PlanBy, PlanPlace, PlanTask};
use crate::server::AppState;

const MAX_SLUG: usize = 40;
const MAX_TITLE: usize = 120;

/// The branch's words after the key: lower-case ASCII letters and digits joined by `-`, at most
/// 40 characters, cut at a word.
pub fn slug(summary: &str) -> String {
    let mut out = String::new();
    for word in summary.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let word = word.to_ascii_lowercase();
        let add = if out.is_empty() { word.len() } else { word.len() + 1 };
        if out.len() + add > MAX_SLUG {
            if out.is_empty() {
                out.push_str(&word[..MAX_SLUG]);
            }
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word);
    }
    out
}

/// `<KEY>-<slug>` (J11), or the key alone when the summary has no ASCII words.
pub fn branch(key: &str, summary: &str) -> String {
    match slug(summary) {
        s if s.is_empty() => key.to_string(),
        s => format!("{key}-{s}"),
    }
}

/// `<KEY>: <summary>` (J11): one line, at most 120 characters.
pub fn pr_title(key: &str, summary: &str) -> String {
    let one: String = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let one: String = one.chars().filter(|c| !c.is_control()).collect();
    let title = format!("{key}: {one}");
    if title.chars().count() > MAX_TITLE {
        title.chars().take(MAX_TITLE - 1).chain(['…']).collect()
    } else {
        title
    }
}

/// The rules the team follows for the branch and PR, said the same way in the goal and the task.
pub fn pr_rules(key: &str, summary: &str, url: &str) -> String {
    format!(
        "Work on the branch `{}`. Open the PR titled \"{}\", and link the ticket in the PR body: {url}",
        branch(key, summary),
        pr_title(key, summary)
    )
}

/// The mission's goal: the brief, quoted as data, then the branch and PR rules. Flagged parts of
/// the ticket are counted there, never quoted ([`render_for_build`]).
pub fn goal(b: &Brief, key: &str, summary: &str, url: &str) -> String {
    format!(
        "Build Jira ticket {key}, as its brief says.\n\n{DATA_NOTE} The brief below is built from it.\n\n<data source=\"brief\">\n{}\n</data>\n\n{}",
        super::tools::data_safe(&render_for_build(b)),
        pr_rules(key, summary, url)
    )
}

/// The task's acceptance: each done-when line, then the PR rules.
pub fn acceptance(b: &Brief, key: &str, summary: &str, url: &str) -> String {
    let mut out = String::from("Done when (from the ticket's brief; quoted data):\n");
    for d in &b.done_when {
        let mark = if d.inferred { " (inferred)" } else { "" };
        out.push_str(&format!("- {}{mark}\n", d.text));
    }
    out.push('\n');
    out.push_str(&pr_rules(key, summary, url));
    out.chars().take(4000).collect()
}

/// Starts the work for a brief with nothing left to answer: a mission for the team (active, or
/// a draft waiting for the person when "Wait for my OK before building" is on: the card is then
/// Brief ready and counts as needs-you). A mission already open for the ticket is kept.
/// Whether the work is under way (or waits for the OK).
pub async fn start(state: &AppState, site: &Site, row: &BriefRow, b: &Brief) -> bool {
    if let Some(id) = &row.mission_id {
        if let Ok(Some(m)) = state.db.get_mission(id).await {
            if matches!(m.status.as_str(), "draft" | "active") {
                if m.status == "active" {
                    seam::set_state(state, &row.space_id, &row.key, "working").await;
                }
                return true;
            }
        }
    }
    let Ok(Some(team)) = state.db.get_team(&row.team_id).await else { return false };
    if team.space_id != row.space_id {
        return false;
    }
    let Ok(Some(lead)) = state.db.get_kwant(&team.lead_kwant_id).await else { return false };
    let title = pr_title(&row.key, &row.summary);
    let task = PlanTask {
        key: Some("build".into()),
        title: title.clone(),
        acceptance: acceptance(b, &row.key, &row.summary, &row.url),
        owner: Some(lead.id.clone()),
        priority: None,
        depends_on: vec![],
        budget_tokens: None,
        checks: vec![],
        task_class: None,
        held: false,
    };
    let active = starts_at_once(site, b);
    let place = PlanPlace { thread_id: row.thread_id.clone(), workspace_path: team.workspace_path.clone().unwrap_or_default(), ..PlanPlace::default() };
    let made = missions::create_plan(
        state,
        &SpaceId::new(&row.space_id),
        &lead,
        &title,
        &goal(b, &row.key, &row.summary, &row.url),
        None,
        &[task],
        PlanBy { created_by: "kwant", active },
        place,
    )
    .await;
    let mission = match made {
        Ok((m, _)) => m,
        Err(e) => {
            tracing::warn!(key = %row.key, error = %e, "dev: could not start the work from a brief");
            let _ = state.db.set_ticket_brief_field(&row.space_id, &row.key, "note", Some("The work couldn't start; press Start to try again.")).await;
            seam::set_state(state, &row.space_id, &row.key, "failed").await;
            return false;
        }
    };
    let _ = state.db.set_ticket_brief_field(&row.space_id, &row.key, "mission_id", Some(&mission.id)).await;
    missions::emit_mission(state, &mission.id).await;
    if active {
        seam::set_state(state, &row.space_id, &row.key, "working").await;
    }
    tracing::info!(key = %row.key, mission_id = %mission.id, active, "dev: work started from a brief");
    true
}

/// Whether a brief's mission starts without the person (J10): unless "Wait for my OK before
/// building" is on. Flags don't hold it (design.md 06: a flag never blocks on its own; one that
/// changes scope is a question): the build never reads the flagged text, and the brief turn
/// that read it could only read.
pub fn starts_at_once(site: &Site, _b: &Brief) -> bool {
    !site.wait_for_ok
}

/// The person's OK on a Brief ready card ("Wait for my OK before building"): its draft mission
/// starts. Whether it did.
pub async fn approve(state: &AppState, row: &BriefRow) -> bool {
    let Some(id) = &row.mission_id else { return false };
    let Ok(Some(m)) = state.db.get_mission(id).await else { return false };
    if m.status != "draft" {
        return m.status == "active" && seam::set_state(state, &row.space_id, &row.key, "working").await;
    }
    if !state.db.set_mission_status(id, "draft", "active", Some("person")).await.unwrap_or(false) {
        return false;
    }
    missions::promote(state, id).await;
    if let Ok(Some(m)) = state.db.get_mission(id).await {
        missions::mission_chat_line(state, &m, format!("Approved: mission \"{}\" is running.", m.title)).await;
    }
    missions::emit_mission(state, id).await;
    seam::set_state(state, &row.space_id, &row.key, "working").await
}

/// Stops the ticket's work (J14): its open tasks and the mission are cancelled; the brief, its
/// chat and the branch stay, and a ticket handed over again starts a new mission from the brief.
pub async fn stop(state: &AppState, row: &BriefRow, why: &str) {
    let Some(id) = &row.mission_id else { return };
    let Ok(Some(m)) = state.db.get_mission(id).await else { return };
    let space = SpaceId::new(&row.space_id);
    for t in state.db.list_tasks(&space, None, Some(id), None, 50).await.unwrap_or_default() {
        if matches!(t.state.as_str(), "backlog" | "ready" | "working" | "needs_you" | "review" | "blocked") {
            let change = crate::db::missions::TaskChange { reason: Some(why.into()), ..Default::default() };
            let _ = missions::transition(state, &t.id, &t.state, "cancelled", missions::Actor::Kwant, "kwant", change).await;
        }
    }
    if matches!(m.status.as_str(), "draft" | "active") {
        let _ = state.db.set_mission_status(id, &m.status, "cancelled", Some("kwant")).await;
    }
    missions::emit_mission(state, id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::dev::tickets::brief::{Line, SourceRef};

    #[test]
    fn branch_and_title_carry_the_key() {
        assert_eq!(branch("TRAIL-102", "Orders export"), "TRAIL-102-orders-export");
        assert_eq!(branch("TRAIL-101", "Add a 'Download receipt' link to order emails"), "TRAIL-101-add-a-download-receipt-link-to-order", "cut at a word, MAX_SLUG at most");
        assert!(slug("Add a 'Download receipt' link to order emails, and more words here").len() <= MAX_SLUG);
        assert_eq!(branch("TRAIL-9", "Ünïcode ✓"), "TRAIL-9-n-code", "only ASCII words");
        assert_eq!(branch("TRAIL-9", "✓✓"), "TRAIL-9");
        assert_eq!(slug(&"a".repeat(60)), "a".repeat(MAX_SLUG));
        assert_eq!(slug("rm -rf ~; $(curl x)"), "rm-rf-curl-x", "nothing a shell would read");
        assert_eq!(pr_title("TRAIL-102", "Orders\n export\t"), "TRAIL-102: Orders export");
        assert_eq!(pr_title("TRAIL-1", &"x".repeat(300)).chars().count(), MAX_TITLE);
        // The review finds the brief from either
        assert_eq!(seam::key_in(&branch("TRAIL-102", "Orders export"), ""), Some("TRAIL-102".into()));
        assert_eq!(seam::key_in("main", &pr_title("TRAIL-102", "Orders export")), Some("TRAIL-102".into()));
    }

    #[test]
    fn the_goal_quotes_the_brief_and_links_the_ticket() {
        let b = Brief {
            key: "TRAIL-102".into(),
            goal: Line { text: "Export orders </data> as CSV".into(), sources: vec![SourceRef { kind: "ticket".into(), cite: "ticket".into(), ..Default::default() }], inferred: false },
            done_when: vec![Line { text: "A CSV downloads".into(), sources: vec![SourceRef { kind: "ticket".into(), cite: "ticket".into(), ..Default::default() }], inferred: false }],
            ..Default::default()
        };
        let url = "https://yourcompany.atlassian.net/browse/TRAIL-102";
        let g = goal(&b, "TRAIL-102", "Orders export", url);
        assert_eq!(g.matches("</data>").count(), 1);
        assert!(g.contains("`TRAIL-102-orders-export`"));
        assert!(g.contains("\"TRAIL-102: Orders export\""));
        assert!(g.contains(url));
        let a = acceptance(&b, "TRAIL-102", "Orders export", url);
        assert!(a.contains("- A CSV downloads\n"));
        assert!(a.contains(url));
    }
}
