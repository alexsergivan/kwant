//! `files`: a preview of the files a bot shared with a message (`share_file`'s "Shared a file"
//! card), and only those. A file qualifies when its stored attachment row is a file this same bot
//! shared, into this same chat, and not yet tied to another message: so an id from another
//! space, chat or bot (or one that doesn't exist) is refused and no block is made. Bytes are read
//! only through [`crate::server::attachments::blob_path`], the same sha256-only lookup the
//! authorized attachment routes use, and never served from here; the chat loads a file or image
//! through those routes.

use std::path::Path;

use sqlx::SqliteConnection;

use crate::db::attachments::Attachment;
use crate::db::models::{MessageExtra, NewMessage};
use crate::server::attachments as att;

use super::verification::safe;
use super::{Block, FileItem, FilePreview, Files, Provenance, Source};

const MAX_FILES: usize = 10;
const MAX_TABLE_BYTES: i64 = 1024 * 1024;
const MAX_TEXT_BYTES: i64 = 1024 * 1024;
const MAX_ROWS: usize = 20;
const MAX_COLS: usize = 8;
const MAX_LINES: usize = 20;
const MAX_CELL: usize = 300;
const MAX_NAME: usize = 120;

/// The files block for a "Shared a file" message (`meta.kind == "file"`), or `None`: not such a
/// message, no files, more than 10, an attachment that isn't this bot's own share in this chat, or
/// nothing the chat doesn't already show (images the chat draws inline).
pub async fn candidate(conn: &mut SqliteConnection, input: &NewMessage, extra: &MessageExtra, root: Option<&Path>) -> Option<Block> {
    let meta = extra.meta.as_ref()?;
    if meta.get("kind").and_then(|v| v.as_str()) != Some("file") {
        return None;
    }
    let ids: Vec<&str> = meta.get("attachments")?.as_array()?.iter().filter_map(|a| a.get("id")?.as_str()).collect();
    if ids.is_empty() || ids.len() > MAX_FILES {
        return None;
    }
    let mut items = Vec::new();
    for id in ids {
        let row = match sqlx::query_as::<_, Attachment>("SELECT * FROM attachments WHERE id = ?").bind(id).fetch_optional(&mut *conn).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                tracing::warn!("smart results: a files block named an attachment that doesn't exist; no block");
                return None;
            }
            Err(e) => {
                tracing::warn!("smart results: couldn't read an attachment for a files block: {e}");
                return None;
            }
        };
        if !belongs(&row, input, extra) {
            tracing::warn!("smart results: a files block named an attachment from another chat or bot; no block");
            return None;
        }
        items.push(item(&row, root, items.len()));
    }
    // A message of only inline images shows them already; a preview would only repeat them
    if items.iter().all(|i| matches!(i.preview, Some(FilePreview::Image))) {
        return None;
    }
    let first = items.first()?.attachment_id.clone();
    Some(Block::Files(Files {
        id: "b0".into(),
        title: None,
        source: Source::slot(),
        provenance: Provenance::kwant_data(format!("attachment:{first}")),
        items,
    }))
}

/// Shared by this message's sender, into this message's own chat and thread, and not yet tied to
/// a message.
fn belongs(row: &Attachment, input: &NewMessage, extra: &MessageExtra) -> bool {
    row.source == "agent"
        && row.message_id.is_none()
        && row.kwant_id.as_deref() == Some(input.sender_id.as_str())
        && row.channel_id == input.channel_id
        && row.direct_kwant_id == input.direct_kwant_id
        && row.thread_id == extra.thread_id
}

fn item(row: &Attachment, root: Option<&Path>, n: usize) -> FileItem {
    let preview = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| preview(row, root))).ok().flatten();
    FileItem {
        id: format!("f{n}"),
        attachment_id: row.id.clone(),
        name: safe(&row.name, MAX_NAME),
        mime: row.mime.clone(),
        size_bytes: row.size.max(0),
        preview,
    }
}

fn preview(row: &Attachment, root: Option<&Path>) -> Option<FilePreview> {
    if att::INLINE_TYPES.contains(&row.mime.as_str()) {
        return Some(FilePreview::Image);
    }
    let lower = row.name.to_ascii_lowercase();
    let delim = if lower.ends_with(".tsv") || row.mime == "text/tab-separated-values" {
        Some(b'\t')
    } else if lower.ends_with(".csv") || row.mime == "text/csv" {
        Some(b',')
    } else {
        None
    };
    let is_text = row.mime == "text/plain" || row.mime == "text/markdown" || lower.ends_with(".md") || lower.ends_with(".txt");
    let limit = match (delim, is_text) {
        (Some(_), _) => MAX_TABLE_BYTES,
        (None, true) => MAX_TEXT_BYTES,
        (None, false) => return None,
    };
    if row.size <= 0 || row.size > limit {
        return None;
    }
    let path = att::blob_path(root?, &row.sha256)?;
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() as i64 > limit {
        return None;
    }
    let text = decode(&bytes)?;
    match delim {
        Some(d) => table(&text, d),
        None => Some(excerpt(&text)),
    }
}

/// UTF-8 (a BOM is dropped) or BOM-marked UTF-16; anything else (invalid UTF-8) has no preview.
fn decode(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    std::str::from_utf8(bytes).ok().map(str::to_string)
}

fn utf16(bytes: &[u8], read: fn([u8; 2]) -> u16) -> Option<String> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| read([c[0], c[1]])).collect();
    String::from_utf16(&units).ok()
}

/// The first 20 lines (control characters dropped, each at most 300 characters) joined by newlines.
fn excerpt(text: &str) -> FilePreview {
    let lines: Vec<String> = text.lines().take(MAX_LINES).map(|l| safe(l, MAX_CELL)).collect();
    FilePreview::Text { excerpt: lines.join("\n") }
}

/// The header and the first 20 records of a CSV/TSV, at most 8 columns. Quotes, doubled quotes and
/// newlines inside quoted fields are read as usual; cells are plain text, never markup, and a
/// cell that starts like a formula is shown as the text it is (this block never opens a file).
fn table(text: &str, delim: u8) -> Option<FilePreview> {
    let mut records = Records { chars: text.chars().peekable(), delim: delim as char };
    let header = records.next()?;
    let width = header.len().min(MAX_COLS);
    if width == 0 || header.iter().all(|c| c.trim().is_empty()) {
        return None;
    }
    let columns: Vec<String> = header.iter().take(width).map(|c| safe(c.trim(), MAX_CELL)).collect();
    let mut rows = Vec::new();
    for record in records.take(MAX_ROWS) {
        let mut row: Vec<String> = record.iter().take(width).map(|c| safe(c.trim(), MAX_CELL)).collect();
        row.resize(width, String::new());
        rows.push(row);
    }
    Some(FilePreview::Table { columns, rows })
}

/// Records of a delimited text, skipping blank lines, reading at most as much as 21 records need.
struct Records<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    delim: char,
}

impl Iterator for Records<'_> {
    type Item = Vec<String>;

    fn next(&mut self) -> Option<Vec<String>> {
        loop {
            self.chars.peek()?;
            let mut fields = Vec::new();
            let mut field = String::new();
            let mut quoted = false;
            let mut any = false;
            while let Some(c) = self.chars.next() {
                any = true;
                match c {
                    '"' if quoted && self.chars.peek() == Some(&'"') => {
                        self.chars.next();
                        field.push('"');
                    }
                    '"' if field.trim().is_empty() && !quoted => {
                        quoted = true;
                        field.clear();
                    }
                    '"' if quoted => quoted = false,
                    c if c == self.delim && !quoted => fields.push(std::mem::take(&mut field)),
                    '\r' if !quoted => {}
                    '\n' if !quoted => break,
                    c => field.push(c),
                }
                // A 1,000-column row or one huge quoted cell is cut where the preview stops caring
                if fields.len() > 1000 || field.len() > 100_000 {
                    for c in self.chars.by_ref() {
                        if c == '\n' && !quoted {
                            break;
                        }
                    }
                    break;
                }
            }
            fields.push(field);
            if any && !(fields.len() == 1 && fields[0].trim().is_empty()) {
                return Some(fields);
            }
        }
    }
}
