//! Routines: the HTTP API and the scheduler that runs them. One tokio task sleeps until the next
//! routine is due (at most [`MAX_SLEEP`], or until a routine changes) and starts due runs; runs
//! missed while kwnt was down are skipped, not replayed.

// Handler helpers return the ready-made error `Response`
#![allow(clippy::result_large_err)]

use super::continuity::{notify, Notification};
use super::spaces::Scope;
use super::AppState;
use crate::db::continuity::{NewRoutine, RoutineRow, RoutineRun};
use crate::drivers::events::RuntimeEvent;
use crate::routines::Schedule;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Longest the scheduler sleeps between checks.
pub const MAX_SLEEP: Duration = Duration::from_secs(30);
const MAX_NAME_CHARS: usize = 100;
const MAX_ROUTINE_INSTRUCTIONS_CHARS: usize = 4000;
pub(crate) const MAX_ACCEPTANCE_CHARS: usize = 4000;
/// Checks one routine copies onto each run's task.
const MAX_ROUTINE_CHECKS: usize = 10;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// A routine whose agent and channel are in different spaces is a 400; anything else is a 500.
fn db_error(e: crate::db::DbError) -> Response {
    match e {
        crate::db::DbError::OtherSpace(m) => (StatusCode::BAD_REQUEST, m).into_response(),
        _ => internal(),
    }
}

fn internal() -> Response {
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// A routine as the API returns it.
#[derive(Debug, Serialize)]
pub struct RoutineView {
    pub id: String,
    pub kwant_id: String,
    /// `null`: the kwant's direct chat.
    pub channel_id: Option<String>,
    pub name: String,
    pub instructions: String,
    pub schedule: serde_json::Value,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    /// The last run's error, e.g. "You said no to Bash; nothing ran" (28b round 2), so the routine
    /// card can tell a no from a failure. `null`: no run yet, or it had none.
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    /// The schedule in words, e.g. "monthly on the 1st at 09:00 Europe/Berlin".
    pub schedule_text: String,
    /// Set when it delivers into one of its bot's threads.
    pub thread_id: Option<String>,
    /// The message it follows up on (in the chat it delivers to), when it is a follow-up.
    pub context_message_id: Option<String>,
    /// `"discovery"` when the person accepted an Automation Discovery suggestion; else `null`.
    pub origin: Option<String>,
    pub suggestion_id: Option<String>,
    /// Each run's board task gets this Flight School class (`null`: `general`).
    pub task_class: Option<String>,
    /// Each run's board task's acceptance (`null`: the instructions, as before).
    pub acceptance: Option<String>,
    /// `[{kind, spec, required}]` each run's task gets, approved by the person; specs are shown
    /// with secrets masked. `[]`: none.
    pub checks: serde_json::Value,
}

impl From<RoutineRow> for RoutineView {
    fn from(r: RoutineRow) -> Self {
        let schedule: serde_json::Value = serde_json::from_str(&r.schedule).unwrap_or(serde_json::Value::Null);
        Self {
            schedule_text: super::bot_actions::schedule_text(&schedule),
            schedule,
            thread_id: r.thread_id,
            context_message_id: r.context_message_id,
            checks: shown_checks(r.checks.as_deref()),
            origin: r.origin,
            suggestion_id: r.suggestion_id,
            task_class: r.task_class,
            acceptance: r.acceptance,
            id: r.id,
            kwant_id: r.kwant_id,
            channel_id: r.channel_id,
            name: r.name,
            instructions: r.instructions,
            enabled: r.enabled,
            next_run_at: r.next_run_at,
            last_run_at: r.last_run_at,
            last_status: r.last_status,
            last_error: r.last_error,
            created_at: r.created_at,
        }
    }
}

/// A routine's stored checks, specs masked as everywhere checks are shown.
fn shown_checks(stored: Option<&str>) -> serde_json::Value {
    let list: Vec<serde_json::Value> = stored.and_then(|c| serde_json::from_str(c).ok()).unwrap_or_default();
    serde_json::Value::Array(
        list.into_iter()
            .map(|mut c| {
                c["spec"] = super::missions::autofix::redact_value(&c["spec"]);
                c
            })
            .collect(),
    )
}

/// One check a routine copies onto each run's task.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoutineCheck {
    pub kind: String,
    #[serde(default)]
    pub spec: serde_json::Value,
    #[serde(default = "yes")]
    pub required: bool,
}

fn yes() -> bool {
    true
}

/// `POST /api/routines` body; `PATCH` takes the same fields, all optional.
#[derive(Debug, Deserialize, Default)]
pub struct RoutinePayload {
    pub kwant_id: Option<String>,
    /// `""` or `null`: the kwant's direct chat.
    #[serde(default, deserialize_with = "double_option")]
    pub channel_id: Option<Option<String>>,
    pub name: Option<String>,
    pub instructions: Option<String>,
    pub schedule: Option<Schedule>,
    pub enabled: Option<bool>,
    /// One of the bot's own threads to deliver into, instead of a channel or the direct chat.
    #[serde(default, deserialize_with = "double_option")]
    pub thread_id: Option<Option<String>>,
    /// A message in the delivery chat this routine follows up on.
    #[serde(default, deserialize_with = "double_option")]
    pub context_message_id: Option<Option<String>>,
    /// The Flight School class each run's task gets; `null`, `""` or `"general"` clears it.
    /// The person's only, like the checks.
    #[serde(default, deserialize_with = "double_option")]
    pub task_class: Option<Option<String>>,
    /// What a good result is, for each run's task; `null` or `""`: the instructions.
    #[serde(default, deserialize_with = "double_option")]
    pub acceptance: Option<Option<String>>,
    /// Checks each run's task gets, approved by the person; `null` or `[]` clears them. The
    /// person's only: a bot's token gets a 403.
    #[serde(default, deserialize_with = "double_option")]
    pub checks: Option<Option<Vec<RoutineCheck>>>,
}

/// Tells a missing field (`None`) from an explicit `null` (`Some(None)`).
pub(crate) fn double_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Ok(Some(Option::<T>::deserialize(d)?))
}

/// Who sends a routine change: the class and checks a run's task gets are the person's to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Person,
    Bot,
}

impl RoutinePayload {
    fn sets_person_fields(&self) -> bool {
        self.task_class.is_some() || self.checks.is_some()
    }
}

/// Merges the class, acceptance and checks of `p` over `existing` and checks them: the JSON to
/// store for each (`None`: not set).
#[allow(clippy::type_complexity)]
fn build_outcome(
    p: &RoutinePayload,
    existing: Option<&RoutineRow>,
) -> Result<(Option<String>, Option<String>, Option<String>), Response> {
    let bad = |m: &str| error(StatusCode::BAD_REQUEST, m);
    let task_class = match &p.task_class {
        Some(c) => c.as_deref().map(str::trim).filter(|c| !c.is_empty() && *c != crate::db::flight_school::DEFAULT_CLASS).map(str::to_string),
        None => existing.and_then(|e| e.task_class.clone()),
    };
    if task_class.as_deref().is_some_and(|c| !crate::db::flight_school::valid_class(c)) {
        return Err(bad("task_class must be 1–40 of a-z, 0-9 and -"));
    }
    let acceptance = match &p.acceptance {
        Some(a) => a.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string),
        None => existing.and_then(|e| e.acceptance.clone()),
    };
    if acceptance.as_deref().is_some_and(|a| a.chars().count() > MAX_ACCEPTANCE_CHARS) {
        return Err(bad("acceptance must be at most 4000 characters"));
    }
    let checks = match &p.checks {
        Some(list) => {
            let list = list.as_deref().unwrap_or_default();
            if list.len() > MAX_ROUTINE_CHECKS {
                return Err(bad("at most 10 checks"));
            }
            for c in list {
                // Shown masked; one sent back that way would check for the mask
                if c.spec.to_string().contains("[redacted]") {
                    return Err(bad("a check spec has masked values ([redacted]); send the real ones"));
                }
                super::missions::validate_check(&c.kind, &c.spec, true).map_err(|m| bad(&m))?;
            }
            (!list.is_empty()).then(|| serde_json::to_string(list).unwrap_or_default())
        }
        None => existing.and_then(|e| e.checks.clone()),
    };
    Ok((task_class, acceptance, checks))
}

/// Merges `p` over `existing` and checks the result.
pub(crate) async fn build(
    state: &AppState,
    p: RoutinePayload,
    existing: Option<&RoutineRow>,
) -> Result<NewRoutine, Response> {
    let bad = |m: &str| error(StatusCode::BAD_REQUEST, m);
    let (task_class, acceptance, checks) = build_outcome(&p, existing)?;
    let kwant_id = p
        .kwant_id
        .or_else(|| existing.map(|e| e.kwant_id.clone()))
        .ok_or_else(|| bad("kwant_id is required"))?;
    match state.db.get_kwant(&kwant_id).await {
        // Nothing new is scheduled in an archived space
        Ok(Some(k)) => super::spaces::ensure_active(state, &crate::db::spaces::SpaceId::new(&k.space_id)).await?,
        Ok(None) => return Err(bad("unknown kwant_id")),
        Err(_) => return Err(internal()),
    }
    // A new bot or chat leaves the old thread and the exchange it followed up on behind
    let moved = existing.is_some_and(|e| e.kwant_id != kwant_id) || p.channel_id.is_some();
    let thread_given = p.thread_id.is_some();
    let thread_id = match p.thread_id {
        Some(t) => t.filter(|t| !t.trim().is_empty()),
        None if moved => None,
        None => existing.and_then(|e| e.thread_id.clone()),
    };
    if let Some(t) = &thread_id {
        match state.db.get_thread(t).await {
            Ok(Some(t)) if t.kwant_id == kwant_id => {}
            Ok(_) => return Err(bad("thread_id must be one of the bot's own threads")),
            Err(_) => return Err(internal()),
        }
    }
    let channel_id = match p.channel_id {
        _ if thread_id.is_some() => None,
        Some(c) => c.filter(|c| !c.trim().is_empty()),
        None => existing.and_then(|e| e.channel_id.clone()),
    };
    if let Some(c) = &channel_id {
        match state.db.get_channel(c).await {
            Ok(Some(_)) => {}
            Ok(None) => return Err(bad("unknown channel_id")),
            Err(_) => return Err(internal()),
        }
    }
    let name = p
        .name
        .or_else(|| existing.map(|e| e.name.clone()))
        .unwrap_or_default()
        .trim()
        .to_string();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(bad("name must be 1-100 characters"));
    }
    let instructions = p
        .instructions
        .or_else(|| existing.map(|e| e.instructions.clone()))
        .unwrap_or_default()
        .trim()
        .to_string();
    if instructions.is_empty() || instructions.chars().count() > MAX_ROUTINE_INSTRUCTIONS_CHARS {
        return Err(bad("instructions must be 1-4000 characters"));
    }
    let schedule = match p.schedule {
        Some(s) => s,
        None => existing
            .and_then(|e| serde_json::from_str::<Schedule>(&e.schedule).ok())
            .ok_or_else(|| bad("schedule is required"))?,
    };
    schedule.validate().map_err(|m| bad(&m))?;
    let schedule = schedule.canonical();
    let enabled = p.enabled.or(existing.map(|e| e.enabled)).unwrap_or(true);
    let next_run_at = if enabled {
        schedule.next_after(Utc::now(), existing.and_then(|e| e.last_run_at))
    } else {
        None
    };
    if enabled && next_run_at.is_none() {
        return Err(bad("a 'once' routine needs a time in the future"));
    }
    let context_message_id = match p.context_message_id {
        Some(m) => m.filter(|m| !m.trim().is_empty()),
        None if moved || thread_given => None,
        None => existing.and_then(|e| e.context_message_id.clone()),
    };
    if let Some(m) = &context_message_id {
        let chat = delivery_chat(thread_id.as_deref(), channel_id.as_deref(), &kwant_id);
        match state.db.list_messages_around(chat, m, 1).await {
            Ok(Some(_)) => {}
            Ok(None) => return Err(bad("context_message_id must be a message in the chat the routine delivers to")),
            Err(_) => return Err(internal()),
        }
    }
    Ok(NewRoutine {
        kwant_id,
        channel_id,
        thread_id,
        context_message_id,
        name,
        instructions,
        schedule: serde_json::to_string(&schedule).unwrap_or_default(),
        enabled,
        next_run_at,
        origin: existing.and_then(|e| e.origin.clone()),
        suggestion_id: existing.and_then(|e| e.suggestion_id.clone()),
        task_class,
        acceptance,
        checks,
    })
}

pub async fn list(State(state): State<AppState>, Scope(space): Scope) -> Response {
    match state.db.list_routines(&space).await {
        Ok(list) => {
            Json(list.into_iter().map(RoutineView::from).collect::<Vec<_>>()).into_response()
        }
        Err(_) => internal(),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_routine(&id).await {
        Ok(Some(r)) => Json(RoutineView::from(r)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => internal(),
    }
}

/// `POST /api/routines`: the routine lives in its agent's space. An explicit `?space=` must
/// name that space; without one the agent decides, as before spaces had a picker.
pub async fn create(
    State(state): State<AppState>,
    Scope(space): Scope,
    uri: axum::http::Uri,
    headers: HeaderMap,
    Json(p): Json<RoutinePayload>,
) -> Response {
    let by = caller(&state, &headers).await;
    create_by(&state, space, uri, p, by).await
}

/// A bot's turn token, or a bearer that isn't the API token, is a bot; anything else is the person
/// (auth already let the request in).
async fn caller(state: &AppState, headers: &HeaderMap) -> By {
    match super::mcp_suggest::refuse_agent_tokens(state, headers, "").await {
        Ok(()) => By::Person,
        Err(_) => By::Bot,
    }
}

fn person_fields_refused() -> Response {
    error(StatusCode::FORBIDDEN, "only the person sets a routine's class and checks")
}

/// [`create`] for in-process callers: a bot's own tools pass [`By::Bot`].
pub async fn create_by(state: &AppState, space: crate::db::spaces::SpaceId, uri: axum::http::Uri, p: RoutinePayload, by: By) -> Response {
    if by == By::Bot && p.sets_person_fields() {
        return person_fields_refused();
    }
    let input = match build(state, p, None).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let explicit = super::spaces::space_param(uri.query()).is_some();
    match state.db.space_of_kwant(&input.kwant_id).await {
        Ok(Some(of)) if of == space || !explicit => {}
        Ok(_) => return error(StatusCode::BAD_REQUEST, "the bot is in another space"),
        Err(_) => return internal(),
    }
    match state.db.create_routine(&input).await {
        Ok(r) => {
            state.scheduler_wake.notify_one();
            (StatusCode::CREATED, Json(RoutineView::from(r))).into_response()
        }
        Err(e) => db_error(e),
    }
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(p): Json<RoutinePayload>,
) -> Response {
    let by = caller(&state, &headers).await;
    update_by(&state, id, p, by).await
}

/// [`update`] for in-process callers: a bot's own tools pass [`By::Bot`].
pub async fn update_by(state: &AppState, id: String, p: RoutinePayload, by: By) -> Response {
    if by == By::Bot && p.sets_person_fields() {
        return person_fields_refused();
    }
    let existing = match state.db.get_routine(&id).await {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return internal(),
    };
    let input = match build(state, p, Some(&existing)).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    match state.db.update_routine(&id, &input).await {
        Ok(Some(r)) => {
            state.scheduler_wake.notify_one();
            // A discovered routine's switches feed its suggestion's history (design §10)
            if let (Some(sug), true) = (existing.suggestion_id.as_deref(), existing.enabled != r.enabled) {
                let actor = if by == By::Bot { "kwant" } else { "person" };
                if let Err(e) = state.db.suggestion_switched(sug, r.enabled, actor).await {
                    tracing::warn!(error = %e, routine = %id, "recording a discovered routine's switch");
                }
            }
            Json(RoutineView::from(r)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => db_error(e),
    }
}

pub async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.delete_routine(&id).await {
        Ok(true) => {
            state.scheduler_wake.notify_one();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => internal(),
    }
}

#[derive(Deserialize)]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

pub async fn runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RunsQuery>,
) -> Response {
    match state.db.get_routine(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return internal(),
    }
    match state
        .db
        .list_routine_runs(&id, q.limit.unwrap_or(20).clamp(1, 200))
        .await
    {
        Ok(list) => Json(list).into_response(),
        Err(_) => internal(),
    }
}

/// `POST /api/routines/:id/run`: starts a run now (202 with the run). If the routine is already
/// running, the run is recorded as `skipped` and returned, still with 202.
pub async fn run_now(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let routine = match state.db.get_routine(&id).await {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return internal(),
    };
    if let Err(r) = super::spaces::ensure_active(&state, &crate::db::spaces::SpaceId::new(&routine.space_id)).await {
        return r;
    }
    match start_run(&state, routine, "manual").await {
        Some(run) => (StatusCode::ACCEPTED, Json(run)).into_response(),
        None => internal(),
    }
}

/// The trigger of AutoFix's own run of a failed routine (routine-retry-once). Like `schedule`, it is
/// no press: only `manual` (Run now) takes the press-only paths below. History says "Retried
/// automatically".
pub const RETRY: &str = "retry";

/// Records a run and starts it in the background, or records it as skipped when the routine is
/// still running. `None` if the run couldn't be recorded.
pub async fn start_run(state: &AppState, routine: RoutineRow, trigger: &str) -> Option<RoutineRun> {
    start_run_noted(state, routine, trigger, None).await
}

/// [`start_run`], with a note for the run's history (G2: "Ran late: … was paused at its monthly
/// budget." on the one run a routine gets after its bot's pause ends).
pub async fn start_run_noted(state: &AppState, routine: RoutineRow, trigger: &str, note: Option<&str>) -> Option<RoutineRun> {
    // Today's spending stop holds new turns: the run is skipped, not failed (no AutoFix retry)
    if let Some(why) = super::spend_stop::held(state, &routine.space_id).await {
        let run = state.db.start_routine_run(&routine.id, trigger, "skipped", Some(&why)).await.ok()?;
        send_run_event(state, &run);
        return Some(run);
    }
    // Its bot is paused at its monthly budget (G2): held, and run once when the pause ends. Only
    // the first held time goes in the history, so missed runs collapse into one late run
    if let Some(why) = super::bot_budget::held(state, &routine.kwant_id).await {
        // A scheduled time while it's already held adds nothing; Run now still gets its answer
        let first = state.db.hold_budget_routine(&routine.id, &routine.kwant_id).await.unwrap_or(false);
        if !first && trigger != "manual" {
            return None;
        }
        let run = state.db.start_routine_run(&routine.id, trigger, "skipped", Some(&why)).await.ok()?;
        send_run_event(state, &run);
        return Some(run);
    }
    let claimed = state
        .running_routines
        .lock()
        .await
        .insert(routine.id.clone());
    if !claimed {
        let run = state
            .db
            .start_routine_run(
                &routine.id,
                trigger,
                "skipped",
                Some("the previous run was still going"),
            )
            .await
            .ok()?;
        send_run_event(state, &run);
        return Some(run);
    }
    let run = match state
        .db
        .start_routine_run(&routine.id, trigger, "running", None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "could not record routine run");
            state.running_routines.lock().await.remove(&routine.id);
            return None;
        }
    };
    if let Some(note) = note {
        let _ = state.db.set_routine_run_note(&run.id, note).await;
    }
    // 26b: "Run now" counts as the routine's last run too. Only `last_run_at`: the next scheduled
    // run stays where the schedule put it
    if trigger == "manual" {
        let _ = state
            .db
            .set_routine_last_run(&routine.id, run.started_at)
            .await;
    }
    send_run_event(state, &run);
    let state = state.clone();
    let run_id = run.id.clone();
    tokio::spawn(async move {
        let result = execute(&state, &routine, &run_id).await;
        let (status, err, message_id) = match result {
            // A reply alone isn't "ok": what its steps did decides (26b)
            Ok(ran) => {
                let (status, why) = ran.outcome;
                (status, why, ran.reply_id)
            }
            // The stop came down while the run was starting: a skip, like one caught above
            Err(e) if super::spend_stop::is_stop_error(&e) => ("skipped", Some(e), None),
            Err(e) => ("error", Some(e), None),
        };
        let err = err.as_deref();
        if let Err(e) = state
            .db
            .finish_routine_run(&run_id, status, err, message_id.as_deref())
            .await
        {
            tracing::warn!(error = %e, "could not record routine run result");
        }
        state.running_routines.lock().await.remove(&routine.id);
        let _ = state.event_sender.send(RuntimeEvent::RoutineRun {
            routine_id: routine.id.clone(),
            run_id: run_id.clone(),
            status: status.into(),
            waiting: false,
            approval_id: None,
        });
        // A run that needs the person already has its card in the Inbox: no second notice. One
        // failure is one notice: none while AutoFix will run it again, and the last attempt's says
        // how often it was retried (routine-retry-once)
        let retries = if matches!(status, "error" | "incomplete") {
            super::missions::autofix::routine_run_failed(&state, &routine.id, &run_id, status).await
        } else {
            None
        };
        if let Some(retries) = retries {
            let kwant_name = state
                .db
                .get_kwant(&routine.kwant_id)
                .await
                .ok()
                .flatten()
                .map(|k| k.name)
                .unwrap_or_default();
            notify(
                &state,
                Notification {
                    kind: "routine_failed",
                    kwant_id: routine.kwant_id.clone(),
                    kwant_name,
                    channel_id: routine.channel_id.clone(),
                    title: format!("Routine \"{}\" failed", routine.name),
                    body: failed_body(&routine.name, err, retries),
                    thread_id: None,
                    card_id: None,
                },
            )
            .await;
        }
    });
    Some(run)
}

/// The notification for a routine run that failed (0.9.5): what stopped, why (the error's first
/// line, clipped) and what to do about it; then, when AutoFix ran it again first, how often.
pub fn failed_body(name: &str, err: Option<&str>, retries: i64) -> String {
    let why = err.map(super::stopped::short_line).filter(|w| !w.is_empty());
    let why: Option<String> = why.map(|w| {
        let w = w.trim_end_matches('.');
        if w.chars().count() > 160 { format!("{}…", w.chars().take(160).collect::<String>().trim_end()) } else { w.to_string() }
    });
    let body = match why {
        Some(why) => format!("The routine \"{name}\" did not finish: {why}. Open the routine to see the run or run it again."),
        None => format!("The routine \"{name}\" did not finish. Open the routine to see the run or run it again."),
    };
    match retries {
        n if n <= 0 => body,
        1 => format!("{body} It was retried once."),
        2 => format!("{body} It was retried twice."),
        n => format!("{body} It was retried {n} times."),
    }
}

fn send_run_event(state: &AppState, run: &RoutineRun) {
    let _ = state.event_sender.send(RuntimeEvent::RoutineRun {
        routine_id: run.routine_id.clone(),
        run_id: run.id.clone(),
        status: run.status.clone(),
        waiting: false,
        approval_id: None,
    });
}

/// A routine run's error when a step waits on an approval card nobody answered (status
/// `needs_you`, 26b).
pub const NEEDS_YOU_ERROR: &str = "A step is waiting for your approval, so it didn't run";

/// unanswered-question-not-ok: a routine run's error when it waits on a question (`ask_user`)
/// nobody answered (status `needs_you`). The open question is its one Inbox item
/// (`db::away::FAILED_RUNS` leaves the run out), and History reads "Needed your answer, didn’t finish".
pub const QUESTION_ERROR: &str = "A step is waiting for your answer, so it didn't finish";

/// What a routine's turn did, once it replied.
struct Ran {
    reply_id: Option<String>,
    /// The run's status and error (see [`run_outcome`]).
    outcome: (&'static str, Option<String>),
}

/// One tool call or step of a run's turn, as [`run_outcome`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct StepSeen {
    pub tool: String,
    /// "ok", "failed", "unanswered", or anything else (ignored).
    pub status: &'static str,
    pub reason: Option<String>,
    /// A "failed" step the person said no to on its card (28b): its reason is the `said_no` sentence, and
    /// a real failure's reason wins over it.
    pub said_no: bool,
    /// An `unanswered` question of the turn (unanswered-question-not-ok), not an approval card.
    pub question: bool,
}

/// The run's status from what its turn's steps did, in order (26b; P1: never "ok" when nothing
/// ran):
/// - any call `unanswered` (its card is still open) → `needs_you`, [`NEEDS_YOU_ERROR`]; when
///   only questions went unanswered, [`QUESTION_ERROR`] (an open card wins: it names a call
///   that didn't run);
/// - a call that failed (error, denied, cancelled) with no later "ok" call of the same tool →
///   `incomplete`, with the first such call's reason on one line. A real failure's reason wins
///   over a "You said no" one (28b round 2), so a no never hides what broke after it;
/// - otherwise `ok`.
pub fn run_outcome(steps: &[StepSeen]) -> (&'static str, Option<String>) {
    let mut unanswered = steps.iter().filter(|s| s.status == "unanswered").peekable();
    if unanswered.peek().is_some() {
        let error = if unanswered.all(|s| s.question) { QUESTION_ERROR } else { NEEDS_YOU_ERROR };
        return ("needs_you", Some(error.to_string()));
    }
    let unfixed: Vec<&StepSeen> = steps
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            s.status == "failed"
                && !steps[i + 1..]
                    .iter()
                    .any(|later| later.status == "ok" && later.tool == s.tool)
        })
        .map(|(_, s)| s)
        .collect();
    let first = unfixed
        .iter()
        .find(|s| !s.said_no)
        .or_else(|| unfixed.first());
    match first {
        Some(s) => {
            let why = s
                .reason
                .as_deref()
                .map(super::stopped::short_line)
                .filter(|w| !w.trim().is_empty());
            (
                "incomplete",
                Some(why.unwrap_or_else(|| format!("{} failed", s.tool))),
            )
        }
        None => ("ok", None),
    }
}

/// The steps a reply's saved tool calls (`tool_calls`) and activity (`meta.activity`) show. The
/// two are read separately, each in its own order: kwnt's tool calls record every engine's
/// approvals, and the activity has the claude CLI's own tool results.
pub fn steps_seen(
    tool_calls: Option<&serde_json::Value>,
    meta: Option<&serde_json::Value>,
) -> Vec<StepSeen> {
    let text = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let activity: &[serde_json::Value] = meta
        .and_then(|m| m["activity"]["steps"].as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    // A `denied` call is the person's no only when its card was declined (28b round 2): each
    // declined card of the same tool counts for one call. The pre-push gate, kwnt's secret files,
    // a ticket that couldn't be stored and a wait that ran out record `denied` with no card the
    // person said no on.
    let mut cards_left = cards(activity, "declined");
    // 28k: a card that stopped waiting with no answer (the wait ran out and couldn't be kept
    // open, or the card was withdrawn) explains one `denied` call of its tool: nobody answered
    let mut stopped_left = cards(activity, "stopped");
    let mut seen = Vec::new();
    for c in tool_calls.and_then(|v| v.as_array()).into_iter().flatten() {
        let status = match c["status"].as_str().unwrap_or("") {
            "ok" => "ok",
            "unanswered" => "unanswered",
            s if crate::mcp::missed_status(s) => "failed",
            _ => continue,
        };
        let tool = text(&c["name"]).unwrap_or_default();
        let reason = text(&c["error"]).or_else(|| text(&c["result"]));
        // A refusal's reason is what kwnt told the bot, not news; say what it was instead
        let card = if c["status"] == "denied" {
            cards_left.iter().position(|t| *t == tool)
        } else {
            None
        };
        let said = card.is_some();
        if let Some(k) = card {
            cards_left.remove(k);
        }
        let unanswered = c["status"] == "denied" && !said && take(&mut stopped_left, &tool);
        let reason = match c["status"].as_str() {
            Some("denied") if said => Some(said_no(&tool)),
            Some("denied") if unanswered => Some(nobody_answered(&tool)),
            Some("denied") => Some(format!("{tool} was denied")),
            _ => reason.map(|r| {
                r.replace(crate::mcp::NO_FALSE_CLAIMS_TEXT, "")
                    .trim()
                    .to_string()
            }),
        };
        seen.push(StepSeen {
            tool,
            status,
            reason,
            said_no: said,
            question: false,
        });
    }
    for step in activity {
        let status = match (step["kind"].as_str(), step["status"].as_str()) {
            (_, Some("unanswered")) => "unanswered",
            (Some("tool"), Some("ok")) => "ok",
            (Some("tool"), Some("failed")) => "failed",
            // The person said no on its card (28b): it didn't run, so the run isn't ok
            (Some("tool"), Some("declined")) => "failed",
            _ => continue,
        };
        let tool = text(&step["tool"]).unwrap_or_default();
        // `activity.rs` ends a tool step `declined` only when it paired it with its declined card
        let said = step["status"] == "declined";
        let reason = if said { Some(said_no(&tool)) } else { text(&step["detail"]) };
        seen.push(StepSeen {
            tool,
            status,
            reason,
            said_no: said,
            // unanswered-question-not-ok: `activity.rs` `question_unanswered`
            question: step["kind"] == "question",
        });
    }
    seen
}

/// A routine run's reason when the person said no to `tool` (28b).
fn said_no(tool: &str) -> String {
    format!("You said no to {tool}; nothing ran")
}

/// A routine run's reason when `tool` waited on a card nobody answered and didn't run (28k). It
/// stays a failure, but not the person's no.
fn nobody_answered(tool: &str) -> String {
    format!("Nobody answered in time; {tool} didn't run")
}

/// The tools of `activity`'s approval steps that settled `status`, in order.
fn cards<'a>(activity: &'a [serde_json::Value], status: &str) -> Vec<&'a str> {
    activity
        .iter()
        .filter(|s| s["kind"] == "approval" && s["status"] == status)
        .filter_map(|s| s["tool"].as_str())
        .collect()
}

/// Uses up one entry for `tool` in `left`: whether there was one.
fn take(left: &mut Vec<&str>, tool: &str) -> bool {
    match left.iter().position(|t| *t == tool) {
        Some(k) => {
            left.remove(k);
            true
        }
        None => false,
    }
}

/// The meta key of a routine run's id on each reply of its turn (see `run_routine_turn`).
pub(super) const ROUTINE_RUN_KEY: &str = "routine_run";

/// What a run's own replies show: their steps, and the id of the last one.
struct OwnReplies {
    steps: Vec<StepSeen>,
    last_id: Option<String>,
}

/// The bot's replies after the routine's message (`prompt_id`) that belong to run `run_id` (its
/// turn, busy retries and automatic continuation; [`ROUTINE_RUN_KEY`] in their meta), and every
/// step they show. 26b D1: another routine's run, or the person's own turn, in the same chat at
/// the same time never counts.
async fn own_replies(
    state: &AppState,
    chat: &str,
    bot_name: &str,
    prompt_id: &str,
    run_id: &str,
) -> OwnReplies {
    let messages = state.db.list_messages(chat, 40).await.unwrap_or_default();
    let mut own = OwnReplies { steps: Vec::new(), last_id: None };
    let Some(start) = messages.iter().position(|m| m.id == prompt_id) else {
        return own;
    };
    let parse = |s: &Option<String>| {
        s.as_deref()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
    };
    for m in messages[start + 1..]
        .iter()
        .filter(|m| m.sender_role == "kwant" && m.sender_id == bot_name)
    {
        let meta = parse(&m.meta);
        if meta.as_ref().map(|v| &v[ROUTINE_RUN_KEY]).and_then(|v| v.as_str()) != Some(run_id) {
            continue;
        }
        own.steps.extend(steps_seen(parse(&m.tool_calls).as_ref(), meta.as_ref()));
        own.last_id = Some(m.id.clone());
    }
    own
}

/// 26b: routine runs whose turn is going, by bot and chat (one reply per bot per chat at a time),
/// so an approval card the turn opens can mark its run as waiting.
static RUNS_IN_CHATS: std::sync::LazyLock<std::sync::Mutex<RunsInChats>> = std::sync::LazyLock::new(Default::default);

/// (bot id, chat key) → (routine id, run id).
type RunsInChats = std::collections::HashMap<(String, String), (String, String)>;

/// Holds a run's place in [`RUNS_IN_CHATS`] while its turn runs.
struct RunInChat((String, String));

impl RunInChat {
    fn new(kwant_id: &str, chat: &str, routine_id: &str, run_id: &str) -> Self {
        let key = (kwant_id.to_string(), chat.to_string());
        if let Ok(mut runs) = RUNS_IN_CHATS.lock() {
            runs.insert(key.clone(), (routine_id.to_string(), run_id.to_string()));
        }
        RunInChat(key)
    }
}

impl Drop for RunInChat {
    fn drop(&mut self) {
        if let Ok(mut runs) = RUNS_IN_CHATS.lock() {
            runs.remove(&self.0);
        }
    }
}

/// An approval card of `kwant_id`'s turn in `chat` opened (`Some(id)`) or closed (`None`): when
/// that turn is a routine run's, its `running` event says so (`waiting`, `approval_id`). `chat` is
/// the thread, channel or direct-chat bot id, as the run's message was posted.
pub fn approval_wait(
    events: &tokio::sync::broadcast::Sender<RuntimeEvent>,
    kwant_id: &str,
    chat: &str,
    approval_id: Option<&str>,
) {
    let run = RUNS_IN_CHATS
        .lock()
        .ok()
        .and_then(|runs| runs.get(&(kwant_id.to_string(), chat.to_string())).cloned());
    if let Some((routine_id, run_id)) = run {
        let _ = events.send(RuntimeEvent::RoutineRun {
            routine_id,
            run_id,
            status: "running".into(),
            waiting: approval_id.is_some(),
            approval_id: approval_id.map(str::to_string),
        });
    }
}

/// Posts the routine's instructions as a message from "Routine: <name>" and runs the kwant's turn.
async fn execute(state: &AppState, routine: &RoutineRow, run_id: &str) -> Result<Ran, String> {
    let kwant = state
        .db
        .get_kwant(&routine.kwant_id)
        .await
        .map_err(|_| "could not load the bot".to_string())?
        .ok_or_else(|| "the routine's bot no longer exists".to_string())?;
    // The API never makes a routine across spaces, but a row edited by hand must not run one
    // space's bot in another space's chat
    if kwant.space_id != routine.space_id {
        return Err("the routine's bot is in another space, so it was not run".into());
    }
    if state.db.space_archived(&crate::db::spaces::SpaceId::new(&routine.space_id)).await.unwrap_or(true) {
        return Err("the routine's space is archived, so it was not run".into());
    }
    if let Some(c) = &routine.channel_id {
        let channel = state
            .db
            .get_channel(c)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| "the routine's channel no longer exists".to_string())?;
        if channel.space_id != routine.space_id {
            return Err("the routine's channel is in another space, so it was not run".into());
        }
    }
    if let Some(t) = &routine.thread_id {
        let thread = state.db.get_thread(t).await.ok().flatten().ok_or_else(|| "the routine's thread no longer exists".to_string())?;
        if thread.kwant_id != kwant.id {
            return Err("the routine's thread belongs to another bot, so it was not run".into());
        }
    }
    let in_thread = routine.thread_id.is_some();
    let message = crate::db::models::NewMessage {
        channel_id: routine.channel_id.clone(),
        direct_kwant_id: if routine.channel_id.is_none() && !in_thread {
            Some(kwant.id.clone())
        } else {
            None
        },
        sender_role: "user".into(),
        sender_id: format!("Routine: {}", routine.name),
        text: routine.instructions.clone(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: routine.instructions.len() as i64 / 4,
    };
    let extra = crate::db::models::MessageExtra { thread_id: routine.thread_id.clone(), meta: None };
    let saved = state
        .db
        .insert_message_with(&message, &extra)
        .await
        .map_err(|_| "could not post the routine's message".to_string())?;
    if let Some(t) = &routine.thread_id {
        let _ = state.db.touch_thread(t).await;
    }
    let prompt = routine.instructions.clone()
        + &follow_up_context(state, routine, &kwant, &saved.id).await
        + &last_run_context(state, routine, &kwant).await;
    let chat = delivery_chat(
        routine.thread_id.as_deref(),
        routine.channel_id.as_deref(),
        &kwant.id,
    )
    .to_string();
    let in_chat = RunInChat::new(&kwant.id, &chat, &routine.id, run_id);
    let reply_id = super::routes::run_routine_turn(
        state,
        &kwant,
        routine.channel_id.clone(),
        routine.thread_id.clone(),
        prompt,
        &routine.name,
        run_id,
    )
    .await?;
    drop(in_chat);
    let own = own_replies(state, &chat, &kwant.name, &saved.id, run_id).await;
    // The run's own last reply; the turn's answer is the chat's latest reply by the bot, which
    // can be another run's when two share the chat
    Ok(Ran { reply_id: own.last_id.or(reply_id), outcome: run_outcome(&own.steps) })
}

/// The chat id a routine delivers to: its thread, its channel, or its bot's direct chat.
fn delivery_chat<'a>(thread_id: Option<&'a str>, channel_id: Option<&'a str>, kwant_id: &'a str) -> &'a str {
    thread_id.or(channel_id).unwrap_or(kwant_id)
}

/// Messages a follow-up shows of the exchange it follows up on, and how much of each.
const FOLLOW_UP_MESSAGES: i64 = 6;
const FOLLOW_UP_CHARS_EACH: usize = 600;
/// What a turn already sees of its chat (see `run_kwant_turn`'s history).
const TURN_HISTORY_MESSAGES: i64 = 20;

/// Most of a routine's last output its next run quotes.
const LAST_RUN_CHARS: usize = 1500;

/// For a routine that ran before, its last successful output, so a run builds on it instead of
/// repeating it. When that reply is still in the history the turn sees, only points at it.
pub async fn last_run_context(state: &AppState, routine: &RoutineRow, kwant: &crate::db::models::Kwant) -> String {
    let runs = state.db.list_routine_runs(&routine.id, 10).await.unwrap_or_default();
    let Some((run, id)) = runs.iter().find_map(|r| Some((r, r.message_id.as_deref().filter(|_| r.status == "ok")?))) else {
        return String::new();
    };
    let Ok(Some(reply)) = state.db.get_message(id).await else {
        return String::new();
    };
    let when = run.started_at.format("%Y-%m-%d %H:%M UTC");
    let chat = delivery_chat(routine.thread_id.as_deref(), routine.channel_id.as_deref(), &kwant.id);
    let recent = state.db.list_messages(chat, TURN_HISTORY_MESSAGES).await.unwrap_or_default();
    if recent.iter().any(|m| m.id == reply.id) {
        return format!(
            "\n\n[This routine last ran {when}; its reply is above in this chat. Build on it and don't repeat it word for word.]"
        );
    }
    format!(
        "\n\n[This routine last ran {when}. Its reply then, for continuity (build on it, don't repeat it word for word):\n{}]",
        super::routes::truncate_chars(reply.text.trim(), LAST_RUN_CHARS)
    )
}

/// For a follow-up routine, the exchange it follows up on (from the same chat only), when it has
/// scrolled out of the history the turn sees anyway. Empty otherwise.
pub async fn follow_up_context(state: &AppState, routine: &RoutineRow, kwant: &crate::db::models::Kwant, run_message_id: &str) -> String {
    let Some(anchor) = &routine.context_message_id else {
        return String::new();
    };
    let chat = delivery_chat(routine.thread_id.as_deref(), routine.channel_id.as_deref(), &kwant.id);
    let recent = state.db.list_messages(chat, TURN_HISTORY_MESSAGES).await.unwrap_or_default();
    if recent.iter().any(|m| &m.id == anchor) {
        return String::new();
    }
    let Ok(Some(around)) = state.db.list_messages_around(chat, anchor, FOLLOW_UP_MESSAGES).await else {
        return String::new();
    };
    let lines: Vec<String> = around
        .iter()
        .filter(|m| m.id != run_message_id && m.sender_role != "system")
        .map(|m| {
            let when = m.created_at.map(|t| t.format("%Y-%m-%d %H:%M UTC ").to_string()).unwrap_or_default();
            // "Person" only for the person's own words: a customer's or a connected agent's keep
            // their name and their frame (truncated first, so the frame's end marker survives)
            let who = super::delivery_plan::sender_label(m, "Person");
            let text = super::routes::truncate_chars(m.text.trim(), FOLLOW_UP_CHARS_EACH).to_string();
            let text = crate::drivers::user_history_text(&crate::db::models::Message { text, ..m.clone() });
            format!("- {when}{who}: {text}")
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "\n\n[This is a follow-up you scheduled. The earlier exchange it follows up on, from this chat:\n{}\n\
        Follow up on it now: ask or check what you promised, and keep it short.]",
        lines.join("\n")
    )
}

/// Moves every missed or unset next run to its first slot after `now`: runs missed while kwnt was
/// down are dropped, and a `once` routine whose time has passed is switched off.
pub async fn reschedule_all(state: &AppState, now: DateTime<Utc>) {
    let mut routines = Vec::new();
    for space in state.db.list_spaces().await.unwrap_or_default() {
        let space = crate::db::spaces::SpaceId::new(&space.id);
        routines.extend(state.db.list_routines(&space).await.unwrap_or_default());
    }
    // A slot still ahead is kept: recomputing it would push interval routines back on every restart
    for r in routines
        .into_iter()
        .filter(|r| r.enabled && r.next_run_at.is_none_or(|t| t <= now))
    {
        let next = serde_json::from_str::<Schedule>(&r.schedule)
            .ok()
            .and_then(|s| s.next_after(now, r.last_run_at));
        if next != r.next_run_at {
            let _ = state
                .db
                .set_routine_schedule_state(&r.id, next, next.is_some(), None)
                .await;
        }
    }
}

/// Starts every routine due at `now` and moves each one's next run on. Returns how many started.
pub async fn tick(state: &AppState, now: DateTime<Utc>) -> usize {
    if let Ok(n) = state.db.delete_orphan_routines().await {
        if n > 0 {
            tracing::info!(count = n, "removed routines whose agent or channel is gone");
        }
    }
    let due = state.db.due_routines(now).await.unwrap_or_default();
    let mut started = 0;
    for r in due {
        let schedule = serde_json::from_str::<Schedule>(&r.schedule).ok();
        // From `now`, not from the slot: a late tick never queues up catch-up runs
        let next = schedule.as_ref().and_then(|s| s.next_after(now, Some(now)));
        let keep = next.is_some() && !matches!(schedule, Some(Schedule::Once { .. }) | None);
        if let Err(e) = state
            .db
            .set_routine_schedule_state(&r.id, if keep { next } else { None }, keep, Some(now))
            .await
        {
            tracing::warn!(error = %e, "could not move routine on; not running it");
            continue;
        }
        if schedule.is_none() {
            let _ = state
                .db
                .start_routine_run(&r.id, "schedule", "error", Some("the schedule is invalid"))
                .await;
            continue;
        }
        if start_run(state, r, "schedule").await.is_some() {
            started += 1;
        }
    }
    started
}

/// Runs the scheduler until `shutdown`.
pub fn spawn(state: AppState, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Ok(n) = state.db.fail_interrupted_runs().await {
            if n > 0 {
                tracing::info!(
                    count = n,
                    "marked routine runs cut short by a restart as failed"
                );
            }
        }
        let _ = state.db.delete_orphan_routines().await;
        super::routes::load_turn_limits(&state).await;
        super::delegation::interrupt_open(&state).await;
        reschedule_all(&state, Utc::now()).await;
        loop {
            tick(&state, Utc::now()).await;
            super::delegation::follow_ups(&state, Utc::now()).await;
            // G2: bots whose budget pause ended on its own (a new month) start what they held
            super::bot_budget::release_due(&state).await;
            super::parked::run_due(&state, Utc::now()).await;
            // 0.9.5: an approval card nobody answered in time tells its chat, with Retry
            super::mcp_runtime::expire_unanswered(&state).await;
            super::waiting::run_due(&state, Utc::now()).await;
            super::work_ahead::scan(&state, Utc::now(), false).await;
            super::idea_jar::scan(&state, Utc::now(), false).await;
            super::missions::runner::tick(&state).await;
            super::goals_remediation::tick(&state).await;
            let next = [
                state.db.next_routine_due().await.ok().flatten(),
                state.db.next_parked_due().await.ok().flatten(),
                state.db.next_waiting_due().await.ok().flatten(),
            ]
            .into_iter()
            .flatten()
            .min();
            let sleep = match next {
                Some(at) => (at - Utc::now())
                    .to_std()
                    .unwrap_or(Duration::ZERO)
                    .min(MAX_SLEEP),
                None => MAX_SLEEP,
            };
            // A due-now routine that couldn't be moved on must not spin the loop
            let sleep = sleep.max(Duration::from_millis(500));
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = state.scheduler_wake.notified() => {}
                _ = tokio::time::sleep(sleep) => {}
            }
        }
    })
}

/// Starts the scheduler and the notifier.
pub fn spawn_background(state: AppState, shutdown: CancellationToken) {
    super::continuity::spawn_notifier(state.clone());
    super::bot_status::spawn_watcher(state.clone());
    crate::zombie::hunt::spawn(state.clone(), shutdown.clone());
    crate::documents::reminders::spawn(state.clone(), shutdown.clone());
    super::missions::listener::spawn(state.clone(), shutdown.clone());
    super::memory_recall::spawn(state.clone(), shutdown.clone());
    super::missions::foundry::spawn(state.clone(), shutdown.clone());
    super::goals::runner::spawn(state.clone(), shutdown.clone());
    super::missions::discovery::spawn(state.clone(), shutdown.clone());
    super::dev::poller::spawn(state.clone(), shutdown.clone());
    super::dev::worktrees::spawn(state.clone(), shutdown.clone());
    super::dev::tickets::pickup::spawn(state.clone(), shutdown.clone());
    super::telegram::spawn(state.clone(), shutdown.clone());
    super::telegram::customers::spawn(state.clone(), shutdown.clone());
    state.mcp_directory.clone().spawn(state.mcp_registry.clone(), shutdown.clone());
    spawn(state, shutdown);
}

#[cfg(test)]
mod no_silent_stop_tests {
    use super::failed_body;

    #[test]
    fn a_failed_routine_says_why_and_what_to_do() {
        assert_eq!(
            failed_body("Daily digest", Some("⚠️ [Ben] The model sent nothing for 300 s, so this reply was stopped.\nmore"), 0),
            "The routine \"Daily digest\" did not finish: The model sent nothing for 300 s, so this reply was stopped. Open the routine to see the run or run it again."
        );
        assert_eq!(failed_body("D", None, 0), "The routine \"D\" did not finish. Open the routine to see the run or run it again.");
        assert_eq!(failed_body("D", Some("  \n "), 0), failed_body("D", None, 0));
        let long = failed_body("D", Some(&"x".repeat(500)), 0);
        assert!(long.contains(&format!("{}…", "x".repeat(160))) && !long.contains(&"x".repeat(161)), "{long}");
    }

    /// routine-retry-once: the last attempt's notice says how often AutoFix ran it again.
    #[test]
    fn a_retried_routine_says_how_often() {
        let base = "The routine \"D\" did not finish: it died. Open the routine to see the run or run it again.";
        assert_eq!(failed_body("D", Some("it died"), 0), base);
        assert_eq!(failed_body("D", Some("it died"), 1), format!("{base} It was retried once."));
        assert_eq!(failed_body("D", Some("it died"), 2), format!("{base} It was retried twice."));
        assert_eq!(failed_body("D", Some("it died"), 4), format!("{base} It was retried 4 times."));
    }
}

#[cfg(test)]
mod run_outcome_tests {
    use super::*;

    fn step(tool: &str, status: &'static str, reason: Option<&str>) -> StepSeen {
        StepSeen {
            tool: tool.into(),
            status,
            reason: reason.map(str::to_string),
            said_no: false,
            question: false,
        }
    }

    /// An unanswered question of the turn (unanswered-question-not-ok).
    fn asked() -> StepSeen {
        StepSeen {
            question: true,
            ..step("ask_user", "unanswered", Some(crate::mcp::QUESTION_STEP_DETAIL))
        }
    }

    /// unanswered-question-not-ok: a question nobody answered isn't "ok", and says it waits on an
    /// answer; an open approval card keeps 26b's words, also next to a question.
    #[test]
    fn an_unanswered_question_needs_you_in_its_own_words() {
        assert_eq!(run_outcome(&[asked()]), ("needs_you", Some(QUESTION_ERROR.into())));
        assert_eq!(run_outcome(&[step("web", "ok", None), asked()]), ("needs_you", Some(QUESTION_ERROR.into())));
        assert_eq!(run_outcome(&[asked(), step("ha", "unanswered", None)]), ("needs_you", Some(NEEDS_YOU_ERROR.into())));
        assert_eq!(run_outcome(&[step("ha", "unanswered", None), asked()]), ("needs_you", Some(NEEDS_YOU_ERROR.into())));
        // An answered question leaves no step: the run is what its calls did
        assert_eq!(run_outcome(&[step("ask_user", "ok", None)]), ("ok", None));
        assert_ne!(QUESTION_ERROR, NEEDS_YOU_ERROR);
    }

    /// The question step `activity.rs` saves is read as a question; an approval step is not.
    #[test]
    fn a_saved_question_step_reads_as_a_question() {
        let meta = json!({"activity": {"steps": [
            {"kind": "tool", "tool": "mcp__kwnt__ask_user", "status": "ok"},
            {"kind": "question", "tool": "ask_user", "status": "unanswered", "detail": crate::mcp::QUESTION_STEP_DETAIL},
        ]}});
        let seen = steps_seen(None, Some(&meta));
        assert_eq!(seen, vec![step("mcp__kwnt__ask_user", "ok", None), asked()]);
        assert_eq!(run_outcome(&seen), ("needs_you", Some(QUESTION_ERROR.into())));
        let card = json!({"activity": {"steps": [{"kind": "approval", "tool": "ask_user", "status": "unanswered"}]}});
        assert_eq!(run_outcome(&steps_seen(None, Some(&card))), ("needs_you", Some(NEEDS_YOU_ERROR.into())));
    }

    /// A step the person said no to on its card (28b).
    fn no(tool: &str) -> StepSeen {
        StepSeen {
            said_no: true,
            ..step(tool, "failed", Some(said_no(tool).as_str()))
        }
    }

    /// 26b: a run whose only call nobody approved needs the person, whatever the reply says.
    #[test]
    fn an_unanswered_call_needs_you() {
        assert_eq!(
            run_outcome(&[step("ha", "unanswered", None)]),
            ("needs_you", Some(NEEDS_YOU_ERROR.into()))
        );
        // even after other calls worked
        assert_eq!(
            run_outcome(&[step("a", "ok", None), step("ha", "unanswered", None)]).0,
            "needs_you"
        );
    }

    #[test]
    fn a_failure_fixed_by_a_later_call_of_the_same_tool_is_ok() {
        assert_eq!(
            run_outcome(&[
                step("web", "failed", Some("timeout")),
                step("web", "ok", None)
            ]),
            ("ok", None)
        );
        assert_eq!(run_outcome(&[]), ("ok", None));
    }

    #[test]
    fn a_failure_with_no_later_success_is_incomplete_with_its_reason() {
        let steps = [
            step("web", "ok", None),
            step("web", "failed", Some("⚠️ 502 from the site\nstack…")),
            step("mail", "ok", None),
        ];
        assert_eq!(
            run_outcome(&steps),
            ("incomplete", Some("502 from the site".into()))
        );
        // another tool's success doesn't fix it; no reason names the tool
        assert_eq!(
            run_outcome(&[step("web", "failed", None), step("mail", "ok", None)]),
            ("incomplete", Some("web failed".into()))
        );
    }

    /// The saved tool calls (every engine's approvals, kwnt's own tools) and the activity (the
    /// claude CLI's own tool results) are both read; the reminder to the model is left out.
    #[test]
    fn steps_come_from_tool_calls_and_activity() {
        let calls = json!([
            {"name": "ha__turn_on", "status": "unanswered"},
            {"name": "web__get", "status": "error", "result": crate::mcp::with_no_false_claims("502")},
            {"name": "mail__send", "status": "denied", "result": crate::mcp::DENIED_TEXT},
            {"name": "x", "status": "allowed"},
            {"name": "y", "status": "rehearsed"},
        ]);
        // As the reply saves it (`ActivityLog::to_json`): the steps under `steps`
        let meta = json!({"activity": {"steps": [
            {"kind": "thinking", "status": "ok"},
            {"kind": "approval", "status": "unanswered", "tool": "Bash"},
            {"kind": "tool", "status": "failed", "tool": "Bash", "detail": "exit 1"},
            {"kind": "tool", "status": "ok", "tool": "Bash"},
        ]}});
        let seen = steps_seen(Some(&calls), Some(&meta));
        assert_eq!(
            seen,
            vec![
                step("ha__turn_on", "unanswered", None),
                step("web__get", "failed", Some("502")),
                // no declined card of its own: main's words, not the person's no (28b round 2)
                step("mail__send", "failed", Some("mail__send was denied")),
                step("Bash", "unanswered", None),
                step("Bash", "failed", Some("exit 1")),
                step("Bash", "ok", None),
            ]
        );
        assert_eq!(
            run_outcome(&steps_seen(Some(&json!([calls[1].clone()])), None)),
            ("incomplete", Some("502".into()))
        );
        assert_eq!(run_outcome(&steps_seen(None, None)), ("ok", None));
    }

    /// The waiting flag goes only to the routine run whose bot and chat the card belongs to, and
    /// stops when the run's turn is over.
    #[test]
    fn a_card_marks_only_its_own_run_as_waiting() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        let held = RunInChat::new("bot_w1", "chat_w1", "routine_w1", "run_w1");
        approval_wait(&tx, "bot_w1", "chat_w1", Some("apr_1"));
        approval_wait(&tx, "bot_other", "chat_w1", Some("apr_2"));
        approval_wait(&tx, "bot_w1", "chat_w1", None);
        drop(held);
        approval_wait(&tx, "bot_w1", "chat_w1", Some("apr_3"));
        let got: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            got,
            vec![
                RuntimeEvent::RoutineRun {
                    routine_id: "routine_w1".into(),
                    run_id: "run_w1".into(),
                    status: "running".into(),
                    waiting: true,
                    approval_id: Some("apr_1".into()),
                },
                RuntimeEvent::RoutineRun {
                    routine_id: "routine_w1".into(),
                    run_id: "run_w1".into(),
                    status: "running".into(),
                    waiting: false,
                    approval_id: None,
                },
            ]
        );
        // Old clients: the new fields are optional both ways
        let old: RuntimeEvent = serde_json::from_value(json!({"type": "routine_run", "data": {"routine_id": "r", "run_id": "x", "status": "ok"}})).unwrap();
        assert!(matches!(
            old,
            RuntimeEvent::RoutineRun {
                waiting: false,
                approval_id: None,
                ..
            }
        ));
    }

    /// 28b: a routine whose only step the person said no to on its card isn't ok: it stays
    /// `incomplete`, and its reason says so in the person's words. A Deny on a kwnt tool reads
    /// the same.
    #[test]
    fn a_no_ends_the_run_incomplete_in_the_persons_words() {
        let meta = json!({"activity": {"steps": [
            {"kind": "tool", "status": "declined", "tool": "Bash", "detail": crate::mcp::DENIED_STEP_DETAIL},
            {"kind": "approval", "status": "declined", "tool": "Bash"},
        ]}});
        let seen = steps_seen(None, Some(&meta));
        assert_eq!(seen, vec![no("Bash")]);
        assert_eq!(seen[0].reason.as_deref(), Some("You said no to Bash; nothing ran"));
        assert_eq!(
            run_outcome(&seen),
            ("incomplete", Some("You said no to Bash; nothing ran".into()))
        );
        let calls = json!([{"name": "mail__send", "status": "denied", "result": crate::mcp::DENIED_TEXT}]);
        let card = json!({"activity": {"steps": [{"kind": "approval", "status": "declined", "tool": "mail__send"}]}});
        assert_eq!(
            run_outcome(&steps_seen(Some(&calls), Some(&card))),
            ("incomplete", Some("You said no to mail__send; nothing ran".into()))
        );
    }

    /// 28b round 2: kwnt records `denied` for refusals the person never saw (the pre-push gate,
    /// kwnt's secret files, a ticket it couldn't store, a wait that ran out). Only a declined card
    /// of the same tool makes it the person's no, and each card counts for one call.
    #[test]
    fn a_denied_call_is_the_persons_no_only_with_its_declined_card() {
        let denied = |name: &str| json!({"name": name, "status": "denied", "result": crate::mcp::DENIED_TEXT});
        let cards = |steps: serde_json::Value| json!({"activity": {"steps": steps}});
        // no card at all
        let calls = json!([denied("github__push")]);
        assert_eq!(
            steps_seen(Some(&calls), None),
            vec![step("github__push", "failed", Some("github__push was denied"))]
        );
        // a declined card of the same tool
        let same = cards(json!([{"kind": "approval", "status": "declined", "tool": "github__push"}]));
        assert_eq!(steps_seen(Some(&calls), Some(&same)), vec![no("github__push")]);
        // a declined card of another tool, or a card of the same tool that wasn't declined
        let other = cards(json!([
            {"kind": "approval", "status": "declined", "tool": "Bash"},
            {"kind": "approval", "status": "ok", "tool": "github__push"},
        ]));
        let seen = steps_seen(Some(&calls), Some(&other));
        assert_eq!(seen[0], step("github__push", "failed", Some("github__push was denied")));
        // a card of the same tool that stopped waiting: nobody answered (28k)
        let stopped = cards(json!([{"kind": "approval", "status": "stopped", "tool": "github__push"}]));
        let seen = steps_seen(Some(&calls), Some(&stopped));
        assert_eq!(seen[0], step("github__push", "failed", Some("Nobody answered in time; github__push didn't run")));
        // one card, two denied calls of its tool: one no, one plain denial
        let twice = json!([denied("github__push"), denied("github__push")]);
        assert_eq!(
            steps_seen(Some(&twice), Some(&same)),
            vec![no("github__push"), step("github__push", "failed", Some("github__push was denied"))]
        );
        // a CLI builtin pairs by its raw name too
        let bash = json!([denied("Bash")]);
        let bash_card = cards(json!([{"kind": "approval", "status": "declined", "tool": "Bash"}]));
        assert_eq!(steps_seen(Some(&bash), Some(&bash_card))[0], no("Bash"));
    }

    /// 28k: a routine wait that ran out with no answer reads "Nobody answered in time; X didn't
    /// run", a rose failure (`said_no` false), never the person's no; each stopped card explains
    /// one denied call. The person's real no keeps its words, and a refusal kwnt made on its own
    /// reads as its saved step says.
    #[test]
    fn a_wait_that_ran_out_says_nobody_answered() {
        let denied = |name: &str| json!({"name": name, "status": "denied"});
        let meta = |steps: serde_json::Value| json!({"activity": {"steps": steps}});
        let calls = json!([denied("mail__send"), denied("mail__send")]);
        let one_card = meta(json!([{"kind": "approval", "status": "stopped", "tool": "mail__send"}]));
        let seen = steps_seen(Some(&calls), Some(&one_card));
        assert_eq!(
            seen,
            vec![
                step("mail__send", "failed", Some("Nobody answered in time; mail__send didn't run")),
                step("mail__send", "failed", Some("mail__send was denied")),
            ]
        );
        assert!(!seen[0].said_no);
        assert_eq!(
            run_outcome(&seen[..1]),
            ("incomplete", Some("Nobody answered in time; mail__send didn't run".into()))
        );
        // A Deny on one card and a wait that ran out on another: each call gets its own words
        let both = meta(json!([
            {"kind": "approval", "status": "stopped", "tool": "mail__send"},
            {"kind": "approval", "status": "declined", "tool": "mail__send"},
        ]));
        assert_eq!(
            steps_seen(Some(&calls), Some(&both)),
            vec![no("mail__send"), step("mail__send", "failed", Some("Nobody answered in time; mail__send didn't run"))]
        );
        // The claude CLI's own step for it, as the activity log saves it (28k)
        let cli = meta(json!([
            {"kind": "tool", "status": "failed", "tool": "Bash", "detail": crate::mcp::NOBODY_ANSWERED_STEP_DETAIL},
            {"kind": "approval", "status": "stopped", "tool": "Bash"},
            {"kind": "tool", "status": "failed", "tool": "Bash", "detail": crate::mcp::PREPUSH_BLOCKED_STEP_DETAIL},
            {"kind": "tool", "status": "failed", "tool": "Read", "detail": crate::mcp::BLOCKED_STEP_DETAIL},
        ]));
        let seen = steps_seen(None, Some(&cli));
        assert_eq!(
            seen,
            vec![
                step("Bash", "failed", Some("Nobody answered in time; nothing ran")),
                step("Bash", "failed", Some("Blocked before push; nothing ran")),
                step("Read", "failed", Some("kwant blocked this; nothing ran")),
            ]
        );
        assert!(seen.iter().all(|s| !s.said_no));
    }

    /// 28b round 2: a real failure's reason wins over the person's no, whichever came first; the
    /// no is the reason only when nothing else failed. The run is `incomplete` either way.
    #[test]
    fn a_real_failure_wins_over_a_no_in_the_runs_reason() {
        let http = step("WebFetch", "failed", Some("HTTP 500"));
        assert_eq!(
            run_outcome(&[no("Bash"), http.clone()]),
            ("incomplete", Some("HTTP 500".into()))
        );
        assert_eq!(
            run_outcome(&[http.clone(), no("Bash")]),
            ("incomplete", Some("HTTP 500".into()))
        );
        assert_eq!(
            run_outcome(&[no("Bash")]),
            ("incomplete", Some("You said no to Bash; nothing ran".into()))
        );
        // a failure a later call fixed doesn't count; the no is what's left
        assert_eq!(
            run_outcome(&[no("Bash"), http, step("WebFetch", "ok", None)]),
            ("incomplete", Some("You said no to Bash; nothing ran".into()))
        );
        // two noes: the first one's tool
        assert_eq!(
            run_outcome(&[no("Bash"), no("Write")]),
            ("incomplete", Some("You said no to Bash; nothing ran".into()))
        );
    }
}
