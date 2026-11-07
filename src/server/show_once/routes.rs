//! `/api/show-once/...` (api.md §1). Person-only, every method (see `auth::person_only`).

// Handler helpers return the ready-made error `Response`
#![allow(clippy::result_large_err)]

use super::stage::{self, Cmd};
use super::{capture, draft, emit, emit_id, engine_for, now, recording_json, step_json, trial};
use crate::db::show_once::{Recording, ShowOnceError, StepRow};
use crate::drivers::events::RuntimeEvent;
use crate::server::AppState;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get as get_route, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Longest `start_url` taken.
const MAX_URL: usize = 2048;

pub(super) fn fail(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": code, "message": message.into()}))).into_response()
}

fn internal() -> Response {
    fail(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong on kwant's side. Try again.")
}

fn show_once_error(e: ShowOnceError) -> Response {
    use ShowOnceError::*;
    match e {
        NotFound => fail(StatusCode::NOT_FOUND, "no_recording", "That recording isn't there any more."),
        AlreadyOpen(rec) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_open",
                "message": "This bot already has a recording open. Finish or discard it first.",
                "recording": recording_json(&rec, now(), None),
            })),
        )
            .into_response(),
        StageBusy => fail(StatusCode::CONFLICT, "stage_busy", "Another bot is being shown something in this browser right now."),
        WrongState(s) => fail(StatusCode::CONFLICT, "wrong_state", format!("That can't be done while the recording is {s}.")),
        LimitReached => fail(StatusCode::CONFLICT, "limit_reached", "This recording has used its 200 steps or 30 minutes."),
        TooManyNotes => fail(StatusCode::CONFLICT, "too_many_notes", "A recording holds at most 50 notes."),
        NoSteps => fail(StatusCode::CONFLICT, "no_steps", "There are no steps left to write a skill from."),
        UndoOver => fail(StatusCode::CONFLICT, "undo_over", "It's too late to undo that."),
        Db(detail) => {
            tracing::warn!("show it once: {detail}");
            internal()
        }
    }
}

async fn recording_or_404(state: &AppState, id: &str) -> Result<Recording, Response> {
    match state.db.get_recording(id).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err(show_once_error(ShowOnceError::NotFound)),
        Err(_) => Err(internal()),
    }
}

/// The recording as a reply, with the engine Write would use.
async fn recording_reply(state: &AppState, rec: &Recording) -> Value {
    let engine = engine_for(state, &rec.kwant_id).await;
    recording_json(rec, now(), engine.as_deref())
}

/// A step with its display number in the recording's list.
async fn numbered(state: &AppState, rec_id: &str, step: &StepRow) -> Value {
    let t = now();
    let steps = state.db.list_steps(rec_id, t).await.unwrap_or_default();
    let n = steps.iter().position(|s| s.id == step.id).map(|i| i + 1).unwrap_or(steps.len());
    step_json(step, n, t)
}

/// After a change made from the review or the stage: the stage's socket gets the new state.
async fn tell_stage(state: &AppState, id: &str) {
    if let Ok(Some(rec)) = state.db.get_recording(id).await {
        stage::send(&rec.stage, Cmd::Frame(stage::state_frame(&rec)));
    }
}

/// `start_url`: an http(s) address, nothing else.
fn start_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.to_string())
}

/// A note's text: 1–1000 characters once cleaned.
fn note_text(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() || text.chars().count() > capture::MAX_NOTE {
        return None;
    }
    Some(capture::clean(text, capture::MAX_NOTE)).filter(|t| !t.is_empty())
}

// 1.1 Start

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StartIn {
    bot: Option<String>,
    stage: Option<String>,
    start_url: Option<String>,
    first_note: Option<String>,
}

/// Whether `id` is a stage: a team, channel or bot.
async fn is_stage(state: &AppState, id: &str) -> bool {
    matches!(state.db.get_kwant(id).await, Ok(Some(_)))
        || matches!(state.db.get_channel(id).await, Ok(Some(_)))
        || matches!(state.db.get_team(id).await, Ok(Some(_)))
}

pub async fn start(State(state): State<AppState>, Json(p): Json<StartIn>) -> Response {
    let Some(bot_id) = p.bot.as_deref().map(str::trim).filter(|b| !b.is_empty()) else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "Say which bot to show.");
    };
    let url = match p.start_url.as_deref().filter(|u| !u.trim().is_empty()) {
        Some(raw) => match start_url(raw) {
            Some(u) => Some(u),
            None => return fail(StatusCode::BAD_REQUEST, "bad_request", "The start address must be an http or https link."),
        },
        None => None,
    };
    let note = match p.first_note.as_deref().filter(|n| !n.trim().is_empty()) {
        Some(raw) => match note_text(raw) {
            Some(n) => Some(n),
            None => return fail(StatusCode::BAD_REQUEST, "bad_request", "A note can be at most 1000 characters."),
        },
        None => None,
    };
    let bot = match state.db.get_kwant(bot_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "no_bot", "That bot isn't there."),
        Err(_) => return internal(),
    };
    let stage_id = p.stage.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(&bot.id).to_string();
    if !is_stage(&state, &stage_id).await {
        return fail(StatusCode::NOT_FOUND, "no_stage", "That browser isn't a team's, a channel's or a bot's.");
    }
    if !stage::taken_over(&stage_id) {
        return fail(StatusCode::CONFLICT, "not_taken_over", "Take over the browser first, then start showing.");
    }
    let t = now();
    let rec = match state.db.start_recording(&bot.id, &bot.space_id, &stage_id, t).await {
        Ok(r) => r,
        Err(e) => return show_once_error(e),
    };
    if let Some(text) = note {
        if let Err(e) = state.db.add_note(&rec.id, &text, None, t).await {
            tracing::warn!("show it once: the first note wasn't saved: {e:?}");
        }
    }
    if !stage::send(&stage_id, Cmd::Begin { recording_id: rec.id.clone(), start_url: url }) {
        // Take over ended between the check and now: the socket went away, not a Hand back
        let _ = state.db.stop_recording(&rec.id, "closed", now()).await;
        emit_id(&state, &rec.id).await;
        return fail(StatusCode::CONFLICT, "not_taken_over", "Take over ended before the recording could start.");
    }
    let rec = state.db.get_recording(&rec.id).await.ok().flatten().unwrap_or(rec);
    emit(&state, &rec).await;
    (StatusCode::CREATED, Json(json!({"recording": recording_reply(&state, &rec).await}))).into_response()
}

// 1.2 Read

#[derive(Debug, Deserialize)]
pub struct OpenQuery {
    bot: Option<String>,
}

pub async fn open_for(State(state): State<AppState>, Query(q): Query<OpenQuery>) -> Response {
    let Some(bot) = q.bot.as_deref().filter(|b| !b.is_empty()) else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "Say which bot.");
    };
    let rec = match state.db.open_recording_for(bot).await {
        Ok(r) => r,
        Err(_) => return internal(),
    };
    // A26: where the bot's stage is (or was last), scrubbed like a step's address, to prefill Start
    let stage_id = rec.as_ref().map(|r| r.stage.clone()).unwrap_or_else(|| bot.to_string());
    let last_url = last_url(&state, &stage_id).await;
    let recording = match &rec {
        Some(r) => recording_reply(&state, r).await,
        None => Value::Null,
    };
    Json(json!({"recording": recording, "last_url": last_url})).into_response()
}

/// The page the stage's browser is on, or was last on: origin and path only (api.md §3), `None`
/// when it has no http(s) page. Never opens a browser.
async fn last_url(state: &AppState, stage_id: &str) -> Option<String> {
    let current = state.browser_controllers.lock().await.get(stage_id).map(|c| c.current_url.clone())?;
    let raw = current.lock().await.clone()?;
    prefill(&raw)
}

/// A browser address as Start's prefill: scrubbed like a step's (no query, fragment, login or
/// long ids), `None` for a page that isn't http(s).
fn prefill(raw: &str) -> Option<String> {
    super::scrub::scrub(raw).map(|p| format!("{}{}", p.origin, p.path))
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let rec = match recording_or_404(&state, &id).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let t = now();
    let steps = state.db.list_steps(&id, t).await.unwrap_or_default();
    Json(json!({"recording": recording_reply(&state, &rec).await, "steps": super::steps_json(&steps, t)})).into_response()
}

// 1.3 While recording

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NoteIn {
    text: String,
    after: Option<i64>,
}

pub async fn add_note(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<NoteIn>) -> Response {
    let Some(text) = note_text(&p.text) else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "A note is 1 to 1000 characters.");
    };
    match state.db.add_note(&id, &text, p.after, now()).await {
        Ok(step) => {
            let step = numbered(&state, &id, &step).await;
            tell_stage(&state, &id).await;
            (StatusCode::CREATED, Json(json!({"step": step}))).into_response()
        }
        Err(e) => show_once_error(e),
    }
}

/// The reason a Stop body gives (A27): none, `{}` or `"stop"` is Stop; `"handed_back"` is Hand back.
/// Anything else is refused.
fn stop_reason(body: &[u8]) -> Option<&'static str> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Some("stop");
    }
    let v: Value = serde_json::from_slice(body).ok()?;
    match v.as_object()?.get("reason") {
        None | Some(Value::Null) => Some("stop"),
        Some(Value::String(r)) if r == "stop" => Some("stop"),
        Some(Value::String(r)) if r == "handed_back" => Some("handed_back"),
        Some(_) => None,
    }
}

pub async fn stop(State(state): State<AppState>, Path(id): Path<String>, body: Bytes) -> Response {
    let Some(reason) = stop_reason(&body) else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "A recording stops with no reason, or with \"handed_back\".");
    };
    let rec = match recording_or_404(&state, &id).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    // the stage takes the field edit still pending in the page, then stops it (and tells the chat)
    if rec.state == "recording" {
        stage::stop_live(&rec.stage, &id, reason).await;
    }
    match state.db.stop_recording(&id, reason, now()).await {
        Ok(true) => {
            stage::send(&rec.stage, Cmd::End { recording_id: id.clone(), reason: reason.into() });
            emit_id(&state, &id).await;
        }
        // Stop twice, or the stage stopped it: a no-op here
        Ok(false) => {}
        Err(e) => return show_once_error(e),
    }
    match recording_or_404(&state, &id).await {
        Ok(rec) => Json(json!({"recording": recording_reply(&state, &rec).await})).into_response(),
        Err(r) => r,
    }
}

pub async fn resume(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let rec = match recording_or_404(&state, &id).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !stage::taken_over(&rec.stage) {
        return fail(StatusCode::CONFLICT, "not_taken_over", "Take over the browser first, then keep recording.");
    }
    let rec = match state.db.resume_recording(&id, now()).await {
        Ok(r) => r,
        Err(e) => return show_once_error(e),
    };
    if !stage::send(&rec.stage, Cmd::Begin { recording_id: id.clone(), start_url: None }) {
        let _ = state.db.stop_recording(&id, "closed", now()).await;
        emit_id(&state, &id).await;
        return fail(StatusCode::CONFLICT, "not_taken_over", "Take over ended before the recording could go on.");
    }
    emit(&state, &rec).await;
    Json(json!({"recording": recording_reply(&state, &rec).await})).into_response()
}

// 1.4 Review

pub async fn delete_step(State(state): State<AppState>, Path((id, step_id)): Path<(String, i64)>) -> Response {
    match state.db.delete_step(&id, step_id, now()).await {
        Ok(step) => {
            let step = numbered(&state, &id, &step).await;
            tell_stage(&state, &id).await;
            Json(json!({"step": step})).into_response()
        }
        Err(e) => show_once_error(e),
    }
}

pub async fn restore_step(State(state): State<AppState>, Path((id, step_id)): Path<(String, i64)>) -> Response {
    match state.db.restore_step(&id, step_id, now()).await {
        Ok(step) => {
            let step = numbered(&state, &id, &step).await;
            tell_stage(&state, &id).await;
            Json(json!({"step": step})).into_response()
        }
        Err(e) => show_once_error(e),
    }
}

pub async fn discard(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.discard_recording(&id).await {
        Ok(rec) => {
            stage::recording_ended(&id, "discarded");
            let _ = state.event_sender.send(RuntimeEvent::RecordingUpdated {
                recording: json!({"id": rec.id, "kwant_id": rec.kwant_id, "space_id": rec.space_id, "deleted": true}),
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => show_once_error(e),
    }
}

// 1.5 Write

pub async fn write(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let rec = match recording_or_404(&state, &id).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    if rec.state != "stopped" {
        return show_once_error(ShowOnceError::WrongState(rec.state));
    }
    if engine_for(&state, &rec.kwant_id).await.is_none() {
        return fail(StatusCode::CONFLICT, "no_engine", "This bot has no engine to write the skill on.");
    }
    let rec = match state.db.begin_writing(&id).await {
        Ok(r) => r,
        Err(e) => return show_once_error(e),
    };
    emit(&state, &rec).await;
    let reply = recording_reply(&state, &rec).await;
    let bg = state.clone();
    tokio::spawn(async move { draft::write(bg, id).await });
    (StatusCode::ACCEPTED, Json(json!({"recording": reply}))).into_response()
}

// 1.6 On a shown skill

pub async fn delete_skill_steps(State(state): State<AppState>, Path(skill_id): Path<String>) -> Response {
    match state.db.delete_shown_steps(&skill_id, now()).await {
        Ok(Some(rec)) => {
            emit(&state, &rec).await;
            super::skill_changed(&state, &skill_id).await;
            Json(json!({"recording": recording_reply(&state, &rec).await})).into_response()
        }
        Ok(None) => fail(StatusCode::NOT_FOUND, "not_shown", "That skill wasn't shown with Show it once."),
        Err(_) => internal(),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct TryIn {
    inputs: HashMap<String, String>,
}

pub async fn try_once(State(state): State<AppState>, Path(skill_id): Path<String>, body: Option<Json<TryIn>>) -> Response {
    let inputs = body.map(|Json(b)| b.inputs).unwrap_or_default();
    match trial::start(&state, &skill_id, inputs).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err((status, code, message)) => fail(status, code, message),
    }
}

// 1.7 Test seams

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ClockIn {
    offset_secs: i64,
}

/// `POST /api/show-once/test/clock`: only registered in a build with the `test-seams` feature
/// (see [`router_with`]), and refused anyway without it.
pub async fn test_clock(Json(p): Json<ClockIn>) -> Response {
    if !crate::clock::TEST_SEAMS {
        return StatusCode::NOT_FOUND.into_response();
    }
    crate::clock::set_test_offset(p.offset_secs);
    Json(json!({"offset_secs": crate::clock::offset_secs(), "now": crate::clock::now().to_rfc3339()})).into_response()
}

/// `POST /api/show-once/test/sweep`: like [`test_clock`], `test-seams` builds only.
pub async fn test_sweep(State(state): State<AppState>) -> Response {
    if !crate::clock::TEST_SEAMS {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(super::sweep(&state).await).into_response()
}

/// Every Show it once route (api.md §1). Merged into `create_router` before the auth layers, so
/// the guard (and `auth::person_only`) covers all of them. The §1.7 test seams are there only in a
/// build with the `test-seams` feature (`cargo test`, or a QA build made with `--features
/// test-seams`); a release, `make run` or Docker build doesn't have the routes at all.
pub fn router() -> Router<AppState> {
    router_with(crate::clock::TEST_SEAMS)
}

/// [`router`], with or without the §1.7 test seams.
pub fn router_with(seams: bool) -> Router<AppState> {
    let routes = Router::new()
        .route("/api/show-once/recordings", post(start).get(open_for))
        .route("/api/show-once/recordings/:id", get_route(get).delete(discard))
        .route("/api/show-once/recordings/:id/notes", post(add_note))
        .route("/api/show-once/recordings/:id/stop", post(stop))
        .route("/api/show-once/recordings/:id/resume", post(resume))
        .route("/api/show-once/recordings/:id/steps/:step_id", delete(delete_step))
        .route("/api/show-once/recordings/:id/steps/:step_id/restore", post(restore_step))
        .route("/api/show-once/recordings/:id/write", post(write))
        .route("/api/show-once/skills/:skill_id/steps", delete(delete_skill_steps))
        .route("/api/show-once/skills/:skill_id/try", post(try_once));
    if !seams {
        return routes;
    }
    routes
        .route("/api/show-once/test/clock", post(test_clock))
        .route("/api/show-once/test/sweep", post(test_sweep))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_address_is_http_or_https_only() {
        assert_eq!(start_url(" https://shop.example.com/admin ").as_deref(), Some("https://shop.example.com/admin"));
        assert!(start_url("http://127.0.0.1:9491/").is_some());
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x", "chrome://settings", "", "not a url"] {
            assert!(start_url(bad).is_none(), "{bad}");
        }
        assert!(start_url(&format!("https://a.example/{}", "x".repeat(MAX_URL))).is_none());
    }

    #[test]
    fn the_start_prefill_is_scrubbed_like_a_step() {
        assert_eq!(
            prefill("https://sam:pw@shop.example.com/admin/orders/AbCdEfGhIjKlMnOpQrSt?token=s3cret#x").as_deref(),
            Some("https://shop.example.com/admin/orders/…")
        );
        assert_eq!(prefill("http://127.0.0.1:9491/a").as_deref(), Some("http://127.0.0.1:9491/a"));
        for none in ["about:blank", "data:text/html,x", "chrome://newtab", "file:///etc/passwd"] {
            assert!(prefill(none).is_none(), "{none}");
        }
    }

    #[test]
    fn a_stop_says_stop_or_handed_back_and_nothing_else() {
        for (body, want) in [
            ("", Some("stop")),
            ("  ", Some("stop")),
            ("{}", Some("stop")),
            (r#"{"reason":null}"#, Some("stop")),
            (r#"{"reason":"stop"}"#, Some("stop")),
            (r#"{"reason":"handed_back"}"#, Some("handed_back")),
            (r#"{"reason":"limit_time"}"#, None),
            (r#"{"reason":"closed"}"#, None),
            (r#"{"reason":1}"#, None),
            ("[]", None),
            ("not json", None),
        ] {
            assert_eq!(stop_reason(body.as_bytes()), want, "{body}");
        }
    }

    #[test]
    fn a_note_is_one_to_a_thousand_characters() {
        assert_eq!(note_text("  use the\nexport  button ").as_deref(), Some("use the export button"));
        assert!(note_text("   ").is_none());
        assert!(note_text(&"n".repeat(1001)).is_none());
        assert!(note_text(&"n".repeat(1000)).is_some());
    }
}
