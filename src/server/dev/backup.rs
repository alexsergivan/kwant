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

/// An `ok` backup younger than this lets a step that needs one through.
pub const FRESH_SECS: i64 = 600;
/// The longest a backup command may run.
pub const TIMEOUT_SECS: u64 = 600;
/// How much of the command's output an error keeps.
const ERROR_CHARS: usize = 400;

static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Whether a backup is running in the space now (the card's `taking`).
pub fn is_running(space_id: &str) -> bool {
    RUNNING.lock().unwrap().as_ref().is_some_and(|s| s.contains(space_id))
}

fn claim(space_id: &str) -> bool {
    RUNNING.lock().unwrap().get_or_insert_with(HashSet::new).insert(space_id.to_string())
}

fn release(space_id: &str) {
    if let Some(s) = RUNNING.lock().unwrap().as_mut() {
        s.remove(space_id);
    }
}

#[derive(Debug)]
pub enum BackupError {
    NoCommand,
    Busy,
    Db(crate::db::DbError),
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
pub async fn run(state: &AppState, space: &SpaceId, kwant_id: Option<&str>) -> Result<DevBackupRow, BackupError> {
    let mode = state.db.dev_mode(space.as_str()).await.map_err(BackupError::Db)?;
    let Some(command) = mode.and_then(|m| m.backup_command).filter(|c| !c.trim().is_empty()) else {
        return Err(BackupError::NoCommand);
    };
    if !claim(space.as_str()) {
        return Err(BackupError::Busy);
    }
    let cwd = folder(state, space, kwant_id);
    let out = crate::tools::shell::execute_shell(&cwd, &json!({"command": command, "timeout_secs": TIMEOUT_SECS})).await;
    release(space.as_str());
    let taken_at = Utc::now();
    let (ok, path, bytes, error) = match out {
        Ok(v) => judge(&v, &cwd),
        Err(e) => (false, None, None, Some(e.to_string())),
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

/// `ok`, path, bytes and error from the command's result: exit 0, and when it printed a local
/// path, that file (or folder) exists and isn't empty.
pub fn judge(v: &Value, cwd: &Path) -> (bool, Option<String>, Option<i64>, Option<String>) {
    let stdout = v["stdout"].as_str().unwrap_or("");
    let stderr = v["stderr"].as_str().unwrap_or("");
    if v["success"] != true {
        let code = v["exit_code"].as_i64().unwrap_or(-1);
        let tail = tail_of(if stderr.trim().is_empty() { stdout } else { stderr });
        return (false, None, None, Some(format!("The backup command stopped with exit code {code}. {tail}").trim().to_string()));
    }
    match find_path(stdout, cwd).or_else(|| find_path(stderr, cwd)) {
        Some(Found::Local(p, 0)) => (false, Some(p), Some(0), Some("The backup file is empty".into())),
        Some(Found::Local(p, bytes)) => (true, Some(p), Some(bytes as i64), None),
        Some(Found::Missing(p)) => (false, Some(p), None, Some("The backup command named a file that isn't there".into())),
        Some(Found::Remote(p)) => (true, Some(p), printed_bytes(stdout), None),
        None => (true, None, printed_bytes(stdout), None),
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
    Local(String, u64),
    Missing(String),
    Remote(String),
}

const REMOTE: [&str; 6] = ["s3://", "gs://", "r2://", "b2://", "az://", "https://"];
const BACKUP_EXT: [&str; 12] = [".sql", ".gz", ".tgz", ".tar", ".zip", ".zst", ".dump", ".db", ".sqlite", ".sqlite3", ".bak", ".xz"];

/// The path a backup printed: the last word, from the end, that is a remote URL, an existing file
/// or folder, or a file name with a backup extension.
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
            if !looks {
                continue;
            }
            let p = expand(w, cwd);
            match std::fs::metadata(&p) {
                Ok(m) if m.is_file() => return Some(Found::Local(p.to_string_lossy().into(), m.len())),
                Ok(m) if m.is_dir() && line_names_backup(line) => return Some(Found::Local(p.to_string_lossy().into(), dir_bytes(&p, 0))),
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

fn dir_bytes(p: &Path, depth: usize) -> u64 {
    if depth > 6 {
        return 0;
    }
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten()
        .take(10_000)
        .map(|e| match e.metadata() {
            Ok(m) if m.is_file() => m.len(),
            Ok(m) if m.is_dir() => dir_bytes(&e.path(), depth + 1),
            _ => 0,
        })
        .sum()
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
    card_state(no_data, has_command, is_running(space_id), last.as_ref(), ok.as_ref(), Utc::now())
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
        // Nothing named: exit 0 is all there is to go on
        assert_eq!(judge(&ok("done"), &dir), (true, None, None, None));
        let (good, _, _, err) = judge(&json!({"success": false, "exit_code": 2, "stdout": "", "stderr": "pg_dump: error: connection to db.example.com failed"}), &dir);
        assert!(!good);
        assert!(err.unwrap().contains("exit code 2") );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_backup_runs_at_a_time_per_space() {
        assert!(claim("s-claim"));
        assert!(is_running("s-claim"));
        assert!(!claim("s-claim"));
        assert!(claim("s-other"));
        release("s-claim");
        release("s-other");
        assert!(!is_running("s-claim"));
    }
}
