//! The repo check (plan §3.2 `POST /dev_mode/check`, D4): one GitHub repo per space, read
//! through the Loose ends GitHub connection (its vault token and Enterprise `api_base`).
//! kwant writes nothing to GitHub.

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

/// The web host repos of this API base live on: `api.github.com` → `github.com`,
/// `https://ghe.example/api/v3` → `ghe.example`.
pub fn web_host(cfg: &GithubConfig) -> String {
    let host = url::Url::parse(cfg.api_base.trim()).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default();
    if host == "api.github.com" {
        "github.com".into()
    } else {
        host
    }
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
}

impl Check {
    fn of(state: &'static str, repo: Option<String>) -> Self {
        Check { state, repo, branch: None, deploy_hint: None }
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

/// The client whose connection can read `repo` (the first that finds it), with the repo.
pub async fn client_for(state: &AppState, repo: &str) -> Result<(GithubSource, crate::zombie::github::GhRepo), GhError> {
    let list = clients(state).await.map_err(|_| GhError::Auth)?;
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

/// The repo a space's teams point at, normalised: the first team whose `repo_url` reads.
pub async fn prefill(state: &AppState, space: &SpaceId) -> Option<String> {
    let teams = state.db.list_teams(space).await.ok()?;
    teams.iter().filter_map(|t| t.repo_url.as_deref()).find_map(normalize).map(|(_, r)| r)
}

/// Checks `input` (or the team's repo when it's empty). `Err` is a GitHub failure that is
/// neither found nor not found (a rate limit, GitHub down): the route answers 502 with it.
pub async fn check(state: &AppState, space: &SpaceId, input: &str) -> Result<Check, String> {
    let input = if input.trim().is_empty() { prefill(state, space).await.unwrap_or_default() } else { input.to_string() };
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
                answer = Check { state: "found", repo: Some(r.full_name.clone()), branch: Some(r.default_branch.clone()), deploy_hint: deploy_hint(&root) };
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
}
