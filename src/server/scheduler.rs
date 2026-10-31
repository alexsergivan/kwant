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

/// Records a run and starts it in the background, or records it as skipped when the routine is
/// still running. `None` if the run couldn't be recorded.
pub async fn start_run(state: &AppState, routine: RoutineRow, trigger: &str) -> Option<RoutineRun> {
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
    send_run_event(state, &run);
    let state = state.clone();
    let run_id = run.id.clone();
    tokio::spawn(async move {
        let result = execute(&state, &routine).await;
        let (status, err, message_id) = match &result {
            Ok(message_id) => ("ok", None, message_id.clone()),
            Err(e) => ("error", Some(e.as_str()), None),
        };
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
            run_id,
            status: status.into(),
        });
        if result.is_err() {
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
                    body: format!(
                        "The routine \"{}\" did not finish. Open kwant bots to see its run history.",
                        routine.name
                    ),
                },
            )
            .await;
        }
    });
    Some(run)
}

fn send_run_event(state: &AppState, run: &RoutineRun) {
    let _ = state.event_sender.send(RuntimeEvent::RoutineRun {
        routine_id: run.routine_id.clone(),
        run_id: run.id.clone(),
        status: run.status.clone(),
    });
}

/// Posts the routine's instructions as a message from "Routine: <name>" and runs the kwant's turn.
async fn execute(state: &AppState, routine: &RoutineRow) -> Result<Option<String>, String> {
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
    super::routes::run_routine_turn(state, &kwant, routine.channel_id.clone(), routine.thread_id.clone(), prompt).await
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
            let who = if m.sender_role == "user" { "Person" } else { m.sender_id.as_str() };
            format!("- {when}{who}: {}", super::routes::truncate_chars(m.text.trim(), FOLLOW_UP_CHARS_EACH))
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
            super::parked::run_due(&state, Utc::now()).await;
            super::waiting::run_due(&state, Utc::now()).await;
            super::work_ahead::scan(&state, Utc::now(), false).await;
            super::missions::runner::tick(&state).await;
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
    crate::zombie::hunt::spawn(state.clone(), shutdown.clone());
    crate::documents::reminders::spawn(state.clone(), shutdown.clone());
    super::missions::listener::spawn(state.clone(), shutdown.clone());
    super::missions::foundry::spawn(state.clone(), shutdown.clone());
    super::missions::discovery::spawn(state.clone(), shutdown.clone());
    spawn(state, shutdown);
}
