//! Phase B1: the work graph. "May be invalid" flags (evidence drift and forks of verified work),
//! the drift scan, resolving a flag (the person only) and forking a task to redo it. A flag never
//! moves a task; only the person's resolution does. Contract: docs/mission-control.md §11.
#![allow(clippy::result_large_err)]

use super::checks::{hex, inside};
use super::routes::{bad, conflict, internal, load_mission, load_task, moved, not_found};
use super::{emit_mission, emit_task, emit_task_id, name_of, post_line, task_folder, task_json, transition, Actor};
use crate::db::mission_graph::{FlagClaim, GraphScan, TaskFlag, FLAG_RESOLUTIONS};
use crate::db::missions::{Mission, Task, TaskChange};
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Most evidence files one scan hashes.
pub const SCAN_MAX_FILES: usize = 200;
/// Files over this are skipped, never treated as changed.
pub const SCAN_MAX_BYTES: u64 = 50 * 1024 * 1024;
/// The runner scans an active mission at most this often.
pub const SCAN_EVERY_MINUTES: i64 = 10;

pub fn flag_json(f: &TaskFlag) -> Value {
    json!({
        "id": f.id,
        "kind": f.kind,
        "cause": f.cause,
        "cause_task_id": f.cause_task_id,
        "cause_title": f.cause_title,
        "detail": f.detail_value(),
        "created_at": f.created_at,
        "resolved_at": f.resolved_at,
        "resolution": f.resolution,
    })
}

/// Why the task may no longer hold, as a sentence.
pub fn cause_text(f: &TaskFlag) -> String {
    let d = f.detail_value();
    let path = d["path"].as_str().unwrap_or("a file");
    let upstream = f.cause_title.as_deref().unwrap_or("a task it depends on");
    match f.cause.as_str() {
        "evidence_changed" if d["new_sha256"].is_null() => format!("its evidence file {path} is gone"),
        "evidence_changed" => format!("its evidence file {path} changed"),
        "upstream_evidence_changed" => format!("\"{upstream}\", which it depends on, has a changed evidence file ({path})"),
        "upstream_forked" => format!("\"{upstream}\", which it depends on, is being redone"),
        _ => "an input changed".into(),
    }
}

pub async fn open_flags(state: &AppState, task: &Task) -> Vec<Value> {
    state.db.flags_of(&task.id, false).await.unwrap_or_default().iter().map(flag_json).collect()
}

/// The `needs_you` items of the task's open flags, for every state.
pub async fn flag_items(state: &AppState, task: &Task) -> Vec<Value> {
    let flags = state.db.flags_of(&task.id, false).await.unwrap_or_default();
    flags
        .iter()
        .map(|f| {
            let text = format!("Check this still holds: {}", cause_text(f));
            json!({"kind": "may_be_invalid", "id": f.id, "text": text.chars().take(200).collect::<String>()})
        })
        .collect()
}

/// Raises a flag on `task` unless the same one is open, then tells the board and the owner.
async fn raise(state: &AppState, task: &Task, cause: &str, cause_task_id: &str, detail: &Value) -> Option<TaskFlag> {
    let flag = match state.db.raise_flag(&task.id, cause, cause_task_id, detail).await {
        Ok(Some(f)) => f,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, task = %task.id, "could not raise a flag");
            return None;
        }
    };
    emit_task(state, task).await;
    flag_chat_line(state, task, &flag).await;
    Some(flag)
}

/// Tells the task's owner (in the mission's chat, or the task's own) that it may be invalid.
/// There is no tool to resolve it: the person decides.
async fn flag_chat_line(state: &AppState, t: &Task, f: &TaskFlag) {
    let text = format!("Flagged \"{}\": {}. It may no longer hold; the person decides on the board.", t.title, cause_text(f));
    let meta = json!({"kind": "mission", "task_id": t.id, "mission_id": t.mission_id, "flag_id": f.id});
    let m = match &t.mission_id {
        Some(m) => state.db.get_mission(m).await.ok().flatten(),
        None => None,
    };
    let sender = t.owner_kwant_id.clone().or_else(|| m.as_ref().map(|m| m.lead_kwant_id.clone())).unwrap_or_else(|| "kwant".into());
    match m {
        Some(m) => post_line(state, m.channel_id, m.direct_kwant_id, m.thread_id, &sender, text, meta).await,
        None => post_line(state, t.channel_id.clone(), t.direct_kwant_id.clone(), t.thread_id.clone(), &sender, text, meta).await,
    }
}

// The drift scan

#[derive(Debug, Default)]
pub struct ScanResult {
    pub scanned: usize,
    pub flagged: Vec<String>,
    pub skipped: Vec<Value>,
}

/// The sha256 of `path`, `Ok(None)` when it is gone, or why it was skipped.
async fn hash_file(path: std::path::PathBuf) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("can't be read".to_string()),
        };
        if !meta.is_file() {
            return Err("not a file".into());
        }
        if meta.len() > SCAN_MAX_BYTES {
            return Err("over 50 MB".into());
        }
        let mut file = std::fs::File::open(&path).map_err(|_| "can't be read".to_string())?;
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        std::io::copy(&mut file, &mut hasher).map_err(|_| "can't be read".to_string())?;
        Ok(Some(hex(&sha2::Digest::finalize(hasher))))
    })
    .await
    .unwrap_or_else(|_| Err("can't be read".into()))
}

/// Hashes the file evidence of the mission's verified tasks (the latest row per path) and flags
/// each task whose file changed or is gone, plus its started direct dependents. Idempotent: an
/// open flag isn't raised twice, and a drift already flagged (even one the person called still
/// valid) isn't raised again.
pub async fn scan_mission(state: &AppState, mission: &Mission) -> ScanResult {
    let mut out = ScanResult::default();
    for task in state.db.verified_in_mission(&mission.id).await.unwrap_or_default() {
        let evidence = state.db.list_evidence(&task.id).await.unwrap_or_default();
        // Latest row per path: a file captured twice is judged against its last hash
        let mut latest: Vec<(String, String)> = Vec::new();
        for e in evidence.iter().rev().filter(|e| e.kind == "file") {
            let (Some(path), Some(sha)) = (&e.path, &e.sha256) else { continue };
            if !latest.iter().any(|(p, _)| p == path) {
                latest.push((path.clone(), sha.clone()));
            }
        }
        latest.reverse();
        let folder = task_folder(state, &task).await;
        for (path, old) in latest {
            let skip = |why: &str| json!({"task_id": task.id, "path": path, "why": why});
            if out.scanned >= SCAN_MAX_FILES {
                out.skipped.push(skip("over the 200-file scan limit"));
                continue;
            }
            let Some(folder) = &folder else {
                out.skipped.push(skip("the task has no folder"));
                continue;
            };
            let real = match inside(folder, &path) {
                Ok(p) => p,
                Err(why) => {
                    out.skipped.push(skip(&why));
                    continue;
                }
            };
            let new = match hash_file(real).await {
                Ok(h) => h,
                Err(why) => {
                    out.skipped.push(skip(&why));
                    continue;
                }
            };
            out.scanned += 1;
            if new.as_deref() == Some(old.as_str()) {
                continue;
            }
            let detail = json!({"path": path, "old_sha256": old, "new_sha256": new});
            if !state.db.drift_known(&task.id, "evidence_changed", &task.id, &path, new.as_deref()).await.unwrap_or(true) {
                if let Some(f) = raise(state, &task, "evidence_changed", &task.id, &detail).await {
                    out.flagged.push(f.id);
                }
            }
            for dep in state.db.started_dependents(&task.id).await.unwrap_or_default() {
                if state.db.drift_known(&dep.id, "upstream_evidence_changed", &task.id, &path, new.as_deref()).await.unwrap_or(true) {
                    continue;
                }
                if let Some(f) = raise(state, &dep, "upstream_evidence_changed", &task.id, &detail).await {
                    out.flagged.push(f.id);
                }
            }
        }
    }
    if let Err(e) = state.db.record_graph_scan(&mission.id, &json!(out.skipped)).await {
        tracing::warn!(error = %e, mission = %mission.id, "could not record the graph scan");
    }
    if !out.flagged.is_empty() {
        emit_mission(state, &mission.id).await;
    }
    out
}

/// Scans the mission if its last scan is older than [`SCAN_EVERY_MINUTES`]; for the runner tick.
pub async fn scan_if_due(state: &AppState, mission: &Mission) {
    let due = match state.db.graph_scan(&mission.id).await {
        Ok(Some(s)) => chrono::Utc::now() - s.scanned_at >= chrono::Duration::minutes(SCAN_EVERY_MINUTES),
        Ok(None) => true,
        Err(_) => false,
    };
    if due {
        scan_mission(state, mission).await;
    }
}

// Forks

/// Forks `orig` to redo it (see `Database::fork_task`). A done mission opens again so the fork
/// runs; a cancelled one refuses. Forking a verified task flags its started direct dependents
/// (`upstream_forked`); they keep depending on the original until the person decides.
/// With `claim`, the flag is resolved in the fork's own transaction: a flag that is already
/// resolved is a 409 and makes no fork, so racing redos make one copy.
pub async fn fork(state: &AppState, orig: &Task, reason: &str, claim: Option<&FlagClaim<'_>>) -> Result<Task, Response> {
    if let Some(mid) = &orig.mission_id {
        let m = match state.db.get_mission(mid).await {
            Ok(Some(m)) => m,
            Ok(None) => return Err(not_found("mission")),
            Err(e) => return Err(internal(e)),
        };
        match m.status.as_str() {
            "cancelled" => return Err(conflict("the mission is cancelled")),
            "done" if state.db.set_mission_status(mid, "done", "active", Some("person")).await.unwrap_or(false) => {
                super::mission_chat_line(state, &m, format!("Mission \"{}\" is open again: \"{}\" is being redone.", m.title, orig.title)).await;
            }
            _ => {}
        }
    }
    let f = match state.db.fork_task(orig, reason, claim).await {
        Ok(Some(f)) => f,
        Ok(None) => return Err(conflict("this flag is already resolved")),
        Err(e) => return Err(internal(e)),
    };
    if orig.state == "verified" {
        let detail = json!({"fork_id": f.id, "reason": reason});
        for dep in state.db.started_dependents(&orig.id).await.unwrap_or_default() {
            raise(state, &dep, "upstream_forked", &orig.id, &detail).await;
        }
    }
    emit_task_id(state, &orig.id).await;
    emit_task(state, &f).await;
    super::promote_one(state, &f).await;
    if let Some(mid) = &f.mission_id {
        emit_mission(state, mid).await;
    }
    Ok(state.db.get_task(&f.id).await.ok().flatten().unwrap_or(f))
}

// Idempotency

static IDEMPOTENT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Runs `f` once per `Idempotency-Key`: the same key on the same route returns the stored
/// response, the same key on another route is refused (422). Only successful responses are
/// stored, for 24 h. Without the header `f` just runs.
pub async fn idempotent<F, Fut>(state: &AppState, headers: &HeaderMap, route: String, f: F) -> Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, Response>>,
{
    let key = match headers.get("idempotency-key").map(|v| v.to_str().map(str::trim)) {
        None => return f().await.map_or_else(|r| r, |v| Json(v).into_response()),
        Some(Ok(k)) if !k.is_empty() && k.len() <= 200 => k.to_string(),
        Some(_) => return bad("Idempotency-Key must be 1–200 visible characters"),
    };
    // One keyed request at a time, so two retries with the same key can't both run
    let _held = IDEMPOTENT.lock().await;
    match state.db.idempotent_response(&key).await {
        Ok(Some((r, _))) if r != route => {
            return super::routes::err(StatusCode::UNPROCESSABLE_ENTITY, "that Idempotency-Key was used for another request");
        }
        Ok(Some((_, stored))) => {
            let v: Value = serde_json::from_str(&stored).unwrap_or(Value::Null);
            return ([("idempotent-replayed", "true")], Json(v)).into_response();
        }
        Ok(None) => {}
        Err(e) => return internal(e),
    }
    match f().await {
        Ok(v) => {
            if let Err(e) = state.db.store_idempotent_response(&key, &route, &v).await {
                tracing::warn!(error = %e, "could not store an idempotent response");
            }
            Json(v).into_response()
        }
        Err(r) => r,
    }
}

// Routes

#[derive(Deserialize)]
pub struct ForkBody {
    #[serde(default)]
    reason: String,
    from_checkpoint: Option<String>,
}

/// `POST /api/tasks/:id/fork` (person-only; `Idempotency-Key` optional).
pub async fn fork_task(State(state): State<AppState>, RawQuery(q): RawQuery, headers: HeaderMap, Path(id): Path<String>, Json(p): Json<ForkBody>) -> Response {
    let reason = p.reason.trim().to_string();
    if reason.is_empty() || reason.chars().count() > 500 {
        return bad("reason is required (at most 500 characters)");
    }
    let route = format!("POST /api/tasks/{id}/fork");
    idempotent(&state, &headers, route, || async {
        let orig = load_task(&state, q.as_deref(), &id).await?;
        // The fork starts from the checkpoint: the original's task-owned paths go back first
        let restore = match p.from_checkpoint.as_deref().map(str::trim) {
            Some(cid) => Some(super::checkpoints::restore_for_fork(&state, &orig, cid).await?),
            None => None,
        };
        let f = fork(&state, &orig, &reason, None).await?;
        let mut out = task_json(&state, &f).await;
        if let Some(r) = restore {
            out["restore"] = r;
        }
        Ok(out)
    })
    .await
}

#[derive(Deserialize)]
pub struct ResolveBody {
    resolution: String,
    note: Option<String>,
}

/// `POST /api/tasks/:id/flags/:flag_id/resolve` (person-only; `Idempotency-Key` optional, so
/// two Redo clicks from two tabs make one fork).
pub async fn resolve_flag(
    State(state): State<AppState>,
    RawQuery(q): RawQuery,
    headers: HeaderMap,
    Path((id, flag_id)): Path<(String, String)>,
    Json(p): Json<ResolveBody>,
) -> Response {
    if !FLAG_RESOLUTIONS.contains(&p.resolution.as_str()) {
        return bad("resolution must be still_valid, redo or cancelled");
    }
    let note = p.note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(500).collect::<String>());
    let route = format!("POST /api/tasks/{id}/flags/{flag_id}/resolve");
    idempotent(&state, &headers, route, || async {
        let t = load_task(&state, q.as_deref(), &id).await?;
        let flag = match state.db.get_flag(&flag_id).await {
            Ok(Some(f)) if f.task_id == t.id => f,
            Ok(_) => return Err(not_found("flag")),
            Err(e) => return Err(internal(e)),
        };
        if flag.resolved_at.is_some() {
            return Err(conflict("this flag is already resolved"));
        }
        let mut fork_out = None;
        match (p.resolution.as_str(), t.state.as_str()) {
            ("still_valid", _) => {}
            ("redo", "working" | "needs_you") => return Err(conflict("it's running; resolve when it stops")),
            ("redo", "cancelled") => return Err(conflict("the task is cancelled; fork it to redo it")),
            ("redo", "verified") => {
                let reason = note.clone().unwrap_or_else(|| "redo: it may no longer hold".into());
                // The fork claims the flag first, in its own transaction
                let claim = FlagClaim { id: &flag.id, resolution: &p.resolution, note: note.as_deref() };
                fork_out = Some(fork(&state, &t, &reason, Some(&claim)).await?);
            }
            ("redo", from @ ("review" | "failed")) => {
                let change = TaskChange { reason: Some("redo: it may no longer hold".into()), detail: Some(json!({"flag_id": flag.id})), ..Default::default() };
                moved(transition(&state, &t.id, from, "ready", Actor::Person, "person", change).await)?;
                let _ = state.db.reset_attempts(&t.id).await;
            }
            ("redo", "blocked") => {
                let change = TaskChange { reason: Some("redo: it may no longer hold".into()), detail: Some(json!({"flag_id": flag.id})), ..Default::default() };
                let t = moved(transition(&state, &t.id, "blocked", "backlog", Actor::Person, "person", change).await)?;
                super::promote_one(&state, &t).await;
            }
            // backlog or ready: it runs again anyway
            ("redo", _) => {}
            ("cancelled", "verified") => return Err(conflict("a verified task can't be cancelled; choose still_valid or redo")),
            ("cancelled", "cancelled") => {}
            ("cancelled", from) => {
                let change = TaskChange { reason: Some("cancelled by the person: it may no longer hold".into()), ..Default::default() };
                moved(transition(&state, &t.id, from, "cancelled", Actor::Person, "person", change).await)?;
            }
            _ => return Err(bad("resolution must be still_valid, redo or cancelled")),
        }
        if fork_out.is_none() {
            match state.db.resolve_flag(&flag.id, &p.resolution, note.as_deref(), None).await {
                Ok(true) => {}
                Ok(false) => return Err(conflict("this flag is already resolved")),
                Err(e) => return Err(internal(e)),
            }
        }
        let t = state.db.get_task(&t.id).await.ok().flatten().unwrap_or(t);
        emit_task(&state, &t).await;
        if let Some(mid) = &t.mission_id {
            emit_mission(&state, mid).await;
        }
        let flag = state.db.get_flag(&flag.id).await.ok().flatten().unwrap_or(flag);
        let fork_json = match &fork_out {
            Some(f) => task_json(&state, f).await,
            None => Value::Null,
        };
        Ok(json!({"flag": flag_json(&flag), "task": task_json(&state, &t).await, "fork": fork_json}))
    })
    .await
}

/// `GET /api/tasks/:id/flags` (open ones; `?all=1` adds resolved ones).
pub async fn task_flags(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let all = q.as_deref().unwrap_or("").split('&').any(|p| matches!(p, "all=1" | "all=true"));
    match state.db.flags_of(&t.id, all).await {
        Ok(flags) => Json(json!({"flags": flags.iter().map(flag_json).collect::<Vec<_>>()})).into_response(),
        Err(e) => internal(e),
    }
}

/// `POST /api/missions/:id/graph/scan` (person-only).
pub async fn scan(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if m.status == "cancelled" {
        return conflict("the mission is cancelled");
    }
    let r = scan_mission(&state, &m).await;
    Json(json!({"scanned": r.scanned, "flagged": r.flagged, "skipped": r.skipped})).into_response()
}

/// `GET /api/missions/:id/graph`.
pub async fn mission_graph(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let space = crate::db::spaces::SpaceId::new(&m.space_id);
    let tasks = match state.db.list_tasks(&space, None, Some(&m.id), None, 1000).await {
        Ok(t) => t,
        Err(e) => return internal(e),
    };
    let scan = state.db.graph_scan(&m.id).await.ok().flatten();
    graph_out(&state, Some(&m.id), &tasks, scan).await
}

/// `GET /api/graph`: the active space's tasks without a mission, open or finished in the last 7 days.
pub async fn loose_graph(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let since = chrono::Utc::now() - chrono::Duration::days(7);
    match state.db.loose_tasks(&space, since).await {
        Ok(tasks) => graph_out(&state, None, &tasks, None).await,
        Err(e) => internal(e),
    }
}

async fn graph_out(state: &AppState, mission_id: Option<&str>, tasks: &[Task], scan: Option<GraphScan>) -> Response {
    let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
    let edges = match state.db.edges_among(&ids).await {
        Ok(e) => e,
        Err(e) => return internal(e),
    };
    let downstream = downstream_counts(&ids, &edges);
    let mut nodes = Vec::with_capacity(tasks.len());
    for t in tasks {
        let checks = state.db.list_checks(&t.id).await.unwrap_or_default();
        nodes.push(json!({
            "id": t.id,
            "title": t.title,
            "state": t.state,
            "owner_kwant_id": t.owner_kwant_id,
            "owner_name": name_of(state, t.owner_kwant_id.as_deref()).await,
            "attempts": t.attempts,
            "max_attempts": t.max_attempts,
            "forked_from": t.forked_from,
            "downstream": downstream.get(t.id.as_str()).copied().unwrap_or(0),
            "flags": open_flags(state, t).await,
            "checks_summary": super::checks_summary(t, &checks),
        }));
    }
    let (scanned_at, skipped) = match scan {
        Some(s) => (json!(s.scanned_at), serde_json::from_str(&s.skipped).unwrap_or_else(|_| json!([]))),
        None => (Value::Null, json!([])),
    };
    Json(json!({
        "mission_id": mission_id,
        "nodes": nodes,
        "edges": edges.iter().map(|(from, to)| json!({"from": from, "to": to})).collect::<Vec<_>>(),
        "scanned_at": scanned_at,
        "scan_skipped": skipped,
    }))
    .into_response()
}

/// For each task, how many tasks depend on it directly or further down (the graph shows "N more
/// downstream" next to a flag without flagging them, decision D2).
pub fn downstream_counts<'a>(ids: &'a [String], edges: &[(String, String)]) -> HashMap<&'a str, usize> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in edges {
        children.entry(from.as_str()).or_default().push(to.as_str());
    }
    let mut out = HashMap::new();
    for id in ids {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack: Vec<&str> = children.get(id.as_str()).cloned().unwrap_or_default();
        while let Some(n) = stack.pop() {
            if seen.insert(n) {
                stack.extend(children.get(n).cloned().unwrap_or_default());
            }
        }
        out.insert(id.as_str(), seen.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downstream_counts_every_task_below_once() {
        let ids: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let e = |a: &str, b: &str| (a.to_string(), b.to_string());
        // a → b → c, a → c, d alone
        let edges = vec![e("a", "b"), e("b", "c"), e("a", "c")];
        let n = downstream_counts(&ids, &edges);
        assert_eq!(n["a"], 2);
        assert_eq!(n["b"], 1);
        assert_eq!(n["c"], 0);
        assert_eq!(n["d"], 0);
    }
}
