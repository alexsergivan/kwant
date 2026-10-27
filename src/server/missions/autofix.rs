//! Mission Control Phase B3: AutoFix. Every failure of a task is recorded with a class from
//! [`classify`] (deterministic, no model), its redacted output and a server-only diagnostics file.
//! With `autofix` on, a failure the task can still recover from parks the task in `failed` and
//! plans one retry (by class: wait and retry, retry with the diagnostics, continue where it
//! stopped), which moves it back to `ready` as [`Actor::AutoFix`] once its time has come and the
//! task's own attempts and recovery budget allow. `suggest` plans the same retry but waits for the
//! person. Permanent changes (a routine's instructions or schedule, a check's spec, a task's
//! acceptance or max attempts) are only ever proposed; the person approves them, and they are
//! applied once. Contract: docs/mission-control.md §13.

use super::routes::{bad, conflict, internal, load_task, not_found};
use super::{emit_task, transition, Actor};
use crate::db::autofix::{Attempt, ChangeWrite, Failure, NewAttempt, NewFailure, WorkflowChange, TARGETS};
use crate::db::missions::{Task, TaskChange};
use crate::server::AppState;
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SETTING: &str = "autofix";
pub const MODES: [&str; 3] = ["off", "suggest", "on"];
pub const PCT_SETTING: &str = "recovery_budget_pct";
pub const DEFAULT_PCT: i64 = 25;
pub const PCT_RANGE: (i64, i64) = (0, 100);
/// Waits before the 1st, 2nd and later retries of a transient failure.
pub const BACKOFF_MINUTES: [i64; 3] = [1, 5, 25];
/// Largest diagnostics file kept per failure.
const DIAGNOSTICS_MAX: usize = 256 * 1024;

pub async fn mode(state: &AppState) -> &'static str {
    match state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim) {
        Some("on") => "on",
        Some("suggest") => "suggest",
        _ => "off",
    }
}

pub async fn recovery_pct(state: &AppState) -> i64 {
    super::int_setting(state, PCT_SETTING, PCT_RANGE, DEFAULT_PCT).await
}

pub fn settings_error(mode: Option<&str>, pct: Option<i64>) -> Option<String> {
    if let Some(m) = mode.filter(|m| !MODES.contains(m)) {
        return Some(format!("autofix must be off, suggest or on (got {m})"));
    }
    if let Some(n) = pct.filter(|n| !(PCT_RANGE.0..=PCT_RANGE.1).contains(n)) {
        return Some(format!("recovery_budget_pct must be {}–{} (got {n})", PCT_RANGE.0, PCT_RANGE.1));
    }
    None
}

// Classification

const AUTH: [&str; 16] = [
    "not logged in",
    "not signed in",
    "/login",
    "auth login",
    "unauthorized",
    "invalid api key",
    "invalid x-api-key",
    "api key rejected",
    "authentication_error",
    "token has expired",
    "login has expired",
    // kwant's own messages when no account can take the turn (profile_turn.rs, drivers/engines.rs)
    "signed out",
    "key was rejected",
    "no account can run bots",
    "no other account is on",
    "no custom api account is on",
];

// A usage limit or empty balance: a retry before it resets only burns an attempt
const QUOTA: [&str; 13] = [
    "usage limit",
    "hit your limit",
    "reached your limit",
    "hour limit reached",
    "weekly limit reached",
    "out of credits",
    "insufficient_quota",
    "exceeded your current quota",
    "usage_limit_exceeded",
    "credit balance is too low",
    "billing_error",
    // kwant's own messages when an account ran out (profile_turn.rs, drivers/engines.rs)
    "out of usage",
    "quota exceeded",
];

const TRANSIENT: [&str; 18] = [
    "rate limit",
    "rate-limit",
    "rate_limit",
    "overloaded",
    "too many requests",
    "timed out",
    "timeout",
    "connection refused",
    "connection reset",
    "connection closed",
    "network",
    "temporarily unavailable",
    "service unavailable",
    "bad gateway",
    "gateway timeout",
    "internal server error",
    "econnreset",
    "etimedout",
];

const TRANSIENT_CODES: [&str; 6] = ["429", "500", "502", "503", "504", "529"];

/// Whether `code` appears in `text` as a number of its own (not inside a longer one).
fn has_code(text: &str, code: &str) -> bool {
    text.match_indices(code).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + code.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

fn is_auth(t: &str) -> bool {
    AUTH.iter().any(|p| t.contains(p)) || has_code(t, "401")
}

fn is_quota(t: &str) -> bool {
    QUOTA.iter().any(|p| t.contains(p)) || has_code(t, "402")
}

fn is_transient(t: &str) -> bool {
    TRANSIENT.iter().any(|p| t.contains(p)) || TRANSIENT_CODES.iter().any(|c| has_code(t, c))
}

/// The class of a failure, from where it came (`source`), how it ended (`status`: `failed` or
/// `error` for a check; `error`, `stalled`, `denied` or `needs_input` for a turn) and its text.
pub fn classify(source: &str, status: &str, text: &str) -> &'static str {
    let t = text.to_lowercase();
    match (source, status) {
        ("check", "failed") if t.contains("exit code 127") || t.contains("command not found") => "bad_spec",
        ("check", "failed") => "check_failed",
        ("check", _) if is_transient(&t) => "transient",
        ("check", _) => "bad_spec",
        ("lease", _) => "stalled",
        ("budget", _) => "budget",
        ("turn", "stalled") => "stalled",
        ("turn", "denied") => "denied",
        ("turn", "needs_input") => "needs_input",
        // quota and auth before transient: "429 usage limit reached" won't pass in a minute, and
        // "401" next to "Reconnecting… timed out" is still a login problem
        (_, _) if is_quota(&t) => "quota",
        (_, _) if is_auth(&t) => "auth",
        (_, _) if is_transient(&t) => "transient",
        _ => "unknown",
    }
}

/// The strategy for a class (`autofix=on`); `unknown` is retried once per task.
fn strategy_for(class: &str) -> &'static str {
    match class {
        "transient" => "wait_and_retry",
        "stalled" => "retry_same",
        "check_failed" => "retry_with_diagnostics",
        "bad_spec" => "propose_change",
        "unknown" => "retry_same",
        _ => "none",
    }
}

/// What the person must do when AutoFix stopped short of the retry budget.
pub const RECOVERY_BUDGET_FIX: &str = "Retry budget used up: raise it or retry by hand.";

/// What the person must do for a failure AutoFix never retries. It never repeats the failure's
/// summary: the task already shows that next to it.
fn fix_text(class: &str) -> Option<&'static str> {
    match class {
        "auth" => Some("Sign in to the bot's engine again, then retry."),
        "quota" => Some("Usage limit reached: wait for it to reset or switch account, then retry."),
        "budget" => Some("Raise the task's budget, then retry it."),
        "denied" => Some("An action was denied during the run. Decide whether it may run, then retry."),
        _ => None,
    }
}

// Redaction

const MASK: &str = "[redacted]";

/// Masks anything that looks like a key, token or password, line by line, and is safe to run
/// twice. Three rules, each kept narrow so that command names, paths and ids stay readable:
/// a labelled value of any length (`password=x`, `--token x`, `Authorization: Basic x`,
/// `user:pass@host`, plus `key=x`, `--key x` and a password flag of a tool that takes one,
/// [`mask_flags`]); a word with a known key prefix (`sk-`, `ghp_`…); and a bare word that reads
/// as random ([`looks_random`]). A short bare password with no label can't be told from a word
/// and is not masked.
pub fn redact(text: &str) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || "-_.=/+".contains(c);
    text.lines()
        .map(|line| {
            let line = mask_flags(&crate::profiles::classify::mask_credentials(line));
            let mut out = String::with_capacity(line.len());
            let mut rest = line.as_str();
            while let Some(start) = rest.find(word) {
                out.push_str(&rest[..start]);
                let tail = &rest[start..];
                let end = tail.find(|c: char| !word(c)).unwrap_or(tail.len());
                let w = &tail[..end];
                let prefixed = ["sk-", "xai-", "sess-", "ghp_", "gho_", "github_pat_", "xox", "AKIA"].iter().any(|p| w.starts_with(p)) && w.len() > 8;
                out.push_str(if prefixed || bare_secret(w) { MASK } else { w });
                rest = &tail[end..];
            }
            out.push_str(rest);
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Tools whose password flag is a single letter: `mysql -phunter2`, `sshpass -p hunter2`,
/// `redis-cli -a hunter2`. Elsewhere `-p` is a port or "parents" and stays readable.
const PASSWORD_FLAGS: [(&str, &str); 7] = [
    ("mysql", "-p"),
    ("mysqldump", "-p"),
    ("mysqladmin", "-p"),
    ("mariadb", "-p"),
    ("mariadb-dump", "-p"),
    ("sshpass", "-p"),
    ("redis-cli", "-a"),
];

/// The values [`mask_credentials`](crate::profiles::classify::mask_credentials) leaves: `key=x`,
/// `"key": "x"`, `--key x` / `--key=x` (and `--…-key`), and the password flag of a tool in
/// [`PASSWORD_FLAGS`] used on this line, glued (`-phunter2`) or not. A value already masked is
/// left as it is.
fn mask_flags(line: &str) -> String {
    let tokens: Vec<&str> = line.split(' ').collect();
    let program = |t: &str| t.rsplit('/').next().unwrap_or(t).to_string();
    let flags: Vec<&str> = PASSWORD_FLAGS.iter().filter(|(tool, _)| tokens.iter().any(|t| program(t) == *tool)).map(|(_, f)| *f).collect();
    let is_key_flag = |t: &str| t.starts_with("--") && (t == "--key" || t.ends_with("-key")) && !t.starts_with("--no-");
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut hide_next = false;
    for t in &tokens {
        if hide_next && !t.is_empty() && !t.starts_with('-') {
            out.push(mask_word(t));
            hide_next = false;
            continue;
        }
        hide_next = false;
        if is_key_flag(t) || flags.contains(t) {
            hide_next = true;
            out.push((*t).to_string());
            continue;
        }
        if let Some((name, value)) = t.split_once('=') {
            if !value.is_empty() && (name.eq_ignore_ascii_case("key") || is_key_flag(name)) {
                out.push(format!("{name}={}", mask_word(value)));
                continue;
            }
        }
        if let Some(f) = flags.iter().find(|f| t.len() > f.len() && t.starts_with(**f) && !t[f.len()..].starts_with(MASK)) {
            out.push(format!("{f}{MASK}"));
            continue;
        }
        out.push((*t).to_string());
    }
    let line = out.join(" ");
    // JSON: `"key": "x"`
    let mut res = String::with_capacity(line.len());
    let mut rest = line.as_str();
    while let Some(at) = rest.find("\"key\"") {
        let after = &rest[at + 5..];
        let gap = after.len() - after.trim_start().len();
        let after_colon = after.trim_start().strip_prefix(':').map(str::trim_start);
        match after_colon.and_then(|v| v.strip_prefix('"')).and_then(|v| v.find('"').map(|e| (v, e))) {
            Some((v, e)) if e > 0 && &v[..e] != MASK => {
                let value_at = rest.len() - v.len();
                res.push_str(&rest[..value_at]);
                res.push_str(MASK);
                rest = &rest[value_at + e..];
            }
            _ => {
                res.push_str(&rest[..at + 5 + gap]);
                rest = &rest[at + 5 + gap..];
            }
        }
    }
    res.push_str(rest);
    res
}

/// A flag's value masked, keeping a closing quote or bracket it was written with.
fn mask_word(t: &str) -> String {
    let core = t.trim_matches(|c| "\"'".contains(c));
    if core.is_empty() || core.starts_with(MASK) {
        return t.to_string();
    }
    let lead = &t[..t.len() - t.trim_start_matches(|c| "\"'".contains(c)).len()];
    let trail = &t[t.trim_end_matches(|c| "\"'".contains(c)).len()..];
    format!("{lead}{MASK}{trail}")
}

/// Whether a bare word reads as a secret: not a URL or absolute path, and (for a relative path
/// like `./scripts/x.sh`, each part on its own) [`looks_random`]. Base64 (`+`, a trailing `=`) is
/// judged whole, since its `/` is not a folder.
fn bare_secret(w: &str) -> bool {
    if w.starts_with('/') || w.starts_with("http") || w.starts_with('~') {
        return false;
    }
    if w.contains('+') || w.ends_with('=') || !w.contains('/') {
        return looks_random(w);
    }
    w.split('/').any(looks_random)
}

/// At least 20 letters and digits that break into short pieces, as generated keys do
/// (`Zq8VwK3mT9pL2rX7nB4c`, a hex key), while names built from words and a number
/// (`nonexistent_cmd_xyz_ev10`, `MyServiceV2HealthCheck2026`) break into long ones. A piece ends
/// where digits meet letters or a lower-case letter meets an upper-case one; a capital followed
/// by lower case letters is one piece (`Health`). A UUID is an id, not a secret.
fn looks_random(w: &str) -> bool {
    if is_uuid(w) {
        return false;
    }
    let class = |c: char| if c.is_ascii_digit() { 0 } else if c.is_ascii_uppercase() { 1 } else { 2 };
    let chars: Vec<char> = w.chars().filter(char::is_ascii_alphanumeric).collect();
    let n = chars.len();
    if n < 20 || !chars.iter().any(char::is_ascii_digit) || !chars.iter().any(char::is_ascii_alphabetic) {
        return false;
    }
    let pieces = 1 + chars.windows(2).filter(|p| class(p[0]) != class(p[1]) && !(class(p[0]) == 1 && class(p[1]) == 2)).count();
    pieces >= 5 && pieces * 3 > n
}

fn is_uuid(w: &str) -> bool {
    let parts: Vec<&str> = w.split('-').collect();
    parts.len() == 5
        && parts.iter().zip([8, 4, 4, 4, 12]).all(|(p, len)| p.len() == len && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// [`redact`] over every string in `v` (a check's spec, a change's before/after).
pub fn redact_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact(s)),
        Value::Array(a) => Value::Array(a.iter().map(redact_value).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), redact_value(v))).collect()),
        other => other.clone(),
    }
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let s: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{s}…")
}

fn diagnostics_root(state: &AppState) -> std::path::PathBuf {
    state.config.data_dir.join("autofix")
}

/// Writes the redacted diagnostics of a failure, 0600 in a 0700 folder. Server-only.
fn write_diagnostics(state: &AppState, task_id: &str, name: &str, text: &str) -> Option<String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = diagnostics_root(state).join(task_id);
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).ok()?;
    let path = dir.join(format!("{name}.log"));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
    let body: String = text.chars().take(DIAGNOSTICS_MAX).collect();
    std::io::Write::write_all(&mut f, body.as_bytes()).ok()?;
    Some(path.to_string_lossy().into_owned())
}

// Recording and planning

/// Mission and person tasks are retried in place; routine runs as a new run (§4.4).
fn retried_in_place(task: &Task) -> bool {
    matches!(task.source_kind.as_str(), "mission" | "person")
}

/// Whether a failure of `task` with attempts left parks it in `failed` (for AutoFix) instead of
/// sending it straight back to `ready` (Phase A). `attempt` is whether this failure uses one.
pub async fn parks(state: &AppState, task: &Task, attempt: bool) -> bool {
    retried_in_place(task) && mode(state).await != "off" && task.attempts + i64::from(attempt) < task.max_attempts
}

/// A failed task waiting for AutoFix: the board shows no "failed" chat line for it.
pub async fn parked(state: &AppState, task: &Task) -> bool {
    task.state == "failed" && retried_in_place(task) && mode(state).await != "off" && task.attempts < task.max_attempts
}

/// Attempts the task has used, counting a routine run's own run.
fn used(task: &Task) -> i64 {
    if retried_in_place(task) {
        task.attempts
    } else {
        task.attempts + 1
    }
}

/// The recovery budget: `recovery_budget_pct` of the task's budget.
pub fn recovery_budget(task: &Task, pct: i64) -> i64 {
    task.budget_tokens.saturating_mul(pct) / 100
}

/// Why another attempt can't start, as an API code: `no_attempts_left`, `over_recovery_budget`,
/// `not_retryable`.
async fn limit(state: &AppState, task: &Task) -> Option<&'static str> {
    if task.state == "blocked" {
        return Some("not_retryable");
    }
    if used(task) >= task.max_attempts {
        return Some("no_attempts_left");
    }
    let estimate = task.spent_tokens / task.attempts.max(1);
    let recovery = recovery_budget(task, recovery_pct(state).await);
    if task.recovery_spent_tokens + estimate > recovery {
        return Some("over_recovery_budget");
    }
    None
}

fn limit_reason(code: &str) -> &'static str {
    match code {
        "no_attempts_left" => "no_attempts_left: the task used all its attempts",
        "over_recovery_budget" => "over_recovery_budget: another attempt would go past the recovery budget",
        _ => "not_retryable: AutoFix never moves this task",
    }
}

fn key(task_id: &str, failure_id: &str, strategy: &str) -> String {
    let d = Sha256::digest(format!("{task_id}:{failure_id}:{strategy}").as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Records a failure of `task` (already moved to where Phase A or [`parks`] put it), then plans
/// AutoFix's answer. `text` is the raw output; only its redacted form is stored.
pub async fn record(state: &AppState, task_id: &str, source: &str, status: &str, text: &str, summary: &str) -> Option<Failure> {
    let task = state.db.get_task(task_id).await.ok().flatten()?;
    let class = classify(source, status, text);
    let clean = redact(text);
    let excerpt = (!clean.trim().is_empty()).then(|| cut(clean.trim(), super::MAX_EXCERPT));
    let summary = cut(&redact(summary), 200);
    let name = uuid::Uuid::new_v4().to_string();
    let diagnostics_path = (!clean.trim().is_empty()).then(|| write_diagnostics(state, &task.id, &name, &clean)).flatten();
    let checkpoint_id = state
        .db
        .checkpoints_of(&task.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|c| c.kind == "after_run" && !c.failed())
        .map(|c| c.id);
    let new = NewFailure { task: &task, source, class, summary, excerpt, diagnostics_path, checkpoint_id };
    let failure = match state.db.insert_failure(&new).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(task_id = %task.id, error = %e, "could not record a task failure");
            return None;
        }
    };
    let task = state.db.get_task(task_id).await.ok().flatten().unwrap_or(task);
    if mode(state).await != "off" {
        plan(state, &task, &failure).await;
    }
    emit_task(state, &task).await;
    Some(failure)
}

/// Plans AutoFix's answer to `failure`: one attempt (keyed, so a repeat plans nothing), a
/// proposal, or nothing for a class only the person can fix.
async fn plan(state: &AppState, task: &Task, failure: &Failure) -> Option<Attempt> {
    if failure.class == "needs_input" || task.source_kind == "delegation" {
        return None;
    }
    let mut strategy = strategy_for(&failure.class);
    if failure.class == "stalled" && failure.checkpoint_id.is_some() {
        strategy = "retry_from_checkpoint";
    }
    if failure.class == "unknown" {
        let earlier = state.db.attempts_of(&task.id).await.unwrap_or_default();
        let fails = state.db.failures_of(&task.id).await.unwrap_or_default();
        let unknown_retried = earlier.iter().any(|a| a.strategy == "retry_same" && fails.iter().any(|f| f.id == a.failure_id && f.class == "unknown"));
        if unknown_retried {
            strategy = "none";
        }
    }
    let k = key(&task.id, &failure.id, strategy);
    let new = |status, not_before, reason: String| NewAttempt { task, failure_id: &failure.id, strategy, idempotency_key: k.clone(), status, not_before, reason };
    if strategy == "none" {
        // The code first, as for the other skips, so the attempt doesn't repeat the Needs-you text
        let why = fix_text(&failure.class).map_or_else(|| "AutoFix retried this once already".into(), |fix| format!("not_retryable: {fix}"));
        return insert(state, &new("skipped", None, why)).await;
    }
    if strategy == "propose_change" {
        propose_check_change(state, task, failure).await;
        return insert(state, &new("suggested", None, "a check that can't run: allow a change to it first".into())).await;
    }
    if let Some(code) = limit(state, task).await {
        return insert(state, &new("skipped", None, limit_reason(code).into())).await;
    }
    let wait = if strategy == "wait_and_retry" {
        let n = state.db.count_failures(&task.id, "transient", &failure.id).await.unwrap_or(0);
        Some(failure.created_at + Duration::minutes(BACKOFF_MINUTES[usize::try_from(n).unwrap_or(2).min(2)]))
    } else {
        None
    };
    let reason = match strategy {
        "wait_and_retry" => "a temporary error: wait, then run it again".to_string(),
        "retry_with_diagnostics" => "a check failed: run it again with the check's output".into(),
        "retry_from_checkpoint" => "the run stopped: continue from where it left off".into(),
        _ => "run it again".into(),
    };
    let status = if mode(state).await == "on" { "scheduled" } else { "suggested" };
    let a = insert(state, &new(status, wait, reason)).await?;
    if a.status == "scheduled" {
        soon(state, &a);
    }
    Some(a)
}

async fn insert(state: &AppState, a: &NewAttempt<'_>) -> Option<Attempt> {
    match state.db.insert_attempt(a).await {
        Ok(a) => a,
        Err(e) => {
            tracing::info!(task_id = %a.task.id, error = %e, "an autofix attempt wasn't planned");
            None
        }
    }
}

/// A check that can't run: propose making it optional (the person can edit the spec instead).
async fn propose_check_change(state: &AppState, task: &Task, failure: &Failure) {
    let checks = state.db.list_checks(&task.id).await.unwrap_or_default();
    let Some(c) = checks.iter().find(|c| c.required && matches!(c.status.as_str(), "failed" | "error")).or_else(|| checks.iter().find(|c| matches!(c.status.as_str(), "failed" | "error"))) else {
        return;
    };
    let before = json!({"check_id": c.id, "kind": c.kind, "spec": c.spec_value(), "required": c.required});
    let after = json!({"check_id": c.id, "kind": c.kind, "spec": c.spec_value(), "required": false});
    let reason = format!("The {} check can't run ({}). Make it optional, or edit its spec.", c.kind, failure.summary);
    if let Ok(change) = state.db.propose_change(Some(task), None, "check_spec", &before, &after, &reason, "kwant").await {
        tracing::info!(change_id = %change.id, task_id = %task.id, "autofix proposed a check change");
    }
}

// Running attempts

/// Runs `a` as soon as the task's last turn has ended (its spend has landed) and the mission's
/// runner slot is free; the scheduler tick ([`run_due`]) catches anything left.
fn soon(state: &AppState, a: &Attempt) {
    if a.not_before.is_some_and(|t| t > Utc::now()) {
        return;
    }
    let state = state.clone();
    let id = a.id.clone();
    tokio::spawn(async move {
        for _ in 0..240 {
            let Ok(Some(a)) = state.db.get_attempt(&id).await else { return };
            if a.status != "scheduled" {
                return;
            }
            if !busy(&state, &a.task_id).await {
                execute(&state, &a).await;
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

/// Whether the task's last claim still has a turn going (its spend hasn't landed) or its
/// mission's runner slot is taken.
async fn busy(state: &AppState, task_id: &str) -> bool {
    let Ok(Some(task)) = state.db.get_task(task_id).await else { return false };
    if task.mission_id.as_deref().is_some_and(super::runner::is_running) {
        return true;
    }
    let claim = match state.db.open_claim(task_id).await.ok().flatten() {
        Some(c) => Some(c),
        None => state.db.last_released_claim(task_id).await.ok().flatten(),
    };
    let Some(claim) = claim else { return false };
    state
        .turns
        .active_turns()
        .iter()
        .any(|t| t.kwant_id == claim.kwant_id && claim.context_id.as_deref().is_none_or(|c| c == t.context_id))
}

/// Scheduled attempts whose time has come, on each scheduler tick (before the runner picks
/// tasks). A restart loses nothing: the attempt waits in the table.
pub async fn run_due(state: &AppState) {
    if mode(state).await == "off" {
        return;
    }
    for a in state.db.due_attempts(Utc::now()).await.unwrap_or_default() {
        if !busy(state, &a.task_id).await {
            execute(state, &a).await;
        }
    }
}

/// Starts a scheduled attempt: limits are checked again (spend has landed by now), then the task
/// goes back to `ready` as [`Actor::AutoFix`], or a routine gets a new run.
async fn execute(state: &AppState, a: &Attempt) {
    let Ok(Some(task)) = state.db.get_task(&a.task_id).await else { return };
    if task.state != "failed" {
        let _ = state.db.move_attempt(a, "scheduled", "skipped", Some("the task moved on before the retry")).await;
        emit_task(state, &task).await;
        return;
    }
    if let Some(code) = limit(state, &task).await {
        let _ = state.db.move_attempt(a, "scheduled", "skipped", Some(limit_reason(code))).await;
        emit_task(state, &task).await;
        return;
    }
    match state.db.move_attempt(a, "scheduled", "running", None).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::info!(autofix_id = %a.id, error = %e, "an autofix attempt couldn't start");
            let _ = state.db.move_attempt(a, "scheduled", "skipped", Some(limit_reason("no_attempts_left"))).await;
            return;
        }
    }
    if retried_in_place(&task) {
        let change = TaskChange {
            kind: "autofix".into(),
            reason: Some(format!("AutoFix: {}", a.reason)),
            detail: Some(json!({"autofix_id": a.id, "strategy": a.strategy})),
            ..Default::default()
        };
        if let Err(e) = transition(state, &task.id, "failed", "ready", Actor::AutoFix, "kwant", change).await {
            let _ = state.db.move_attempt(a, "running", "failed", Some(&format!("couldn't make the task ready: {e}"))).await;
            emit_task(state, &task).await;
            return;
        }
        if let Some(mid) = &task.mission_id {
            if let Ok(Some(m)) = state.db.get_mission(mid).await {
                if m.status == "active" && !super::runner::is_running(mid) {
                    let _ = super::runner::run_mission(state, &m, 1).await;
                }
            }
        }
    } else if task.source_kind == "routine" {
        retry_routine(state, a, &task).await;
    }
}

/// A routine retry is a new run of the routine; its task is linked to the failed one (§4.4).
async fn retry_routine(state: &AppState, a: &Attempt, failed: &Task) {
    let routine = match failed.source_id.as_deref() {
        Some(run) => match state.db.routine_run_of(run).await {
            Ok(Some((routine, _))) => state.db.get_routine(&routine).await.ok().flatten(),
            _ => None,
        },
        None => None,
    };
    let Some(routine) = routine else {
        let _ = state.db.move_attempt(a, "running", "failed", Some("the routine is gone")).await;
        return;
    };
    let routine_id = routine.id.clone();
    let Some(run) = crate::server::scheduler::start_run(state, routine, "manual").await else {
        let _ = state.db.move_attempt(a, "running", "failed", Some("the routine run couldn't start")).await;
        return;
    };
    if run.status != "running" {
        let _ = state.db.move_attempt(a, "running", "failed", Some("the routine was still running")).await;
        return;
    }
    super::listener::on_routine_run(state, &routine_id, &run.id, "running").await;
    if let Ok(Some(new)) = state.db.task_by_source("routine", &run.id).await {
        let _ = state.db.link_retry_task(&new.id, failed, &a.id).await;
        let _ = state.db.set_retry_task(&a.id, &new.id).await;
        super::emit_task_id(state, &new.id).await;
    }
    super::emit_task_id(state, &failed.id).await;
}

/// The task's live attempt ended with the task: verified → succeeded, cancelled → skipped.
pub async fn settle(state: &AppState, task: &Task, to: &str) {
    let status = match to {
        "verified" => "succeeded",
        "cancelled" => "skipped",
        _ => return,
    };
    let _ = state.db.finish_running_attempt(&task.id, status).await;
}

// Recovery spend

/// Recovery spend: a turn of a claim AutoFix took the task for adds to `recovery_spent_tokens`
/// and the attempt's `tokens_spent` too.
pub async fn spend_on_claim(state: &AppState, task_id: &str, claim_id: Option<&str>, tokens: i64) {
    let aid = match claim_id {
        Some(c) => state.db.claim_autofix(c).await.ok().flatten(),
        None => state.db.latest_claim_autofix(task_id).await.ok().flatten(),
    };
    if let Some(aid) = aid {
        let _ = state.db.spend_recovery(task_id, &aid, tokens).await;
    }
}

// The retry prompt

/// What a retry's prompt adds: why it runs again, the diagnostics, the idempotency key and the
/// external calls already made. `None` when the task isn't on an AutoFix attempt.
pub async fn retry_note(state: &AppState, task: &Task) -> Option<String> {
    let a = state.db.current_attempt(&task.id).await.ok().flatten().filter(|a| a.status == "running")?;
    let failure = state.db.get_failure(&a.failure_id).await.ok().flatten()?;
    let mut out = format!(
        "\n\n[AutoFix] This is attempt {} of {} (strategy {}). The last attempt failed: {}.\n",
        task.attempts + 1,
        task.max_attempts,
        a.strategy,
        failure.summary
    );
    match a.strategy.as_str() {
        "retry_with_diagnostics" => {
            let checks = state.db.list_checks(&task.id).await.unwrap_or_default();
            if let Some(c) = checks.iter().find(|c| matches!(c.status.as_str(), "failed" | "error")) {
                out.push_str(&format!("The check that failed: {} {}\n", c.kind, redact_value(&c.spec_value())));
            }
            if let Some(x) = &failure.excerpt {
                out.push_str(&format!("Its output (secrets masked):\n{x}\n"));
            }
        }
        "retry_from_checkpoint" | "retry_same" if failure.class == "stalled" => out.push_str("Continue; the files are as you left them.\n"),
        _ => {
            if let Some(x) = &failure.excerpt {
                out.push_str(&format!("The error (secrets masked):\n{}\n", cut(x, 500)));
            }
        }
    }
    out.push_str(&format!("KWANT_IDEMPOTENCY_KEY={}\n", a.idempotency_key));
    let external: Vec<String> = state
        .db
        .tool_calls_of(&task.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.effect == "external" && c.decision != "denied")
        .map(|c| format!("- {}: {}", c.tool_name, cut(&redact(&c.summary), 160)))
        .collect();
    if external.is_empty() {
        out.push_str("No external calls are recorded for this task yet.");
    } else {
        out.push_str(&format!(
            "These external calls already happened in earlier attempts; check before repeating them, and pass the key where the service accepts one:\n{}",
            external.iter().rev().take(20).rev().cloned().collect::<Vec<_>>().join("\n")
        ));
    }
    Some(out)
}

// Board JSON

pub fn attempt_json(a: &Attempt) -> Value {
    json!({
        "id": a.id,
        "strategy": a.strategy,
        "status": a.status,
        "not_before": a.not_before,
        "reason": a.reason,
        "tokens_spent": a.tokens_spent,
        "retry_task_id": a.retry_task_id,
    })
}

/// Adds the AutoFix fields to a task's JSON.
pub async fn extend_task_json(state: &AppState, task: &Task, obj: &mut serde_json::Map<String, Value>) {
    let pct = recovery_pct(state).await;
    obj.insert("recovery_budget_tokens".into(), json!(recovery_budget(task, pct)));
    let failures = state.db.failures_of(&task.id).await.unwrap_or_default();
    obj.insert(
        "last_failure".into(),
        failures.last().map_or(Value::Null, |f| json!({"class": f.class, "summary": f.summary, "created_at": f.created_at})),
    );
    let current = state.db.current_attempt(&task.id).await.ok().flatten();
    obj.insert("autofix".into(), current.as_ref().map_or(Value::Null, attempt_json));
    // Retries AutoFix actually started (planned, suggested or skipped ones don't count)
    let ran = state.db.attempts_of(&task.id).await.unwrap_or_default().iter().filter(|a| matches!(a.status.as_str(), "running" | "succeeded" | "failed")).count();
    obj.insert("autofix_attempts".into(), json!(ran));
}

/// AutoFix's Needs-you items, for a task that isn't running: a suggested retry, a proposed
/// change, or what the person must fix.
pub async fn needs_you(state: &AppState, task: &Task) -> Vec<Value> {
    if matches!(task.state.as_str(), "working" | "needs_you") {
        return Vec::new();
    }
    let mut items = Vec::new();
    let failures = state.db.failures_of(&task.id).await.unwrap_or_default();
    let last = failures.last();
    if matches!(task.state.as_str(), "failed" | "blocked") {
        if let Some(a) = state.db.current_attempt(&task.id).await.ok().flatten().filter(|a| a.status == "suggested" && a.strategy != "propose_change") {
            let summary = last.map(|f| f.summary.as_str()).unwrap_or("");
            let text = match a.strategy.as_str() {
                "retry_with_diagnostics" => format!("Retry with the check output? {summary}"),
                "wait_and_retry" => format!("Retry after a temporary error? {summary}"),
                _ => format!("Retry from where it stopped? {summary}"),
            };
            items.push(json!({"kind": "autofix_suggested", "id": a.id, "text": cut(&text, 200)}));
        }
        if let Some(f) = last {
            let over_budget = || async {
                let attempts = state.db.attempts_of(&task.id).await.unwrap_or_default();
                attempts.iter().rev().find(|a| a.failure_id == f.id).is_some_and(|a| a.status == "skipped" && a.reason.starts_with("over_recovery_budget"))
            };
            if let Some(text) = fix_text(&f.class) {
                items.push(json!({"kind": "fix_needed", "id": f.id, "text": text}));
            } else if task.state == "failed" && over_budget().await {
                items.push(json!({"kind": "fix_needed", "id": f.id, "text": RECOVERY_BUDGET_FIX}));
            }
        }
    }
    for c in state.db.list_changes(Some("proposed"), Some(&task.id)).await.unwrap_or_default() {
        let text = format!("Allow this change to {}: {}", target_label(&c.target), c.reason);
        items.push(json!({"kind": "workflow_change", "id": c.id, "text": cut(&text, 200)}));
    }
    items
}

fn target_label(target: &str) -> &'static str {
    match target {
        "routine_instructions" => "the routine's instructions",
        "routine_schedule" => "the routine's schedule",
        "check_spec" => "a check",
        "task_acceptance" => "the task's acceptance",
        _ => "the task's max attempts",
    }
}

// API

/// `GET /api/tasks/:id/failures`
pub async fn task_failures(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let failures = match state.db.failures_of(&t.id).await {
        Ok(f) => f,
        Err(e) => return internal(e),
    };
    let attempts = state.db.attempts_of(&t.id).await.unwrap_or_default();
    let list: Vec<Value> = failures
        .iter()
        .map(|f| {
            let a = attempts.iter().rev().find(|a| a.failure_id == f.id);
            json!({
                "id": f.id,
                "attempt": f.attempt,
                "source": f.source,
                "class": f.class,
                "summary": f.summary,
                "excerpt": f.excerpt,
                "checkpoint_id": f.checkpoint_id,
                "created_at": f.created_at,
                "autofix": a.map_or(Value::Null, attempt_json),
            })
        })
        .collect();
    Json(json!(list)).into_response()
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AutofixBody {
    pub failure_id: Option<String>,
}

fn coded(status: StatusCode, code: &str, msg: &str) -> Response {
    (status, Json(json!({"error": msg, "code": code}))).into_response()
}

/// `POST /api/tasks/:id/autofix` (person-only, `Idempotency-Key`): runs the suggested retry of a
/// failure (the last one by default), or plans one and runs it.
pub async fn run_autofix(State(state): State<AppState>, RawQuery(q): RawQuery, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<AutofixBody>>) -> Response {
    let body = body.map(|b| b.0).unwrap_or_default();
    let route = format!("POST /api/tasks/{id}/autofix");
    super::graph::idempotent(&state, &headers, route, || async {
        let t = load_task(&state, q.as_deref(), &id).await?;
        let failures = state.db.failures_of(&t.id).await.map_err(internal)?;
        let failure = match &body.failure_id {
            Some(fid) => failures.iter().find(|f| &f.id == fid).cloned().ok_or_else(|| not_found("failure"))?,
            None => failures.last().cloned().ok_or_else(|| coded(StatusCode::CONFLICT, "not_retryable", "the task has no failure to fix"))?,
        };
        if t.state != "failed" || matches!(failure.class.as_str(), "auth" | "quota" | "budget" | "denied" | "needs_input" | "bad_spec") || t.source_kind == "delegation" {
            return Err(coded(StatusCode::CONFLICT, "not_retryable", "AutoFix can't retry this; see the task's Needs-you item"));
        }
        if let Some(code) = limit(&state, &t).await {
            return Err(coded(StatusCode::CONFLICT, code, limit_reason(code)));
        }
        let attempts = state.db.attempts_of(&t.id).await.map_err(internal)?;
        let live = attempts.iter().find(|a| matches!(a.status.as_str(), "scheduled" | "running"));
        let a = match (attempts.iter().rev().find(|a| a.failure_id == failure.id && a.status == "suggested"), live) {
            (_, Some(live)) => live.clone(),
            (Some(s), None) => {
                if !state.db.move_attempt(s, "suggested", "scheduled", Some("the person asked for this retry")).await.map_err(internal)? {
                    return Err(conflict("the retry changed meanwhile; look at it again"));
                }
                state.db.get_attempt(&s.id).await.map_err(internal)?.ok_or_else(|| not_found("autofix"))?
            }
            (None, None) => {
                let strategy = match strategy_for(&failure.class) {
                    "none" => "retry_same",
                    s => s,
                };
                let new = NewAttempt {
                    task: &t,
                    failure_id: &failure.id,
                    strategy,
                    idempotency_key: key(&t.id, &failure.id, strategy),
                    status: "scheduled",
                    not_before: None,
                    reason: "the person asked for this retry".into(),
                };
                match state.db.insert_attempt(&new).await {
                    Ok(Some(a)) => a,
                    Ok(None) => return Err(coded(StatusCode::CONFLICT, "not_retryable", "this failure was retried that way already")),
                    Err(e) => return Err(conflict(e.to_string())),
                }
            }
        };
        // The person asked now: no waiting for a backoff
        if a.status == "scheduled" && !busy(&state, &t.id).await {
            let now = Attempt { not_before: None, ..a.clone() };
            execute(&state, &now).await;
        } else if a.status == "scheduled" {
            soon(&state, &Attempt { not_before: None, ..a.clone() });
        }
        let a = state.db.get_attempt(&a.id).await.map_err(internal)?.unwrap_or(a);
        Ok(attempt_json(&a))
    })
    .await
}

/// `POST /api/tasks/:id/autofix/:aid/cancel` (person-only): a planned retry won't run.
pub async fn cancel_autofix(State(state): State<AppState>, RawQuery(q): RawQuery, Path((id, aid)): Path<(String, String)>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let a = match state.db.get_attempt(&aid).await {
        Ok(Some(a)) if a.task_id == t.id => a,
        Ok(_) => return not_found("autofix"),
        Err(e) => return internal(e),
    };
    if !matches!(a.status.as_str(), "scheduled" | "suggested") {
        return conflict("only a planned retry can be cancelled");
    }
    match state.db.move_attempt(&a, &a.status, "skipped", Some("cancelled by the person")).await {
        Ok(true) => {}
        Ok(false) => return conflict("the retry changed meanwhile; look at it again"),
        Err(e) => return internal(e),
    }
    emit_task(&state, &t).await;
    let a = state.db.get_attempt(&aid).await.ok().flatten().unwrap_or(a);
    Json(attempt_json(&a)).into_response()
}

async fn change_json(state: &AppState, c: &WorkflowChange) -> Value {
    let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Null);
    json!({
        "id": c.id,
        "task_id": c.task_id,
        "routine_id": c.routine_id,
        "target": c.target,
        "before": redact_value(&parse(&c.before)),
        "after": redact_value(&parse(&c.after)),
        "reason": c.reason,
        "proposed_by": c.proposed_by,
        "proposed_by_name": super::name_of(state, Some(&c.proposed_by)).await,
        "status": c.status,
        "decided_at": c.decided_at,
        "created_at": c.created_at,
    })
}

/// `GET /api/workflow-changes?status=proposed&task_id=…` (person-only; both filters optional).
pub async fn list_changes(State(state): State<AppState>, RawQuery(q): RawQuery) -> Response {
    let param = |name: &str| q.as_deref().unwrap_or("").split('&').find_map(|p| p.strip_prefix(name)).map(str::to_string);
    let status = param("status=");
    if let Some(s) = status.as_deref().filter(|s| !["proposed", "approved", "rejected", "applied", "stale"].contains(s)) {
        return bad(format!("status must be proposed, approved, rejected, applied or stale (got {s})"));
    }
    let task_id = param("task_id=").filter(|t| !t.is_empty());
    match state.db.list_changes(status.as_deref(), task_id.as_deref()).await {
        Ok(list) => {
            let mut out = Vec::new();
            for c in &list {
                out.push(change_json(&state, c).await);
            }
            Json(json!(out)).into_response()
        }
        Err(e) => internal(e),
    }
}

/// `POST /api/workflow-changes/:id/approve` (person-only): approves and applies it once.
pub async fn approve_change(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    decide(&state, &id, true).await
}

/// `POST /api/workflow-changes/:id/reject` (person-only).
pub async fn reject_change(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    decide(&state, &id, false).await
}

async fn decide(state: &AppState, id: &str, approve: bool) -> Response {
    let c = match state.db.get_change(id).await {
        Ok(Some(c)) => c,
        Ok(None) => return not_found("workflow change"),
        Err(e) => return internal(e),
    };
    let to = if approve { "approved" } else { "rejected" };
    match state.db.decide_change(&c, to).await {
        Ok(true) => {}
        Ok(false) => return conflict(format!("this change is {} already", c.status)),
        Err(e) => return internal(e),
    }
    if approve {
        let write = change_write(state, &c).await;
        if let Err(e) = state.db.apply_change(&c, &write).await {
            return internal(e);
        }
    }
    if let Some(t) = &c.task_id {
        super::emit_task_id(state, t).await;
    }
    let c = state.db.get_change(id).await.ok().flatten().unwrap_or(c);
    Json(change_json(state, &c).await).into_response()
}

/// What applying `c` writes, if its target still holds `before`.
async fn change_write(state: &AppState, c: &WorkflowChange) -> ChangeWrite {
    let before: Value = serde_json::from_str(&c.before).unwrap_or(Value::Null);
    let after: Value = serde_json::from_str(&c.after).unwrap_or(Value::Null);
    let task = match &c.task_id {
        Some(t) => state.db.get_task(t).await.ok().flatten(),
        None => None,
    };
    match c.target.as_str() {
        "routine_instructions" | "routine_schedule" => {
            let Some(rid) = &c.routine_id else { return ChangeWrite::Stale };
            let Ok(Some(r)) = state.db.get_routine(rid).await else { return ChangeWrite::Stale };
            if c.target == "routine_instructions" {
                let (Some(b), Some(a)) = (before["instructions"].as_str(), after["instructions"].as_str()) else { return ChangeWrite::Stale };
                if r.instructions != b {
                    return ChangeWrite::Stale;
                }
                return ChangeWrite::RoutineInstructions { routine_id: rid.clone(), before: b.into(), after: a.into() };
            }
            let Ok(schedule) = serde_json::from_value::<crate::routines::Schedule>(after["schedule"].clone()) else { return ChangeWrite::Stale };
            if schedule.validate().is_err() || serde_json::from_str::<Value>(&r.schedule).ok() != Some(before["schedule"].clone()) {
                return ChangeWrite::Stale;
            }
            let schedule = schedule.canonical();
            let next_run_at = if r.enabled { schedule.next_after(Utc::now(), r.last_run_at) } else { r.next_run_at };
            let after = serde_json::to_string(&schedule).unwrap_or_default();
            ChangeWrite::RoutineSchedule { routine_id: rid.clone(), before: r.schedule.clone(), after, next_run_at }
        }
        "check_spec" => {
            let Some(cid) = before["check_id"].as_str() else { return ChangeWrite::Stale };
            let Ok(Some(check)) = state.db.get_check(cid).await else { return ChangeWrite::Stale };
            if check.spec_value() != before["spec"] || check.required != before["required"].as_bool().unwrap_or(check.required) {
                return ChangeWrite::Stale;
            }
            let after_spec = after.get("spec").cloned().unwrap_or_else(|| check.spec_value());
            if super::validate_check(&check.kind, &after_spec, true).is_err() {
                return ChangeWrite::Stale;
            }
            ChangeWrite::CheckSpec {
                check_id: cid.into(),
                before_spec: check.spec.clone(),
                before_required: check.required,
                after_spec: after_spec.to_string(),
                after_required: after["required"].as_bool().unwrap_or(check.required),
            }
        }
        "task_acceptance" => {
            let (Some(t), Some(b), Some(a)) = (task, before["acceptance"].as_str(), after["acceptance"].as_str()) else { return ChangeWrite::Stale };
            if t.acceptance != b {
                return ChangeWrite::Stale;
            }
            ChangeWrite::Acceptance { task_id: t.id, before: b.into(), after: a.into() }
        }
        "max_attempts" => {
            let (Some(t), Some(b), Some(a)) = (task, before["max_attempts"].as_i64(), after["max_attempts"].as_i64()) else { return ChangeWrite::Stale };
            if t.max_attempts != b || !(1..=10).contains(&a) {
                return ChangeWrite::Stale;
            }
            ChangeWrite::MaxAttempts { task_id: t.id, before: b, after: a }
        }
        _ => ChangeWrite::Stale,
    }
}

/// The bot tool `workflow_propose`: only ever a `proposed` row. `before` is read here, never
/// taken from the bot.
pub async fn propose(state: &AppState, bot_id: &str, args: &Value) -> Result<Value, String> {
    let target = args["target"].as_str().unwrap_or("");
    if !TARGETS.contains(&target) {
        return Err(format!("target must be one of {}", TARGETS.join(", ")));
    }
    let reason = args["reason"].as_str().map(str::trim).filter(|r| !r.is_empty()).ok_or("reason is required")?;
    let reason: String = reason.chars().take(1000).collect();
    let after = &args["after"];
    let task = match args["task_id"].as_str() {
        Some(t) => Some(state.db.get_task(t).await.map_err(|e| e.to_string())?.ok_or("no such task")?),
        None => None,
    };
    let (routine_id, before, after) = match target {
        "routine_instructions" | "routine_schedule" => {
            let rid = args["routine_id"].as_str().ok_or("routine_id is required for a routine change")?;
            let r = state.db.get_routine(rid).await.map_err(|e| e.to_string())?.ok_or("no such routine")?;
            if target == "routine_instructions" {
                let a = after.as_str().ok_or("after must be the new instructions (a string)")?;
                (Some(rid.to_string()), json!({"instructions": r.instructions}), json!({"instructions": a.chars().take(20_000).collect::<String>()}))
            } else {
                let s = serde_json::from_value::<crate::routines::Schedule>(after.clone()).map_err(|e| format!("after must be a schedule: {e}"))?;
                s.validate()?;
                let before = serde_json::from_str::<Value>(&r.schedule).unwrap_or(Value::Null);
                (Some(rid.to_string()), json!({"schedule": before}), json!({"schedule": after}))
            }
        }
        "check_spec" => {
            let t = task.as_ref().ok_or("task_id is required for a check change")?;
            let cid = after["check_id"].as_str().ok_or("after.check_id is required")?;
            let c = state.db.get_check(cid).await.map_err(|e| e.to_string())?.filter(|c| c.task_id == t.id).ok_or("no such check on this task")?;
            let spec = after.get("spec").cloned().unwrap_or_else(|| c.spec_value());
            if spec.to_string().contains("[redacted]") {
                return Err("after.spec has masked values ([redacted]); ask the person for the real ones".into());
            }
            super::validate_check(&c.kind, &spec, true)?;
            let required = after["required"].as_bool().unwrap_or(c.required);
            (None, json!({"check_id": c.id, "kind": c.kind, "spec": c.spec_value(), "required": c.required}), json!({"check_id": c.id, "kind": c.kind, "spec": spec, "required": required}))
        }
        "task_acceptance" => {
            let t = task.as_ref().ok_or("task_id is required")?;
            let a = after.as_str().ok_or("after must be the new acceptance (a string)")?;
            (None, json!({"acceptance": t.acceptance}), json!({"acceptance": a.chars().take(4000).collect::<String>()}))
        }
        _ => {
            let t = task.as_ref().ok_or("task_id is required")?;
            let a = after.as_i64().filter(|n| (1..=10).contains(n)).ok_or("after must be a number 1–10")?;
            (None, json!({"max_attempts": t.max_attempts}), json!({"max_attempts": a}))
        }
    };
    let c = state.db.propose_change(task.as_ref(), routine_id.as_deref(), target, &before, &after, &reason, bot_id).await.map_err(|e| e.to_string())?;
    if let Some(t) = &task {
        emit_task(state, t).await;
    }
    Ok(json!({"id": c.id, "status": c.status, "note": "Proposed. Only the person can approve it; nothing changes until then."}))
}

/// Insights for a space: `autofix_7d` and `failures_by_class_7d`.
pub async fn insights(state: &AppState, space_id: &str, since: DateTime<Utc>) -> (Value, Value) {
    let (attempts, succeeded, tokens, classes) = state.db.autofix_stats(space_id, since).await.unwrap_or_default();
    let by_class: serde_json::Map<String, Value> = classes.into_iter().map(|(c, n)| (c, json!(n))).collect();
    (json!({"attempts": attempts, "succeeded": succeeded, "tokens": tokens}), Value::Object(by_class))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kwants_own_no_account_messages_are_auth() {
        use crate::db::models::LlmProfile;
        use crate::profiles::classify::{FailClass, ProviderFailure};
        use crate::server::profile_turn::{give_up_notice, switch_notice};
        let p = |name: &str| LlmProfile {
            id: name.into(),
            engine: "claude".into(),
            name: name.into(),
            kind: "subscription".into(),
            home_dir: None,
            priority: 0,
            enabled: true,
            is_default: true,
            created_at: None,
            space_id: "spc_default".into(),
        };
        let auth = ProviderFailure { class: FailClass::Auth, retry_at: None, detail: "401".into() };
        // The exact texts a turn ends with, built by the code that writes them: a rewording breaks this
        let texts = [
            give_up_notice("Ann", &p("This machine"), &auth, 1),
            give_up_notice("Ann", &p("Work"), &auth, 2),
            switch_notice(&p("Work"), &p("Personal"), &auth),
            crate::drivers::engines::no_engine_notice("Ann", false).0,
            crate::drivers::engines::no_engine_notice("Ann", true).0,
            crate::drivers::engines::NO_CUSTOM_ACCOUNT.to_string(),
        ];
        for t in &texts {
            assert_eq!(classify("turn", "error", t), "auth", "{t}");
            assert_eq!(strategy_for(classify("turn", "error", t)), "none", "{t}");
        }
        // The give-up text has no "401" of its own: the words alone must carry it
        assert!(!texts[0].contains("401"), "{}", texts[0]);
    }

    #[test]
    fn kwants_own_out_of_usage_messages_are_quota() {
        use crate::db::models::LlmProfile;
        use crate::profiles::classify::{FailClass, ProviderFailure};
        use crate::server::profile_turn::{give_up_notice, switch_notice};
        let p = |name: &str| LlmProfile {
            id: name.into(),
            engine: "claude".into(),
            name: name.into(),
            kind: "subscription".into(),
            home_dir: None,
            priority: 0,
            enabled: true,
            is_default: true,
            created_at: None,
            space_id: "spc_default".into(),
        };
        let reset = chrono::Utc::now() + chrono::Duration::hours(2);
        let until = ProviderFailure { class: FailClass::Quota, retry_at: Some(reset), detail: "429".into() };
        let open = ProviderFailure { class: FailClass::Quota, retry_at: None, detail: String::new() };
        // The exact texts a turn ends with, built by the code that writes them: a rewording breaks this
        let texts = [
            give_up_notice("Ann", &p("This machine"), &until, 1),
            give_up_notice("Ann", &p("Work"), &open, 2),
            switch_notice(&p("Work"), &p("Personal"), &until),
            // and the Claude CLI's own usage-limit lines
            "Claude AI usage limit reached|1792162800".to_string(),
            "You've hit your limit · resets 5pm".to_string(),
            "You've hit your usage limit. Upgrade to Pro or try again in 3 hours 20 minutes.".to_string(),
            "5-hour limit reached ∙ resets 3pm".to_string(),
            "Your credit balance is too low to access the Anthropic API".to_string(),
            "API Error: 429 {\"type\":\"error\",\"error\":{\"message\":\"usage limit reached\"}}".to_string(),
        ];
        for t in &texts {
            assert_eq!(classify("turn", "error", t), "quota", "{t}");
            assert_eq!(strategy_for(classify("turn", "error", t)), "none", "{t}");
            assert_eq!(fix_text(classify("turn", "error", t)), Some("Usage limit reached: wait for it to reset or switch account, then retry."), "{t}");
        }
        // A plain rate limit still passes in a minute
        assert_eq!(classify("turn", "error", "stream error: rate-limiting (HTTP 429)"), "transient");
        assert_eq!(classify("turn", "error", "rate limit exceeded, retry in 20s"), "transient");
    }

    #[tokio::test]
    async fn an_http_check_on_a_closed_port_is_transient_and_waits() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let spec = json!({"url": format!("http://127.0.0.1:{port}/health")});
        let (status, out) = super::super::checks::run_spec("http", &spec, None, true).await;
        assert_eq!(status, "error", "{out}");
        assert!(out.contains("network error"), "{out}");
        assert_eq!(classify("check", &status, &out), "transient", "{out}");
        assert_eq!(strategy_for("transient"), "wait_and_retry");
        // The text fallback still holds for words the engines print
        assert_eq!(classify("check", "error", "connection refused"), "transient");
    }

    #[test]
    fn a_bare_key_inside_a_spec_is_masked() {
        // Glued to other characters, as typed into a command: no space on either side of the key
        let spec = json!({"command": "test \"$(cat t)\" = \"Zq8VwK3mT9pL2rX7nB4c\">/dev/null && curl https://example.com/v1/items", "timeout_secs": 30});
        let out = redact_value(&spec);
        let s = out.to_string();
        assert!(!s.contains("Zq8VwK3mT9pL2rX7nB4c"), "{s}");
        assert!(s.contains("= \\\"[redacted]\\\">/dev/null"), "{s}");
        assert!(s.contains("https://example.com/v1/items"), "{s}");
        // As text (the failure excerpt) too
        assert!(!redact(&spec.to_string()).contains("Zq8VwK3mT9pL2rX7nB4c"));
        assert_eq!(out["timeout_secs"], 30);
        // Quotes and brackets around a key are kept as they were
        assert_eq!(redact("key [sk-ant-api03abcdef]"), "key [[redacted]]");
        assert_eq!(redact("'ghp_abcdefghij0123456789'"), "'[redacted]'");
        assert_eq!(redact("exit code 1\nexpected 42 got 41"), "exit code 1\nexpected 42 got 41");
    }

    #[test]
    fn names_paths_and_ids_next_to_a_secret_stay_readable() {
        // eval 10: the command name was masked as `[redacted] --go`
        assert_eq!(redact("nonexistent_cmd_xyz_ev10 --go"), "nonexistent_cmd_xyz_ev10 --go");
        assert_eq!(redact("./scripts/check_deploy_20261027.sh --go"), "./scripts/check_deploy_20261027.sh --go");
        assert_eq!(redact("MyServiceV2HealthCheckEndpoint2026 ok"), "MyServiceV2HealthCheckEndpoint2026 ok");
        assert_eq!(redact("task 71579df3-4d9b-4edc-9d88-20e540399f9c failed"), "task 71579df3-4d9b-4edc-9d88-20e540399f9c failed");
        assert_eq!(redact("report_2026_10_27_final_v12.txt"), "report_2026_10_27_final_v12.txt");
        // the neighbours of a real key keep their names
        assert_eq!(
            redact("nonexistent_cmd_ev10b --go Zq8VwK3mT9pL2rX7nB4c done"),
            "nonexistent_cmd_ev10b --go [redacted] done"
        );
        // generated keys are still found, bare and inside a relative path
        assert_eq!(redact("aB3dE5gH7jK9mN1pQ3sT"), "[redacted]");
        assert_eq!(redact("9f86d081884c7d659a2feaa0c55ad015"), "[redacted]");
        assert_eq!(redact("cfg/Zq8VwK3mT9pL2rX7nB4c/x"), "[redacted]");
        assert_eq!(redact("dGhpc2lzYVNlY3JldDEyMzQ1Ng+/x9="), "[redacted]");
    }

    #[test]
    fn labelled_secrets_are_masked_however_short() {
        let cases = [
            ("login --password hunter2 --go", "login --password [redacted] --go"),
            ("login --password=pw1", "login --password=[redacted]"),
            ("password=x1 user=bob", "password=[redacted] user=bob"),
            ("curl -H 'X-Token: abc'", "curl -H 'X-Token: [redacted]'"),
            ("api --token t0k", "api --token [redacted]"),
            ("secret: s3", "secret: [redacted]"),
            ("run --key k1 --go", "run --key [redacted] --go"),
            ("run --key=k1", "run --key=[redacted]"),
            ("run --api-key=k1", "run --api-key=[redacted]"),
            ("run --license-key 'ab'", "run --license-key '[redacted]'"),
            ("key=k1 && go", "key=[redacted] && go"),
            (r#"{"key": "k1", "keep": "me"}"#, r#"{"key": "[redacted]", "keep": "me"}"#),
            ("Authorization: Basic YWI6Y2Q=", "Authorization: Basic [redacted]"),
            ("Authorization: Bearer t1", "Authorization: Bearer [redacted]"),
            ("curl https://bob:pw@db.local/x", "curl https://bob:[redacted]@db.local/x"),
            ("mysql -u root -phunter2 db", "mysql -u root -p[redacted] db"),
            ("sshpass -p pw ssh host", "sshpass -p [redacted] ssh host"),
            ("/usr/bin/redis-cli -a pw ping", "/usr/bin/redis-cli -a [redacted] ping"),
        ];
        for (text, want) in cases {
            assert_eq!(redact(text), want, "{text}");
        }
        // `-p` of other tools is a port or "parents", and a key in prose isn't a label
        assert_eq!(redact("mkdir -p out && ssh -p 2222 host"), "mkdir -p out && ssh -p 2222 host");
        assert_eq!(redact("the key point: it works"), "the key point: it works");
        assert_eq!(redact("cargo test --no-key x"), "cargo test --no-key x");
    }

    #[test]
    fn masking_twice_changes_nothing() {
        // eval 10: a stored (masked) excerpt was masked again and showed `[redacted]]`
        for text in [
            "echo password=hunter2hunter2 token=abc; exit 1",
            "nonexistent_cmd_ev10b --key sk-ant-api03abcdefghij --token ghp_abcdefghij0123456789",
            "mysql -phunter2 && curl https://u:p@h/x -H 'Authorization: Bearer abc'",
            r#"{"command": "x --password pw", "key": "k1"}"#,
        ] {
            let once = redact(text);
            assert_eq!(redact(&once), once, "{text}");
            assert!(!once.contains("]]"), "{once}");
            assert_eq!(redact_value(&redact_value(&json!({"c": text}))), redact_value(&json!({"c": text})));
        }
    }

    #[test]
    fn a_never_retried_failure_says_what_to_do_once() {
        for class in ["auth", "quota", "budget", "denied"] {
            let fix = fix_text(class).unwrap();
            assert!(fix.ends_with('.') && !fix.contains("retry:"), "{fix}");
        }
        assert_eq!(fix_text("transient"), None);
    }
}
