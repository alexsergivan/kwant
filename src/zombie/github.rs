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
    fn repos_come_only_from_well_formed_urls() {
        assert_eq!(repo_of("https://api.github.com/repos/o/r").as_deref(), Some("o/r"));
        assert_eq!(repo_of("https://api.github.com/repos/o/.."), None);
        assert_eq!(repo_of("https://api.github.com/repos/o/r?x=1"), None);
    }
}
