//! Camera-to-Completed's API (see `crate::documents`). The person only: every route needs the
//! API token; a bot's turn token gets 403, no token 401. Nothing here sends anything.
//!
//! - `POST   /api/documents {attachment_ids: [id, ..]}` → 202 `{case_id}`. 1–5 uploads from
//!   `POST /api/attachments` not yet sent anywhere (else 400; 409 when one was), each PNG, JPEG,
//!   WebP or PDF (else 400). The letter is read in the background; 409 `{code: "busy"}` while
//!   another one is being read. The case shows in `GET /api/zombie/cases` at once.
//! - `GET    /api/documents/:id` → `DocumentView`: the case, what was read and the reminders.
//! - `DELETE /api/documents/:id` → `{deleted: true}`: the case, the extracted text, the reminders
//!   and the page files nothing else uses. 409 while it is being read.
//! - `PATCH  /api/zombie/cases/:id/fields {sender?, request?, reply_to?, deadline?, references?,
//!   amounts?, checklist?}` → `DocumentView` (see `documents::extract::patch`). Letter cases
//!   only (else 422), 409 while being read, 400 on a value that does not parse.

#![allow(clippy::result_large_err)]

use super::zombie_routes::CaseView;
use super::AppState;
use crate::db::documents::ExtractionRow;
use crate::documents::extract;
use crate::documents::{Fields, MAX_PAGES, PAGE_MIMES};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const REFUSAL: &str = "only the person can manage their letters";

type Res = Result<Response, Response>;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn db_error(e: crate::db::DbError) -> Response {
    tracing::warn!("documents api: database error: {e}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "kwant could not read or save the letter")
}

async fn person(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await
}

/// One page of the letter, for the UI to show beside the fields.
#[derive(Debug, Serialize)]
pub struct PageView {
    pub id: String,
    pub name: String,
    pub mime: String,
    /// `/api/attachments/:id/content`.
    pub url: String,
}

/// What was read from the letter.
#[derive(Debug, Serialize)]
pub struct ExtractionView {
    /// `running`, `read`, `text_only`, `unreadable`.
    pub status: String,
    /// `pdf_text`, `model_vision`, `pdf_text+model_vision`, or null.
    pub method: Option<String>,
    /// Why it is unreadable or text only, in kwant's words.
    pub note: Option<String>,
    pub fields: Fields,
    /// Everything read from the letter (PDF text and the model's transcript of the images).
    pub text: String,
    pub read_by: Option<String>,
    pub pages: Vec<PageView>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReminderView {
    /// 7, 1 or 0 days before the deadline.
    pub offset_days: i64,
    pub due_at: DateTime<Utc>,
    pub fired_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct DocumentView {
    pub case: CaseView,
    pub extraction: Option<ExtractionView>,
    pub reminders: Vec<ReminderView>,
}

async fn document_view(state: &AppState, case_id: &str) -> Res {
    // The extraction first: the reader writes the case and reminders before the extraction's
    // status, so a view that sees the letter read also sees the case it updated. Read the other
    // way round, a poll landing between those writes got "read" with the case still "reading…"
    let extraction = state.db.document_extraction(case_id).await.map_err(db_error)?;
    let Some(c) = state.db.zombie_case(case_id).await.map_err(db_error)?.filter(|c| !c.redacted && c.source == "document") else {
        return Err(error(StatusCode::NOT_FOUND, "no such letter"));
    };
    let extraction = match extraction {
        Some(e) => Some(extraction_view(state, e).await),
        None => None,
    };
    let reminders = state
        .db
        .document_reminders(case_id)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(|r| ReminderView { offset_days: r.offset_days, due_at: r.due_at, fired_at: r.fired_at })
        .collect();
    Ok(Json(DocumentView { case: CaseView::of(c, Utc::now()), extraction, reminders }).into_response())
}

async fn extraction_view(state: &AppState, e: ExtractionRow) -> ExtractionView {
    let ids: Vec<String> = serde_json::from_str(&e.attachment_ids).unwrap_or_default();
    let mut pages = Vec::new();
    for id in ids {
        if let Ok(Some(a)) = state.db.get_attachment(&id).await {
            pages.push(PageView { url: format!("/api/attachments/{}/content", a.id), id: a.id, name: a.name, mime: a.mime });
        }
    }
    ExtractionView {
        status: e.status,
        method: e.method,
        note: e.note,
        fields: serde_json::from_str(&e.fields).unwrap_or_default(),
        text: e.text,
        read_by: e.read_by,
        pages,
        updated_at: e.updated_at,
    }
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub attachment_ids: Vec<String>,
}

pub async fn create(State(state): State<AppState>, headers: HeaderMap, body: Option<Json<CreateBody>>) -> Res {
    person(&state, &headers).await?;
    let Some(Json(b)) = body else {
        return Err(error(StatusCode::BAD_REQUEST, "send {attachment_ids: [..]}"));
    };
    if b.attachment_ids.is_empty() || b.attachment_ids.len() > MAX_PAGES {
        return Err(error(StatusCode::BAD_REQUEST, &format!("send 1 to {MAX_PAGES} pages")));
    }
    let mut attachments = Vec::new();
    for (i, id) in b.attachment_ids.iter().enumerate() {
        if b.attachment_ids[..i].contains(id) {
            return Err(error(StatusCode::BAD_REQUEST, "the same page was sent twice"));
        }
        let found = match uuid::Uuid::parse_str(id) {
            Ok(_) => state.db.get_attachment(id).await.map_err(db_error)?,
            Err(_) => None,
        };
        let Some(a) = found else {
            return Err(error(StatusCode::BAD_REQUEST, "a page is not a known upload"));
        };
        if a.source != "user" || a.message_id.is_some() {
            return Err(error(StatusCode::CONFLICT, "a page was already used: upload it again"));
        }
        if !PAGE_MIMES.contains(&a.mime.as_str()) {
            return Err(error(StatusCode::BAD_REQUEST, "pages must be photos (PNG, JPEG, WebP) or PDFs"));
        }
        attachments.push(a);
    }
    let Some(guard) = state.documents.try_begin() else {
        return Err((StatusCode::CONFLICT, Json(json!({"error": "another letter is being read: try again in a minute", "code": "busy"}))).into_response());
    };
    let now = Utc::now();
    let case_id = uuid::Uuid::new_v4().to_string();
    let context = format!("document:{case_id}");
    let linked = state.db.link_attachments(&b.attachment_ids, &context, &context, None, None, None).await.map_err(db_error)?;
    if linked as usize != attachments.len() {
        return Err(error(StatusCode::CONFLICT, "a page was already used: upload it again"));
    }
    state.db.zombie_put_case(&extract::new_case(&case_id, now)).await.map_err(db_error)?;
    state.db.document_put_extraction(&extract::running_row(&case_id, &b.attachment_ids, now)).await.map_err(db_error)?;
    tracing::info!(case_id = %case_id, pages = attachments.len(), "documents: reading a letter");
    tokio::spawn(extract::run(state.clone(), case_id.clone(), attachments, guard));
    Ok((StatusCode::ACCEPTED, Json(json!({"case_id": case_id}))).into_response())
}

pub async fn get_one(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    document_view(&state, &id).await
}

async fn being_read(state: &AppState, id: &str) -> Result<Option<ExtractionRow>, Response> {
    let e = state.db.document_extraction(id).await.map_err(db_error)?;
    if e.as_ref().is_some_and(|e| e.status == "running") {
        return Err(error(StatusCode::CONFLICT, "the letter is still being read"));
    }
    Ok(e)
}

pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    let Some(_) = state.db.zombie_case(&id).await.map_err(db_error)?.filter(|c| c.source == "document") else {
        return Err(error(StatusCode::NOT_FOUND, "no such letter"));
    };
    being_read(&state, &id).await?;
    let removed = state.db.document_delete(&id).await.map_err(db_error)?;
    let root = super::attachments::root(&state);
    let mut files = 0;
    for a in removed {
        if !state.db.attachment_blob_in_use(&a.sha256).await.map_err(db_error)? {
            if let Some(p) = super::attachments::blob_path(&root, &a.sha256) {
                if tokio::fs::remove_file(&p).await.is_ok() {
                    files += 1;
                }
            }
        }
    }
    tracing::info!(case_id = %id, files, "documents: letter deleted");
    Ok(Json(json!({"deleted": true})).into_response())
}

pub async fn patch_fields(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<Value>>) -> Res {
    person(&state, &headers).await?;
    let Some(mut c) = state.db.zombie_case(&id).await.map_err(db_error)?.filter(|c| !c.redacted) else {
        return Err(error(StatusCode::NOT_FOUND, "no such case"));
    };
    if c.source != "document" {
        return Err(error(StatusCode::UNPROCESSABLE_ENTITY, "only letters have fields to correct"));
    }
    let Some(mut e) = being_read(&state, &id).await? else {
        return Err(error(StatusCode::UNPROCESSABLE_ENTITY, "this letter has nothing read yet"));
    };
    let Some(Json(body)) = body else {
        return Err(error(StatusCode::BAD_REQUEST, "send a JSON object of fields"));
    };
    let old: Fields = serde_json::from_str(&e.fields).unwrap_or_default();
    let fields = extract::patch(&old, &body).map_err(|m| error(StatusCode::BAD_REQUEST, &m))?;
    let now = Utc::now();
    e.fields = serde_json::to_string(&fields).unwrap_or_else(|_| "{}".into());
    e.updated_at = now;
    state.db.document_put_extraction(&e).await.map_err(db_error)?;
    extract::apply(&mut c, &fields, now);
    state.db.zombie_put_case(&c).await.map_err(db_error)?;
    let reminders = extract::reschedule(&state.db, &c, &fields, now).await.map_err(db_error)?;
    tracing::info!(case_id = %id, reminders, uncertain = fields.uncertain_count(), "documents: fields corrected");
    document_view(&state, &id).await
}

/// At startup: a letter a restart cut short is marked unreadable, so the person can fill it in.
pub async fn mark_interrupted(state: &AppState) {
    match state.db.document_mark_interrupted("kwant restarted while reading it: fill in the fields yourself, or add the letter again", Utc::now()).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("documents: {n} letters were cut short by a restart"),
        Err(e) => tracing::warn!("documents: could not mark interrupted letters: {e}"),
    }
}
