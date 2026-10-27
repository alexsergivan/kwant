//! Outcome checks kwant runs itself: `command` (exit code 0, in the mission's folder), `http` (GET,
//! status and text), `file_exists`, `file_contains`, `file_sha256` (inside the mission's folder).
//! Each result is stored on the check and as `check_output` evidence; then the task is evaluated.

use super::{evaluate, task_folder, MAX_EXCERPT};
use crate::db::missions::{NewEvidence, OutcomeCheck, Task, KWANT_CHECK_KINDS};
use crate::server::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const MAX_COMMAND_SECS: u64 = 900;
const DEFAULT_COMMAND_SECS: u64 = 300;
const HTTP_TIMEOUT_SECS: u64 = 20;
/// Bytes of a page or file read for `contains` / `file_contains`.
const MAX_READ_BYTES: usize = 5 * 1024 * 1024;

/// File names a check or evidence never reads: saved passwords, keys and env files.
pub fn secret_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.contains("password")
        || name.contains("secret")
        || name == ".git-credentials"
        || name == ".netrc"
        || matches!(ext.as_str(), "pem" | "key" | "p12" | "pfx" | "kdbx" | "keychain")
        || path.components().any(|c| matches!(c.as_os_str().to_str(), Some(".ssh" | ".gnupg")))
}

/// `rel` resolved inside `folder` (symlinks followed), or why not.
pub fn inside(folder: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err("path is empty".into());
    }
    let Some(root) = crate::mcp::builtin_tools::canonical_folder(Some(folder)) else {
        return Err("the mission's folder doesn't exist".into());
    };
    if Path::new(rel).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("paths can't use ..".into());
    }
    if !crate::mcp::builtin_tools::is_inside(&root, rel) {
        return Err("that path is outside the mission's folder".into());
    }
    let p = Path::new(rel);
    let joined = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let real = joined.canonicalize().unwrap_or(joined);
    if !real.starts_with(&root) {
        return Err("that path is outside the mission's folder".into());
    }
    if secret_file(&real) {
        return Err("checks don't read password, key or env files".into());
    }
    Ok(real)
}

fn excerpt(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MAX_EXCERPT {
        return s.to_string();
    }
    // Keep the end: test runners print the verdict last
    let tail: String = s.chars().rev().take(MAX_EXCERPT - 2).collect::<Vec<_>>().into_iter().rev().collect();
    format!("…\n{tail}")
}

/// Runs one check; returns `(status, output)` with status "passed", "failed" or "error".
/// `person_ok`: the person added or approved it, so an `http` check may reach this computer or a
/// private network; otherwise the URL must resolve to public addresses only, and is fetched from them.
pub async fn run_spec(kind: &str, spec: &Value, folder: Option<&Path>, person_ok: bool) -> (String, String) {
    let s = |k: &str| spec[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    let need_folder = || folder.ok_or_else(|| "this task has no folder to check in".to_string());
    let r: Result<(bool, String), String> = match kind {
        "command" => match (s("command"), need_folder()) {
            (Some(cmd), Ok(dir)) => {
                let secs = spec["timeout_secs"].as_u64().unwrap_or(DEFAULT_COMMAND_SECS).clamp(1, MAX_COMMAND_SECS);
                match crate::tools::shell::execute_shell(dir, &json!({"command": cmd, "timeout_secs": secs})).await {
                    Ok(v) => {
                        let code = v["exit_code"].as_i64().unwrap_or(-1);
                        let out = format!("exit code {code}\n{}{}", v["stdout"].as_str().unwrap_or(""), v["stderr"].as_str().unwrap_or(""));
                        Ok((code == 0, out))
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            (None, _) => Err("spec.command is missing".into()),
            (_, Err(e)) => Err(e),
        },
        "http" => match s("url") {
            Some(url) => http(url, spec, person_ok).await,
            None => Err("spec.url is missing".into()),
        },
        "file_exists" | "file_contains" | "file_sha256" => match (s("path"), need_folder()) {
            (Some(rel), Ok(dir)) => file(kind, rel, spec, dir).await,
            (None, _) => Err("spec.path is missing".into()),
            (_, Err(e)) => Err(e),
        },
        other => Err(format!("kwant doesn't run {other} checks")),
    };
    match r {
        Ok((true, out)) => ("passed".into(), excerpt(&out)),
        Ok((false, out)) => ("failed".into(), excerpt(&out)),
        Err(e) => ("error".into(), excerpt(&e)),
    }
}

async fn http(url: &str, spec: &Value, person_ok: bool) -> Result<(bool, String), String> {
    let mut builder = reqwest::Client::builder();
    if !person_ok {
        let (host, addrs) = crate::mcp::fetch_risk::public_only(url).map_err(|why| format!("not fetched: {why}; the person can approve it"))?;
        // The name is fetched from the addresses just checked, not looked up again
        builder = builder.resolve_to_addrs(&host, &addrs);
    }
    let client = builder
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        // A redirect could lead to a private address the check wasn't approved for
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| send_error(url, &e))?;
    let status = resp.status().as_u16();
    let ok_status = match spec["expect_status"].as_u64() {
        Some(want) => u64::from(status) == want,
        None => (200..300).contains(&status),
    };
    let mut body = Vec::new();
    let mut stream = resp;
    while let Ok(Some(chunk)) = stream.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_READ_BYTES {
            break;
        }
    }
    let text = String::from_utf8_lossy(&body);
    let contains = spec["contains"].as_str().filter(|t| !t.is_empty());
    let has = contains.is_none_or(|t| text.contains(t));
    let mut out = format!("GET {url} → {status}");
    if let Some(t) = contains {
        out.push_str(&format!("; {} \"{t}\"", if has { "contains" } else { "does not contain" }));
    }
    Ok((ok_status && has, out))
}

/// Why a GET didn't get a response. A refused or dropped connection and a timeout say so in
/// words AutoFix classes as temporary; reqwest's own text ("error sending request") doesn't.
fn send_error(url: &str, e: &reqwest::Error) -> String {
    let why = if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "connection refused or unreachable"
    } else {
        return format!("GET {url} failed: {e}");
    };
    format!("GET {url} failed: network error, {why} ({e})")
}

async fn file(kind: &str, rel: &str, spec: &Value, dir: &Path) -> Result<(bool, String), String> {
    let path = inside(dir, rel)?;
    let meta = tokio::fs::metadata(&path).await;
    match kind {
        "file_exists" => Ok((meta.as_ref().is_ok_and(|m| m.is_file()), match &meta {
            Ok(m) => format!("{rel} exists ({} bytes)", m.len()),
            Err(_) => format!("{rel} doesn't exist"),
        })),
        _ => {
            let Ok(m) = meta else { return Ok((false, format!("{rel} doesn't exist"))) };
            if !m.is_file() {
                return Ok((false, format!("{rel} isn't a file")));
            }
            let bytes = tokio::fs::read(&path).await.map_err(|e| format!("can't read {rel}: {e}"))?;
            if kind == "file_sha256" {
                let got = hex(&Sha256::digest(&bytes));
                let want = spec["sha256"].as_str().unwrap_or("").trim().to_ascii_lowercase();
                Ok((got == want, format!("sha256 of {rel} is {got}")))
            } else {
                let want = spec["text"].as_str().unwrap_or("");
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_READ_BYTES)]);
                let has = !want.is_empty() && text.contains(want);
                Ok((has, format!("{rel} {} \"{want}\"", if has { "contains" } else { "does not contain" })))
            }
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs `check` if kwant runs its kind and it is approved; stores the result and its evidence.
pub async fn run_check(state: &AppState, task: &Task, check: &OutcomeCheck) -> OutcomeCheck {
    if !KWANT_CHECK_KINDS.contains(&check.kind.as_str()) || !check.approved {
        return check.clone();
    }
    if !state.db.start_check(&check.id).await.unwrap_or(false) {
        return state.db.get_check(&check.id).await.ok().flatten().unwrap_or_else(|| check.clone());
    }
    super::emit_task(state, task).await;
    let folder = task_folder(state, task).await;
    let spec = check.spec_value();
    // Only approved checks get here: a bot's command or private-address check is approved by the
    // person (check approval and mission approval are person-only routes). An http check the person
    // didn't add or approve is held to public addresses again now, pinned against DNS rebinding.
    let person_ok = check.requested_by == "person" || approved_by_person(state, check).await;
    let (status, output) = run_spec(&check.kind, &spec, folder.as_deref(), person_ok).await;
    // The excerpt reaches bots and the API: credentials in the output never get stored
    let output = super::autofix::redact(&output);
    let ev = state
        .db
        .add_evidence(&NewEvidence {
            task_id: task.id.clone(),
            kind: "check_output".into(),
            label: format!("{} check: {status}", check.kind),
            excerpt: Some(output.clone()),
            captured_by: "kwant".into(),
            run_id: Some(check.id.clone()),
            ..Default::default()
        })
        .await
        .ok();
    let _ = state.db.finish_check(&check.id, &status, Some(&output), "kwant", ev.as_ref().map(|e| e.id.as_str())).await;
    state.db.get_check(&check.id).await.ok().flatten().unwrap_or_else(|| check.clone())
}

/// Whether the person approved `check` (`POST /api/checks/:id/approve`, or the mission it came in).
async fn approved_by_person(state: &AppState, check: &OutcomeCheck) -> bool {
    let events = state.db.task_events(&check.task_id, 500).await.unwrap_or_default();
    if events.iter().any(|e| e.kind == "check_added" && e.actor == "person" && e.detail_value()["check_id"] == check.id.as_str()) {
        return true;
    }
    // A check planned with a mission is approved when the person approves the mission
    match state.db.get_task(&check.task_id).await {
        Ok(Some(t)) => match t.mission_id {
            Some(m) => state.db.get_mission(&m).await.ok().flatten().is_some_and(|m| m.approved_by.as_deref() == Some("person") && m.approved_at.is_some_and(|at| check.created_at <= at)),
            None => false,
        },
        _ => false,
    }
}

/// Runs the task's approved kwant checks (or just `only`), then evaluates the task. Returns the
/// checks after the run and the task.
pub async fn run_task_checks(state: &AppState, task_id: &str, only: Option<&str>) -> (Vec<OutcomeCheck>, Option<Task>) {
    let Ok(Some(task)) = state.db.get_task(task_id).await else { return (Vec::new(), None) };
    for c in state.db.list_checks(task_id).await.unwrap_or_default() {
        if only.is_some_and(|id| id != c.id) {
            continue;
        }
        run_check(state, &task, &c).await;
    }
    let task = evaluate(state, task_id).await;
    if let Some(t) = &task {
        super::emit_task(state, t).await;
    }
    (state.db.list_checks(task_id).await.unwrap_or_default(), task)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_escapes_are_refused() {
        let dir = std::env::temp_dir().join(format!("kwnt-mc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/out.txt"), "hello 0.1.4").unwrap();
        std::fs::write(dir.join(".env"), "KEY=x").unwrap();
        assert!(inside(&dir, "sub/out.txt").is_ok());
        assert!(inside(&dir, "../etc/passwd").is_err());
        assert!(inside(&dir, "/etc/passwd").is_err());
        assert!(inside(&dir, ".env").is_err());
        assert!(inside(&dir, "keys/server.pem").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
            assert!(inside(&dir, "link/hostname").is_err(), "a symlink out of the folder");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn file_and_command_checks() {
        let dir = std::env::temp_dir().join(format!("kwnt-mc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CHANGELOG.md"), "## 0.1.4").unwrap();
        let d = Some(dir.as_path());
        assert_eq!(run_spec("file_exists", &json!({"path": "CHANGELOG.md"}), d, true).await.0, "passed");
        assert_eq!(run_spec("file_exists", &json!({"path": "nope.md"}), d, true).await.0, "failed");
        assert_eq!(run_spec("file_contains", &json!({"path": "CHANGELOG.md", "text": "0.1.4"}), d, true).await.0, "passed");
        assert_eq!(run_spec("file_contains", &json!({"path": "CHANGELOG.md", "text": "0.2"}), d, true).await.0, "failed");
        let sha = hex(&Sha256::digest(b"## 0.1.4"));
        assert_eq!(run_spec("file_sha256", &json!({"path": "CHANGELOG.md", "sha256": sha}), d, true).await.0, "passed");
        assert_eq!(run_spec("file_exists", &json!({"path": "../x"}), d, true).await.0, "error");
        assert_eq!(run_spec("command", &json!({"command": "test -f CHANGELOG.md"}), d, true).await.0, "passed");
        let (status, out) = run_spec("command", &json!({"command": "echo boom; exit 3"}), d, true).await;
        assert_eq!(status, "failed");
        assert!(out.contains("exit code 3") && out.contains("boom"), "{out}");
        assert_eq!(run_spec("command", &json!({"command": "true"}), None, true).await.0, "error", "no folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
