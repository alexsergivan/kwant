//! Zombie Mission Hunter's API. The person only: every route needs the API token (see
//! `auth::person_only`); a bot's turn token gets 403, no token 401. Nothing here sends anything.
//!
//! Cases:
//! - `GET  /api/zombie/cases?status=open|snoozed|dismissed|waiting_reply|closed` → `{cases:
//!   [CaseView], running}` (all statuses when `status` is absent; redacted cases never listed).
//! - `GET  /api/zombie/cases/:id` → `CaseView`.
//! - `POST /api/zombie/cases/:id/snooze {days} | {until}` → `CaseView` (1–365 days ahead).
//! - `POST /api/zombie/cases/:id/dismiss {scope?: "thread" | "sender", reason?}` → `CaseView`.
//!   `thread` (default): this conversation never comes back. `sender`: nothing from this address
//!   or login in this source comes back, and its other open cases are dismissed too.
//! - `POST /api/zombie/cases/:id/done` → `CaseView` (closed, resolution `done`).
//! - `POST /api/zombie/cases/:id/followed-up` → `CaseView` (`waiting_reply`: reopens if nobody
//!   answers within the stale days).
//! - `POST /api/zombie/cases/:id/reopen` → `CaseView` (open again; lifts a sender mute).
//! - `POST /api/zombie/cases/:id/draft` → `CaseView` with a new draft (the model's when one can
//!   run and keeps the reference, else the template).
//!
//! Sources (the kwant source is built in and switched in the settings):
//! - `GET    /api/zombie/sources` → `{sources: [SourceView], vault_locked}`.
//! - `POST   /api/zombie/sources {kind: "imap"|"github", label, config, secret}` → 201 `{source,
//!   test}`. The connection is tested first; nothing is saved when it fails (400 `{error}`). The
//!   secret goes to the password vault (423 when locked) and is never echoed.
//! - `PATCH  /api/zombie/sources/:id {label?, enabled?, config?, secret?}` → `{source, test?}`.
//! - `DELETE /api/zombie/sources/:id` → `{deleted, cases_removed}`: the secret, its cases and
//!   mutes go too.
//! - `POST   /api/zombie/sources/:id/test` → `{ok: true, test}` or 400 `{error}`.
//!
//! Runs and settings:
//! - `POST /api/zombie/run {review?: bool}` → 202 `{started: true}`; 409 when one is running.
//! - `GET  /api/zombie/status` → `{running, last_scan: ScanReport | null, next_review_at}`.
//! - `GET  /api/zombie/settings` → `Settings`; `PUT` the whole object → `Settings` (400 on bad).
//! - `GET  /api/zombie/reviews` → `{reviews: [ReviewRow]}` (last 20; `shown` is an array of case ids).

#![allow(clippy::result_large_err)]

use super::AppState;
use crate::db::zombie::{CaseRow, SourceRow, CASE_STATUSES};
use crate::sandbox::credentials::{normalize_origin, CredentialStore, StoreError};
use crate::zombie::draft::Draft;
use crate::zombie::github::{GithubConfig, GithubSource};
use crate::zombie::hunt::{self, ScanError};
use crate::zombie::imap::{ImapConfig, ImapSource};
use crate::zombie::settings::Settings;
use crate::zombie::{Evidence, VAULT_OWNER};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use zeroize::Zeroizing;

const REFUSAL: &str = "only the person can manage loose ends";
const MAX_SECRET: usize = 4096;

type Res = Result<Response, Response>;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn db_error(e: crate::db::DbError) -> Response {
    tracing::warn!("zombie api: database error: {e}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "kwant could not read or save its cases")
}

fn vault_error(e: StoreError) -> Response {
    match e {
        StoreError::Locked => (
            StatusCode::LOCKED,
            Json(json!({"error": "the password vault is locked: unlock it first", "locked": true})),
        )
            .into_response(),
        e => {
            tracing::warn!("zombie api: vault error: {e}");
            error(StatusCode::SERVICE_UNAVAILABLE, "the password vault is unavailable right now")
        }
    }
}

async fn person(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await
}

/// A case as the UI shows it.
#[derive(Debug, Serialize)]
pub struct CaseView {
    pub id: String,
    pub status: String,
    /// `awaiting_reply`, `promise_lapsed`, `you_promised`, `you_owe_reply`, `stale_issue`,
    /// `failed_task`, `unanswered_question`.
    pub kind: String,
    /// `email`, `issue`, `pull_request`, `kwant_thread`, `kwant_question`, `kwant_delegation`.
    pub conv_kind: String,
    /// `kwant`, `imap`, `github`.
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub counterpart: Option<String>,
    pub counterpart_name: Option<String>,
    pub reference: Option<String>,
    /// `https://…` (GitHub) or `kwant://thread/<id>`; email has none.
    pub link: Option<String>,
    pub why: String,
    pub evidence: Vec<Evidence>,
    pub draft: Option<Draft>,
    pub age_days: i64,
    pub last_activity_at: DateTime<Utc>,
    pub first_seen: DateTime<Utc>,
    pub status_changed_at: DateTime<Utc>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub waiting_since: Option<DateTime<Utc>>,
    pub follow_ups: i64,
    /// `YYYY-MM-DD`, a promised date that passed.
    pub deadline: Option<String>,
    /// `done`, `resolved`.
    pub resolution: Option<String>,
    pub dismiss_reason: Option<String>,
    pub space_id: Option<String>,
}

impl CaseView {
    fn of(c: CaseRow, now: DateTime<Utc>) -> Self {
        Self {
            evidence: serde_json::from_str(&c.evidence).unwrap_or_default(),
            draft: c.draft.as_deref().and_then(|d| serde_json::from_str(d).ok()),
            age_days: (now - c.last_activity_at).num_days().max(0),
            id: c.id,
            status: c.status,
            kind: c.kind,
            conv_kind: c.conv_kind,
            source: c.source,
            source_id: c.source_id,
            title: c.title,
            counterpart: c.counterpart,
            counterpart_name: c.counterpart_name,
            reference: c.reference,
            link: c.link,
            why: c.why,
            last_activity_at: c.last_activity_at,
            first_seen: c.first_seen,
            status_changed_at: c.status_changed_at,
            snoozed_until: c.snoozed_until,
            waiting_since: c.waiting_since,
            follow_ups: c.follow_ups,
            deadline: c.deadline,
            resolution: c.resolution,
            dismiss_reason: c.dismiss_reason,
            space_id: c.space_id,
        }
    }
}

fn view(c: CaseRow) -> Response {
    Json(CaseView::of(c, Utc::now())).into_response()
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

pub async fn list_cases(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<ListQuery>) -> Res {
    person(&state, &headers).await?;
    let status = q.status.filter(|s| !s.is_empty());
    if status.as_deref().is_some_and(|s| !CASE_STATUSES.contains(&s)) {
        return Err(error(StatusCode::BAD_REQUEST, "status must be open, snoozed, dismissed, waiting_reply or closed"));
    }
    let now = Utc::now();
    let cases: Vec<CaseView> = state.db.zombie_cases(status.as_deref()).await.map_err(db_error)?.into_iter().map(|c| CaseView::of(c, now)).collect();
    Ok(Json(json!({"cases": cases, "running": state.zombie.running()})).into_response())
}

async fn case_or_404(state: &AppState, id: &str) -> Result<CaseRow, Response> {
    match state.db.zombie_case(id).await.map_err(db_error)? {
        Some(c) if !c.redacted => Ok(c),
        _ => Err(error(StatusCode::NOT_FOUND, "no such case")),
    }
}

pub async fn get_case(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    Ok(view(case_or_404(&state, &id).await?))
}

fn set_status(c: &mut CaseRow, status: &str) {
    if c.status != status {
        c.status = status.into();
        c.status_changed_at = Utc::now();
    }
}

#[derive(Deserialize, Default)]
pub struct SnoozeBody {
    pub days: Option<i64>,
    pub until: Option<DateTime<Utc>>,
}

pub async fn snooze(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(b): Json<SnoozeBody>) -> Res {
    person(&state, &headers).await?;
    let mut c = case_or_404(&state, &id).await?;
    let now = Utc::now();
    let until = match (b.until, b.days) {
        (Some(u), _) => u,
        (None, Some(d)) if (1..=365).contains(&d) => now + Duration::days(d),
        _ => return Err(error(StatusCode::BAD_REQUEST, "give days (1–365) or until")),
    };
    if until <= now || until > now + Duration::days(366) {
        return Err(error(StatusCode::BAD_REQUEST, "until must be in the next year"));
    }
    set_status(&mut c, "snoozed");
    c.snoozed_until = Some(until);
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    Ok(view(c))
}

#[derive(Deserialize, Default)]
pub struct DismissBody {
    pub scope: Option<String>,
    pub reason: Option<String>,
}

pub async fn dismiss(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<DismissBody>>) -> Res {
    person(&state, &headers).await?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let mut c = case_or_404(&state, &id).await?;
    let reason = b.reason.map(|r| crate::zombie::clip(r.trim(), 200)).filter(|r| !r.is_empty());
    match b.scope.as_deref().unwrap_or("thread") {
        "thread" => {}
        "sender" => {
            let Some(sender) = c.counterpart.clone().filter(|s| !s.is_empty()) else {
                return Err(error(StatusCode::BAD_REQUEST, "this case has no sender to mute"));
            };
            state.db.zombie_mute(&c.source_id, "sender", &sender).await.map_err(db_error)?;
            for mut other in state.db.zombie_cases_of_source(&c.source_id).await.map_err(db_error)? {
                let same = other.counterpart.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&sender));
                if same && other.id != c.id && matches!(other.status.as_str(), "open" | "snoozed" | "waiting_reply") {
                    set_status(&mut other, "dismissed");
                    other.dismiss_reason = Some("sender muted".into());
                    state.db.zombie_put_case(&other).await.map_err(db_error)?;
                }
            }
        }
        _ => return Err(error(StatusCode::BAD_REQUEST, "scope must be thread or sender")),
    }
    set_status(&mut c, "dismissed");
    c.dismiss_reason = reason;
    c.snoozed_until = None;
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    Ok(view(c))
}

pub async fn done(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    let mut c = case_or_404(&state, &id).await?;
    set_status(&mut c, "closed");
    c.resolution = Some("done".into());
    c.snoozed_until = None;
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    Ok(view(c))
}

pub async fn followed_up(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    let mut c = case_or_404(&state, &id).await?;
    set_status(&mut c, "waiting_reply");
    c.waiting_since = Some(Utc::now());
    c.snoozed_until = None;
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    Ok(view(c))
}

pub async fn reopen(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    let mut c = case_or_404(&state, &id).await?;
    if let Some(sender) = &c.counterpart {
        state.db.zombie_unmute(&c.source_id, "sender", sender).await.map_err(db_error)?;
    }
    set_status(&mut c, "open");
    c.resolution = None;
    c.dismiss_reason = None;
    c.snoozed_until = None;
    c.waiting_since = None;
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    Ok(view(c))
}

pub async fn regenerate_draft(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    let mut c = case_or_404(&state, &id).await?;
    let Some(d) = hunt::regenerate_draft(&state, &c).await else {
        return Err(error(StatusCode::UNPROCESSABLE_ENTITY, "this case has no draft"));
    };
    c.draft = serde_json::to_string(&d).ok();
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    Ok(view(c))
}

// ---- sources ----

/// A source as the UI shows it: never a secret.
#[derive(Debug, Serialize)]
pub struct SourceView {
    pub id: String,
    /// `kwant`, `imap`, `github`.
    pub kind: String,
    pub label: String,
    /// `ImapConfig` or `GithubConfig`; `{}` for kwant.
    pub config: Value,
    pub enabled: bool,
    /// The kwant source is built in: it can be switched off in the settings, not deleted.
    pub builtin: bool,
    pub last_scan_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

fn source_view(s: SourceRow) -> SourceView {
    SourceView {
        config: serde_json::from_str(&s.config).unwrap_or(Value::Null),
        id: s.id,
        kind: s.kind,
        label: s.label,
        enabled: s.enabled,
        builtin: false,
        last_scan_at: s.last_scan_at,
        last_status: s.last_status,
        last_error: s.last_error,
        created_at: Some(s.created_at),
    }
}

pub async fn list_sources(State(state): State<AppState>, headers: HeaderMap) -> Res {
    person(&state, &headers).await?;
    let settings = Settings::load(&state.db).await.map_err(db_error)?;
    let last = state.zombie.last_report();
    let kwant_scan = last.as_ref().and_then(|r| r.sources.iter().find(|s| s.kind == "kwant").map(|s| (r.started_at, s.clone())));
    let mut sources = vec![SourceView {
        id: crate::zombie::kwant_source::SOURCE_ID.into(),
        kind: "kwant".into(),
        label: "kwant's own threads, tasks and questions".into(),
        config: json!({}),
        enabled: settings.kwant_source,
        builtin: true,
        last_scan_at: kwant_scan.as_ref().map(|(at, _)| *at),
        last_status: kwant_scan.as_ref().map(|(_, s)| if s.ok { "ok".into() } else { "error".into() }),
        last_error: kwant_scan.and_then(|(_, s)| s.error),
        created_at: None,
    }];
    sources.extend(state.db.zombie_sources().await.map_err(db_error)?.into_iter().map(source_view));
    Ok(Json(json!({"sources": sources, "vault_locked": state.credentials.is_locked().await})).into_response())
}

/// A checked source config, as JSON to store, with its vault origin.
fn checked_config(kind: &str, config: &Value) -> Result<(String, String), Response> {
    let bad = |m: String| error(StatusCode::BAD_REQUEST, &m);
    match kind {
        "imap" => {
            let cfg: ImapConfig = serde_json::from_value(config.clone()).map_err(|e| bad(format!("config: {e}")))?;
            cfg.validate().map_err(bad)?;
            Ok((serde_json::to_string(&cfg).unwrap_or_default(), cfg.vault_origin()))
        }
        "github" => {
            let cfg: GithubConfig = serde_json::from_value(config.clone()).map_err(|e| bad(format!("config: {e}")))?;
            cfg.validate().map_err(bad)?;
            Ok((serde_json::to_string(&cfg).unwrap_or_default(), cfg.vault_origin()))
        }
        _ => Err(bad("kind must be imap or github".into())),
    }
}

/// Connects with `config` and `secret`: what the test found, or 400 with why not.
async fn test_connection(id: &str, kind: &str, config: &str, secret: Zeroizing<String>) -> Result<Value, Response> {
    let failed = |e: String| error(StatusCode::BAD_REQUEST, &format!("connection test failed: {e}"));
    match kind {
        "imap" => {
            let cfg: ImapConfig = serde_json::from_str(config).map_err(|_| failed("unreadable settings".into()))?;
            let report = ImapSource::new(id, cfg, secret, &[]).test().await.map_err(failed)?;
            Ok(serde_json::to_value(report).unwrap_or(Value::Null))
        }
        _ => {
            let cfg: GithubConfig = serde_json::from_str(config).map_err(|_| failed("unreadable settings".into()))?;
            let login = GithubSource::new(id, cfg, secret).test().await.map_err(failed)?;
            Ok(json!({"login": login}))
        }
    }
}

fn checked_label(label: &str) -> Result<String, Response> {
    let l = label.trim();
    if l.is_empty() || l.chars().count() > 80 || l.chars().any(char::is_control) {
        return Err(error(StatusCode::BAD_REQUEST, "label must be 1–80 characters"));
    }
    Ok(l.to_string())
}

fn checked_secret(secret: &str) -> Result<Zeroizing<String>, Response> {
    if secret.is_empty() || secret.len() > MAX_SECRET || secret.contains(['\r', '\n']) {
        return Err(error(StatusCode::BAD_REQUEST, "secret must be the password or token, on one line"));
    }
    Ok(Zeroizing::new(secret.to_string()))
}

async fn save_secret(state: &AppState, id: &str, origin: &str, secret: &str) -> Result<(), Response> {
    if let Some(refused) = super::passwords::refuse_local_save(state).await {
        return Err(refused);
    }
    let store = state.credentials.as_ref();
    let origin = normalize_origin(origin).map_err(|_| error(StatusCode::BAD_REQUEST, "the server address is not valid"))?;
    hunt::forget_secret(store, id).await.map_err(vault_error)?;
    store.save(VAULT_OWNER, &origin, id, secret).await.map_err(vault_error)?;
    Ok(())
}

#[derive(Deserialize)]
pub struct NewSource {
    pub kind: String,
    pub label: String,
    pub config: Value,
    pub secret: String,
}

pub async fn add_source(State(state): State<AppState>, headers: HeaderMap, Json(b): Json<NewSource>) -> Res {
    person(&state, &headers).await?;
    let label = checked_label(&b.label)?;
    let (config, origin) = checked_config(&b.kind, &b.config)?;
    let secret = checked_secret(&b.secret)?;
    drop(b.secret);
    if state.credentials.is_locked().await {
        return Err(vault_error(StoreError::Locked));
    }
    // Before the connection test, so a refused save never reaches the server
    if let Some(refused) = super::passwords::refuse_local_save(&state).await {
        return Err(refused);
    }
    let id = format!("src-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let test = test_connection(&id, &b.kind, &config, secret.clone()).await?;
    save_secret(&state, &id, &origin, &secret).await?;
    let row = SourceRow {
        id: id.clone(),
        kind: b.kind,
        label,
        config,
        enabled: true,
        last_scan_at: None,
        last_status: None,
        last_error: None,
        created_at: Utc::now(),
    };
    if let Err(e) = state.db.zombie_put_source(&row).await {
        let _ = hunt::forget_secret(state.credentials.as_ref(), &id).await;
        return Err(db_error(e));
    }
    Ok((StatusCode::CREATED, Json(json!({"source": source_view(row), "test": test}))).into_response())
}

#[derive(Deserialize, Default)]
pub struct SourcePatch {
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<Value>,
    pub secret: Option<String>,
}

async fn source_or_404(state: &AppState, id: &str) -> Result<SourceRow, Response> {
    state.db.zombie_source(id).await.map_err(db_error)?.ok_or_else(|| error(StatusCode::NOT_FOUND, "no such source"))
}

pub async fn update_source(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(b): Json<SourcePatch>) -> Res {
    person(&state, &headers).await?;
    let mut row = source_or_404(&state, &id).await?;
    if let Some(l) = &b.label {
        row.label = checked_label(l)?;
    }
    if let Some(e) = b.enabled {
        row.enabled = e;
    }
    let mut test = None;
    if b.config.is_some() || b.secret.is_some() {
        let (config, origin) = match &b.config {
            Some(c) => checked_config(&row.kind, c)?,
            None => (row.config.clone(), hunt::origin_of(&row).map_err(|e| error(StatusCode::BAD_REQUEST, &e))?),
        };
        let secret = match b.secret.as_deref() {
            Some(s) => checked_secret(s)?,
            None => {
                let old = hunt::origin_of(&row).map_err(|e| error(StatusCode::BAD_REQUEST, &e))?;
                hunt::source_secret(state.credentials.as_ref(), &id, &old)
                    .await
                    .map_err(vault_error)?
                    .ok_or_else(|| error(StatusCode::BAD_REQUEST, "no secret is saved for this source: send secret"))?
            }
        };
        test = Some(test_connection(&id, &row.kind, &config, secret.clone()).await?);
        save_secret(&state, &id, &origin, &secret).await?;
        row.config = config;
    }
    state.db.zombie_put_source(&row).await.map_err(db_error)?;
    Ok(Json(json!({"source": source_view(row), "test": test})).into_response())
}

pub async fn delete_source(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    source_or_404(&state, &id).await?;
    hunt::forget_secret(state.credentials.as_ref(), &id).await.map_err(vault_error)?;
    let removed = state.db.zombie_delete_cases_of_source(&id).await.map_err(db_error)?;
    let deleted = state.db.zombie_delete_source(&id).await.map_err(db_error)?;
    Ok(Json(json!({"deleted": deleted, "cases_removed": removed})).into_response())
}

pub async fn test_source(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    let row = source_or_404(&state, &id).await?;
    let origin = hunt::origin_of(&row).map_err(|e| error(StatusCode::BAD_REQUEST, &e))?;
    let secret = hunt::source_secret(state.credentials.as_ref(), &id, &origin)
        .await
        .map_err(vault_error)?
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "no secret is saved for this source: add it again"))?;
    let test = test_connection(&id, &row.kind, &row.config, secret).await?;
    Ok(Json(json!({"ok": true, "test": test})).into_response())
}

// ---- runs and settings ----

#[derive(Deserialize, Default)]
pub struct RunBody {
    #[serde(default)]
    pub review: bool,
}

pub async fn run(State(state): State<AppState>, headers: HeaderMap, body: Option<Json<RunBody>>) -> Res {
    person(&state, &headers).await?;
    let review = body.map(|Json(b)| b.review).unwrap_or(false);
    if state.zombie.running() {
        return Err(error(StatusCode::CONFLICT, "a scan is already running"));
    }
    let bg = state.clone();
    tokio::spawn(async move {
        let result = if review { hunt::review(&bg, "manual").await.map(|_| ()) } else { hunt::scan(&bg).await.map(|_| ()) };
        match result {
            Ok(()) | Err(ScanError::Busy) => {}
            Err(e) => tracing::info!("zombie: manual run failed: {e}"),
        }
    });
    Ok((StatusCode::ACCEPTED, Json(json!({"started": true}))).into_response())
}

pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Res {
    person(&state, &headers).await?;
    let settings = Settings::load(&state.db).await.map_err(db_error)?;
    let next = hunt::next_review(&state.db, &settings, Utc::now()).await;
    Ok(Json(json!({"running": state.zombie.running(), "last_scan": state.zombie.last_report(), "next_review_at": next})).into_response())
}

pub async fn get_settings(State(state): State<AppState>, headers: HeaderMap) -> Res {
    person(&state, &headers).await?;
    Ok(Json(Settings::load(&state.db).await.map_err(db_error)?).into_response())
}

pub async fn put_settings(State(state): State<AppState>, headers: HeaderMap, Json(s): Json<Settings>) -> Res {
    person(&state, &headers).await?;
    s.validate().map_err(|e| error(StatusCode::BAD_REQUEST, &e))?;
    let old = Settings::load(&state.db).await.map_err(db_error)?;
    s.save(&state.db).await.map_err(db_error)?;
    if old.schedule("") != s.schedule("") || old.enabled != s.enabled || old.review_enabled != s.review_enabled {
        hunt::reset_schedule(&state.db).await;
    }
    Ok(Json(s).into_response())
}

pub async fn reviews(State(state): State<AppState>, headers: HeaderMap) -> Res {
    person(&state, &headers).await?;
    let reviews: Vec<Value> = state
        .db
        .zombie_reviews(20)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(|r| {
            let shown: Vec<String> = serde_json::from_str(&r.shown).unwrap_or_default();
            let mut v = serde_json::to_value(&r).unwrap_or(Value::Null);
            v["shown"] = json!(shown);
            v
        })
        .collect();
    Ok(Json(json!({"reviews": reviews})).into_response())
}
