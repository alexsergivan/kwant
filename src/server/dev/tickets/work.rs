//! The `work` object on a ticket row (api-decisions 11, api.md §2.6): who works on the ticket and
//! where the work is. `null` until work starts. Read-only: it joins what kwant already has (the
//! team, the mission's tasks, the newest live Dev mode card whose branch or title carries the key)
//! with Vega's seam (`hooks::work_seam`: the mission id and the Matches the ticket verdict).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use super::hooks::{self, WorkSeam};
use super::store;
use crate::db::dev_jira::{self, TicketRow};
use crate::db::dev_mode::DevCardRow;
use crate::db::spaces::SpaceId;
use crate::server::AppState;

/// The verdicts `matches` may carry on the wire; anything else from the seam reads as `null`.
pub const MATCHES: &[&str] = &["met", "not_met", "unclear"];

/// At most this many plan steps on one ticket row.
pub const MAX_PLAN: usize = 30;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Work {
    pub team_id: Option<String>,
    pub lead_id: Option<String>,
    pub builder_ids: Vec<String>,
    pub reviewer_id: Option<String>,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub head_sha: Option<String>,
    /// When the PR merged (from the ticket's landed card); `null` until it does (api-decisions 22).
    pub merged_at: Option<DateTime<Utc>>,
    pub plan: Vec<Step>,
    pub matches: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Step {
    pub step: String,
    pub state: String,
}

/// One task of the ticket's mission, as the plan shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskLine {
    pub title: String,
    pub state: String,
    pub owner: Option<String>,
}

/// What a live Dev mode card says about the ticket's PR.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardWork {
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub head_sha: Option<String>,
    pub reviewer_id: Option<String>,
    pub merged_at: Option<DateTime<Utc>>,
}

impl CardWork {
    pub fn of(row: &DevCardRow) -> Self {
        let facts: Value = serde_json::from_str(&row.facts).unwrap_or(Value::Null);
        let card: Value = row.card.as_deref().and_then(|c| serde_json::from_str(c).ok()).unwrap_or(Value::Null);
        let text = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        CardWork {
            branch: text(&facts["head_branch"]).or_else(|| text(&card["branch"])),
            pr_url: text(&facts["links"]["pr"]).filter(|u| u.starts_with("https://")),
            head_sha: Some(row.head_sha.clone()).filter(|s| !s.is_empty()),
            reviewer_id: text(&card["reviewer"]["id"]),
            merged_at: (row.kind == crate::db::dev_mode::LANDED).then(|| facts["merged_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok())).flatten().map(|t| t.with_timezone(&Utc)),
        }
    }
}

/// The work object, or `None` while no work has started: the row is `working`, `in_review` or
/// `landed`, or the seam names a mission (a stopped or failed ticket keeps the work it had).
pub fn compose(ticket_state: &str, team_id: Option<&str>, seam: &WorkSeam, team_lead: Option<&str>, mission_lead: Option<&str>, tasks: &[TaskLine], card: Option<&CardWork>) -> Option<Work> {
    let started = matches!(ticket_state, dev_jira::WORKING | dev_jira::IN_REVIEW | dev_jira::LANDED);
    if !started && seam.mission_id.is_none() {
        return None;
    }
    let mut builder_ids: Vec<String> = Vec::new();
    for id in tasks.iter().filter_map(|t| t.owner.as_deref()) {
        if !builder_ids.iter().any(|b| b == id) {
            builder_ids.push(id.to_string());
        }
    }
    let card = card.cloned().unwrap_or_default();
    Some(Work {
        team_id: team_id.filter(|t| !t.is_empty()).map(str::to_string),
        lead_id: mission_lead.or(team_lead).filter(|l| !l.is_empty()).map(str::to_string),
        builder_ids,
        reviewer_id: card.reviewer_id,
        branch: card.branch,
        pr_url: card.pr_url,
        head_sha: card.head_sha,
        merged_at: card.merged_at,
        plan: tasks.iter().take(MAX_PLAN).map(|t| Step { step: t.title.clone(), state: t.state.clone() }).collect(),
        matches: seam.matches.as_deref().and_then(|m| MATCHES.iter().copied().find(|v| *v == m)),
    })
}

/// The newest live review or landed card per ticket key (cards carry keys in their branch or
/// title; `store::card_keys`).
async fn cards_by_key(state: &AppState, space: &str) -> HashMap<String, CardWork> {
    let mut cards: Vec<DevCardRow> = Vec::new();
    for kind in ["review", "landed"] {
        cards.extend(state.db.dev_cards(space, kind, None, 200).await.unwrap_or_default());
    }
    cards.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    let mut out: HashMap<String, CardWork> = HashMap::new();
    for c in &cards {
        if let Some((keys, _)) = store::card_keys(c) {
            for k in keys {
                out.entry(k).or_insert_with(|| CardWork::of(c));
            }
        }
    }
    // A merge counts whichever card is newest (a review card can be touched after the landing)
    for c in cards.iter().filter(|c| c.kind == crate::db::dev_mode::LANDED) {
        let Some(at) = CardWork::of(c).merged_at else { continue };
        if let Some((keys, _)) = store::card_keys(c) {
            for k in keys {
                if let Some(w) = out.get_mut(&k) {
                    w.merged_at.get_or_insert(at);
                }
            }
        }
    }
    out
}

/// Each row as its JSON with `work` added (the list, one ticket, and the lifecycle answers).
pub async fn rows_with_work(state: &AppState, space: &str, rows: &[TicketRow]) -> Vec<Value> {
    let mut team_leads: HashMap<String, Option<String>> = HashMap::new();
    let cards = if rows.iter().any(|r| r.state != dev_jira::WAITING) { cards_by_key(state, space).await } else { HashMap::new() };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let seam = hooks::work_seam(state, space, &row.key).await;
        let mut mission_lead = None;
        let mut tasks = Vec::new();
        if let Some(mid) = seam.mission_id.as_deref() {
            mission_lead = state.db.get_mission(mid).await.ok().flatten().filter(|m| m.space_id == space).map(|m| m.lead_kwant_id);
            tasks = state.db.list_tasks(&SpaceId::new(space), None, Some(mid), None, MAX_PLAN as i64).await.unwrap_or_default().into_iter().map(|t| TaskLine { title: t.title, state: t.state, owner: t.owner_kwant_id }).collect();
        }
        let team_lead = match row.team_id.as_deref().filter(|t| !t.is_empty()) {
            Some(t) => {
                if !team_leads.contains_key(t) {
                    let lead = state.db.get_team(t).await.ok().flatten().map(|team| team.lead_kwant_id);
                    team_leads.insert(t.to_string(), lead);
                }
                team_leads.get(t).cloned().flatten()
            }
            None => None,
        };
        let work = compose(&row.state, row.team_id.as_deref(), &seam, team_lead.as_deref(), mission_lead.as_deref(), &tasks, cards.get(&row.key));
        let mut v = serde_json::to_value(row).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.insert("work".into(), serde_json::to_value(work).unwrap_or(Value::Null));
            o.insert("error".into(), store::error_json(row));
        }
        out.push(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn task(title: &str, state: &str, owner: Option<&str>) -> TaskLine {
        TaskLine { title: title.into(), state: state.into(), owner: owner.map(str::to_string) }
    }

    #[test]
    fn work_is_null_until_work_starts() {
        let none = WorkSeam::default();
        for s in [dev_jira::WAITING, dev_jira::PICKED_UP, dev_jira::READING, dev_jira::NEEDS_ANSWER, dev_jira::BRIEF_READY, dev_jira::TAKEN_BACK, dev_jira::CLOSED, dev_jira::FAILED] {
            assert_eq!(compose(s, Some("team_1"), &none, Some("kw_lead"), None, &[], None), None, "{s}");
        }
        for s in [dev_jira::WORKING, dev_jira::IN_REVIEW, dev_jira::LANDED] {
            assert!(compose(s, Some("team_1"), &none, Some("kw_lead"), None, &[], None).is_some(), "{s}");
        }
        // A stopped ticket keeps the work it had
        let seam = WorkSeam { mission_id: Some("m1".into()), matches: None };
        assert!(compose(dev_jira::TAKEN_BACK, Some("team_1"), &seam, None, None, &[], None).is_some());
    }

    #[test]
    fn work_names_the_lead_builders_reviewer_pr_and_plan() {
        let seam = WorkSeam { mission_id: Some("m1".into()), matches: Some("not_met".into()) };
        let tasks = [task("Add the export", "done", Some("kw_a")), task("Test it", "running", Some("kw_b")), task("Docs", "ready", Some("kw_a")), task("Later", "ready", None)];
        let merged = "2026-11-07T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let card = CardWork { branch: Some("feat/TRAIL-1-export".into()), pr_url: Some("https://github.com/o/r/pull/7".into()), head_sha: Some("abc123".into()), reviewer_id: Some("kw_r".into()), merged_at: Some(merged) };
        let w = compose(dev_jira::IN_REVIEW, Some("team_1"), &seam, Some("kw_team_lead"), Some("kw_mission_lead"), &tasks, Some(&card)).unwrap();
        assert_eq!(w.team_id.as_deref(), Some("team_1"));
        assert_eq!(w.lead_id.as_deref(), Some("kw_mission_lead"), "the mission's lead wins over the team's");
        assert_eq!(w.builder_ids, vec!["kw_a".to_string(), "kw_b".to_string()], "distinct owners, in plan order");
        assert_eq!(w.reviewer_id.as_deref(), Some("kw_r"));
        assert_eq!((w.branch.as_deref(), w.pr_url.as_deref(), w.head_sha.as_deref()), (Some("feat/TRAIL-1-export"), Some("https://github.com/o/r/pull/7"), Some("abc123")));
        assert_eq!(w.plan.iter().map(|s| (s.step.as_str(), s.state.as_str())).collect::<Vec<_>>(), vec![("Add the export", "done"), ("Test it", "running"), ("Docs", "ready"), ("Later", "ready")]);
        assert_eq!(w.matches, Some("not_met"));
        assert_eq!(w.merged_at, Some(merged));
        // Without a mission: the team's lead; an unknown verdict reads as null
        let w = compose(dev_jira::WORKING, None, &WorkSeam { mission_id: None, matches: Some("maybe".into()) }, Some("kw_team_lead"), None, &[], None).unwrap();
        assert_eq!((w.lead_id.as_deref(), w.matches, w.plan.len(), w.pr_url.as_deref(), w.merged_at), (Some("kw_team_lead"), None, 0, None, None), "merged_at is null until the PR merges");
        let v = serde_json::to_value(&w).unwrap();
        for k in ["team_id", "lead_id", "builder_ids", "reviewer_id", "branch", "pr_url", "head_sha", "merged_at", "plan", "matches"] {
            assert!(v.get(k).is_some(), "work.{k} is always present");
        }
    }

    #[test]
    fn a_card_gives_branch_pr_head_and_reviewer_and_never_a_non_https_link() {
        let mk_kind = |kind: &str, facts: Value, card: Option<Value>| DevCardRow {
            id: "c".into(),
            space_id: "s".into(),
            kind: kind.into(),
            ref_: "pr/7".into(),
            head_sha: "deadbeef".into(),
            status: "ready".into(),
            facts: facts.to_string(),
            card: card.map(|c| c.to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            repo: String::new(),
            posted: None,
            post_error: None,
        };
        let mk = |facts: Value, card: Option<Value>| mk_kind("review", facts, card);
        let c = CardWork::of(&mk(serde_json::json!({"head_branch": "TRAIL-1", "links": {"pr": "https://github.com/o/r/pull/7"}}), Some(serde_json::json!({"reviewer": {"id": "kw_r", "name": "Rigel"}}))));
        assert_eq!(c, CardWork { branch: Some("TRAIL-1".into()), pr_url: Some("https://github.com/o/r/pull/7".into()), head_sha: Some("deadbeef".into()), reviewer_id: Some("kw_r".into()), merged_at: None });
        let c = CardWork::of(&mk(serde_json::json!({"links": {"pr": "javascript:alert(1)"}}), Some(serde_json::json!({"branch": "b"}))));
        assert_eq!((c.branch.as_deref(), c.pr_url, c.reviewer_id), (Some("b"), None, None));
        // merged_at comes from a landed card's facts only
        let at = serde_json::json!({"merged_at": "2026-11-07T10:00:00Z"});
        assert_eq!(CardWork::of(&mk_kind("landed", at.clone(), None)).merged_at.map(|t| t.to_rfc3339()), Some("2026-11-07T10:00:00+00:00".into()));
        assert_eq!(CardWork::of(&mk_kind("review", at, None)).merged_at, None);
    }
}
