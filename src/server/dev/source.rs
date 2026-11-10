//! Host-neutral history types (any-git.md G3). The poller, `landed`, `review` and the scan read
//! these, never a host's own shapes: a commit is a [`Commit`], a pull or merge request is a
//! [`Change`], a changed file is a [`FileDiff`] and a CI run is a [`Check`]. Each host converts at
//! its own boundary (GitHub's REST shapes below; git itself in `git`), so a card made from a plain
//! git repo and one made from GitHub's API carry the same facts.

use super::facts::{Check, FileDiff};
use super::git::{Auth, GitError, Repo};
use super::forge::ForgeKind;
use crate::zombie::github::{EtagCache, GhCheck, GhCommit, GhError, GhFile, GhGitPerson, GhPull, GhReview, GhReviewComment, GithubSource, PostError, ReviewPost};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

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

/// One inline review comment on a change (0.9.0, `dev::fix_it`): a file line and what was said.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReviewComment {
    pub id: u64,
    pub path: String,
    /// The line it's on (the last line of a range), in the file at `commit`; none when it no longer
    /// maps to a line of the current diff and the forge didn't keep the original.
    pub line: Option<u64>,
    /// The first line of a multi-line comment.
    pub start_line: Option<u64>,
    pub body: String,
    /// The forge account that wrote it.
    pub author: Option<String>,
    /// The author's tie to the repo as the forge says it (GitHub: OWNER, MEMBER, COLLABORATOR,
    /// CONTRIBUTOR, NONE, …); none when the forge doesn't say.
    pub author_association: Option<String>,
    /// The commit the comment was made on.
    pub commit: Option<String>,
    pub url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
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

impl From<GhReviewComment> for ReviewComment {
    fn from(c: GhReviewComment) -> Self {
        ReviewComment {
            id: c.id,
            path: c.path,
            line: c.line.or(c.original_line),
            start_line: c.start_line.or(c.original_start_line),
            body: c.body,
            author: c.user.map(|u| u.login),
            author_association: c.author_association,
            commit: c.commit_id,
            url: c.html_url,
            created_at: c.created_at,
        }
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

impl From<GitError> for GhError {
    /// git's fixed words, as a poll error the desk shows.
    fn from(e: GitError) -> Self {
        GhError::Other(e.to_string())
    }
}

/// Where a watched repo's history comes from (any-git.md G1): git first (a mirror kwant keeps, or
/// a team folder's own `.git`), GitHub's API when git can't answer. Commits, diffs, files, tags,
/// path history and root names all come through here; what only a forge knows (pull requests,
/// checks) comes through [`Forge`].
pub struct History<'a> {
    git: Option<Repo>,
    /// What the mirror fetches from (empty for a team folder), and the header that goes with it.
    url: String,
    auth: Option<Auth>,
    api: Option<(&'a GithubSource, &'a str)>,
    branch: String,
    /// `<base>/<sha>` is a commit's web page.
    commit_base: Option<String>,
}

impl<'a> History<'a> {
    pub fn new(git: Option<Repo>, url: String, auth: Option<Auth>, api: Option<(&'a GithubSource, &'a str)>, branch: &str, commit_base: Option<String>) -> Self {
        History { git, url, auth, api, branch: branch.to_string(), commit_base }
    }

    pub fn has_git(&self) -> bool {
        self.git.is_some()
    }

    /// Brings the mirror up to date. When that fails and the API can answer, git is set aside for
    /// this poll; without an API, the error is the poll's.
    pub async fn fetch(&mut self) -> Result<(), GhError> {
        let Some(g) = &self.git else { return Ok(()) };
        match g.fetch(&self.url, &self.branch, self.auth.as_ref()).await {
            Ok(()) => Ok(()),
            Err(_) if self.api.is_some() => {
                self.git = None;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    fn no_source() -> GhError {
        GitError::Failed.into()
    }

    fn with_url(&self, mut c: Commit) -> Commit {
        if c.url.is_none() {
            c.url = self.commit_base.as_ref().map(|b| format!("{b}/{}", c.sha));
        }
        c
    }

    /// The tag nearest the branch tip and when its commit was made. `known` is what the last poll
    /// saw: the API isn't asked for a date it already gave.
    pub async fn latest_tag(&self, known: Option<&(String, Option<DateTime<Utc>>)>, cache: &EtagCache) -> Result<Option<(String, Option<DateTime<Utc>>)>, GhError> {
        if let Some(g) = &self.git {
            if let Ok(t) = g.latest_tag(&self.branch).await {
                return Ok(t);
            }
        }
        let (client, repo) = self.api.ok_or_else(Self::no_source)?;
        let Some((name, sha)) = client.dev_latest_tag(repo, cache).await? else { return Ok(None) };
        if let Some(k) = known.filter(|(n, at)| *n == name && at.is_some()) {
            return Ok(Some(k.clone()));
        }
        let at = match sha {
            Some(s) => client.dev_commit(repo, &s).await.ok().map(Commit::from).and_then(|c| c.committer.date.or(c.author.date)),
            None => None,
        };
        Ok(Some((name, at)))
    }

    /// Commits on the branch newer than `last_seen`, oldest first, and whether `last_seen` was
    /// found (see [`Repo::commits_since`]).
    pub async fn commits_since(&self, last_seen: Option<&str>, cache: &EtagCache) -> Result<(Vec<Commit>, bool), GhError> {
        if let Some(g) = &self.git {
            match g.commits_since(&self.branch, last_seen).await {
                Ok((list, found)) => return Ok((list.into_iter().map(|c| self.with_url(c)).collect(), found)),
                Err(e) if self.api.is_none() => return Err(e.into()),
                Err(_) => {}
            }
        }
        let (client, repo) = self.api.ok_or_else(Self::no_source)?;
        let (list, found) = client.dev_commits_since(repo, &self.branch, last_seen, cache).await?;
        Ok((all(list), found))
    }

    /// One commit with its files against its first parent.
    pub async fn commit(&self, sha: &str) -> Result<Commit, GhError> {
        if let Some(g) = &self.git {
            match g.commit(sha).await {
                Ok(c) => return Ok(self.with_url(c)),
                Err(e) if self.api.is_none() => return Err(e.into()),
                Err(_) => {}
            }
        }
        let (client, repo) = self.api.ok_or_else(Self::no_source)?;
        Ok(client.dev_commit(repo, sha).await?.into())
    }

    /// What landed with `head`: the files from `base` (the commit before the merge, or before the
    /// first commit of a rebase-merge) to `head`. `None` when git can't say (the caller asks the
    /// forge for the PR's files instead).
    pub async fn landed_files(&self, base: Option<&str>, head: &str) -> Option<Vec<FileDiff>> {
        self.git.as_ref()?.diff(base, head).await.ok()
    }

    /// An open change's files, from where it grew off `base_branch` to its head, read from the
    /// forge's head ref (`head_ref`, fetched into the mirror). `None` when git can't say.
    pub async fn change_files(&self, head_ref: Option<&str>, number: u64, head_sha: &str, base_branch: &str) -> Option<Vec<FileDiff>> {
        let g = self.git.as_ref()?;
        let local = format!("refs/kwant/pull/{number}");
        if g.resolve(head_sha).await.is_err() {
            g.fetch_ref(&self.url, head_ref?, &local, self.auth.as_ref()).await.ok()?;
        }
        let got = async {
            if g.resolve(head_sha).await.ok()? != head_sha {
                return None;
            }
            let base = g.merge_base(&format!("refs/heads/{base_branch}"), head_sha).await.ok()?;
            g.diff(Some(&base), head_sha).await.ok()
        }
        .await;
        g.drop_ref(&local).await;
        got
    }

    /// The repo's branches and their heads (see [`Repo::heads`]); empty with no git.
    pub async fn branches(&self) -> Result<Vec<(String, String)>, GhError> {
        match &self.git {
            Some(g) => Ok(g.heads(&self.url, self.auth.as_ref()).await?),
            None => Ok(Vec::new()),
        }
    }

    /// What branch `name` (head `sha`) changes against `base_branch` (any-git.md G5): the diff from
    /// their merge base to `sha`. A mirror fetches the branch into `refs/kwant/branch/<name>` for
    /// the read and drops it after. `None` when it has nothing new (its head is already in the base
    /// branch), or git couldn't read it.
    pub async fn branch_files(&self, name: &str, sha: &str, base_branch: &str) -> Option<Vec<FileDiff>> {
        let g = self.git.as_ref()?;
        let local = format!("refs/kwant/branch/{name}");
        let fetched = if g.is_local() || g.resolve(sha).await.is_ok() {
            false
        } else {
            g.fetch_ref(&self.url, &format!("refs/heads/{name}"), &local, self.auth.as_ref()).await.ok()?;
            true
        };
        let got = async {
            if g.resolve(sha).await.ok()? != sha {
                return None;
            }
            let base_ref = format!("refs/heads/{base_branch}");
            if g.is_ancestor(sha, &base_ref).await.ok()? {
                return None;
            }
            let base = g.merge_base(&base_ref, sha).await.ok()?;
            g.diff(Some(&base), sha).await.ok()
        }
        .await;
        if fetched {
            g.drop_ref(&local).await;
        }
        got
    }

    /// The committer dates of those of `shas` git already has (see [`Repo::commit_dates`]): every
    /// branch head of a team folder, and those a mirror fetched before (their objects stay after
    /// [`Self::branch_files`] drops its ref). Empty with no git, or when git can't read them.
    pub async fn commit_dates(&self, shas: &[&str]) -> HashMap<String, DateTime<Utc>> {
        match &self.git {
            Some(g) if !shas.is_empty() => g.commit_dates(shas).await.unwrap_or_default(),
            _ => HashMap::new(),
        }
    }

    /// Whether branch heads are fetched before they can be read (a mirror), rather than read in
    /// place (a team folder).
    pub fn fetches_branches(&self) -> bool {
        self.git.as_ref().is_some_and(|g| !g.is_local())
    }

    /// When branch `name`'s head `sha`, which the mirror doesn't have yet, was committed: fetches
    /// the branch the way [`Self::branch_files`] does, reads the date and drops the ref. The
    /// objects stay, so a fresh branch's [`Self::branch_files`] right after fetches nothing more,
    /// and later polls read the date with no fetch. `None` when the fetch failed or didn't bring
    /// `sha` (the branch moved since it was listed): the branch waits for the next poll, as when
    /// [`Self::branch_files`] can't read it.
    pub async fn fetch_branch_date(&self, name: &str, sha: &str) -> Option<DateTime<Utc>> {
        let g = self.git.as_ref()?;
        if g.resolve(sha).await.is_ok() {
            return g.commit_dates(&[sha]).await.ok().and_then(|d| d.get(sha).copied());
        }
        let local = format!("refs/kwant/branch/{name}");
        let at = match g.fetch_ref(&self.url, &format!("refs/heads/{name}"), &local, self.auth.as_ref()).await {
            Ok(_) => g.commit_dates(&[sha]).await.ok().and_then(|d| d.get(sha).copied()),
            Err(_) => None,
        };
        g.drop_ref(&local).await;
        at
    }

    /// Whether `sha` is already in `base_branch` (a pushed branch that merged, or never moved
    /// ahead). `false` when git can't tell (the mirror doesn't have `sha` yet).
    pub async fn in_branch(&self, sha: &str, base_branch: &str) -> bool {
        match &self.git {
            Some(g) => g.is_ancestor(sha, &format!("refs/heads/{base_branch}")).await.unwrap_or(false),
            None => false,
        }
    }

    /// A text file at `sha` (see [`Repo::file_at`]).
    pub async fn file_at(&self, path: &str, sha: &str) -> Result<Option<String>, GhError> {
        if let Some(g) = &self.git {
            match g.file_at(path, sha).await {
                Ok(t) => return Ok(t),
                Err(e) if self.api.is_none() => return Err(e.into()),
                Err(_) => {}
            }
        }
        let (client, repo) = self.api.ok_or_else(Self::no_source)?;
        client.dev_file_at(repo, path, sha).await
    }

    /// How many commits on the branch touched `path` in the window (at most 2).
    pub async fn path_commits(&self, path: &str, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<usize, GhError> {
        if let Some(g) = &self.git {
            match g.path_commits(&self.branch, path, since, until).await {
                Ok(n) => return Ok(n),
                Err(e) if self.api.is_none() => return Err(e.into()),
                Err(_) => {}
            }
        }
        let (client, repo) = self.api.ok_or_else(Self::no_source)?;
        client.dev_path_commits(repo, &self.branch, path, since, until).await
    }

    /// The names at the branch's root (for the deploy hint); git only.
    pub async fn root_names(&self) -> Option<Vec<String>> {
        self.git.as_ref()?.root_names(&self.branch).await.ok()
    }
}

/// What only a forge knows about a repo: its pull or merge requests and CI checks (any-git.md
/// G4 seam). GitHub's API implements it ([`GithubForge`]); a plain git repo has none, so its
/// commits get landed cards without a PR or checks, and its pushed branches review cards without
/// CI or Post (G5). GitLab and Gitea implement it in `forge` (G4).
#[async_trait::async_trait]
pub trait Forge: Send + Sync {
    /// Which forge this is.
    fn kind(&self) -> ForgeKind;
    /// The changes `sha` belongs to (merged ones carry the merge sha).
    async fn changes_for_commit(&self, sha: &str) -> Result<Vec<Change>, GhError>;
    /// The open changes into `base`.
    async fn open_changes(&self, base: &str, cache: &EtagCache) -> Result<Vec<Change>, GhError>;
    /// The CI checks on `sha`.
    async fn checks(&self, sha: &str, cache: &EtagCache) -> Result<Vec<Check>, GhError>;
    /// A change's files as the forge lists them (when git can't read them).
    async fn change_files(&self, number: u64) -> Result<Vec<FileDiff>, GhError>;
    /// The ref git fetches a change's head from (`refs/pull/7/head`, `refs/merge-requests/7/head`).
    fn change_head_ref(&self, number: u64) -> Option<String>;
    /// The forge account that wrote `sha`, when the forge matches it (a direct push has no change
    /// to name its author).
    async fn commit_login(&self, sha: &str) -> Option<String>;
    /// One change, with its head and base. `Ok(None)` when there is no such change.
    async fn change(&self, number: u64) -> Result<Option<Change>, GhError>;
    /// The account the token belongs to.
    async fn login(&self) -> Result<String, GhError>;
    /// Posts a review on change `number`. GitHub only until G7 (Lin): the others say so.
    async fn post_review(&self, number: u64, post: &ReviewPost) -> Result<GhReview, PostError>;
    /// The newest inline review comments on change `number` (0.9.0 `fix_it`, Juno). A forge with
    /// no reader for them yet has none.
    async fn review_comments(&self, _number: u64) -> Result<Vec<ReviewComment>, GhError> {
        Ok(Vec::new())
    }
    /// What failed check `name` on `sha` said (its output and annotations), as the forge has it,
    /// uncut. None when the forge can't tell; `fix_it` cuts and quotes it as data.
    async fn check_excerpt(&self, _sha: &str, _name: &str) -> Option<String> {
        None
    }
}

/// [`Forge`] over GitHub's REST API.
pub struct GithubForge<'a> {
    pub client: &'a GithubSource,
    pub repo: &'a str,
}

#[async_trait::async_trait]
impl Forge for GithubForge<'_> {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Github
    }
    async fn changes_for_commit(&self, sha: &str) -> Result<Vec<Change>, GhError> {
        Ok(all(self.client.dev_pulls_for_commit(self.repo, sha).await?))
    }
    async fn open_changes(&self, base: &str, cache: &EtagCache) -> Result<Vec<Change>, GhError> {
        Ok(all(self.client.dev_open_pulls(self.repo, base, cache).await?))
    }
    async fn checks(&self, sha: &str, cache: &EtagCache) -> Result<Vec<Check>, GhError> {
        Ok(all(self.client.dev_check_runs(self.repo, sha, cache).await?))
    }
    async fn change_files(&self, number: u64) -> Result<Vec<FileDiff>, GhError> {
        Ok(all(self.client.dev_pull_files(self.repo, number).await?))
    }
    fn change_head_ref(&self, number: u64) -> Option<String> {
        Some(format!("refs/pull/{number}/head"))
    }
    async fn commit_login(&self, sha: &str) -> Option<String> {
        self.client.dev_commit(self.repo, sha).await.ok().and_then(|c| c.author.map(|u| u.login))
    }
    async fn change(&self, number: u64) -> Result<Option<Change>, GhError> {
        match self.client.dev_pull(self.repo, number).await {
            Ok(p) => Ok(Some(p.into())),
            Err(GhError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn login(&self) -> Result<String, GhError> {
        self.client.dev_login().await
    }
    async fn post_review(&self, number: u64, post: &ReviewPost) -> Result<GhReview, PostError> {
        self.client.dev_post_review(self.repo, number, post).await
    }
    async fn review_comments(&self, number: u64) -> Result<Vec<ReviewComment>, GhError> {
        Ok(all(self.client.dev_review_comments(self.repo, number).await?))
    }
    async fn check_excerpt(&self, sha: &str, name: &str) -> Option<String> {
        self.client.dev_check_output(self.repo, sha, name).await.ok().flatten()
    }
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
