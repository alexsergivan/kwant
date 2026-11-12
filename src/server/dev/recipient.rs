//! Who a review's Fix it task and its line notes go to (fix-it-any-pr brief, Nov 12): one rule for
//! both paths.
//!
//! 1. The PR's author bot in this space: the bot the facts name, else the bot the head branch
//!    names (`pixel/…`).
//! 2. Else the lead of the team that owns the card's repo: `dev_repos.team` of the card's repo key
//!    → that team in the same space → its `lead_kwant_id` → a bot of the space.
//! 3. Else nobody. No other guessing (not "the first bot of the space").

use crate::db::models::{Kwant, Team};
use crate::db::spaces::SpaceId;
use crate::server::dev::facts::DevFacts;
use crate::server::AppState;

/// How the recipient was picked: stored on the finding as `sent_via`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Author,
    TeamLead,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Author => "author",
            Via::TeamLead => "team_lead",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Recipient {
    pub bot: Kwant,
    pub via: Via,
    /// The owning team's name, when [`Via::TeamLead`].
    pub team: Option<String>,
}

/// The PR's author bot among `kwants` (the space's bots): the review's author bot, else the bot
/// the head branch names (`pixel/…`).
pub fn author_bot<'a>(facts: &DevFacts, kwants: &'a [Kwant]) -> Option<&'a Kwant> {
    if let Some(b) = &facts.author.bot {
        if let Some(k) = kwants.iter().find(|k| k.id == b.id) {
            return Some(k);
        }
    }
    let (prefix, _) = facts.head_branch.as_deref()?.split_once('/')?;
    let prefix = prefix.to_lowercase();
    let slug = |n: &str| n.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    kwants.iter().find(|k| !k.name.trim().is_empty() && slug(&k.name) == prefix)
}

/// The lead of `team` among `kwants`, when the team is in `space` and its lead is a bot there.
pub fn team_lead<'a>(team: &Team, space: &str, kwants: &'a [Kwant]) -> Option<&'a Kwant> {
    if team.space_id != space {
        return None;
    }
    let lead = team.lead_kwant_id.trim();
    if lead.is_empty() {
        return None;
    }
    kwants.iter().find(|k| k.id == lead)
}

/// Who gets a fix or notes for a card of `repo_key` (the card's `repo`, the `dev_repos` key) in
/// `space`, by the rule above.
pub async fn pick(state: &AppState, space: &SpaceId, repo_key: &str, facts: Option<&DevFacts>) -> Option<Recipient> {
    let kwants = state.db.list_kwants(space).await.ok()?;
    if let Some(k) = facts.and_then(|f| author_bot(f, &kwants)) {
        return Some(Recipient { bot: k.clone(), via: Via::Author, team: None });
    }
    let team_id = state.db.dev_repo(space.as_str(), repo_key).await.ok().flatten()?.team?;
    let team = state.db.get_team(&team_id).await.ok().flatten()?;
    let lead = team_lead(&team, space.as_str(), &kwants)?;
    Some(Recipient { bot: lead.clone(), via: Via::TeamLead, team: Some(team.name.clone()) })
}

/// The finding's error when there is nobody: what to do instead of a dead end.
pub fn nobody_for_fix(repo: &str) -> String {
    let repo = if repo.trim().is_empty() { "this repo".to_string() } else { repo.trim().to_string() };
    format!("No bot wrote this PR and no team owns {repo}. Add the repo to a team (Manage team › Git Repo), then press Fix it again.")
}

/// Line notes' `no_author` message when there is nobody.
pub fn nobody_for_notes(repo: &str) -> String {
    let repo = if repo.trim().is_empty() { "this repo".to_string() } else { repo.trim().to_string() };
    format!("No bot wrote this PR and no team owns {repo}. Add the repo to a team (Manage team › Git Repo), then send again.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kwant(id: &str, name: &str) -> Kwant {
        serde_json::from_value(json!({
            "id": id, "name": name, "title": "Engineer", "persona_prompt": "", "soul": "", "avatar_sprite": "",
            "driver_type": "cli", "preferred_model": "claude/demo-model", "temperature": 0.0, "max_tokens": 0,
            "level": 1, "turns_completed": 0, "created_at": null, "space_id": "s1"
        }))
        .unwrap()
    }

    fn team(space: &str, lead: &str) -> Team {
        Team {
            id: "t1".into(),
            name: "Core".into(),
            description: None,
            icon: String::new(),
            lead_kwant_id: lead.into(),
            member_kwants: "[]".into(),
            workspace_path: None,
            repo_url: None,
            created_at: None,
            instructions: None,
            space_id: space.into(),
        }
    }

    fn facts(bot: Option<&str>, branch: Option<&str>) -> DevFacts {
        serde_json::from_value(json!({
            "repo": "o/r", "kind": "review", "pr": 7, "title": "t", "body": "", "head_sha": "a", "base_branch": "main",
            "merged_at": null, "merged_by": null,
            "author": {"login": null, "name": "Oleks", "email": null, "bot": bot.map(|b| json!({"id": b, "name": "x", "engine": "claude", "model": "x"}))},
            "files": [], "cut": null, "checks": [], "ci": "green", "task": null, "first_edit_files": [], "head_branch": branch
        }))
        .unwrap()
    }

    #[test]
    fn the_author_bot_then_the_branch_prefix() {
        let ks = vec![kwant("k1", "Pixel"), kwant("k2", "Lin Bot")];
        assert_eq!(author_bot(&facts(Some("k2"), Some("pixel/x")), &ks).map(|k| k.id.as_str()), Some("k2"));
        // The author bot is gone: the branch still names one
        assert_eq!(author_bot(&facts(Some("gone"), Some("pixel/x")), &ks).map(|k| k.id.as_str()), Some("k1"));
        assert_eq!(author_bot(&facts(None, Some("lin-bot/y")), &ks).map(|k| k.id.as_str()), Some("k2"));
        assert!(author_bot(&facts(None, Some("oleks/tweak")), &ks).is_none());
        assert!(author_bot(&facts(None, None), &ks).is_none());
    }

    #[test]
    fn a_lead_counts_only_in_the_same_space_and_as_a_bot_there() {
        let ks = vec![kwant("k1", "Pixel"), kwant("k2", "Ada")];
        assert_eq!(team_lead(&team("s1", "k2"), "s1", &ks).map(|k| k.id.as_str()), Some("k2"));
        assert!(team_lead(&team("s2", "k2"), "s1", &ks).is_none(), "a team of another space");
        assert!(team_lead(&team("s1", "gone"), "s1", &ks).is_none(), "a lead that isn't a bot here");
        assert!(team_lead(&team("s1", " "), "s1", &ks).is_none(), "no lead");
    }

    #[test]
    fn nobody_says_what_to_do() {
        assert_eq!(
            nobody_for_fix("oleks/edge"),
            "No bot wrote this PR and no team owns oleks/edge. Add the repo to a team (Manage team › Git Repo), then press Fix it again."
        );
        assert!(nobody_for_notes("").contains("no team owns this repo"));
        assert_eq!(Via::Author.as_str(), "author");
        assert_eq!(Via::TeamLead.as_str(), "team_lead");
    }
}
