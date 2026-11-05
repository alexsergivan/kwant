//! What the poller learned about one merge or one open PR head (plan §3.3). Lin writes it into
//! `dev_cards.facts`; card generation (`generate`) reads it. Facts only: nothing here is model text.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Most diff characters kept across a card's files (plan D8).
pub const DIFF_CHARS: usize = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Landed,
    Review,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevFacts {
    /// `owner/name`.
    pub repo: String,
    pub kind: Kind,
    /// The PR number: a review always has one; a landed card when the merge came from a PR.
    pub pr: Option<u64>,
    /// PR title, else the commit's first line.
    pub title: String,
    /// PR body, else the rest of the commit message.
    pub body: String,
    /// Landed: the merge (or pushed) commit. Review: the PR head.
    pub head_sha: String,
    pub base_branch: String,
    pub merged_at: Option<DateTime<Utc>>,
    /// GitHub login of whoever merged it (landed only).
    pub merged_by: Option<String>,
    pub author: Author,
    pub files: Vec<FileDiff>,
    /// `"diff cut, N files not read"` when the diff went past [`DIFF_CHARS`], else none.
    pub cut: Option<String>,
    pub checks: Vec<Check>,
    pub ci: CiState,
    /// The kwant task, mission or thread text the PR came from, if known.
    pub task: Option<String>,
    /// Files with no earlier commit by anyone in the last 90 days.
    pub first_edit_files: Vec<String>,
    /// Review only: GitHub marks the PR a draft (the poller skips drafts, D8).
    #[serde(default)]
    pub draft: bool,
    /// Review only: the PR's head branch.
    #[serde(default)]
    pub head_branch: Option<String>,
    /// Links for the card footer: PR, commit and CI pages (https only).
    #[serde(default)]
    pub links: Links,
    /// Landed from a PR only: the PR's head commit when it merged (`head_sha` is then the merge
    /// commit). A review counts for the landed card only if it reviewed this commit.
    #[serde(default)]
    pub pr_head_sha: Option<String>,
    /// Review only: the gitleaks and new-package checks over the whole diff (Juno, plan §3.4).
    /// None when the head wasn't scanned (landed cards, cards written before the scan existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<super::scan::Security>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    pub pr: Option<String>,
    pub commit: Option<String>,
    pub ci: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Author {
    /// GitHub login, when the commit is linked to an account.
    pub login: Option<String>,
    /// Git author name.
    pub name: String,
    /// Git author email.
    pub email: Option<String>,
    /// The kwant bot this was matched to by name or email (plan D7); none means the person.
    pub bot: Option<KwantRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KwantRef {
    pub id: String,
    pub name: String,
    /// `claude`, `codex`, `grok`, …
    pub engine: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    /// GitHub's file status: `added`, `modified`, `removed`, `renamed`, `copied`, `changed`, `unchanged`.
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    /// The unified diff hunk text, cut to fit [`DIFF_CHARS`]; none for binary, very large or unread files.
    pub patch: Option<String>,
    pub previous_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    /// `queued`, `in_progress` or `completed`.
    pub status: String,
    /// When completed: `success`, `failure`, `neutral`, `cancelled`, `skipped`, `timed_out`, `action_required`.
    pub conclusion: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    Green,
    Red,
    Pending,
    None,
}

impl CiState {
    /// Red if any completed check failed, else pending while any runs, else green; no checks is none.
    pub fn of(checks: &[Check]) -> CiState {
        if checks.is_empty() {
            return CiState::None;
        }
        let bad = |c: &Check| matches!(c.conclusion.as_deref(), Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure"));
        if checks.iter().any(bad) {
            CiState::Red
        } else if checks.iter().any(|c| c.status != "completed") {
            CiState::Pending
        } else {
            CiState::Green
        }
    }
}

/// What a `release_notes` card is drafted from: the landed cards since the last tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseFacts {
    pub repo: String,
    /// The newest tag on the watched branch, if any (`mac-v0.6.12`).
    pub since_tag: Option<String>,
    /// The landed `dev_cards` ids it covers, oldest first.
    pub cards: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: &str, conclusion: Option<&str>) -> Check {
        Check { name: "rust".into(), status: status.into(), conclusion: conclusion.map(Into::into), url: None }
    }

    #[test]
    fn ci_state_reads_failures_first_then_running_checks() {
        assert_eq!(CiState::of(&[]), CiState::None);
        assert_eq!(CiState::of(&[check("completed", Some("success")), check("completed", Some("skipped"))]), CiState::Green);
        assert_eq!(CiState::of(&[check("completed", Some("success")), check("in_progress", None)]), CiState::Pending);
        assert_eq!(CiState::of(&[check("in_progress", None), check("completed", Some("failure"))]), CiState::Red);
    }

    #[test]
    fn facts_round_trip_as_snake_case_json() {
        let f = DevFacts {
            repo: "o/r".into(),
            kind: Kind::Review,
            pr: Some(7),
            title: "t".into(),
            body: String::new(),
            head_sha: "abc".into(),
            base_branch: "main".into(),
            merged_at: None,
            merged_by: None,
            author: Author::default(),
            files: vec![],
            cut: None,
            checks: vec![],
            ci: CiState::None,
            task: None,
            first_edit_files: vec![],
            draft: false,
            head_branch: Some("b".into()),
            links: Links::default(),
            pr_head_sha: None,
            security: None,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["kind"], "review");
        assert_eq!(v["ci"], "none");
        assert_eq!(serde_json::from_value::<DevFacts>(v).unwrap(), f);
    }
}
