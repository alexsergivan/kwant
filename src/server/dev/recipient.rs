//! Who a review's Fix it task and its line notes go to (fix-it-any-pr brief, Nov 12): one rule for
//! both paths.
//!
//! 1. The PR's author bot in this space: the bot the facts name, else the bot the head branch
//!    names (`pixel/…`).
//! 2. Else the lead of the team that owns the card's repo: `dev_repos.team` of the card's repo key
//!    → that team in the same space → its `lead_kwant_id` → a bot of the space.
//! 3. Else nobody. No other guessing (not "the first bot of the space").
//!
//! "Send to…" (fix-it-send-to brief, Nov 13): the person may pick any bot of the card's space
//! instead ([`Via::Chosen`]). [`candidates`] lists who they can pick, the rule's pick first.

use crate::db::models::{Kwant, Team};
use crate::db::spaces::SpaceId;
use crate::server::dev::facts::DevFacts;
use crate::server::AppState;

/// How the recipient was picked: stored on the finding as `sent_via`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Author,
    TeamLead,
    /// The person picked this bot ("Send to…"), even when it is the author bot or the lead.
    Chosen,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Author => "author",
            Via::TeamLead => "team_lead",
            Via::Chosen => "chosen",
        }
    }
}

/// Why a bot gets a fix or notes, for the lines its task or message adds: the author bot gets none;
/// a lead and a picked bot that didn't write the PR are told so, and never to force-push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked<'a> {
    /// The PR's author bot (picked by the rule or by the person).
    Author,
    /// The lead of this team, the one that owns the repo, because a person wrote the PR.
    TeamLead(&'a str),
    /// Picked by the person; names who wrote the PR (its author bot, else the person's login).
    Chosen(&'a str),
}

impl Asked<'_> {
    /// True when the bot didn't write the PR: it pushes a new commit, never force-pushes.
    pub fn not_the_author(self) -> bool {
        !matches!(self, Asked::Author)
    }
}

/// Who wrote the PR, for [`Asked::Chosen`]: its author bot's name, else the person's login, else
/// their git name.
pub fn writer(facts: Option<&DevFacts>, kwants: &[Kwant]) -> String {
    let Some(f) = facts else { return "Someone else".into() };
    if let Some(k) = author_bot(f, kwants) {
        return k.name.clone();
    }
    let login = f.author.login.as_deref().map(str::trim).filter(|l| !l.is_empty());
    let name = Some(f.author.name.trim()).filter(|n| !n.is_empty());
    login.or(name).unwrap_or("Someone else").to_string()
}

impl Recipient {
    /// Why this recipient gets it. `writer` is [`writer`]'s name; a chosen bot that is the PR's
    /// author bot (`author_id`) is asked as the author.
    pub fn asked<'a>(&'a self, author_id: Option<&str>, writer: &'a str) -> Asked<'a> {
        match self.via {
            Via::Author => Asked::Author,
            Via::TeamLead => Asked::TeamLead(self.team.as_deref().unwrap_or("its team")),
            Via::Chosen if author_id == Some(self.bot.id.as_str()) => Asked::Author,
            Via::Chosen => Asked::Chosen(writer),
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
    let team = owning_team(state, space, repo_key).await?;
    let lead = team_lead(&team, space.as_str(), &kwants)?;
    Some(Recipient { bot: lead.clone(), via: Via::TeamLead, team: Some(team.name.clone()) })
}

/// The team that owns the card's repo (`dev_repos.team`), when there is one.
pub async fn owning_team(state: &AppState, space: &SpaceId, repo_key: &str) -> Option<Team> {
    let team_id = state.db.dev_repo(space.as_str(), repo_key).await.ok().flatten()?.team?;
    state.db.get_team(&team_id).await.ok().flatten()
}

/// The bot `id` when it is a bot of `space`, as [`Via::Chosen`]. None: not a bot there (another
/// space's, deleted, or no such id).
pub async fn chosen(state: &AppState, space: &SpaceId, id: &str) -> Option<Recipient> {
    let id = id.trim();
    let kwants = state.db.list_kwants(space).await.ok()?;
    let bot = kwants.into_iter().find(|k| k.id == id)?;
    Some(Recipient { bot, via: Via::Chosen, team: None })
}

/// The 400 when a "Send to…" bot isn't one of the card's space.
pub const BAD_RECIPIENT: &str = "That bot isn't in this space";

/// The finding's error when the chosen bot left the space during the Undo window (or before a
/// redelivery after a restart).
pub fn chosen_gone(name: &str) -> String {
    let name = if name.trim().is_empty() { "That bot" } else { name.trim() };
    format!("{name} isn't in this space any more. Pick another bot and press Fix it again.")
}

/// The ids of a team's lead and members.
fn team_ids(team: &Team) -> Vec<String> {
    let mut ids: Vec<String> = serde_json::from_str(&team.member_kwants).unwrap_or_default();
    ids.push(team.lead_kwant_id.trim().to_string());
    ids
}

/// "Send to…" order for `kwants` (the space's bots): `default` first, then the lead and members of
/// `owner` (the team that owns the repo, when in `space`), then the rest; by name within each.
/// Each bot carries a team: `owner` when it is in it, else the first of `teams` (by name) it leads
/// or is in.
pub fn order<'a>(kwants: &'a [Kwant], default: Option<&str>, owner: Option<&Team>, teams: &[Team], space: &str) -> Vec<(&'a Kwant, Option<String>)> {
    let owner = owner.filter(|t| t.space_id == space);
    let owner_ids = owner.map(team_ids).unwrap_or_default();
    let mut teams: Vec<&Team> = teams.iter().filter(|t| t.space_id == space).collect();
    teams.sort_by_key(|t| t.name.to_lowercase());
    let team_of = |k: &Kwant| -> Option<String> {
        if owner_ids.contains(&k.id) {
            return owner.map(|t| t.name.clone());
        }
        teams.iter().find(|t| team_ids(t).contains(&k.id)).map(|t| t.name.clone())
    };
    let rank = |k: &Kwant| {
        if Some(k.id.as_str()) == default {
            0
        } else if owner_ids.contains(&k.id) {
            1
        } else {
            2
        }
    };
    let mut out: Vec<&Kwant> = kwants.iter().collect();
    out.sort_by_key(|k| (rank(k), k.name.to_lowercase(), k.id.clone()));
    out.into_iter().map(|k| (k, team_of(k))).collect()
}

/// Who a card's fix or notes can go to: the rule's pick ([`pick`]) and every bot of the space in
/// [`order`].
pub async fn candidates(state: &AppState, space: &SpaceId, repo_key: &str, facts: Option<&DevFacts>) -> (Option<Recipient>, Vec<(Kwant, Option<String>)>) {
    let default = pick(state, space, repo_key, facts).await;
    let kwants = state.db.list_kwants(space).await.unwrap_or_default();
    let teams = state.db.list_teams(space).await.unwrap_or_default();
    let owner = owning_team(state, space, repo_key).await;
    let bots = order(&kwants, default.as_ref().map(|r| r.bot.id.as_str()), owner.as_ref(), &teams, space.as_str()).into_iter().map(|(k, t)| (k.clone(), t)).collect();
    (default, bots)
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
        assert_eq!(Via::Chosen.as_str(), "chosen");
        assert_eq!(chosen_gone("Pixel"), "Pixel isn't in this space any more. Pick another bot and press Fix it again.");
        assert!(chosen_gone(" ").starts_with("That bot isn't"));
    }

    #[test]
    fn the_send_to_list_is_the_default_then_the_owning_team_then_the_rest() {
        let ks = vec![kwant("k1", "zed"), kwant("k2", "Ada"), kwant("k3", "Pixel"), kwant("k4", "Lin"), kwant("k5", "Bea")];
        let mut owner = team("s1", "k4");
        owner.member_kwants = r#"["k1"]"#.into();
        let mut other = team("s1", "k5");
        other.id = "t2".into();
        other.name = "Web".into();
        let ids = |v: Vec<(&Kwant, Option<String>)>| v.into_iter().map(|(k, t)| format!("{}:{}", k.id, t.unwrap_or_default())).collect::<Vec<_>>();
        let teams = vec![owner.clone(), other.clone()];
        assert_eq!(ids(order(&ks, Some("k3"), Some(&owner), &teams, "s1")), ["k3:", "k4:Core", "k1:Core", "k2:", "k5:Web"]);
        // No default and no owner: all by name
        assert_eq!(ids(order(&ks, None, None, &[], "s1")), ["k2:", "k5:", "k4:", "k3:", "k1:"]);
        // An owner in another space counts for nothing
        owner.space_id = "s2".into();
        assert_eq!(ids(order(&ks, None, Some(&owner), &[], "s1"))[0], "k2:");
    }

    #[test]
    fn a_chosen_bot_is_told_who_wrote_it_unless_it_did() {
        let ks = vec![kwant("k1", "Pixel"), kwant("k2", "Ada")];
        let r = |id: &str, via: Via| Recipient { bot: ks.iter().find(|k| k.id == id).unwrap().clone(), via, team: Some("Edge".into()) };
        let (a, b, c) = (r("k1", Via::Chosen), r("k2", Via::Chosen), r("k2", Via::TeamLead));
        assert_eq!(a.asked(Some("k1"), "Pixel"), Asked::Author);
        assert_eq!(b.asked(Some("k1"), "Pixel"), Asked::Chosen("Pixel"));
        assert_eq!(c.asked(None, "oleks"), Asked::TeamLead("Edge"));
        assert!(!Asked::Author.not_the_author() && Asked::Chosen("x").not_the_author());
        // Who wrote it: the author bot, else the person's login, else their name
        assert_eq!(writer(Some(&facts(Some("k1"), None)), &ks), "Pixel");
        let mut f = facts(None, Some("oleks/x"));
        assert_eq!(writer(Some(&f), &ks), "Oleks");
        f.author.login = Some("oleks".into());
        assert_eq!(writer(Some(&f), &ks), "oleks");
        assert_eq!(writer(None, &ks), "Someone else");
    }
}
