//! Forges other than GitHub (any-git.md G4): GitLab (`/api/v4`: merge requests, pipelines and
//! commit statuses, gitlab.com and self-hosted) and Gitea, Forgejo and Codeberg (`/api/v1`: pull
//! requests and commit statuses). Each implements [`Forge`], so the poller reads their changes and
//! CI the way it reads GitHub's.
//!
//! - **Which forge.** A host kwant knows ([`known`]: github.com, gitlab.com, codeberg.org,
//!   gitea.com) is that forge. Any other host is asked: `GET /api/v4/version` (GitLab answers, or
//!   says 401 the GitLab way), then `GET /api/v1/version` (Gitea and Forgejo answer). The answer
//!   is kept for a day per host ([`detect`]).
//! - **Tokens.** A forge's token is a Loose ends connection of kind `forge` ([`ForgeConfig`]): one
//!   per host, the token in the vault under the forge's origin. GitHub keeps its own `github` kind.
//! - **Addresses.** Every request goes through [`client`]: its host is looked up with the same
//!   [`git::Resolver`] and refused on the same [`git::blocked`] addresses as a git fetch (this
//!   machine, link-local, cloud metadata), and the connection goes to the very addresses that were
//!   checked (the resolver's answer is what reqwest connects to, so DNS can't change between check
//!   and connect). No proxy, no redirects. The one exception is a connection the person typed as
//!   `http://` to this machine (`http://127.0.0.1:…`), the same allowance GitHub's `api_base`
//!   has for a local test server; an address from a team's folder never gets it.
//! - **Posting** stays GitHub's until G7 (Lin): [`Forge::post_review`] on GitLab and Gitea says so.

use super::facts::{Check, FileDiff};
use super::git::{self, Auth, GitError, Resolver};
use super::source::{Change, Forge};
use crate::zombie::github::{EtagCache, GhError, GhReview, PostError, ReviewPost, MAX_PR_FILES};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// One request's time limit.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// A probe's time limit (a host that isn't a forge shouldn't hold a poll up).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The most one answer may be.
pub const MAX_BODY: usize = 8 << 20;
/// How long a probe's answer is kept.
pub const DETECT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Open changes read per poll (the newest first).
pub const MAX_OPEN: usize = 50;

/// A forge kwant can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeKind {
    Github,
    Gitlab,
    /// Gitea, and Forgejo (Codeberg) which speaks the same API.
    Gitea,
}

impl ForgeKind {
    /// `github`, `gitlab`, `gitea`: the desk's `forge`.
    pub fn as_str(self) -> &'static str {
        match self {
            ForgeKind::Github => "github",
            ForgeKind::Gitlab => "gitlab",
            ForgeKind::Gitea => "gitea",
        }
    }

    /// How the person calls it.
    pub fn name(self) -> &'static str {
        match self {
            ForgeKind::Github => "GitHub",
            ForgeKind::Gitlab => "GitLab",
            ForgeKind::Gitea => "Gitea",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "github" => Some(ForgeKind::Github),
            "gitlab" => Some(ForgeKind::Gitlab),
            "gitea" | "forgejo" | "codeberg" => Some(ForgeKind::Gitea),
            _ => None,
        }
    }

    /// The API under a forge's web address.
    fn api_path(self) -> &'static str {
        match self {
            ForgeKind::Github => "",
            ForgeKind::Gitlab => "/api/v4",
            ForgeKind::Gitea => "/api/v1",
        }
    }
}

/// The forge a well-known host is, with no request. `None`: ask it ([`detect`]).
pub fn known(host: &str) -> Option<ForgeKind> {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    match h.as_str() {
        "github.com" | "www.github.com" => Some(ForgeKind::Github),
        "gitlab.com" | "www.gitlab.com" => Some(ForgeKind::Gitlab),
        "codeberg.org" | "gitea.com" => Some(ForgeKind::Gitea),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The checked HTTP client

/// The same check as a git fetch, as reqwest's resolver: the host's addresses, refused when any
/// is [`git::blocked`] (and not a loopback a person-typed `http://` address allows), else exactly
/// those addresses, which is what reqwest then connects to.
struct Pinned {
    resolver: Arc<dyn Resolver>,
    loopback: bool,
}

#[derive(Debug)]
struct RefusedAddress;

impl std::fmt::Display for RefusedAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("kwant doesn't connect to this address")
    }
}

impl std::error::Error for RefusedAddress {}

fn refused(ip: IpAddr, loopback: bool) -> bool {
    git::blocked(ip) && !(loopback && ip.is_loopback())
}

impl reqwest::dns::Resolve for Pinned {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let (resolver, loopback, host) = (self.resolver.clone(), self.loopback, name.as_str().to_string());
        Box::pin(async move {
            let addrs = resolver.lookup(&host, 0).await.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            if addrs.is_empty() || addrs.iter().any(|a| refused(*a, loopback)) {
                return Err(Box::new(RefusedAddress) as Box<dyn std::error::Error + Send + Sync>);
            }
            let list: Vec<SocketAddr> = addrs.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect();
            Ok(Box::new(list.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Whether a request failed because [`Pinned`] refused its host's addresses.
fn refused_by_pin(e: &reqwest::Error) -> bool {
    let mut src: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(s) = src {
        if s.downcast_ref::<RefusedAddress>().is_some() {
            return true;
        }
        src = s.source();
    }
    format!("{e:?}").contains("RefusedAddress")
}

/// Where a request may go before it is sent: https (or `http://` to this machine when `loopback`),
/// no user or password in it, and a literal address (which reqwest doesn't look up) checked here.
pub fn check_url(u: &url::Url, loopback: bool) -> Result<(), GitError> {
    if !u.username().is_empty() || u.password().is_some() {
        return Err(GitError::Refused);
    }
    let ip = match u.host() {
        Some(url::Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if !loopback && (d == "localhost" || d.ends_with(".localhost")) {
                return Err(GitError::Refused);
            }
            None
        }
        None => return Err(GitError::Refused),
    };
    match u.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(GitError::Refused),
    }
    if ip.is_some_and(|ip| refused(ip, loopback)) {
        return Err(GitError::Refused);
    }
    Ok(())
}

/// Whether `base` is a person-typed `http://` address on this machine (a local test server): the
/// only way a forge request reaches loopback.
pub fn loopback_allowed(base: &str) -> bool {
    url::Url::parse(base).ok().is_some_and(|u| u.scheme() == "http" && u.host_str().is_some_and(crate::zombie::imap::is_loopback))
}

/// A client whose every connection goes through the address check: see the module docs.
pub fn client(resolver: Arc<dyn Resolver>, loopback: bool, timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .user_agent("kwnt-dev-mode")
        .dns_resolver(Arc::new(Pinned { resolver, loopback }))
        .build()
        .unwrap_or_default()
}

/// Reads an answer's body, at most [`MAX_BODY`].
async fn body(mut res: reqwest::Response) -> Option<Vec<u8>> {
    if res.content_length().is_some_and(|n| n as usize > MAX_BODY) {
        return None;
    }
    let mut out = Vec::new();
    while let Ok(Some(chunk)) = res.chunk().await {
        if out.len() + chunk.len() > MAX_BODY {
            return None;
        }
        out.extend_from_slice(&chunk);
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// Detection

/// What each host answered (`None`: neither forge) and when.
type Detected = Mutex<HashMap<String, (Option<ForgeKind>, Instant)>>;

fn detected() -> &'static Detected {
    static D: OnceLock<Detected> = OnceLock::new();
    D.get_or_init(Default::default)
}

/// What [`detect`] last found for `host`, without asking: for the desk.
pub fn detected_kind(host: &str) -> Option<ForgeKind> {
    known(host).or_else(|| detected().lock().unwrap().get(&host.to_ascii_lowercase()).and_then(|(k, _)| *k))
}

/// The forge on `host`: [`known`], else a cached probe of `https://<host>` ([`probe`]) through the
/// address check, with no loopback allowance (the host comes from a team's folder or repo link).
pub async fn detect(host: &str, resolver: Arc<dyn Resolver>) -> Option<ForgeKind> {
    if let Some(k) = known(host) {
        return Some(k);
    }
    let key = host.to_ascii_lowercase();
    if let Some((k, at)) = detected().lock().unwrap().get(&key) {
        if at.elapsed() < DETECT_TTL {
            return *k;
        }
    }
    let found = probe(&format!("https://{key}"), resolver, false).await;
    let mut map = detected().lock().unwrap();
    if map.len() > 500 {
        map.clear();
    }
    map.insert(key, (found, Instant::now()));
    found
}

/// Asks `web` (a forge's web address) what it runs: `GET /api/v4/version` answering 200 with a
/// `version`, or 401 with GitLab's `{"message": "401 Unauthorized"}`, is GitLab; else
/// `GET /api/v1/version` answering 200 with a `version` is Gitea (or Forgejo). No token is sent.
pub async fn probe(web: &str, resolver: Arc<dyn Resolver>, loopback: bool) -> Option<ForgeKind> {
    let http = client(resolver, loopback, PROBE_TIMEOUT);
    let ask = |path: &'static str| {
        let http = http.clone();
        let url = format!("{}{path}", web.trim_end_matches('/'));
        async move {
            let u = url::Url::parse(&url).ok()?;
            check_url(&u, loopback).ok()?;
            let res = http.get(u).header("Accept", "application/json").send().await.ok()?;
            let status = res.status().as_u16();
            let v: Value = serde_json::from_slice(&body(res).await?).ok()?;
            Some((status, v))
        }
    };
    if let Some((status, v)) = ask("/api/v4/version").await {
        let gitlab_401 = status == 401 && v["message"].as_str().is_some_and(|m| m.starts_with("401"));
        if (status == 200 && v["version"].is_string()) || gitlab_401 {
            return Some(ForgeKind::Gitlab);
        }
    }
    match ask("/api/v1/version").await {
        Some((200, v)) if v["version"].is_string() => Some(ForgeKind::Gitea),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The Loose ends connection

/// A Loose ends connection of kind `forge`: a GitLab, Gitea or Forgejo server and its token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForgeConfig {
    /// The forge's web address: `https://gitlab.com`, `https://git.example.com`, or one under a
    /// path (`https://example.com/gitlab`).
    pub url: String,
    /// `gitlab` or `gitea`; asked of the server when the connection is added without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<ForgeKind>,
}

impl ForgeConfig {
    pub fn validate(&self) -> Result<(), String> {
        const BAD: &str = "url must be the forge's https address, like https://gitlab.example.com";
        let u = url::Url::parse(self.url.trim()).map_err(|_| BAD.to_string())?;
        let host = u.host_str().unwrap_or_default();
        let ok = match u.scheme() {
            "https" => !host.is_empty(),
            // A local mock server in tests, never a remote host in the clear (as GitHub's api_base)
            "http" => crate::zombie::imap::is_loopback(host),
            _ => false,
        };
        if !ok || !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() {
            return Err(BAD.into());
        }
        if self.forge == Some(ForgeKind::Github) {
            return Err("GitHub is its own connection: add it as GitHub".into());
        }
        Ok(())
    }

    /// The web address without a trailing `/`.
    pub fn web(&self) -> String {
        self.url.trim().trim_end_matches('/').to_string()
    }

    /// `host` or `host:port`, lowercase: what a watched repo's host is matched against.
    pub fn host(&self) -> Option<String> {
        let u = url::Url::parse(self.url.trim()).ok()?;
        let h = u.host_str()?.to_ascii_lowercase();
        Some(match u.port() {
            Some(p) => format!("{h}:{p}"),
            None => h,
        })
    }

    /// The host alone, lowercase: what a watched repo's `host` is (it has no port).
    pub fn host_name(&self) -> Option<String> {
        url::Url::parse(self.url.trim()).ok()?.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase())
    }

    /// The vault origin the token is saved under.
    pub fn vault_origin(&self) -> String {
        url::Url::parse(self.url.trim()).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
    }

    /// The path the forge sits under (`/gitlab`), empty at the root.
    fn base_path(&self) -> String {
        url::Url::parse(self.url.trim()).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default()
    }
}

/// Fills in a new connection's `forge` by asking the server (the person typed the address, so a
/// `http://` one on this machine may be asked). Errors are for the person.
pub async fn settle(mut cfg: ForgeConfig, resolver: Arc<dyn Resolver>) -> Result<ForgeConfig, String> {
    if cfg.forge.is_none() {
        let host = cfg.host().unwrap_or_default();
        cfg.forge = match known(&host) {
            Some(k) => Some(k),
            None => probe(&cfg.web(), resolver, loopback_allowed(&cfg.url)).await,
        };
    }
    match cfg.forge {
        None => Err(format!("kwant couldn't tell whether {} runs GitLab, Gitea or Forgejo: set \"forge\" to gitlab or gitea", cfg.host().unwrap_or_default())),
        Some(ForgeKind::Github) => Err("GitHub is its own connection: add it as GitHub".into()),
        Some(_) => Ok(cfg),
    }
}

/// A forge server with its token: what a [`GitlabForge`] or [`GiteaForge`] asks through.
pub struct ForgeClient {
    pub id: String,
    pub kind: ForgeKind,
    cfg: ForgeConfig,
    api: String,
    token: Zeroizing<String>,
    http: reqwest::Client,
    loopback: bool,
}

impl std::fmt::Debug for ForgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgeClient").field("id", &self.id).field("kind", &self.kind).field("api", &self.api).finish_non_exhaustive()
    }
}

impl ForgeClient {
    /// `None` for a config that isn't valid or has no (non-GitHub) forge yet.
    pub fn new(id: &str, cfg: ForgeConfig, token: Zeroizing<String>, resolver: Arc<dyn Resolver>) -> Option<Self> {
        cfg.validate().ok()?;
        let kind = cfg.forge.filter(|k| *k != ForgeKind::Github)?;
        let loopback = loopback_allowed(&cfg.url);
        let api = format!("{}{}", cfg.web(), kind.api_path());
        Some(ForgeClient { id: id.into(), kind, api, token, http: client(resolver, loopback, TIMEOUT), loopback, cfg })
    }

    pub fn config(&self) -> &ForgeConfig {
        &self.cfg
    }

    /// The repo's path on this forge, from a watched repo's path (which, under a forge at
    /// `https://example.com/gitlab`, starts with `gitlab/`).
    pub fn repo_path(&self, repo: &str) -> String {
        let repo = repo.trim_matches('/');
        let base = self.cfg.base_path();
        let base = base.trim_matches('/');
        let repo = if base.is_empty() { repo } else { repo.strip_prefix(base).and_then(|r| r.strip_prefix('/')).unwrap_or(repo) };
        repo.trim_end_matches(".git").to_string()
    }

    fn auth_value(&self) -> String {
        match self.kind {
            ForgeKind::Gitea => format!("token {}", self.token.as_str()),
            _ => format!("Bearer {}", self.token.as_str()),
        }
    }

    /// The header git sends when it fetches `url` from this forge over https: GitLab takes
    /// `oauth2:<token>`, Gitea and Forgejo a token as the user name. `None` for another host, or
    /// for ssh (the server's own key is used).
    pub fn git_auth(&self, url: &str) -> Option<Auth> {
        use base64::Engine;
        let u = url::Url::parse(url).ok()?;
        let host = u.host_str()?.to_ascii_lowercase();
        let host = match u.port() {
            Some(p) => format!("{host}:{p}"),
            None => host,
        };
        if Some(host) != self.cfg.host() || !matches!(u.scheme(), "https" | "http") {
            return None;
        }
        let pair = match self.kind {
            ForgeKind::Gitea => format!("{}:x-oauth-basic", self.token.as_str()),
            _ => format!("oauth2:{}", self.token.as_str()),
        };
        let value = Zeroizing::new(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair.as_bytes())));
        Auth::header(url, value)
    }

    /// One GET under the API, as JSON. `Ok(None)` for a 404 (the caller says what that means).
    async fn get(&self, path: &str) -> Result<Option<Value>, GhError> {
        let name = self.kind.name();
        let url = url::Url::parse(&format!("{}{path}", self.api)).map_err(|_| GhError::Other(format!("{name}: kwant made an address it can't use")))?;
        check_url(&url, self.loopback).map_err(|_| GhError::Other(format!("Dev mode can’t use this {name} address.")))?;
        let res = self.http.get(url).header("Authorization", self.auth_value()).header("Accept", "application/json").send().await.map_err(|e| {
            if refused_by_pin(&e) {
                GhError::Other(format!("Dev mode can’t use this {name} address."))
            } else if e.is_timeout() {
                GhError::Other(format!("{name} took too long to answer."))
            } else {
                GhError::Other(format!("Couldn’t reach {name}."))
            }
        })?;
        let status = res.status().as_u16();
        let now = Utc::now().timestamp().max(0) as u64;
        match status {
            200 => {
                let bytes = body(res).await.ok_or_else(|| GhError::Other(format!("{name} sent an answer too big to read.")))?;
                serde_json::from_slice(&bytes).map(Some).map_err(|_| GhError::Other(format!("{name} sent an answer kwant could not read.")))
            }
            404 => Ok(None),
            401 => Err(GhError::Other(format!("{name} refused the token (401): check it in Loose ends."))),
            403 | 429 => Err(match limited(status, res.headers(), now) {
                Some(s) => GhError::Limited { retry_after_s: s },
                None => GhError::Other(format!("{name} refused the request (403): the token may lack access to this repo.")),
            }),
            300..=399 => Err(GhError::Other(format!("{name} answered with a redirect, which kwant doesn't follow: check the address in Loose ends."))),
            s => Err(GhError::Other(format!("{name} answered {s}."))),
        }
    }

    /// [`Self::get`], with a 404 an error.
    async fn need(&self, path: &str) -> Result<Value, GhError> {
        self.get(path).await?.ok_or_else(|| GhError::Other(format!("{} has no such repo, or the token can't see it.", self.kind.name())))
    }

    /// The account the token belongs to.
    pub async fn login(&self) -> Result<String, GhError> {
        let v = self.need("/user").await?;
        let key = if self.kind == ForgeKind::Gitlab { "username" } else { "login" };
        v[key].as_str().map(str::to_string).ok_or_else(|| GhError::Other(format!("{} sent an answer kwant could not read.", self.kind.name())))
    }

    /// The [`Forge`] of `repo` (a watched repo's path) on this server.
    pub fn forge_for<'a>(&'a self, repo: &str) -> Box<dyn Forge + 'a> {
        match self.kind {
            ForgeKind::Gitlab => Box::new(GitlabForge { client: self, project: self.repo_path(repo) }),
            _ => Box::new(GiteaForge { client: self, repo: self.repo_path(repo) }),
        }
    }
}

/// How long to wait after a 403/429: `Retry-After`, else a `RateLimit-Reset` (GitLab) or
/// `X-RateLimit-Reset` (Gitea) epoch when none are left; a bare 429 waits a minute.
fn limited(status: u16, headers: &reqwest::header::HeaderMap, now: u64) -> Option<u64> {
    let h = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(s) = h("retry-after").and_then(|v| v.parse::<u64>().ok()) {
        return Some(s.clamp(1, 3600));
    }
    let none_left = h("ratelimit-remaining") == Some("0") || h("x-ratelimit-remaining") == Some("0");
    if none_left {
        let reset = h("ratelimit-reset").or(h("x-ratelimit-reset")).and_then(|v| v.parse::<u64>().ok()).unwrap_or(now + 60);
        return Some(reset.saturating_sub(now).clamp(1, 3600));
    }
    (status == 429).then_some(60)
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// Lines added and removed in a unified diff body.
fn counts(diff: &str) -> (u64, u64) {
    let mut add = 0;
    let mut del = 0;
    for l in diff.lines() {
        if l.starts_with('+') && !l.starts_with("+++") {
            add += 1;
        } else if l.starts_with('-') && !l.starts_with("---") {
            del += 1;
        }
    }
    (add, del)
}

/// Where a check's link may go: https only (the poller drops the rest too).
fn link(v: &Value) -> Option<String> {
    v.as_str().filter(|u| u.starts_with("https://")).map(str::to_string)
}

const UNSUPPORTED_POST: &str = "comes in a later kwant version";

// ---------------------------------------------------------------------------------------------
// GitLab

/// [`Forge`] over GitLab's REST API (`/api/v4`): merge requests, commit statuses and pipelines.
pub struct GitlabForge<'a> {
    pub client: &'a ForgeClient,
    /// `group/sub/name`.
    pub project: String,
}

/// A merge request as GitLab sends it, as a [`Change`].
pub fn gitlab_change(v: &Value) -> Option<Change> {
    let number = v["iid"].as_u64()?;
    let head_sha = text(&v["sha"]).unwrap_or_default();
    let merged = v["state"] == "merged";
    // A merge commit, else a squash commit, else (fast-forward) the head itself
    let merge_sha = text(&v["merge_commit_sha"]).or_else(|| text(&v["squash_commit_sha"])).or_else(|| merged.then(|| head_sha.clone()).filter(|s| !s.is_empty()));
    let title = text(&v["title"]).unwrap_or_default();
    let draft = v["draft"].as_bool().or(v["work_in_progress"].as_bool()).unwrap_or(false);
    Some(Change {
        number,
        title,
        body: text(&v["description"]),
        draft,
        url: text(&v["web_url"]),
        author: text(&v["author"]["username"]),
        head_branch: text(&v["source_branch"]).unwrap_or_default(),
        head_sha,
        base_branch: text(&v["target_branch"]).unwrap_or_default(),
        merged_at: date(&v["merged_at"]).or_else(|| if merged { date(&v["updated_at"]) } else { None }),
        merge_sha: if merged { merge_sha } else { None },
        merged_by: text(&v["merge_user"]["username"]).or_else(|| text(&v["merged_by"]["username"])),
        updated_at: date(&v["updated_at"]),
    })
}

/// A GitLab commit status or pipeline status as a [`Check`]'s status and conclusion.
pub fn gitlab_state(s: &str) -> (&'static str, Option<&'static str>) {
    match s {
        "success" => ("completed", Some("success")),
        "failed" => ("completed", Some("failure")),
        "canceled" | "canceling" => ("completed", Some("cancelled")),
        "skipped" => ("completed", Some("skipped")),
        // A manual job waits for a person: it isn't running
        "manual" => ("completed", Some("neutral")),
        _ => ("in_progress", None),
    }
}

impl GitlabForge<'_> {
    fn p(&self) -> String {
        format!("/projects/{}", enc(&self.project))
    }

    fn list(v: Value) -> Vec<Change> {
        v.as_array().map(|a| a.iter().filter_map(gitlab_change).collect()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl Forge for GitlabForge<'_> {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Gitlab
    }
    async fn changes_for_commit(&self, sha: &str) -> Result<Vec<Change>, GhError> {
        Ok(self.client.get(&format!("{}/repository/commits/{}/merge_requests", self.p(), enc(sha))).await?.map(Self::list).unwrap_or_default())
    }
    async fn open_changes(&self, base: &str, _cache: &EtagCache) -> Result<Vec<Change>, GhError> {
        let v = self.client.need(&format!("{}/merge_requests?state=opened&target_branch={}&order_by=updated_at&sort=desc&per_page={MAX_OPEN}", self.p(), enc(base))).await?;
        Ok(Self::list(v).into_iter().filter(|c| c.base_branch == base).collect())
    }
    async fn change(&self, number: u64) -> Result<Option<Change>, GhError> {
        Ok(self.client.get(&format!("{}/merge_requests/{number}", self.p())).await?.as_ref().and_then(gitlab_change))
    }
    async fn checks(&self, sha: &str, _cache: &EtagCache) -> Result<Vec<Check>, GhError> {
        let statuses = self.client.get(&format!("{}/repository/commits/{}/statuses?all=true&per_page=100", self.p(), enc(sha))).await?.unwrap_or(Value::Null);
        // The newest status of each job name (a retried job sends a new one)
        let mut newest: HashMap<String, &Value> = HashMap::new();
        for s in statuses.as_array().into_iter().flatten() {
            let name = text(&s["name"]).unwrap_or_else(|| "ci".into());
            let id = s["id"].as_u64().unwrap_or(0);
            if newest.get(&name).is_none_or(|o| o["id"].as_u64().unwrap_or(0) < id) {
                newest.insert(name, s);
            }
        }
        let mut out: Vec<Check> = newest
            .into_iter()
            .map(|(name, s)| {
                let (status, conclusion) = gitlab_state(s["status"].as_str().unwrap_or_default());
                let conclusion = if conclusion == Some("failure") && s["allow_failure"].as_bool() == Some(true) { Some("neutral") } else { conclusion };
                Check { name, status: status.into(), conclusion: conclusion.map(str::to_string), url: link(&s["target_url"]) }
            })
            .collect();
        if out.is_empty() {
            // No job statuses: the commit's pipelines, the newest of each source
            let pipes = self.client.get(&format!("{}/pipelines?sha={}&per_page=20", self.p(), enc(sha))).await?.unwrap_or(Value::Null);
            if let Some(p) = pipes.as_array().and_then(|a| a.iter().max_by_key(|p| p["id"].as_u64().unwrap_or(0))) {
                let (status, conclusion) = gitlab_state(p["status"].as_str().unwrap_or_default());
                out.push(Check { name: "pipeline".into(), status: status.into(), conclusion: conclusion.map(str::to_string), url: link(&p["web_url"]) });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    async fn change_files(&self, number: u64) -> Result<Vec<FileDiff>, GhError> {
        let mut out = Vec::new();
        for page in 1..=(MAX_PR_FILES / 100).max(1) {
            let Some(v) = self.client.get(&format!("{}/merge_requests/{number}/diffs?per_page=100&page={page}", self.p())).await? else { break };
            let list = v.as_array().cloned().unwrap_or_default();
            let n = list.len();
            for d in list {
                let diff = text(&d["diff"]);
                let (additions, deletions) = diff.as_deref().map(counts).unwrap_or((0, 0));
                let status = if d["new_file"] == true {
                    "added"
                } else if d["deleted_file"] == true {
                    "removed"
                } else if d["renamed_file"] == true {
                    "renamed"
                } else {
                    "modified"
                };
                let path = text(&d["new_path"]).unwrap_or_default();
                let old = text(&d["old_path"]).filter(|o| status == "renamed" && *o != path);
                out.push(FileDiff { path, status: status.into(), additions, deletions, patch: diff.filter(|p| !p.is_empty()), previous_path: old });
            }
            if n < 100 || out.len() >= MAX_PR_FILES {
                break;
            }
        }
        out.truncate(MAX_PR_FILES);
        Ok(out)
    }
    fn change_head_ref(&self, number: u64) -> Option<String> {
        Some(format!("refs/merge-requests/{number}/head"))
    }
    async fn commit_login(&self, _sha: &str) -> Option<String> {
        // GitLab doesn't tie a commit to an account
        None
    }
    async fn login(&self) -> Result<String, GhError> {
        self.client.login().await
    }
    async fn post_review(&self, _number: u64, _post: &ReviewPost) -> Result<GhReview, PostError> {
        Err(PostError::Gh(GhError::Other(format!("Posting to GitLab {UNSUPPORTED_POST}."))))
    }
}

// ---------------------------------------------------------------------------------------------
// Gitea, Forgejo, Codeberg

/// [`Forge`] over Gitea's REST API (`/api/v1`), which Forgejo and Codeberg speak too.
pub struct GiteaForge<'a> {
    pub client: &'a ForgeClient,
    /// `owner/name`.
    pub repo: String,
}

/// A pull request as Gitea sends it, as a [`Change`]. A title starting `WIP:` or `[WIP]` is a
/// draft (Gitea's default work-in-progress prefixes), as is `draft: true` where it's sent.
pub fn gitea_change(v: &Value) -> Option<Change> {
    let number = v["number"].as_u64()?;
    let title = text(&v["title"]).unwrap_or_default();
    let t = title.trim_start().to_ascii_uppercase();
    let draft = v["draft"].as_bool().unwrap_or(false) || t.starts_with("WIP:") || t.starts_with("[WIP]");
    let merged = v["merged"].as_bool().unwrap_or(false) || !v["merged_at"].is_null();
    Some(Change {
        number,
        title,
        body: text(&v["body"]),
        draft,
        url: text(&v["html_url"]),
        author: text(&v["user"]["login"]),
        head_branch: text(&v["head"]["ref"]).unwrap_or_default(),
        head_sha: text(&v["head"]["sha"]).unwrap_or_default(),
        base_branch: text(&v["base"]["ref"]).unwrap_or_default(),
        merged_at: date(&v["merged_at"]),
        merge_sha: if merged { text(&v["merge_commit_sha"]) } else { None },
        merged_by: text(&v["merged_by"]["login"]),
        updated_at: date(&v["updated_at"]),
    })
}

/// A Gitea commit status as a [`Check`]'s status and conclusion.
pub fn gitea_state(s: &str) -> (&'static str, Option<&'static str>) {
    match s {
        "success" => ("completed", Some("success")),
        "failure" | "error" => ("completed", Some("failure")),
        "warning" => ("completed", Some("neutral")),
        "skipped" => ("completed", Some("skipped")),
        _ => ("in_progress", None),
    }
}

impl GiteaForge<'_> {
    fn r(&self) -> String {
        let (o, n) = self.repo.split_once('/').unwrap_or((self.repo.as_str(), ""));
        format!("/repos/{}/{}", enc(o), enc(n))
    }
}

#[async_trait::async_trait]
impl Forge for GiteaForge<'_> {
    fn kind(&self) -> ForgeKind {
        ForgeKind::Gitea
    }
    async fn changes_for_commit(&self, sha: &str) -> Result<Vec<Change>, GhError> {
        // The pull request a commit came from; 404 when none did (or on a Gitea before 1.21)
        Ok(self.client.get(&format!("{}/commits/{}/pull", self.r(), enc(sha))).await.unwrap_or(None).as_ref().and_then(gitea_change).into_iter().collect())
    }
    async fn open_changes(&self, base: &str, _cache: &EtagCache) -> Result<Vec<Change>, GhError> {
        let v = self.client.need(&format!("{}/pulls?state=open&sort=recentupdate&limit={MAX_OPEN}", self.r())).await?;
        Ok(v.as_array().map(|a| a.iter().filter_map(gitea_change).filter(|c| c.base_branch == base).collect()).unwrap_or_default())
    }
    async fn change(&self, number: u64) -> Result<Option<Change>, GhError> {
        Ok(self.client.get(&format!("{}/pulls/{number}", self.r())).await?.as_ref().and_then(gitea_change))
    }
    async fn checks(&self, sha: &str, _cache: &EtagCache) -> Result<Vec<Check>, GhError> {
        let v = self.client.get(&format!("{}/commits/{}/statuses?limit=50", self.r(), enc(sha))).await?.unwrap_or(Value::Null);
        let mut newest: HashMap<String, &Value> = HashMap::new();
        for s in v.as_array().into_iter().flatten() {
            let name = text(&s["context"]).unwrap_or_else(|| "ci".into());
            let id = s["id"].as_u64().unwrap_or(0);
            if newest.get(&name).is_none_or(|o| o["id"].as_u64().unwrap_or(0) < id) {
                newest.insert(name, s);
            }
        }
        let mut out: Vec<Check> = newest
            .into_iter()
            .map(|(name, s)| {
                let (status, conclusion) = gitea_state(s["status"].as_str().or(s["state"].as_str()).unwrap_or_default());
                Check { name, status: status.into(), conclusion: conclusion.map(str::to_string), url: link(&s["target_url"]) }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    async fn change_files(&self, number: u64) -> Result<Vec<FileDiff>, GhError> {
        let mut out = Vec::new();
        for page in 1..=(MAX_PR_FILES / 50).max(1) {
            let Some(v) = self.client.get(&format!("{}/pulls/{number}/files?limit=50&page={page}", self.r())).await? else { break };
            let list = v.as_array().cloned().unwrap_or_default();
            let n = list.len();
            for f in list {
                let status = match f["status"].as_str().unwrap_or("modified") {
                    "added" => "added",
                    "deleted" | "removed" => "removed",
                    "renamed" => "renamed",
                    _ => "modified",
                };
                out.push(FileDiff {
                    path: text(&f["filename"]).unwrap_or_default(),
                    status: status.into(),
                    additions: f["additions"].as_u64().unwrap_or(0),
                    deletions: f["deletions"].as_u64().unwrap_or(0),
                    // Gitea lists no patch: the diff comes from git when it can read the head
                    patch: None,
                    previous_path: text(&f["previous_filename"]).filter(|p| !p.is_empty()),
                });
            }
            if n < 50 || out.len() >= MAX_PR_FILES {
                break;
            }
        }
        out.truncate(MAX_PR_FILES);
        Ok(out)
    }
    fn change_head_ref(&self, number: u64) -> Option<String> {
        Some(format!("refs/pull/{number}/head"))
    }
    async fn commit_login(&self, sha: &str) -> Option<String> {
        let v = self.client.get(&format!("{}/git/commits/{}", self.r(), enc(sha))).await.ok()??;
        text(&v["author"]["login"]).filter(|l| !l.is_empty())
    }
    async fn login(&self) -> Result<String, GhError> {
        self.client.login().await
    }
    async fn post_review(&self, _number: u64, _post: &ReviewPost) -> Result<GhReview, PostError> {
        Err(PostError::Gh(GhError::Other(format!("Posting to Gitea {UNSUPPORTED_POST}."))))
    }
}

// ---------------------------------------------------------------------------------------------
// Connections

/// The forge connections Loose ends has (kind `forge`, switched on, with a token in the vault).
pub async fn clients(state: &crate::server::AppState, resolver: Arc<dyn Resolver>) -> Vec<ForgeClient> {
    let rows = state.db.zombie_sources().await.unwrap_or_default();
    let mut out = Vec::new();
    for row in rows.into_iter().filter(|r| r.kind == "forge" && r.enabled) {
        let Ok(cfg) = serde_json::from_str::<ForgeConfig>(&row.config) else { continue };
        match crate::zombie::hunt::source_secret(state.credentials.as_ref(), &row.id, &cfg.vault_origin()).await {
            Ok(Some(token)) => out.extend(ForgeClient::new(&row.id, cfg, token, resolver.clone())),
            Ok(None) => {}
            Err(e) => tracing::warn!(source = %row.id, "dev: vault read failed: {e}"),
        }
    }
    out
}

/// The connection for a watched repo's `host`, if Loose ends has one.
pub async fn client_for_host(state: &crate::server::AppState, host: &str, resolver: Arc<dyn Resolver>) -> Option<ForgeClient> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    clients(state, resolver).await.into_iter().find(|c| c.cfg.host_name().as_deref() == Some(host.as_str()))
}

/// The host (no port) of every forge connection, with its forge (no vault read): which watched
/// repos have a token, for the desk.
pub async fn connected_hosts(state: &crate::server::AppState) -> Vec<(String, Option<ForgeKind>)> {
    state
        .db
        .zombie_sources()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.kind == "forge" && r.enabled)
        .filter_map(|r| serde_json::from_str::<ForgeConfig>(&r.config).ok())
        .filter_map(|c| Some((c.host_name()?, c.forge)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fake(Vec<IpAddr>);

    #[async_trait::async_trait]
    impl Resolver for Fake {
        async fn lookup(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, GitError> {
            Ok(self.0.clone())
        }
    }

    fn fake(ips: &[&str]) -> Arc<dyn Resolver> {
        Arc::new(Fake(ips.iter().map(|s| s.parse().unwrap()).collect()))
    }

    #[test]
    fn well_known_hosts_need_no_probe() {
        assert_eq!(known("github.com"), Some(ForgeKind::Github));
        assert_eq!(known("GitLab.com"), Some(ForgeKind::Gitlab));
        assert_eq!(known("codeberg.org"), Some(ForgeKind::Gitea));
        assert_eq!(known("gitea.com"), Some(ForgeKind::Gitea));
        assert_eq!(known("git.example.com"), None);
        assert_eq!(known("gitlab.example.com"), None, "a name is a hint, not a forge: it is probed");
        assert_eq!(ForgeKind::parse("Forgejo"), Some(ForgeKind::Gitea));
        assert_eq!(serde_json::to_value(ForgeKind::Gitlab).unwrap(), json!("gitlab"));
    }

    #[test]
    fn urls_are_checked_before_a_request() {
        let ok = |s: &str, lb: bool| check_url(&url::Url::parse(s).unwrap(), lb).is_ok();
        assert!(ok("https://gitlab.example.com/api/v4/user", false));
        assert!(ok("https://192.168.1.20/api/v1/user", false), "a LAN forge is fine");
        assert!(!ok("http://gitlab.example.com/api/v4/user", false), "never in the clear");
        assert!(!ok("https://127.0.0.1/api/v4/user", false));
        assert!(!ok("https://localhost/api/v4/user", false));
        assert!(!ok("https://169.254.169.254/latest", false));
        assert!(!ok("https://169.254.169.254/latest", true), "link-local is never allowed");
        assert!(!ok("https://100.100.100.200/latest", true), "nor a metadata address");
        assert!(!ok("https://[fd00:ec2::254]/", true));
        assert!(!ok("https://user:pw@gitlab.example.com/", false));
        assert!(ok("http://127.0.0.1:9471/api/v4/user", true), "a person-typed local test server");
        assert!(!ok("file:///etc/passwd", true));
        assert!(loopback_allowed("http://127.0.0.1:9000"));
        assert!(!loopback_allowed("https://127.0.0.1:9000"), "only the http test-server form");
        assert!(!loopback_allowed("http://gitlab.example.com"));
    }

    #[tokio::test]
    async fn the_resolver_refuses_blocked_answers_and_pins_the_rest() {
        use reqwest::dns::Resolve;
        let name = |s: &str| s.parse::<reqwest::dns::Name>().unwrap();
        for bad in [&["169.254.169.254"][..], &["100.100.100.200"], &["10.0.0.5", "127.0.0.1"], &["::1"], &["::ffff:127.0.0.1"], &["0.0.0.0"], &[]] {
            let r = Pinned { resolver: fake(bad), loopback: false };
            assert!(r.resolve(name("forge.example.com")).await.is_err(), "{bad:?}");
        }
        let r = Pinned { resolver: fake(&["169.254.169.254"]), loopback: true };
        assert!(r.resolve(name("forge.example.com")).await.is_err(), "the loopback allowance never covers link-local");
        let r = Pinned { resolver: fake(&["10.0.0.5", "192.168.1.9"]), loopback: false };
        let got: Vec<SocketAddr> = r.resolve(name("forge.example.com")).await.unwrap().collect();
        assert_eq!(got, ["10.0.0.5:0".parse().unwrap(), "192.168.1.9:0".parse().unwrap()], "exactly the checked addresses");
    }

    /// A tiny server on this machine that answers every request with `status`, `headers`, `body`.
    async fn serve(status: u16, headers: Vec<(&'static str, String)>, body: &'static str) -> u16 {
        use axum::response::IntoResponse;
        let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        let app = axum::Router::new().fallback(move || {
            let headers = headers.clone();
            async move {
                let mut r = (axum::http::StatusCode::from_u16(status).unwrap(), body).into_response();
                for (k, v) in headers {
                    r.headers_mut().insert(k, v.parse().unwrap());
                }
                r
            }
        });
        tokio::spawn(async move {
            let _ = axum::serve(l, app).await;
        });
        port
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_this_machine_is_never_connected_to() {
        // The server is real and listening, but `gitlab.test` resolves to it: refused before any
        // byte is sent, even though the connection was typed by a person
        let port = serve(200, vec![], r#"{"username":"x"}"#).await;
        let cfg = ForgeConfig { url: format!("https://gitlab.test:{port}"), forge: Some(ForgeKind::Gitlab) };
        let c = ForgeClient::new("src", cfg, Zeroizing::new("t".into()), fake(&["127.0.0.1"])).unwrap();
        let e = c.login().await.unwrap_err().to_string();
        assert!(e.contains("can’t use this GitLab address"), "{e}");
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let target = serve(200, vec![], r#"{"username":"elsewhere"}"#).await;
        let port = serve(302, vec![("location", format!("http://127.0.0.1:{target}/api/v4/user"))], "").await;
        let cfg = ForgeConfig { url: format!("http://127.0.0.1:{port}"), forge: Some(ForgeKind::Gitlab) };
        let c = ForgeClient::new("src", cfg, Zeroizing::new("t".into()), fake(&[])).unwrap();
        let e = c.login().await.unwrap_err().to_string();
        assert!(e.contains("redirect"), "{e}");
    }

    #[tokio::test]
    async fn probes_tell_gitlab_from_gitea() {
        let gl = serve(401, vec![("content-type", "application/json".into())], r#"{"message":"401 Unauthorized"}"#).await;
        assert_eq!(probe(&format!("http://127.0.0.1:{gl}"), fake(&[]), true).await, Some(ForgeKind::Gitlab));
        let gt = serve(200, vec![("content-type", "application/json".into())], r#"{"version":"1.22.1"}"#).await;
        // `/api/v4/version` answers 200 too here, with a version: a server that says both is read
        // as GitLab, which is asked first. A Gitea answers 404 there, below.
        assert_eq!(probe(&format!("http://127.0.0.1:{gt}"), fake(&[]), true).await, Some(ForgeKind::Gitlab));
        let none = serve(404, vec![], "not here").await;
        assert_eq!(probe(&format!("http://127.0.0.1:{none}"), fake(&[]), true).await, None);
        // Without the allowance, this machine is never probed
        assert_eq!(probe(&format!("http://127.0.0.1:{gl}"), fake(&[]), false).await, None);
        assert_eq!(probe("https://forge.example.com", fake(&["169.254.169.254"]), false).await, None);
    }

    #[test]
    fn gitlab_merge_requests_convert() {
        let v = json!({"iid": 7, "title": "Add search", "description": "Why", "draft": false, "web_url": "https://gitlab.com/g/app/-/merge_requests/7",
            "author": {"username": "lin"}, "source_branch": "feat", "sha": "h1", "target_branch": "main", "state": "merged",
            "merged_at": "2026-11-05T01:00:00Z", "merge_commit_sha": null, "squash_commit_sha": "s1", "merge_user": {"username": "ada"}});
        let c = gitlab_change(&v).unwrap();
        assert_eq!((c.number, c.title.as_str(), c.body.as_deref()), (7, "Add search", Some("Why")));
        assert_eq!((c.head_branch.as_str(), c.head_sha.as_str(), c.base_branch.as_str()), ("feat", "h1", "main"));
        assert_eq!((c.merge_sha.as_deref(), c.merged_by.as_deref(), c.author.as_deref()), (Some("s1"), Some("ada"), Some("lin")));
        let ff = json!({"iid": 8, "title": "t", "sha": "h2", "state": "merged", "merged_at": "2026-11-05T01:00:00Z", "source_branch": "a", "target_branch": "main"});
        assert_eq!(gitlab_change(&ff).unwrap().merge_sha.as_deref(), Some("h2"), "a fast-forward merge is its head");
        let open = json!({"iid": 9, "title": "Draft: x", "sha": "h3", "state": "opened", "work_in_progress": true, "merge_commit_sha": "zz", "source_branch": "a", "target_branch": "main"});
        let o = gitlab_change(&open).unwrap();
        assert!(o.draft);
        assert_eq!(o.merge_sha, None, "an open MR has no merge commit");
        assert_eq!(gitlab_state("running"), ("in_progress", None));
        assert_eq!(gitlab_state("failed"), ("completed", Some("failure")));
        assert_eq!(gitlab_state("canceled"), ("completed", Some("cancelled")));
    }

    #[test]
    fn gitea_pulls_convert() {
        let v = json!({"number": 3, "title": "[WIP] tidy", "body": "B", "html_url": "https://codeberg.org/o/app/pulls/3", "user": {"login": "pixel"},
            "head": {"ref": "tidy", "sha": "h1"}, "base": {"ref": "main"}, "merged": true, "merged_at": "2026-11-05T01:00:00Z", "merge_commit_sha": "m1", "merged_by": {"login": "ada"}});
        let c = gitea_change(&v).unwrap();
        assert!(c.draft, "Gitea's WIP prefix is a draft");
        assert_eq!((c.merge_sha.as_deref(), c.merged_by.as_deref(), c.author.as_deref()), (Some("m1"), Some("ada"), Some("pixel")));
        assert_eq!(gitea_state("error"), ("completed", Some("failure")));
        assert_eq!(gitea_state("pending"), ("in_progress", None));
        assert_eq!(gitea_state("warning"), ("completed", Some("neutral")));
    }

    #[test]
    fn configs_validate_and_match_hosts() {
        let c = |u: &str| ForgeConfig { url: u.into(), forge: None };
        assert!(c("https://gitlab.example.com").validate().is_ok());
        assert!(c("https://example.com/gitlab/").validate().is_ok());
        assert!(c("http://gitlab.example.com").validate().is_err());
        assert!(c("http://127.0.0.1:9000").validate().is_ok());
        assert!(c("https://u:p@gitlab.example.com").validate().is_err());
        assert!(c("https://gitlab.example.com?x=1").validate().is_err());
        assert!(ForgeConfig { url: "https://github.com".into(), forge: Some(ForgeKind::Github) }.validate().is_err());
        assert_eq!(c("https://GitLab.Example.com:8443/").host().as_deref(), Some("gitlab.example.com:8443"));
        assert_eq!(c("https://example.com/gitlab").vault_origin(), "https://example.com");
        let cl = ForgeClient::new("s", ForgeConfig { url: "https://example.com/gitlab".into(), forge: Some(ForgeKind::Gitlab) }, Zeroizing::new("t".into()), fake(&[])).unwrap();
        assert_eq!(cl.repo_path("gitlab/group/app.git"), "group/app");
        assert_eq!(cl.repo_path("group/app"), "group/app");
        assert!(ForgeClient::new("s", c("https://gitlab.example.com"), Zeroizing::new("t".into()), fake(&[])).is_none(), "no forge yet");
    }

    #[test]
    fn git_gets_the_token_only_for_its_own_host() {
        let gl = ForgeClient::new("s", ForgeConfig { url: "https://gitlab.example.com".into(), forge: Some(ForgeKind::Gitlab) }, Zeroizing::new("tok".into()), fake(&[])).unwrap();
        assert!(gl.git_auth("https://gitlab.example.com/g/app.git").is_some());
        assert!(gl.git_auth("https://elsewhere.example.com/g/app.git").is_none());
        assert!(gl.git_auth("ssh://git@gitlab.example.com/g/app.git").is_none());
        assert!(gl.git_auth("git@gitlab.example.com:g/app.git").is_none());
        assert!(format!("{:?}", gl.git_auth("https://gitlab.example.com/g/app.git").unwrap()).contains("<hidden>"));
        assert!(!format!("{gl:?}").contains("tok"));
    }

    #[test]
    fn diff_counts() {
        assert_eq!(counts("@@ -1,2 +1,3 @@\n a\n-b\n+c\n+d\n"), (2, 1));
    }
}
