//! Mission Control Phase B2: the Time Machine. Checkpoints of a task's folder, restore of the
//! paths the task's own runs changed, and the side-effect ledger it reports as "not undone".
//!
//! The store is `<data_dir>/checkpoints/` (0700): `blobs/<sha256>` (deduplicated across
//! checkpoints) and `<task_id>/<checkpoint_id>.json` manifests, plus `<task_id>/runs.jsonl`,
//! the paths each run of the task changed (the task-owned paths). Nothing under it, no path
//! and no file content ever reaches the API. Contract: docs/mission-control.md §12.

use super::routes::{bad, internal, load_task, not_found};
use super::{emit_task, int_setting, task_folder, task_json, transition, Actor};
use crate::db::checkpoints::{new_id, Checkpoint, NewCheckpoint, ToolCall};
use crate::db::missions::{Claim, Task, TaskChange};
use crate::server::gitfile::{self, plain_ref};
use crate::server::AppState;
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use crate::fsx::UnixMode;
use std::path::{Component, Path as FsPath, PathBuf};

pub const SETTING: &str = "checkpoints";
pub const MAX_MB_SETTING: &str = "checkpoint_max_mb";
pub const KEEP_SETTING: &str = "checkpoint_keep";
pub const DEFAULT_MAX_MB: i64 = 200;
pub const MAX_MB_RANGE: (i64, i64) = (10, 2000);
pub const DEFAULT_KEEP: i64 = 5;
pub const KEEP_RANGE: (i64, i64) = (1, 20);
/// A bigger file is never snapshotted.
pub const FILE_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// All checkpoints together; past it the oldest unprotected ones are pruned.
pub const TOTAL_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Files per checkpoint.
pub const MAX_FILES: usize = 20_000;
/// Folders below the task folder a snapshot goes into; deeper ones are skipped. Each folder on
/// the way down stays open while it's walked, so this also bounds the descriptors one walk holds.
pub const MAX_DEPTH: usize = 64;
/// Entries (files, folders, anything) a snapshot looks at before it stops walking: a folder with
/// millions of entries costs kwant this much listing and memory, not more.
pub const MAX_ENTRIES: usize = MAX_FILES * 4;
/// How a manifest's skip says the walk stopped early (after the number).
const WALK_STOPPED: &str = "entry limit";
/// Folders never snapshotted (and never restored).
pub const SKIP_DIRS: [&str; 4] = [".git", "node_modules", "target", ".venv"];
/// What restore says about external side effects, always.
pub const NOT_UNDONE: &str = "kwant can't undo these. Check them yourself.";
/// A conflict no task's own calls account for (D11).
pub const OUTSIDE: &str = "changed outside this task";

/// One snapshot, prune, restore or restore plan at a time: blob GC never races a snapshot writing
/// blobs, and a plan never sees a checkpoint without its run record.
static STORE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn enabled(state: &AppState) -> bool {
    !matches!(state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim), Some("false" | "0" | "off"))
}

pub async fn max_mb(state: &AppState) -> i64 {
    int_setting(state, MAX_MB_SETTING, MAX_MB_RANGE, DEFAULT_MAX_MB).await
}

pub async fn keep(state: &AppState) -> i64 {
    int_setting(state, KEEP_SETTING, KEEP_RANGE, DEFAULT_KEEP).await
}

pub fn settings_error(max_mb: Option<i64>, keep: Option<i64>) -> Option<String> {
    if let Some(n) = max_mb.filter(|n| !(MAX_MB_RANGE.0..=MAX_MB_RANGE.1).contains(n)) {
        return Some(format!("checkpoint_max_mb must be {}–{} (got {n})", MAX_MB_RANGE.0, MAX_MB_RANGE.1));
    }
    if let Some(n) = keep.filter(|n| !(KEEP_RANGE.0..=KEEP_RANGE.1).contains(n)) {
        return Some(format!("checkpoint_keep must be {}–{} (got {n})", KEEP_RANGE.0, KEEP_RANGE.1));
    }
    None
}

fn root(state: &AppState) -> PathBuf {
    state.config.data_dir.join("checkpoints")
}

// The store (blocking; callers hold STORE and run these in spawn_blocking)

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub files: Vec<Entry>,
    #[serde(default)]
    pub skipped: Vec<Skip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skip {
    pub path: String,
    pub why: String,
}

/// One run of a task: the paths that differ between its `before_run` and `after_run`.
/// In a shared folder `paths` also holds what other tasks changed meanwhile; `touched` is
/// only what this run's own logged file calls named (D11: the proof that names a task).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Run {
    claim_id: Option<String>,
    after_run_id: String,
    at: chrono::DateTime<chrono::Utc>,
    folder: String,
    paths: Vec<String>,
    #[serde(default)]
    touched: Vec<String>,
}

fn private_dir(dir: &FsPath) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    crate::fsx::set_mode(dir, 0o700)
}

fn hex(bytes: &[u8]) -> String {
    super::checks::hex(bytes)
}

/// `rel` is a plain relative path: no `..`, not absolute, no empty or `.` parts.
pub fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains('\0')
        && !rel.ends_with('/')
        && FsPath::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

/// Whether `folder/rel` or any folder on the way is a symlink (never followed by restore).
fn through_symlink(folder: &FsPath, rel: &str) -> bool {
    let mut p = folder.to_path_buf();
    for c in FsPath::new(rel).components() {
        p.push(c);
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// What the folder holds at `rel` now: `Ok(Some(sha))`, `Ok(None)` when absent, `Err(why)`.
/// Opened from the folder's descriptor, never through a symlink (S-8), the folder's own path
/// included (QA F-3).
fn current(folder: &FsPath, rel: &str) -> Result<Option<String>, String> {
    if through_symlink(folder, rel) {
        return Err("symlink, not followed".into());
    }
    let opened = crate::sandbox::safe_fs::Dir::open_strict(folder).and_then(|d| d.open_file(FsPath::new(rel)));
    let mut f = match opened {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        // A symlink that appeared since the check above
        Err(e) if symlink_refused(&e) => return Err("symlink, not followed".into()),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => return Err("not a regular file".into()),
        Err(_) => return Err("can't be read".into()),
    };
    // At most FILE_MAX_BYTES + 1 bytes, however large the file is or grows: a longer file's hash
    // (of that much) still never equals a stored copy's, which is at most FILE_MAX_BYTES long
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    std::io::copy(&mut (&mut f).take(FILE_MAX_BYTES + 1), &mut h).map_err(|_| "can't be read".to_string())?;
    Ok(Some(hex(&sha2::Digest::finalize(h))))
}

/// Whether opening failed because a symlink was met on the way. Mac and Linux: `O_NOFOLLOW` gives
/// `ELOOP` on the last part and `O_DIRECTORY` gives `ENOTDIR` on a folder swapped for a symlink.
#[cfg(unix)]
fn symlink_refused(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR))
}

/// Off unix [`crate::sandbox::safe_fs::Dir`] checks by path and refuses a symlink or reparse
/// point as "not a regular file" (`InvalidInput`, the arm after this one) or "a symlink on the
/// way" for the folder itself, so there is no separate symlink error to tell apart here.
#[cfg(not(unix))]
fn symlink_refused(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::InvalidInput && e.to_string() == "a symlink on the way"
}

/// Streams `input` (already opened, never by path: S-8) into the blob store, returning its sha256
/// and how many bytes it really held. The listing's size is only a hint: the file can grow after
/// it, so at most `cap` bytes are read and a longer file is refused
/// ([`crate::sandbox::safe_fs::is_too_large`]), with nothing stored.
fn store_blob(blobs: &FsPath, input: std::fs::File, cap: u64) -> std::io::Result<(String, u64)> {
    if input.metadata()?.len() > cap {
        return Err(crate::sandbox::safe_fs::too_large());
    }
    let tmp = blobs.join(format!(".tmp-{}", new_id()));
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).unix_mode(0o600).open(&tmp)?;
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut input = input.take(cap + 1);
    let mut total: u64 = 0;
    let res = (|| -> std::io::Result<()> {
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > cap {
                return Err(crate::sandbox::safe_fs::too_large());
            }
            sha2::Digest::update(&mut h, &buf[..n]);
            out.write_all(&buf[..n])?;
        }
        out.flush()
    })();
    drop(out);
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let sha = hex(&sha2::Digest::finalize(h));
    let dest = blobs.join(&sha);
    if dest.exists() {
        let _ = std::fs::remove_file(&tmp);
    } else {
        std::fs::rename(&tmp, &dest)?;
    }
    Ok((sha, total))
}

/// [`git_head_in`] of `folder`, opened with no symlink followed.
#[cfg(test)]
fn git_head(folder: &FsPath) -> Option<String> {
    git_head_in(&crate::sandbox::safe_fs::Dir::open_strict(folder).ok()?)
}

/// The commit the folder's git checkout is on, if it is one (read-only info).
///
/// The folder is written by the task's bots, so everything under `.git` is untrusted: it never
/// follows a symlink (at any component), never opens anything but a regular file with one link (a
/// FIFO can't block it, a hard link can't name a host file), reads at most [`gitfile::REF_MAX`] /
/// [`gitfile::PACKED_MAX`] bytes, only follows a `ref:` that is a plain `refs/...` name, and only returns 40 or 64 hex characters. Anything else,
/// including a `.git` file (a `gitdir:` worktree pointer), is `None`. Read from the folder's
/// descriptor, the one its snapshot walked (QA F-3).
fn git_head_in(folder: &crate::sandbox::safe_fs::Dir) -> Option<String> {
    // `.git` is opened once, without following it (a symlink or a `.git` file fails), and HEAD,
    // the ref and packed-refs are all read from that one descriptor
    let git = folder.open_dir(FsPath::new(".git")).ok()?;
    let head = gitfile::read(&git, "HEAD", gitfile::REF_MAX)?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else {
        return commit_id(head);
    };
    let r = r.trim();
    if !plain_ref(r) {
        return None;
    }
    if let Some(s) = gitfile::read(&git, r, gitfile::REF_MAX) {
        return commit_id(s.trim());
    }
    let packed = gitfile::read(&git, "packed-refs", gitfile::PACKED_MAX)?;
    packed
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
        .find_map(|l| l.split_once(' ').filter(|(_, name)| name.trim_end() == r).map(|(sha, _)| sha))
        .and_then(commit_id)
}

/// A full SHA-1 (40) or SHA-256 (64) commit id, lowercased.
fn commit_id(s: &str) -> Option<String> {
    ((s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

struct Taken {
    manifest: Manifest,
    bytes: u64,
    git_head: Option<String>,
}

/// The task folder as a checkpoint records it (QA F-4): the folders above it resolved now, its own
/// name kept as it is. A checkpoint's restore writes back only into the folder it recorded, opened
/// with [`Dir::open_strict`](crate::sandbox::safe_fs::Dir::open_strict), so a symlink the person
/// made above the folder (`~/Projects -> /mnt/ssd/Projects`, a linked home or data folder) works
/// as before, while any part swapped for a symlink after the checkpoint is refused (QA F-3).
/// Content and place stay bound: what a checkpoint read is only ever written back where it was
/// read. Never resolved at restore time. The folder itself, if a symlink, stays refused by
/// [`snapshot`] as before. A parent that can't be resolved (missing) is kept as typed.
#[cfg(unix)]
fn resolve_above(folder: &FsPath) -> PathBuf {
    match (folder.parent(), folder.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => parent.canonicalize().map(|p| p.join(name)).unwrap_or_else(|_| folder.to_path_buf()),
        _ => folder.to_path_buf(),
    }
}

/// Off unix the folder is kept as typed: Windows' `canonicalize` gives `\\?\` paths, and
/// checkpoints are new there in 0.7.7 (nothing to stay compatible with).
#[cfg(not(unix))]
fn resolve_above(folder: &FsPath) -> PathBuf {
    folder.to_path_buf()
}

/// Walks `folder` (never following symlinks) and stores each file's content as a blob.
/// A missing folder is an empty snapshot; an unreadable one is an error.
fn snapshot(blobs: &FsPath, folder: &FsPath, max_bytes: u64) -> Result<Taken, String> {
    match std::fs::symlink_metadata(folder) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            private_dir(blobs).map_err(|e| format!("the checkpoint store can't be written: {e}"))?;
            return Ok(Taken { manifest: Manifest::default(), bytes: 0, git_head: None });
        }
        Err(e) => return Err(format!("the folder can't be read: {e}")),
        Ok(meta) if !meta.is_dir() => return Err("the task's folder isn't a folder".into()),
        Ok(_) => {}
    }
    // No part of the folder's own path is followed if it is a symlink (QA F-3): a task folder
    // nested in a bot's folder can be swapped by that bot
    let top = crate::sandbox::safe_fs::Dir::open_strict(folder).map_err(|e| format!("the folder can't be read: {e}"))?;
    snapshot_in(blobs, top, max_bytes)
}

/// [`snapshot`] of a folder already open: a restore snapshots and writes through the same
/// descriptor, so a folder swapped in between is never the one written (QA F-3).
fn snapshot_in(blobs: &FsPath, top: crate::sandbox::safe_fs::Dir, max_bytes: u64) -> Result<Taken, String> {
    snapshot_within(blobs, top, max_bytes, MAX_DEPTH, MAX_ENTRIES)
}

/// [`snapshot_in`] with its walk limits given (tests use small ones).
fn snapshot_within(blobs: &FsPath, top: crate::sandbox::safe_fs::Dir, max_bytes: u64, max_depth: usize, max_entries: usize) -> Result<Taken, String> {
    private_dir(blobs).map_err(|e| format!("the checkpoint store can't be written: {e}"))?;
    let mut m = Manifest::default();
    // Every folder is reached from its parent's descriptor and every file opened from the
    // folder's with `openat(…, O_NOFOLLOW)` (S-8): a part swapped for a symlink mid-walk is
    // skipped, never followed. Candidates keep their exact relative path to open later.
    use crate::sandbox::safe_fs::{Dir, Kind};
    let top = std::rc::Rc::new(top);
    let mut candidates: Vec<(String, PathBuf, u64, u32)> = Vec::new();
    // Each stack item: the parent's descriptor, the folder's name in it, its relative path and its
    // depth (the task folder is 0). Depth-first, so at most MAX_DEPTH folders are open at once.
    type Pending = (std::rc::Rc<Dir>, Option<std::ffi::OsString>, String, PathBuf, usize);
    let mut stack: Vec<Pending> = vec![(top.clone(), None, String::new(), PathBuf::new(), 0)];
    let mut first = true;
    let mut looked_at: usize = 0;
    while let Some((parent, name, prefix, at, depth)) = stack.pop() {
        // One more than what's left, so a folder that fills the budget exactly isn't called cut
        let room = max_entries - looked_at;
        let listed = match name {
            None => Ok(parent),
            Some(n) => parent.open_dir(FsPath::new(&n)).map(std::rc::Rc::new),
        }
        .and_then(|d| d.list_at_most(room.saturating_add(1)).map(|l| (d, l)));
        let (dir, (mut entries, mut cut)) = match listed {
            Ok(x) => x,
            Err(e) if first => return Err(format!("the folder can't be read: {e}")),
            Err(_) => {
                m.skipped.push(Skip { path: prefix.to_string(), why: "folder can't be read".into() });
                continue;
            }
        };
        first = false;
        if entries.len() > room {
            entries.truncate(room);
            cut = true;
        }
        looked_at += entries.len();
        for entry in entries {
            let name = entry.name.to_string_lossy().into_owned();
            let rel = format!("{prefix}{name}");
            match entry.kind {
                Kind::Symlink => m.skipped.push(Skip { path: rel, why: "symlink, not followed".into() }),
                Kind::Dir if SKIP_DIRS.contains(&name.as_str()) => {
                    m.skipped.push(Skip { path: format!("{rel}/"), why: "not snapshotted".into() })
                }
                Kind::Dir if depth >= max_depth => {
                    m.skipped.push(Skip { path: format!("{rel}/"), why: format!("deeper than {max_depth} folders") })
                }
                Kind::Dir => stack.push((dir.clone(), Some(entry.name.clone()), format!("{rel}/"), at.join(&entry.name), depth + 1)),
                Kind::Other => m.skipped.push(Skip { path: rel, why: "not a regular file".into() }),
                Kind::File if entry.len > FILE_MAX_BYTES => m.skipped.push(Skip { path: rel, why: "over 20 MB".into() }),
                Kind::File => candidates.push((rel, at.join(&entry.name), entry.len, entry.mode & 0o777)),
            }
        }
        if cut || looked_at >= max_entries && !stack.is_empty() {
            // One skip names where the walk stopped; folders not reached yet aren't listed
            let path = if prefix.is_empty() { "./".to_string() } else { prefix.clone() };
            m.skipped.push(Skip { path, why: format!("walk stopped at the {max_entries}-{WALK_STOPPED}") });
            break;
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    if candidates.len() > MAX_FILES {
        for c in candidates.drain(MAX_FILES..) {
            m.skipped.push(Skip { path: c.0, why: format!("over the {MAX_FILES}-file limit") });
        }
    }
    // Past the cap, the largest files go first
    let mut total: u64 = candidates.iter().map(|c| c.2).sum();
    if total > max_bytes {
        let mut by_size: Vec<usize> = (0..candidates.len()).collect();
        by_size.sort_by(|a, b| candidates[*b].2.cmp(&candidates[*a].2));
        let mut drop = BTreeSet::new();
        for i in by_size {
            if total <= max_bytes {
                break;
            }
            total -= candidates[i].2;
            drop.insert(i);
        }
        let mb = max_bytes / (1024 * 1024);
        let mut kept = Vec::new();
        for (i, c) in candidates.into_iter().enumerate() {
            if drop.contains(&i) {
                m.skipped.push(Skip { path: c.0, why: format!("over the {mb} MB checkpoint cap") });
            } else {
                kept.push(c);
            }
        }
        candidates = kept;
    }
    // What counts toward the cap is what was really stored, not the listing's sizes
    let mut bytes: u64 = 0;
    let mb = max_bytes / (1024 * 1024);
    for (rel, at, _listed, mode) in candidates {
        let cap = FILE_MAX_BYTES.min(max_bytes.saturating_sub(bytes));
        match top.open_file(&at).and_then(|f| store_blob(blobs, f, cap)) {
            Ok((sha, len)) => {
                bytes += len;
                m.files.push(Entry { path: rel, sha256: sha, bytes: len, mode });
            }
            Err(e) if crate::sandbox::safe_fs::is_too_large(&e) && cap < FILE_MAX_BYTES => {
                m.skipped.push(Skip { path: rel, why: format!("over the {mb} MB checkpoint cap") })
            }
            Err(e) if crate::sandbox::safe_fs::is_too_large(&e) => m.skipped.push(Skip { path: rel, why: "over 20 MB".into() }),
            Err(_) => m.skipped.push(Skip { path: rel, why: "can't be read".into() }),
        }
    }
    m.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Taken { manifest: m, bytes, git_head: git_head_in(&top) })
}

fn write_manifest(dir: &FsPath, id: &str, m: &Manifest) -> std::io::Result<(PathBuf, String)> {
    private_dir(dir)?;
    let body = serde_json::to_vec(m).map_err(std::io::Error::other)?;
    let path = dir.join(format!("{id}.json"));
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).unix_mode(0o600).open(&path)?;
    f.write_all(&body)?;
    Ok((path, hex(&<sha2::Sha256 as sha2::Digest>::digest(&body))))
}

/// Writes `task_id`'s first checkpoint (its folder before its first run) into `dest`, an empty
/// scratch folder, for a Skill Foundry replay. `Ok(0)` when the task has no usable checkpoint.
/// Never touches the task's own folder.
pub async fn materialize_first(state: &AppState, task_id: &str, dest: &FsPath) -> Result<usize, String> {
    let first = state.db.checkpoints_of(task_id).await.map_err(|e| e.to_string())?.into_iter().find(|c| !c.failed());
    let Some(first) = first else { return Ok(0) };
    let _store = STORE.lock().await;
    let (blobs, dest) = (root(state).join("blobs"), dest.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let manifest = read_manifest(&first).ok_or("the checkpoint's manifest is gone")?;
        private_dir(&dest).map_err(|e| e.to_string())?;
        let mut n = 0;
        for e in manifest.files.iter().filter(|e| safe_rel(&e.path)) {
            let to = dest.join(&e.path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // A missing blob is skipped: the replay sees less, never something else
            if std::fs::copy(blobs.join(&e.sha256), &to).is_ok() {
                let _ = crate::fsx::set_mode(&to, (e.mode & 0o700) | 0o600);
                n += 1;
            }
        }
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A clean copy of a task's last run for the verifier (0.9.0 V2): what [`materialize_run`] wrote.
#[derive(Debug, Clone, Default)]
pub struct CleanCopy {
    /// Files written into the copy.
    pub files: usize,
    /// The paths the run changed (from its `before_run` to its `after_run`).
    pub changed: Vec<String>,
    /// What the copy is missing, in plain words (skipped by the snapshot, blob gone).
    pub missing: Vec<String>,
    pub after_run_id: String,
    /// The run's window: its `before_run` (or `after_run` when there is none) and its `after_run`.
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
}

/// How many skipped paths [`materialize_run`] lists by name before it counts the rest.
const MAX_MISSING_LISTED: usize = 8;

/// Writes `task_id`'s last run into `dest`, an empty scratch folder: the run's `before_run`
/// snapshot with the paths the run changed taken from its `after_run` (a path the run deleted is
/// left out). `Ok(None)` when the task has no usable `after_run`. Never touches the task's folder.
pub async fn materialize_run(state: &AppState, task_id: &str, dest: &FsPath) -> Result<Option<CleanCopy>, String> {
    let all = state.db.checkpoints_of(task_id).await.map_err(|e| e.to_string())?;
    let Some(after) = all.iter().rev().find(|c| c.kind == "after_run" && !c.failed()).cloned() else { return Ok(None) };
    let before = all.iter().rev().find(|c| c.kind == "before_run" && c.claim_id.is_some() && c.claim_id == after.claim_id && !c.failed() && c.folder == after.folder).cloned();
    let _store = STORE.lock().await;
    let (blobs, dest, runs_dir) = (root(state).join("blobs"), dest.to_path_buf(), root(state).join(task_id));
    tokio::task::spawn_blocking(move || {
        let after_m = read_manifest(&after).ok_or("the after_run checkpoint's manifest is gone")?;
        let before_m = before.as_ref().and_then(read_manifest);
        let recorded = read_runs(&runs_dir).into_iter().rev().find(|r| r.after_run_id == after.id).map(|r| r.paths);
        let changed = match (&before_m, recorded) {
            (_, Some(paths)) => paths,
            (Some(b), None) => changed(b, &after_m),
            (None, None) => after_m.files.iter().map(|f| f.path.clone()).collect(),
        };
        let mut files: BTreeMap<String, Entry> = before_m.as_ref().map(|m| m.files.iter().map(|e| (e.path.clone(), e.clone())).collect()).unwrap_or_default();
        let after_files: BTreeMap<&str, &Entry> = after_m.files.iter().map(|e| (e.path.as_str(), e)).collect();
        for p in &changed {
            match after_files.get(p.as_str()) {
                Some(e) => {
                    files.insert(p.clone(), (*e).clone());
                }
                None => {
                    files.remove(p);
                }
            }
        }
        let mut skipped: BTreeSet<String> = after_m.skipped.iter().map(|s| s.path.clone()).collect();
        if let Some(b) = &before_m {
            skipped.extend(b.skipped.iter().map(|s| s.path.clone()));
        }
        let mut missing: Vec<String> = skipped.iter().take(MAX_MISSING_LISTED).map(|p| format!("{p} (not in the snapshot)")).collect();
        if skipped.len() > MAX_MISSING_LISTED {
            missing.push(format!("{} more paths not in the snapshot", skipped.len() - MAX_MISSING_LISTED));
        }
        private_dir(&dest).map_err(|e| e.to_string())?;
        let mut n = 0;
        for e in files.values() {
            if !safe_rel(&e.path) {
                missing.push(format!("{} (unsafe path, left out)", e.path));
                continue;
            }
            let to = dest.join(&e.path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if std::fs::copy(blobs.join(&e.sha256), &to).is_ok() {
                let _ = crate::fsx::set_mode(&to, (e.mode & 0o700) | 0o600);
                n += 1;
            } else {
                missing.push(format!("{} (its stored copy is gone)", e.path));
            }
        }
        let from = before.as_ref().map_or(after.created_at, |b| b.created_at);
        Ok(Some(CleanCopy { files: n, changed, missing, after_run_id: after.id.clone(), from, to: after.created_at }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether `task_id` has an `after_run` checkpoint a clean copy can be made from.
pub async fn has_run(state: &AppState, task_id: &str) -> bool {
    state.db.checkpoints_of(task_id).await.unwrap_or_default().iter().any(|c| c.kind == "after_run" && !c.failed())
}

/// The manifest of `c`, if it is still on disk.
pub fn read_manifest(c: &Checkpoint) -> Option<Manifest> {
    if c.failed() || c.manifest_path.is_empty() {
        return None;
    }
    serde_json::from_slice(&std::fs::read(&c.manifest_path).ok()?).ok()
}

/// A path's state in a manifest: its sha, `Skipped`, or absent.
#[derive(Debug, Clone, PartialEq)]
enum At {
    File(String),
    Skipped(String),
    Absent,
}

fn index(m: &Manifest) -> BTreeMap<String, At> {
    let mut out = BTreeMap::new();
    for s in &m.skipped {
        if !s.path.ends_with('/') {
            out.insert(s.path.clone(), At::Skipped(s.why.clone()));
        }
    }
    for f in &m.files {
        out.insert(f.path.clone(), At::File(f.sha256.clone()));
    }
    out
}

/// The paths that differ between two snapshots of the same folder.
fn changed(before: &Manifest, after: &Manifest) -> Vec<String> {
    let (b, a) = (index(before), index(after));
    let keys: BTreeSet<&String> = b.keys().chain(a.keys()).collect();
    keys.into_iter()
        .filter(|k| {
            let (x, y) = (b.get(*k).cloned().unwrap_or(At::Absent), a.get(*k).cloned().unwrap_or(At::Absent));
            x != y && !matches!((&x, &y), (At::Skipped(_), At::Skipped(_)))
        })
        .cloned()
        .collect()
}

/// Why the checkpoint can't say what `rel` was when it was taken, or None when it can (the file
/// is in it, or the walk reached where it would be and it wasn't there). A restore never creates,
/// overwrites or deletes a path the checkpoint can't vouch for: a file under a folder the walk
/// skipped (too deep, unreadable, never snapshotted), or anywhere at all once the walk stopped at
/// [`MAX_ENTRIES`], since folders not reached yet aren't listed.
/// The caller has already checked that `rel` isn't one of the checkpoint's files.
fn not_vouched(m: &Manifest, rel: &str) -> Option<String> {
    if let Some(s) = m.skipped.iter().find(|s| s.path == rel) {
        return Some(s.why.clone());
    }
    if let Some(s) = m.skipped.iter().find(|s| s.path.ends_with('/') && s.path != "./" && rel.starts_with(&s.path)) {
        return Some(s.why.clone());
    }
    m.skipped.iter().find(|s| s.why.ends_with(WALK_STOPPED)).map(|s| s.why.clone())
}

fn read_runs(dir: &FsPath) -> Vec<Run> {
    std::fs::read_to_string(dir.join("runs.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append_run(dir: &FsPath, run: &Run) -> std::io::Result<()> {
    private_dir(dir)?;
    let mut f = std::fs::OpenOptions::new().append(true).create(true).unix_mode(0o600).open(dir.join("runs.jsonl"))?;
    writeln!(f, "{}", serde_json::to_string(run).map_err(std::io::Error::other)?)
}

// Taking checkpoints

/// Takes a checkpoint of `task`'s folder. Best-effort: a failure is stored as a checkpoint with
/// its `error` (not restorable) and a `checkpoint_failed` event, and never stops the run.
/// Run checkpoints (`before_run`, `after_run`) follow the `checkpoints` setting; manual and
/// `before_restore` ones are always taken.
pub async fn take(state: &AppState, task: &Task, kind: &str, claim: Option<&Claim>, created_by: &str) -> Option<Checkpoint> {
    take_in(state, task, kind, claim, created_by, None).await
}

/// [`take`], of `pinned`'s folder (already open, by that path) instead of the task's folder.
async fn take_in(
    state: &AppState,
    task: &Task,
    kind: &str,
    claim: Option<&Claim>,
    created_by: &str,
    pinned: Option<(PathBuf, std::sync::Arc<crate::sandbox::safe_fs::Dir>)>,
) -> Option<Checkpoint> {
    if matches!(kind, "before_run" | "after_run") && !enabled(state).await {
        return None;
    }
    let (folder, pinned) = match pinned {
        Some((f, d)) => (Some(f), Some(d)),
        None => (task_folder(state, task).await.map(|f| resolve_above(&f)), None),
    };
    let max_bytes = max_mb(state).await as u64 * 1024 * 1024;
    let root = root(state);
    let task_dir = root.join(&task.id);
    let id = new_id();
    let held = STORE.lock().await;
    let result = match folder.clone() {
        None => Err("the task has no folder".to_string()),
        Some(folder) => {
            let (root, task_dir, id) = (root.clone(), task_dir.clone(), id.clone());
            tokio::task::spawn_blocking(move || -> Result<(Taken, PathBuf, String), String> {
                private_dir(&root).map_err(|e| format!("the checkpoint store can't be written: {e}"))?;
                let taken = match pinned {
                    Some(d) => snapshot_in(&root.join("blobs"), d.open_dir(FsPath::new("")).map_err(|e| format!("the folder can't be read: {e}"))?, max_bytes)?,
                    None => snapshot(&root.join("blobs"), &folder, max_bytes)?,
                };
                let (path, sha) = write_manifest(&task_dir, &id, &taken.manifest).map_err(|e| format!("the manifest can't be written: {e}"))?;
                Ok((taken, path, sha))
            })
            .await
            .unwrap_or_else(|e| Err(format!("the snapshot stopped: {e}")))
        }
    };
    let folder_s = folder.as_ref().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let new = |manifest_path: String, manifest_sha256: String, files: i64, bytes: i64, skipped: i64, git_head: Option<String>, error: Option<String>| NewCheckpoint {
        id: id.clone(),
        task_id: &task.id,
        mission_id: task.mission_id.as_deref(),
        kind,
        claim_id: claim.map(|c| c.id.as_str()),
        turn_id: claim.and_then(|c| c.turn_id.as_deref()),
        attempt: task.attempts,
        manifest_path,
        manifest_sha256,
        folder: folder_s.clone(),
        files,
        bytes,
        skipped,
        git_head,
        error,
        created_by,
    };
    let (row, manifest) = match result {
        Ok((t, path, sha)) => {
            let n = new(
                path.to_string_lossy().into_owned(),
                sha,
                t.manifest.files.len() as i64,
                t.bytes as i64,
                t.manifest.skipped.len() as i64,
                t.git_head.clone(),
                None,
            );
            (state.db.insert_checkpoint(&n).await, Some(t.manifest))
        }
        Err(why) => {
            tracing::warn!(task_id = %task.id, %kind, %why, "checkpoint failed");
            (state.db.insert_checkpoint(&new(String::new(), String::new(), 0, 0, 0, None, Some(why))).await, None)
        }
    };
    let row = match row {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "could not store a checkpoint");
            return None;
        }
    };
    if kind == "after_run" {
        if let Some(after) = &manifest {
            note_run(state, task, &row, after).await;
        }
    }
    retain(state, &task.id).await;
    gc(state).await;
    drop(held);
    emit_task_id(state, &task.id).await;
    Some(row)
}

async fn emit_task_id(state: &AppState, id: &str) {
    if let Ok(Some(t)) = state.db.get_task(id).await {
        emit_task(state, &t).await;
    }
}

/// Records the paths this run changed: its `after_run` against the `before_run` of the same claim.
async fn note_run(state: &AppState, task: &Task, after: &Checkpoint, after_m: &Manifest) {
    let all = state.db.checkpoints_of(&task.id).await.unwrap_or_default();
    let before = all
        .iter()
        .rev()
        .find(|c| c.kind == "before_run" && c.claim_id.is_some() && c.claim_id == after.claim_id && !c.failed())
        .and_then(|c| read_manifest(c).map(|m| (c.folder.clone(), m)));
    let paths = match before {
        Some((folder, m)) if folder == after.folder => changed(&m, after_m),
        // No usable before_run (it failed, or checkpoints were off): every file is the task's
        _ => {
            tracing::info!(task_id = %task.id, "after_run without its before_run: every file counts as changed");
            after_m.files.iter().map(|f| f.path.clone()).collect()
        }
    };
    // The run's own workspace calls since its before_run (all of the task's when there is none)
    let since = all.iter().rev().find(|c| c.kind == "before_run" && c.claim_id.is_some() && c.claim_id == after.claim_id).map(|c| c.created_at);
    let calls = state.db.tool_calls_of(&task.id).await.unwrap_or_default();
    let named: BTreeSet<&str> = calls
        .iter()
        .filter(|t| t.decision != "denied" && since.is_none_or(|s| t.created_at >= s) && t.created_at <= after.created_at)
        .filter_map(|t| t.path.as_deref())
        .collect();
    let touched = paths.iter().filter(|p| named.contains(p.as_str())).cloned().collect();
    let run = Run {
        claim_id: after.claim_id.clone(),
        after_run_id: after.id.clone(),
        at: after.created_at,
        folder: after.folder.clone(),
        paths,
        touched,
    };
    let dir = root(state).join(&task.id);
    if let Err(e) = tokio::task::spawn_blocking(move || append_run(&dir, &run)).await.unwrap_or_else(|e| Err(std::io::Error::other(e))) {
        tracing::warn!(error = %e, "could not record a run's paths");
    }
}

/// Keeps the last `checkpoint_keep` of the task plus its first `before_run`. Caller holds STORE.
async fn retain(state: &AppState, task_id: &str) {
    let keep = keep(state).await as usize;
    let all = state.db.checkpoints_of(task_id).await.unwrap_or_default();
    if all.len() <= keep {
        return;
    }
    let first = all.iter().find(|c| c.kind == "before_run").map(|c| c.id.clone());
    let cut = all.len() - keep;
    for c in all.iter().take(cut).filter(|c| Some(&c.id) != first.as_ref()) {
        prune(state, c, "past checkpoint_keep").await;
    }
}

async fn prune(state: &AppState, c: &Checkpoint, why: &str) {
    if !c.manifest_path.is_empty() {
        let _ = std::fs::remove_file(&c.manifest_path);
    }
    if let Err(e) = state.db.prune_checkpoint(c, why).await {
        tracing::warn!(error = %e, "could not prune a checkpoint");
    }
}

/// Deletes blobs no manifest refers to, then prunes the oldest checkpoints (sparing each task's
/// first `before_run`) while the store is over 2 GB. Caller holds STORE.
async fn gc(state: &AppState) {
    let blobs = root(state).join("blobs");
    loop {
        let all = state.db.all_checkpoints().await.unwrap_or_default();
        let manifests: Vec<String> = all.iter().filter(|c| !c.manifest_path.is_empty()).map(|c| c.manifest_path.clone()).collect();
        let dir = blobs.clone();
        let total = tokio::task::spawn_blocking(move || sweep_blobs(&dir, &manifests)).await.unwrap_or(0);
        if total <= TOTAL_MAX_BYTES {
            return;
        }
        let mut firsts = BTreeSet::new();
        for c in &all {
            if c.kind == "before_run" && !firsts.iter().any(|(t, _): &(String, String)| *t == c.task_id) {
                firsts.insert((c.task_id.clone(), c.id.clone()));
            }
        }
        let Some(oldest) = all.iter().find(|c| !firsts.iter().any(|(_, id)| *id == c.id)) else { return };
        prune(state, oldest, "over the 2 GB total").await;
    }
}

/// Removes unreferenced blobs and leftover temp files; returns the bytes still stored.
fn sweep_blobs(blobs: &FsPath, manifests: &[String]) -> u64 {
    let mut used = BTreeSet::new();
    for p in manifests {
        if let Some(m) = std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<Manifest>(&b).ok()) {
            used.extend(m.files.into_iter().map(|f| f.sha256));
        }
    }
    let mut total = 0;
    for e in std::fs::read_dir(blobs).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !used.contains(&name) {
            let _ = std::fs::remove_file(e.path());
        } else {
            total += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

/// `before_run`, right after a successful claim and before the turn starts.
pub async fn before_run(state: &AppState, task: &Task) {
    let claim = state.db.open_claim(&task.id).await.ok().flatten();
    take(state, task, "before_run", claim.as_ref(), "kwant").await;
}

/// `after_run`, once a run's claim was released (from `transition`, after the commit).
pub async fn after_run(state: &AppState, task: &Task) {
    let claim = state.db.last_released_claim(&task.id).await.ok().flatten();
    take(state, task, "after_run", claim.as_ref(), "kwant").await;
}

// The side-effect ledger

/// Claude's tools that only read or plan.
const NO_EFFECT_TOOLS: [&str; 11] =
    ["Read", "Glob", "Grep", "LS", "WebSearch", "TodoWrite", "TodoRead", "NotebookRead", "ToolSearch", "BashOutput", "ExitPlanMode"];
/// Claude's tools that write one file.
const FILE_TOOLS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// The side-effect class of a call (contract §3.4): unproven is `external`, and `Bash` always is.
pub fn effect(req: &crate::mcp::ToolApprovalRequest, task_folder: Option<&FsPath>) -> &'static str {
    if req.server == crate::mcp::SELF_SERVER_NAME {
        return if req.tool.starts_with("task_") || req.tool == "mission_plan" { "none" } else { "external" };
    }
    if !req.server.is_empty() || req.tool_name.starts_with("mcp__") {
        return "external";
    }
    let tool = req.tool_name.as_str();
    if NO_EFFECT_TOOLS.contains(&tool) {
        return "none";
    }
    if FILE_TOOLS.contains(&tool) {
        let Some((folder, abs)) = file_target(req, task_folder) else { return "external" };
        return if crate::mcp::builtin_tools::is_inside(&folder, &abs.to_string_lossy()) { "workspace" } else { "external" };
    }
    "external"
}

/// A file tool's canonical task folder and the absolute path it names (a relative path is the
/// bot's working folder's).
fn file_target(req: &crate::mcp::ToolApprovalRequest, task_folder: Option<&FsPath>) -> Option<(PathBuf, PathBuf)> {
    let path = ["file_path", "notebook_path", "path"].iter().find_map(|k| req.arguments[*k].as_str()).unwrap_or("");
    let folder = task_folder.and_then(|f| crate::mcp::builtin_tools::canonical_folder(Some(f)))?;
    if path.is_empty() {
        return None;
    }
    let p = FsPath::new(path);
    let abs = match (p.is_absolute(), req.builtin.as_ref().and_then(|b| b.cwd.as_deref())) {
        (true, _) => p.to_path_buf(),
        (false, Some(cwd)) => FsPath::new(cwd).join(p),
        (false, None) => return None,
    };
    Some((folder, abs))
}

/// The task-folder-relative file a `workspace` call names, as the manifests spell it.
fn workspace_path(req: &crate::mcp::ToolApprovalRequest, task_folder: Option<&FsPath>) -> Option<String> {
    let (folder, abs) = file_target(req, task_folder)?;
    crate::mcp::builtin_tools::inside_rel(&folder, &abs.to_string_lossy()).filter(|r| safe_rel(r))
}

/// Keys whose values are bodies, never shown: only their length.
const BODY_KEYS: [&str; 8] = ["content", "new_string", "old_string", "edits", "new_source", "body", "text", "html"];

/// The tool name and a redacted outline of its arguments, at most 300 characters.
pub fn summary(tool_name: &str, args: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(obj) = args.as_object() {
        for (k, v) in obj.iter().take(8) {
            let shown = match v {
                Value::String(s) if BODY_KEYS.contains(&k.as_str()) => format!("<{} chars>", s.chars().count()),
                Value::String(s) => {
                    let short: String = s.chars().take(80).collect();
                    if s.chars().count() > 80 {
                        format!("{short}…")
                    } else {
                        short
                    }
                }
                Value::Array(a) => format!("[{} items]", a.len()),
                Value::Object(_) => "{…}".into(),
                other => other.to_string(),
            };
            parts.push(format!("{k}={shown}"));
        }
    }
    let raw = if parts.is_empty() { tool_name.to_string() } else { format!("{tool_name} {}", parts.join(", ")) };
    let (scrubbed, _) = crate::import::scrub::scrub_text(&crate::profiles::classify::mask_credentials(&raw));
    let mut out: String = scrubbed.chars().take(300).collect();
    if scrubbed.chars().count() > 300 {
        out.pop();
        out.push('…');
    }
    out
}

/// Records one permission decision of a task turn (D9: a turn without a task records nothing).
pub async fn log_call(db: &crate::db::Database, data_dir: &FsPath, req: &crate::mcp::ToolApprovalRequest, decision: &'static str, approval_id: Option<&str>) {
    let Ok(Some(task_id)) = db.task_of_turn(&req.kwant_id, &req.turn_id).await else { return };
    let folder = match db.get_task(&task_id).await {
        Ok(Some(t)) => super::task_folder_in(db, data_dir, &t).await,
        _ => None,
    };
    let effect = effect(req, folder.as_deref());
    let path = if effect == "workspace" { workspace_path(req, folder.as_deref()) } else { None };
    let summary = summary(&req.tool_name, &req.arguments);
    if let Err(e) = db.log_tool_call(&req.turn_id, &req.kwant_id, &task_id, &req.tool_name, effect, &summary, decision, approval_id, path.as_deref()).await {
        tracing::warn!(error = %e, "could not log a tool call");
    }
}

// Restore planning

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub path: String,
    pub action: &'static str,
}

/// What restoring `c` would do. `conflicts` are left out of `changes`; `forced` holds what each
/// would do if `force_paths` names it.
#[derive(Debug, Clone)]
pub struct Plan {
    pub changes: Vec<Change>,
    pub conflicts: Vec<Value>,
    forced: BTreeMap<String, &'static str>,
    pub skipped: Vec<Value>,
    pub external: Vec<Value>,
    pub effects_known: bool,
    pub refused: Option<&'static str>,
    pub state_after: Option<&'static str>,
    pub sha: String,
    folder: PathBuf,
    entries: BTreeMap<String, Entry>,
}

#[derive(Debug)]
pub enum PlanError {
    NotRestorable,
    Db(String),
}

fn tool_json(t: &ToolCall) -> Value {
    json!({"tool_name": t.tool_name, "summary": t.summary, "at": t.created_at})
}

/// Whether a run's claim is still live.
async fn running(state: &AppState, task: &Task) -> bool {
    matches!(task.state.as_str(), "working" | "needs_you")
        && matches!(state.db.open_claim(&task.id).await, Ok(Some(c)) if c.expires_at > crate::db::missions::now_ms())
}

/// What restoring `c` would do. Holds STORE, so it never reads a run's `after_run` checkpoint
/// before that run's paths are recorded (`note_run`), which would show no changes.
pub async fn plan(state: &AppState, task: &Task, c: &Checkpoint) -> Result<Plan, PlanError> {
    let _store = STORE.lock().await;
    let Some(manifest) = read_manifest(c) else { return Err(PlanError::NotRestorable) };
    let folder = PathBuf::from(&c.folder);
    let task_dir = root(state).join(&task.id);
    let blobs = root(state).join("blobs");
    // Every path this task's runs changed, and what other tasks' runs changed after `c`
    let mut owned = BTreeSet::new();
    let mut own_touched = BTreeSet::new();
    for r in read_runs(&task_dir) {
        if r.folder == c.folder {
            owned.extend(r.paths);
            own_touched.extend(r.touched);
        }
    }
    // D11: a task is named only when its own logged calls wrote the path; a path that merely
    // changed while another task ran in the same folder is "changed outside this task"
    let mut others: BTreeMap<String, String> = BTreeMap::new();
    let mut outside = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for o in state.db.run_checkpoints_except(&task.id).await.map_err(|e| PlanError::Db(e.to_string()))? {
        if seen.insert(o.task_id.clone()) {
            for r in read_runs(&root(state).join(&o.task_id)) {
                if r.folder == c.folder && r.at > c.created_at {
                    for p in r.touched {
                        others.entry(p).or_insert_with(|| o.task_id.clone());
                    }
                    outside.extend(r.paths);
                }
            }
        }
    }
    // What this task's last run left: a path its own calls wrote, still as it left it, is its own
    let all = state.db.checkpoints_of(&task.id).await.map_err(|e| PlanError::Db(e.to_string()))?;
    let left: Option<BTreeMap<String, String>> = all
        .iter()
        .rev()
        .find(|x| x.kind == "after_run" && !x.failed() && x.folder == c.folder && x.created_at >= c.created_at)
        .and_then(read_manifest)
        .map(|m| m.files.into_iter().map(|e| (e.path, e.sha256)).collect());
    let mut skipped = Vec::new();
    let mut entries = BTreeMap::new();
    for e in &manifest.files {
        if !safe_rel(&e.path) {
            skipped.push(json!({"path": e.path, "why": "unsafe path"}));
        } else if owned.contains(&e.path) {
            entries.insert(e.path.clone(), e.clone());
        }
    }
    let mut changes = Vec::new();
    let mut conflicts = Vec::new();
    let mut forced = BTreeMap::new();
    let mut hashes = Vec::new();
    let check = {
        let (folder, owned, entries, blobs) = (folder.clone(), owned.clone(), entries.clone(), blobs.clone());
        let manifest = manifest.clone();
        tokio::task::spawn_blocking(move || {
            owned
                .into_iter()
                .map(|p| {
                    if !safe_rel(&p) {
                        return (p, Err("unsafe path".to_string()));
                    }
                    if let Some(why) = (!entries.contains_key(&p)).then(|| not_vouched(&manifest, &p)).flatten() {
                        return (p, Err(format!("{why}, not in the checkpoint")));
                    }
                    let now = match current(&folder, &p) {
                        Ok(now) => now,
                        Err(why) => return (p, Err(why)),
                    };
                    let action: Result<Option<&'static str>, String> = match (entries.get(&p), &now) {
                        (Some(e), Some(sha)) if *sha == e.sha256 => Ok(None),
                        (Some(e), _) if !blobs.join(&e.sha256).is_file() => Err("the stored copy is missing".into()),
                        (Some(_), Some(_)) => Ok(Some("overwrite")),
                        (Some(_), None) => Ok(Some("create")),
                        (None, Some(_)) => Ok(Some("delete")),
                        (None, None) => Ok(None),
                    };
                    let sha = now.unwrap_or_default();
                    (p, action.map(|a| (a, sha)))
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| PlanError::Db(e.to_string()))?
    };
    for (p, r) in check {
        match r {
            Err(why) => skipped.push(json!({"path": p, "why": why})),
            Ok((None, sha)) => hashes.push((p, sha)),
            Ok((Some(action), sha)) => {
                hashes.push((p.clone(), sha.clone()));
                let as_left = own_touched.contains(&p) && left.as_ref().is_some_and(|l| l.get(&p).map(String::as_str).unwrap_or("") == sha);
                if let Some(t) = others.get(&p) {
                    conflicts.push(json!({"path": p, "changed_by_task": t}));
                    forced.insert(p, action);
                } else if outside.contains(&p) && !as_left {
                    conflicts.push(json!({"path": p, "changed_by_task": null, "why": OUTSIDE}));
                    forced.insert(p, action);
                } else {
                    changes.push(Change { path: p, action });
                }
            }
        }
    }
    let calls = state.db.tool_calls_of(&task.id).await.map_err(|e| PlanError::Db(e.to_string()))?;
    let after: Vec<&ToolCall> = calls.iter().filter(|t| t.created_at >= c.created_at).collect();
    let external: Vec<Value> = after.iter().filter(|t| t.effect == "external").map(|t| tool_json(t)).collect();
    // A run after `c` changed files but logged no call that could have: the log is incomplete
    let runs_after = read_runs(&task_dir).into_iter().any(|r| r.at > c.created_at && !r.paths.is_empty());
    let effects_known = !runs_after || after.iter().any(|t| t.effect != "none");
    let refused = if task.state == "verified" {
        Some("fork_instead")
    } else if running(state, task).await {
        Some("running")
    } else {
        None
    };
    let state_after = match task.state.as_str() {
        "review" | "failed" => Some("ready"),
        "blocked" => Some("backlog"),
        _ => None,
    };
    let fingerprint = json!({"checkpoint": c.id, "task_state": task.state, "paths": hashes, "skipped": skipped, "conflicts": conflicts});
    let sha = hex(&<sha2::Sha256 as sha2::Digest>::digest(fingerprint.to_string().as_bytes()));
    Ok(Plan { changes, conflicts, forced, skipped, external, effects_known, refused, state_after, sha, folder, entries })
}

fn plan_json(c: &Checkpoint, p: &Plan) -> Value {
    json!({
        "checkpoint_id": c.id,
        "preview_sha256": p.sha,
        "changes": p.changes,
        "conflicts": p.conflicts,
        "skipped": p.skipped,
        "external_not_undone": p.external,
        "not_undone_note": NOT_UNDONE,
        "effects_known": p.effects_known,
        "task_state_after": p.state_after,
        "refused": p.refused,
    })
}

/// Writes the plan's changes (plus the forced conflicts) into `root`, the plan's folder opened
/// once with [`Dir::open_strict`](crate::sandbox::safe_fs::Dir::open_strict) (the same descriptor
/// the safety checkpoint read: QA F-3); never through a symlink.
fn apply(blobs: &FsPath, p: &Plan, force: &BTreeSet<String>, root: &crate::sandbox::safe_fs::Dir) -> (Vec<String>, Vec<String>, Vec<Value>) {
    let mut todo: Vec<(String, &'static str)> = p.changes.iter().map(|c| (c.path.clone(), c.action)).collect();
    todo.extend(p.forced.iter().filter(|(k, _)| force.contains(*k)).map(|(k, a)| (k.clone(), *a)));
    let (mut restored, mut deleted, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for (rel, action) in todo {
        if !safe_rel(&rel) || through_symlink(&p.folder, &rel) {
            failed.push(json!({"path": rel, "why": "symlink, not followed"}));
            continue;
        }
        // Every folder on the way is opened (or made) from the task folder's descriptor with
        // `O_NOFOLLOW`, and the file goes in with `openat`/`renameat` (S-8): a folder swapped for
        // a symlink is refused, never written through
        let relp = FsPath::new(&rel);
        let (parent, name) = (relp.parent().unwrap_or(FsPath::new("")), relp.file_name().unwrap_or_default());
        let r = (|| {
            if action == "delete" {
                root.open_dir(parent)?.remove_file(name).map(|_| false)
            } else {
                let e = &p.entries[&rel];
                let dir = root.make_dirs(parent)?;
                let mut src = std::fs::File::open(blobs.join(&e.sha256))?;
                dir.replace_file(name, &format!(".kwant-restore-{}", new_id()), &mut src, if e.mode == 0 { 0o644 } else { e.mode })?;
                Ok(true)
            }
        })();
        match r {
            Ok(true) => restored.push(rel),
            Ok(false) => deleted.push(rel),
            Err(e) => failed.push(json!({"path": rel, "why": format!("couldn't be written: {}", e.kind())})),
        }
    }
    (restored, deleted, failed)
}

/// Takes a `before_restore` checkpoint, then writes the plan. Returns the counts JSON.
#[allow(clippy::result_large_err)]
async fn write_back(state: &AppState, task: &Task, c: &Checkpoint, p: &Plan, force: BTreeSet<String>) -> Result<Value, Response> {
    // The folder is opened once, no symlink followed on any part of its path, and the safety
    // checkpoint and the writes both go through that descriptor (QA F-3): a task folder nested in
    // a bot's folder swapped for a symlink in between is never written through
    let folder = p.folder.clone();
    let opened = tokio::task::spawn_blocking(move || crate::sandbox::safe_fs::Dir::open_strict(&folder)).await.map_err(internal)?;
    let Ok(dir) = opened.map(std::sync::Arc::new) else {
        return Err(coded(
            StatusCode::INTERNAL_SERVER_ERROR,
            "checkpoint_failed",
            "the task's folder is missing, moved or a symlink now, so the safety checkpoint couldn't be taken and nothing was restored",
        ));
    };
    let Some(safety) = take_in(state, task, "before_restore", None, "person", Some((p.folder.clone(), dir.clone()))).await.filter(|s| !s.failed()) else {
        return Err(coded(StatusCode::INTERNAL_SERVER_ERROR, "checkpoint_failed", "the safety checkpoint couldn't be taken, so nothing was restored"));
    };
    let held = STORE.lock().await;
    let (blobs, plan) = (root(state).join("blobs"), p.clone());
    let (restored, deleted, failed) = tokio::task::spawn_blocking(move || apply(&blobs, &plan, &force, &dir)).await.map_err(internal)?;
    drop(held);
    let detail = json!({
        "checkpoint_id": c.id,
        "before_restore_checkpoint_id": safety.id,
        "restored": restored.len(),
        "deleted": deleted.len(),
        "failed": failed.len(),
        "external_not_undone": p.external.len(),
    });
    state.db.task_event(&task.id, task.mission_id.as_deref(), "restored", "person", &detail).await.map_err(internal)?;
    Ok(json!({
        "restored": restored,
        "deleted": deleted,
        "failed": failed,
        "before_restore_checkpoint_id": safety.id,
        "external_not_undone": p.external,
        "not_undone_note": NOT_UNDONE,
        "effects_known": p.effects_known,
    }))
}

fn coded(status: StatusCode, code: &str, msg: &str) -> Response {
    (status, Json(json!({"error": msg, "code": code}))).into_response()
}

#[allow(clippy::result_large_err)]
async fn load_checkpoint(state: &AppState, task: &Task, cid: &str) -> Result<Checkpoint, Response> {
    match state.db.get_checkpoint(cid).await {
        Ok(Some(c)) if c.task_id == task.id => Ok(c),
        Ok(_) => Err(not_found("checkpoint")),
        Err(e) => Err(internal(e)),
    }
}

#[allow(clippy::result_large_err)]
fn plan_err(e: PlanError) -> Response {
    match e {
        PlanError::NotRestorable => coded(StatusCode::CONFLICT, "not_restorable", "that checkpoint can't be restored (its snapshot failed or was removed)"),
        PlanError::Db(e) => internal(e),
    }
}

// API

/// The list item of one checkpoint; `next` is when the following one was taken.
fn item(c: &Checkpoint, calls: &[ToolCall], next: Option<chrono::DateTime<chrono::Utc>>) -> Value {
    let between: Vec<&ToolCall> = calls.iter().filter(|t| t.created_at >= c.created_at && next.is_none_or(|n| t.created_at < n)).collect();
    json!({
        "id": c.id,
        "kind": c.kind,
        "attempt": c.attempt,
        "created_at": c.created_at,
        "created_by": c.created_by,
        "files": c.files,
        "bytes": c.bytes,
        "skipped": c.skipped,
        "git_head": c.git_head,
        "error": c.error,
        "restorable": !c.failed() && FsPath::new(&c.manifest_path).is_file(),
        "effects_after": {
            "workspace": between.iter().filter(|t| t.effect == "workspace").count(),
            "external": between.iter().filter(|t| t.effect == "external").map(|t| tool_json(t)).collect::<Vec<_>>(),
        },
    })
}

#[allow(clippy::result_large_err)]
async fn items(state: &AppState, task: &Task) -> Result<Vec<Value>, Response> {
    let all = state.db.checkpoints_of(&task.id).await.map_err(internal)?;
    let calls = state.db.tool_calls_of(&task.id).await.map_err(internal)?;
    Ok(all.iter().enumerate().map(|(i, c)| item(c, &calls, all.get(i + 1).map(|n| n.created_at))).collect())
}

/// `GET /api/tasks/:id/checkpoints`
pub async fn list(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let task = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    match items(&state, &task).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

/// `POST /api/tasks/:id/checkpoints` (person-only): a manual checkpoint.
pub async fn create(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let task = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let Some(c) = take(&state, &task, "manual", None, "person").await else { return internal("the checkpoint wasn't stored") };
    let calls = state.db.tool_calls_of(&task.id).await.unwrap_or_default();
    let status = if c.failed() { StatusCode::INTERNAL_SERVER_ERROR } else { StatusCode::CREATED };
    (status, Json(item(&c, &calls, None))).into_response()
}

/// `GET /api/tasks/:id/checkpoints/:cid/restore-preview`
pub async fn preview(State(state): State<AppState>, RawQuery(q): RawQuery, Path((id, cid)): Path<(String, String)>) -> Response {
    let task = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let c = match load_checkpoint(&state, &task, &cid).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match plan(&state, &task, &c).await {
        Ok(p) => Json(plan_json(&c, &p)).into_response(),
        Err(e) => plan_err(e),
    }
}

/// `force_paths` entries the preview lists neither as a change nor as a conflict.
fn unlisted_force_paths<'a>(p: &Plan, force: &'a [String]) -> Vec<&'a str> {
    let listed: BTreeSet<&str> = p.changes.iter().map(|c| c.path.as_str()).chain(p.conflicts.iter().filter_map(|c| c["path"].as_str())).collect();
    let mut out: Vec<&str> = force.iter().map(String::as_str).filter(|f| !listed.contains(f)).collect();
    out.dedup();
    out
}

#[derive(Deserialize)]
pub struct RestoreBody {
    preview_sha256: String,
    #[serde(default)]
    force_paths: Vec<String>,
    /// "Restore and run again"; otherwise a task the runner could pick up is held (D10).
    #[serde(default)]
    rerun: bool,
}

/// `POST /api/tasks/:id/checkpoints/:cid/restore` (person-only, `Idempotency-Key` required).
pub async fn restore(
    State(state): State<AppState>,
    RawQuery(q): RawQuery,
    headers: HeaderMap,
    Path((id, cid)): Path<(String, String)>,
    Json(b): Json<RestoreBody>,
) -> Response {
    if headers.get("idempotency-key").is_none() {
        return bad("an Idempotency-Key header is required");
    }
    let route = format!("POST /api/tasks/{id}/checkpoints/{cid}/restore");
    super::graph::idempotent(&state, &headers, route, || async {
        let task = load_task(&state, q.as_deref(), &id).await?;
        let c = load_checkpoint(&state, &task, &cid).await?;
        let p = plan(&state, &task, &c).await.map_err(plan_err)?;
        match p.refused {
            Some("fork_instead") => {
                return Err(coded(StatusCode::CONFLICT, "fork_instead", "a verified task isn't restored; fork it from this checkpoint instead"))
            }
            Some(_) => return Err(coded(StatusCode::CONFLICT, "running", "the task is running; restore it once the run ends")),
            None => {}
        }
        if p.sha != b.preview_sha256.trim() {
            return Err(coded(StatusCode::CONFLICT, "stale_preview", "the folder or the task changed since the preview; look at it again"));
        }
        let unknown = unlisted_force_paths(&p, &b.force_paths);
        if !unknown.is_empty() {
            let msg = format!("force_paths names a path the preview doesn't list: {}", unknown.join(", "));
            return Err(coded(StatusCode::UNPROCESSABLE_ENTITY, "unknown_force_path", &msg));
        }
        let was_approved = super::flight_school::approved(&state.db, &task).await;
        let mut out = write_back(&state, &task, &c, &p, b.force_paths.into_iter().collect()).await?;
        out["stepped_down"] = super::flight_school::step_down_after_rollback(&state, &task, was_approved, &c.id).await;
        // Held before the move, so the runner never sees it ready and unheld (D10)
        let runnable = matches!(p.state_after.unwrap_or(task.state.as_str()), "ready" | "backlog");
        let changed = state.db.set_task_hold(&task.id, runnable && !b.rerun).await.map_err(internal)?;
        if changed {
            let kind = if b.rerun { "released" } else { "held" };
            state.db.task_event(&task.id, task.mission_id.as_deref(), kind, "person", &json!({"checkpoint_id": c.id})).await.map_err(internal)?;
        }
        let mut t = state.db.get_task(&task.id).await.map_err(internal)?.unwrap_or(task.clone());
        if let Some(to) = p.state_after {
            let change = TaskChange { reason: Some("restored from a checkpoint".into()), detail: Some(json!({"checkpoint_id": c.id})), ..Default::default() };
            t = super::routes::moved(transition(&state, &task.id, &task.state, to, Actor::Person, "person", change).await)?;
        } else {
            emit_task(&state, &t).await;
        }
        out["task"] = task_json(&state, &t).await;
        Ok(out)
    })
    .await
}

/// `POST /api/tasks/:id/run` (person-only; `Idempotency-Key` optional): lets a task a restore
/// held be claimed again (D10). The same key again replays the first 200 instead of `not_held`.
pub async fn run_held(State(state): State<AppState>, RawQuery(q): RawQuery, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let route = format!("POST /api/tasks/{id}/run");
    super::graph::idempotent(&state, &headers, route, || async {
        let task = load_task(&state, q.as_deref(), &id).await?;
        match state.db.set_task_hold(&task.id, false).await {
            Ok(true) => {
                let _ = state.db.task_event(&task.id, task.mission_id.as_deref(), "released", "person", &json!({})).await;
            }
            Ok(false) => return Err(coded(StatusCode::CONFLICT, "not_held", "the task isn't on hold")),
            Err(e) => return Err(internal(e)),
        }
        let t = state.db.get_task(&task.id).await.ok().flatten().unwrap_or(task);
        emit_task(&state, &t).await;
        Ok(json!({"task": task_json(&state, &t).await}))
    })
    .await
}

/// Fork `from_checkpoint` (B1 `POST /api/tasks/:id/fork`): writes the original's task-owned
/// paths back to the checkpoint (after a `before_restore`), skipping other tasks' changes.
#[allow(clippy::result_large_err)]
pub async fn restore_for_fork(state: &AppState, orig: &Task, cid: &str) -> Result<Value, Response> {
    let c = load_checkpoint(state, orig, cid).await?;
    let p = plan(state, orig, &c).await.map_err(plan_err)?;
    if p.refused == Some("running") {
        return Err(coded(StatusCode::CONFLICT, "running", "the task is running; fork it once the run ends"));
    }
    let mut out = write_back(state, orig, &c, &p, BTreeSet::new()).await?;
    out["checkpoint_id"] = json!(c.id);
    out["conflicts"] = json!(p.conflicts);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Should-fix 1: the blob store reads at most its cap from the open handle and returns what
    /// it really read; a longer file is refused with nothing stored.
    #[test]
    fn a_blob_is_read_up_to_its_cap_and_counted_by_its_real_bytes() {
        let d = tempfile::tempdir().unwrap();
        let blobs = d.path().join("blobs");
        std::fs::create_dir(&blobs).unwrap();
        let src = d.path().join("f");
        std::fs::write(&src, b"0123456789").unwrap();
        let (sha, n) = store_blob(&blobs, std::fs::File::open(&src).unwrap(), 10).unwrap();
        assert_eq!(n, 10);
        assert_eq!(std::fs::read(blobs.join(&sha)).unwrap(), b"0123456789");
        let e = store_blob(&blobs, std::fs::File::open(&src).unwrap(), 9).unwrap_err();
        assert!(crate::sandbox::safe_fs::is_too_large(&e), "{e}");
        // Grown past the cap after an open that saw it small: the read itself stops
        let f = std::fs::File::open(&src).unwrap();
        std::fs::write(&src, vec![b'x'; 64]).unwrap();
        assert!(crate::sandbox::safe_fs::is_too_large(&store_blob(&blobs, f, 20).unwrap_err()));
        assert_eq!(std::fs::read_dir(&blobs).unwrap().count(), 1, "nothing stored for a refused file, no temp left");
    }

    #[test]
    fn only_plain_relative_paths_are_safe() {
        assert!(safe_rel("a.txt"));
        assert!(safe_rel("dir/b.txt"));
        assert!(!safe_rel("../escape"));
        assert!(!safe_rel("a/../../b"));
        assert!(!safe_rel("/etc/passwd"));
        assert!(!safe_rel("./a"));
        assert!(!safe_rel(""));
        assert!(!safe_rel("dir/"));
    }

    fn m(files: &[(&str, &str)], skipped: &[&str]) -> Manifest {
        Manifest {
            files: files.iter().map(|(p, s)| Entry { path: p.to_string(), sha256: s.to_string(), bytes: 1, mode: 0o644 }).collect(),
            skipped: skipped.iter().map(|p| Skip { path: p.to_string(), why: "over 20 MB".into() }).collect(),
        }
    }

    fn req(tool_name: &str, server: &str, args: Value, cwd: &str) -> crate::mcp::ToolApprovalRequest {
        crate::mcp::ToolApprovalRequest {
            kwant_id: "k".into(),
            kwant_name: "K".into(),
            turn_id: "t".into(),
            tool_name: tool_name.into(),
            server: server.into(),
            tool: tool_name.rsplit("__").next().unwrap_or(tool_name).into(),
            arguments: args,
            builtin: server.is_empty().then(|| crate::mcp::BuiltinCall { cwd: Some(cwd.into()), outside_folder: false }),
            smart: None,
            payment: None,
        }
    }

    #[test]
    fn unproven_side_effects_are_external() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_str().unwrap();
        let f = Some(dir.path());
        assert_eq!(effect(&req("Read", "", json!({"file_path": "/etc/hosts"}), d), f), "none");
        assert_eq!(effect(&req("Write", "", json!({"file_path": "a.txt"}), d), f), "workspace");
        assert_eq!(effect(&req("Edit", "", json!({"file_path": format!("{d}/b/c.txt")}), d), f), "workspace");
        assert_eq!(effect(&req("Write", "", json!({"file_path": "/tmp/elsewhere.txt"}), d), f), "external");
        assert_eq!(effect(&req("Write", "", json!({"file_path": "../up.txt"}), d), f), "external");
        assert_eq!(effect(&req("Write", "", json!({"file_path": "a.txt"}), d), None), "external");
        assert_eq!(effect(&req("Bash", "", json!({"command": "ls"}), d), f), "external", "D6: Bash is always external");
        assert_eq!(effect(&req("WebFetch", "", json!({"url": "https://x"}), d), f), "external");
        assert_eq!(effect(&req("mcp__gmail__send", "gmail", json!({}), d), f), "external");
        assert_eq!(effect(&req("mcp__kwnt_tools__task_update", crate::mcp::SELF_SERVER_NAME, json!({}), d), f), "none");
        assert_eq!(effect(&req("mcp__kwnt_tools__delegate", crate::mcp::SELF_SERVER_NAME, json!({}), d), f), "external");
    }

    #[test]
    fn summaries_never_carry_bodies_or_secrets() {
        let s = summary("Write", &json!({"file_path": "a.txt", "content": "TOP SECRET BODY"}));
        assert!(s.starts_with("Write ") && s.contains("a.txt") && s.contains("<15 chars>") && !s.contains("SECRET BODY"), "{s}");
        let s = summary("Bash", &json!({"command": "curl -H 'Authorization: Bearer sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789' https://x"}));
        assert!(!s.contains("abcdefghijklmnopqrstuvwxyz0123456789"), "{s}");
        assert!(summary("Bash", &json!({"command": "x".repeat(1000)})).chars().count() <= 300);
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    /// Hex-shaped on purpose: only the symlink and path checks can keep it out, not the hex check.
    const CANARY: &str = "c0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ff";

    /// A task folder with `.git/` and a canary file outside it.
    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("task");
        std::fs::create_dir_all(folder.join(".git/refs/heads")).unwrap();
        let canary = dir.path().join("canary");
        std::fs::write(&canary, format!("{CANARY}\n")).unwrap();
        (dir, folder, canary)
    }

    fn head_of(folder: &FsPath) -> Option<String> {
        let h = git_head(folder);
        assert!(!h.as_deref().unwrap_or("").contains("c0ffee"), "canary leaked: {h:?}");
        h
    }

    #[test]
    fn git_head_reads_a_branch_a_detached_head_and_a_packed_ref() {
        let (_d, folder, _) = repo();
        let git = folder.join(".git");
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/heads/main"), format!("{SHA}\n")).unwrap();
        assert_eq!(head_of(&folder).as_deref(), Some(SHA));

        std::fs::write(git.join("HEAD"), format!("{}\n", SHA.to_uppercase())).unwrap();
        assert_eq!(head_of(&folder).as_deref(), Some(SHA), "detached, lowercased");
        let sha256 = "ab".repeat(32);
        std::fs::write(git.join("HEAD"), &sha256).unwrap();
        assert_eq!(head_of(&folder), Some(sha256), "a SHA-256 repo");

        std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        std::fs::write(
            git.join("packed-refs"),
            format!("# pack-refs with: peeled fully-peeled sorted\n{CANARY} refs/heads/feature/xy\n{SHA} refs/heads/feature/x\n^{CANARY}\n"),
        )
        .unwrap();
        assert_eq!(head_of(&folder).as_deref(), Some(SHA), "exact name in packed-refs");
        std::fs::write(git.join("HEAD"), "ref: refs/heads/none\n").unwrap();
        assert_eq!(head_of(&folder), None, "a branch with no commit yet");
        assert_eq!(head_of(&folder.join("missing")), None);
    }

    #[test]
    fn git_head_only_returns_a_commit_id() {
        let (_d, folder, _) = repo();
        let git = folder.join(".git");
        for bad in ["hello world", "0123456789abcdef", &format!("{SHA}0"), "g123456789abcdef0123456789abcdef01234567", "", "ref: "] {
            std::fs::write(git.join("HEAD"), bad).unwrap();
            assert_eq!(head_of(&folder), None, "{bad:?}");
        }
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(git.join("refs/heads/main"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();
        assert_eq!(head_of(&folder), None, "a ref file that isn't a commit id");
        std::fs::write(git.join("refs/heads/main"), format!("{SHA}\n{}", " ".repeat(5000))).unwrap();
        assert_eq!(head_of(&folder), None, "over the 4 KB cap");
    }

    #[test]
    fn git_head_never_leaves_dot_git() {
        let (dir, folder, _) = repo();
        let git = folder.join(".git");
        // A real loose ref outside .git that a traversal would reach
        std::fs::create_dir_all(dir.path().join("refs/heads")).unwrap();
        std::fs::write(dir.path().join("refs/heads/main"), CANARY).unwrap();
        for r in [
            "../../canary",
            "refs/../../../canary",
            "refs/heads/../../../../canary",
            "/etc/hostname",
            "refs//heads/main",
            "refs/./heads/main",
            "refs\\heads\\main",
            "refs/heads/ma in",
            "heads/main",
            "refs/",
        ] {
            std::fs::write(git.join("HEAD"), format!("ref: {r}\n")).unwrap();
            assert_eq!(head_of(&folder), None, "{r:?}");
        }
        assert!(plain_ref("refs/heads/feature/x-1_2.3") && !plain_ref("refs/heads/a..b"));
    }

    #[test]
    fn git_head_ignores_a_gitdir_file() {
        let (dir, folder, _) = repo();
        std::fs::remove_dir_all(folder.join(".git")).unwrap();
        let other = dir.path().join("other.git");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("HEAD"), CANARY).unwrap();
        std::fs::write(folder.join(".git"), format!("gitdir: {}\n", other.display())).unwrap();
        assert_eq!(head_of(&folder), None);
    }

    #[cfg(unix)]
    #[test]
    fn git_head_never_follows_a_symlink() {
        use std::os::unix::fs::symlink;
        let (dir, folder, canary) = repo();
        let git = folder.join(".git");

        // HEAD → canary
        symlink(&canary, git.join("HEAD")).unwrap();
        assert_eq!(head_of(&folder), None, "HEAD symlink");
        std::fs::remove_file(git.join("HEAD")).unwrap();

        // the ref file → canary
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        symlink(&canary, git.join("refs/heads/main")).unwrap();
        assert_eq!(head_of(&folder), None, "ref file symlink");
        std::fs::remove_file(git.join("refs/heads/main")).unwrap();

        // a folder on the ref's way → elsewhere
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("main"), CANARY).unwrap();
        std::fs::remove_dir(git.join("refs/heads")).unwrap();
        symlink(&elsewhere, git.join("refs/heads")).unwrap();
        assert_eq!(head_of(&folder), None, "refs/heads symlink");
        std::fs::remove_file(git.join("refs/heads")).unwrap();
        std::fs::create_dir(git.join("refs/heads")).unwrap();

        // packed-refs → a canary in packed-refs shape
        let packed = dir.path().join("packed");
        std::fs::write(&packed, format!("{CANARY} refs/heads/main\n")).unwrap();
        symlink(&packed, git.join("packed-refs")).unwrap();
        assert_eq!(head_of(&folder), None, "packed-refs symlink");
        std::fs::remove_file(git.join("packed-refs")).unwrap();

        // .git → a whole other repo whose HEAD is the canary
        let other = dir.path().join("other.git");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("HEAD"), CANARY).unwrap();
        std::fs::remove_dir_all(&git).unwrap();
        symlink(&other, &git).unwrap();
        assert_eq!(head_of(&folder), None, ".git symlink");
    }

    /// Windows: no FIFOs or O_NOFOLLOW there, so the same refusals come from the reparse-point and
    /// link-count checks (gitfile). A hard-linked ref, a junction on the way or as `.git` is `None`.
    #[cfg(windows)]
    #[test]
    fn git_head_refuses_junctions_and_hard_links_on_windows() {
        use crate::server::gitfile::windows_tests::junction;
        let (d, folder, canary) = repo();
        let git = folder.join(".git");
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/heads/main"), format!("{SHA}\n")).unwrap();
        assert_eq!(head_of(&folder).as_deref(), Some(SHA), "a plain repo reads");
        std::fs::remove_file(git.join("refs/heads/main")).unwrap();
        std::fs::hard_link(&canary, git.join("refs/heads/main")).unwrap();
        assert_eq!(head_of(&folder), None, "a ref that is the canary's second name");
        std::fs::remove_file(git.join("refs/heads/main")).unwrap();
        let elsewhere = d.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::copy(&canary, elsewhere.join("main")).unwrap();
        std::fs::remove_dir(git.join("refs/heads")).unwrap();
        junction(&git.join("refs/heads"), &elsewhere);
        assert_eq!(head_of(&folder), None, "refs/heads a junction");
        let other = d.path().join("other");
        std::fs::create_dir_all(other.join("refs/heads")).unwrap();
        std::fs::write(other.join("HEAD"), format!("{CANARY}\n")).unwrap();
        let task2 = d.path().join("task2");
        std::fs::create_dir_all(&task2).unwrap();
        junction(&task2.join(".git"), &other);
        assert_eq!(head_of(&task2), None, ".git a junction");
    }

    #[cfg(unix)]
    #[test]
    fn git_head_does_not_block_on_a_fifo() {
        let (_d, folder, _) = repo();
        let git = folder.join(".git");
        let fifo = |p: &FsPath| {
            let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        };
        fifo(&git.join("HEAD"));
        let run = |folder: PathBuf| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || tx.send(git_head(&folder)).unwrap());
            rx.recv_timeout(std::time::Duration::from_secs(5)).expect("git_head blocked on a FIFO")
        };
        assert_eq!(run(folder.clone()), None, "HEAD FIFO");
        std::fs::remove_file(git.join("HEAD")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fifo(&git.join("refs/heads/main"));
        fifo(&git.join("packed-refs"));
        assert_eq!(run(folder), None, "ref and packed-refs FIFOs");
    }

    /// F-2: a hard link is a regular file, so a HEAD (or ref) that is another name for a host
    /// file whose whole content is hex would pass every other check.
    #[cfg(unix)]
    #[test]
    fn git_head_never_reads_a_hard_link() {
        let (_d, folder, canary) = repo();
        let git = folder.join(".git");
        std::fs::hard_link(&canary, git.join("HEAD")).unwrap();
        assert_eq!(head_of(&folder), None, "HEAD hard-linked to a host file");
        std::fs::remove_file(git.join("HEAD")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::hard_link(&canary, git.join("refs/heads/main")).unwrap();
        assert_eq!(head_of(&folder), None, "loose ref hard-linked to a host file");
    }

    #[test]
    fn a_run_owns_what_it_created_changed_or_deleted() {
        let before = m(&[("same", "1"), ("edited", "1"), ("gone", "1")], &["big", "node_modules/"]);
        let after = m(&[("same", "1"), ("edited", "2"), ("new", "1")], &["big", "node_modules/", "huge"]);
        assert_eq!(changed(&before, &after), vec!["edited", "gone", "huge", "new"]);
    }

    // QA F-3 (Rigel's probes in 446-probes/checkpoints-nested-probes.diff): a task folder nested in
    // a bot's folder, which that bot's container can write, swapped for a symlink to a folder
    // outside it between the safety checkpoint and the writes
    #[cfg(target_os = "linux")]
    fn exchange(a: &FsPath, b: &FsPath) -> bool {
        use std::os::unix::ffi::OsStrExt;
        let (a, b) = (std::ffi::CString::new(a.as_os_str().as_bytes()).unwrap(), std::ffi::CString::new(b.as_os_str().as_bytes()).unwrap());
        // SAFETY: valid C strings; RENAME_EXCHANGE (2) swaps two existing entries atomically
        unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), 2u32) == 0 }
    }

    #[cfg(unix)]
    fn nested() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let bot = d.path().join("bot-folder");
        let task = bot.join("proj");
        std::fs::create_dir_all(&task).unwrap();
        std::fs::write(task.join("authorized_keys"), "ATTACKER-KEY\n").unwrap();
        let victim = d.path().join("victim-ssh");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("authorized_keys"), "ORIGINAL\n").unwrap();
        let blobs = d.path().join("blobs");
        (d, bot, task, victim, blobs)
    }

    #[cfg(unix)]
    fn overwrite_plan(task: &FsPath, taken: &Taken) -> Plan {
        let entries: BTreeMap<String, Entry> = taken.manifest.files.iter().map(|e| (e.path.clone(), e.clone())).collect();
        Plan {
            changes: vec![Change { path: "authorized_keys".into(), action: "overwrite" }],
            conflicts: vec![],
            forced: BTreeMap::new(),
            skipped: vec![],
            external: vec![],
            effects_known: true,
            refused: None,
            state_after: None,
            sha: String::new(),
            folder: task.to_path_buf(),
            entries,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_task_folder_swapped_for_a_symlink_is_refused_by_the_safety_checkpoint() {
        let (_d, bot, task, victim, blobs) = nested();
        std::fs::rename(&task, bot.join("proj.moved")).unwrap();
        std::os::unix::fs::symlink(&victim, &task).unwrap();
        assert!(snapshot(&blobs, &task, 1 << 30).is_err(), "a symlinked task folder is never snapshotted");
        assert!(crate::sandbox::safe_fs::Dir::open_strict(&task).is_err(), "nor opened for a restore");
        assert_eq!(current(&task, "authorized_keys"), Err("symlink, not followed".into()));
        // A symlink on a part above the task folder is refused too
        std::fs::rename(&bot, bot.with_file_name("bot-real")).unwrap();
        std::os::unix::fs::symlink(bot.with_file_name("bot-real"), &bot).unwrap();
        assert!(crate::sandbox::safe_fs::Dir::open_strict(&bot.join("proj.moved")).is_err());
        assert_eq!(std::fs::read_to_string(victim.join("authorized_keys")).unwrap(), "ORIGINAL\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_restore_writes_through_the_descriptor_its_safety_checkpoint_read() {
        let (_d, bot, task, victim, blobs) = nested();
        let taken = snapshot(&blobs, &task, 1 << 30).unwrap();
        let plan = overwrite_plan(&task, &taken);
        std::fs::write(task.join("authorized_keys"), "CHANGED\n").unwrap();
        // write_back: open the folder, take the safety checkpoint through it…
        let root = crate::sandbox::safe_fs::Dir::open_strict(&task).unwrap();
        let safety = snapshot_in(&blobs, root.open_dir(FsPath::new("")).unwrap(), 1 << 30).unwrap();
        assert_eq!(safety.manifest.files.len(), 1);
        // …the bot swaps the folder for a symlink to one outside its own…
        std::fs::rename(&task, bot.join("proj.moved")).unwrap();
        std::os::unix::fs::symlink(&victim, &task).unwrap();
        // …and the writes still land in the folder the checkpoint read, never in the other one
        let (restored, _deleted, failed) = apply(&blobs, &plan, &BTreeSet::new(), &root);
        assert_eq!(restored, vec!["authorized_keys".to_string()], "{failed:?}");
        assert_eq!(std::fs::read_to_string(victim.join("authorized_keys")).unwrap(), "ORIGINAL\n");
        assert_eq!(std::fs::read_to_string(bot.join("proj.moved/authorized_keys")).unwrap(), "ATTACKER-KEY\n");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_restore_racing_a_folder_swap_never_writes_outside_it() {
        let (_d, bot, task, victim, blobs) = nested();
        let lnk = bot.join("lnk");
        std::os::unix::fs::symlink(&victim, &lnk).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let swapper = {
            let (stop, t, l) = (stop.clone(), task.clone(), lnk.clone());
            std::thread::spawn(move || {
                let mut swaps = 0u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    swaps += u32::from(exchange(&t, &l));
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                swaps
            })
        };
        let (mut hits, mut applied) = (0, 0);
        for _ in 0..300 {
            // As write_back does: one strict open, then the snapshot and the writes through it
            let Ok(root) = crate::sandbox::safe_fs::Dir::open_strict(&task) else { continue };
            let Ok(taken) = snapshot_in(&blobs, root.open_dir(FsPath::new("")).unwrap(), 1 << 30) else { continue };
            if taken.manifest.files.is_empty() {
                continue;
            }
            let _ = apply(&blobs, &overwrite_plan(&task, &taken), &BTreeSet::new(), &root);
            applied += 1;
            if std::fs::read_to_string(victim.join("authorized_keys")).unwrap_or_default().contains("ATTACKER") {
                hits += 1;
                std::fs::write(victim.join("authorized_keys"), "ORIGINAL\n").unwrap();
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(swapper.join().unwrap() > 0, "the folder was really swapped");
        assert!(applied > 0, "some restores ran");
        assert_eq!(hits, 0, "a restore wrote into the folder the task folder was swapped for");
    }

    /// QA F-4: a symlink the person made above the task folder (`~/Projects -> /mnt/ssd/Projects`)
    /// works for checkpoint and restore, because a checkpoint records the folder with the parts
    /// above it resolved when it is taken; a swap after that is still refused (QA F-3).
    #[cfg(unix)]
    #[test]
    fn a_symlink_the_person_made_above_the_folder_works_and_a_later_swap_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let ssd = d.path().join("ssd");
        std::fs::create_dir_all(ssd.join("Projects/proj")).unwrap();
        std::fs::write(ssd.join("Projects/proj/authorized_keys"), "V0\n").unwrap();
        let home = d.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::os::unix::fs::symlink(ssd.join("Projects"), home.join("Projects")).unwrap();
        let typed = home.join("Projects/proj");
        let blobs = d.path().join("blobs");
        assert!(crate::sandbox::safe_fs::Dir::open_strict(&typed).is_err(), "the typed path has the person's symlink on the way");

        // Checkpoint: recorded with the parts above resolved, so it is taken, read and restored
        let folder = resolve_above(&typed);
        assert_eq!(folder, ssd.canonicalize().unwrap().join("Projects/proj"));
        let taken = snapshot(&blobs, &folder, 1 << 30).unwrap();
        assert_eq!(taken.manifest.files.len(), 1);
        std::fs::write(typed.join("authorized_keys"), "CHANGED\n").unwrap();
        assert!(matches!(current(&folder, "authorized_keys"), Ok(Some(_))));
        let root = crate::sandbox::safe_fs::Dir::open_strict(&folder).unwrap();
        let (restored, _deleted, failed) = apply(&blobs, &overwrite_plan(&folder, &taken), &BTreeSet::new(), &root);
        assert_eq!(restored, vec!["authorized_keys".to_string()], "{failed:?}");
        assert_eq!(std::fs::read_to_string(typed.join("authorized_keys")).unwrap(), "V0\n");

        // The folder swapped for a symlink after the checkpoint: refused, the other folder untouched
        let victim = d.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("authorized_keys"), "ORIGINAL\n").unwrap();
        std::fs::rename(&folder, folder.with_file_name("proj.moved")).unwrap();
        std::os::unix::fs::symlink(&victim, &folder).unwrap();
        assert!(crate::sandbox::safe_fs::Dir::open_strict(&folder).is_err());
        assert!(snapshot(&blobs, &folder, 1 << 30).is_err());
        assert_eq!(current(&folder, "authorized_keys"), Err("symlink, not followed".into()));
        std::fs::remove_file(&folder).unwrap();
        std::fs::rename(folder.with_file_name("proj.moved"), &folder).unwrap();

        // A part above it swapped for a symlink after the checkpoint: refused too
        std::fs::rename(&ssd, d.path().join("ssd.real")).unwrap();
        std::os::unix::fs::symlink(d.path().join("ssd.real"), &ssd).unwrap();
        assert!(crate::sandbox::safe_fs::Dir::open_strict(&folder).is_err());
        assert_eq!(std::fs::read_to_string(victim.join("authorized_keys")).unwrap(), "ORIGINAL\n");

        // The task folder itself a symlink is never resolved, and stays refused as on main
        assert_eq!(resolve_above(&home.join("Projects")), home.canonicalize().unwrap().join("Projects"));
        assert!(snapshot(&blobs, &resolve_above(&home.join("Projects")), 1 << 30).is_err());
    }

    /// #446 should-fix 2: the walk goes MAX_DEPTH folders down and no further. Each folder deeper
    /// is one skip naming it, and a file at the limit is still snapshotted.
    #[test]
    fn a_snapshot_goes_64_folders_deep_and_skips_what_is_deeper() {
        let d = tempfile::tempdir().unwrap();
        let (folder, blobs) = (d.path().join("proj"), d.path().join("blobs"));
        // One-letter names keep the deepest path under Windows' 260 characters
        let mut at_limit = folder.clone();
        for _ in 0..MAX_DEPTH {
            at_limit = at_limit.join("d");
        }
        let too_deep = at_limit.join("deeper");
        std::fs::create_dir_all(&too_deep).unwrap();
        std::fs::write(at_limit.join("at-the-limit.txt"), "kept").unwrap();
        std::fs::write(too_deep.join("lost.txt"), "skipped").unwrap();
        let t = snapshot(&blobs, &folder, 1 << 30).unwrap();
        let rel_limit = "d/".repeat(MAX_DEPTH);
        let files: Vec<&str> = t.manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(files, vec![format!("{rel_limit}at-the-limit.txt")]);
        let skip = t.manifest.skipped.iter().find(|s| s.path == format!("{rel_limit}deeper/")).expect("the deeper folder is named");
        assert_eq!(skip.why, "deeper than 64 folders");
        // A restore never acts on a path under it
        assert_eq!(not_vouched(&t.manifest, &format!("{rel_limit}deeper/lost.txt")).as_deref(), Some("deeper than 64 folders"));
    }

    /// #446 should-fix 2: the walk looks at no more than its entry budget, records one skip where
    /// it stopped, and a folder that fills the budget exactly isn't called cut.
    #[test]
    fn a_snapshot_stops_at_its_entry_limit_with_one_skip() {
        let d = tempfile::tempdir().unwrap();
        let (folder, blobs) = (d.path().join("proj"), d.path().join("blobs"));
        for sub in ["a", "b"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
            for i in 0..10 {
                std::fs::write(folder.join(format!("{sub}/f{i}")), format!("{sub}{i}")).unwrap();
            }
        }
        let open = || crate::sandbox::safe_fs::Dir::open_strict(&folder).unwrap();
        // 2 folders + 20 files = 22 entries; a budget of 12 reaches the top and one folder
        let t = snapshot_within(&blobs, open(), 1 << 30, MAX_DEPTH, 12).unwrap();
        assert_eq!(t.manifest.files.len(), 10, "{:?}", t.manifest.files);
        let stops: Vec<&Skip> = t.manifest.skipped.iter().filter(|s| s.why.ends_with(WALK_STOPPED)).collect();
        assert_eq!(stops.len(), 1, "{:?}", t.manifest.skipped);
        assert_eq!(stops[0].why, "walk stopped at the 12-entry limit");
        // Inside one wide folder: cut while listing it
        let t = snapshot_within(&blobs, open(), 1 << 30, MAX_DEPTH, 5).unwrap();
        assert!(t.manifest.files.len() <= 5);
        assert_eq!(t.manifest.skipped.iter().filter(|s| s.why.ends_with(WALK_STOPPED)).count(), 1, "{:?}", t.manifest.skipped);
        // Exactly the budget: everything, no stop
        let t = snapshot_within(&blobs, open(), 1 << 30, MAX_DEPTH, 22).unwrap();
        assert_eq!(t.manifest.files.len(), 20);
        assert!(t.manifest.skipped.is_empty(), "{:?}", t.manifest.skipped);
        assert_eq!(MAX_ENTRIES, 4 * MAX_FILES);
    }

    /// A walk that stopped early can't say a file it never reached was absent: when the run's
    /// before_run stopped short and its after_run didn't, the files the after_run newly saw look
    /// created by the run, and a restore must skip them, never delete them.
    #[test]
    fn a_restore_never_deletes_a_file_a_stopped_walk_did_not_reach() {
        let d = tempfile::tempdir().unwrap();
        let (folder, blobs) = (d.path().join("proj"), d.path().join("blobs"));
        for sub in ["a", "b"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
            for i in 0..10 {
                std::fs::write(folder.join(format!("{sub}/f{i}")), format!("{sub}{i}")).unwrap();
            }
        }
        let open = || crate::sandbox::safe_fs::Dir::open_strict(&folder).unwrap();
        let before = snapshot_within(&blobs, open(), 1 << 30, MAX_DEPTH, 12).unwrap().manifest;
        let after = snapshot_within(&blobs, open(), 1 << 30, MAX_DEPTH, 22).unwrap().manifest;
        let unseen: Vec<String> = changed(&before, &after);
        assert_eq!(unseen.len(), 10, "{unseen:?}");
        for p in &unseen {
            assert_eq!(not_vouched(&before, p).as_deref(), Some("walk stopped at the 12-entry limit"), "{p}");
        }
        // A complete walk vouches for an absent path, so a real delete is still planned
        assert_eq!(not_vouched(&after, "a/new-file"), None);
        // An unreadable or never-snapshotted folder covers what is under it
        let m = Manifest { files: vec![], skipped: vec![Skip { path: "x/".into(), why: "folder can't be read".into() }] };
        assert_eq!(not_vouched(&m, "x/y/z").as_deref(), Some("folder can't be read"));
        assert_eq!(not_vouched(&m, "xy"), None);
    }
}
