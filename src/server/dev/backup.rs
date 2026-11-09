//! Backups for the production floor (D9). The person sets a backup command in Dev mode settings;
//! **Back up first** runs it in the bot's folder through the normal shell path (never gated by the
//! floor: the person wrote it) and records the path, size and time it printed, checked against
//! the file on disk when the path is local. A step that `needs_backup` is refused Allow until an
//! `ok` backup is at most [`FRESH_SECS`] old.

use crate::db::dev_mode::DevBackupRow;
use crate::db::spaces::SpaceId;
use crate::server::AppState;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// An `ok` backup younger than this lets a step that needs one through.
pub const FRESH_SECS: i64 = 600;
/// The longest a backup command may run.
pub const TIMEOUT_SECS: u64 = 600;
/// How much of the command's output an error keeps.
const ERROR_CHARS: usize = 400;
/// A local backup file may be dated this much before the command started (coarse file-system
/// clocks, a dump that opened its file a moment early).
pub const MTIME_SLACK: Duration = Duration::from_secs(5);
/// The error when exit 0 is all there is to go on.
pub const NOTHING_NAMED: &str = "kwant couldn't find a backup file in the output";

static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Whether a backup is running in the space now (the card's `taking`).
pub fn is_running(space_id: &str) -> bool {
    RUNNING.lock().unwrap().as_ref().is_some_and(|s| s.contains(space_id))
}

/// A space's running backup. Dropping it frees the space, however the run ends: done, failed,
/// panicked or cancelled.
struct Claim(String);

impl Claim {
    fn take(space_id: &str) -> Option<Claim> {
        RUNNING.lock().unwrap().get_or_insert_with(HashSet::new).insert(space_id.to_string()).then(|| Claim(space_id.to_string()))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(s) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            s.remove(&self.0);
        }
    }
}

#[derive(Debug)]
pub enum BackupError {
    NoCommand,
    Busy,
    Db(crate::db::DbError),
    /// The run's task ended without an answer (it panicked).
    Lost(String),
}

/// The folder the command runs in: the bot's own folder when a bot's card asked for it, else the
/// space's shared folder.
pub fn folder(state: &AppState, space: &SpaceId, kwant_id: Option<&str>) -> PathBuf {
    let dir = crate::db::spaces::space_data_dir(&state.config.data_dir, space);
    match kwant_id {
        Some(k) => crate::tools::bot_workspace(&dir, k),
        None => {
            let d = dir.join("workspace");
            let _ = std::fs::create_dir_all(&d);
            d
        }
    }
}

/// Runs the space's backup command and records the result. A failed run is recorded too (`ok`
/// false with the error), so the card can say why.
///
/// The run is its own task: if the person's request goes away (a closed tab, a dropped
/// connection), the dump still finishes and is recorded, and the space is freed when it ends.
pub async fn run(state: &AppState, space: &SpaceId, kwant_id: Option<&str>) -> Result<DevBackupRow, BackupError> {
    let (state, space, kwant_id) = (state.clone(), space.clone(), kwant_id.map(str::to_string));
    match tokio::spawn(async move { run_here(&state, &space, kwant_id.as_deref()).await }).await {
        Ok(r) => r,
        Err(e) => Err(BackupError::Lost(e.to_string())),
    }
}

async fn run_here(state: &AppState, space: &SpaceId, kwant_id: Option<&str>) -> Result<DevBackupRow, BackupError> {
    let mode = state.db.dev_mode(space.as_str()).await.map_err(BackupError::Db)?;
    let Some(command) = mode.and_then(|m| m.backup_command).filter(|c| !c.trim().is_empty()) else {
        return Err(BackupError::NoCommand);
    };
    let Some(_claim) = Claim::take(space.as_str()) else {
        return Err(BackupError::Busy);
    };
    let cwd = folder(state, space, kwant_id);
    let started = SystemTime::now();
    // D4 (C9c): the backup command runs bash on this computer as kwnt; never in a folder an
    // isolated bot used. The refusal is recorded like a failed backup.
    let refused = state.db.host_command_refusal(&cwd).await;
    let out = match &refused {
        Some(_) => None,
        None => Some(crate::tools::shell::execute_shell(&cwd, &json!({"command": command, "timeout_secs": TIMEOUT_SECS})).await),
    };
    let taken_at = super::now();
    let (ok, path, bytes, error) = match out {
        None => (false, None, None, refused),
        Some(Ok(v)) => judge(&v, &cwd, started),
        Some(Err(e)) => (false, None, None, Some(e.to_string())),
    };
    let row = DevBackupRow {
        id: format!("dvb_{}", uuid::Uuid::new_v4().simple()),
        space_id: space.as_str().into(),
        kwant_id: kwant_id.map(str::to_string),
        path,
        bytes,
        ok,
        error,
        taken_at,
    };
    state.db.insert_dev_backup(&row).await.map_err(BackupError::Db)?;
    tracing::info!(space = %space, ok = row.ok, "dev: backup taken");
    Ok(row)
}

/// `ok`, path, bytes and error from the command's result. A backup is `ok` only when the command
/// exits 0 and its output names either a remote target or a local file (or folder) that exists,
/// isn't empty, and was written after the command `started` (less [`MTIME_SLACK`]). Exit 0 with
/// nothing named is not a backup: `echo done` proves nothing.
pub fn judge(v: &Value, cwd: &Path, started: SystemTime) -> (bool, Option<String>, Option<i64>, Option<String>) {
    let stdout = v["stdout"].as_str().unwrap_or("");
    let stderr = v["stderr"].as_str().unwrap_or("");
    if v["success"] != true {
        let code = v["exit_code"].as_i64().unwrap_or(-1);
        let tail = tail_of(if stderr.trim().is_empty() { stdout } else { stderr });
        return (false, None, None, Some(format!("The backup command stopped with exit code {code}. {tail}").trim().to_string()));
    }
    match find_path(stdout, cwd).or_else(|| find_path(stderr, cwd)) {
        Some(Found::Local(p, 0, _)) => (false, Some(p), Some(0), Some("The backup file is empty".into())),
        Some(Found::Local(p, bytes, modified)) if !modified.is_some_and(|m| m + MTIME_SLACK >= started) => {
            (false, Some(p), Some(bytes as i64), Some("The backup file is older than this backup run, so it isn't the one the command just made".into()))
        }
        Some(Found::Local(p, bytes, _)) => (true, Some(p), Some(bytes as i64), None),
        Some(Found::Missing(p)) => (false, Some(p), None, Some("The backup command named a file that isn't there".into())),
        Some(Found::Remote(p)) => (true, Some(p), printed_bytes(stdout), None),
        None => (false, None, None, Some(NOTHING_NAMED.into())),
    }
}

fn tail_of(s: &str) -> String {
    let s = s.trim();
    let n = s.chars().count();
    if n <= ERROR_CHARS {
        s.to_string()
    } else {
        format!("…{}", s.chars().skip(n - ERROR_CHARS).collect::<String>())
    }
}

#[derive(Debug, PartialEq)]
enum Found {
    /// Path, bytes, and when it was last written (the newest file, for a folder).
    Local(String, u64, Option<SystemTime>),
    Missing(String),
    Remote(String),
}

const REMOTE: [&str; 6] = ["s3://", "gs://", "r2://", "b2://", "az://", "https://"];
const BACKUP_EXT: [&str; 12] = [".sql", ".gz", ".tgz", ".tar", ".zip", ".zst", ".dump", ".db", ".sqlite", ".sqlite3", ".bak", ".xz"];

/// The path a backup printed: the last word, from the end, that is a remote URL, an existing file
/// or folder, or a file name with a backup extension. A bare name (`backups`, `latest`) counts
/// when it exists in `cwd` and its line says it's the backup.
fn find_path(out: &str, cwd: &Path) -> Option<Found> {
    let mut missing = None;
    for line in out.lines().rev().take(200) {
        for word in line.split_whitespace().rev() {
            let w = word
                .trim_start_matches(['"', '\'', '`', '(', '[', '<'])
                .trim_end_matches(['"', '\'', '`', ',', ';', ')', ']', '>', '.', ':']);
            let w = w.split_once('=').map(|(_, v)| v).unwrap_or(w);
            if w.len() < 2 || w.len() > 1024 {
                continue;
            }
            if REMOTE.iter().any(|p| w.starts_with(p)) {
                return Some(Found::Remote(w.to_string()));
            }
            let looks = w.contains('/') || BACKUP_EXT.iter().any(|e| w.to_lowercase().ends_with(e));
            let bare = !looks && w != "." && w != ".." && line_names_backup(line);
            if !looks && !bare {
                continue;
            }
            let p = expand(w, cwd);
            match std::fs::metadata(&p) {
                Ok(m) if m.is_file() => return Some(Found::Local(p.to_string_lossy().into(), m.len(), m.modified().ok())),
                Ok(m) if m.is_dir() && line_names_backup(line) => {
                    let (bytes, newest) = dir_stats(&p, 0);
                    return Some(Found::Local(p.to_string_lossy().into(), bytes, newest));
                }
                Ok(_) => {}
                Err(_) if BACKUP_EXT.iter().any(|e| w.to_lowercase().ends_with(e)) && missing.is_none() => missing = Some(p.to_string_lossy().into()),
                Err(_) => {}
            }
        }
    }
    missing.map(Found::Missing)
}

fn line_names_backup(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("backup") || l.contains("saved") || l.contains("wrote") || l.contains("dump") || l.contains("snapshot")
}

fn expand(w: &str, cwd: &Path) -> PathBuf {
    if let Some(rest) = w.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let p = PathBuf::from(w);
    if p.is_absolute() {
        p
    } else {
        cwd.join(p)
    }
}

/// A folder's bytes and its newest file's write time.
fn dir_stats(p: &Path, depth: usize) -> (u64, Option<SystemTime>) {
    if depth > 6 {
        return (0, None);
    }
    let Ok(rd) = std::fs::read_dir(p) else { return (0, None) };
    let mut bytes = 0;
    let mut newest: Option<SystemTime> = None;
    for e in rd.flatten().take(10_000) {
        let (b, t) = match e.metadata() {
            Ok(m) if m.is_file() => (m.len(), m.modified().ok()),
            Ok(m) if m.is_dir() => dir_stats(&e.path(), depth + 1),
            _ => (0, None),
        };
        bytes += b;
        newest = newest.max(t);
    }
    (bytes, newest)
}

/// `12345 bytes`, `bytes=12345`, `size: 12345` in the output, for a remote backup.
fn printed_bytes(out: &str) -> Option<i64> {
    let words: Vec<&str> = out.split(|c: char| c.is_whitespace() || c == '=' || c == ':').filter(|w| !w.is_empty()).collect();
    for (i, w) in words.iter().enumerate() {
        let lw = w.to_lowercase();
        if lw == "bytes" && i > 0 {
            if let Ok(n) = words[i - 1].replace(',', "").parse::<i64>() {
                return Some(n);
            }
        }
        if matches!(lw.as_str(), "bytes" | "size") {
            if let Some(n) = words.get(i + 1).and_then(|x| x.replace(',', "").parse::<i64>().ok()) {
                return Some(n);
            }
        }
    }
    None
}

/// `POST /dev/backup`'s reply.
pub fn json(b: &DevBackupRow) -> Value {
    json!({"id": b.id, "ok": b.ok, "path": b.path, "bytes": b.bytes, "taken_at": b.taken_at, "error": b.error})
}

/// Whether an `ok` backup is fresh enough for a step that needs one.
pub fn is_fresh(ok: Option<&DevBackupRow>, now: DateTime<Utc>) -> bool {
    ok.is_some_and(|b| b.ok && (now - b.taken_at).num_seconds() <= FRESH_SECS && b.taken_at <= now + chrono::Duration::seconds(5))
}

/// The floor card's `backup` object (plan §3.5): `state` is `fresh`, `old`, `none`, `no_data`,
/// `no_command`, `taking` or `failed`.
pub fn card_state(no_data: bool, has_command: bool, taking: bool, last: Option<&DevBackupRow>, last_ok: Option<&DevBackupRow>, now: DateTime<Utc>) -> Value {
    let age = |b: &DevBackupRow| (now - b.taken_at).num_seconds().max(0);
    let shown = last_ok.or(last);
    let state = if no_data {
        "no_data"
    } else if taking {
        "taking"
    } else if !has_command {
        "no_command"
    } else if last.is_some_and(|b| !b.ok) && !is_fresh(last_ok, now) {
        "failed"
    } else if is_fresh(last_ok, now) {
        "fresh"
    } else if last_ok.is_some() {
        "old"
    } else {
        "none"
    };
    let error = if state == "failed" { last.and_then(|b| b.error.clone()) } else { None };
    json!({
        "state": state,
        "age_s": shown.filter(|_| state != "no_data").map(age),
        "bytes": last_ok.filter(|_| state != "no_data").and_then(|b| b.bytes),
        "path": last_ok.filter(|_| state != "no_data").and_then(|b| b.path.clone()),
        "error": error,
    })
}

/// The `backup` object for a space now.
pub async fn state_of(db: &crate::db::Database, space_id: &str, no_data: bool) -> Value {
    let has_command = db.dev_mode(space_id).await.ok().flatten().and_then(|m| m.backup_command).is_some_and(|c| !c.trim().is_empty());
    let (last, ok) = db.latest_dev_backups(space_id).await.unwrap_or((None, None));
    card_state(no_data, has_command, is_running(space_id), last.as_ref(), ok.as_ref(), super::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(ok: bool, secs_ago: i64, now: DateTime<Utc>) -> DevBackupRow {
        DevBackupRow {
            id: "dvb_1".into(),
            space_id: "s1".into(),
            kwant_id: None,
            path: Some("/srv/backups/app.sql.gz".into()),
            bytes: Some(1024),
            ok,
            error: (!ok).then(|| "pg_dump: connection refused".into()),
            taken_at: now - chrono::Duration::seconds(secs_ago),
        }
    }

    #[test]
    fn a_backup_is_fresh_for_ten_minutes() {
        let now = Utc::now();
        assert!(is_fresh(Some(&b(true, 0, now)), now));
        assert!(is_fresh(Some(&b(true, 600, now)), now));
        assert!(!is_fresh(Some(&b(true, 601, now)), now));
        assert!(!is_fresh(Some(&b(false, 10, now)), now), "a failed backup never counts");
        assert!(!is_fresh(None, now));
        assert!(!is_fresh(Some(&b(true, -3600, now)), now), "a backup dated in the future doesn't count");
    }

    #[test]
    fn the_card_has_all_seven_backup_states() {
        let now = Utc::now();
        let s = |no_data, cmd, taking, last: Option<DevBackupRow>, ok: Option<DevBackupRow>| card_state(no_data, cmd, taking, last.as_ref(), ok.as_ref(), now)["state"].clone();
        assert_eq!(s(true, false, false, None, None), "no_data");
        assert_eq!(s(false, false, false, None, None), "no_command");
        assert_eq!(s(false, true, true, None, None), "taking");
        assert_eq!(s(false, true, false, None, None), "none");
        assert_eq!(s(false, true, false, Some(b(true, 30, now)), Some(b(true, 30, now))), "fresh");
        assert_eq!(s(false, true, false, Some(b(true, 3600, now)), Some(b(true, 3600, now))), "old");
        assert_eq!(s(false, true, false, Some(b(false, 5, now)), Some(b(true, 3600, now))), "failed");
        // A failure after a fresh one keeps the fresh one usable
        assert_eq!(s(false, true, false, Some(b(false, 5, now)), Some(b(true, 60, now))), "fresh");
        let v = card_state(false, true, false, Some(&b(false, 5, now)), None, now);
        assert_eq!(v["error"], "pg_dump: connection refused");
        let v = card_state(false, true, false, Some(&b(true, 30, now)), Some(&b(true, 30, now)), now);
        assert_eq!(v["age_s"], 30);
        assert_eq!(v["bytes"], 1024);
        assert_eq!(v["path"], "/srv/backups/app.sql.gz");
    }

    #[test]
    fn the_output_names_the_backup_and_the_disk_confirms_it() {
        let dir = std::env::temp_dir().join(format!("kwnt-dev-backup-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(dir.join("backups")).unwrap();
        std::fs::write(dir.join("backups/app-2026-11-05.sql.gz"), vec![1u8; 2048]).unwrap();
        std::fs::write(dir.join("backups/empty.sql"), b"").unwrap();
        let ok = |out: &str| json!({"success": true, "exit_code": 0, "stdout": out, "stderr": ""});
        let started = SystemTime::now() - Duration::from_secs(1);
        let judge = |v: &Value, dir: &Path| super::judge(v, dir, started);
        let (good, path, bytes, err) = judge(&ok("dumping…\nBackup saved to backups/app-2026-11-05.sql.gz\n"), &dir);
        assert!(good, "{err:?}");
        assert!(path.unwrap().ends_with("backups/app-2026-11-05.sql.gz"));
        assert_eq!(bytes, Some(2048));
        let abs = dir.join("backups/app-2026-11-05.sql.gz");
        assert_eq!(judge(&ok(&format!("wrote \"{}\".", abs.display())), &dir).2, Some(2048));
        assert_eq!(judge(&ok(&format!("path={}", abs.display())), &dir).2, Some(2048));
        let (good, _, _, err) = judge(&ok("saved backups/empty.sql"), &dir);
        assert!(!good);
        assert_eq!(err.as_deref(), Some("The backup file is empty"));
        let (good, path, _, err) = judge(&ok("saved backups/missing.sql.gz"), &dir);
        assert!(!good && path.is_some());
        assert!(err.unwrap().contains("isn't there"));
        let (good, path, bytes, _) = judge(&ok("upload: ./x.gz to s3://demo-backups/app.sql.gz\n12345 bytes"), &dir);
        assert!(good);
        assert_eq!(path.as_deref(), Some("s3://demo-backups/app.sql.gz"));
        assert_eq!(bytes, Some(12345));
        // A whole folder, when the line says it's the backup
        let (good, _, bytes, _) = judge(&ok(&format!("Backup written to {}", dir.join("backups").display())), &dir);
        assert!(good);
        assert_eq!(bytes, Some(2048));
        // Nothing named is not a backup (F2)
        assert_eq!(judge(&ok("done"), &dir), (false, None, None, Some(NOTHING_NAMED.into())));
        let (good, _, _, err) = judge(&json!({"success": false, "exit_code": 2, "stdout": "", "stderr": "pg_dump: error: connection to db.example.com failed"}), &dir);
        assert!(!good);
        assert!(err.unwrap().contains("exit code 2") );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_backup_runs_at_a_time_per_space() {
        let a = Claim::take("s-claim").unwrap();
        assert!(is_running("s-claim"));
        assert!(Claim::take("s-claim").is_none());
        let b = Claim::take("s-other").unwrap();
        drop(a);
        drop(b);
        assert!(!is_running("s-claim"));
        assert!(!is_running("s-other"));
    }

    /// F1: a run that is cancelled midway (the request went away) still frees the space.
    #[tokio::test]
    async fn a_cancelled_run_frees_the_space() {
        let run = tokio::spawn(async {
            let _claim = Claim::take("s-cancel").unwrap();
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(is_running("s-cancel"));
        run.abort();
        let _ = run.await;
        assert!(!is_running("s-cancel"), "an aborted run must not hold the space");
        // And a panicking one
        let run = tokio::spawn(async {
            let _claim = Claim::take("s-cancel").unwrap();
            panic!("the dump crashed");
        });
        assert!(run.await.is_err());
        assert!(!is_running("s-cancel"));
    }

    /// F2: exit 0 with nothing named is never ok, whatever it printed.
    #[test]
    fn exit_zero_with_no_file_named_is_not_a_backup() {
        let dir = std::env::temp_dir().join(format!("kwnt-dev-backup-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = |out: &str| json!({"success": true, "exit_code": 0, "stdout": out, "stderr": ""});
        for out in ["", "done", "Backup complete!", "OK 12345 bytes"] {
            let (good, path, _, err) = judge(&ok(out), &dir, SystemTime::now());
            assert!(!good, "{out:?} must not count as a backup");
            assert_eq!(path, None);
            assert_eq!(err.as_deref(), Some(NOTHING_NAMED), "{out:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// F3: a file written before the command started is not this run's backup; a bare folder
    /// name is found in the run's folder.
    #[test]
    fn an_old_file_is_not_this_runs_backup_and_bare_names_resolve() {
        let dir = std::env::temp_dir().join(format!("kwnt-dev-backup-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(dir.join("snapshots")).unwrap();
        std::fs::write(dir.join("old.sql.gz"), vec![1u8; 512]).unwrap();
        std::fs::write(dir.join("snapshots/app.dump"), vec![1u8; 256]).unwrap();
        let ok = |out: &str| json!({"success": true, "exit_code": 0, "stdout": out, "stderr": ""});
        // Started a minute after the files were written: they're stale
        let later = SystemTime::now() + Duration::from_secs(60);
        let (good, path, bytes, err) = judge(&ok("Backup saved to old.sql.gz"), &dir, later);
        assert!(!good);
        assert!(path.unwrap().ends_with("old.sql.gz"));
        assert_eq!(bytes, Some(512));
        assert!(err.unwrap().contains("older than this backup run"));
        let (good, _, _, err) = judge(&ok("Backup written to snapshots"), &dir, later);
        assert!(!good, "a folder whose newest file is old is stale too");
        assert!(err.unwrap().contains("older"));
        // Started now (within the slack): fresh, and the bare folder name resolves against cwd
        let now = SystemTime::now();
        let (good, path, _, _) = judge(&ok("Backup saved to old.sql.gz"), &dir, now);
        assert!(good);
        assert!(path.unwrap().ends_with("old.sql.gz"));
        let (good, path, bytes, err) = judge(&ok("Backup written to snapshots"), &dir, now);
        assert!(good, "{err:?}");
        assert_eq!(path.as_deref(), Some(dir.join("snapshots").to_string_lossy().as_ref()));
        assert_eq!(bytes, Some(256));
        // A bare word on a line that doesn't say it's the backup is ignored
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::write(dir.join("notes/a.txt"), b"x").unwrap();
        assert_eq!(judge(&ok("updated notes"), &dir, now).3.as_deref(), Some(NOTHING_NAMED));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
