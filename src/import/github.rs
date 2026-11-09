//! Import from a GitHub link (0.8.0 A3). Only `github.com` links are accepted, and the only host
//! ever contacted is `codeload.github.com`: one `GET` of the repository zip, no redirects
//! followed, no cookies, no token (public repositories only), at most 20 MB.

use super::parsers::{self, RepoZip};
use super::{Bundle, ImportError, MAX_ZIP_BYTES};
use serde::Serialize;
use std::time::Duration;

pub const CODELOAD: &str = "https://codeload.github.com";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_PART: usize = 100;

/// A parsed `github.com` link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLink {
    pub owner: String,
    pub repo: String,
    /// `None`: the default branch (`HEAD`).
    pub ref_: Option<String>,
    /// A folder (`/tree/…`) or file (`/blob/…`) inside the repository.
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Licence {
    pub spdx: Option<String>,
    pub name: String,
    pub file: String,
}

/// The `github` block of a preview.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GithubInfo {
    pub owner: String,
    pub repo: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub path: Option<String>,
    pub commit: Option<String>,
    pub licence: Option<Licence>,
}

#[derive(Debug, PartialEq)]
pub enum GithubError {
    /// 422 `not_github`
    NotGithub(String),
    /// 422 `redirect`
    Redirect,
    /// 404 `not_found`
    NotFound,
    /// 502 `fetch_failed`
    FetchFailed(String),
    /// 413 / 415 / 422 from the zip reader
    Import(ImportError),
}

fn not_github(msg: &str) -> GithubError {
    GithubError::NotGithub(msg.into())
}

fn plain_part(p: &str) -> bool {
    !p.is_empty() && p.len() <= MAX_PART && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Reads a link the person pasted. Refuses any host but `github.com` / `www.github.com`, any
/// scheme but https, user info, a port, and shapes other than the repo, `/tree/` and `/blob/`.
pub fn parse_link(raw: &str) -> Result<RepoLink, GithubError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2048 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(not_github("Paste a link to a repository on github.com."));
    }
    // `url` would quietly resolve `..`, `.` and their encoded forms: refuse them as written
    let lower = raw.to_lowercase();
    if ["%2e", "%2f", "%5c", "\\"].iter().any(|m| lower.contains(m)) || raw.split(['/', '?', '#']).any(|s| s == ".." || s == ".") {
        return Err(not_github("Use a link to a repository, a folder (…/tree/<branch>/…) or a file (…/blob/<branch>/…) on github.com."));
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&with_scheme).map_err(|_| not_github("That isn't a link kwant can read."))?;
    if url.scheme() != "https" {
        return Err(not_github("Only https links to github.com are supported."));
    }
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err(not_github("Links with a user name, password or port aren't supported."));
    }
    match url.host() {
        Some(url::Host::Domain(h)) if h.eq_ignore_ascii_case("github.com") || h.eq_ignore_ascii_case("www.github.com") => {}
        _ => return Err(not_github("Only links to github.com are supported.")),
    }
    let parts: Vec<&str> = url.path().trim_end_matches('/').split('/').skip(1).collect();
    let bad_shape = || not_github("Use a link to a repository, a folder (…/tree/<branch>/…) or a file (…/blob/<branch>/…) on github.com.");
    let (owner, repo, rest) = match parts.as_slice() {
        [o, r, rest @ ..] => (*o, r.strip_suffix(".git").unwrap_or(r), rest),
        _ => return Err(bad_shape()),
    };
    if !plain_part(owner) || !plain_part(repo) {
        return Err(bad_shape());
    }
    let (ref_, path) = match rest {
        [] => (None, None),
        ["tree", r] => (Some(*r), None),
        ["tree", r, p @ ..] | ["blob", r, p @ ..] if !p.is_empty() => {
            // `url` already resolved `.` and `..`; refuse anything odd that is left
            if p.iter().any(|s| s.is_empty() || *s == "." || *s == ".." || s.contains('%')) {
                return Err(bad_shape());
            }
            (Some(*r), Some(p.join("/")))
        }
        _ => return Err(bad_shape()),
    };
    if let Some(r) = ref_ {
        if !plain_part(r) {
            return Err(not_github("Branch names with / or other special characters aren't supported yet."));
        }
    }
    Ok(RepoLink { owner: owner.into(), repo: repo.into(), ref_: ref_.map(Into::into), path })
}

/// `https://codeload.github.com/<owner>/<repo>/zip/<ref>`; `base` is swapped only in tests.
pub fn zip_url(base: &str, link: &RepoLink) -> String {
    format!("{base}/{}/{}/zip/{}", link.owner, link.repo, link.ref_.as_deref().unwrap_or("HEAD"))
}

/// Fetches the zip at `url`: no redirects, 20 MB at most (refused up front when the length says
/// so, counted while streaming otherwise).
pub async fn fetch(url: &str) -> Result<Vec<u8>, GithubError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .user_agent(concat!("kwant/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| GithubError::FetchFailed("kwant couldn't start the download.".into()))?;
    let failed = |e: reqwest::Error| {
        let why = if e.is_timeout() { "GitHub took too long to answer" } else { "kwant couldn't reach GitHub" };
        GithubError::FetchFailed(format!("{why}. Try again in a moment."))
    };
    let mut resp = client.get(url).send().await.map_err(failed)?;
    let status = resp.status();
    if status.is_redirection() {
        return Err(GithubError::Redirect);
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(GithubError::NotFound);
    }
    if !status.is_success() {
        return Err(GithubError::FetchFailed(format!("GitHub answered {}. Try again in a moment.", status.as_u16())));
    }
    let too_large = || GithubError::Import(ImportError::TooLarge("This repository's zip is larger than 20 MB. Link to a folder inside it, or download and zip only the parts you want.".into()));
    if resp.content_length().is_some_and(|n| n > MAX_ZIP_BYTES as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(failed)? {
        if body.len() + chunk.len() > MAX_ZIP_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A best guess at the SPDX id of a licence text.
pub fn guess_spdx(text: &str) -> Option<(&'static str, &'static str)> {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let has = |s: &str| t.contains(s);
    Some(if has("gnu affero general public license") {
        ("AGPL-3.0", "GNU Affero General Public License v3.0")
    } else if has("gnu lesser general public license") {
        ("LGPL-3.0", "GNU Lesser General Public License v3.0")
    } else if has("gnu general public license") && has("version 3") {
        ("GPL-3.0", "GNU General Public License v3.0")
    } else if has("gnu general public license") && has("version 2") {
        ("GPL-2.0", "GNU General Public License v2.0")
    } else if has("mozilla public license") && has("2.0") {
        ("MPL-2.0", "Mozilla Public License 2.0")
    } else if has("apache license") && has("version 2.0") {
        ("Apache-2.0", "Apache License 2.0")
    } else if has("this is free and unencumbered software released into the public domain") {
        ("Unlicense", "The Unlicense")
    } else if has("cc0 1.0 universal") {
        ("CC0-1.0", "Creative Commons Zero v1.0 Universal")
    } else if has("attribution-sharealike 4.0") {
        ("CC-BY-SA-4.0", "Creative Commons Attribution Share Alike 4.0")
    } else if has("attribution 4.0 international") {
        ("CC-BY-4.0", "Creative Commons Attribution 4.0")
    } else if has("permission is hereby granted, free of charge") {
        ("MIT", "MIT License")
    } else if has("permission to use, copy, modify, and/or distribute this software for any purpose with or without fee") {
        ("ISC", "ISC License")
    } else if has("redistribution and use in source and binary forms") {
        if has("neither the name") || has("may be used to endorse or promote") {
            ("BSD-3-Clause", "BSD 3-Clause License")
        } else {
            ("BSD-2-Clause", "BSD 2-Clause License")
        }
    } else {
        return None;
    })
}

pub fn licence_of(found: Option<(String, String)>) -> Option<Licence> {
    let (file, text) = found?;
    Some(match guess_spdx(&text) {
        Some((spdx, name)) => Licence { spdx: Some(spdx.into()), name: name.into(), file },
        None => Licence { spdx: None, name: "Unknown licence".into(), file },
    })
}

pub const NO_LICENCE: &str = "This repository has no licence file, so you may not have the right to reuse it.";

/// Reads the fetched zip into a bundle and the `github` block.
pub fn read(link: &RepoLink, zip: &[u8]) -> Result<(Bundle, GithubInfo), GithubError> {
    if !zip.starts_with(b"PK") {
        return Err(GithubError::Import(ImportError::Invalid("GitHub didn't send a zip.".into())));
    }
    let (mut bundle, RepoZip { commit, licence }) = parsers::parse_repo_zip(zip, link.path.as_deref()).map_err(GithubError::Import)?;
    let licence = licence_of(licence);
    let shown = format!("github.com/{}/{}", link.owner, link.repo);
    bundle.source.label = Some(match &link.path {
        Some(p) => format!("{shown}/{p}"),
        None => shown.clone(),
    });
    let which = licence.as_ref().map_or("no licence file".to_string(), |l| l.spdx.clone().unwrap_or_else(|| l.name.clone()));
    bundle.omitted.push(format!("From {shown} ({which})"));
    let commit = commit.map(|c| c.trim().to_string()).filter(|c| c.len() == 40 && c.chars().all(|ch| ch.is_ascii_hexdigit()));
    let info = GithubInfo {
        owner: link.owner.clone(),
        repo: link.repo.clone(),
        ref_: link.ref_.clone().unwrap_or_else(|| "HEAD".into()),
        path: link.path.clone(),
        commit,
        licence,
    };
    Ok((bundle, info))
}

/// Fetches and reads `raw` (a pasted link) from `base` (codeload outside tests).
pub async fn preview(base: &str, raw: &str) -> Result<(Bundle, GithubInfo), GithubError> {
    let link = parse_link(raw)?;
    let zip = fetch(&zip_url(base, &link)).await?;
    read(&link, &zip)
}

#[cfg(test)]
mod tests;
