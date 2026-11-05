//! GitHub issues and pull requests the person opened or is assigned to, read with the REST API
//! (GET only) and a token from the password vault. The token is only ever sent to the API base
//! the person configured; every URL is built from it, never taken from a response.

use super::{clip, ConvKind, Conversation, MissionSource, Msg, SourceKind, MAX_MESSAGE_CHARS};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const DEFAULT_API: &str = "https://api.github.com";
/// Most issues read per scan, and most whose comments are read.
const MAX_ISSUES: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GithubConfig {
    /// `https://api.github.com`, or a GitHub Enterprise `https://host/api/v3`.
    #[serde(default = "default_api")]
    pub api_base: String,
}

fn default_api() -> String {
    DEFAULT_API.into()
}

impl GithubConfig {
    pub fn validate(&self) -> Result<(), String> {
        let url = url::Url::parse(self.api_base.trim()).map_err(|_| "api_base must be a URL like https://api.github.com".to_string())?;
        let host = url.host_str().unwrap_or_default();
        let ok = match url.scheme() {
            "https" => !host.is_empty(),
            // A local mock server in tests, never a remote host in the clear
            "http" => super::imap::is_loopback(host),
            _ => false,
        };
        if !ok || !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
            return Err("api_base must be an https URL like https://api.github.com".into());
        }
        Ok(())
    }

    pub fn base(&self) -> String {
        self.api_base.trim().trim_end_matches('/').to_string()
    }

    /// The vault origin the token is saved under.
    pub fn vault_origin(&self) -> String {
        url::Url::parse(self.api_base.trim()).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
    }
}

pub struct GithubSource {
    id: String,
    cfg: GithubConfig,
    token: Zeroizing<String>,
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct User {
    login: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    html_url: String,
    repository_url: String,
    user: User,
    #[serde(default)]
    assignees: Vec<User>,
    created_at: DateTime<Utc>,
    #[serde(default)]
    comments: u64,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Comment {
    user: User,
    #[serde(default)]
    body: Option<String>,
    created_at: DateTime<Utc>,
}

fn is_bot(u: &User) -> bool {
    u.kind.eq_ignore_ascii_case("bot") || u.login.ends_with("[bot]")
}

impl GithubSource {
    pub fn new(id: &str, cfg: GithubConfig, token: Zeroizing<String>) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("kwnt-zombie-hunter")
            .build()
            .unwrap_or_default();
        Self { id: id.into(), cfg, token, http }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path_and_query: &str) -> Result<T, String> {
        let url = format!("{}{path_and_query}", self.cfg.base());
        let res = self
            .http
            .get(&url)
            .bearer_auth(self.token.as_str())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| format!("GitHub could not be reached: {}", e.without_url()))?;
        match res.status().as_u16() {
            200 => res.json().await.map_err(|_| "GitHub sent an answer kwant could not read".to_string()),
            401 => Err("GitHub refused the token (401): check it in the source settings".into()),
            403 | 429 => Err(format!("GitHub refused the request ({}): the token may lack access or hit a rate limit", res.status().as_u16())),
            s => Err(format!("GitHub answered {s}")),
        }
    }

    /// The token's account login.
    pub async fn test(&self) -> Result<String, String> {
        Ok(self.get::<User>("/user").await?.login)
    }
}

// ---------------------------------------------------------------------------------------------
// Dev mode reads (0.7, plan D4/D5). GET only, against the same API base and token. Lists that
// are polled go through an [`EtagCache`], so an unchanged answer is a 304 that GitHub doesn't
// count against the rate limit; 403/429 with a rate-limit sign come back as [`GhError::Limited`]
// so the poller can back off.

/// Why a Dev mode read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhError {
    /// 401: the token is wrong or expired.
    Auth,
    /// 403 without a rate-limit sign: the token can't read this.
    Forbidden,
    /// 404 (GitHub also answers 404 for a private repo the token can't see).
    NotFound,
    /// 429, or 403 with `retry-after` or no requests left: wait this long.
    Limited { retry_after_s: u64 },
    Other(String),
}

impl std::fmt::Display for GhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GhError::Auth => write!(f, "GitHub refused the token (401): check it in Loose ends"),
            GhError::Forbidden => write!(f, "GitHub refused the request (403): the token may lack access to this repo"),
            GhError::NotFound => write!(f, "GitHub has no such repo or branch, or the token can't see it"),
            GhError::Limited { retry_after_s } => write!(f, "GitHub's rate limit was hit: kwant tries again in {} min", retry_after_s.div_ceil(60).max(1)),
            GhError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// How long to wait after a 403/429, from GitHub's own headers. `None` when the answer
/// carries no rate-limit sign (a plain 403 is about access, not speed).
pub fn retry_after(status: u16, headers: &reqwest::header::HeaderMap, now_epoch: u64) -> Option<u64> {
    let h = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(s) = h("retry-after").and_then(|v| v.parse::<u64>().ok()) {
        return Some(s.clamp(1, 3600));
    }
    if h("x-ratelimit-remaining") == Some("0") {
        let reset = h("x-ratelimit-reset").and_then(|v| v.parse::<u64>().ok()).unwrap_or(now_epoch + 60);
        return Some(reset.saturating_sub(now_epoch).clamp(1, 3600));
    }
    // GitHub's docs: a 429, or a secondary limit with neither header, waits at least a minute
    (status == 429).then_some(60)
}

/// Remembered answers by URL, for conditional requests. Bounded: the oldest half is dropped
/// when it grows past [`EtagCache::MAX`].
#[derive(Default)]
pub struct EtagCache {
    map: std::sync::Mutex<std::collections::HashMap<String, (u64, String, serde_json::Value)>>,
    tick: std::sync::atomic::AtomicU64,
}

impl EtagCache {
    const MAX: usize = 400;

    fn get(&self, url: &str) -> Option<(String, serde_json::Value)> {
        self.map.lock().ok()?.get(url).map(|(_, e, v)| (e.clone(), v.clone()))
    }

    fn put(&self, url: &str, etag: String, body: serde_json::Value) {
        let n = self.tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Ok(mut m) = self.map.lock() else { return };
        if m.len() >= Self::MAX {
            let mut ages: Vec<u64> = m.values().map(|(t, _, _)| *t).collect();
            ages.sort_unstable();
            let cut = ages[ages.len() / 2];
            m.retain(|_, (t, _, _)| *t > cut);
        }
        m.insert(url.to_string(), (n, etag, body));
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhRepo {
    pub full_name: String,
    pub default_branch: String,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhUser {
    pub login: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhGitPerson {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhCommitInner {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub author: GhGitPerson,
    #[serde(default)]
    pub committer: GhGitPerson,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhCommit {
    pub sha: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub commit: GhCommitInner,
    #[serde(default)]
    pub author: Option<GhUser>,
    #[serde(default)]
    pub parents: Vec<serde_json::Value>,
    /// Only on a single-commit read.
    #[serde(default)]
    pub files: Vec<GhFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhRef {
    #[serde(rename = "ref", default)]
    pub branch: String,
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhPull {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub user: Option<GhUser>,
    #[serde(default)]
    pub head: GhRef,
    #[serde(default)]
    pub base: GhRef,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    #[serde(default)]
    pub merged_by: Option<GhUser>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhFile {
    pub filename: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub previous_filename: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GhCheck {
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CheckRuns {
    #[serde(default)]
    check_runs: Vec<GhCheck>,
}

#[derive(Debug, Deserialize)]
struct Tag {
    name: String,
    #[serde(default)]
    commit: Option<TagCommit>,
}

#[derive(Debug, Deserialize)]
struct TagCommit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct ContentItem {
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
}

/// Most PR files read (GitHub stops at 3000; a card never needs that many).
pub const MAX_PR_FILES: usize = 300;
/// Most commits read back from the branch head per poll.
pub const MAX_COMMITS: usize = 50;

/// A path segment for an `owner/name` repo, a branch or a sha: only the characters GitHub
/// allows there, so nothing a repo or a response holds can change the URL's shape.
fn seg_ok(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// `owner/name` checked for use in a URL.
pub fn repo_path(repo: &str) -> Result<&str, GhError> {
    match repo.split_once('/') {
        Some((o, n)) if seg_ok(o) && seg_ok(n) => Ok(repo),
        _ => Err(GhError::Other("the repo must be owner/name".into())),
    }
}

/// A branch, path or ref for a query string or path, percent-encoded.
fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

impl GithubSource {
    /// A client for Dev mode reads against `cfg` (the Loose ends GitHub connection's API base).
    pub fn for_dev(id: &str, cfg: GithubConfig, token: Zeroizing<String>) -> Self {
        Self::new(id, cfg, token)
    }

    async fn get_dev(&self, path_and_query: &str, cache: Option<&EtagCache>) -> Result<serde_json::Value, GhError> {
        let url = format!("{}{path_and_query}", self.cfg.base());
        let cached = cache.and_then(|c| c.get(&url));
        let mut req = self
            .http
            .get(&url)
            .bearer_auth(self.token.as_str())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some((etag, _)) = &cached {
            req = req.header("If-None-Match", etag.as_str());
        }
        let res = req.send().await.map_err(|e| GhError::Other(format!("GitHub could not be reached: {}", e.without_url())))?;
        let status = res.status().as_u16();
        let now = Utc::now().timestamp().max(0) as u64;
        match status {
            200 => {
                let etag = res.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
                let body: serde_json::Value = res.json().await.map_err(|_| GhError::Other("GitHub sent an answer kwant could not read".into()))?;
                if let (Some(c), Some(e)) = (cache, etag) {
                    c.put(&url, e, body.clone());
                }
                Ok(body)
            }
            304 => cached.map(|(_, v)| v).ok_or_else(|| GhError::Other("GitHub answered 304 to a request kwant never made".into())),
            401 => Err(GhError::Auth),
            404 | 422 => Err(GhError::NotFound),
            403 | 429 => Err(match retry_after(status, res.headers(), now) {
                Some(s) => GhError::Limited { retry_after_s: s },
                None => GhError::Forbidden,
            }),
            s => Err(GhError::Other(format!("GitHub answered {s}"))),
        }
    }

    async fn get_as<T: serde::de::DeserializeOwned>(&self, path: &str, cache: Option<&EtagCache>) -> Result<T, GhError> {
        serde_json::from_value(self.get_dev(path, cache).await?).map_err(|_| GhError::Other("GitHub sent an answer kwant could not read".into()))
    }

    /// The repo and its default branch.
    pub async fn dev_repo(&self, repo: &str) -> Result<GhRepo, GhError> {
        self.get_as(&format!("/repos/{}", repo_path(repo)?), None).await
    }

    /// Whether `branch` exists.
    pub async fn dev_branch_exists(&self, repo: &str, branch: &str) -> Result<bool, GhError> {
        match self.get_dev(&format!("/repos/{}/branches/{}", repo_path(repo)?, enc(branch)), None).await {
            Ok(_) => Ok(true),
            Err(GhError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Names of the files and folders at the repo's root on `branch` (for the deploy hint).
    pub async fn dev_root_names(&self, repo: &str, branch: &str) -> Result<Vec<String>, GhError> {
        let items: Vec<ContentItem> = self.get_as(&format!("/repos/{}/contents?ref={}", repo_path(repo)?, enc(branch)), None).await?;
        Ok(items.into_iter().map(|i| if i.kind == "dir" { format!("{}/", i.name) } else { i.name }).collect())
    }

    /// Commits on `branch` newer than `last_seen`, oldest first. `None` when `last_seen` isn't
    /// among the newest [`MAX_COMMITS`] (a force-push, or a long pause): the caller then takes
    /// the newest ones only.
    pub async fn dev_commits_since(&self, repo: &str, branch: &str, last_seen: Option<&str>, cache: &EtagCache) -> Result<(Vec<GhCommit>, bool), GhError> {
        let list: Vec<GhCommit> = self
            .get_as(&format!("/repos/{}/commits?sha={}&per_page={MAX_COMMITS}", repo_path(repo)?, enc(branch)), Some(cache))
            .await?;
        let found = last_seen.and_then(|s| list.iter().position(|c| c.sha == s));
        let mut newer: Vec<GhCommit> = match found {
            Some(i) => list[..i].to_vec(),
            None => list,
        };
        newer.reverse();
        Ok((newer, found.is_some()))
    }

    /// One commit with its files.
    pub async fn dev_commit(&self, repo: &str, sha: &str) -> Result<GhCommit, GhError> {
        if !seg_ok(sha) {
            return Err(GhError::Other("bad sha".into()));
        }
        self.get_as(&format!("/repos/{}/commits/{sha}", repo_path(repo)?), None).await
    }

    /// The pull requests a commit belongs to.
    pub async fn dev_pulls_for_commit(&self, repo: &str, sha: &str) -> Result<Vec<GhPull>, GhError> {
        if !seg_ok(sha) {
            return Err(GhError::Other("bad sha".into()));
        }
        self.get_as(&format!("/repos/{}/commits/{sha}/pulls", repo_path(repo)?), None).await
    }

    pub async fn dev_pull(&self, repo: &str, number: u64) -> Result<GhPull, GhError> {
        self.get_as(&format!("/repos/{}/pulls/{number}", repo_path(repo)?), None).await
    }

    /// A PR's files with their patches, up to [`MAX_PR_FILES`].
    pub async fn dev_pull_files(&self, repo: &str, number: u64) -> Result<Vec<GhFile>, GhError> {
        let repo = repo_path(repo)?;
        let mut out = Vec::new();
        for page in 1..=(MAX_PR_FILES / 100) {
            let batch: Vec<GhFile> = self.get_as(&format!("/repos/{repo}/pulls/{number}/files?per_page=100&page={page}"), None).await?;
            let last = batch.len() < 100;
            out.extend(batch);
            if last {
                break;
            }
        }
        Ok(out)
    }

    /// A file's text at `sha` (Dev mode's package check and secrets scan): none when it isn't
    /// there, is a folder, isn't UTF-8 or is too big for the contents API (over 1 MB).
    pub async fn dev_file_at(&self, repo: &str, path: &str, sha: &str) -> Result<Option<String>, GhError> {
        if !seg_ok(sha) {
            return Err(GhError::Other("bad sha".into()));
        }
        let parts: Vec<&str> = path.split('/').collect();
        if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
            return Ok(None);
        }
        const SEG: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');
        let path = parts.iter().map(|p| percent_encoding::utf8_percent_encode(p, SEG).to_string()).collect::<Vec<_>>().join("/");
        let v = match self.get_dev(&format!("/repos/{}/contents/{path}?ref={}", repo_path(repo)?, enc(sha)), None).await {
            Ok(v) => v,
            Err(GhError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        if v["type"].as_str() != Some("file") || v["encoding"].as_str() != Some("base64") {
            return Ok(None);
        }
        use base64::Engine;
        let b64: String = v["content"].as_str().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| GhError::Other("GitHub sent a file kwant could not read".into()))?;
        Ok(String::from_utf8(bytes).ok())
    }

    /// Check runs on a commit.
    pub async fn dev_check_runs(&self, repo: &str, sha: &str, cache: &EtagCache) -> Result<Vec<GhCheck>, GhError> {
        if !seg_ok(sha) {
            return Err(GhError::Other("bad sha".into()));
        }
        let runs: CheckRuns = self.get_as(&format!("/repos/{}/commits/{sha}/check-runs?per_page=100", repo_path(repo)?), Some(cache)).await?;
        Ok(runs.check_runs)
    }

    /// Open PRs into `base`, most recently updated first.
    pub async fn dev_open_pulls(&self, repo: &str, base: &str, cache: &EtagCache) -> Result<Vec<GhPull>, GhError> {
        self.get_as(&format!("/repos/{}/pulls?state=open&base={}&sort=updated&direction=desc&per_page=50", repo_path(repo)?, enc(base)), Some(cache))
            .await
    }

    /// The newest tag's name and the sha it points at.
    pub async fn dev_latest_tag(&self, repo: &str, cache: &EtagCache) -> Result<Option<(String, Option<String>)>, GhError> {
        let tags: Vec<Tag> = self.get_as(&format!("/repos/{}/tags?per_page=1", repo_path(repo)?), Some(cache)).await?;
        Ok(tags.into_iter().next().map(|t| (t.name, t.commit.map(|c| c.sha))))
    }

    /// How many commits on `branch` touched `path` between `since` and `until` (counting at most 2).
    pub async fn dev_path_commits(&self, repo: &str, branch: &str, path: &str, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<usize, GhError> {
        let list: Vec<serde_json::Value> = self
            .get_as(
                &format!(
                    "/repos/{}/commits?sha={}&path={}&since={}&until={}&per_page=2",
                    repo_path(repo)?,
                    enc(branch),
                    enc(path),
                    since.format("%Y-%m-%dT%H:%M:%SZ"),
                    until.format("%Y-%m-%dT%H:%M:%SZ")
                ),
                None,
            )
            .await?;
        Ok(list.len())
    }
}

/// `owner/repo` from `https://api.github.com/repos/owner/repo`.
fn repo_of(repository_url: &str) -> Option<String> {
    let mut parts = repository_url.trim_end_matches('/').rsplit('/');
    let repo = parts.next()?;
    let owner = parts.next()?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && s != "." && s != "..";
    (ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}"))
}

#[async_trait::async_trait]
impl MissionSource for GithubSource {
    fn kind(&self) -> SourceKind {
        SourceKind::Github
    }

    fn id(&self) -> &str {
        &self.id
    }

    async fn conversations(&self, since: DateTime<Utc>) -> Result<Vec<Conversation>, String> {
        let me = self.get::<User>("/user").await?.login;
        let mut issues: Vec<Issue> = Vec::new();
        for filter in ["created", "assigned"] {
            let page: Vec<Issue> = self
                .get(&format!("/issues?filter={filter}&state=open&sort=updated&direction=desc&per_page=100&since={}", since.format("%Y-%m-%dT%H:%M:%SZ")))
                .await?;
            for i in page {
                if !issues.iter().any(|x| x.html_url == i.html_url) {
                    issues.push(i);
                }
            }
        }
        issues.truncate(MAX_ISSUES);
        let mut out = Vec::new();
        for issue in issues {
            let Some(repo) = repo_of(&issue.repository_url) else { continue };
            let mut comments: Vec<Comment> = Vec::new();
            if issue.comments > 0 {
                // The last page holds the newest comments: that is where the answer would be
                let last_page = issue.comments.div_ceil(100).max(1);
                comments = self.get(&format!("/repos/{repo}/issues/{}/comments?per_page=100&page={last_page}", issue.number)).await?;
            }
            let pr = issue.pull_request.is_some();
            let mut c = Conversation::new(
                SourceKind::Github,
                &self.id,
                &issue.html_url,
                if pr { ConvKind::PullRequest } else { ConvKind::Issue },
                &issue.title,
            );
            c.link = issue.html_url.starts_with("https://").then(|| issue.html_url.clone());
            c.reference = Some(format!("{repo}#{}", issue.number));
            c.open = issue.state == "open";
            c.mine = issue.user.login.eq_ignore_ascii_case(&me) || issue.assignees.iter().any(|a| a.login.eq_ignore_ascii_case(&me));
            let text = |b: &Option<String>| clip(&super::imap::strip_quoted(b.as_deref().unwrap_or("")), MAX_MESSAGE_CHARS);
            let body = text(&issue.body);
            c.messages.push(Msg {
                from: issue.user.login.clone(),
                outbound: issue.user.login.eq_ignore_ascii_case(&me),
                at: issue.created_at,
                text: if body.is_empty() { issue.title.clone() } else { body },
                auto: false,
                bulk: is_bot(&issue.user),
                message_id: None,
            });
            for cm in &comments {
                c.messages.push(Msg {
                    from: cm.user.login.clone(),
                    outbound: cm.user.login.eq_ignore_ascii_case(&me),
                    at: cm.created_at,
                    text: text(&cm.body),
                    // CI and dependency bots are not an answer
                    auto: is_bot(&cm.user),
                    bulk: is_bot(&cm.user),
                    message_id: None,
                });
            }
            c.messages.sort_by_key(|m| m.at);
            c.counterpart = c.messages.iter().rev().find(|m| !m.outbound && !m.auto).map(|m| format!("@{}", m.from));
            out.push(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_must_be_https_or_a_local_mock() {
        GithubConfig { api_base: DEFAULT_API.into() }.validate().unwrap();
        GithubConfig { api_base: "http://127.0.0.1:18931".into() }.validate().unwrap();
        assert!(GithubConfig { api_base: "http://api.github.com".into() }.validate().is_err());
        assert!(GithubConfig { api_base: "https://u:p@api.github.com".into() }.validate().is_err());
        assert!(GithubConfig { api_base: "file:///etc".into() }.validate().is_err());
        assert_eq!(GithubConfig { api_base: "https://ghe.example/api/v3/".into() }.vault_origin(), "https://ghe.example");
    }

    #[test]
    fn rate_limits_back_off_by_githubs_own_headers() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        assert_eq!(retry_after(403, &h, 1000), None, "a plain 403 is about access");
        assert_eq!(retry_after(429, &h, 1000), Some(60));
        h.insert("retry-after", HeaderValue::from_static("120"));
        assert_eq!(retry_after(403, &h, 1000), Some(120));
        let mut h = HeaderMap::new();
        h.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        h.insert("x-ratelimit-reset", HeaderValue::from_static("1300"));
        assert_eq!(retry_after(403, &h, 1000), Some(300));
        h.insert("x-ratelimit-reset", HeaderValue::from_static("99999"));
        assert_eq!(retry_after(403, &h, 1000), Some(3600), "never more than an hour");
    }

    #[test]
    fn dev_paths_take_only_well_formed_repos() {
        assert!(repo_path("alexsergivan/kwant").is_ok());
        for bad in ["kwant", "o/r/x", "../r", "o/..", "o/r?x=1", "-o/r", "o /r", ""] {
            assert!(repo_path(bad).is_err(), "{bad}");
        }
        assert_eq!(enc("feat/a b&c"), "feat%2Fa+b%26c");
    }

    #[test]
    fn the_etag_cache_stays_bounded() {
        let c = EtagCache::default();
        for i in 0..(EtagCache::MAX + 50) {
            c.put(&format!("u{i}"), "e".into(), serde_json::json!(i));
        }
        assert!(c.map.lock().unwrap().len() <= EtagCache::MAX);
        assert!(c.get(&format!("u{}", EtagCache::MAX + 49)).is_some(), "the newest stays");
    }

    #[test]
    fn repos_come_only_from_well_formed_urls() {
        assert_eq!(repo_of("https://api.github.com/repos/o/r").as_deref(), Some("o/r"));
        assert_eq!(repo_of("https://api.github.com/repos/o/.."), None);
        assert_eq!(repo_of("https://api.github.com/repos/o/r?x=1"), None);
    }

    /// A mock GitHub on a private local port (9481–9484 on the shared server, any free one
    /// otherwise): Dev mode's reads send the ETag back, reuse the cached answer on a 304, and
    /// report a rate limit with its wait.
    #[tokio::test]
    async fn dev_reads_against_a_mock_github() {
        use axum::{extract::State, http::HeaderMap, response::IntoResponse, routing::get, Json, Router};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[derive(Default)]
        struct Seen {
            commits: AtomicUsize,
            revalidated: AtomicUsize,
        }
        async fn commits(State(seen): State<Arc<Seen>>, headers: HeaderMap) -> axum::response::Response {
            seen.commits.fetch_add(1, Ordering::SeqCst);
            assert_eq!(headers.get("authorization").and_then(|v| v.to_str().ok()), Some("Bearer demo-token"));
            if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some("\"c3\"") {
                seen.revalidated.fetch_add(1, Ordering::SeqCst);
                return axum::http::StatusCode::NOT_MODIFIED.into_response();
            }
            let c = |sha: &str| serde_json::json!({"sha": sha, "commit": {"message": format!("demo {sha}"), "author": {"name": "Demo (kwant bot)", "email": "demo-bot@example.com"}}});
            ([("etag", "\"c3\"")], Json(serde_json::json!([c("c3"), c("c2"), c("c1")]))).into_response()
        }
        async fn pulls() -> axum::response::Response {
            (axum::http::StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")], "slow down").into_response()
        }
        async fn tags() -> Json<serde_json::Value> {
            Json(serde_json::json!([{"name": "v0.7.0", "commit": {"sha": "c2"}}]))
        }
        let seen = Arc::new(Seen::default());
        let app = Router::new()
            .route("/repos/demo-org/demo-app/commits", get(commits))
            .route("/repos/demo-org/demo-app/pulls", get(pulls))
            .route("/repos/demo-org/demo-app/tags", get(tags))
            .with_state(seen.clone());
        let mut listener = None;
        for port in [9481u16, 9482, 9483, 9484, 0] {
            if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                listener = Some(l);
                break;
            }
        }
        let listener = listener.expect("a local port for the mock");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let cfg = GithubConfig { api_base: base };
        cfg.validate().unwrap();
        let gh = GithubSource::for_dev("demo", cfg, Zeroizing::new("demo-token".into()));
        let cache = EtagCache::default();

        let (newer, found) = gh.dev_commits_since("demo-org/demo-app", "main", Some("c1"), &cache).await.unwrap();
        assert!(found);
        assert_eq!(newer.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>(), ["c2", "c3"], "oldest first, after last_seen");
        // The second read sends the ETag back and gets the same answer from the cache
        let (again, _) = gh.dev_commits_since("demo-org/demo-app", "main", Some("c2"), &cache).await.unwrap();
        assert_eq!(again.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>(), ["c3"]);
        assert_eq!(seen.commits.load(Ordering::SeqCst), 2);
        assert_eq!(seen.revalidated.load(Ordering::SeqCst), 1);
        // A last_seen GitHub no longer lists: everything it sent, and `found` says so
        let (all, found) = gh.dev_commits_since("demo-org/demo-app", "main", Some("gone"), &cache).await.unwrap();
        assert!(!found);
        assert_eq!(all.len(), 3);

        match gh.dev_open_pulls("demo-org/demo-app", "main", &cache).await {
            Err(GhError::Limited { retry_after_s }) => assert_eq!(retry_after_s, 120),
            other => panic!("expected a rate limit, got {other:?}"),
        }
        assert_eq!(gh.dev_latest_tag("demo-org/demo-app", &cache).await.unwrap(), Some(("v0.7.0".into(), Some("c2".into()))));
        assert!(matches!(gh.dev_latest_tag("demo-org/no-such", &cache).await, Err(GhError::NotFound)));
        server.abort();
    }
}
