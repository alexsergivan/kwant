//! The repo check (plan §3.2 `POST /dev_mode/check`, D4), detection of the repos a space's teams
//! point at (any-git.md S1, S2), and [`reconcile`], which keeps a space's watched repos in step
//! with its teams (team-repos.md T3, T4). Repos are read through the Loose ends GitHub connection
//! (its vault token and Enterprise `api_base`). kwant writes nothing to GitHub.

use super::git;
use crate::db::dev_mode::{DevRepoRow, RepoAdd, BY_HAND, BY_MIGRATION, BY_TEAM, MAX_REPOS};
use crate::db::spaces::SpaceId;
use crate::server::AppState;
use crate::zombie::github::{GhError, GithubConfig, GithubSource};
use serde::Serialize;

/// `owner/name` from what a person pastes: `owner/name`, a web URL (with or without `.git`,
/// `/tree/main`, a query), `github.com/owner/name`, `git@host:owner/name.git` or
/// `ssh://git@host/owner/name.git`. Returns the host it named, if any, and `owner/name`.
pub fn normalize(input: &str) -> Option<(Option<String>, String)> {
    let s = input.trim();
    if s.is_empty() || s.len() > 300 || s.chars().any(char::is_whitespace) {
        return None;
    }
    let (host, path) = if let Some(rest) = s.strip_prefix("git@") {
        // scp-like: git@github.com:owner/name.git
        let (host, path) = rest.split_once(':')?;
        (Some(host.to_string()), path.to_string())
    } else if s.contains("://") {
        let s = s.strip_prefix("git+").unwrap_or(s);
        let url = url::Url::parse(s).ok()?;
        if !matches!(url.scheme(), "https" | "http" | "ssh" | "git") || url.password().is_some() {
            return None;
        }
        (url.host_str().map(str::to_string), url.path().to_string())
    } else if let Some((first, rest)) = s.split_once('/') {
        // `github.com/owner/name` or plain `owner/name`
        if first.contains('.') && rest.contains('/') {
            (Some(first.to_string()), rest.to_string())
        } else {
            (None, s.to_string())
        }
    } else {
        return None;
    };
    let mut parts = path.trim_matches('/').split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    // Owner and repo names are ASCII letters, digits, `-`, `_` and `.` on GitHub
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.starts_with('-') && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !ok(owner) || !ok(name) || owner.len() > 100 || name.len() > 100 {
        return None;
    }
    // A plain `owner/name/extra` isn't a repo; a URL path may go on (`/tree/main`, `/pull/3`)
    if host.is_none() && parts.next().is_some() {
        return None;
    }
    Some((host.map(|h| h.to_lowercase()), format!("{owner}/{name}")))
}

/// The web host repos of this API base live on ([`GithubConfig::web_origin`]):
/// `api.github.com` → `github.com`, `https://ghe.example/api/v3` → `ghe.example`,
/// `api.acme.ghe.com` → `acme.ghe.com`. Old rows are normalised first, so a stored
/// `https://acme.ghe.com` gives `acme.ghe.com` too.
pub fn web_host(cfg: &GithubConfig) -> String {
    url::Url::parse(&cfg.web_origin()).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default()
}

/// What a repo's root says about how it ships (plan §3.1 `deploy_hint`).
pub fn deploy_hint(root: &[String]) -> Option<&'static str> {
    let has = |n: &str| root.iter().any(|r| r.eq_ignore_ascii_case(n));
    if has("fly.toml") {
        Some("fly")
    } else if has("wrangler.toml") || has("wrangler.json") || has("wrangler.jsonc") {
        Some("cloudflare_pages")
    } else if ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml", "Dockerfile"].iter().any(|n| has(n)) {
        Some("docker")
    } else {
        None
    }
}

/// The reply of `POST /dev_mode/check`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Check {
    /// `found`, `not_found` or `not_connected`.
    pub state: &'static str,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub deploy_hint: Option<&'static str>,
    /// The web host of the connection that found it (never in a reply).
    #[serde(skip)]
    pub host: Option<String>,
}

impl Check {
    pub(crate) fn of(state: &'static str, repo: Option<String>) -> Self {
        Check { state, repo, branch: None, deploy_hint: None, host: None }
    }
}

/// The GitHub connections Loose ends has, as Dev mode clients, enabled ones first. `Err` says
/// why there is none (no connection, a locked vault).
pub async fn clients(state: &AppState) -> Result<Vec<(GithubConfig, GithubSource)>, String> {
    let mut rows: Vec<_> = state.db.zombie_sources().await.map_err(|e| e.to_string())?.into_iter().filter(|r| r.kind == "github").collect();
    rows.sort_by_key(|r| !r.enabled);
    let mut out = Vec::new();
    let mut why = "Connect GitHub in Loose ends first: Dev mode reads the repo with that token".to_string();
    for row in rows {
        let Ok(cfg) = serde_json::from_str::<GithubConfig>(&row.config) else { continue };
        if cfg.validate().is_err() {
            continue;
        }
        match crate::zombie::hunt::source_secret(state.credentials.as_ref(), &row.id, &cfg.vault_origin()).await {
            Ok(Some(token)) => out.push((cfg.clone(), GithubSource::for_dev(&row.id, cfg, token))),
            Ok(None) => {}
            Err(crate::sandbox::credentials::StoreError::Locked) => why = "The password vault is locked: unlock it so kwant can read GitHub".into(),
            Err(e) => tracing::warn!(source = %row.id, "dev: vault read failed: {e}"),
        }
    }
    if out.is_empty() {
        Err(why)
    } else {
        Ok(out)
    }
}

/// The client whose connection can read `repo` (the first that finds it), with the repo. With no
/// connection the error says so (why [`clients`] found none), never a refused token: the desk
/// shows such a row as `not_connected` (QA h).
pub async fn client_for(state: &AppState, repo: &str) -> Result<(GithubSource, crate::zombie::github::GhRepo), GhError> {
    let list = clients(state).await.map_err(GhError::Other)?;
    let mut last = GhError::NotFound;
    for (_, c) in list {
        match c.dev_repo(repo).await {
            Ok(r) => return Ok((c, r)),
            Err(e @ GhError::Limited { .. }) => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The repo a space's teams point at, as `owner/name`: the first detected source on a GitHub
/// host (see [`detect`]).
pub async fn prefill(state: &AppState, space: &SpaceId) -> Option<String> {
    let hosts = github_hosts(state).await;
    detect(state, space).await.into_iter().find(|s| s.on_github(&hosts)).map(|s| s.repo)
}

/// Checks `input` (or the first detected GitHub source when it's empty). `Err` is a GitHub
/// failure that is neither found nor not found (a rate limit, GitHub down): the route answers
/// 502 with it.
pub async fn check(state: &AppState, space: &SpaceId, input: &str) -> Result<Check, String> {
    let input = if input.trim().is_empty() {
        let hosts = github_hosts(state).await;
        detect(state, space).await.into_iter().find(|s| s.on_github(&hosts)).map(|s| s.check_input()).unwrap_or_default()
    } else {
        strip_userinfo(input)
    };
    let Some((host, repo)) = normalize(&input) else {
        return Ok(Check::of("not_found", None));
    };
    let list = match clients(state).await {
        Ok(l) => l,
        Err(_) => return Ok(Check::of("not_connected", Some(repo))),
    };
    let mut answer = Check::of("not_found", Some(repo.clone()));
    let mut refused = 0;
    let total = list.len();
    for (cfg, client) in list {
        // A URL on another host isn't this connection's repo
        if let Some(h) = &host {
            let h = h.strip_prefix("www.").unwrap_or(h);
            if h != web_host(&cfg) && !(h == "github.com" && web_host(&cfg) == "github.com") {
                continue;
            }
        }
        match client.dev_repo(&repo).await {
            Ok(r) => {
                let root = client.dev_root_names(&r.full_name, &r.default_branch).await.unwrap_or_default();
                answer = Check { state: "found", repo: Some(r.full_name.clone()), branch: Some(r.default_branch.clone()), deploy_hint: deploy_hint(&root), host: Some(web_host(&cfg)) };
                return Ok(answer);
            }
            Err(GhError::NotFound | GhError::Forbidden) => {}
            Err(GhError::Auth) => refused += 1,
            Err(e) => return Err(e.to_string()),
        }
    }
    // Every connection's token was refused: that's a connection problem, not a missing repo
    if refused > 0 && refused == total {
        answer.state = "not_connected";
    }
    Ok(answer)
}

// ---------------------------------------------------------------------------------------------
// Detection (plan any-git.md S1, S2): the repos a space's teams already point at, from each team's
// repo link or else its folder's git remote. Nothing here touches the network, and no URL leaves
// [`parse_remote`] with its userinfo.

/// A repo one of the space's teams points at. Serialised as `{team, repo, host, from}`: the team's
/// name, never its folder.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Source {
    pub team: String,
    /// The team's id: a team row's `dev_repos.team`.
    #[serde(skip)]
    pub team_id: String,
    /// `owner/name` on a GitHub host, the whole path (`group/sub/name`) on another host, and the
    /// folder's own name for a `local` source.
    pub repo: String,
    /// `github.com`, `gitlab.example.com`; `null` for a `local` source.
    pub host: Option<String>,
    /// `repo` (the team's repo link), `folder` (its folder's git remote) or `local` (a git folder
    /// with no remote).
    pub from: &'static str,
    /// The folder's default branch (`refs/remotes/<remote>/HEAD`, else the checked-out one).
    #[serde(skip)]
    pub branch: Option<String>,
    /// What dedupes sources: `host/repo` lowercased, or the folder for a `local` one.
    #[serde(skip)]
    key: String,
    /// What git reads it from (any-git.md G1): the link's or remote's URL without its userinfo
    /// ([`git::fetch_url`]), or the folder itself (an absolute path) for a `local` source.
    /// `None` when git couldn't read it. Never sent to the browser.
    #[serde(skip)]
    pub fetch: Option<String>,
}

impl Source {
    /// Whether this source is on GitHub (github.com, or the web host of a GitHub Enterprise
    /// connection in `hosts`).
    pub fn on_github(&self, hosts: &[String]) -> bool {
        self.host.as_deref().is_some_and(|h| hosts.iter().any(|g| g == h))
    }

    /// Its watched-repo key: `host/repo` lowercased (as [`repo_key`]), or `local:…` for a folder
    /// with no remote (never shown; see [`Self::api_key`]).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The key the API shows: [`Self::key`], except `local:<team name lowercased>` for a `local`
    /// source, so no folder path leaves the server.
    pub fn api_key(&self) -> String {
        if self.host.is_some() {
            self.key.clone()
        } else {
            format!("local:{}", self.team.to_lowercase())
        }
    }

    /// The key of its watched row: [`Self::key`] for a remote, [`Self::api_key`] for a `local`
    /// source (so no folder path is stored as a key or shown).
    pub fn row_key(&self) -> String {
        self.api_key()
    }

    /// What [`check`] takes for this source: `host/owner/name`.
    pub fn check_input(&self) -> String {
        match &self.host {
            Some(h) => format!("{h}/{}", self.repo),
            None => self.repo.clone(),
        }
    }
}

/// `url` without its userinfo: `https://user:token@host/p` → `https://host/p`,
/// `git@host:p` → `host:p`. Anything else comes back as it was.
pub fn strip_userinfo(url: &str) -> String {
    let s = url.trim();
    // Everything up to the last `@` is userinfo: a token with a `/` in it can't hide past it
    let (scheme, rest) = match s.find("://") {
        Some(i) => s.split_at(i + 3),
        None => ("", s),
    };
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    match rest[..end].rfind('@') {
        Some(at) => format!("{scheme}{}", &rest[at + 1..]),
        None => s.to_string(),
    }
}

/// `(host, path)` of a git remote URL (`https://`, `ssh://`, `git://`, `git+https://` or scp-like
/// `git@host:path`), its userinfo dropped first, the host lowercased without `www.` and the path
/// without `.git`. `None` for a local path or anything that isn't a repo path.
pub fn parse_remote(raw: &str) -> Option<(String, String)> {
    let s = strip_userinfo(raw);
    if s.is_empty() || s.len() > 500 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (host, path) = if let Some(i) = s.find("://") {
        let scheme = s[..i].to_ascii_lowercase();
        let scheme = scheme.strip_prefix("git+").unwrap_or(&scheme);
        if !matches!(scheme, "https" | "http" | "ssh" | "git") {
            return None;
        }
        let url = url::Url::parse(&format!("{scheme}{}", &s[i..])).ok()?;
        (url.host_str()?.to_string(), url.path().to_string())
    } else {
        let (h, p) = s.split_once(':')?;
        // `./x:y`, `/srv/x:y` and `C:\\x` are paths, not hosts
        if h.is_empty() || h.len() == 1 || !h.chars().all(|c| c.is_ascii_alphanumeric() || "-.".contains(c)) {
            return None;
        }
        (h.to_string(), p.to_string())
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segs: Vec<&str> = path.split('/').collect();
    let ok = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.starts_with('-') && p.len() <= 100 && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if host.is_empty() || segs.len() < 2 || segs.len() > 20 || !segs.iter().all(|p| ok(p)) {
        return None;
    }
    Some((host, segs.join("/")))
}

/// `(host, repo)` of what a team's repo link or folder remote says: `owner/name` on a GitHub
/// host in `gh_hosts`, the whole path on any other. In a repo link (`shorthand`), a plain
/// `owner/name` is github.com's; to git, that's a folder.
pub(crate) fn source_repo(raw: &str, gh_hosts: &[String], shorthand: bool) -> Option<(String, String)> {
    let (host, path) = match parse_remote(raw) {
        Some(hp) => hp,
        None if !shorthand => return None,
        None => {
            // `owner/name`, `github.com/owner/name`: [`normalize`] reads those (it refuses userinfo)
            let (host, repo) = normalize(&strip_userinfo(raw))?;
            let host = host.unwrap_or_else(|| "github.com".into());
            (host.strip_prefix("www.").unwrap_or(&host).to_string(), repo)
        }
    };
    if gh_hosts.contains(&host) {
        // GitHub URLs go on past the repo (`/tree/main`, `/pull/3`)
        let mut it = path.split('/');
        let (o, n) = (it.next()?, it.next()?);
        return Some((host, format!("{o}/{n}")));
    }
    Some((host, path))
}

/// The web hosts of GitHub repos: github.com, and each GitHub connection's Enterprise host. Reads
/// the connections' settings only, not their tokens.
pub async fn github_hosts(state: &AppState) -> Vec<String> {
    let mut hosts = vec!["github.com".to_string()];
    for row in state.db.zombie_sources().await.unwrap_or_default().into_iter().filter(|r| r.kind == "github") {
        if let Ok(cfg) = serde_json::from_str::<GithubConfig>(&row.config) {
            let h = web_host(&cfg);
            if !h.is_empty() && !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }
    hosts
}

/// How long one git command in a team folder may take.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// `git -C dir args…`, local only: no prompt, no fsmonitor hook, no walking up out of `dir`, and
/// killed after [`GIT_TIMEOUT`]. Its trimmed stdout on success. Its stderr is dropped unread, so
/// a URL in an error never reaches a log.
async fn git(dir: &std::path::Path, args: &[&str]) -> Option<String> {
    // The shared host git rules (D4c): no system config, hooks, fsmonitor or pager, no GIT_* env
    let mut cmd = git::host_git_command("git");
    cmd.arg("-C").arg(dir).args(args);
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    if let Some(parent) = dir.parent() {
        cmd.env("GIT_CEILING_DIRECTORIES", parent);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(GIT_TIMEOUT, cmd.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// What a team folder's git says: its remote's URL (`origin`, else the first remote; `None` when
/// it has none) and its default branch. `None` when the folder isn't a git checkout.
pub async fn read_folder(dir: &std::path::Path) -> Option<(Option<String>, Option<String>)> {
    // Only the folder's own repo: a `.git` symlink or `gitdir:` file naming another repo would
    // show that repo's remote and branch here (G2 delta D1)
    let d = dir.to_path_buf();
    if !matches!(tokio::task::spawn_blocking(move || git::own_git_dir(&d)).await, Ok(Ok(_))) {
        return None;
    }
    // A folder git can't open (dubious ownership, a broken .git) counts as no checkout
    git(dir, &["rev-parse", "--git-dir"]).await?;
    let remotes = git(dir, &["remote"]).await.unwrap_or_default();
    let names: Vec<&str> = remotes.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let remote = names.iter().find(|n| **n == "origin").or(names.first()).copied();
    let url = match remote {
        Some(r) => git(dir, &["remote", "get-url", "--", r]).await,
        None => None,
    };
    let mut branch = None;
    if let Some(r) = remote {
        let head = format!("refs/remotes/{r}/HEAD");
        if let Some(b) = git(dir, &["symbolic-ref", "--quiet", "--short", &head]).await {
            branch = b.strip_prefix(&format!("{r}/")).map(str::to_string).or(Some(b));
        }
    }
    if branch.is_none() {
        branch = git(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await;
    }
    Some((url, branch.filter(|b| super::routes::valid_branch(b))))
}

/// One team as detection reads it: its name, folder setting and repo link.
pub struct TeamRef<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub workspace_path: Option<&'a str>,
    pub repo_url: Option<&'a str>,
}

/// The sources of `teams`, in team order, deduped by repo (case-insensitive): each team's repo
/// link, or else its folder (`tools::team_workspace_path` under `space_dir`) when that is a git
/// checkout. `gh_hosts` are the hosts whose repos are `owner/name` (see [`github_hosts`]).
pub async fn detect_teams(teams: &[TeamRef<'_>], space_dir: &std::path::Path, gh_hosts: &[String]) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    for t in teams {
        let link = t.repo_url.map(str::trim).filter(|s| !s.is_empty());
        let folder = crate::tools::team_workspace_path(space_dir, t.name, t.workspace_path, link);
        let read = read_folder(&folder).await;
        let branch = read.as_ref().and_then(|(_, b)| b.clone());
        let found = if let Some((host, repo)) = link.and_then(|l| source_repo(l, gh_hosts, true)) {
            Some((Some(host), repo, "repo", link.and_then(git::remote_url)))
        } else if let Some((url, _)) = &read {
            match url.as_deref().and_then(|u| source_repo(u, gh_hosts, false).map(|hr| (hr, u))) {
                Some(((host, repo), u)) => Some((Some(host), repo, "folder", git::remote_url(u))),
                // A checkout with no remote, or one that isn't a URL (another folder)
                None => folder.file_name().map(|n| (None, n.to_string_lossy().to_string(), "local", None)),
            }
        } else {
            None
        };
        let Some((host, repo, from, mut fetch)) = found else { continue };
        let key = match &host {
            Some(h) => format!("{h}/{repo}").to_lowercase(),
            None => {
                let real = std::fs::canonicalize(&folder).unwrap_or(folder.clone());
                fetch = real.to_str().filter(|p| p.starts_with('/')).map(str::to_string);
                format!("local:{}", real.display())
            }
        };
        if out.iter().any(|s| s.key == key) {
            continue;
        }
        out.push(Source { team: t.name.to_string(), team_id: t.id.to_string(), repo, host, from, branch, key, fetch });
    }
    out
}

/// The repos `space`'s teams already point at, in team order (plan any-git.md S1).
pub async fn detect(state: &AppState, space: &SpaceId) -> Vec<Source> {
    let teams = state.db.list_teams(space).await.unwrap_or_default();
    let refs: Vec<TeamRef> = teams.iter().map(|t| TeamRef { id: &t.id, name: &t.name, workspace_path: t.workspace_path.as_deref(), repo_url: t.repo_url.as_deref() }).collect();
    let dir = crate::db::spaces::space_data_dir(&state.config.data_dir, space);
    detect_teams(&refs, &dir, &github_hosts(state).await).await
}

// ---------------------------------------------------------------------------------------------
// Reconcile (team-repos.md T3, T4): the rows of `dev_repos` follow the teams.

/// One reconcile at a time, server-wide: two would both see a source with no row and check it twice.
fn reconcile_lock() -> &'static tokio::sync::Mutex<()> {
    static L: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(Default::default)
}

/// How often a GET may reconcile one space (the hooks and PUT don't wait for it).
pub const RECONCILE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

fn reconciled() -> &'static std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>> {
    static T: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>> = std::sync::OnceLock::new();
    T.get_or_init(Default::default)
}

/// What one reconcile did, for logs and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// Keys of rows added for a team source.
    pub added: Vec<String>,
    /// Keys of team rows removed because no team points at them any more.
    pub removed: Vec<String>,
    /// Keys of GitHub sources left without a row because the space watches [`MAX_REPOS`].
    pub too_many: Vec<String>,
}

/// Brings `space`'s watched repos in line with its teams (T3):
/// - a migrated 0.7.1 row becomes its team's row (re-keyed if that team's repo is the same
///   `owner/name` on a GitHub Enterprise host), else a hand row;
/// - a team row no team points at any more is removed (its cards stay);
/// - a hand row names the team that now points at it, if any;
/// - while on, each GitHub source with no row is checked and gets a team row, up to
///   [`MAX_REPOS`] (T4); a source GitHub says isn't there (or no connection reads) gets none, and
///   one GitHub doesn't answer for is added with the folder's branch (the poller says why);
/// - while on, a row with no branch yet (added while off) gets the repo's default branch.
///
/// - while on and git is on the server, each other source (GitLab, Gitea, a bare git server, a
///   `local` folder) with no row gets a git row ([`crate::db::dev_mode::GIT`], any-git.md G1): the
///   poller reads it with git alone and makes landed cards. Without git it gets none (`no_git`).
///
/// Existing rows are never checked again here: the poller reports their errors. Off, nothing is
/// fetched (D12). A space with no Dev mode row is left alone. A removed row's git mirror is deleted.
pub async fn reconcile(state: &AppState, space: &SpaceId) -> Reconciled {
    let _one = reconcile_lock().lock().await;
    reconciled().lock().unwrap().insert(space.as_str().to_string(), std::time::Instant::now());
    let mut out = Reconciled::default();
    let Ok(Some(mode)) = state.db.dev_mode(space.as_str()).await else { return out };
    let hosts = github_hosts(state).await;
    let sources = detect(state, space).await;
    let gh: Vec<&Source> = sources.iter().filter(|s| s.on_github(&hosts)).collect();
    let Ok(mut rows) = state.db.dev_repos(space.as_str()).await else { return out };
    let db = &state.db;
    let sp = space.as_str();

    // 0.7.1 rows: the team that points at the same owner/name takes them
    for r in rows.iter().filter(|r| r.added_by == BY_MIGRATION) {
        match gh.iter().find(|s| s.repo.eq_ignore_ascii_case(&r.repo)) {
            Some(s) => {
                let mut key = r.key.clone();
                if s.key != r.key {
                    let host = s.host.clone().unwrap_or_default();
                    match db.rekey_dev_repo(sp, &r.key, &s.key, &host).await {
                        Ok(true) => key = s.key.clone(),
                        // The team's key has its own row already: this one stays a hand row
                        _ => {
                            let _ = db.set_dev_repo_owner(sp, &r.key, BY_HAND, None).await;
                            continue;
                        }
                    }
                }
                let _ = db.set_dev_repo_owner(sp, &key, BY_TEAM, Some(&s.team_id)).await;
            }
            None => {
                let _ = db.set_dev_repo_owner(sp, &r.key, BY_HAND, None).await;
            }
        }
    }
    rows = db.dev_repos(sp).await.unwrap_or(rows);

    for r in &rows {
        let source = if r.is_git() { sources.iter().find(|s| !s.on_github(&hosts) && s.row_key() == r.key) } else { gh.iter().copied().find(|s| s.key == r.key) };
        match (r.added_by.as_str(), source) {
            // A team row whose team no longer points at it
            (BY_TEAM, None) => {
                if matches!(db.delete_dev_repo(sp, &r.key).await, Ok(true)) {
                    git::remove_mirror(&state.config.data_dir, sp, &r.key).await;
                    tracing::info!(space = %space, repo = %r.repo, "dev: stopped watching a repo no team points at");
                    out.removed.push(r.key.clone());
                }
            }
            (BY_TEAM, Some(s)) if r.team.as_deref() != Some(s.team_id.as_str()) => {
                let _ = db.set_dev_repo_owner(sp, &r.key, BY_TEAM, Some(&s.team_id)).await;
            }
            (BY_TEAM, Some(_)) => {}
            // A hand row: names the team that points at it now, or none
            (_, s) => {
                let team = s.map(|s| s.team_id.as_str());
                if r.team.as_deref() != team {
                    let _ = db.set_dev_repo_owner(sp, &r.key, BY_HAND, team).await;
                }
            }
        }
    }
    if !mode.on {
        return out;
    }
    let mut rows = db.dev_repos(sp).await.unwrap_or_default();

    // Rows added while off have no branch yet. One the check can't read keeps no branch (the
    // poller skips it) and records why, so the desk shows an error, not watching (QA g); the next
    // reconcile checks again, and setting the branch clears the error. A git row's branch is read
    // by the poller, which records its own errors.
    for r in rows.iter().filter(|r| r.branch.trim().is_empty() && !r.is_git()) {
        let why = match check(state, space, &r.check_input()).await {
            Ok(found) if found.state == "found" => {
                let branch = found.branch.clone().unwrap_or_else(|| "main".into());
                let _ = db.set_dev_repo_branch(sp, &r.key, &branch).await;
                let _ = db.set_dev_repo_hint(sp, &r.key, found.deploy_hint).await;
                continue;
            }
            Ok(found) if found.state == "not_connected" => clients(state).await.err().unwrap_or_else(|| GhError::Auth.to_string()),
            Ok(_) => GhError::NotFound.to_string(),
            Err(e) => e,
        };
        if r.poll_error.as_deref() != Some(why.as_str()) {
            let _ = db.set_dev_repo_error(sp, &r.key, &why).await;
        }
    }

    for s in gh {
        if rows.iter().any(|r| r.key == s.key) {
            continue;
        }
        if rows.len() >= MAX_REPOS {
            out.too_many.push(s.key.clone());
            continue;
        }
        let (repo, branch, hint) = match check(state, space, &s.check_input()).await {
            Ok(found) if found.state == "found" => {
                let full = found.repo.clone().unwrap_or_else(|| s.repo.clone());
                let default = found.branch.clone().unwrap_or_else(|| "main".into());
                // The folder's branch when the repo has it, else the default (0.7.1's choice)
                let branch = match s.branch.as_deref() {
                    Some(b) if b != default => match client_for(state, &full).await {
                        Ok((c, _)) if matches!(c.dev_branch_exists(&full, b).await, Ok(true)) => b.to_string(),
                        _ => default,
                    },
                    _ => default,
                };
                (full, branch, found.deploy_hint)
            }
            // Not there, or no connection reads it: no row; the desk says not_connected
            Ok(_) => continue,
            // GitHub didn't answer: watched anyway, the poller retries and says why
            Err(_) => (s.repo.clone(), s.branch.clone().unwrap_or_else(|| "main".into()), None),
        };
        let host = s.host.as_deref();
        match db.add_dev_repo(sp, &s.key, Some(&s.team_id), &repo, host, &branch, hint, BY_TEAM).await {
            Ok(RepoAdd::Added) => {
                tracing::info!(space = %space, repo = %repo, branch = %branch, "dev: watching a team's repo");
                out.added.push(s.key.clone());
            }
            Ok(RepoAdd::Kept) => {}
            Ok(RepoAdd::TooMany) => out.too_many.push(s.key.clone()),
            Err(e) => tracing::warn!(space = %space, "dev: could not add a repo: {e}"),
        }
        rows = db.dev_repos(sp).await.unwrap_or(rows);
    }
    // Repos GitHub's API can't read: git reads them
    if git::available().await {
        for s in sources.iter().filter(|s| !s.on_github(&hosts)) {
            let key = s.row_key();
            let Some(url) = s.fetch.as_deref() else { continue };
            if rows.iter().any(|r| r.key == key) {
                continue;
            }
            if rows.len() >= MAX_REPOS {
                out.too_many.push(key);
                continue;
            }
            let branch = s.branch.as_deref().unwrap_or("");
            match db.add_dev_repo_git(sp, &key, Some(&s.team_id), &s.repo, s.host.as_deref(), branch, url, BY_TEAM).await {
                Ok(RepoAdd::Added) => {
                    tracing::info!(space = %space, repo = %s.repo, "dev: watching a team's repo with git");
                    out.added.push(key);
                }
                Ok(RepoAdd::Kept) => {}
                Ok(RepoAdd::TooMany) => out.too_many.push(key),
                Err(e) => tracing::warn!(space = %space, "dev: could not add a repo: {e}"),
            }
            rows = db.dev_repos(sp).await.unwrap_or(rows);
        }
    }
    if !out.added.is_empty() {
        super::poller::poll_soon(state, sp);
    }
    out
}

/// [`reconcile`] for a GET: at most once per [`RECONCILE_COOLDOWN`] for the space.
pub async fn reconcile_on_read(state: &AppState, space: &SpaceId) {
    let recent = reconciled().lock().unwrap().get(space.as_str()).is_some_and(|t| t.elapsed() < RECONCILE_COOLDOWN);
    if !recent {
        reconcile(state, space).await;
    }
}

/// [`reconcile`] for every space with Dev mode on: after a GitHub connection is added or changed.
pub async fn reconcile_all(state: &AppState) {
    for row in state.db.dev_mode_on_spaces().await.unwrap_or_default() {
        reconcile(state, &SpaceId::new(&row.space_id)).await;
    }
}

/// [`reconcile`] in the background, after a team is made, changed or deleted: the team route
/// doesn't wait on GitHub.
pub fn reconcile_soon(state: &AppState, space: &SpaceId) {
    let (state, space) = (state.clone(), space.clone());
    tokio::spawn(async move {
        reconcile(&state, &space).await;
    });
}

trait CheckInput {
    fn check_input(&self) -> String;
}

impl CheckInput for DevRepoRow {
    fn check_input(&self) -> String {
        match &self.host {
            Some(h) => format!("{h}/{}", self.repo),
            None => self.repo.clone(),
        }
    }
}

/// The desk's line (plan any-git.md S4), from whether Dev mode is on, the watched repo (`""`:
/// none), the detected sources, whether a GitHub connection exists and whether git is on the
/// server ([`git::available`]):
/// - `watching`: a repo is watched (or, while off, would be: a GitHub source and a connection, or
///   any other source git reads);
/// - `no_repo`: no team points at a git repo;
/// - `not_connected`: a GitHub source that no connection reads;
/// - `no_git`: only non-GitHub or `local` sources, and no git on the server to read them
///   ([`git::INSTALL_HINT`]). 0.7.2's `other_host` is retired: git reads those hosts now.
pub fn status(on: bool, watched: &str, sources: &[Source], gh_hosts: &[String], connected: bool, git: bool) -> &'static str {
    let github = sources.iter().any(|s| s.on_github(gh_hosts));
    let other = sources.iter().any(|s| !s.on_github(gh_hosts) && s.fetch.is_some());
    if !watched.is_empty() {
        "watching"
    } else if sources.is_empty() {
        "no_repo"
    } else if (github && !on && connected) || (other && git) {
        "watching"
    } else if github {
        // On with no repo means switching on found none a connection reads
        "not_connected"
    } else if git {
        // Sources git can't fetch from (an unknown URL scheme)
        "no_repo"
    } else {
        "no_git"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_urls_normalise_to_owner_and_name() {
        let r = |s: &str| normalize(s).map(|(h, r)| (h.unwrap_or_default(), r));
        let gh = |r: &str| Some(("github.com".to_string(), r.to_string()));
        assert_eq!(r("alexsergivan/kwant"), Some((String::new(), "alexsergivan/kwant".into())));
        assert_eq!(r("  alexsergivan/kwant  "), Some((String::new(), "alexsergivan/kwant".into())));
        assert_eq!(r("https://github.com/alexsergivan/kwant"), gh("alexsergivan/kwant"));
        assert_eq!(r("https://github.com/alexsergivan/kwant.git"), gh("alexsergivan/kwant"));
        assert_eq!(r("https://github.com/alexsergivan/kwant/"), gh("alexsergivan/kwant"));
        assert_eq!(r("https://github.com/alexsergivan/kwant/tree/main/src"), gh("alexsergivan/kwant"));
        assert_eq!(r("https://github.com/alexsergivan/kwant/pull/301?x=1#y"), gh("alexsergivan/kwant"));
        assert_eq!(r("https://GitHub.com/alexsergivan/kwant"), gh("alexsergivan/kwant"));
        assert_eq!(r("github.com/alexsergivan/kwant"), gh("alexsergivan/kwant"));
        assert_eq!(r("git@github.com:alexsergivan/kwant.git"), gh("alexsergivan/kwant"));
        assert_eq!(r("git@github.com:alexsergivan/kwant"), gh("alexsergivan/kwant"));
        assert_eq!(r("ssh://git@github.com/alexsergivan/kwant.git"), gh("alexsergivan/kwant"));
        assert_eq!(r("git+https://github.com/alexsergivan/kwant.git"), gh("alexsergivan/kwant"));
        assert_eq!(r("https://ghe.example.com/team/app"), Some(("ghe.example.com".into(), "team/app".into())));
        assert_eq!(r("demo-org/my.site_2"), Some((String::new(), "demo-org/my.site_2".into())));
    }

    #[test]
    fn malformed_repos_are_refused() {
        for bad in [
            "",
            "kwant",
            "owner/",
            "/name",
            "owner/name/extra",
            "owner/..",
            "../name",
            "-owner/name",
            "owner name/x",
            "owner/na me",
            "https://github.com/onlyowner",
            "https://user:secret@github.com/o/r",
            "file:///etc/passwd",
            "javascript:alert(1)/x",
            "owner/name;rm -rf",
            "owner/na%2e%2e",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
        assert_eq!(normalize(&format!("o/{}", "n".repeat(101))), None);
    }

    #[test]
    fn the_web_host_follows_the_api_base() {
        assert_eq!(web_host(&GithubConfig { api_base: "https://api.github.com".into() }), "github.com");
        assert_eq!(web_host(&GithubConfig { api_base: "https://ghe.example.com/api/v3".into() }), "ghe.example.com");
        assert_eq!(web_host(&GithubConfig { api_base: "https://github.acme.com".into() }), "github.acme.com");
        assert_eq!(web_host(&GithubConfig { api_base: "".into() }), "github.com");
    }

    #[test]
    fn a_ghe_com_repo_matches_its_connection() {
        // Saved (0.7.4+) and old (as typed) rows both give the repos' host
        for api in ["https://api.acme.ghe.com", "https://api.acme.ghe.com/", "https://acme.ghe.com", "https://ACME.ghe.com/"] {
            assert_eq!(web_host(&GithubConfig { api_base: api.into() }), "acme.ghe.com", "{api:?}");
        }
        let hosts = vec!["github.com".to_string(), "acme.ghe.com".to_string()];
        assert_eq!(source_repo("https://acme.ghe.com/org/app/tree/main", &hosts, true), Some(("acme.ghe.com".into(), "org/app".into())));
        assert_eq!(source_repo("git@acme.ghe.com:org/app.git", &hosts, false), Some(("acme.ghe.com".into(), "org/app".into())));
        assert_eq!(normalize("https://acme.ghe.com/org/app"), Some((Some("acme.ghe.com".into()), "org/app".into())));
    }

    #[test]
    fn the_deploy_hint_reads_the_repo_root() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(deploy_hint(&v(&["Cargo.toml", "Dockerfile", "fly.toml"])), Some("fly"));
        assert_eq!(deploy_hint(&v(&["package.json", "wrangler.toml"])), Some("cloudflare_pages"));
        assert_eq!(deploy_hint(&v(&["wrangler.jsonc", "Dockerfile"])), Some("cloudflare_pages"));
        assert_eq!(deploy_hint(&v(&["docker-compose.yml"])), Some("docker"));
        assert_eq!(deploy_hint(&v(&["compose.yaml"])), Some("docker"));
        assert_eq!(deploy_hint(&v(&["dockerfile"])), Some("docker"));
        assert_eq!(deploy_hint(&v(&["README.md", "src/"])), None);
    }

    // ---- Detection (plan any-git.md S1, S2): throwaway repos, no network ----------------------

    fn fake_token() -> String {
        // Built at runtime so no scanner flags the source
        ["gh", "p_", "JUNOFAKE", "0123456789abcdefghijklmnopqrstuv"].concat()
    }

    fn sh_git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").output().unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    /// A `git init` checkout in `root/name` on `branch`, with these remotes.
    fn checkout(root: &std::path::Path, name: &str, branch: &str, remotes: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        sh_git(&dir, &["init", "-q", "-b", branch]);
        for (n, url) in remotes {
            sh_git(&dir, &["remote", "add", n, url]);
        }
        dir
    }

    fn team<'a>(name: &'a str, folder: Option<&'a str>, link: Option<&'a str>) -> TeamRef<'a> {
        TeamRef { id: name, name, workspace_path: folder, repo_url: link }
    }

    fn gh() -> Vec<String> {
        vec!["github.com".into()]
    }

    /// A source as `(team, repo, host, from, branch)`, for comparing in one assert.
    type Brief = (String, String, Option<String>, &'static str, Option<String>);

    fn brief(s: &[Source]) -> Vec<Brief> {
        s.iter().map(|s| (s.team.clone(), s.repo.clone(), s.host.clone(), s.from, s.branch.clone())).collect()
    }

    #[test]
    fn userinfo_is_stripped_before_parsing() {
        let t = fake_token();
        assert_eq!(strip_userinfo(&format!("https://x-access-token:{t}@github.com/o/r.git")), "https://github.com/o/r.git");
        assert_eq!(strip_userinfo(&format!("https://{t}@github.com/o/r")), "https://github.com/o/r");
        assert_eq!(strip_userinfo("git@github.com:o/r.git"), "github.com:o/r.git");
        assert_eq!(strip_userinfo(&format!("ssh://git:{t}@gitlab.example.com:2222/g/r.git")), "ssh://gitlab.example.com:2222/g/r.git");
        // A token with a `/` in it can't push its tail past the host
        assert_eq!(strip_userinfo("https://u:ab/cd@host.example/o/r"), "https://host.example/o/r");
        assert_eq!(strip_userinfo("https://github.com/o/r"), "https://github.com/o/r");
    }

    #[test]
    fn remotes_parse_to_host_and_path() {
        let p = |s: &str| parse_remote(s).map(|(h, p)| format!("{h} {p}"));
        let t = fake_token();
        assert_eq!(p("https://github.com/demo-org/app.git").as_deref(), Some("github.com demo-org/app"));
        assert_eq!(p(&format!("https://x-access-token:{t}@github.com/demo-org/app.git")).as_deref(), Some("github.com demo-org/app"));
        assert_eq!(p("git@github.com:demo-org/app.git").as_deref(), Some("github.com demo-org/app"));
        assert_eq!(p("ssh://git@github.com/demo-org/app.git").as_deref(), Some("github.com demo-org/app"));
        assert_eq!(p("ssh://git@GitLab.Example.com:2222/group/sub/app.git").as_deref(), Some("gitlab.example.com group/sub/app"));
        assert_eq!(p("git+https://www.github.com/demo-org/app").as_deref(), Some("github.com demo-org/app"));
        assert_eq!(p("http://127.0.0.1:9495/demo-org/app.git").as_deref(), Some("127.0.0.1 demo-org/app"));
        assert_eq!(p("codeberg.org:me/app.git").as_deref(), Some("codeberg.org me/app"));
        for bad in ["/srv/git/app.git", "./app", "../app.git", "file:///srv/app.git", "C:\\repos\\app", "https://github.com/onlyowner", "", "git@host:o/r; rm -rf /"] {
            assert_eq!(parse_remote(bad), None, "{bad:?}");
        }
        // GitHub repos are `owner/name` even when the link goes on; other hosts keep the path
        assert_eq!(source_repo("https://github.com/o/r/tree/main", &gh(), true), Some(("github.com".into(), "o/r".into())));
        assert_eq!(source_repo("o/r", &gh(), true), Some(("github.com".into(), "o/r".into())));
        assert_eq!(source_repo("github.com/o/r", &gh(), true), Some(("github.com".into(), "o/r".into())));
        assert_eq!(source_repo("https://gitlab.com/g/sub/r", &gh(), true), Some(("gitlab.com".into(), "g/sub/r".into())));
        let ghe = vec!["github.com".to_string(), "ghe.example.com".to_string()];
        assert_eq!(source_repo("git@ghe.example.com:team/app.git", &ghe, true), Some(("ghe.example.com".into(), "team/app".into())));
        // To git, `owner/name` is a folder
        assert_eq!(source_repo("o/r", &gh(), false), None);
    }

    #[tokio::test]
    async fn folders_are_read_by_their_git_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let t = fake_token();
        let https = checkout(root, "https", "main", &[("origin", &format!("https://x-access-token:{t}@github.com/Demo-Org/App.git"))]);
        let scp = checkout(root, "scp", "main", &[("origin", "git@github.com:demo-org/scp-app.git")]);
        let ssh = checkout(root, "ssh", "trunk", &[("origin", "ssh://git@gitlab.example.com/group/sub/app.git")]);
        // No `origin`: the first remote
        let other = checkout(root, "other", "dev", &[("upstream", "https://codeberg.org/me/thing.git")]);
        // origin/HEAD names the default branch over the checked-out one
        sh_git(&scp, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/release"]);
        let (s1, s2, s3, s4) = (https.to_string_lossy(), scp.to_string_lossy(), ssh.to_string_lossy(), other.to_string_lossy());
        let teams = [team("Web", Some(&s1), None), team("Api", Some(&s2), None), team("Ops", Some(&s3), None), team("Docs", Some(&s4), None)];
        let got = detect_teams(&teams, root, &gh()).await;
        let gh_ = Some("github.com".to_string());
        assert_eq!(
            brief(&got),
            vec![
                ("Web".into(), "Demo-Org/App".into(), gh_.clone(), "folder", Some("main".into())),
                ("Api".into(), "demo-org/scp-app".into(), gh_, "folder", Some("release".into())),
                ("Ops".into(), "group/sub/app".into(), Some("gitlab.example.com".into()), "folder", Some("trunk".into())),
                ("Docs".into(), "me/thing".into(), Some("codeberg.org".into()), "folder", Some("dev".into())),
            ]
        );
        assert!(got[0].on_github(&gh()) && !got[2].on_github(&gh()));
        let out = serde_json::to_string(&got).unwrap();
        assert!(!out.contains(&t), "the token left the parser: {out}");
        assert!(!out.contains(&*root.to_string_lossy()), "a folder path is in the reply: {out}");
        assert!(!out.contains("branch") && !out.contains("key"), "only team, repo, host, from: {out}");
    }

    #[tokio::test]
    async fn plain_folders_and_remoteless_checkouts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        // A folder inside someone else's checkout is not a checkout itself
        let outer = checkout(root, "outer", "main", &[("origin", "https://github.com/o/outer.git")]);
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        let local = checkout(root, "notes", "draft", &[]);
        // A remote that is another folder counts as local too
        let mirror = checkout(root, "mirror", "main", &[("origin", "/srv/git/mirror.git")]);
        let (p, i, l, m) = (plain.to_string_lossy(), inner.to_string_lossy(), local.to_string_lossy(), mirror.to_string_lossy());
        let missing = root.join("gone").to_string_lossy().to_string();
        let teams = [team("Plain", Some(&p), None), team("Inner", Some(&i), None), team("Gone", Some(&missing), None), team("Notes", Some(&l), None), team("Mirror", Some(&m), None)];
        let got = detect_teams(&teams, root, &gh()).await;
        assert_eq!(brief(&got), vec![("Notes".into(), "notes".into(), None, "local", Some("draft".into())), ("Mirror".into(), "mirror".into(), None, "local", Some("main".into()))]);
        // An empty space has nothing
        assert!(detect_teams(&[], root, &gh()).await.is_empty());
        // A team with no folder setting uses the space's shared folder (`<space>/workspace`)
        let shared = checkout(root, "workspace", "main", &[("origin", "git@github.com:o/shared.git")]);
        assert!(shared.ends_with("workspace"));
        assert_eq!(brief(&detect_teams(&[team("Any", None, None)], root, &gh()).await), vec![("Any".into(), "o/shared".into(), Some("github.com".into()), "folder", Some("main".into()))]);
    }

    #[tokio::test]
    async fn sources_dedupe_by_repo_and_keep_team_order() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let a = checkout(root, "a", "main", &[("origin", "https://github.com/demo-org/app.git")]);
        let b = checkout(root, "b", "main", &[("origin", "git@github.com:DEMO-ORG/APP.git")]);
        let c = checkout(root, "c", "main", &[("origin", "https://github.com/demo-org/site.git")]);
        let (a, b, c) = (a.to_string_lossy(), b.to_string_lossy(), c.to_string_lossy());
        // Two teams on one repo: the first names it
        let got = detect_teams(&[team("First", Some(&a), None), team("Second", Some(&b), None)], root, &gh()).await;
        assert_eq!(got.iter().map(|s| s.team.as_str()).collect::<Vec<_>>(), ["First"]);
        // Two repos: both, in team order
        let got = detect_teams(&[team("Site", Some(&c), None), team("App", Some(&a), None)], root, &gh()).await;
        assert_eq!(got.iter().map(|s| (s.team.as_str(), s.repo.as_str())).collect::<Vec<_>>(), [("Site", "demo-org/site"), ("App", "demo-org/app")]);
        // A repo link wins over the folder, and a link to the same repo as a folder dedupes
        let t = fake_token();
        let link = format!("https://bot:{t}@github.com/demo-org/linked");
        let got = detect_teams(&[team("Linked", Some(&c), Some(&link)), team("App", Some(&a), Some("demo-org/app")), team("Again", Some(&a), None)], root, &gh()).await;
        assert_eq!(
            brief(&got),
            vec![("Linked".into(), "demo-org/linked".into(), Some("github.com".into()), "repo", Some("main".into())), ("App".into(), "demo-org/app".into(), Some("github.com".into()), "repo", Some("main".into()))]
        );
        assert!(!format!("{got:?}").contains(&t), "the token reached a Source");
        // Two local folders are two sources; the same folder twice is one
        let n = checkout(root, "n", "main", &[]).to_string_lossy().to_string();
        let got = detect_teams(&[team("X", Some(&n), None), team("Y", Some(&n), None)], root, &gh()).await;
        assert_eq!(got.len(), 1);
    }

    /// Another team's private checkout (G2 delta D1): its remote carries a token and its branch a
    /// name only that team should see.
    fn victim(root: &std::path::Path) -> std::path::PathBuf {
        let t = fake_token();
        checkout(root, "victim", "b-private-branch", &[("origin", &format!("https://bob:{t}@git.example.org/b/private.git"))])
    }

    /// What a team's folder detection must never show: the other repo, its branch or its token.
    fn shows_nothing_of(got: &[Source], what: &str) {
        let out = format!("{got:?}");
        assert!(got.is_empty(), "{what}: detected {out}");
        for leak in ["private", "b-private-branch", "git.example.org", &fake_token()] {
            assert!(!out.contains(leak), "{what}: {leak} leaked");
        }
    }

    #[tokio::test]
    async fn a_team_folder_whose_git_is_a_symlink_to_another_repo_is_not_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let v = victim(root);
        let a = root.join("team-a");
        std::fs::create_dir_all(&a).unwrap();
        std::os::unix::fs::symlink(v.join(".git"), a.join(".git")).unwrap();
        let a = a.to_string_lossy();
        shows_nothing_of(&detect_teams(&[team("A", Some(&a), None)], root, &gh()).await, ".git symlink");
        assert!(read_folder(std::path::Path::new(&*a)).await.is_none());
    }

    #[tokio::test]
    async fn a_team_folder_whose_git_file_names_another_repo_is_not_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let v = victim(root);
        let a = root.join("team-a");
        let r = root.join("team-r");
        for (dir, target) in [(&a, v.join(".git").to_string_lossy().to_string()), (&r, "../victim/.git".to_string())] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(".git"), format!("gitdir: {target}\n")).unwrap();
        }
        let (a, r) = (a.to_string_lossy(), r.to_string_lossy());
        shows_nothing_of(&detect_teams(&[team("A", Some(&a), None)], root, &gh()).await, "gitdir: absolute");
        shows_nothing_of(&detect_teams(&[team("R", Some(&r), None)], root, &gh()).await, "gitdir: relative");
    }

    #[tokio::test]
    async fn a_real_worktree_is_still_detected_and_one_borrowing_its_gitdir_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let main = checkout(root, "main", "main", &[("origin", "https://git.example.org/team/app.git")]);
        sh_git(&main, &["-c", "user.name=t", "-c", "user.email=t@example.org", "commit", "-q", "--allow-empty", "-m", "first"]);
        let wt = root.join("wt");
        sh_git(&main, &["worktree", "add", "-q", "-b", "feature", wt.to_str().unwrap()]);
        assert!(wt.join(".git").is_file(), "git wrote a .git file");
        // Same `gitdir:` line as the real worktree, but the git dir points back at `wt`, not here
        let forged = root.join("forged");
        std::fs::create_dir_all(&forged).unwrap();
        std::fs::copy(wt.join(".git"), forged.join(".git")).unwrap();
        let (w, f) = (wt.to_string_lossy(), forged.to_string_lossy());
        // The forged team comes first: had it been detected, it would take the repo from `wt`
        let got = detect_teams(&[team("Forged", Some(&f), None), team("Wt", Some(&w), None)], root, &gh()).await;
        assert_eq!(brief(&got), vec![("Wt".into(), "team/app".into(), Some("git.example.org".into()), "folder", Some("feature".into()))]);
        // An ordinary checkout still reads as before
        let m = main.to_string_lossy();
        assert_eq!(brief(&detect_teams(&[team("Main", Some(&m), None)], root, &gh()).await), vec![("Main".into(), "team/app".into(), Some("git.example.org".into()), "folder", Some("main".into()))]);
    }

    #[test]
    fn the_status_says_what_the_desk_shows() {
        let src = |host: Option<&str>| Source { team: "T".into(), team_id: "t".into(), repo: "o/r".into(), host: host.map(str::to_string), from: "folder", branch: None, key: String::new(), fetch: Some("https://x.example/o/r".into()) };
        let github = [src(Some("github.com"))];
        let gitlab = [src(Some("gitlab.com")), src(None)];
        assert_eq!(status(true, "o/r", &github, &gh(), false, true), "watching");
        assert_eq!(status(true, "", &[], &gh(), false, true), "no_repo");
        assert_eq!(status(false, "", &[], &gh(), true, true), "no_repo");
        assert_eq!(status(true, "", &github, &gh(), true, true), "not_connected", "on with no repo: no connection read it");
        assert_eq!(status(false, "", &github, &gh(), false, true), "not_connected");
        assert_eq!(status(false, "", &github, &gh(), true, true), "watching", "off: turning on would watch it");
        assert_eq!(status(true, "", &gitlab, &gh(), true, true), "watching", "git reads other hosts");
        assert_eq!(status(true, "", &gitlab, &gh(), true, false), "no_git");
        assert_eq!(status(false, "", &gitlab, &gh(), false, true), "watching", "off: turning on would watch it");
        let mixed = [src(Some("github.com")), src(Some("gitlab.com"))];
        assert_eq!(status(true, "", &mixed, &gh(), false, false), "not_connected");
        assert_eq!(status(true, "", &mixed, &gh(), false, true), "watching");
        let unread = [Source { fetch: None, ..src(Some("gitlab.com")) }];
        assert_eq!(status(true, "", &unread, &gh(), false, true), "no_repo", "nothing git can fetch");
        // A GitHub Enterprise host is GitHub
        let ghe = vec!["github.com".to_string(), "gitlab.com".to_string()];
        assert_eq!(status(true, "", &gitlab, &ghe, false, false), "not_connected");
    }
}
