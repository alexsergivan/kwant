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
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
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
fn current(folder: &FsPath, rel: &str) -> Result<Option<String>, String> {
    if through_symlink(folder, rel) {
        return Err("symlink, not followed".into());
    }
    let p = folder.join(rel);
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("can't be read".into()),
    };
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    let mut f = std::fs::File::open(&p).map_err(|_| "can't be read".to_string())?;
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    std::io::copy(&mut f, &mut h).map_err(|_| "can't be read".to_string())?;
    Ok(Some(hex(&sha2::Digest::finalize(h))))
}

/// Streams `src` into the blob store, returning its sha256.
fn store_blob(blobs: &FsPath, src: &FsPath) -> std::io::Result<String> {
    let mut input = std::fs::File::open(src)?;
    let tmp = blobs.join(format!(".tmp-{}", new_id()));
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    let mut h = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = vec![0u8; 64 * 1024];
    let res = (|| -> std::io::Result<()> {
        loop {
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
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
    Ok(sha)
}

/// The commit the folder's git checkout is on, if it is one (read-only info).
fn git_head(folder: &FsPath) -> Option<String> {
    let git = folder.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else {
        return Some(head.chars().take(64).collect());
    };
    if let Ok(s) = std::fs::read_to_string(git.join(r)) {
        return Some(s.trim().chars().take(64).collect());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|l| l.strip_suffix(r).map(|sha| sha.trim().chars().take(64).collect()))
}

struct Taken {
    manifest: Manifest,
    bytes: u64,
    git_head: Option<String>,
}

/// Walks `folder` (never following symlinks) and stores each file's content as a blob.
/// A missing folder is an empty snapshot; an unreadable one is an error.
fn snapshot(blobs: &FsPath, folder: &FsPath, max_bytes: u64) -> Result<Taken, String> {
    private_dir(blobs).map_err(|e| format!("the checkpoint store can't be written: {e}"))?;
    let mut m = Manifest::default();
    match std::fs::symlink_metadata(folder) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Taken { manifest: m, bytes: 0, git_head: None }),
        Err(e) => return Err(format!("the folder can't be read: {e}")),
        Ok(meta) if !meta.is_dir() => return Err("the task's folder isn't a folder".into()),
        Ok(_) => {}
    }
    let mut candidates: Vec<(String, PathBuf, u64, u32)> = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(folder.to_path_buf(), String::new())];
    let mut first = true;
    while let Some((dir, prefix)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if first => return Err(format!("the folder can't be read: {e}")),
            Err(_) => {
                m.skipped.push(Skip { path: prefix.to_string(), why: "folder can't be read".into() });
                continue;
            }
        };
        first = false;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("{prefix}{name}");
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                m.skipped.push(Skip { path: rel, why: "can't be read".into() });
                continue;
            };
            let ft = meta.file_type();
            if ft.is_symlink() {
                m.skipped.push(Skip { path: rel, why: "symlink, not followed".into() });
            } else if ft.is_dir() {
                if SKIP_DIRS.contains(&name.as_str()) {
                    m.skipped.push(Skip { path: format!("{rel}/"), why: "not snapshotted".into() });
                } else {
                    stack.push((entry.path(), format!("{rel}/")));
                }
            } else if !ft.is_file() {
                m.skipped.push(Skip { path: rel, why: "not a regular file".into() });
            } else if meta.len() > FILE_MAX_BYTES {
                m.skipped.push(Skip { path: rel, why: "over 20 MB".into() });
            } else {
                candidates.push((rel, entry.path(), meta.len(), meta.permissions().mode() & 0o777));
            }
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
    let mut bytes = 0;
    for (rel, abs, len, mode) in candidates {
        match store_blob(blobs, &abs) {
            Ok(sha) => {
                bytes += len;
                m.files.push(Entry { path: rel, sha256: sha, bytes: len, mode });
            }
            Err(_) => m.skipped.push(Skip { path: rel, why: "can't be read".into() }),
        }
    }
    m.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Taken { manifest: m, bytes, git_head: git_head(folder) })
}

fn write_manifest(dir: &FsPath, id: &str, m: &Manifest) -> std::io::Result<(PathBuf, String)> {
    private_dir(dir)?;
    let body = serde_json::to_vec(m).map_err(std::io::Error::other)?;
    let path = dir.join(format!("{id}.json"));
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
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
                let _ = std::fs::set_permissions(&to, std::fs::Permissions::from_mode((e.mode & 0o700) | 0o600));
                n += 1;
            }
        }
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
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

fn read_runs(dir: &FsPath) -> Vec<Run> {
    std::fs::read_to_string(dir.join("runs.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append_run(dir: &FsPath, run: &Run) -> std::io::Result<()> {
    private_dir(dir)?;
    let mut f = std::fs::OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join("runs.jsonl"))?;
    writeln!(f, "{}", serde_json::to_string(run).map_err(std::io::Error::other)?)
}

// Taking checkpoints

/// Takes a checkpoint of `task`'s folder. Best-effort: a failure is stored as a checkpoint with
/// its `error` (not restorable) and a `checkpoint_failed` event, and never stops the run.
/// Run checkpoints (`before_run`, `after_run`) follow the `checkpoints` setting; manual and
/// `before_restore` ones are always taken.
pub async fn take(state: &AppState, task: &Task, kind: &str, claim: Option<&Claim>, created_by: &str) -> Option<Checkpoint> {
    if matches!(kind, "before_run" | "after_run") && !enabled(state).await {
        return None;
    }
    let folder = task_folder(state, task).await;
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
                let taken = snapshot(&root.join("blobs"), &folder, max_bytes)?;
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
    let skipped_in: BTreeMap<&str, &str> = manifest.skipped.iter().map(|s| (s.path.as_str(), s.why.as_str())).collect();
    let mut changes = Vec::new();
    let mut conflicts = Vec::new();
    let mut forced = BTreeMap::new();
    let mut hashes = Vec::new();
    let check = {
        let (folder, owned, entries, blobs) = (folder.clone(), owned.clone(), entries.clone(), blobs.clone());
        let skipped_in: BTreeMap<String, String> = skipped_in.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        tokio::task::spawn_blocking(move || {
            owned
                .into_iter()
                .map(|p| {
                    if !safe_rel(&p) {
                        return (p, Err("unsafe path".to_string()));
                    }
                    if let Some(why) = skipped_in.get(&p) {
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

/// Writes the plan's changes (plus the forced conflicts) into the folder; never through a symlink.
fn apply(blobs: &FsPath, p: &Plan, force: &BTreeSet<String>) -> (Vec<String>, Vec<String>, Vec<Value>) {
    let mut todo: Vec<(String, &'static str)> = p.changes.iter().map(|c| (c.path.clone(), c.action)).collect();
    todo.extend(p.forced.iter().filter(|(k, _)| force.contains(*k)).map(|(k, a)| (k.clone(), *a)));
    let (mut restored, mut deleted, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for (rel, action) in todo {
        if !safe_rel(&rel) || through_symlink(&p.folder, &rel) {
            failed.push(json!({"path": rel, "why": "symlink, not followed"}));
            continue;
        }
        let dest = p.folder.join(&rel);
        let r = if action == "delete" {
            std::fs::remove_file(&dest).map(|_| false)
        } else {
            let e = &p.entries[&rel];
            (|| -> std::io::Result<bool> {
                let parent = dest.parent().unwrap_or(&p.folder);
                std::fs::create_dir_all(parent)?;
                // The folders just created can't be symlinks, but one may have appeared meanwhile
                if through_symlink(&p.folder, &rel) {
                    return Err(std::io::Error::other("symlink"));
                }
                let tmp = parent.join(format!(".kwant-restore-{}", new_id()));
                std::fs::copy(blobs.join(&e.sha256), &tmp)?;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(if e.mode == 0 { 0o644 } else { e.mode }))?;
                std::fs::rename(&tmp, &dest).inspect_err(|_| {
                    let _ = std::fs::remove_file(&tmp);
                })?;
                Ok(true)
            })()
        };
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
    let Some(safety) = take(state, task, "before_restore", None, "person").await.filter(|s| !s.failed()) else {
        return Err(coded(StatusCode::INTERNAL_SERVER_ERROR, "checkpoint_failed", "the safety checkpoint couldn't be taken, so nothing was restored"));
    };
    let held = STORE.lock().await;
    let (blobs, plan) = (root(state).join("blobs"), p.clone());
    let (restored, deleted, failed) = tokio::task::spawn_blocking(move || apply(&blobs, &plan, &force)).await.map_err(internal)?;
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
        let mut out = write_back(&state, &task, &c, &p, b.force_paths.into_iter().collect()).await?;
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

    #[test]
    fn a_run_owns_what_it_created_changed_or_deleted() {
        let before = m(&[("same", "1"), ("edited", "1"), ("gone", "1")], &["big", "node_modules/"]);
        let after = m(&[("same", "1"), ("edited", "2"), ("new", "1")], &["big", "node_modules/", "huge"]);
        assert_eq!(changed(&before, &after), vec!["edited", "gone", "huge", "new"]);
    }
}
