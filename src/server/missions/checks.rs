//! Outcome checks kwant runs itself: `command` (exit code 0, in the mission's folder), `http` (GET,
//! status and text), `file_exists`, `file_contains`, `file_sha256` (inside the mission's folder).
//! Each result is stored on the check and as `check_output` evidence; then the task is evaluated.

use super::{evaluate, task_folder, MAX_EXCERPT};
use crate::db::missions::{NewEvidence, OutcomeCheck, Task, DEFAULT_RECHECK_MINS, DEFAULT_WAIT_HOURS, KWANT_CHECK_KINDS, WAITING_EXIT};
use chrono::{DateTime, Duration, Utc};
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
    resolve_inside(folder, rel).map(|(_, real)| real)
}

/// Opens `rel` after [`inside`]'s checks, walking from the folder's descriptor (S-8): in vm mode
/// the bot can swap the checked file for a symlink to a host file, or a FIFO, before the open;
/// then the open fails (`NotFound` when it's gone) instead of reading that file or blocking.
pub fn open_inside(folder: &Path, rel: &str) -> Result<std::io::Result<std::fs::File>, String> {
    let (root, real) = resolve_inside(folder, rel)?;
    Ok(crate::sandbox::safe_fs::open_checked(&root, &real))
}

/// The most a file check or evidence hash reads: the graph scan's limit, so what was attached
/// can be checked again later.
pub const HASH_MAX_BYTES: u64 = super::graph::SCAN_MAX_BYTES;

/// The sha256 and size of an open file, refused past [`HASH_MAX_BYTES`]. Blocking: call it off the
/// async runtime.
pub fn hash_open(file: std::fs::File) -> std::io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let n = crate::sandbox::safe_fs::copy_capped(file, &mut hasher, HASH_MAX_BYTES)?;
    Ok((hex(&hasher.finalize()), n))
}

/// The resolved folder and the resolved path of `rel` inside it.
fn resolve_inside(folder: &Path, rel: &str) -> Result<(PathBuf, PathBuf), String> {
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
    Ok((root, real))
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

/// What a check run saw beyond passed or failed, for a goal's typed predicate (§22): a command's
/// exit code and stdout, an http GET's status and body text (up to 5 MB).
#[derive(Debug, Default, Clone)]
pub struct Seen {
    pub exit_code: Option<i64>,
    pub stdout: Option<String>,
    pub http_status: Option<u16>,
    pub body: Option<String>,
    /// The body was longer than the read cap, so `body` is only its start.
    pub body_truncated: bool,
    /// Where a 3xx answer points: the host only (never the path or query, which may hold a token),
    /// or `None` inside it when the address stays on the checked host. Not followed.
    pub redirect: Option<Option<String>>,
    /// A `file_sha256` check's `(actual, expected)` hashes, both lowercase hex.
    pub sha256: Option<(String, String)>,
}

/// What one run of a check said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Said {
    Pass,
    Fail,
    /// "Not yet": a command's exit 75, or an http check's `waiting_status` / `waiting_contains`.
    Wait,
}

impl From<bool> for Said {
    fn from(ok: bool) -> Self {
        if ok {
            Said::Pass
        } else {
            Said::Fail
        }
    }
}

/// Runs one check; returns `(status, output)` with status "passed", "failed" or "error".
/// `person_ok`: the person added or approved it, so an `http` check may reach this computer or a
/// private network; otherwise the URL must resolve to public addresses only, and is fetched from them.
/// A "not yet" (exit 75, `waiting_status`, `waiting_contains`) counts as "failed" here: only
/// Mission Control's own runs ([`run_check`]) wait.
pub async fn run_spec(kind: &str, spec: &Value, folder: Option<&Path>, person_ok: bool) -> (String, String) {
    let (status, output, _) = run_spec_seen(kind, spec, folder, person_ok).await;
    (status, output)
}

/// [`run_spec`], also returning what the run saw.
pub async fn run_spec_seen(kind: &str, spec: &Value, folder: Option<&Path>, person_ok: bool) -> (String, String, Seen) {
    let (status, output, seen) = run_spec_env(kind, spec, folder, person_ok, &[]).await;
    (if status == "waiting" { "failed".into() } else { status }, output, seen)
}

/// [`run_spec_seen`] with extra environment for a command check, and status "waiting" for a
/// "not yet".
async fn run_spec_env(kind: &str, spec: &Value, folder: Option<&Path>, person_ok: bool, env: &[(String, String)]) -> (String, String, Seen) {
    let mut seen = Seen::default();
    let s = |k: &str| spec[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    let need_folder = || folder.ok_or_else(|| "this task has no folder to check in".to_string());
    let r: Result<(Said, String), String> = match kind {
        "command" => match (s("command"), need_folder()) {
            (Some(cmd), Ok(dir)) => {
                let secs = spec["timeout_secs"].as_u64().unwrap_or(DEFAULT_COMMAND_SECS).clamp(1, MAX_COMMAND_SECS);
                match crate::tools::shell::execute_shell_env(dir, &json!({"command": cmd, "timeout_secs": secs}), env).await {
                    Ok(v) => {
                        let code = v["exit_code"].as_i64().unwrap_or(-1);
                        let out = format!("exit code {code}\n{}{}", v["stdout"].as_str().unwrap_or(""), v["stderr"].as_str().unwrap_or(""));
                        seen.exit_code = Some(code);
                        seen.stdout = Some(v["stdout"].as_str().unwrap_or("").to_string());
                        let said = match code {
                            0 => Said::Pass,
                            WAITING_EXIT => Said::Wait,
                            _ => Said::Fail,
                        };
                        Ok((said, out))
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            (None, _) => Err("spec.command is missing".into()),
            (_, Err(e)) => Err(e),
        },
        "http" => match s("url") {
            Some(url) => http(url, spec, person_ok, &mut seen).await,
            None => Err("spec.url is missing".into()),
        },
        "file_exists" | "file_contains" | "file_sha256" => match (s("path"), need_folder()) {
            (Some(rel), Ok(dir)) => file(kind, rel, spec, dir, &mut seen).await.map(|(ok, out)| (Said::from(ok), out)),
            (None, _) => Err("spec.path is missing".into()),
            (_, Err(e)) => Err(e),
        },
        other => Err(format!("kwant doesn't run {other} checks")),
    };
    match r {
        Ok((Said::Pass, out)) => ("passed".into(), excerpt(&out), seen),
        Ok((Said::Fail, out)) => ("failed".into(), excerpt(&out), seen),
        Ok((Said::Wait, out)) => ("waiting".into(), excerpt(&out), seen),
        Err(e) => ("error".into(), excerpt(&e), seen),
    }
}

async fn http(url: &str, spec: &Value, person_ok: bool, seen: &mut Seen) -> Result<(Said, String), String> {
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
    if (300..400).contains(&status) {
        let from = resp.url().clone();
        let to = resp.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok()).and_then(|l| from.join(l).ok());
        seen.redirect = to.map(|u| u.host_str().filter(|h| Some(*h) != from.host_str()).map(str::to_string));
    }
    let mut body = Vec::new();
    let mut stream = resp;
    while let Ok(Some(chunk)) = stream.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_READ_BYTES {
            // Cut at the cap exactly (a chunk may run past it); it only counts as cut if more follows
            seen.body_truncated = body.len() > MAX_READ_BYTES || matches!(stream.chunk().await, Ok(Some(c)) if !c.is_empty());
            body.truncate(MAX_READ_BYTES);
            break;
        }
    }
    let text = String::from_utf8_lossy(&body);
    let contains = spec["contains"].as_str().filter(|t| !t.is_empty());
    let has = contains.is_none_or(|t| text.contains(t));
    // "Not yet" wins over pass and fail: a 202 is a 2xx, and a "pending" page can still match `contains`
    let wait_status = spec["waiting_status"].as_u64().is_some_and(|w| u64::from(status) == w);
    let wait_text = spec["waiting_contains"].as_str().filter(|t| !t.is_empty()).filter(|t| text.contains(t));
    let mut out = format!("GET {url} → {status}");
    if let Some(t) = contains {
        out.push_str(&format!("; {} \"{t}\"", if has { "contains" } else { "does not contain" }));
    }
    if wait_status {
        out.push_str("; not ready yet (waiting status)");
    }
    if let Some(t) = wait_text {
        out.push_str(&format!("; not ready yet (contains \"{t}\")"));
    }
    if let Some(to) = &seen.redirect {
        out.push_str(&format!(
            "; redirects to {}, which kwant doesn't follow",
            to.as_deref().unwrap_or("another address on the same site")
        ));
    }
    if seen.body_truncated {
        out.push_str("; the page is larger than 5 MB, so only its start was read");
    }
    seen.http_status = Some(status);
    seen.body = Some(text.into_owned());
    let said = if wait_status || wait_text.is_some() { Said::Wait } else { Said::from(ok_status && has) };
    Ok((said, out))
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

pub(crate) async fn file(kind: &str, rel: &str, spec: &Value, dir: &Path, seen: &mut Seen) -> Result<(bool, String), String> {
    let (dir, rel_s, kind_s) = (dir.to_path_buf(), rel.to_string(), kind.to_string());
    // Opened by walking from the folder and read from that handle only (S-8), off the runtime
    let read = tokio::task::spawn_blocking(move || -> Result<FileSeen, String> {
        let file = match open_inside(&dir, &rel_s)? {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileSeen::Missing),
            Err(_) => return Ok(FileSeen::NotAFile),
        };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(match kind_s.as_str() {
            "file_exists" => FileSeen::Exists(len),
            "file_sha256" => match hash_open(file) {
                Ok((sha, _)) => FileSeen::Hash(sha),
                Err(e) if crate::sandbox::safe_fs::is_too_large(&e) => FileSeen::TooLarge,
                Err(e) => return Err(format!("can't read {rel_s}: {e}")),
            },
            _ => {
                let mut head = Vec::new();
                std::io::Read::read_to_end(&mut std::io::Read::take(file, MAX_READ_BYTES as u64), &mut head)
                    .map_err(|e| format!("can't read {rel_s}: {e}"))?;
                FileSeen::Head(head)
            }
        })
    })
    .await
    .map_err(|_| format!("can't read {rel}"))??;
    match read {
        FileSeen::Missing => Ok((false, format!("{rel} doesn't exist"))),
        FileSeen::NotAFile => Ok((false, format!("{rel} isn't a file"))),
        FileSeen::Exists(n) => Ok((true, format!("{rel} exists ({n} bytes)"))),
        FileSeen::TooLarge => Ok((false, format!("{rel} is over {} MB, so it isn't hashed", HASH_MAX_BYTES / (1024 * 1024)))),
        FileSeen::Hash(got) => {
            let want = spec["sha256"].as_str().unwrap_or("").trim().to_ascii_lowercase();
            seen.sha256 = Some((got.clone(), want.clone()));
            Ok((got == want, format!("sha256 of {rel} is {got}")))
        }
        FileSeen::Head(bytes) => {
            let want = spec["text"].as_str().unwrap_or("");
            let text = String::from_utf8_lossy(&bytes);
            let has = !want.is_empty() && text.contains(want);
            Ok((has, format!("{rel} {} \"{want}\"", if has { "contains" } else { "does not contain" })))
        }
    }
}

/// What a file check saw through its one open handle.
enum FileSeen {
    Missing,
    NotAFile,
    Exists(u64),
    TooLarge,
    Hash(String),
    Head(Vec<u8>),
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The key an outside service dedupes on: sha256 of the check's id and the round, so every run
/// of a round sends the same key, and the round after a fail sends a new one.
pub fn idempotency_key(check_id: &str, round: i64) -> String {
    hex(&Sha256::digest(format!("{check_id}:{round}").as_bytes()))
}

/// The environment a command check runs with (`KWANT_TASK_ID` and friends).
fn check_env(task: &Task, check: &OutcomeCheck, run: i64, round: i64) -> Vec<(String, String)> {
    vec![
        ("KWANT_TASK_ID".into(), task.id.clone()),
        ("KWANT_TASK_TITLE".into(), task.title.clone()),
        ("KWANT_TASK_ACCEPTANCE".into(), task.acceptance.clone()),
        ("KWANT_CHECK_ID".into(), check.id.clone()),
        ("KWANT_CHECK_RUN".into(), run.to_string()),
        ("KWANT_CHECK_ROUND".into(), round.to_string()),
        ("KWANT_IDEMPOTENCY_KEY".into(), idempotency_key(&check.id, round)),
    ]
}

/// A spec's whole-number field, or its default (validation keeps it in range; clamped anyway).
fn spec_num(spec: &Value, key: &str, default: i64, range: std::ops::RangeInclusive<i64>) -> i64 {
    spec[key].as_i64().unwrap_or(default).clamp(*range.start(), *range.end())
}

/// What a check is called to the person: its command or URL, cut short.
pub fn check_label(c: &OutcomeCheck) -> String {
    let spec = c.spec_value();
    let what = match c.kind.as_str() {
        "command" => spec["command"].as_str(),
        "http" => spec["url"].as_str(),
        _ => spec["path"].as_str(),
    };
    let what = super::autofix::redact(what.unwrap_or(&c.kind));
    let what = what.trim();
    if what.chars().count() > 80 {
        format!("{}…", what.chars().take(79).collect::<String>())
    } else {
        what.to_string()
    }
}

/// Runs `check` if kwant runs its kind and it is approved; stores the result and its evidence.
pub async fn run_check(state: &AppState, task: &Task, check: &OutcomeCheck) -> OutcomeCheck {
    run_check_at(state, task, check, Utc::now()).await
}

/// [`run_check`] at `now`, which schedules a waiting check's next run and judges its deadline.
pub async fn run_check_at(state: &AppState, task: &Task, check: &OutcomeCheck, now: DateTime<Utc>) -> OutcomeCheck {
    if !KWANT_CHECK_KINDS.contains(&check.kind.as_str()) || !check.approved {
        return check.clone();
    }
    if !state.db.start_check(&check.id).await.unwrap_or(false) {
        return state.db.get_check(&check.id).await.ok().flatten().unwrap_or_else(|| check.clone());
    }
    // The round: how many times the task went to review (at least 1, for a run before the first)
    let round = state.db.review_rounds(&task.id).await.unwrap_or(0).max(1);
    let run = state.db.count_check_run(&check.id, round).await.unwrap_or(1);
    super::emit_task(state, task).await;
    let folder = task_folder(state, task).await;
    let spec = check.spec_value();
    // Only approved checks get here: a bot's command or private-address check is approved by the
    // person (check approval and mission approval are person-only routes). An http check the person
    // didn't add or approve is held to public addresses again now, pinned against DNS rebinding.
    let person_ok = check.requested_by == "person" || approved_by_person(state, check).await;
    // D4 (C9b): a command check runs bash on this computer as kwnt; never in a folder an
    // isolated bot used, where the command (`npm test`) runs whatever that bot left there
    let refused = match (check.kind.as_str(), folder.as_deref()) {
        ("command", Some(dir)) => state.db.host_command_refusal(dir).await,
        _ => None,
    };
    let env = check_env(task, check, run, round);
    let (status, output) = match refused {
        Some(why) => ("error".to_string(), why),
        None => {
            let (status, output, _) = run_spec_env(&check.kind, &spec, folder.as_deref(), person_ok, &env).await;
            (status, output)
        }
    };
    // The excerpt reaches bots and the API: credentials in the output never get stored
    let output = super::autofix::redact(&output);
    let output = if status == "waiting" { format!("run {run}: not ready yet\n{output}") } else { output };
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
    let ev_id = ev.as_ref().map(|e| e.id.as_str());
    if status != "waiting" {
        let _ = state.db.finish_check(&check.id, &status, Some(&output), "kwant", ev_id).await;
        return state.db.get_check(&check.id).await.ok().flatten().unwrap_or_else(|| check.clone());
    }
    // "Not yet": no failure, no AutoFix, no bot turn; the missions tick runs it again when due
    let wait_hours = spec_num(&spec, "wait_hours", DEFAULT_WAIT_HOURS, crate::db::missions::WAIT_HOURS);
    let recheck = spec_num(&spec, "recheck_mins", DEFAULT_RECHECK_MINS, crate::db::missions::RECHECK_MINS);
    // A later run keeps the round's deadline (the person may have moved it)
    let first = run <= 1 || check.wait_until.is_none() || check.wait_round != round;
    let deadline = if first { now + Duration::hours(wait_hours) } else { check.wait_until.unwrap_or(now) };
    if now >= deadline {
        let since = check.waiting_since.unwrap_or(now);
        let hours = ((deadline - since).num_minutes() as f64 / 60.0).round() as i64;
        let why = format!("{} didn't answer in {} h", check_label(check), hours.max(1));
        let out = format!("{why}. Wait longer, or verify by hand.\n{output}");
        let _ = state.db.time_out_check(&check.id, Some(&out), ev_id, now).await;
        // One failure row, class waited_too_long: AutoFix never retries it
        let text = format!("{} {}\n{}", check.kind, super::autofix::redact_value(&spec), out);
        super::autofix::record(state, &task.id, "check", "waited_too_long", &text, &why).await;
    } else {
        let next = (now + Duration::minutes(recheck)).min(deadline);
        let _ = state.db.wait_check(&check.id, Some(&output), ev_id, now, next, deadline).await;
    }
    state.db.get_check(&check.id).await.ok().flatten().unwrap_or_else(|| check.clone())
}

/// Set while [`spawn_due`]'s run is going, so ticks don't stack runs of slow checks.
static DUE_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// [`run_due`] at now, in the background, unless the last one is still going.
pub fn spawn_due(state: &AppState) {
    use std::sync::atomic::Ordering;
    if DUE_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                DUE_RUNNING.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        let _done = Done;
        run_due(&state, Utc::now()).await;
    });
}

/// Most waiting checks one tick runs; the rest are due on the next.
const DUE_PER_TICK: i64 = 8;

/// Runs the waiting checks due by `now` (tasks in review only), one after another, and evaluates
/// each task after. Returns how many ran. The missions tick calls it; tests pass their own `now`.
pub async fn run_due(state: &AppState, now: DateTime<Utc>) -> usize {
    let due = state.db.due_waiting_checks(now, DUE_PER_TICK).await.unwrap_or_default();
    let mut ran = 0;
    for c in due {
        let Ok(Some(task)) = state.db.get_task(&c.task_id).await else { continue };
        if task.state != "review" {
            continue;
        }
        run_check_at(state, &task, &c, now).await;
        ran += 1;
        if let Some(t) = evaluate(state, &task.id).await {
            super::emit_task(state, &t).await;
        }
    }
    ran
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

    #[tokio::test]
    async fn exit_75_waits_only_for_mission_control() {
        let dir = std::env::temp_dir().join(format!("kwnt-mc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = Some(dir.as_path());
        let spec = json!({"command": "echo queued; exit 75"});
        // The verifier, goals, Foundry and AutoFix read "not ready yet" as not passed
        assert_eq!(run_spec("command", &spec, d, true).await.0, "failed");
        let (status, out, _) = run_spec_env("command", &spec, d, true, &[]).await;
        assert_eq!(status, "waiting");
        assert!(out.contains("queued"), "{out}");
        let env = vec![("KWANT_CHECK_RUN".to_string(), "3".to_string())];
        let (status, out, _) = run_spec_env("command", &json!({"command": "echo run $KWANT_CHECK_RUN"}), d, true, &env).await;
        assert_eq!(status, "passed");
        assert!(out.contains("run 3"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_idempotency_key_is_per_check_and_round() {
        let k = idempotency_key("c1", 1);
        assert_eq!(k.len(), 64);
        assert_eq!(k, idempotency_key("c1", 1), "stable across runs of a round");
        assert_ne!(k, idempotency_key("c1", 2), "a new round, a new key");
        assert_ne!(k, idempotency_key("c2", 1));
        assert_eq!(k, hex(&Sha256::digest(b"c1:1")));
    }
}
