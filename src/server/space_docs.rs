//! A space's own documents (roadmap 10c): the FAQ, policies or catalogue a business keeps in its
//! space's `docs/` folder, which a bot can quote with the same word-for-word check kwant runs on
//! web pages ([`crate::sandbox::cite::cite_text`], unchanged).
//!
//! - `GET /api/spaces/:id/docs`: the documents (name, size, modified, pages for a PDF).
//! - `POST /api/spaces/:id/docs/cite` `{file, quote}`: the quote checked against that file, one
//!   [`FrameText`] per PDF page (`kwant-doc://<space>/<file>#page=N`), answered as the web cite
//!   answers (`{quote, url, link}`) or refused with cite.rs's own reason (422).
//!
//! The folder is `<space's data folder>/docs` ([`docs_dir`]): one per space, outside every bot's
//! work folder, so a bot working in its folder can't write the words it then cites. Only the
//! folder's own files are read (no subfolders), each opened from the folder's descriptor without
//! following a link ([`safe_fs::Dir`]); a `docs` that is itself a link is refused. Text, Markdown
//! and PDF (its text layer, the reader attachments use: no OCR, no model call), 2 MB a file, 200
//! files a space. Nothing here writes.
#![allow(clippy::result_large_err)]

use super::AppState;
use crate::db::spaces::SpaceId;
use crate::sandbox::cite::{self, FrameText};
use crate::sandbox::safe_fs::{self, Dir, Kind};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

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

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Where a space's documents live: `docs` in the space's own data folder (the data folder itself
/// for Default).
pub fn docs_dir(data_dir: &std::path::Path, space: &SpaceId) -> PathBuf {
    crate::db::spaces::space_data_dir(data_dir, space).join("docs")
}

/// Why `name` can't name a document: it must be a plain file name in the folder (no separator,
/// no `..`, no leading dot) of a kind kwant reads.
pub fn bad_name(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("file is required: a document's name, as the list gives it")
    } else if name.contains(['/', '\\', '\0']) || name.contains("..") || name.starts_with('.') || name != name.trim() {
        Some("file must be a document's own name in the space's docs folder (no folders, no ..)")
    } else if kind_of(name).is_none() {
        Some("only .md, .markdown, .txt and .pdf documents can be read")
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

/// The space's documents, sorted by name, at most [`MAX_FILES`]; the flag says some were left out.
/// Links, folders and other kinds of file are skipped.
pub async fn list_docs(dir: &std::path::Path) -> Result<(Vec<DocInfo>, bool), Response> {
    let Some(docs) = open_docs(dir)? else {
        return Ok((Vec::new(), false));
    };
    let (names, more) = doc_names(&docs)?;
    let mut out = Vec::with_capacity(names.len());
    for (name, len) in names {
        // Opened again from the folder's descriptor: a file swapped for a link since the list is left out
        let Ok(file) = docs.open_file(std::path::Path::new(&name)) else { continue };
        let meta = file.metadata().ok();
        let size = meta.as_ref().map_or(len, |m| m.len());
        let modified = meta.and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        let too_large = size > MAX_FILE_BYTES;
        let pages = match (is_pdf(&name), too_large) {
            (true, false) => match safe_fs::read_capped(file, MAX_FILE_BYTES) {
                Ok(bytes) => pdf_page_count(bytes).await,
                Err(_) => None,
            },
            _ => None,
        };
        out.push(DocInfo { name, size, modified, pages, too_large });
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
}
