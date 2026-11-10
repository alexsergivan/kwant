//! H1 + P3 (0.9.0): a worktree per bot task, and cleaning finished worktrees' build output.
//! The API is release-090/h1-contract.md; Mira's desk is release-090/designs/h1-worktrees.md.
//!
//! Borrowed from herdr `src/worktree.rs` (Apache-2.0, read at ee5bfb5), ideas only: the
//! `<root>/<repo>/<branch-slug>` layout, "remove never deletes the branch", "a dirty checkout is
//! refused unless forced", and "a leftover folder is removed only if its `.git` points into this
//! repo's `worktrees/`".
//!
//! Every git call goes through [`git::host_git_command`] with Dev mode's safe `-c` set, and blanks
//! each `filter.*` driver the repo's own config sets ([`Git::open`]): a bot can write those into
//! the team repo's `.git/config`, and a checkout or a status read would run them as the person.

use super::git;
use crate::db::dev_worktrees::{NewWorktree, WorktreeRow};
use crate::db::spaces::SpaceId;
use crate::server::AppState;
use axum::{
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// The only folder names P3 removes.
pub const BUILD_DIRS: [&str; 5] = ["node_modules", "dist", "target", ".next", ".turbo"];
/// P3 runs this long after the work finished.
pub const CLEAN_AFTER_MINUTES: i64 = 60;
/// How often the P3 sweep looks.
pub const SWEEP_EVERY: Duration = Duration::from_secs(600);
/// A read (status, list, rev-parse) may take this long.
const READ_TIMEOUT: Duration = Duration::from_secs(3);
/// A checkout or a removal may take this long.
const WRITE_TIMEOUT: Duration = Duration::from_secs(120);
/// Changed files listed with a dirty count.
const LISTED_CHANGES: usize = 8;
/// A branch slug is at most this long.
const SLUG_MAX: usize = 60;
/// P3 looks this deep into a worktree, and at this many folders at most.
const WALK_DEPTH: usize = 12;
const WALK_DIRS: usize = 20_000;

// ---------------------------------------------------------------------------------------------
// Names and folders

/// A folder name from a branch or a folder name: lowercased, every run of characters outside
/// `[a-z0-9_]` turned into one `-`, trimmed of `-`, at most [`SLUG_MAX`] characters; `worktree`
/// when nothing is left. `lin/H1 Worktrees` → `lin-h1-worktrees`. Never `.`/`..`, never hidden.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.trim_matches('-').chars().take(SLUG_MAX).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "worktree".into()
    } else {
        out
    }
}

/// A space's worktree root: `<space data dir>/worktrees`.
pub fn root_of(data_dir: &Path, space: &SpaceId) -> PathBuf {
    crate::db::spaces::space_data_dir(data_dir, space).join("worktrees")
}

/// Makes `root/<parts…>` as real folders, refusing any part that is a symlink or not a folder
/// (a bot that can write next to the root could plant `worktrees/web` → `/somewhere`).
fn real_dirs(root: &Path, parts: &[&str]) -> Result<PathBuf, WtError> {
    std::fs::create_dir_all(root).map_err(|_| WtError::Failed)?;
    let meta = std::fs::symlink_metadata(root).map_err(|_| WtError::Failed)?;
    if !meta.is_dir() {
        return Err(WtError::Failed);
    }
    let mut at = root.to_path_buf();
    for p in parts {
        at.push(p);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(WtError::FolderTaken),
            Err(_) => std::fs::create_dir(&at).map_err(|_| WtError::Failed)?,
        }
    }
    Ok(at)
}

/// `path` with `.`/`..` resolved by text alone (no filesystem): what a `.git` file names.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Whether the folder `dir` was a linked worktree of the repo whose git dir is `common`: its
/// `.git` is a small regular file saying `gitdir: <common>/worktrees/<name>` (absolute, or
/// relative to `dir`). Works after git forgot the worktree (a leftover), when the admin folder
/// is gone and `gitdir` can't be followed.
pub fn points_into(dir: &Path, common: &Path) -> bool {
    let dot_git = dir.join(".git");
    let Ok(meta) = std::fs::symlink_metadata(&dot_git) else { return false };
    if !meta.is_file() || meta.len() > 4096 {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&dot_git) else { return false };
    let Some(target) = text.trim_end_matches(['\n', '\r']).strip_prefix("gitdir: ").filter(|t| !t.is_empty()) else { return false };
    let target = Path::new(target);
    let target = lexical(&if target.is_absolute() { target.to_path_buf() } else { dir.join(target) });
    let admin = common.join("worktrees");
    target.parent() == Some(admin.as_path()) && target.file_name().is_some()
}

// ---------------------------------------------------------------------------------------------
// Errors

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WtError {
    BadBranch,
    BadBase,
    UnknownBranch,
    UnknownBase,
    NotARepo,
    BranchExists,
    BranchCheckedOut,
    FolderTaken,
    NoGit,
    /// Changed or untracked files: how many, and the first [`LISTED_CHANGES`].
    Dirty(usize, Vec<Change>),
    /// A folder at the worktree's place that kwant can't prove is this repo's worktree.
    NotOurs,
    /// The folder is gone (P3 has nothing to clean).
    Missing,
    /// A bot is running a turn in the worktree's thread.
    Working,
    Failed,
}

impl WtError {
    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        use WtError::*;
        match self {
            BadBranch => (StatusCode::BAD_REQUEST, "bad_branch", "That isn't a valid branch name."),
            BadBase => (StatusCode::BAD_REQUEST, "bad_base", "That isn't a valid base."),
            UnknownBranch => (StatusCode::NOT_FOUND, "unknown_branch", "There's no such branch in this repo."),
            UnknownBase => (StatusCode::NOT_FOUND, "unknown_base", "There's no such branch, tag or commit in this repo."),
            NotARepo => (StatusCode::CONFLICT, "not_a_repo", "This team's folder isn't a git checkout kwant can use."),
            BranchExists => (StatusCode::CONFLICT, "branch_exists", "That branch already exists."),
            BranchCheckedOut => (StatusCode::CONFLICT, "branch_checked_out", "That branch is checked out in another worktree."),
            FolderTaken => (StatusCode::CONFLICT, "folder_taken", "That folder already exists. kwant won't write into it."),
            NoGit => (StatusCode::SERVICE_UNAVAILABLE, "no_git", "Needs git 2.31 or newer on the server."),
            Dirty(..) => (StatusCode::CONFLICT, "dirty", "It has changes nobody committed."),
            NotOurs => (StatusCode::CONFLICT, "not_ours", "This folder isn't a worktree kwant made, so kwant won't delete it."),
            Missing => (StatusCode::CONFLICT, "missing", "The folder is gone."),
            Working => (StatusCode::CONFLICT, "working", "A bot is working there. Stop it first."),
            Failed => (StatusCode::INTERNAL_SERVER_ERROR, "git_failed", "Git couldn't do that. Nothing was changed."),
        }
    }

    fn response(&self) -> Response {
        let (status, code, msg) = self.parts();
        let mut body = json!({"error": msg, "code": code});
        if let WtError::Dirty(n, changes) = self {
            body["dirty"] = json!(n);
            body["changes"] = json!(changes);
        }
        (status, Json(body)).into_response()
    }
}

/// One changed file: git's short status letter (`M`, `A`, `D`, `R`, `?` for untracked…) and its path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Change {
    pub status: String,
    pub path: String,
}

// ---------------------------------------------------------------------------------------------
// Git

/// Git in one folder, with the repo's own filter drivers blanked.
pub struct Git {
    dir: PathBuf,
    /// `filter.<name>.<key>=` overrides for each driver the repo's own config sets.
    neutral: Vec<String>,
}

struct Out {
    ok: bool,
    code: Option<i32>,
    stdout: String,
}

impl Git {
    /// Git for `dir`, after reading which filter drivers the repo's own config (scope `local` or
    /// `worktree`, includes too) sets. Drivers from the person's global config (Git LFS) still run.
    pub async fn open(dir: &Path) -> Result<Git, WtError> {
        let mut g = Git { dir: dir.to_path_buf(), neutral: Vec::new() };
        let out = g.run(&["config", "--show-scope", "--get-regexp", r"^filter\."], READ_TIMEOUT, true).await?;
        // Exit 1: no such key
        if !out.ok && out.code != Some(1) {
            return Err(WtError::Failed);
        }
        g.neutral = blanks(&out.stdout)?;
        Ok(g)
    }

    async fn run(&self, args: &[&str], limit: Duration, read: bool) -> Result<Out, WtError> {
        let mut cmd = git::host_git_command("git");
        cmd.arg("-C").arg(&self.dir).arg("--no-pager");
        for c in git::safe_config() {
            cmd.arg("-c").arg(c);
        }
        for c in &self.neutral {
            cmd.arg("-c").arg(c);
        }
        cmd.args(args);
        if read {
            cmd.env("GIT_OPTIONAL_LOCKS", "0");
        }
        cmd.env("GIT_LITERAL_PATHSPECS", "1");
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        let out = match tokio::time::timeout(limit, cmd.output()).await {
            Err(_) => {
                tracing::warn!(args = ?args.first(), "dev: worktree git call timed out");
                return Err(WtError::Failed);
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err(WtError::NoGit),
            Ok(Err(_)) => return Err(WtError::Failed),
            Ok(Ok(o)) => o,
        };
        if !out.status.success() && out.status.code() != Some(1) {
            // Logged for the admin, never returned: it can hold paths and remote URLs
            let err = String::from_utf8_lossy(&out.stderr);
            tracing::info!(args = ?args.first(), stderr = %err.chars().take(300).collect::<String>(), "dev: worktree git call failed");
        }
        Ok(Out { ok: out.status.success(), code: out.status.code(), stdout: String::from_utf8_lossy(&out.stdout).into_owned() })
    }

    async fn read(&self, args: &[&str]) -> Result<Option<String>, WtError> {
        let out = self.run(args, READ_TIMEOUT, true).await?;
        Ok(out.ok.then(|| out.stdout.trim().to_string()))
    }

    /// Whether `refs/heads/<branch>` exists.
    async fn has_branch(&self, branch: &str) -> Result<bool, WtError> {
        Ok(self.read(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await?.is_some())
    }

    /// The short sha of HEAD.
    pub async fn head(&self) -> Option<String> {
        self.read(&["rev-parse", "--short", "HEAD"]).await.ok().flatten().filter(|s| !s.is_empty())
    }

    /// Changed and untracked files: how many, and the first [`LISTED_CHANGES`].
    pub async fn changes(&self) -> Result<(usize, Vec<Change>), WtError> {
        let out = self.run(&["status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignore-submodules=none"], READ_TIMEOUT, true).await?;
        if !out.ok {
            return Err(WtError::Failed);
        }
        Ok(parse_status(&out.stdout))
    }

    /// `(ahead, behind)` of HEAD against `base`.
    pub async fn ahead_behind(&self, base: &str) -> Option<(u64, u64)> {
        if base.starts_with('-') {
            return None;
        }
        let text = self.read(&["rev-list", "--left-right", "--count", &format!("{base}...HEAD")]).await.ok().flatten()?;
        let mut it = text.split_whitespace().map(|n| n.parse::<u64>().ok());
        let behind = it.next()??;
        let ahead = it.next()??;
        Some((ahead, behind))
    }

    /// Linked worktrees git lists (not the main checkout): `(path, branch, head)`.
    pub async fn listed(&self) -> Result<Vec<Listed>, WtError> {
        let out = self.run(&["worktree", "list", "--porcelain", "-z"], READ_TIMEOUT, true).await?;
        if !out.ok {
            return Err(WtError::Failed);
        }
        Ok(parse_list(&out.stdout).into_iter().skip(1).collect())
    }
}

/// The `-c filter.<name>.<key>=` overrides for `git config --show-scope --get-regexp` output
/// (`<scope>\t<key> <value>` lines). Global drivers are left alone. A driver name git can't take
/// back in `-c` (one with `=`) refuses the repo.
fn blanks(listing: &str) -> Result<Vec<String>, WtError> {
    let mut names: Vec<&str> = Vec::new();
    for line in listing.lines() {
        let Some((scope, rest)) = line.split_once('\t') else { continue };
        if scope == "global" {
            continue;
        }
        let key = rest.split_once(' ').map_or(rest, |(k, _)| k);
        let Some((name, _var)) = key.strip_prefix("filter.").and_then(|k| k.rsplit_once('.')) else { continue };
        if name.is_empty() || name.contains('=') || name.contains('\n') {
            return Err(WtError::Failed);
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let mut out = Vec::new();
    for n in names {
        for (k, v) in [("smudge", ""), ("clean", ""), ("process", ""), ("required", "false")] {
            out.push(format!("filter.{n}.{k}={v}"));
        }
    }
    Ok(out)
}

fn parse_status(z: &str) -> (usize, Vec<Change>) {
    let mut n = 0;
    let mut listed = Vec::new();
    let mut parts = z.split('\0').filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        if entry.len() < 4 {
            continue;
        }
        let (xy, path) = entry.split_at(3);
        let xy = xy.trim();
        // A rename or copy has its old path next
        if xy.starts_with('R') || xy.starts_with('C') {
            parts.next();
        }
        n += 1;
        if listed.len() < LISTED_CHANGES {
            let status = if xy == "??" { "?".to_string() } else { xy.chars().find(|c| *c != ' ').unwrap_or('M').to_string() };
            listed.push(Change { status, path: path.to_string() });
        }
    }
    (n, listed)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listed {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    /// Git lists it but its folder is gone.
    pub prunable: bool,
}

fn parse_list(z: &str) -> Vec<Listed> {
    let mut out = Vec::new();
    let mut cur: Option<Listed> = None;
    for field in z.split('\0') {
        if field.is_empty() {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            continue;
        }
        if let Some(p) = field.strip_prefix("worktree ") {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            cur = Some(Listed { path: PathBuf::from(p), ..Default::default() });
        } else if let Some(c) = cur.as_mut() {
            if let Some(h) = field.strip_prefix("HEAD ") {
                c.head = Some(h.chars().take(8).collect());
            } else if let Some(b) = field.strip_prefix("branch ") {
                c.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            } else if field == "prunable" || field.starts_with("prunable ") {
                c.prunable = true;
            }
        }
    }
    if let Some(c) = cur {
        out.push(c);
    }
    out
}

/// The canonical form of a path git printed, for comparing with ours.
fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

// ---------------------------------------------------------------------------------------------
// Create

/// What [`create`] needs.
pub struct CreateIn<'a> {
    /// The team folder (the main checkout).
    pub repo: &'a Path,
    /// The main repo's git dir, canonical.
    pub common: &'a Path,
    /// `<root>/<repo dir>`.
    pub repo_root: &'a Path,
    pub branch: &'a str,
    pub base: Option<&'a str>,
}

/// `git worktree add` into `<repo_root>/<slug(branch)>`: a new branch from `base`, or an existing
/// branch. Returns the folder. Nothing is written when a check fails.
pub async fn create(c: &CreateIn<'_>) -> Result<PathBuf, WtError> {
    if !super::routes::valid_branch(c.branch) {
        return Err(WtError::BadBranch);
    }
    if let Some(b) = c.base {
        if b.trim().is_empty() || b.starts_with('-') || b.chars().any(|ch| ch.is_control() || ch.is_whitespace()) || b.len() > 200 {
            return Err(WtError::BadBase);
        }
    }
    let g = Git::open(c.repo).await?;
    let exists = g.has_branch(c.branch).await?;
    match c.base {
        Some(base) => {
            if exists {
                return Err(WtError::BranchExists);
            }
            if g.read(&["rev-parse", "--verify", "--quiet", &format!("{base}^{{commit}}")]).await?.is_none() {
                return Err(WtError::UnknownBase);
            }
        }
        None => {
            if !exists {
                return Err(WtError::UnknownBranch);
            }
            if g.listed_with_main().await?.iter().any(|l| l.branch.as_deref() == Some(c.branch)) {
                return Err(WtError::BranchCheckedOut);
            }
        }
    }
    let path = c.repo_root.join(slug(c.branch));
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(WtError::FolderTaken);
    }
    let p = path.to_str().ok_or(WtError::Failed)?;
    let out = match c.base {
        Some(base) => g.run(&["worktree", "add", "--quiet", "--no-track", "-b", c.branch, p, base], WRITE_TIMEOUT, false).await?,
        None => g.run(&["worktree", "add", "--quiet", p, c.branch], WRITE_TIMEOUT, false).await?,
    };
    if !out.ok {
        return Err(WtError::Failed);
    }
    // The folder git made must be the one we named, and point back into this repo
    if !points_into(&path, c.common) {
        tracing::warn!("dev: a new worktree's .git doesn't point into its repo");
        return Err(WtError::Failed);
    }
    Ok(path)
}

impl Git {
    /// Every worktree git lists, the main checkout first.
    async fn listed_with_main(&self) -> Result<Vec<Listed>, WtError> {
        let out = self.run(&["worktree", "list", "--porcelain", "-z"], READ_TIMEOUT, true).await?;
        if !out.ok {
            return Err(WtError::Failed);
        }
        Ok(parse_list(&out.stdout))
    }
}

// ---------------------------------------------------------------------------------------------
// Remove

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The folder was a leftover git no longer listed, and kwant deleted it.
    pub leftover_removed: bool,
}

/// Removes kwant's worktree at `path` of the repo `repo` (git dir `common`). The branch is never
/// deleted. A dirty checkout is refused unless `force`. A folder git doesn't list any more is
/// deleted only if its `.git` points into this repo's `worktrees/` ([`points_into`]); git's entry
/// for a missing folder is pruned.
pub async fn remove(repo: &Path, common: &Path, path: &Path, force: bool) -> Result<Removed, WtError> {
    let g = Git::open(repo).await?;
    let listed = g.listed_with_main().await?;
    let known = listed.iter().skip(1).find(|l| same_path(&l.path, path));
    let meta = std::fs::symlink_metadata(path).ok();
    match (meta, known) {
        // Gone: drop git's entry for it, if any
        (None, _) => {
            g.run(&["worktree", "prune"], WRITE_TIMEOUT, false).await?;
            Ok(Removed { leftover_removed: false })
        }
        (Some(m), _) if !m.is_dir() => Err(WtError::NotOurs),
        // A worktree git knows: it must still be this repo's
        (Some(_), Some(_)) => {
            if !points_into(path, common) {
                return Err(WtError::NotOurs);
            }
            let wt = Git::open(path).await?;
            let (n, changes) = wt.changes().await?;
            if n > 0 && !force {
                return Err(WtError::Dirty(n, changes));
            }
            let p = path.to_str().ok_or(WtError::Failed)?;
            // Without force git checks again that it's clean, so a change since our read stops it
            let args: &[&str] = if force { &["worktree", "remove", "--force", p] } else { &["worktree", "remove", p] };
            let out = g.run(args, WRITE_TIMEOUT, false).await?;
            if !out.ok {
                return Err(if !force && wt.changes().await.map(|(n, _)| n > 0).unwrap_or(false) { WtError::Dirty(n.max(1), changes) } else { WtError::Failed });
            }
            Ok(Removed { leftover_removed: false })
        }
        // A folder git doesn't list: only a provable leftover of this repo goes
        (Some(_), None) => {
            if !points_into(path, common) {
                return Err(WtError::NotOurs);
            }
            let p = path.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(p)).await.map_err(|_| WtError::Failed)?.map_err(|_| WtError::Failed)?;
            g.run(&["worktree", "prune"], WRITE_TIMEOUT, false).await?;
            Ok(Removed { leftover_removed: true })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// P3: cleaning build output

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Cleaned {
    /// Folders removed, relative to the worktree, `/`-separated.
    pub removed: Vec<String>,
    pub bytes: u64,
    /// Folders kept, with why: `tracked` (git tracks a file in it) or `not_ignored`.
    pub kept: Vec<Kept>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Kept {
    pub dir: String,
    pub why: &'static str,
}

/// The [`BUILD_DIRS`] folders in `wt`, relative: real folders only (never a symlink), never inside
/// `.git`, and never inside a folder that is itself one (it goes whole).
fn candidates(wt: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![(PathBuf::new(), 0usize)];
    let mut seen = 0;
    while let Some((rel, depth)) = todo.pop() {
        seen += 1;
        if seen > WALK_DIRS {
            break;
        }
        let Ok(rd) = std::fs::read_dir(wt.join(&rel)) else { continue };
        for e in rd.flatten() {
            // The entry's own type: a symlink is never followed
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let name = e.file_name();
            let Some(n) = name.to_str() else { continue };
            if n == ".git" {
                continue;
            }
            let child = rel.join(n);
            if BUILD_DIRS.contains(&n) {
                found.push(child);
            } else if depth + 1 < WALK_DEPTH {
                todo.push((child, depth + 1));
            }
        }
    }
    found.sort();
    found
}

/// Bytes of the files under `dir`, without following symlinks.
fn size_of(dir: &Path) -> u64 {
    let mut total = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
            if m.is_dir() {
                todo.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

/// P3 on the worktree `wt` of the repo with git dir `common`: removes each [`BUILD_DIRS`] folder
/// that is a real folder, ignored by git, and holds no tracked file. Everything else stays.
/// [`WtError::NotOurs`] when `wt`'s `.git` no longer points into that repo (nothing is touched).
pub async fn clean(wt: &Path, common: &Path) -> Result<Cleaned, WtError> {
    match std::fs::symlink_metadata(wt) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(WtError::NotOurs),
        Err(_) => return Err(WtError::Missing),
    }
    if !points_into(wt, common) {
        return Err(WtError::NotOurs);
    }
    let g = Git::open(wt).await?;
    let dir = wt.to_path_buf();
    let found = tokio::task::spawn_blocking(move || candidates(&dir)).await.map_err(|_| WtError::Failed)?;
    let mut done = Cleaned::default();
    for rel in found {
        let text = rel.to_string_lossy().replace('\\', "/");
        let Some(r) = rel.to_str() else { continue };
        // Ignored: check-ignore exits 0 for an ignored path, 1 for one that isn't
        let ignored = g.run(&["check-ignore", "--quiet", "--", r], READ_TIMEOUT, true).await?;
        if !ignored.ok {
            if ignored.code != Some(1) {
                return Err(WtError::Failed);
            }
            done.kept.push(Kept { dir: text, why: "not_ignored" });
            continue;
        }
        let tracked = g.run(&["ls-files", "-z", "--", r], READ_TIMEOUT, true).await?;
        if !tracked.ok {
            return Err(WtError::Failed);
        }
        if !tracked.stdout.is_empty() {
            done.kept.push(Kept { dir: text, why: "tracked" });
            continue;
        }
        let abs = wt.join(&rel);
        let removed = tokio::task::spawn_blocking(move || {
            // Still a real folder right before it goes
            match std::fs::symlink_metadata(&abs) {
                Ok(m) if m.is_dir() => {}
                _ => return None,
            }
            let bytes = size_of(&abs);
            std::fs::remove_dir_all(&abs).ok().map(|_| bytes)
        })
        .await
        .map_err(|_| WtError::Failed)?;
        if let Some(bytes) = removed {
            done.bytes += bytes;
            done.removed.push(text);
        }
    }
    Ok(done)
}

// ---------------------------------------------------------------------------------------------
// The sweep

/// When the worktree's work finished (ms): its task's `finished_at`, else `POST …/done`. A turn
/// that started later means it isn't finished.
async fn finished_at(state: &AppState, w: &WorktreeRow) -> Option<i64> {
    let task = match &w.task_id {
        Some(t) => state.db.task_finished_ms(t).await.ok().flatten(),
        None => None,
    };
    let done = task.or(w.done_at)?;
    match w.last_turn_at {
        Some(t) if t > done => None,
        _ => Some(done),
    }
}

/// Whether P3 is due for `w` at `now_ms`, and for which finish.
fn due(w: &WorktreeRow, finished: Option<i64>, now_ms: i64) -> Option<i64> {
    let f = finished?;
    (!w.keep_build && w.removed_at.is_none() && w.cleaned_for != Some(f) && now_ms - f >= CLEAN_AFTER_MINUTES * 60_000).then_some(f)
}

/// One sweep: cleans each live worktree whose work finished [`CLEAN_AFTER_MINUTES`] ago, once per
/// finish, unless the person kept it or a bot works there.
pub async fn sweep(state: &AppState) {
    let Ok(rows) = state.db.live_worktrees().await else { return };
    let now = Utc::now().timestamp_millis();
    for w in rows {
        let Some(f) = due(&w, finished_at(state, &w).await, now) else { continue };
        if w.thread_id.as_deref().is_some_and(|t| state.turns.has_job(t)) {
            continue;
        }
        if !state.db.dev_mode(&w.space_id).await.ok().flatten().is_some_and(|m| m.on) {
            continue;
        }
        match clean(Path::new(&w.path), Path::new(&w.common_dir)).await {
            Ok(c) => save_cleaned(state, &w.id, &c, Some(f)).await,
            Err(WtError::Missing) => {}
            Err(e) => tracing::info!(worktree = %w.id, error = ?e, "dev: P3 skipped a worktree"),
        }
    }
}

async fn save_cleaned(state: &AppState, id: &str, c: &Cleaned, done_for: Option<i64>) {
    let removed = serde_json::to_string(&c.removed).unwrap_or_else(|_| "[]".into());
    let kept = serde_json::to_string(&c.kept).unwrap_or_else(|_| "[]".into());
    if let Err(e) = state.db.worktree_cleaned(id, &removed, c.bytes as i64, &kept, done_for).await {
        tracing::warn!("dev: saving a P3 result failed: {e}");
    }
}

/// Runs [`sweep`] every [`SWEEP_EVERY`] until shutdown.
pub fn spawn(state: AppState, shutdown: tokio_util::sync::CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(SWEEP_EVERY) => {}
            }
            if state.db.any_dev_mode_on() {
                sweep(&state).await;
            }
        }
    });
}

/// The folder a thread's turns run in: its live worktree, if it has one whose folder is still
/// this repo's worktree. Notes that a turn started (the work isn't done).
pub async fn folder_for_thread(state: &AppState, thread_id: &str) -> Option<PathBuf> {
    let w = state.db.worktree_of_thread(thread_id).await.ok().flatten()?;
    let path = PathBuf::from(&w.path);
    let common = PathBuf::from(&w.common_dir);
    let p = path.clone();
    let ours = tokio::task::spawn_blocking(move || std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) && points_into(&p, &common)).await.unwrap_or(false);
    if !ours {
        tracing::info!(worktree = %w.id, "dev: a thread's worktree is gone or not ours; its turn runs in the team folder");
        return None;
    }
    let _ = state.db.worktree_turn_started(&w.id).await;
    Some(path)
}

// ---------------------------------------------------------------------------------------------
// Routes (person-only: auth refuses bots on /api/spaces/:id/dev… and /api/dev/…)

/// A team whose folder is a git checkout kwant may use: the folder and its main git dir.
struct TeamRepo {
    team_id: String,
    team: String,
    folder: PathBuf,
    common: PathBuf,
    /// The folder's own name as a slug: the `<repo>` part.
    repo: String,
    /// The default branch, for ahead/behind of worktrees made outside kwant.
    default_branch: Option<String>,
}

async fn team_repos(state: &AppState, space: &SpaceId) -> Vec<TeamRepo> {
    let teams = state.db.list_teams(space).await.unwrap_or_default();
    let dir = crate::db::spaces::space_data_dir(&state.config.data_dir, space);
    let data = state.config.data_dir.clone();
    let mut out: Vec<TeamRepo> = Vec::new();
    for t in teams {
        let folder = crate::tools::team_workspace_path(&dir, &t.name, t.workspace_path.as_deref(), t.repo_url.as_deref());
        let (f, d) = (folder.clone(), data.clone());
        let common = tokio::task::spawn_blocking(move || {
            git::local_git_dir(&f, &d).ok()?;
            git::own_git_dirs(&f).ok().map(|(_, common)| common)
        })
        .await
        .ok()
        .flatten();
        let Some(common) = common else { continue };
        // A linked worktree as a team folder: its main repo is what counts
        if out.iter().any(|r| r.common == common) {
            continue;
        }
        let default_branch = super::repo::read_folder(&folder).await.and_then(|(_, b)| b);
        let repo = slug(&folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        out.push(TeamRepo { team_id: t.id.clone(), team: t.name.clone(), folder, common, repo, default_branch });
    }
    out
}

/// The `<repo>` folder for `r` under the root: its name, or `-2`, `-3`… when a different repo
/// (another main git dir) already has that name in this space's records.
fn repo_dir_for(r: &TeamRepo, rows: &[WorktreeRow]) -> String {
    if let Some(w) = rows.iter().find(|w| Path::new(&w.common_dir) == r.common) {
        return w.repo_dir.clone();
    }
    let taken = |name: &str| rows.iter().any(|w| w.repo_dir == name && Path::new(&w.common_dir) != r.common);
    if !taken(&r.repo) {
        return r.repo.clone();
    }
    (2..).map(|i| format!("{}-{i}", r.repo)).find(|n| !taken(n)).unwrap_or_default()
}

fn iso(ms: Option<i64>) -> Value {
    match ms.and_then(|m| Utc.timestamp_millis_opt(m).single()) {
        Some(t) => json!(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        None => Value::Null,
    }
}

/// The worktree's state: `live`, `missing` (folder gone), `leftover` (folder there, git doesn't
/// list it; `ours` says whether kwant may delete it) or `removed`.
fn state_of(w: &WorktreeRow, listed: Option<&[Listed]>) -> (&'static str, bool) {
    if w.removed_at.is_some() {
        return ("removed", false);
    }
    let path = Path::new(&w.path);
    let there = std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    if !there {
        return ("missing", false);
    }
    let ours = points_into(path, Path::new(&w.common_dir));
    match listed {
        Some(l) if !l.iter().any(|x| same_path(&x.path, path)) => ("leftover", ours),
        _ => ("live", ours),
    }
}

/// A worktree as the API shows it (contract §Shapes), with what git says right now.
async fn worktree_json(state: &AppState, w: &WorktreeRow, r: Option<&TeamRepo>, listed: Option<&[Listed]>) -> Value {
    let (st, ours) = state_of(w, listed);
    let (mut head, mut dirty, mut changes, mut ab) = (None, None, Vec::new(), None);
    if st == "live" && ours {
        if let Ok(g) = Git::open(Path::new(&w.path)).await {
            head = g.head().await;
            if let Ok((n, c)) = g.changes().await {
                dirty = Some(n);
                changes = c;
            }
            if let Some(b) = w.base.as_deref().or(r.and_then(|r| r.default_branch.as_deref())) {
                ab = g.ahead_behind(b).await;
            }
        }
    }
    let finished = finished_at(state, w).await;
    let task_title = match &w.task_id {
        Some(t) => state.db.task_title(t).await.ok().flatten(),
        None => None,
    };
    let list = |s: &Option<String>| s.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()).unwrap_or(json!([]));
    json!({
        "id": w.id,
        "team_id": w.team_id,
        "team": r.map(|r| r.team.clone()),
        "repo": w.repo_dir,
        "folder": format!("{}/{}", w.repo_dir, Path::new(&w.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        "path": w.path,
        "branch": w.branch,
        "base": w.base,
        "made_branch": w.made_branch,
        "head": head,
        "dirty": dirty,
        "changes": changes,
        "ahead": ab.map(|(a, _)| a),
        "behind": ab.map(|(_, b)| b),
        "state": st,
        "ours": ours,
        "working": w.thread_id.as_deref().is_some_and(|t| state.turns.has_job(t)),
        "thread_id": w.thread_id,
        "task_id": w.task_id,
        "task_title": task_title,
        "kwant_id": w.kwant_id,
        "created_at": iso(Some(w.created_at)),
        "done_at": iso(finished),
        "clean_at": iso(finished.filter(|f| !w.keep_build && w.cleaned_for != Some(*f)).map(|f| f + CLEAN_AFTER_MINUTES * 60_000)),
        "keep_build": w.keep_build,
        "cleaned_at": iso(w.cleaned_at),
        "cleaned": list(&w.cleaned),
        "cleaned_bytes": w.cleaned_bytes,
        "kept": list(&w.skipped),
        "removed_at": iso(w.removed_at),
    })
}

fn bad(code: &str, msg: &str, status: StatusCode) -> Response {
    super::routes::coded(status, code, msg)
}

/// The space, if Dev mode is on there (else 409 `off`).
async fn guarded(state: &AppState, headers: &HeaderMap, space_id: &str) -> Result<SpaceId, Response> {
    let space = super::routes::guard(state, headers, space_id).await?;
    if state.db.dev_mode(space.as_str()).await.ok().flatten().is_none_or(|m| !m.on) {
        return Err(bad("off", "Dev mode is off", StatusCode::CONFLICT));
    }
    Ok(space)
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    removed: Option<String>,
}

/// `GET /api/spaces/:id/dev/worktrees`
pub async fn list(State(state): State<AppState>, headers: HeaderMap, UrlPath(id): UrlPath<String>, Query(q): Query<ListQuery>) -> Response {
    let space = match guarded(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let with_removed = matches!(q.removed.as_deref(), Some("1" | "true"));
    let rows = match state.db.worktrees_of_space(space.as_str(), with_removed).await {
        Ok(r) => r,
        Err(e) => return super::routes::internal(e),
    };
    let repos = team_repos(&state, &space).await;
    let mut groups = Vec::new();
    for r in &repos {
        let listed = match Git::open(&r.folder).await {
            Ok(g) => g.listed().await.ok(),
            Err(_) => None,
        };
        let mine: Vec<&WorktreeRow> = rows.iter().filter(|w| Path::new(&w.common_dir) == r.common).collect();
        let mut wts = Vec::new();
        for w in &mine {
            wts.push(worktree_json(&state, w, Some(r), listed.as_deref()).await);
        }
        let other: Vec<Value> = listed
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|l| !mine.iter().any(|w| w.removed_at.is_none() && same_path(&l.path, Path::new(&w.path))))
            .map(|l| {
                json!({
                    "folder_name": l.path.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "path": l.path,
                    "branch": l.branch,
                    "head": l.head,
                    "missing": l.prunable,
                })
            })
            .collect();
        groups.push(json!({
            "repo": repo_dir_for(r, &rows),
            "team_id": r.team_id,
            "team": r.team,
            "checkout": r.folder,
            "default_branch": r.default_branch,
            "error": listed.is_none(),
            "worktrees": wts,
            "other": other,
        }));
    }
    Json(json!({"repos": groups, "clean_after_minutes": CLEAN_AFTER_MINUTES, "build_dirs": BUILD_DIRS})).into_response()
}

#[derive(Deserialize)]
pub struct CreateBody {
    team_id: String,
    branch: String,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    kwant_id: Option<String>,
}

/// `POST /api/spaces/:id/dev/worktrees`
pub async fn create_route(State(state): State<AppState>, headers: HeaderMap, UrlPath(id): UrlPath<String>, Json(b): Json<CreateBody>) -> Response {
    let space = match guarded(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let branch = b.branch.trim();
    if !super::routes::valid_branch(branch) {
        return WtError::BadBranch.response();
    }
    let base = b.base.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if b.base.is_some() && base.is_none() {
        return WtError::BadBase.response();
    }
    let repos = team_repos(&state, &space).await;
    let Some(r) = repos.iter().find(|r| r.team_id == b.team_id) else {
        let known = state.db.list_teams(&space).await.unwrap_or_default().iter().any(|t| t.id == b.team_id);
        return if known { WtError::NotARepo.response() } else { bad("unknown_team", "There's no such team in this space.", StatusCode::NOT_FOUND) };
    };
    // The thread, task and bot must be this space's
    if let Some(t) = &b.thread_id {
        match state.db.get_thread(t).await.ok().flatten() {
            Some(th) if state.db.get_kwant(&th.kwant_id).await.ok().flatten().is_some_and(|k| k.space_id == space.as_str()) => {}
            _ => return bad("unknown_thread", "There's no such thread.", StatusCode::NOT_FOUND),
        }
        if state.db.worktree_of_thread(t).await.ok().flatten().is_some() {
            return bad("thread_has_worktree", "That thread already works in a worktree.", StatusCode::CONFLICT);
        }
    }
    if let Some(t) = &b.task_id {
        if !state.db.get_task(t).await.ok().flatten().is_some_and(|task| task.space_id == space.as_str()) {
            return bad("unknown_task", "There's no such task.", StatusCode::NOT_FOUND);
        }
    }
    if let Some(k) = &b.kwant_id {
        if !state.db.get_kwant(k).await.ok().flatten().is_some_and(|k| k.space_id == space.as_str()) {
            return bad("unknown_kwant", "There's no such bot.", StatusCode::NOT_FOUND);
        }
    }
    let rows = state.db.worktrees_of_space(space.as_str(), true).await.unwrap_or_default();
    let repo_dir = repo_dir_for(r, &rows);
    let root = root_of(&state.config.data_dir, &space);
    let repo_root = match tokio::task::spawn_blocking({
        let (root, d) = (root.clone(), repo_dir.clone());
        move || real_dirs(&root, &[d.as_str()])
    })
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return e.response(),
        Err(_) => return WtError::Failed.response(),
    };
    let made = match create(&CreateIn { repo: &r.folder, common: &r.common, repo_root: &repo_root, branch, base }).await {
        Ok(p) => p,
        Err(e) => return e.response(),
    };
    let path = made.to_string_lossy().into_owned();
    let common = r.common.to_string_lossy().into_owned();
    let new = NewWorktree {
        space_id: space.as_str(),
        team_id: &r.team_id,
        repo_dir: &repo_dir,
        common_dir: &common,
        path: &path,
        branch,
        base,
        made_branch: base.is_some(),
        thread_id: b.thread_id.as_deref(),
        task_id: b.task_id.as_deref(),
        kwant_id: b.kwant_id.as_deref(),
    };
    match state.db.insert_worktree(&new).await {
        Ok(Some(w)) => (StatusCode::CREATED, Json(worktree_json(&state, &w, Some(r), None).await)).into_response(),
        Ok(None) => {
            // Two creates at once for one thread: undo ours (the branch stays)
            let _ = remove(&r.folder, &r.common, &made, false).await;
            bad("thread_has_worktree", "That thread already works in a worktree.", StatusCode::CONFLICT)
        }
        Err(e) => super::routes::internal(e),
    }
}

/// The row behind `/api/dev/worktrees/:id`, its space (Dev mode on) and its team's repo.
async fn row_for(state: &AppState, headers: &HeaderMap, id: &str) -> Result<(WorktreeRow, SpaceId), Response> {
    let w = match state.db.worktree(id.trim()).await {
        Ok(Some(w)) => w,
        Ok(None) => return Err(bad("unknown_worktree", "There's no such worktree.", StatusCode::NOT_FOUND)),
        Err(e) => return Err(super::routes::internal(e)),
    };
    let space = guarded(state, headers, &w.space_id).await?;
    Ok((w, space))
}

#[derive(Deserialize, Default)]
pub struct RemoveQuery {
    #[serde(default)]
    force: Option<String>,
}

/// `DELETE /api/dev/worktrees/:id?force=true`
pub async fn remove_route(State(state): State<AppState>, headers: HeaderMap, UrlPath(id): UrlPath<String>, Query(q): Query<RemoveQuery>) -> Response {
    let (w, _space) = match row_for(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if w.removed_at.is_some() {
        return bad("unknown_worktree", "That worktree was removed already.", StatusCode::NOT_FOUND);
    }
    if w.thread_id.as_deref().is_some_and(|t| state.turns.has_job(t)) {
        return WtError::Working.response();
    }
    let force = matches!(q.force.as_deref(), Some("1" | "true"));
    // The main checkout is where git runs; it's the folder the common dir belongs to
    let common = PathBuf::from(&w.common_dir);
    let Some(main) = common.parent().map(Path::to_path_buf) else { return WtError::Failed.response() };
    match remove(&main, &common, Path::new(&w.path), force).await {
        Ok(done) => {
            if let Err(e) = state.db.worktree_removed(&w.id).await {
                return super::routes::internal(e);
            }
            Json(json!({"removed": true, "branch_kept": w.branch, "leftover_removed": done.leftover_removed})).into_response()
        }
        Err(e) => e.response(),
    }
}

/// `POST /api/dev/worktrees/:id/done`
pub async fn done_route(State(state): State<AppState>, headers: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    let (w, _) = match row_for(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Err(e) = state.db.worktree_done(&w.id).await {
        return super::routes::internal(e);
    }
    one(&state, &w.id).await
}

#[derive(Deserialize)]
pub struct KeepBody {
    keep: bool,
}

/// `POST /api/dev/worktrees/:id/keep` `{keep}`: "Keep it" / "Clean on schedule".
pub async fn keep_route(State(state): State<AppState>, headers: HeaderMap, UrlPath(id): UrlPath<String>, Json(b): Json<KeepBody>) -> Response {
    let (w, _) = match row_for(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if let Err(e) = state.db.worktree_keep_build(&w.id, b.keep).await {
        return super::routes::internal(e);
    }
    one(&state, &w.id).await
}

/// `POST /api/dev/worktrees/:id/clean`: P3 now, same rules.
pub async fn clean_route(State(state): State<AppState>, headers: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    let (w, _) = match row_for(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if w.removed_at.is_some() {
        return bad("unknown_worktree", "That worktree was removed already.", StatusCode::NOT_FOUND);
    }
    if w.thread_id.as_deref().is_some_and(|t| state.turns.has_job(t)) {
        return WtError::Working.response();
    }
    match clean(Path::new(&w.path), Path::new(&w.common_dir)).await {
        Ok(c) => {
            save_cleaned(&state, &w.id, &c, finished_at(&state, &w).await).await;
            one(&state, &w.id).await
        }
        Err(e) => e.response(),
    }
}

async fn one(state: &AppState, id: &str) -> Response {
    let Ok(Some(w)) = state.db.worktree(id).await else { return bad("unknown_worktree", "There's no such worktree.", StatusCode::NOT_FOUND) };
    let space = SpaceId::new(&w.space_id);
    let repos = team_repos(state, &space).await;
    let r = repos.iter().find(|r| Path::new(&w.common_dir) == r.common);
    let listed = match r {
        Some(r) => match Git::open(&r.folder).await {
            Ok(g) => g.listed().await.ok(),
            Err(_) => None,
        },
        None => None,
    };
    Json(worktree_json(state, &w, r, listed.as_deref()).await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Demo Person", "-c", "user.email=demo@example.com", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// A repo in `tmp/web` with one commit on main, `.gitignore` ignoring the build dirs, and a
    /// tracked `vendor/dist/keep.js` under an ignored `dist`.
    fn repo(tmp: &Path) -> (PathBuf, PathBuf) {
        let w = tmp.join("web");
        std::fs::create_dir_all(&w).unwrap();
        sh(&w, &["init", "--quiet"]);
        write(&w, ".gitignore", "node_modules/\ndist/\ntarget/\n.next/\n");
        write(&w, "src/app.js", "console.log(1)\n");
        write(&w, "vendor/dist/keep.js", "tracked\n");
        sh(&w, &["add", "."]);
        sh(&w, &["add", "-f", "vendor/dist/keep.js"]);
        sh(&w, &["commit", "--quiet", "-m", "first"]);
        let common = std::fs::canonicalize(w.join(".git")).unwrap();
        (w, common)
    }

    async fn make(tmp: &Path, w: &Path, common: &Path, branch: &str, base: Option<&str>) -> Result<PathBuf, WtError> {
        let root = real_dirs(&tmp.join("worktrees"), &["web"]).unwrap();
        create(&CreateIn { repo: w, common, repo_root: &root, branch, base }).await
    }

    #[test]
    fn branch_slugs_are_plain_folder_names() {
        assert_eq!(slug("lin/H1 Worktrees"), "lin-h1-worktrees");
        assert_eq!(slug("pixel/inbox-empty-state"), "pixel-inbox-empty-state");
        assert_eq!(slug("../../etc"), "etc");
        assert_eq!(slug(".hidden"), "hidden");
        assert_eq!(slug("///"), "worktree");
        assert_eq!(slug("feat/ü-ß"), "feat");
        assert_eq!(slug(&"a".repeat(80)).len(), 60);
        assert!(!slug(&format!("{}-b", "a".repeat(59))).ends_with('-'));
    }

    #[test]
    fn only_the_repos_own_filter_drivers_are_blanked() {
        let listing = "global\tfilter.lfs.smudge git-lfs smudge -- %f\nlocal\tfilter.evil.smudge sh -c 'touch x'\nlocal\tfilter.a.b.process run\nworktree\tfilter.evil.clean y\n";
        let b = blanks(listing).unwrap();
        assert!(b.contains(&"filter.evil.smudge=".to_string()));
        assert!(b.contains(&"filter.evil.required=false".to_string()));
        assert!(b.contains(&"filter.a.b.process=".to_string()), "a driver name with a dot");
        assert!(!b.iter().any(|x| x.contains("lfs")), "the person's global LFS still runs");
        assert_eq!(b.len(), 8, "two drivers, four keys each: {b:?}");
        assert_eq!(blanks("local\tfilter.x=y.smudge z\n"), Err(WtError::Failed));
        assert!(blanks("").unwrap().is_empty());
    }

    #[test]
    fn status_and_list_parse() {
        let (n, c) = parse_status(" M src/a.js\0?? new.txt\0R  b.js\0old.js\0D  gone.js\0");
        assert_eq!(n, 4);
        assert_eq!(c.iter().map(|c| (c.status.as_str(), c.path.as_str())).collect::<Vec<_>>(), [("M", "src/a.js"), ("?", "new.txt"), ("R", "b.js"), ("D", "gone.js")]);
        let l = parse_list("worktree /r/web\0HEAD 2c931968aaaa\0branch refs/heads/main\0\0worktree /wt/x\0HEAD 1111111122\0branch refs/heads/lin/x\0prunable gitdir file points to non-existent location\0\0");
        assert_eq!(l.len(), 2);
        assert_eq!((l[1].branch.as_deref(), l[1].head.as_deref(), l[1].prunable), (Some("lin/x"), Some("11111111"), true));
    }

    /// The `.git` file proof: a worktree of this repo points into its `worktrees/`; another
    /// repo's worktree, a `..` escape, a symlinked `.git` and a plain folder don't.
    #[tokio::test]
    async fn only_this_repos_worktree_points_into_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, common) = repo(tmp.path());
        let wt = make(tmp.path(), &w, &common, "lin/a", Some("main")).await.unwrap();
        assert!(points_into(&wt, &common));
        let other = tmp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        sh(&other, &["init", "--quiet"]);
        assert!(!points_into(&wt, &std::fs::canonicalize(other.join(".git")).unwrap()));
        let fake = tmp.path().join("fake");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::write(fake.join(".git"), format!("gitdir: {}/worktrees/../../x\n", common.display())).unwrap();
        assert!(!points_into(&fake, &common), "a .. that leaves worktrees/");
        std::fs::write(fake.join(".git"), format!("gitdir: {}/worktrees/x\n", common.display())).unwrap();
        assert!(points_into(&fake, &common), "text alone: a leftover after prune");
        std::fs::remove_file(fake.join(".git")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(wt.join(".git"), fake.join(".git")).unwrap();
        assert!(!points_into(&fake, &common), "a symlinked .git");
        assert!(!points_into(&w, &common), "the main checkout has a .git folder");
    }

    #[tokio::test]
    async fn create_makes_a_new_or_existing_branch_and_refuses_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, common) = repo(tmp.path());
        let a = make(tmp.path(), &w, &common, "lin/H1 a", None).await;
        assert_eq!(a, Err(WtError::BadBranch));
        let a = make(tmp.path(), &w, &common, "lin/a", Some("main")).await.unwrap();
        assert_eq!(a, tmp.path().join("worktrees/web/lin-a"));
        assert_eq!(sh(&a, &["rev-parse", "--abbrev-ref", "HEAD"]), "lin/a");
        assert_eq!(make(tmp.path(), &w, &common, "lin/a", Some("main")).await, Err(WtError::BranchExists));
        assert_eq!(make(tmp.path(), &w, &common, "lin/b", Some("nope")).await, Err(WtError::UnknownBase));
        assert_eq!(make(tmp.path(), &w, &common, "lin/b", Some("--orphan")).await, Err(WtError::BadBase));
        assert_eq!(make(tmp.path(), &w, &common, "lin/b", None).await, Err(WtError::UnknownBranch));
        assert_eq!(make(tmp.path(), &w, &common, "lin/a", None).await, Err(WtError::BranchCheckedOut));
        sh(&w, &["branch", "lin/c"]);
        let c = make(tmp.path(), &w, &common, "lin/c", None).await.unwrap();
        assert_eq!(sh(&c, &["rev-parse", "--abbrev-ref", "HEAD"]), "lin/c");
        // `lin-c` is taken now: another branch with the same slug is refused, not merged into it
        sh(&w, &["branch", "lin-c"]);
        assert_eq!(make(tmp.path(), &w, &common, "lin-c", None).await, Err(WtError::FolderTaken));
    }

    /// A planted `filter.<x>.smudge` in the repo's own config never runs on create or on a read.
    #[tokio::test]
    async fn a_planted_filter_driver_never_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, common) = repo(tmp.path());
        let marker = tmp.path().join("PWNED");
        write(&w, ".gitattributes", "*.txt filter=evil\n");
        write(&w, "a.txt", "hi\n");
        sh(&w, &["add", "."]);
        sh(&w, &["commit", "--quiet", "-m", "attrs"]);
        let cmd = format!("touch {}; cat", marker.display());
        sh(&w, &["config", "filter.evil.smudge", &cmd]);
        sh(&w, &["config", "filter.evil.clean", &cmd]);
        let wt = make(tmp.path(), &w, &common, "lin/f", Some("main")).await.unwrap();
        write(&wt, "a.txt", "changed\n");
        let (n, _) = Git::open(&wt).await.unwrap().changes().await.unwrap();
        assert_eq!(n, 1);
        assert!(!marker.exists(), "the repo's own filter ran");
    }

    #[tokio::test]
    async fn remove_refuses_a_dirty_checkout_unless_forced_and_keeps_the_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, common) = repo(tmp.path());
        let wt = make(tmp.path(), &w, &common, "lin/r", Some("main")).await.unwrap();
        write(&wt, "src/app.js", "changed\n");
        write(&wt, "notes.txt", "untracked\n");
        match remove(&w, &common, &wt, false).await {
            Err(WtError::Dirty(2, changes)) => assert_eq!(changes.len(), 2),
            other => panic!("{other:?}"),
        }
        assert!(wt.join("notes.txt").exists(), "a refused remove touches nothing");
        assert_eq!(remove(&w, &common, &wt, true).await, Ok(Removed { leftover_removed: false }));
        assert!(!wt.exists());
        assert_eq!(sh(&w, &["branch", "--list", "lin/r"]), "lin/r", "the branch stays");
        // A clean one goes without force; the branch stays too
        let wt = make(tmp.path(), &w, &common, "lin/s", Some("main")).await.unwrap();
        assert!(remove(&w, &common, &wt, false).await.is_ok());
        assert!(!wt.exists());
        assert_eq!(sh(&w, &["branch", "--list", "lin/s"]), "lin/s");
    }

    #[tokio::test]
    async fn a_leftover_folder_goes_only_if_it_was_this_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, common) = repo(tmp.path());
        // A folder git forgot (its admin entry pruned): its .git still points into this repo
        let wt = make(tmp.path(), &w, &common, "lin/l", Some("main")).await.unwrap();
        std::fs::remove_dir_all(common.join("worktrees")).unwrap();
        assert_eq!(remove(&w, &common, &wt, false).await, Ok(Removed { leftover_removed: true }));
        assert!(!wt.exists());
        // Someone else's folder at the place: kept
        let theirs = tmp.path().join("worktrees/web/lin-m");
        write(&theirs, "precious.txt", "keep\n");
        assert_eq!(remove(&w, &common, &theirs, true).await, Err(WtError::NotOurs));
        assert!(theirs.join("precious.txt").exists());
        // A worktree git lists whose .git was swapped for another repo's: kept
        let wt = make(tmp.path(), &w, &common, "lin/n", Some("main")).await.unwrap();
        std::fs::write(wt.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
        assert_eq!(remove(&w, &common, &wt, true).await, Err(WtError::NotOurs));
        assert!(wt.exists());
        // A missing folder: git's entry is pruned, nothing else
        let gone = make(tmp.path(), &w, &common, "lin/o", Some("main")).await.unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(remove(&w, &common, &gone, false).await.is_ok());
        assert!(!sh(&w, &["worktree", "list"]).contains("lin-o"));
        assert_eq!(sh(&w, &["branch", "--list", "lin/o"]), "lin/o");
    }

    /// P3 removes only listed folders that git ignores and that hold no tracked file; a symlink
    /// named like one, a tracked one, an unignored one and everything else stay.
    #[tokio::test]
    async fn p3_removes_only_ignored_untracked_build_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, common) = repo(tmp.path());
        let wt = make(tmp.path(), &w, &common, "lin/p", Some("main")).await.unwrap();
        write(&wt, "node_modules/x/index.js", "0123456789");
        write(&wt, "web/dist/app.js", "01234");
        write(&wt, "vendor/dist/extra.js", "x"); // dist with a tracked file
        write(&wt, ".turbo/cache", "x"); // not ignored
        write(&wt, "src/target.rs", "fn main() {}"); // a file named like nothing
        let outside = tmp.path().join("outside");
        write(&outside, "keep.txt", "x");
        #[cfg(unix)]
        {
            std::fs::create_dir_all(wt.join("pkg")).unwrap();
            std::os::unix::fs::symlink(&outside, wt.join("pkg/target")).unwrap();
            write(&wt, ".gitignore", "node_modules/\ndist/\ntarget\n.next/\n");
        }
        let c = clean(&wt, &common).await.unwrap();
        assert_eq!(c.removed, ["node_modules", "web/dist"]);
        assert_eq!(c.bytes, 15);
        assert!(c.kept.contains(&Kept { dir: "vendor/dist".into(), why: "tracked" }), "{:?}", c.kept);
        assert!(c.kept.contains(&Kept { dir: ".turbo".into(), why: "not_ignored" }), "{:?}", c.kept);
        assert!(wt.join("vendor/dist/keep.js").exists() && wt.join("vendor/dist/extra.js").exists());
        assert!(wt.join(".turbo/cache").exists() && wt.join("src/target.rs").exists() && wt.join("src/app.js").exists());
        assert!(outside.join("keep.txt").exists(), "a symlink is never followed");
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(wt.join("pkg/target")).unwrap().file_type().is_symlink());
        // A worktree whose .git no longer points into the repo is skipped whole
        write(&wt, "node_modules/y.js", "x");
        std::fs::write(wt.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
        assert_eq!(clean(&wt, &common).await, Err(WtError::NotOurs));
        assert!(wt.join("node_modules/y.js").exists());
        assert_eq!(clean(&tmp.path().join("nope"), &common).await, Err(WtError::Missing));
    }

    fn row(done_at: Option<i64>) -> WorktreeRow {
        WorktreeRow {
            id: "wt_1".into(),
            space_id: "s".into(),
            team_id: "t".into(),
            repo_dir: "web".into(),
            common_dir: "/r/.git".into(),
            path: "/wt/web/x".into(),
            branch: "x".into(),
            base: None,
            made_branch: false,
            thread_id: None,
            task_id: None,
            kwant_id: None,
            created_at: 0,
            done_at,
            last_turn_at: None,
            keep_build: false,
            cleaned_at: None,
            cleaned_for: None,
            cleaned: None,
            cleaned_bytes: None,
            skipped: None,
            removed_at: None,
        }
    }

    #[test]
    fn p3_is_due_an_hour_after_the_finish_once_per_finish() {
        let hour = 3_600_000;
        let w = row(Some(1_000));
        assert_eq!(due(&w, Some(1_000), 1_000 + hour - 1), None, "not before the hour");
        assert_eq!(due(&w, Some(1_000), 1_000 + hour), Some(1_000));
        assert_eq!(due(&w, None, 10 * hour), None, "not finished");
        let cleaned = WorktreeRow { cleaned_for: Some(1_000), ..w.clone() };
        assert_eq!(due(&cleaned, Some(1_000), 10 * hour), None, "once per finish");
        assert_eq!(due(&cleaned, Some(5 * hour), 6 * hour), Some(5 * hour), "a later finish cleans again");
        let kept = WorktreeRow { keep_build: true, ..w.clone() };
        assert_eq!(due(&kept, Some(1_000), 10 * hour), None, "Keep it");
        let removed = WorktreeRow { removed_at: Some(2), ..w };
        assert_eq!(due(&removed, Some(1_000), 10 * hour), None);
    }

    #[test]
    fn a_second_repo_with_the_same_folder_name_gets_a_suffix() {
        let r = |common: &str| TeamRepo {
            team_id: "t".into(),
            team: "T".into(),
            folder: PathBuf::from("/x/web"),
            common: PathBuf::from(common),
            repo: "web".into(),
            default_branch: None,
        };
        let rows = vec![row(None)];
        assert_eq!(repo_dir_for(&r("/r/.git"), &rows), "web", "the same repo keeps its folder");
        assert_eq!(repo_dir_for(&r("/other/.git"), &rows), "web-2");
        assert_eq!(repo_dir_for(&r("/other/.git"), &[]), "web");
    }

    #[test]
    fn real_dirs_refuse_a_planted_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("worktrees");
        assert!(real_dirs(&root, &["web"]).is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path(), root.join("evil")).unwrap();
            assert_eq!(real_dirs(&root, &["evil"]), Err(WtError::FolderTaken));
        }
    }
}
