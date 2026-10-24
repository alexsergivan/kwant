//! The app's Setup checklist (person-only, see `auth::person_only`):
//!
//! - `GET /api/setup/status`: every part with its state and what Install would run, plus the
//!   `kwnt doctor` checks.
//! - `POST /api/setup/install {"item": "laya"}`: starts installing one part (202 with a job id).
//!   One install at a time (409 while one runs).
//! - `GET /api/setup/jobs/:id?after=N`: the job's state and its output lines from line `N` on;
//!   poll with `after` = the last `next` to follow the install live.
//!
//! The same plans as `kwnt setup` (`crate::setup`): nothing runs with sudo; steps that need it
//! come back as `manual` commands for the person.

use super::AppState;
use crate::setup::{self, ItemId, Step};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

/// Output lines kept per job; older ones are dropped (`first` moves on).
const MAX_LINES: usize = 2000;
/// Finished jobs kept for late readers.
const MAX_JOBS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub id: String,
    pub item: ItemId,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// Index of `lines[0]` in the whole output.
    #[serde(skip)]
    first: usize,
    #[serde(skip)]
    lines: Vec<String>,
}

#[derive(Default)]
pub struct SetupJobs {
    inner: Mutex<Jobs>,
}

#[derive(Default)]
struct Jobs {
    next_id: u64,
    order: Vec<String>,
    jobs: HashMap<String, Job>,
    running: Option<String>,
}

impl SetupJobs {
    /// Registers a running job, or returns the one already running.
    fn begin(&self, item: ItemId) -> Result<String, String> {
        let mut j = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(id) = &j.running {
            return Err(id.clone());
        }
        j.next_id += 1;
        let id = format!("setup-{}", j.next_id);
        j.jobs.insert(
            id.clone(),
            Job {
                id: id.clone(),
                item,
                state: JobState::Running,
                error: None,
                started_at: chrono::Utc::now().to_rfc3339(),
                finished_at: None,
                first: 0,
                lines: Vec::new(),
            },
        );
        j.order.push(id.clone());
        while j.order.len() > MAX_JOBS {
            let old = j.order.remove(0);
            j.jobs.remove(&old);
        }
        j.running = Some(id.clone());
        Ok(id)
    }

    fn push(&self, id: &str, line: String) {
        let mut j = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = j.jobs.get_mut(id) {
            job.lines.push(line);
            if job.lines.len() > MAX_LINES {
                let drop = job.lines.len() - MAX_LINES;
                job.lines.drain(..drop);
                job.first += drop;
            }
        }
    }

    fn finish(&self, id: &str, result: Result<(), String>) {
        let mut j = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(job) = j.jobs.get_mut(id) {
            job.state = if result.is_ok() { JobState::Done } else { JobState::Failed };
            job.error = result.err();
            job.finished_at = Some(chrono::Utc::now().to_rfc3339());
        }
        if j.running.as_deref() == Some(id) {
            j.running = None;
        }
    }

    pub fn running(&self) -> Option<String> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).running.clone()
    }

    /// The job and its lines from `after` on, and the index to ask for next.
    fn read(&self, id: &str, after: usize) -> Option<(Job, Vec<String>, usize)> {
        let j = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let job = j.jobs.get(id)?;
        let from = after.saturating_sub(job.first).min(job.lines.len());
        let next = job.first + job.lines.len();
        Some((job.clone(), job.lines[from..].to_vec(), next))
    }
}

#[derive(Serialize)]
struct CheckView {
    name: String,
    status: &'static str,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
    optional: bool,
}

fn status_str(s: crate::doctor::Status) -> &'static str {
    match s {
        crate::doctor::Status::Ok => "ok",
        crate::doctor::Status::Warn => "warn",
        crate::doctor::Status::Fail => "fail",
        crate::doctor::Status::Skip => "skip",
    }
}

/// `GET /api/setup/status`
pub async fn status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (probe, facts) = tokio::join!(setup::probe(&state.config), crate::doctor::gather(&state.config));
    let signed_in: Vec<_> = facts.clis.iter().map(|(e, _, s, key)| (*e, if *key { Some(true) } else { *s })).collect();
    let items = setup::items(&probe, &signed_in);
    let checks: Vec<CheckView> = crate::doctor::checks(&facts, probe.platform)
        .into_iter()
        .map(|c| CheckView { name: c.name, status: status_str(c.status), detail: c.detail, fix: c.fix, optional: c.optional })
        .collect();
    Json(json!({
        "platform": match probe.platform {
            crate::doctor::Platform::Mac => "mac",
            crate::doctor::Platform::Linux => "linux",
            crate::doctor::Platform::Docker => "docker",
        },
        "items": items,
        "checks": checks,
        "running_job": state.setup.running(),
    }))
}

#[derive(Deserialize)]
pub struct InstallBody {
    pub item: String,
}

fn err(code: StatusCode, body: serde_json::Value) -> Response {
    (code, Json(body)).into_response()
}

/// `POST /api/setup/install`
pub async fn install(State(state): State<AppState>, Json(body): Json<InstallBody>) -> Response {
    let Some(item) = ItemId::parse(&body.item) else {
        return err(
            StatusCode::BAD_REQUEST,
            json!({ "error": format!("unknown part '{}'; use laya, browser, claude, codex, grok or docker", body.item) }),
        );
    };
    let probe = setup::probe(&state.config).await;
    if probe.platform == crate::doctor::Platform::Docker {
        return err(
            StatusCode::CONFLICT,
            json!({ "error": "kwant runs in a Docker container; its parts come from the compose stack (see kwnt doctor)" }),
        );
    }
    if item == ItemId::Laya
        && matches!(
            state.router.status().state,
            crate::router::sidecar::LayaState::Installing | crate::router::sidecar::LayaState::Starting
        )
    {
        return err(StatusCode::CONFLICT, json!({ "error": "kwant is already installing or starting Laya; see the Laya card" }));
    }
    let plan = setup::plan(item, &probe);
    let manual: Vec<serde_json::Value> = plan
        .manual()
        .filter_map(|s| match s {
            Step::Manual { label, command } => Some(json!({ "label": label, "command": command })),
            _ => None,
        })
        .collect();
    if plan.steps.is_empty() {
        return (StatusCode::OK, Json(json!({ "item": item, "already_installed": true }))).into_response();
    }
    if !plan.automatic() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "error": "this part needs a step only you can do", "item": item, "manual": manual }),
        );
    }
    let id = match state.setup.begin(item) {
        Ok(id) => id,
        Err(running) => {
            return err(StatusCode::CONFLICT, json!({ "error": "another install is running", "job_id": running }));
        }
    };
    let steps: Vec<Step> = plan.steps.into_iter().filter(|s| !matches!(s, Step::Manual { .. })).collect();
    let run = setup::Plan { item, steps: steps.clone() };
    let jobs = state.setup.clone();
    let job_id = id.clone();
    let st = state.clone();
    tokio::spawn(async move {
        let log_jobs = jobs.clone();
        let log_id = job_id.clone();
        let log = move |line: String| log_jobs.push(&log_id, line);
        let result = setup::run_plan(&run, &st.config, &log).await;
        if result.is_ok() && item.engine().is_some() {
            crate::drivers::forget_cli_binaries();
            let env = super::engine_routes::probe_env(&st).await;
            st.engines.recheck(&env).await;
        }
        match &result {
            Ok(()) => tracing::info!(item = item.as_str(), "setup: installed"),
            Err(e) => tracing::warn!(item = item.as_str(), error = %e, "setup: install failed"),
        }
        jobs.finish(&job_id, result);
    });
    (StatusCode::ACCEPTED, Json(json!({ "job_id": id, "item": item, "steps": steps, "manual": manual }))).into_response()
}

#[derive(Deserialize)]
pub struct JobQuery {
    #[serde(default)]
    pub after: usize,
}

/// `GET /api/setup/jobs/:id?after=N`
pub async fn job(State(state): State<AppState>, Path(id): Path<String>, Query(q): Query<JobQuery>) -> Response {
    match state.setup.read(&id, q.after) {
        Some((job, lines, next)) => Json(json!({
            "id": job.id,
            "item": job.item,
            "state": job.state,
            "error": job.error,
            "started_at": job.started_at,
            "finished_at": job.finished_at,
            "lines": lines,
            "next": next,
        }))
        .into_response(),
        None => err(StatusCode::NOT_FOUND, json!({ "error": "no such setup job" })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_job_at_a_time_and_lines_page_by_index() {
        let jobs = std::sync::Arc::new(SetupJobs::default());
        let id = jobs.begin(ItemId::Laya).unwrap();
        assert_eq!(jobs.begin(ItemId::Browser), Err(id.clone()));
        for i in 0..5 {
            jobs.push(&id, format!("line {i}"));
        }
        let (job, lines, next) = jobs.read(&id, 3).unwrap();
        assert_eq!(job.state, JobState::Running);
        assert_eq!(lines, vec!["line 3", "line 4"]);
        assert_eq!(next, 5);
        jobs.finish(&id, Err("boom".into()));
        let (job, lines, _) = jobs.read(&id, 5).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.error.as_deref(), Some("boom"));
        assert!(lines.is_empty());
        assert!(jobs.running().is_none());
        assert!(jobs.begin(ItemId::Browser).is_ok());
    }

    #[test]
    fn old_lines_are_dropped_but_indexes_stay_stable() {
        let jobs = SetupJobs::default();
        let id = jobs.begin(ItemId::Laya).unwrap();
        for i in 0..MAX_LINES + 10 {
            jobs.push(&id, format!("{i}"));
        }
        let (_, lines, next) = jobs.read(&id, 0).unwrap();
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[0], "10");
        assert_eq!(next, MAX_LINES + 10);
        let (_, lines, _) = jobs.read(&id, MAX_LINES + 8).unwrap();
        assert_eq!(lines, vec![format!("{}", MAX_LINES + 8), format!("{}", MAX_LINES + 9)]);
    }
}
