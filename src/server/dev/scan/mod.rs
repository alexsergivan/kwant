//! The review card's security tiles (plan §3.4, Juno): a gitleaks pass over the added lines of the
//! whole PR diff, and a check of the packages the PR adds against npm, PyPI and crates.io.
//!
//! The API is small: [`scan`] takes the PR's files with their full patches (before the 60k cut),
//! the head text of its manifests (and of any file GitHub sent no patch for) and returns a
//! [`Scan`]: the `security` value for the card, its findings, and the added lines to hide from
//! the stored facts. [`for_review`] is the poller's glue: it fetches what [`scan`] needs from
//! GitHub, then hides the secret lines in the patches before they are cut, stored or shown.
//!
//! Rules this module keeps: a tile reads `pass` only when its check ran over everything it had
//! to; anything that stopped it (a bad checksum, a failed fetch, a timeout, a registry error, a
//! file it could not read) reads `not_run` with the reason. A secret's value never leaves the
//! scanner: gitleaks runs with `--redact`, findings carry the rule, file and line only, and the
//! whole added line is replaced in the stored patch.

pub mod gitleaks;
pub mod packages;

use super::facts::FileDiff;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Most secret findings listed (the tile still counts every hit, and every hit is hidden).
const MAX_SECRET_FINDINGS: usize = 20;

/// Shown in place of an added line gitleaks flagged, in the facts, the model's prompt and the card.
pub const HIDDEN_LINE: &str = "[kwant: line hidden, possible secret]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pass,
    Hit,
    #[default]
    NotRun,
}

/// One tile of the card's `security` object. `missing` and `reason` are add-only fields on top
/// of plan §3.4: `missing` is how many of the package hits don't exist (what forces High), and
/// `reason` says why a tile is `not_run` (or what was left out of a `hit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Tile {
    pub state: State,
    pub count: u32,
    #[serde(default)]
    pub missing: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Tile {
    pub fn not_run(reason: impl Into<String>) -> Tile {
        Tile { state: State::NotRun, count: 0, missing: 0, reason: Some(reason.into()) }
    }

    /// `hit` when anything hit, else `not_run` when anything stopped the check, else `pass`.
    fn of(count: u32, missing: u32, stopped: &[String]) -> Tile {
        let reason = (!stopped.is_empty()).then(|| stopped.join("; "));
        let state = if count > 0 {
            State::Hit
        } else if reason.is_some() {
            State::NotRun
        } else {
            State::Pass
        };
        Tile { state, count, missing, reason }
    }
}

/// A finding for the card's `findings` list (plan §3.4). Never holds a secret's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub file: String,
    pub line: Option<u64>,
    pub detail: String,
    pub fix: String,
    /// `gitleaks` or `packages`.
    pub source: String,
}

impl Finding {
    /// The finding as the card stores it: open, never sent or dismissed.
    pub fn card_json(&self) -> Value {
        json!({"id": self.id, "severity": self.severity, "title": self.title, "file": self.file, "line": self.line,
               "detail": self.detail, "fix": self.fix, "source": self.source, "state": "open", "sent_at": null, "dismissed": null})
    }
}

/// What the scan found, kept in `DevFacts.security`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Security {
    pub secrets: Tile,
    pub packages: Tile,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

impl Security {
    /// The card's `security` object: the two tiles plus the checks that come later.
    pub fn card_json(&self) -> Value {
        json!({"secrets": self.secrets, "packages": self.packages, "osv": {"state": "later"}, "semgrep": {"state": "later"}})
    }

    /// Both tiles `not_run` with the same reason.
    pub fn not_run(reason: &str) -> Security {
        Security { secrets: Tile::not_run(reason), packages: Tile::not_run(reason), findings: vec![] }
    }
}

/// Where the scanners reach out. Production uses [`Config::new`]; tests point both at mocks.
#[derive(Debug, Clone)]
pub struct Config {
    /// Where the pinned gitleaks binary is cached (`<data_dir>/tools`).
    pub tools_dir: PathBuf,
    /// Where gitleaks is downloaded from and the checksums it must match; none on a platform
    /// with no pinned build.
    pub gitleaks: Option<gitleaks::Source>,
    /// How long one gitleaks run may take.
    pub gitleaks_timeout: std::time::Duration,
    pub registries: packages::Registries,
}

impl Config {
    /// The production setup. Test seams (QA plan §2.2): `KWANT_DEV_GITLEAKS_URL` replaces the
    /// release download base (the pinned checksums still apply) and `KWANT_DEV_REGISTRIES_URL`
    /// sends every registry lookup to one mock (`/npm`, `/npm-downloads`, `/pypi`, `/pypistats`,
    /// `/crates`).
    pub fn new(data_dir: &std::path::Path) -> Config {
        let mut gl = gitleaks::Source::pinned();
        if let (Some(src), Ok(base)) = (gl.as_mut(), std::env::var("KWANT_DEV_GITLEAKS_URL")) {
            src.url = gitleaks::Source::url_for(base.trim_end_matches('/'), src.archive);
        }
        let registries = match std::env::var("KWANT_DEV_REGISTRIES_URL") {
            Ok(base) => packages::Registries::all_at(base.trim_end_matches('/')),
            Err(_) => packages::Registries::default(),
        };
        Config { tools_dir: data_dir.join("tools"), gitleaks: gl, gitleaks_timeout: gitleaks::RUN_TIMEOUT, registries }
    }
}

/// What [`scan`] reads.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// The PR's files with their full patches (nothing cut).
    pub files: Vec<FileDiff>,
    /// Head text by path: the changed manifests, and files GitHub sent no patch for.
    pub contents: BTreeMap<String, String>,
    /// Set when the file list itself was incomplete (GitHub's list stops at a cap).
    pub files_capped: Option<String>,
}

/// The scan's answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    pub security: Security,
    /// Added lines to hide, by path: inclusive new-side line ranges.
    pub hide: BTreeMap<String, Vec<(u64, u64)>>,
}

/// Runs both checks. Never fails: whatever stops a check shows as its tile's `not_run`.
pub async fn scan(cfg: &Config, input: &Input) -> Scan {
    let (secrets, mut findings, hide) = secrets(cfg, input).await;
    let (pkgs, pkg_findings) = packages::run(&cfg.registries, &input.files, &input.contents).await;
    findings.extend(pkg_findings);
    Scan { security: Security { secrets, packages: pkgs, findings }, hide }
}

async fn secrets(cfg: &Config, input: &Input) -> (Tile, Vec<Finding>, BTreeMap<String, Vec<(u64, u64)>>) {
    let mut stopped: Vec<String> = input.files_capped.iter().cloned().collect();
    let targets = gitleaks::targets(&input.files, &input.contents);
    if !targets.unread.is_empty() {
        stopped.push(format!("{} not scanned: GitHub sent no diff ({})", plural(targets.unread.len(), "file"), sample(&targets.unread)));
    }
    let hits = match cfg.gitleaks.as_ref() {
        None => return (Tile::not_run("gitleaks has no pinned build for this computer"), vec![], BTreeMap::new()),
        Some(_) if targets.files.is_empty() => Ok(vec![]),
        Some(src) => match gitleaks::ensure(&cfg.tools_dir, src).await {
            Ok(bin) => gitleaks::run(&bin, &targets.files, cfg.gitleaks_timeout).await,
            Err(e) => Err(e),
        },
    };
    let hits = match hits {
        Ok(h) => h,
        Err(reason) => return (Tile::not_run(reason), vec![], BTreeMap::new()),
    };
    let mut hide: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    let mut findings = Vec::new();
    for (i, h) in hits.iter().enumerate() {
        hide.entry(h.file.clone()).or_default().push((h.start, h.end.max(h.start)));
        if i >= MAX_SECRET_FINDINGS {
            continue;
        }
        findings.push(Finding {
            id: format!("s{}", i + 1),
            severity: "critical".into(),
            title: format!("Possible secret ({})", h.rule),
            file: h.file.clone(),
            line: Some(h.start),
            detail: format!(
                "gitleaks rule {} matched an added line. The value is hidden here and in kwant's copy of the diff.",
                h.rule
            ),
            fix: "Remove the value from the change and its history, rotate it if it is real, and read it from the environment or a secret store instead."
                .into(),
            source: "gitleaks".into(),
        });
    }
    (Tile::of(hits.len() as u32, 0, &stopped), findings, hide)
}

/// Replaces the added lines in `ranges` (new-side line numbers) with [`HIDDEN_LINE`]. Hunks keep
/// their shape, so line numbers in the rest of the patch stay right.
pub fn hide_lines(patch: &str, ranges: &[(u64, u64)]) -> String {
    let mut out = Vec::new();
    let mut new_line = 0u64;
    for line in patch.split('\n') {
        if let Some(start) = hunk_new_start(line) {
            new_line = start;
            out.push(line.to_string());
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                let hit = ranges.iter().any(|&(a, b)| (a..=b).contains(&new_line));
                out.push(if hit { format!("+{HIDDEN_LINE}") } else { line.to_string() });
                new_line += 1;
            }
            Some(b'-') | Some(b'\\') => out.push(line.to_string()),
            _ => {
                out.push(line.to_string());
                new_line += 1;
            }
        }
    }
    out.join("\n")
}

/// `c` from a hunk header `@@ -a,b +c,d @@`.
pub(crate) fn hunk_new_start(line: &str) -> Option<u64> {
    let rest = line.strip_prefix("@@ -")?;
    let plus = rest.split_whitespace().nth(1)?.strip_prefix('+')?;
    plus.split(',').next()?.parse().ok()
}

/// `a` and `c` from a hunk header.
pub(crate) fn hunk_starts(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("@@ -")?;
    let old = rest.split_whitespace().next()?.split(',').next()?.parse().ok()?;
    Some((old, hunk_new_start(line)?))
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// Up to three paths, then "…".
fn sample(paths: &[String]) -> String {
    let mut s = paths.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if paths.len() > 3 {
        s.push_str(", …");
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Poller glue

/// Most patchless files whose head text is fetched for the secrets scan.
const MAX_FETCHED: usize = 20;

/// Scans an open PR head before its facts are built: fetches the manifests and patchless files
/// at `head_sha`, runs [`scan`], and hides the flagged lines in `files`' patches in place.
pub async fn for_review(
    data_dir: &std::path::Path,
    client: &crate::zombie::github::GithubSource,
    repo: &str,
    head_sha: &str,
    files: &mut [crate::zombie::github::GhFile],
    capped: bool,
) -> Security {
    let mut contents = BTreeMap::new();
    let wanted = files
        .iter()
        .filter(|f| f.status != "removed")
        .filter(|f| packages::manifest_kind(&f.filename).is_some() || (f.patch.is_none() && gitleaks::needs_text(&diff_of(f))))
        .take(MAX_FETCHED);
    for f in wanted {
        if let Ok(Some(text)) = client.dev_file_at(repo, &f.filename, head_sha).await {
            contents.insert(f.filename.clone(), text);
        }
    }
    let capped = capped.then(|| format!("the PR has more than {} files; the rest weren't read", files.len()));
    scan_and_hide(&Config::new(data_dir), files, contents, capped).await
}

/// [`scan`], then the flagged added lines replaced in `files`' patches.
pub async fn scan_and_hide(
    cfg: &Config,
    files: &mut [crate::zombie::github::GhFile],
    contents: BTreeMap<String, String>,
    files_capped: Option<String>,
) -> Security {
    let input = Input { files: files.iter().map(diff_of).collect(), contents, files_capped };
    let out = scan(cfg, &input).await;
    for f in files.iter_mut() {
        if let (Some(ranges), Some(p)) = (out.hide.get(&f.filename), f.patch.as_mut()) {
            *p = hide_lines(p, ranges);
        }
    }
    out.security
}

fn diff_of(f: &crate::zombie::github::GhFile) -> FileDiff {
    FileDiff {
        path: f.filename.clone(),
        status: f.status.clone(),
        additions: f.additions,
        deletions: f.deletions,
        patch: f.patch.clone(),
        previous_path: f.previous_filename.clone(),
    }
}

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
mod tests;
