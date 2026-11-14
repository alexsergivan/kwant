//! A space's own documents (roadmap 10c): the FAQ, policies or catalogue a business keeps in its
//! space's `docs/` folder, which a bot can quote with the same word-for-word check kwant runs on
//! web pages ([`crate::sandbox::cite::cite_text`], unchanged).
//!
//! - `GET /api/spaces/:id/docs`: the documents (name, size, modified, pages for a PDF).
//! - `POST /api/spaces/:id/docs/cite` `{file, quote}`: the quote checked against that file, one
//!   [`FrameText`] per PDF page (`kwant-doc://<space>/<file>#page=N`), answered as the web cite
//!   answers (`{quote, url, link}`) or refused with cite.rs's own reason (422).
//! - `GET /api/spaces/:id/docs/page?file=&page=&quote=`: one page's text for the viewer, split
//!   around the quote where cite would tick it ([`page_view`]).
//! - `POST /api/spaces/:id/docs?name=&replace=1` (the file as the raw body) and
//!   `DELETE /api/spaces/:id/docs/:name`: the person adds and removes documents. Both are
//!   person-only (`auth::person_only`): a bot must never write the words it then quotes.
//!
//! The folder is `<space's data folder>/docs` ([`docs_dir`]): one per space, outside every bot's
//! work folder, so a bot working in its folder can't write the words it then cites. Only the
//! folder's own files are read (no subfolders), each opened from the folder's descriptor without
//! following a link ([`safe_fs::Dir`]); a `docs` that is itself a link is refused. Text, Markdown
//! and PDF (its text layer, the reader attachments use: no OCR, no model call), 2 MB a file, 200
//! files a space. Only [`upload`] and [`remove`] write: one at a time, a new file through a hidden
//! temporary file in the folder and a rename, so a reader never sees half a document.
#![allow(clippy::result_large_err)]

use super::AppState;
use crate::db::spaces::SpaceId;
use crate::sandbox::cite::{self, FrameText};
use crate::sandbox::safe_fs::{self, Dir, Kind};
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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// The largest document kwant reads.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// The most documents a space lists.
pub const MAX_FILES: usize = 200;
/// Names read from the folder at most, documents or not: a folder of millions costs no more.
const MAX_SCAN: usize = 2_000;
/// Counting a PDF's pages for the list is given this long.
const PAGE_COUNT_SECS: u64 = 5;
/// The kinds of file read, by extension (any case).
const KINDS: [&str; 4] = ["md", "markdown", "txt", "pdf"];
/// A new document's name is at most this many bytes (file systems stop at 255).
const MAX_NAME_BYTES: usize = 200;
/// Characters of page text the viewer gets on each side of the quote (or after the page start).
pub const PAGE_SIDE_CHARS: usize = 20_000;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// A refusal with a `code` the UI can tell apart: `{error, code}`.
fn refuse(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into(), "code": code }))).into_response()
}

fn docs_link() -> Response {
    refuse(StatusCode::CONFLICT, "docs_link", "the space's docs isn't a plain folder (a link or a file); kwant only reads and writes a real docs folder")
}

/// Where a space's documents live: `docs` in the space's own data folder (the data folder itself
/// for Default).
pub fn docs_dir(data_dir: &std::path::Path, space: &SpaceId) -> PathBuf {
    crate::db::spaces::space_data_dir(data_dir, space).join("docs")
}

/// Why `name` can't name a document: it must be a plain file name in the folder (no separator,
/// no `..`, no leading dot) of a kind kwant reads.
pub fn bad_name(name: &str) -> Option<&'static str> {
    plain_name_problem(name).or_else(|| kind_of(name).is_none().then_some(WRONG_KIND))
}

const WRONG_KIND: &str = "only .md, .markdown, .txt and .pdf documents can be read";

/// Why `name` isn't a plain file name in the folder (any kind).
fn plain_name_problem(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("file is required: a document's name, as the list gives it")
    } else if name.contains(['/', '\\', '\0']) || name.contains("..") || name.starts_with('.') || name != name.trim() {
        Some("file must be a document's own name in the space's docs folder (no folders, no ..)")
    } else {
        None
    }
}

/// Why a new document can't be called `name`: 400 `bad_name` (not a plain one-line name, or too
/// long), 415 `wrong_kind` (a kind kwant doesn't read).
pub fn upload_name_problem(name: &str) -> Option<(StatusCode, &'static str, &'static str)> {
    if let Some(why) = plain_name_problem(name) {
        Some((StatusCode::BAD_REQUEST, "bad_name", why))
    } else if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        Some((StatusCode::BAD_REQUEST, "bad_name", "a document's name is one line of at most 200 bytes"))
    } else if kind_of(name).is_none() {
        Some((StatusCode::UNSUPPORTED_MEDIA_TYPE, "wrong_kind", WRONG_KIND))
    } else {
        None
    }
}

fn kind_of(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    KINDS.iter().copied().find(|k| *k == ext)
}

fn is_pdf(name: &str) -> bool {
    kind_of(name) == Some("pdf")
}

/// Percent-encodes one part of a `kwant-doc://` address: everything but ASCII letters, digits
/// and `-._~`.
fn encode(part: &str) -> String {
    let mut out = String::new();
    for b in part.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The address a cited quote points at: the document, and for a PDF the page.
pub fn doc_url(space: &SpaceId, file: &str, page: Option<usize>) -> String {
    let base = format!("kwant-doc://{}/{}", encode(space.as_str()), encode(file));
    match page {
        Some(n) => format!("{base}#page={n}"),
        None => base,
    }
}

/// What a document is cited from: its text, a frame per PDF page (numbered from 1).
pub fn frames(space: &SpaceId, file: &str, pages: &[String]) -> Vec<FrameText> {
    if is_pdf(file) {
        pages.iter().enumerate().map(|(i, t)| FrameText { url: doc_url(space, file, Some(i + 1)), text: t.clone() }).collect()
    } else {
        pages.iter().map(|t| FrameText { url: doc_url(space, file, None), text: t.clone() }).collect()
    }
}

/// The docs folder, opened once and never through a link: `None` when the space has none.
fn open_docs(dir: &std::path::Path) -> Result<Option<Dir>, Response> {
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
        return Ok(None);
    };
    let root = match Dir::open(parent) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "couldn't open the space's folder")),
    };
    match root.open_dir(std::path::Path::new(name)) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(error(
            StatusCode::CONFLICT,
            "the space's docs isn't a plain folder (a link or a file); kwant only reads a real docs folder",
        )),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocInfo {
    pub name: String,
    pub size: u64,
    /// When the file last changed, RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    /// A PDF's page count, when it could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<usize>,
    /// Over [`MAX_FILE_BYTES`]: listed, but never read or cited.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub too_large: bool,
}

/// The documents' names and sizes as listed (`lstat`), sorted, at most [`MAX_FILES`]; the flag
/// says some were left out. Nothing is opened.
fn doc_names(docs: &Dir) -> Result<(Vec<(String, u64)>, bool), Response> {
    let (entries, cut) = docs
        .list_at_most(MAX_SCAN)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "couldn't list the space's docs folder"))?;
    let mut names: Vec<(String, u64)> = entries
        .into_iter()
        .filter(|e| e.kind == Kind::File)
        .filter_map(|e| Some((e.name.into_string().ok()?, e.len)))
        .filter(|(n, _)| bad_name(n).is_none())
        .collect();
    names.sort();
    let more = cut || names.len() > MAX_FILES;
    names.truncate(MAX_FILES);
    Ok((names, more))
}

/// A PDF's page count from its structure (no text extraction), off the runtime and timed out:
/// `None` for one that can't be parsed.
async fn pdf_page_count(bytes: Vec<u8>) -> Option<usize> {
    let job = tokio::task::spawn_blocking(move || pdf_extract::Document::load_mem(&bytes).ok().map(|d| d.get_pages().len()));
    tokio::time::timeout(std::time::Duration::from_secs(PAGE_COUNT_SECS), job).await.ok()?.ok()?
}

/// Page counts already read, by (folder, name, size, modified) (QA F4): a list the UI asks for
/// again parses only the PDFs that changed. Cleared when it passes a few spaces' worth.
type PageKey = (PathBuf, String, u64, SystemTime);
fn page_counts() -> &'static Mutex<HashMap<PageKey, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<PageKey, usize>>> = OnceLock::new();
    COUNTS.get_or_init(Default::default)
}
const PAGE_COUNTS_KEPT: usize = 4 * MAX_FILES;

/// One listed document, opened again from the folder's descriptor: `None` when it is gone or
/// was swapped for a link since the folder was listed.
async fn doc_info(dir: &std::path::Path, docs: &Dir, name: String, len: u64) -> Option<DocInfo> {
    let file = docs.open_file(std::path::Path::new(&name)).ok()?;
    let meta = file.metadata().ok();
    let size = meta.as_ref().map_or(len, |m| m.len());
    let mtime = meta.and_then(|m| m.modified().ok());
    let modified = mtime.map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
    let too_large = size > MAX_FILE_BYTES;
    let pages = if is_pdf(&name) && !too_large {
        let key = mtime.map(|t| (dir.to_path_buf(), name.clone(), size, t));
        let known = key
            .as_ref()
            .and_then(|k| page_counts().lock().ok()?.get(k).copied());
        match known {
            Some(n) => Some(n),
            None => {
                let n = match safe_fs::read_capped(file, MAX_FILE_BYTES) {
                    Ok(bytes) => pdf_page_count(bytes).await,
                    Err(_) => None,
                };
                if let (Some(n), Some(k), Ok(mut m)) = (n, key, page_counts().lock()) {
                    if m.len() >= PAGE_COUNTS_KEPT {
                        m.clear();
                    }
                    m.insert(k, n);
                }
                n
            }
        }
    } else {
        None
    };
    Some(DocInfo {
        name,
        size,
        modified,
        pages,
        too_large,
    })
}

/// The space's documents, sorted by name, at most [`MAX_FILES`]; the flag says some were left out.
/// Links, folders and other kinds of file are skipped.
pub async fn list_docs(dir: &std::path::Path) -> Result<(Vec<DocInfo>, bool), Response> {
    let Some(docs) = open_docs(dir)? else {
        return Ok((Vec::new(), false));
    };
    let (names, more) = doc_names(&docs)?;
    let mut out = Vec::with_capacity(names.len());
    for (name, len) in names {
        out.extend(doc_info(dir, &docs, name, len).await);
    }
    Ok((out, more))
}

/// A document's text, one string per PDF page (one string for text and Markdown).
pub async fn read_doc(dir: &std::path::Path, file: &str) -> Result<Vec<String>, Response> {
    if let Some(why) = bad_name(file) {
        return Err(error(StatusCode::BAD_REQUEST, why));
    }
    let Some(docs) = open_docs(dir)? else {
        return Err(error(StatusCode::NOT_FOUND, "this space has no documents (no docs folder)"));
    };
    let handle = match docs.open_file(std::path::Path::new(file)) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(error(StatusCode::NOT_FOUND, format!("no document named {file} in this space")))
        }
        Err(_) => return Err(error(StatusCode::BAD_REQUEST, format!("{file} isn't a plain file in the space's docs folder"))),
    };
    let bytes = match safe_fs::read_capped(handle, MAX_FILE_BYTES) {
        Ok(b) => b,
        Err(e) if safe_fs::is_too_large(&e) => {
            return Err(error(StatusCode::PAYLOAD_TOO_LARGE, format!("{file} is over 2 MB; kwant reads documents up to 2 MB")))
        }
        Err(_) => return Err(error(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't read {file}"))),
    };
    if is_pdf(file) {
        let pages = super::attachments::pdf_pages(bytes)
            .await
            .map_err(|why| error(StatusCode::UNPROCESSABLE_ENTITY, format!("Couldn't read {file}: {why}.")))?;
        if pages.iter().all(|p| p.trim().is_empty()) {
            return Err(error(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{file} has no text layer (probably scanned pages), so there is nothing to check a quote against."),
            ));
        }
        return Ok(pages);
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| error(StatusCode::UNPROCESSABLE_ENTITY, format!("{file} isn't UTF-8 text")))?;
    Ok(vec![text.trim_start_matches('\u{feff}').to_string()])
}

/// `GET /api/spaces/:id/docs`: `{folder, docs: [{name, size, modified, pages?, too_large?}], more}`.
/// A space with no docs folder lists nothing.
pub async fn list(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let space = match super::spaces::active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let dir = docs_dir(&state.config.data_dir, &space);
    match list_docs(&dir).await {
        Ok((docs, more)) => Json(json!({ "folder": dir, "docs": docs, "more": more })).into_response(),
        Err(r) => r,
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct CiteDocPayload {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub quote: String,
}

/// `POST /api/spaces/:id/docs/cite` `{file, quote}`: `{quote, url, link}` when the words are in
/// the document word for word and once; 422 with the reason when they aren't. No model is asked.
pub async fn cite(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<CiteDocPayload>) -> Response {
    let space = match super::spaces::active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let file = body.file.trim();
    let pages = match read_doc(&docs_dir(&state.config.data_dir, &space), file).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match cite::cite_text(&frames(&space, file, &pages), &body.quote) {
        Ok(c) => Json(json!(c)).into_response(),
        Err(why) => error(StatusCode::UNPROCESSABLE_ENTITY, why),
    }
}

// ── Adding, removing and viewing (the person's Documents tab) ──────────────────────────

/// Writes to docs folders one at a time, so "already there", the 200 limit and the rename are
/// checked together and two uploads of one name can't both win.
fn writes() -> &'static tokio::sync::Mutex<()> {
    static WRITES: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    WRITES.get_or_init(Default::default)
}

/// The docs folder to write in, made (with the space's folder) when missing; never through a link.
fn docs_for_write(data_dir: &std::path::Path, space: &SpaceId) -> Result<Dir, Response> {
    let dir = docs_dir(data_dir, space);
    match open_docs(&dir) {
        Ok(Some(d)) => return Ok(d),
        Ok(None) => {}
        Err(r) if r.status() == StatusCode::CONFLICT => return Err(docs_link()),
        Err(r) => return Err(r),
    }
    let made = dir
        .strip_prefix(data_dir)
        .ok()
        .and_then(|rel| Dir::open(data_dir).ok()?.make_dirs(rel).ok());
    made.ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "couldn't make the space's docs folder",
        )
    })
}

/// What the folder already has under a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Existing {
    Nothing,
    File,
    /// A link, a folder or anything else that isn't a regular file
    Other,
}

fn existing(docs: &Dir, name: &str) -> Existing {
    match docs.open_file(std::path::Path::new(name)) {
        Ok(_) => Existing::File,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Existing::Nothing,
        Err(_) => Existing::Other,
    }
}

/// Writes `bytes` as `name`: into a new hidden file in the same folder (`.upload-<id>.part`, never
/// listed), then renamed over `name`, so a reader sees the old document or the whole new one. A
/// link at `name` is replaced, never written through.
fn write_doc(docs: &Dir, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = format!(".upload-{}.part", uuid::Uuid::new_v4().simple());
    docs.replace_file(std::ffi::OsStr::new(name), &tmp, &mut &bytes[..], 0o644)
}

/// Refuses bytes kwant couldn't read back as a document of `name`'s kind: text must be UTF-8, and
/// a PDF must have a text layer (the same reader as cite, so what is added can be quoted).
async fn check_content(name: &str, bytes: &[u8]) -> Result<(), Response> {
    if is_pdf(name) {
        let pages = super::attachments::pdf_pages(bytes.to_vec())
            .await
            .map_err(|why| {
                refuse(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "unreadable",
                    format!("Couldn't read {name}: {why}."),
                )
            })?;
        if pages.iter().all(|p| p.trim().is_empty()) {
            return Err(refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "no_text",
                format!("PDF has no text layer: {name} looks like scanned pages, so there are no words a bot could quote. kwant doesn't read pictures of text."),
            ));
        }
    } else if std::str::from_utf8(bytes).is_err() {
        return Err(refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unreadable",
            format!("{name} isn't UTF-8 text"),
        ));
    }
    Ok(())
}

fn too_large() -> Response {
    refuse(
        StatusCode::PAYLOAD_TOO_LARGE,
        "too_large",
        "kwant reads documents up to 2 MB",
    )
}

/// The request body, refused (413) past [`MAX_FILE_BYTES`]: by its declared length before any
/// byte, else at the byte that passes it. The rest of a refused body is drained in the background.
async fn read_body(headers: &HeaderMap, body: Body) -> Result<Vec<u8>, Response> {
    let mut stream = body.into_data_stream();
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_FILE_BYTES) {
        super::attachments::drain(stream);
        return Err(too_large());
    }
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "interrupted",
                "The upload was interrupted.",
            ));
        };
        if (out.len() + chunk.len()) as u64 > MAX_FILE_BYTES {
            super::attachments::drain(stream);
            return Err(too_large());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn yes(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1" | "true" | "yes"))
}

#[derive(Debug, Default, Deserialize)]
pub struct UploadQuery {
    #[serde(default)]
    pub name: String,
    pub replace: Option<String>,
}

/// `POST /api/spaces/:id/docs?name=<file name>[&replace=1]` with the file as the raw body (as
/// `/api/attachments` takes it): 201 with its list entry. 400 `bad_name`, 415 `wrong_kind`, 413
/// `too_large`, 409 `exists` (without `replace=1`), `docs_link` or `full` (200 documents), 422
/// `no_text` (a PDF without a text layer) or `unreadable`. Person-only (`auth::person_only`).
pub async fn upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let space = match super::spaces::active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => {
            super::attachments::drain(body.into_data_stream());
            return r;
        }
    };
    let name = q.name.as_str();
    if let Some((status, code, why)) = upload_name_problem(name) {
        super::attachments::drain(body.into_data_stream());
        return refuse(status, code, why);
    }
    let bytes = match read_body(&headers, body).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    if let Err(r) = check_content(name, &bytes).await {
        return r;
    }
    let _one = writes().lock().await;
    let dir = docs_dir(&state.config.data_dir, &space);
    let docs = match docs_for_write(&state.config.data_dir, &space) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let there = existing(&docs, name);
    if there != Existing::Nothing && !yes(q.replace.as_deref()) {
        return refuse(
            StatusCode::CONFLICT,
            "exists",
            format!("{name} is already in this space's documents; add it with replace to swap it"),
        );
    }
    if there == Existing::Nothing {
        let (names, more) = match doc_names(&docs) {
            Ok(n) => n,
            Err(r) => return r,
        };
        if more || names.len() >= MAX_FILES {
            return refuse(
                StatusCode::CONFLICT,
                "full",
                "this space has 200 documents, the most kwant reads; remove one first",
            );
        }
    }
    if let Err(e) = write_doc(&docs, name, &bytes) {
        tracing::warn!(error = %e, "space document not written");
        return match there {
            Existing::Other => refuse(StatusCode::CONFLICT, "exists", format!("{name} is a folder or something else that isn't a document; kwant won't replace it")),
            _ => error(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't write {name}")),
        };
    }
    tracing::info!(space = %space.as_str(), size = bytes.len(), replaced = there != Existing::Nothing, "space document added");
    match doc_info(&dir, &docs, name.to_string(), bytes.len() as u64).await {
        Some(info) => (StatusCode::CREATED, Json(info)).into_response(),
        None => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{name} was written but couldn't be read back"),
        ),
    }
}

/// `DELETE /api/spaces/:id/docs/:name`: 204, 404 `not_found`, 400 `bad_name` (not a document's
/// own name, or not a plain file there: a link is left alone), 409 `docs_link`. Person-only.
pub async fn remove(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> Response {
    let space = match super::spaces::active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Some(why) = bad_name(&name) {
        return refuse(StatusCode::BAD_REQUEST, "bad_name", why);
    }
    let _one = writes().lock().await;
    let docs = match open_docs(&docs_dir(&state.config.data_dir, &space)) {
        Ok(Some(d)) => d,
        Ok(None) => {
            return refuse(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("no document named {name} in this space"),
            )
        }
        Err(r) if r.status() == StatusCode::CONFLICT => return docs_link(),
        Err(r) => return r,
    };
    match existing(&docs, &name) {
        Existing::Nothing => refuse(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("no document named {name} in this space"),
        ),
        Existing::Other => refuse(
            StatusCode::BAD_REQUEST,
            "bad_name",
            format!("{name} isn't a plain file in the space's docs folder"),
        ),
        Existing::File => match docs.remove_file(std::ffi::OsStr::new(&name)) {
            Ok(()) => {
                tracing::info!(space = %space.as_str(), "space document removed");
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => refuse(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("no document named {name} in this space"),
            ),
            Err(_) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("couldn't remove {name}"),
            ),
        },
    }
}

/// One page of a document for the viewer, split around the quote.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PageView {
    pub file: String,
    pub page: usize,
    pub pages: usize,
    pub text_before: String,
    #[serde(rename = "match")]
    pub matched: String,
    pub text_after: String,
    pub cut_before: bool,
    pub cut_after: bool,
}

/// The last `n` characters of `s`, and whether some were cut.
fn keep_last(s: &str, n: usize) -> (String, bool) {
    match s.char_indices().rev().nth(n.saturating_sub(1)) {
        Some((i, _)) if n > 0 && i > 0 => (s[i..].to_string(), true),
        _ if n == 0 => (String::new(), !s.is_empty()),
        _ => (s.to_string(), false),
    }
}

/// The first `n` characters of `s`, and whether some were cut.
fn keep_first(s: &str, n: usize) -> (String, bool) {
    match s.char_indices().nth(n) {
        Some((i, _)) => (s[..i].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// Page `page` (from 1) of a document read as `pages`: `None` when there is no such page. With a
/// quote that cite would tick on this very page (word for word, once in the document, whole
/// words: [`cite::cite_text`], then [`cite::locate`] for where), the text is split around it;
/// otherwise `matched` is empty and `text_after` is the page. Each side keeps at most
/// [`PAGE_SIDE_CHARS`] characters, nearest the match.
pub fn page_view(
    space: &SpaceId,
    file: &str,
    pages: &[String],
    page: usize,
    quote: &str,
) -> Option<PageView> {
    let text = pages.get(page.checked_sub(1)?)?;
    let here = doc_url(space, file, is_pdf(file).then_some(page));
    let hit = if quote.trim().is_empty() {
        None
    } else {
        cite::cite_text(&frames(space, file, pages), quote)
            .ok()
            .filter(|c| c.url == here)
            .and_then(|_| cite::locate(text, quote))
    };
    let (before, matched, after) = match hit {
        Some(r) => (&text[..r.start], &text[r.clone()], &text[r.end..]),
        None => ("", "", text.as_str()),
    };
    let (text_before, cut_before) = keep_last(before, PAGE_SIDE_CHARS);
    let (text_after, cut_after) = keep_first(after, PAGE_SIDE_CHARS);
    Some(PageView {
        file: file.to_string(),
        page,
        pages: pages.len(),
        text_before,
        matched: matched.to_string(),
        text_after,
        cut_before,
        cut_after,
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub file: String,
    pub page: Option<usize>,
    #[serde(default)]
    pub quote: String,
}

/// `GET /api/spaces/:id/docs/page?file=&page=&quote=`: [`PageView`] (page 1 by default). Errors
/// as cite's: 400 bad name, 404 no such file (or page), 409 docs is a link, 413, 422. Open to
/// bots like the list and cite: it only reads.
pub async fn page(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PageQuery>,
) -> Response {
    let space = match super::spaces::active_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let file = q.file.trim();
    let pages = match read_doc(&docs_dir(&state.config.data_dir, &space), file).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let n = q.page.unwrap_or(1);
    match page_view(&space, file, &pages, n, &q.quote) {
        Some(v) => Json(v).into_response(),
        None => refuse(
            StatusCode::NOT_FOUND,
            "no_page",
            format!("{file} has no page {n} (it has {})", pages.len()),
        ),
    }
}

/// The lines a bot's turn gets when its space has documents: where they are listed and the call
/// that cites one. Empty when the space has none, so other turns' prompts don't change.
pub async fn prompt_block(state: &AppState, space: &SpaceId) -> String {
    // Names only: a turn never opens or parses the documents
    let has_docs = match open_docs(&docs_dir(&state.config.data_dir, space)) {
        Ok(Some(docs)) => doc_names(&docs).is_ok_and(|(names, _)| names.iter().any(|(_, len)| *len <= MAX_FILE_BYTES)),
        _ => false,
    };
    if !has_docs {
        return String::new();
    }
    prompt_lines(&format!("http://127.0.0.1:{}/api/spaces/{}/docs", state.config.port, encode(space.as_str())))
}

/// [`prompt_block`]'s text for the docs address `api`.
pub fn prompt_lines(api: &str) -> String {
    format!(
        "\n\n[Space documents]\n\
        This space keeps its own documents (FAQ, policies, prices, catalogue): list them with curl -sS '{api}', and quote one with \
        curl -sS -X POST '{api}/cite' -H 'Content-Type: application/json' -d '{{\"file\":\"<name from the list>\",\"quote\":\"<the exact words>\"}}', \
        which checks the words are in that file word for word and once and returns quote, url and link; when you answer from the \
        space's own papers, back each fact with it, and if it refuses, quote the file exactly or quote more, never from memory."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_PAGES: &[u8] = include_bytes!("../../tests/fixtures/two-pages.pdf");

    fn space() -> SpaceId {
        SpaceId::new("shop")
    }

    #[test]
    fn names_must_be_plain_files_of_a_kind_kwant_reads() {
        for (name, ok) in [
            ("faq.md", true),
            ("Returns Policy.PDF", true),
            ("prices.txt", true),
            ("notes.markdown", true),
            ("", false),
            ("../secret.md", false),
            ("..md", false),
            ("sub/faq.md", false),
            ("sub\\faq.md", false),
            (".hidden.md", false),
            (" faq.md", false),
            ("photo.png", false),
            ("faq", false),
            ("a\0b.md", false),
        ] {
            assert_eq!(bad_name(name).is_none(), ok, "{name:?}");
        }
    }

    #[test]
    fn addresses_name_the_space_the_file_and_a_pdf_page() {
        assert_eq!(doc_url(&space(), "faq.md", None), "kwant-doc://shop/faq.md");
        assert_eq!(doc_url(&space(), "Returns Policy.pdf", Some(2)), "kwant-doc://shop/Returns%20Policy.pdf#page=2");
        let f = frames(&space(), "a.pdf", &["one".into(), "two".into()]);
        assert_eq!(f.iter().map(|f| f.url.as_str()).collect::<Vec<_>>(), ["kwant-doc://shop/a.pdf#page=1", "kwant-doc://shop/a.pdf#page=2"]);
        assert_eq!(frames(&space(), "a.md", &["x".into()])[0].url, "kwant-doc://shop/a.md");
    }

    fn docs_folder() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("docs");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("faq.md"), "# Returns\n\nYou can return any item within 30 days of delivery.\nShipping is free over 50 euros.\n").unwrap();
        std::fs::write(dir.join("plan.pdf"), TWO_PAGES).unwrap();
        (tmp, dir)
    }

    async fn cite_in(dir: &std::path::Path, file: &str, quote: &str) -> Result<cite::Citation, (StatusCode, String)> {
        let pages = read_doc(dir, file).await.map_err(|r| (r.status(), String::new()))?;
        cite::cite_text(&frames(&space(), file, &pages), quote).map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e))
    }

    #[tokio::test]
    async fn quotes_are_cited_only_when_the_file_has_them_word_for_word() {
        let (_tmp, dir) = docs_folder();
        let ok = cite_in(&dir, "faq.md", "return any item within 30 days").await.unwrap();
        assert_eq!(ok.url, "kwant-doc://shop/faq.md");
        assert_eq!(ok.quote, "return any item within 30 days");
        assert!(ok.link.starts_with("kwant-doc://shop/faq.md#:~:text="), "{}", ok.link);
        // (file, quote, refusal it must give: cite.rs's own words, or the HTTP status)
        for (file, quote, want) in [
            ("faq.md", "return any item within 60 days", "aren't on the page as given"),
            ("faq.md", "Return any item", "other capitals or accents"),
            ("faq.md", "", "Give the exact words"),
            ("faq.md", "eturn any item", "middle of a word"),
        ] {
            let (status, why) = cite_in(&dir, file, quote).await.unwrap_err();
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{quote:?}");
            assert!(why.contains(want), "{quote:?}: {why}");
        }
        std::fs::write(dir.join("twice.txt"), "Open daily.\nOpen daily.\n").unwrap();
        let (_, why) = cite_in(&dir, "twice.txt", "Open daily.").await.unwrap_err();
        assert!(why.contains("2 times"), "{why}");
    }

    #[tokio::test]
    async fn a_quote_on_pdf_page_two_points_at_page_two() {
        let (_tmp, dir) = docs_folder();
        let c = cite_in(&dir, "plan.pdf", "Budget is 42 euros").await.unwrap();
        assert_eq!(c.url, "kwant-doc://shop/plan.pdf#page=2");
        assert!(c.link.starts_with("kwant-doc://shop/plan.pdf#page=2:~:text="), "{}", c.link);
        assert_eq!(cite_in(&dir, "plan.pdf", "PAGE_ONE_MARKER").await.unwrap().url, "kwant-doc://shop/plan.pdf#page=1");
    }

    #[tokio::test]
    async fn files_outside_the_folder_or_not_plain_files_are_refused() {
        let (tmp, dir) = docs_folder();
        std::fs::write(tmp.path().join("secret.md"), "the secret words").unwrap();
        for (file, status) in [
            ("../secret.md", StatusCode::BAD_REQUEST),
            ("missing.md", StatusCode::NOT_FOUND),
            ("x.png", StatusCode::BAD_REQUEST),
        ] {
            assert_eq!(cite_in(&dir, file, "the secret words").await.unwrap_err().0, status, "{file}");
        }
        std::fs::write(dir.join("big.txt"), vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert_eq!(cite_in(&dir, "big.txt", "a").await.unwrap_err().0, StatusCode::PAYLOAD_TOO_LARGE);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join("secret.md"), dir.join("link.md")).unwrap();
            assert_eq!(cite_in(&dir, "link.md", "the secret words").await.unwrap_err().0, StatusCode::BAD_REQUEST);
            let names: Vec<String> = list_docs(&dir).await.unwrap().0.into_iter().map(|d| d.name).collect();
            assert!(!names.contains(&"link.md".to_string()), "{names:?}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_docs_folder_that_is_a_link_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("elsewhere")).unwrap();
        std::fs::write(tmp.path().join("elsewhere/faq.md"), "words").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("elsewhere"), tmp.path().join("docs")).unwrap();
        assert_eq!(list_docs(&tmp.path().join("docs")).await.unwrap_err().status(), StatusCode::CONFLICT);
        assert_eq!(read_doc(&tmp.path().join("docs"), "faq.md").await.unwrap_err().status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn the_list_has_documents_only_with_pdf_pages_and_no_folder_lists_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(list_docs(&tmp.path().join("docs")).await.unwrap(), (Vec::new(), false));
        assert_eq!(list_docs(&tmp.path().join("no-space/docs")).await.unwrap(), (Vec::new(), false));
        let (_tmp, dir) = docs_folder();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("photo.png"), "x").unwrap();
        std::fs::write(dir.join(".hidden.md"), "x").unwrap();
        let (docs, more) = list_docs(&dir).await.unwrap();
        assert!(!more);
        assert_eq!(docs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["faq.md", "plan.pdf"]);
        assert_eq!(docs[1].pages, Some(2));
        assert_eq!(docs[0].pages, None);
        assert_eq!(docs[1].size, TWO_PAGES.len() as u64);
        assert!(docs.iter().all(|d| d.modified.is_some() && !d.too_large));
    }

    #[tokio::test]
    async fn a_space_lists_at_most_two_hundred() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("docs");
        std::fs::create_dir(&dir).unwrap();
        for i in 0..MAX_FILES + 3 {
            std::fs::write(dir.join(format!("{i:03}.txt")), "x").unwrap();
        }
        let (docs, more) = list_docs(&dir).await.unwrap();
        assert_eq!((docs.len(), more), (MAX_FILES, true));
    }

    #[test]
    fn the_prompt_names_both_calls() {
        let p = prompt_lines("http://127.0.0.1:8787/api/spaces/shop/docs");
        assert!(p.contains("curl -sS 'http://127.0.0.1:8787/api/spaces/shop/docs'"), "{p}");
        assert!(p.contains("'http://127.0.0.1:8787/api/spaces/shop/docs/cite'"), "{p}");
        assert!(p.contains("never from memory"), "{p}");
    }

    #[test]
    fn new_names_are_plain_one_line_names_of_a_kind_kwant_reads() {
        let long = format!("{}.md", "a".repeat(MAX_NAME_BYTES));
        for (name, want) in [
            ("faq.md", None),
            ("Returns Policy.PDF", None),
            ("../secret.md", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("sub/faq.md", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("sub\\faq.md", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            (".hidden.md", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("..", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("faq.md ", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("a\nb.md", Some((StatusCode::BAD_REQUEST, "bad_name"))),
            (long.as_str(), Some((StatusCode::BAD_REQUEST, "bad_name"))),
            ("photo.png", Some((StatusCode::UNSUPPORTED_MEDIA_TYPE, "wrong_kind"))),
            ("faq", Some((StatusCode::UNSUPPORTED_MEDIA_TYPE, "wrong_kind"))),
        ] {
            assert_eq!(upload_name_problem(name).map(|(s, c, _)| (s, c)), want, "{name:?}");
        }
    }

    #[test]
    fn the_page_view_splits_around_a_ticked_quote_and_cuts_long_sides() {
        let s = space();
        let page = "Intro.\n\nYou can return any item within 30 days.\nShipping is free.".to_string();
        let v = page_view(&s, "faq.md", std::slice::from_ref(&page), 1, "return any item").unwrap();
        assert_eq!((v.text_before.as_str(), v.matched.as_str(), v.text_after.as_str()), ("Intro.\n\nYou can ", "return any item", " within 30 days.\nShipping is free."));
        assert_eq!((v.page, v.pages, v.cut_before, v.cut_after), (1, 1, false, false));
        // No quote, a quote not there, or one cite refuses: the whole page after, nothing matched
        for q in ["", "return any item within 60 days", "eturn"] {
            let v = page_view(&s, "faq.md", std::slice::from_ref(&page), 1, q).unwrap();
            assert_eq!((v.text_before.as_str(), v.matched.as_str(), v.text_after.as_str()), ("", "", page.as_str()), "{q:?}");
        }
        assert_eq!(page_view(&s, "faq.md", std::slice::from_ref(&page), 2, ""), None);
        assert_eq!(page_view(&s, "faq.md", std::slice::from_ref(&page), 0, ""), None);
        // A PDF: the quote is matched on its own page only
        let pdf = vec!["Page one words.".to_string(), "Budget is 42 euros.".to_string()];
        assert_eq!(page_view(&s, "p.pdf", &pdf, 2, "Budget is 42 euros").unwrap().matched, "Budget is 42 euros");
        assert_eq!(page_view(&s, "p.pdf", &pdf, 1, "Budget is 42 euros").unwrap().matched, "");
        // On two pages: cite refuses, so nothing is highlighted
        let twice = vec!["Open daily.".to_string(), "Open daily.".to_string()];
        assert_eq!(page_view(&s, "p.pdf", &twice, 1, "Open daily.").unwrap().matched, "");
        // Long sides keep the characters nearest the match (multi-byte text cut on a character)
        let long = format!("{} MARKER {}", "é".repeat(PAGE_SIDE_CHARS + 5), "ü".repeat(PAGE_SIDE_CHARS + 7));
        let v = page_view(&s, "big.txt", std::slice::from_ref(&long), 1, "MARKER").unwrap();
        assert_eq!((v.text_before.chars().count(), v.text_after.chars().count()), (PAGE_SIDE_CHARS, PAGE_SIDE_CHARS));
        assert_eq!((v.cut_before, v.matched.as_str(), v.cut_after), (true, "MARKER", true));
        let v = page_view(&s, "big.txt", std::slice::from_ref(&long), 1, "").unwrap();
        assert_eq!((v.text_before.as_str(), v.cut_before, v.text_after.chars().count(), v.cut_after), ("", false, PAGE_SIDE_CHARS, true));
        let exact = "x".repeat(PAGE_SIDE_CHARS);
        assert_eq!(keep_last(&exact, PAGE_SIDE_CHARS), (exact.clone(), false));
        assert_eq!(keep_first(&exact, PAGE_SIDE_CHARS), (exact.clone(), false));
    }

    #[test]
    fn a_write_goes_through_a_hidden_temporary_file_and_replaces_a_link_instead_of_following_it() {
        let (tmp, dir) = docs_folder();
        let docs = Dir::open(&dir).unwrap();
        write_doc(&docs, "new.md", b"fresh words").unwrap();
        assert_eq!(std::fs::read(dir.join("new.md")).unwrap(), b"fresh words");
        write_doc(&docs, "new.md", b"second words").unwrap();
        assert_eq!(std::fs::read(dir.join("new.md")).unwrap(), b"second words");
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).filter(|n| n.starts_with('.')).collect();
        assert!(left.is_empty(), "{left:?}");
        // The temporary name is never listed or readable as a document
        assert!(bad_name(".upload-0123.part").is_some());
        #[cfg(unix)]
        {
            std::fs::write(tmp.path().join("secret.md"), "outside").unwrap();
            std::os::unix::fs::symlink(tmp.path().join("secret.md"), dir.join("link.md")).unwrap();
            assert_eq!(existing(&docs, "link.md"), Existing::Other);
            write_doc(&docs, "link.md", b"inside").unwrap();
            assert_eq!(std::fs::read_to_string(tmp.path().join("secret.md")).unwrap(), "outside");
            assert!(std::fs::symlink_metadata(dir.join("link.md")).unwrap().file_type().is_file());
        }
        assert_eq!(existing(&docs, "faq.md"), Existing::File);
        assert_eq!(existing(&docs, "nope.md"), Existing::Nothing);
    }

    #[tokio::test]
    async fn the_list_reuses_page_counts_until_the_pdf_changes() {
        let (_tmp, dir) = docs_folder();
        let key_for = |dir: &std::path::Path| {
            let m = std::fs::metadata(dir.join("plan.pdf")).unwrap();
            (dir.to_path_buf(), "plan.pdf".to_string(), m.len(), m.modified().unwrap())
        };
        assert_eq!(list_docs(&dir).await.unwrap().0[1].pages, Some(2));
        let key = key_for(&dir);
        assert_eq!(page_counts().lock().unwrap().get(&key), Some(&2));
        // A cached count is answered without parsing (planted 7 is what comes back)
        page_counts().lock().unwrap().insert(key, 7);
        assert_eq!(list_docs(&dir).await.unwrap().0[1].pages, Some(7));
        // Changed bytes (a new size): parsed again
        let mut bytes = TWO_PAGES.to_vec();
        bytes.extend_from_slice(b"\n%padding\n");
        std::fs::write(dir.join("plan.pdf"), bytes).unwrap();
        assert_eq!(list_docs(&dir).await.unwrap().0[1].pages, Some(2));
    }

    /// A one-page PDF with no text on it (a scanned page has pictures, this one nothing).
    fn textless_pdf() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> /Contents 4 0 R >>",
            "<< /Length 0 >>\nstream\n\nendstream",
        ];
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
        }
        let xref = out.len();
        out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
        for o in offsets {
            out.push_str(&format!("{o:010} 00000 n \n"));
        }
        out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1));
        out.into_bytes()
    }

    /// The routes as the server serves them: the router with its token and person-only checks.
    mod routes {
        use super::*;
        use axum::body::to_bytes;
        use axum::http::Request;
        use serde_json::Value;
        use tower::ServiceExt;

        struct Env {
            app: axum::Router,
            state: AppState,
            tmp: tempfile::TempDir,
        }

        async fn setup() -> Env {
            let tmp = tempfile::tempdir().unwrap();
            let db = crate::db::Database::init(tmp.path()).await.unwrap();
            let config = crate::config::Config { data_dir: tmp.path().to_path_buf(), ..Default::default() };
            let state = AppState::new(
                config.clone(),
                db,
                crate::tools::ToolRunner::new(tmp.path().join("workspace")),
                crate::broker::PermissionBroker::new(),
                crate::router::LayaRouter::new(config.laya_url.clone()),
            );
            Env { app: crate::server::create_router(state.clone()), state, tmp }
        }

        fn docs(e: &Env) -> PathBuf {
            docs_dir(e.tmp.path(), &SpaceId::default_space())
        }

        fn person(e: &Env) -> String {
            e.state.api_token.reveal().to_string()
        }

        /// A bot's turn token in #general of the default space.
        async fn bot_token(e: &Env) -> String {
            use crate::server::continuity::{SelfScope, TurnOrigin};
            let space = SpaceId::default_space();
            let new = crate::db::models::NewKwant {
                name: "Ana".into(),
                title: "Reporter".into(),
                persona_prompt: String::new(),
                soul: String::new(),
                avatar_sprite: "s".into(),
                driver_type: "cli".into(),
                preferred_model: "codex".into(),
            };
            let bot = e.state.db.create_kwant(&space, &new).await.unwrap();
            let general = e.state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap();
            let scope = SelfScope {
                kwant_id: bot.id.clone(),
                kwant_name: bot.name.clone(),
                origin: Some(TurnOrigin { channel_id: Some(general.id.clone()), ..Default::default() }),
                turn_id: Some("t-docs".into()),
                ..Default::default()
            };
            crate::server::mcp_runtime::prepare_turn_with_self(&e.state, &bot, Some("t-docs".into()), None, Some(scope)).await.unwrap().1
        }

        async fn call(e: &Env, method: &str, uri: &str, bearer: &str, body: Vec<u8>) -> (StatusCode, Value) {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::from(body))
                .unwrap();
            let resp = e.app.clone().oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        }

        const BASE: &str = "/api/spaces/spc_default/docs";

        async fn add(e: &Env, name: &str, bytes: &[u8]) -> (StatusCode, Value) {
            call(e, "POST", &format!("{BASE}?name={name}"), &person(e), bytes.to_vec()).await
        }

        #[tokio::test]
        async fn the_person_adds_a_document_and_it_is_listed_and_quotable() {
            let e = setup().await;
            assert!(!docs(&e).exists());
            let (s, v) = add(&e, "Returns%20Policy.md", b"You can return any item within 30 days.").await;
            assert_eq!(s, StatusCode::CREATED, "{v}");
            assert_eq!((v["name"].as_str(), v["size"].as_u64()), (Some("Returns Policy.md"), Some(39)));
            assert!(v["modified"].is_string() && v.get("pages").is_none(), "{v}");
            assert_eq!(std::fs::read(docs(&e).join("Returns Policy.md")).unwrap(), b"You can return any item within 30 days.");
            let (s, v) = add(&e, "plan.pdf", TWO_PAGES).await;
            assert_eq!((s, v["pages"].as_u64()), (StatusCode::CREATED, Some(2)), "{v}");
            let (_, list) = call(&e, "GET", BASE, &person(&e), Vec::new()).await;
            let names: Vec<&str> = list["docs"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
            assert_eq!(names, ["Returns Policy.md", "plan.pdf"]);
            // Nothing half-written left behind
            let hidden = std::fs::read_dir(docs(&e)).unwrap().filter(|d| d.as_ref().unwrap().file_name().to_string_lossy().starts_with('.')).count();
            assert_eq!(hidden, 0);
        }

        #[tokio::test]
        async fn each_refusal_has_its_status_and_code_and_writes_nothing() {
            let e = setup().await;
            add(&e, "faq.md", b"first").await;
            let big = vec![b'a'; MAX_FILE_BYTES as usize + 1];
            for (name, bytes, status, code) in [
                ("..%2Fescape.md", b"x".to_vec(), StatusCode::BAD_REQUEST, "bad_name"),
                ("sub%2Fescape.md", b"x".to_vec(), StatusCode::BAD_REQUEST, "bad_name"),
                ("%2Ehidden.md", b"x".to_vec(), StatusCode::BAD_REQUEST, "bad_name"),
                ("a%0Ab.md", b"x".to_vec(), StatusCode::BAD_REQUEST, "bad_name"),
                ("", b"x".to_vec(), StatusCode::BAD_REQUEST, "bad_name"),
                ("photo.png", b"x".to_vec(), StatusCode::UNSUPPORTED_MEDIA_TYPE, "wrong_kind"),
                ("readme", b"x".to_vec(), StatusCode::UNSUPPORTED_MEDIA_TYPE, "wrong_kind"),
                ("big.txt", big, StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
                ("faq.md", b"second".to_vec(), StatusCode::CONFLICT, "exists"),
                ("scan.pdf", textless_pdf(), StatusCode::UNPROCESSABLE_ENTITY, "no_text"),
                ("broken.pdf", b"%PDF-1.7\nnot really".to_vec(), StatusCode::UNPROCESSABLE_ENTITY, "unreadable"),
                ("latin1.txt", vec![b'c', b'a', b'f', 0xE9], StatusCode::UNPROCESSABLE_ENTITY, "unreadable"),
            ] {
                let (s, v) = add(&e, name, &bytes).await;
                assert_eq!((s, v["code"].as_str()), (status, Some(code)), "{name}: {v}");
            }
            let (s, v) = add(&e, "scan.pdf", &textless_pdf()).await;
            assert!(v["error"].as_str().unwrap().starts_with("PDF has no text layer"), "{s} {v}");
            assert!(!e.tmp.path().join("escape.md").exists());
            assert_eq!(std::fs::read(docs(&e).join("faq.md")).unwrap(), b"first");
            let mut left: Vec<String> = std::fs::read_dir(docs(&e)).unwrap().map(|d| d.unwrap().file_name().into_string().unwrap()).collect();
            left.sort();
            assert_eq!(left, ["faq.md"]);
            // A declared length over 2 MB is refused before the body is read
            let req = Request::builder()
                .method("POST")
                .uri(format!("{BASE}?name=huge.md"))
                .header("authorization", format!("Bearer {}", person(&e)))
                .header("content-length", (MAX_FILE_BYTES + 1).to_string())
                .body(Body::from(vec![b'a'; MAX_FILE_BYTES as usize + 1]))
                .unwrap();
            assert_eq!(e.app.clone().oneshot(req).await.unwrap().status(), StatusCode::PAYLOAD_TOO_LARGE);
            // An unknown space
            let (s, _) = call(&e, "POST", "/api/spaces/nope/docs?name=a.md", &person(&e), b"x".to_vec()).await;
            assert_eq!(s, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn replace_swaps_the_words_and_a_link_is_replaced_not_followed() {
            let e = setup().await;
            add(&e, "faq.md", b"old words").await;
            let (s, v) = call(&e, "POST", &format!("{BASE}?name=faq.md&replace=1"), &person(&e), b"new words".to_vec()).await;
            assert_eq!((s, v["size"].as_u64()), (StatusCode::CREATED, Some(9)), "{v}");
            assert_eq!(std::fs::read(docs(&e).join("faq.md")).unwrap(), b"new words");
            // A failed replace keeps the old document
            let (s, _) = call(&e, "POST", &format!("{BASE}?name=faq.md&replace=1"), &person(&e), vec![0xFF, 0xFE]).await;
            assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(std::fs::read(docs(&e).join("faq.md")).unwrap(), b"new words");
            #[cfg(unix)]
            {
                std::fs::write(e.tmp.path().join("secret.md"), "outside").unwrap();
                std::os::unix::fs::symlink(e.tmp.path().join("secret.md"), docs(&e).join("link.md")).unwrap();
                let (s, v) = add(&e, "link.md", b"inside").await;
                assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("exists")), "{v}");
                let (s, _) = call(&e, "POST", &format!("{BASE}?name=link.md&replace=1"), &person(&e), b"inside".to_vec()).await;
                assert_eq!(s, StatusCode::CREATED);
                assert_eq!(std::fs::read_to_string(e.tmp.path().join("secret.md")).unwrap(), "outside");
                assert!(std::fs::symlink_metadata(docs(&e).join("link.md")).unwrap().file_type().is_file());
                // A folder with a document's name is never replaced
                std::fs::create_dir(docs(&e).join("dir.md")).unwrap();
                let (s, v) = call(&e, "POST", &format!("{BASE}?name=dir.md&replace=1"), &person(&e), b"x".to_vec()).await;
                assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("exists")), "{v}");
                assert!(docs(&e).join("dir.md").is_dir());
            }
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn a_docs_folder_that_is_a_link_is_never_written_through() {
            let e = setup().await;
            let elsewhere = e.tmp.path().join("elsewhere");
            std::fs::create_dir(&elsewhere).unwrap();
            std::fs::write(elsewhere.join("faq.md"), "theirs").unwrap();
            std::os::unix::fs::symlink(&elsewhere, docs(&e)).unwrap();
            let (s, v) = add(&e, "new.md", b"x").await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("docs_link")), "{v}");
            let (s, v) = call(&e, "DELETE", &format!("{BASE}/faq.md"), &person(&e), Vec::new()).await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("docs_link")), "{v}");
            assert!(!elsewhere.join("new.md").exists() && elsewhere.join("faq.md").exists());
        }

        #[tokio::test]
        async fn a_space_takes_two_hundred_documents_and_replacing_one_still_works() {
            let e = setup().await;
            std::fs::create_dir_all(docs(&e)).unwrap();
            for i in 0..MAX_FILES {
                std::fs::write(docs(&e).join(format!("{i:03}.txt")), "x").unwrap();
            }
            let (s, v) = add(&e, "one-more.md", b"x").await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("full")), "{v}");
            let (s, _) = call(&e, "POST", &format!("{BASE}?name=000.txt&replace=1"), &person(&e), b"y".to_vec()).await;
            assert_eq!(s, StatusCode::CREATED);
            std::fs::remove_file(docs(&e).join("199.txt")).unwrap();
            assert_eq!(add(&e, "one-more.md", b"x").await.0, StatusCode::CREATED);
        }

        #[tokio::test]
        async fn the_person_removes_a_document_and_nothing_outside_the_folder() {
            let e = setup().await;
            add(&e, "faq.md", b"x").await;
            std::fs::write(e.tmp.path().join("secret.md"), "outside").unwrap();
            assert_eq!(call(&e, "DELETE", &format!("{BASE}/faq.md"), &person(&e), Vec::new()).await.0, StatusCode::NO_CONTENT);
            assert!(!docs(&e).join("faq.md").exists());
            for (name, status, code) in [
                ("faq.md", StatusCode::NOT_FOUND, "not_found"),
                ("..%2Fsecret.md", StatusCode::BAD_REQUEST, "bad_name"),
                ("%2E%2E%2Fsecret.md", StatusCode::BAD_REQUEST, "bad_name"),
                ("photo.png", StatusCode::BAD_REQUEST, "bad_name"),
            ] {
                let (s, v) = call(&e, "DELETE", &format!("{BASE}/{name}"), &person(&e), Vec::new()).await;
                assert_eq!((s, v["code"].as_str()), (status, Some(code)), "{name}: {v}");
            }
            assert!(e.tmp.path().join("secret.md").exists());
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(e.tmp.path().join("secret.md"), docs(&e).join("link.md")).unwrap();
                let (s, v) = call(&e, "DELETE", &format!("{BASE}/link.md"), &person(&e), Vec::new()).await;
                assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_name")), "{v}");
                assert!(e.tmp.path().join("secret.md").exists() && std::fs::symlink_metadata(docs(&e).join("link.md")).is_ok());
            }
            // No docs folder at all: 404
            let e = setup().await;
            assert_eq!(call(&e, "DELETE", &format!("{BASE}/faq.md"), &person(&e), Vec::new()).await.0, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn a_bot_can_read_and_quote_but_never_add_or_remove() {
            let e = setup().await;
            add(&e, "faq.md", b"You can return any item within 30 days.").await;
            let bot = bot_token(&e).await;
            for (method, uri) in [("POST", format!("{BASE}?name=mine.md")), ("POST", format!("{BASE}?name=faq.md&replace=1")), ("DELETE", format!("{BASE}/faq.md"))] {
                let (s, v) = call(&e, method, &uri, &bot, b"words I made up".to_vec()).await;
                assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("person_only")), "{method} {uri}: {v}");
            }
            assert!(!docs(&e).join("mine.md").exists());
            assert_eq!(std::fs::read(docs(&e).join("faq.md")).unwrap(), b"You can return any item within 30 days.");
            // Any other bearer that isn't the API token is refused the same way
            assert_eq!(call(&e, "DELETE", &format!("{BASE}/faq.md"), "not-the-token", Vec::new()).await.0, StatusCode::FORBIDDEN);
            let (s, v) = call(&e, "GET", &format!("{BASE}/page?file=faq.md&quote=return%20any%20item"), &bot, Vec::new()).await;
            assert_eq!((s, v["match"].as_str()), (StatusCode::OK, Some("return any item")), "{v}");
            assert_eq!(call(&e, "GET", BASE, &bot, Vec::new()).await.0, StatusCode::OK);
        }

        #[tokio::test]
        async fn the_page_route_gives_the_page_split_around_the_quote() {
            let e = setup().await;
            add(&e, "faq.md", b"# Returns\n\nYou can return any item\nwithin 30 days.\n").await;
            add(&e, "plan.pdf", TWO_PAGES).await;
            let (s, v) = call(&e, "GET", &format!("{BASE}/page?file=faq.md&quote=return%20any%20item%20within%2030%20days"), &person(&e), Vec::new()).await;
            assert_eq!(s, StatusCode::OK, "{v}");
            assert_eq!(v["file"], "faq.md");
            assert_eq!((v["page"].as_u64(), v["pages"].as_u64()), (Some(1), Some(1)));
            assert_eq!((v["text_before"].as_str(), v["match"].as_str(), v["text_after"].as_str()), (Some("# Returns\n\nYou can "), Some("return any item\nwithin 30 days"), Some(".\n")));
            assert_eq!((v["cut_before"].as_bool(), v["cut_after"].as_bool()), (Some(false), Some(false)));
            let (_, v) = call(&e, "GET", &format!("{BASE}/page?file=faq.md"), &person(&e), Vec::new()).await;
            assert_eq!((v["match"].as_str(), v["text_after"].as_str()), (Some(""), Some("# Returns\n\nYou can return any item\nwithin 30 days.\n")));
            let (_, v) = call(&e, "GET", &format!("{BASE}/page?file=faq.md&quote=within%2060%20days"), &person(&e), Vec::new()).await;
            assert_eq!(v["match"], "");
            let (s, v) = call(&e, "GET", &format!("{BASE}/page?file=plan.pdf&page=2&quote=Budget%20is%2042%20euros"), &person(&e), Vec::new()).await;
            assert_eq!((s, v["match"].as_str(), v["pages"].as_u64()), (StatusCode::OK, Some("Budget is 42 euros"), Some(2)), "{v}");
            // Every refusal is JSON with an error: the Documents tab probes with an empty file and
            // reads a bare 404 (no JSON) as "these routes aren't there yet"
            for (uri, status) in [
                ("page?file=plan.pdf&page=3", StatusCode::NOT_FOUND),
                ("page?file=plan.pdf&page=0", StatusCode::NOT_FOUND),
                ("page?file=faq.md&page=2", StatusCode::NOT_FOUND),
                ("page?file=missing.md", StatusCode::NOT_FOUND),
                ("page?file=..%2Fkwnt.db", StatusCode::BAD_REQUEST),
                ("page?file=&page=1", StatusCode::BAD_REQUEST),
                ("page", StatusCode::BAD_REQUEST),
            ] {
                let (s, v) = call(&e, "GET", &format!("{BASE}/{uri}"), &person(&e), Vec::new()).await;
                assert_eq!(s, status, "{uri}");
                assert!(v["error"].as_str().is_some_and(|m| !m.is_empty()), "{uri}: {v}");
            }
        }
    }
}
