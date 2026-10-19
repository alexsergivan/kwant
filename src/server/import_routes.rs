//! Bringing bots in from other apps and taking them out again.
//!
//! - `POST /api/import/preview?space=&filename=`: a file as the raw body, or JSON `{ "text": … }`
//!   for a pasted reply. Parses and checks it; writes nothing and keeps nothing.
//! - `POST /api/import/apply?space=`: `{ "bundle": …, "skip": [refs] }`, the bundle preview
//!   returned. Checks it again and creates it all in one transaction.
//! - `GET /api/imports?space=`, `GET /api/imports/:id?space=`, `POST /api/imports/:id/undo?space=`.
//! - `GET /api/kwants/:id/export`, `GET /api/export?space=`: `kwant.bundle/1`, keys scrubbed.
//! - `GET /api/import/prompts`: the export prompt to send a bot in Grok, ChatGPT, Claude, ….

use super::spaces::Scope;
use super::AppState;
use crate::db::imports::{ImportRecord, UndoError};
use crate::db::spaces::SpaceId;
use crate::import::{self, parsers, Bundle, Checked, ImportError, BUNDLE_FORMAT, MAX_ZIP_BYTES};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;

/// An apply body: a bundle with up to 50 bots of 2 × 16k characters, plus skills.
const MAX_APPLY_BYTES: usize = 8 * 1024 * 1024;

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into(), "code": code }))).into_response()
}

fn import_error(e: ImportError) -> Response {
    match e {
        ImportError::TooLarge(m) => error(StatusCode::PAYLOAD_TOO_LARGE, "too_large", m),
        ImportError::Unsupported(m) => error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported", m),
        ImportError::Invalid(m) => error(StatusCode::UNPROCESSABLE_ENTITY, "invalid", m),
    }
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "import failed");
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "database error")
}

/// The body, read up to `limit` bytes. Only kept in memory for this request.
#[allow(clippy::result_large_err)]
async fn read_body(headers: &HeaderMap, body: Body, limit: usize, what: &str) -> Result<Vec<u8>, Response> {
    let too_large = || error(StatusCode::PAYLOAD_TOO_LARGE, "too_large", format!("{what} can be at most {} MB.", limit / (1024 * 1024)));
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|n| n > limit) {
        return Err(too_large());
    }
    let mut out = Vec::with_capacity(declared.unwrap_or(0).min(limit));
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| error(StatusCode::BAD_REQUEST, "invalid", "The upload was interrupted."))?;
        if out.len() + chunk.len() > limit {
            return Err(too_large());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[allow(clippy::result_large_err)]
async fn names_in(state: &AppState, space: &SpaceId) -> Result<Vec<String>, Response> {
    state.db.list_kwants(space).await.map(|ks| ks.into_iter().map(|k| k.name).collect()).map_err(internal)
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    filename: Option<String>,
}

#[derive(Deserialize)]
struct Pasted {
    text: String,
}

#[derive(Serialize)]
pub struct Preview {
    #[serde(flatten)]
    checked: Checked,
    /// True when nothing in `problems` stops the import.
    can_apply: bool,
}

/// `POST /api/import/preview`. A JSON body `{ "text": … }` (Content-Type `application/json`)
/// is pasted text; anything else is a file, named by `?filename=`.
pub async fn preview(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<PreviewQuery>, headers: HeaderMap, body: Body) -> Response {
    let bytes = match read_body(&headers, body, MAX_ZIP_BYTES, "A file").await {
        Ok(b) => b,
        Err(r) => return r,
    };
    if bytes.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty", "Nothing was sent.");
    }
    let is_json = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
    let parsed = match (is_json, serde_json::from_slice::<Pasted>(&bytes)) {
        (true, Ok(p)) => parsers::parse_pasted(&p.text),
        _ => parsers::parse_file(&bytes, q.filename.as_deref()),
    };
    drop(bytes);
    let bundle = match parsed {
        Ok(b) => b,
        Err(e) => return import_error(e),
    };
    let names = match names_in(&state, &space).await {
        Ok(n) => n,
        Err(r) => return r,
    };
    let checked = import::normalize(bundle, &names);
    let can_apply = checked.problems.is_empty() && !(checked.bundle.bots.is_empty() && checked.bundle.skills.is_empty());
    Json(Preview { checked, can_apply }).into_response()
}

#[derive(Deserialize)]
struct ApplyRequest {
    bundle: Bundle,
    #[serde(default)]
    skip: Vec<String>,
}

#[derive(Serialize)]
pub struct Applied {
    import: ImportRecord,
    warnings: Vec<import::Notice>,
    dropped: Vec<import::Notice>,
    renames: Vec<import::Rename>,
}

/// `POST /api/import/apply`. Bots named in `skip` (by `ref`) are left out, and teams lose them.
pub async fn apply(State(state): State<AppState>, Scope(space): Scope, headers: HeaderMap, body: Body) -> Response {
    let bytes = match read_body(&headers, body, MAX_APPLY_BYTES, "An import").await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let req: ApplyRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid", format!("This isn't an import from preview: {e}")),
    };
    let mut bundle = req.bundle;
    if bundle.format != BUNDLE_FORMAT {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid", format!("Send the bundle preview returned (format {BUNDLE_FORMAT})."));
    }
    let skip: HashSet<&str> = req.skip.iter().map(String::as_str).collect();
    bundle.bots.retain(|b| !skip.contains(b.ref_.as_str()));
    for t in &mut bundle.teams {
        t.members.retain(|m| !skip.contains(m.as_str()));
        if t.lead.as_deref().is_some_and(|l| skip.contains(l)) {
            t.lead = None;
        }
    }
    bundle.teams.retain(|t| !t.members.is_empty());
    let names = match names_in(&state, &space).await {
        Ok(n) => n,
        Err(r) => return r,
    };
    let checked = import::normalize(bundle, &names);
    if !checked.problems.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": checked.problems.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join(" "),
            "code": "problems",
            "problems": checked.problems,
        })))
            .into_response();
    }
    if checked.bundle.bots.is_empty() && checked.bundle.skills.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty", "There's nothing left to import.");
    }
    match state.db.apply_import(&space, &checked.bundle).await {
        Ok(import) => {
            tracing::info!(import_id = %import.id, space = %space, bots = import.created.kwants.len(), "import applied");
            (
                StatusCode::CREATED,
                Json(Applied { import, warnings: checked.warnings, dropped: checked.dropped, renames: checked.renames }),
            )
                .into_response()
        }
        Err(crate::db::DbError::Conflict(m)) => error(StatusCode::CONFLICT, "name_taken", m),
        Err(e) => internal(e),
    }
}

/// The import, if it is in `space` (404 otherwise, as for any row in another space).
#[allow(clippy::result_large_err)]
async fn import_in(state: &AppState, space: &SpaceId, id: &str) -> Result<ImportRecord, Response> {
    match state.db.get_import(id).await {
        Ok(Some(r)) if r.space_id == space.as_str() => Ok(r),
        Ok(_) => Err(error(StatusCode::NOT_FOUND, "not_found", "No such import in this space.")),
        Err(e) => Err(internal(e)),
    }
}

/// `GET /api/imports`: the space's imports, newest first.
pub async fn list(State(state): State<AppState>, Scope(space): Scope) -> Response {
    match state.db.list_imports(&space).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => internal(e),
    }
}

/// `GET /api/imports/:id`.
pub async fn get(State(state): State<AppState>, Scope(space): Scope, Path(id): Path<String>) -> Response {
    match import_in(&state, &space, &id).await {
        Ok(r) => Json(r).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize, Default)]
struct UndoRequest {
    #[serde(default)]
    force: bool,
}

/// `POST /api/imports/:id/undo`, optional body `{ "force": true }` to remove bots edited since.
pub async fn undo(State(state): State<AppState>, Scope(space): Scope, Path(id): Path<String>, headers: HeaderMap, body: Body) -> Response {
    if let Err(r) = import_in(&state, &space, &id).await {
        return r;
    }
    let bytes = match read_body(&headers, body, 4096, "An undo").await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let req: UndoRequest = if bytes.iter().all(u8::is_ascii_whitespace) || bytes.trim_ascii() == b"null" {
        UndoRequest::default()
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(_) => return error(StatusCode::UNPROCESSABLE_ENTITY, "invalid", "Send {\"force\": true} or nothing."),
        }
    };
    match state.db.undo_import(&id, req.force).await {
        Ok(r) => {
            tracing::info!(import_id = %id, space = %space, "import undone");
            Json(r).into_response()
        }
        Err(UndoError::NotFound) => error(StatusCode::NOT_FOUND, "not_found", "No such import in this space."),
        Err(UndoError::AlreadyUndone) => error(StatusCode::CONFLICT, "already_undone", "This import was already undone."),
        Err(UndoError::Edited(names)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("{} changed since the import. Undo anyway to remove them with your changes.", names.join(", ")),
                "code": "edited",
                "edited": names,
            })),
        )
            .into_response(),
        Err(UndoError::InUse(what)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("This import is in use, so it can't be undone: {}. Delete those bots one by one instead.", what.join("; ")),
                "code": "in_use",
                "in_use": what,
            })),
        )
            .into_response(),
        Err(UndoError::Db(e)) => internal(e),
    }
}

/// A download file name from a bot or space name: ASCII letters, digits, `-` and `_`, other
/// runs as one `-`; `fallback` when nothing is left.
pub fn file_stem(name: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() { fallback.to_string() } else { out.chars().take(80).collect() }
}

fn download(bundle: &Bundle, file: &str) -> Response {
    (
        [(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}.kwant.json\""))],
        Json(bundle),
    )
        .into_response()
}

/// `GET /api/kwants/:id/export` (the row guard checks `?space=`).
pub async fn export_kwant(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.export_kwant(&id).await {
        Ok(Some(b)) => {
            let name = b.bots.first().map(|k| k.name.clone()).unwrap_or_default();
            download(&b, &file_stem(&name, &id))
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found", "bot not found"),
        Err(e) => internal(e),
    }
}

/// `GET /api/export?space=`: every bot and team in the space.
pub async fn export_space(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let name = match state.db.get_space(&space).await {
        Ok(s) => s.map(|s| s.name).unwrap_or_default(),
        Err(e) => return internal(e),
    };
    match state.db.export_space(&space).await {
        Ok(b) => download(&b, &file_stem(&name, space.as_str())),
        Err(e) => internal(e),
    }
}

/// `GET /api/import/prompts`.
pub async fn prompts() -> Response {
    Json(import::prompts::export_prompts()).into_response()
}

#[cfg(test)]
mod tests {
    use super::file_stem;

    #[test]
    fn file_stems() {
        assert_eq!(file_stem("Default", "spc_x"), "Default");
        assert_eq!(file_stem("  My  Team / 2 ", "spc_x"), "My-Team-2");
        assert_eq!(file_stem("Café", "spc_x"), "Caf");
        assert_eq!(file_stem("★★", "spc_x"), "spc_x");
        assert_eq!(file_stem("", "spc_x"), "spc_x");
    }
}
