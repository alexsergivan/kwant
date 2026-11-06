//! Host-neutral history types (any-git.md G3). The poller, `landed`, `review` and the scan read
//! these, never a host's own shapes: a commit is a [`Commit`], a pull or merge request is a
//! [`Change`], a changed file is a [`FileDiff`] and a CI run is a [`Check`]. Each host converts at
//! its own boundary (GitHub's REST shapes below; git itself in `git`), so a card made from a plain
//! git repo and one made from GitHub's API carry the same facts.

use super::facts::{Check, FileDiff};
use crate::zombie::github::{GhCheck, GhCommit, GhFile, GhGitPerson, GhPull};
use chrono::{DateTime, Utc};

/// A git author or committer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Person {
    pub name: String,
    pub email: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

/// One commit. `files` is filled only on a single-commit read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Commit {
    pub sha: String,
    /// The commit's web page, when the host has one.
    pub url: Option<String>,
    pub message: String,
    pub author: Person,
    pub committer: Person,
    /// The host account that wrote it, when the host knows one (GitHub matches the email).
    pub login: Option<String>,
    pub parents: Vec<String>,
    pub files: Vec<FileDiff>,
}

/// A pull request or merge request.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Change {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub draft: bool,
    pub url: Option<String>,
    /// Who opened it.
    pub author: Option<String>,
    pub head_branch: String,
    pub head_sha: String,
    pub base_branch: String,
    pub merged_at: Option<DateTime<Utc>>,
    /// The commit its merge (or squash) put on the base branch.
    pub merge_sha: Option<String>,
    pub merged_by: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<GhGitPerson> for Person {
    fn from(p: GhGitPerson) -> Self {
        Person { name: p.name, email: p.email, date: p.date }
    }
}

impl From<GhFile> for FileDiff {
    fn from(f: GhFile) -> Self {
        FileDiff { path: f.filename, status: f.status, additions: f.additions, deletions: f.deletions, patch: f.patch, previous_path: f.previous_filename }
    }
}

impl From<GhCheck> for Check {
    /// The link as GitHub sent it; the poller keeps only https ones.
    fn from(c: GhCheck) -> Self {
        Check { name: c.name, status: c.status, conclusion: c.conclusion, url: c.html_url }
    }
}

impl From<GhCommit> for Commit {
    fn from(c: GhCommit) -> Self {
        Commit {
            sha: c.sha,
            url: c.html_url,
            message: c.commit.message,
            author: c.commit.author.into(),
            committer: c.commit.committer.into(),
            login: c.author.map(|u| u.login),
            parents: c.parents.iter().filter_map(|p| p["sha"].as_str().map(str::to_string)).collect(),
            files: c.files.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<GhPull> for Change {
    fn from(p: GhPull) -> Self {
        Change {
            number: p.number,
            title: p.title,
            body: p.body,
            draft: p.draft,
            url: p.html_url,
            author: p.user.map(|u| u.login),
            head_branch: p.head.branch,
            head_sha: p.head.sha,
            base_branch: p.base.branch,
            merged_at: p.merged_at,
            merge_sha: p.merge_commit_sha,
            merged_by: p.merged_by.map(|u| u.login),
            updated_at: p.updated_at,
        }
    }
}

/// Converts a list at the host boundary.
pub fn all<A, B: From<A>>(v: Vec<A>) -> Vec<B> {
    v.into_iter().map(B::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_shapes_convert_without_losing_a_field() {
        let c: GhCommit = serde_json::from_value(json!({
            "sha": "m1", "html_url": "https://github.com/demo-org/app/commit/m1",
            "commit": {"message": "Fix\n\nWhy", "author": {"name": "Lin", "email": "lin-bot@example.com", "date": "2026-11-05T01:00:00Z"},
                       "committer": {"name": "GitHub", "date": "2026-11-05T02:00:00Z"}},
            "author": {"login": "lin-bot"}, "parents": [{"sha": "p1"}, {"sha": "p2"}],
            "files": [{"filename": "b.rs", "status": "renamed", "additions": 2, "deletions": 1, "patch": "@@ x", "previous_filename": "a.rs"}]
        }))
        .unwrap();
        let n = Commit::from(c);
        assert_eq!(n.sha, "m1");
        assert_eq!(n.url.as_deref(), Some("https://github.com/demo-org/app/commit/m1"));
        assert_eq!((n.author.name.as_str(), n.author.email.as_deref()), ("Lin", Some("lin-bot@example.com")));
        assert_eq!(n.committer.date.unwrap().to_rfc3339(), "2026-11-05T02:00:00+00:00");
        assert_eq!(n.login.as_deref(), Some("lin-bot"));
        assert_eq!(n.parents, ["p1", "p2"]);
        assert_eq!(n.files[0], FileDiff { path: "b.rs".into(), status: "renamed".into(), additions: 2, deletions: 1, patch: Some("@@ x".into()), previous_path: Some("a.rs".into()) });

        let p: GhPull = serde_json::from_value(json!({
            "number": 7, "title": "T", "body": "B", "draft": true, "html_url": "https://github.com/demo-org/app/pull/7",
            "user": {"login": "oleks"}, "head": {"ref": "feat", "sha": "h1"}, "base": {"ref": "main", "sha": "b1"},
            "merged_at": "2026-11-05T01:00:00Z", "merge_commit_sha": "m1", "merged_by": {"login": "ada"}
        }))
        .unwrap();
        let ch = Change::from(p);
        assert_eq!((ch.number, ch.title.as_str(), ch.body.as_deref(), ch.draft), (7, "T", Some("B"), true));
        assert_eq!((ch.head_branch.as_str(), ch.head_sha.as_str(), ch.base_branch.as_str()), ("feat", "h1", "main"));
        assert_eq!((ch.author.as_deref(), ch.merged_by.as_deref(), ch.merge_sha.as_deref()), (Some("oleks"), Some("ada"), Some("m1")));

        let k: GhCheck = serde_json::from_value(json!({"name": "rust", "status": "completed", "conclusion": "success", "html_url": "https://x"})).unwrap();
        assert_eq!(Check::from(k), Check { name: "rust".into(), status: "completed".into(), conclusion: Some("success".into()), url: Some("https://x".into()) });
    }
}
