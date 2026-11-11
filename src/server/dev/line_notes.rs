//! G1 line notes on a review's diff, sent to the author bot as one message (0.9.0 wave 1, Juno;
//! release-090/g1-api.md, Mira's designs/g1-line-notes.md; Iris builds the UI).
//!
//! * `GET /api/dev/cards/:id/changes` serves the diff of the PR's newest review card as parsed
//!   hunks. It is the card's own files: the patches the poller read for that head, after the
//!   secret scan hid what it found and the 60k cut. A file the cut dropped shows as
//!   `omitted: "too_large"` (the UI links to the forge); nothing here reads the forge, so a line the
//!   scan hid never comes back through this route.
//! * Notes belong to the PR (space, repo, `pr:N`), not to one head. A note keeps the head it was
//!   written on and the lines as they were (`code`); [`locate`] finds them in a newer head by
//!   their text (`at_head`).
//! * Send posts ONE message from the person in the author bot's 1:1, the same path as a chat
//!   message (it queues when the bot is busy), with `meta.kind = "pr_notes"`. A repeat `send_id`
//!   returns the first result and posts nothing.
//! * On the next push (a new review card for the PR), each sent note becomes `resolved` when at
//!   least one of its lines was edited or deleted, else `open` ([`after_push`]).
//! * Every change fires the card's `dev_card` event, so an open desk refetches.
//!
//! Only the person reads or writes notes (agent tokens are refused), and nothing here writes to the
//! forge.

// Route helpers return the refusal `Response` itself, as in `continuity.rs`.
#![allow(clippy::result_large_err)]

use super::facts::{DevFacts, FileDiff};
use crate::drivers::events::RuntimeEvent;
use super::routes::REFUSAL;
use crate::db::dev_line_notes::{LineNoteRow, LineNoteSendRow, DRAFT, OPEN, RESOLVED, SEND_ID_HOURS, SENT};
use crate::db::dev_mode::{DevCardRow, LANDED, REVIEW, STALE};
use crate::db::models::Kwant;
use crate::db::spaces::SpaceId;
use crate::server::spaces::active_space;
use crate::server::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// Most characters in a note.
pub const TEXT_CHARS: usize = 4_000;
/// Most diff lines one note covers.
pub const RANGE_LINES: usize = 200;
/// Most notes in one send.
pub const SEND_MAX: usize = 20;
/// A code line in the message is cut at this many characters.
pub const CODE_CHARS: usize = 160;
/// A range over this many lines sends its first [`HEAD_LINES`] and last [`TAIL_LINES`].
pub const RANGE_SHOWN: usize = 12;
const HEAD_LINES: usize = 6;
const TAIL_LINES: usize = 3;
/// A file with more changed lines than this starts collapsed.
pub const BIG_LINES: u64 = 400;

const LOCKFILES: [&str; 10] = [
    "Cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Gemfile.lock",
    "poetry.lock",
    "composer.lock",
    "go.sum",
    "bun.lockb",
];

// ---------------------------------------------------------------------------------------------
// The diff: pure, and tested below

/// One line of a unified diff: `ctx`, `add` or `del`, its old and new numbers, its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DLine {
    pub kind: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DHunk {
    pub header: String,
    pub lines: Vec<DLine>,
}

/// A note's first or last line: the side (`new` or `old`) and its number on that side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct End {
    pub side: String,
    pub line: u32,
}

/// The `@@ -a,b +c,d @@` numbers: where the old and new sides start.
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let rest = header.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let new = rest.strip_prefix('+')?.split(' ').next()?;
    let first = |s: &str| s.split(',').next()?.parse::<u32>().ok();
    Some((first(old)?, first(new)?))
}

/// The hunks of a unified diff patch (GitHub's `patch` field), with old and new line numbers.
/// Lines before the first `@@`, and `\ No newline at end of file`, are left out.
pub fn parse_patch(patch: &str) -> Vec<DHunk> {
    let mut out: Vec<DHunk> = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    for raw in patch.lines() {
        if raw.starts_with("@@") {
            let Some((o, n)) = hunk_starts(raw) else { continue };
            (old, new) = (o, n);
            out.push(DHunk { header: raw.to_string(), lines: Vec::new() });
            continue;
        }
        let Some(h) = out.last_mut() else { continue };
        let (kind, text) = match raw.as_bytes().first() {
            Some(b'+') => ("add", &raw[1..]),
            Some(b'-') => ("del", &raw[1..]),
            Some(b' ') => ("ctx", &raw[1..]),
            Some(b'\\') => continue,
            _ => ("ctx", raw),
        };
        let line = match kind {
            "add" => DLine { kind: kind.into(), old: None, new: Some(new), text: text.into() },
            "del" => DLine { kind: kind.into(), old: Some(old), new: None, text: text.into() },
            _ => DLine { kind: kind.into(), old: Some(old), new: Some(new), text: text.into() },
        };
        if line.old.is_some() {
            old += 1;
        }
        if line.new.is_some() {
            new += 1;
        }
        h.lines.push(line);
    }
    out
}

/// Where `e` is in a hunk: the line with that number on that side.
fn find(h: &DHunk, e: &End) -> Option<usize> {
    h.lines.iter().position(|l| is_at(l, e))
}

/// Why a range can't be noted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    NotInDiff,
    CrossHunk,
    TooBig,
}

/// The unified lines from `start` to `end` (either order), inclusive, in one hunk.
pub fn select(hunks: &[DHunk], start: &End, end: &End) -> Result<Vec<DLine>, RangeError> {
    let at = |e: &End| hunks.iter().enumerate().find_map(|(i, h)| find(h, e).map(|p| (i, p)));
    let (Some((hs, ps)), Some((he, pe))) = (at(start), at(end)) else { return Err(RangeError::NotInDiff) };
    if hs != he {
        return Err(RangeError::CrossHunk);
    }
    let (a, b) = (ps.min(pe), ps.max(pe));
    if b - a + 1 > RANGE_LINES {
        return Err(RangeError::TooBig);
    }
    Ok(hunks[hs].lines[a..=b].to_vec())
}

/// The end of a line on the side the note prefers, else the side it has.
fn end_of(l: &DLine, prefer: &str) -> End {
    match (prefer, l.new, l.old) {
        ("new", Some(n), _) => End { side: "new".into(), line: n },
        ("old", _, Some(o)) => End { side: "old".into(), line: o },
        (_, Some(n), _) if l.kind != "del" => End { side: "new".into(), line: n },
        (_, _, Some(o)) => End { side: "old".into(), line: o },
        (_, Some(n), _) => End { side: "new".into(), line: n },
        _ => End { side: prefer.into(), line: 0 },
    }
}

/// Where the noted lines `code` are in `hunks`: the same lines (kind and text) one after another,
/// the place nearest line `near` when there are several. Their first and last ends, on the sides
/// the note named. None when they aren't there: at least one was edited or deleted.
pub fn locate(hunks: &[DHunk], code: &[DLine], start_side: &str, end_side: &str, near: u32) -> Option<(End, End)> {
    if code.is_empty() {
        return None;
    }
    let same = |a: &DLine, b: &DLine| a.kind == b.kind && a.text == b.text;
    let mut best: Option<(u32, End, End)> = None;
    for h in hunks {
        if h.lines.len() < code.len() {
            continue;
        }
        for i in 0..=(h.lines.len() - code.len()) {
            if !h.lines[i..i + code.len()].iter().zip(code).all(|(a, b)| same(a, b)) {
                continue;
            }
            let s = end_of(&h.lines[i], start_side);
            let e = end_of(&h.lines[i + code.len() - 1], end_side);
            let d = s.line.abs_diff(near);
            if best.as_ref().is_none_or(|(bd, _, _)| d < *bd) {
                best = Some((d, s, e));
            }
        }
    }
    best.map(|(_, s, e)| (s, e))
}

/// `src/a.tsx:40-41` for the noted lines: new-side numbers when any line has one (removed lines
/// have none), else old.
pub fn at_text(path: &str, code: &[DLine]) -> String {
    let news: Vec<u32> = code.iter().filter(|l| l.kind != "del").filter_map(|l| l.new).collect();
    let nums = if news.is_empty() { code.iter().filter_map(|l| l.old).collect() } else { news };
    match (nums.iter().min(), nums.iter().max()) {
        (Some(a), Some(b)) if a != b => format!("{path}:{a}-{b}"),
        (Some(a), _) => format!("{path}:{a}"),
        _ => path.to_string(),
    }
}

/// What a file's patch shows: `binary` (no patch, no line counts), `too_large` (no patch but
/// changed lines: GitHub's limit or the card's 60k cut), or none.
pub fn omitted(f: &FileDiff) -> Option<&'static str> {
    if f.patch.is_some() || (f.status == "renamed" && f.additions + f.deletions == 0) {
        return None;
    }
    Some(if f.additions + f.deletions == 0 { "binary" } else { "too_large" })
}

/// A file that starts collapsed: many changed lines, or a lockfile.
pub fn is_big(f: &FileDiff) -> bool {
    let name = f.path.rsplit('/').next().unwrap_or(&f.path);
    f.additions + f.deletions > BIG_LINES || LOCKFILES.contains(&name)
}

/// One file of `/changes`.
pub fn file_json(f: &FileDiff, notes: usize) -> Value {
    json!({
        "path": f.path,
        "old_path": f.previous_path,
        "status": f.status,
        "additions": f.additions,
        "deletions": f.deletions,
        "omitted": omitted(f),
        "big": is_big(f),
        "notes": notes,
        "hunks": f.patch.as_deref().map(parse_patch).unwrap_or_default(),
    })
}

fn cut_line(text: &str) -> String {
    if text.chars().count() <= CODE_CHARS {
        return text.to_string();
    }
    let mut s: String = text.chars().take(CODE_CHARS).collect();
    s.push('…');
    s
}

/// The noted lines as the message quotes them: `   40 | code`, a removed line as
/// `   38 (removed) | code`; a range over [`RANGE_SHOWN`] lines keeps its first and last lines.
fn quoted(code: &[DLine]) -> Vec<String> {
    let line = |l: &DLine| match (l.kind.as_str(), l.new, l.old) {
        ("del", _, Some(o)) => format!("   {o} (removed) | {}", cut_line(&l.text)),
        (_, Some(n), _) => format!("   {n} | {}", cut_line(&l.text)),
        (_, _, Some(o)) => format!("   {o} | {}", cut_line(&l.text)),
        _ => format!("   | {}", cut_line(&l.text)),
    };
    if code.len() <= RANGE_SHOWN {
        return code.iter().map(line).collect();
    }
    let mut out: Vec<String> = code[..HEAD_LINES].iter().map(line).collect();
    out.push("   …".into());
    out.extend(code[code.len() - TAIL_LINES..].iter().map(line));
    out
}

/// One note as it goes out.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub path: String,
    pub code: Vec<DLine>,
    pub text: String,
}

/// The message the author bot gets (Mira's block).
pub fn message_text(pr: Option<u64>, branch: Option<&str>, head: &str, notes: &[Outgoing]) -> String {
    let n = notes.len();
    let mut on = match pr {
        Some(p) => format!("#{p}"),
        None => "the branch".to_string(),
    };
    if let Some(b) = branch.filter(|b| !b.is_empty()) {
        on.push(' ');
        on.push_str(b);
    }
    let head7: String = head.chars().take(7).collect();
    let mut out = format!("{n} {} on {on} (at {head7}), from the review:\n", if n == 1 { "note" } else { "notes" });
    for (i, note) in notes.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n", i + 1, at_text(&note.path, &note.code)));
        for l in quoted(&note.code) {
            out.push_str(&l);
            out.push('\n');
        }
        let mut lines = note.text.trim().lines();
        out.push_str(&format!("   → {}\n", lines.next().unwrap_or("")));
        for l in lines {
            out.push_str(&format!("     {l}\n"));
        }
    }
    out.push_str("\nPush to the same branch when done; kwant marks each note resolved when its lines change.");
    out
}

/// A note as the routes return it. `current` is the PR's newest head and its files (when known).
pub fn note_json(row: &LineNoteRow, current: Option<(&str, &HashMap<String, Vec<DHunk>>)>) -> Value {
    let code: Vec<DLine> = serde_json::from_str(&row.code).unwrap_or_default();
    let start = End { side: row.start_side.clone(), line: row.start_line as u32 };
    let end = End { side: row.end_side.clone(), line: row.end_line as u32 };
    let (at_head, seen_sha) = match current {
        Some((head, _)) if head == row.head_sha => (Some(json!({"start": start, "end": end})), None),
        Some((head, files)) => {
            let at = files.get(&row.path).and_then(|h| locate(h, &code, &row.start_side, &row.end_side, row.start_line as u32));
            (at.map(|(s, e)| json!({"start": s, "end": e})), Some(head.to_string()))
        }
        None => (None, None),
    };
    let ms = |v: Option<i64>| v.and_then(|t| Utc.timestamp_millis_opt(t).single());
    json!({
        "id": row.id,
        "path": row.path,
        "start": start,
        "end": end,
        "code": code,
        "text": row.text,
        "state": row.state,
        "head_sha": row.head_sha,
        "created_at": ms(Some(row.created_at)),
        "updated_at": ms(Some(row.updated_at)),
        "sent_at": ms(row.sent_at),
        "message_id": row.message_id,
        "checked_sha": row.checked_sha,
        "at_head": at_head,
        "seen_sha": seen_sha,
    })
}

/// The review summary line's numbers: `{total, changed, open, sha}` once a push checked any sent
/// note, else none. `sha` is the newest push that checked one.
pub fn summary(rows: &[LineNoteRow]) -> Option<Value> {
    let checked: Vec<&LineNoteRow> = rows.iter().filter(|r| r.state == RESOLVED || r.state == OPEN).collect();
    let newest = checked.iter().max_by_key(|r| r.updated_at)?;
    let sha = newest.checked_sha.clone();
    let of_sha: Vec<&&LineNoteRow> = checked.iter().filter(|r| r.checked_sha == sha).collect();
    Some(json!({
        "total": of_sha.len(),
        "changed": of_sha.iter().filter(|r| r.state == RESOLVED).count(),
        "open": of_sha.iter().filter(|r| r.state == OPEN).count(),
        "sha": sha,
    }))
}

/// Whether a sent note's lines changed in a newer head's files: none when that file's diff isn't
/// known there (cut), so it's checked on a later push instead. A file no longer in the PR's diff
/// lost the noted change, so that counts as changed.
pub fn changed(row: &LineNoteRow, files: &[FileDiff]) -> Option<bool> {
    let code: Vec<DLine> = serde_json::from_str(&row.code).ok()?;
    let Some(f) = files.iter().find(|f| f.path == row.path) else { return Some(true) };
    let patch = f.patch.as_deref()?;
    Some(locate(&parse_patch(patch), &code, &row.start_side, &row.end_side, row.start_line as u32).is_none())
}

// ---------------------------------------------------------------------------------------------
// The PR a card is about

/// A card's PR: its newest review card and that card's facts.
struct Pr {
    space: SpaceId,
    current: DevCardRow,
    facts: DevFacts,
}

impl Pr {
    fn files(&self) -> HashMap<String, Vec<DHunk>> {
        self.facts.files.iter().filter_map(|f| Some((f.path.clone(), parse_patch(f.patch.as_deref()?)))).collect()
    }
}

fn err(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"error": message, "code": code}))).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!("dev: line notes: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Dev mode could not read or save that")
}

/// The review card `card_id` and its PR's newest card, for the person only.
async fn pr_of(state: &AppState, headers: &HeaderMap, card_id: &str) -> Result<Pr, Response> {
    super::super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await?;
    let row = match state.db.dev_card(card_id.trim()).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, "not_found", "no such card")),
        Err(e) => return Err(internal(e)),
    };
    let space = active_space(state, &row.space_id).await?;
    if row.kind != REVIEW {
        return Err(err(StatusCode::NOT_FOUND, "not_review_card", "only a review card has changes and notes"));
    }
    let current = state.db.dev_cards_of_ref_in(&row.space_id, &row.repo, REVIEW, &row.ref_).await.map_err(internal)?.into_iter().next().unwrap_or(row);
    let facts: DevFacts = serde_json::from_str(&current.facts).map_err(|e| internal(format!("card {} facts: {e}", current.id)))?;
    Ok(Pr { space, current, facts })
}

/// `open`, `merged` (a landed card of the same repo came from this PR) or `closed` (its newest
/// card went stale with no landed card: closed, gone back to draft or quiet).
async fn pr_state(state: &AppState, pr: &Pr) -> &'static str {
    if pr.current.status != STALE {
        return "open";
    }
    let landed = state.db.dev_cards_in(&pr.current.space_id, Some(pr.current.repo.as_str()), LANDED, None, 200).await.unwrap_or_default();
    let merged = pr.facts.pr.is_some()
        && landed.iter().any(|c| serde_json::from_str::<DevFacts>(&c.facts).is_ok_and(|f| f.pr == pr.facts.pr));
    if merged {
        "merged"
    } else {
        "closed"
    }
}

/// The bot the notes go to: the review's author bot, else the bot the head branch names
/// (`pixel/…`), when it is a bot of the space now.
fn author_bot<'a>(facts: &DevFacts, kwants: &'a [Kwant]) -> Option<&'a Kwant> {
    if let Some(b) = &facts.author.bot {
        if let Some(k) = kwants.iter().find(|k| k.id == b.id) {
            return Some(k);
        }
    }
    let (prefix, _) = facts.head_branch.as_deref()?.split_once('/')?;
    let prefix = prefix.to_lowercase();
    let slug = |n: &str| n.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    kwants.iter().find(|k| !k.name.trim().is_empty() && slug(&k.name) == prefix)
}

fn card_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, l| Arc::strong_count(l) > 1);
    map.entry(key.to_string()).or_default().clone()
}

fn pr_key(row: &DevCardRow) -> String {
    format!("{}\u{1f}{}\u{1f}{}", row.space_id, row.repo, row.ref_)
}

/// Fires the PR's newest card's `dev_card` event, so an open desk refetches. Only the event:
/// [`super::notify_card`]'s ticket sync is about the card, which a note doesn't change.
fn changed_event(state: &AppState, pr: &Pr) {
    let _ = state.event_sender.send(RuntimeEvent::DevCard {
        space_id: pr.current.space_id.clone(),
        id: pr.current.id.clone(),
        kind: pr.current.kind.clone(),
        status: pr.current.status.clone(),
    });
}

fn new_id() -> String {
    format!("ln_{}", uuid::Uuid::new_v4().simple())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------------------------
// Routes

/// `GET /api/dev/cards/:id/changes`
pub async fn changes(State(state): State<AppState>, headers: HeaderMap, Path(card_id): Path<String>) -> Response {
    let pr = match pr_of(&state, &headers, &card_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let notes = match state.db.line_notes_of_pr(&pr.current.space_id, &pr.current.repo, &pr.current.ref_).await {
        Ok(n) => n,
        Err(e) => return internal(e),
    };
    let kwants = state.db.list_kwants(&pr.space).await.unwrap_or_default();
    let author = author_bot(&pr.facts, &kwants);
    let files: Vec<Value> = pr.facts.files.iter().map(|f| file_json(f, notes.iter().filter(|n| n.path == f.path).count())).collect();
    let unread = pr.facts.files.iter().filter(|f| omitted(f) == Some("too_large")).count();
    let cut = pr.facts.cut.as_ref().map(|_| json!({"files_not_read": unread}));
    Json(json!({
        "card_id": pr.current.id,
        "repo": pr.facts.repo,
        "pr": pr.facts.pr,
        "head_sha": pr.current.head_sha,
        "pr_url": pr.facts.links.pr,
        "pr_state": pr_state(&state, &pr).await,
        "author_bot": author.map(|k| json!({"id": k.id, "name": k.name})),
        "author_login": pr.facts.author.login,
        "files": files,
        "cut": cut,
    }))
    .into_response()
}

/// `GET /api/dev/cards/:id/notes`
pub async fn list(State(state): State<AppState>, headers: HeaderMap, Path(card_id): Path<String>) -> Response {
    let pr = match pr_of(&state, &headers, &card_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    // A push the poller saw before this build ran: check it now
    check_sent(&state, &pr).await;
    notes_response(&state, &pr).await
}

async fn notes_response(state: &AppState, pr: &Pr) -> Response {
    let rows = match state.db.line_notes_of_pr(&pr.current.space_id, &pr.current.repo, &pr.current.ref_).await {
        Ok(n) => n,
        Err(e) => return internal(e),
    };
    let files = pr.files();
    let notes: Vec<Value> = rows.iter().map(|r| note_json(r, Some((&pr.current.head_sha, &files)))).collect();
    Json(json!({"notes": notes, "summary": summary(&rows)})).into_response()
}

#[derive(Deserialize)]
pub struct NewNote {
    pub path: String,
    pub start: End,
    pub end: End,
    pub head_sha: String,
    pub text: String,
}

fn check_text(text: &str) -> Result<String, Response> {
    let t = text.trim();
    if t.is_empty() {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "empty", "Write the note first"));
    }
    if t.chars().count() > TEXT_CHARS {
        return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "too_long", "A note is at most 4000 characters"));
    }
    Ok(t.to_string())
}

/// Notes can't be added or sent once the PR is merged or closed.
async fn writable(state: &AppState, pr: &Pr) -> Result<(), Response> {
    match pr_state(state, pr).await {
        "merged" => Err(err(StatusCode::CONFLICT, "pr_merged", "This PR is merged. Notes are read-only.")),
        "closed" => Err(err(StatusCode::CONFLICT, "pr_closed", "This PR is closed. Notes are read-only.")),
        _ => Ok(()),
    }
}

/// `POST /api/dev/cards/:id/notes`
pub async fn create(State(state): State<AppState>, headers: HeaderMap, Path(card_id): Path<String>, body: Option<Json<Value>>) -> Response {
    let Some(Ok(b)) = body.map(|Json(v)| serde_json::from_value::<NewNote>(v)) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "send {path, start, end, head_sha, text}");
    };
    let pr = match pr_of(&state, &headers, &card_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = writable(&state, &pr).await {
        return r;
    }
    let text = match check_text(&b.text) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if b.head_sha.trim() != pr.current.head_sha {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "The PR has a newer push. Load the changes again.", "code": "stale_head", "head_sha": pr.current.head_sha})),
        )
            .into_response();
    }
    let Some(file) = pr.facts.files.iter().find(|f| f.path == b.path) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "not_in_diff", "Those lines aren't in this diff");
    };
    let hunks = parse_patch(file.patch.as_deref().unwrap_or(""));
    let code = match select(&hunks, &b.start, &b.end) {
        Ok(c) => c,
        Err(RangeError::NotInDiff) => return err(StatusCode::UNPROCESSABLE_ENTITY, "not_in_diff", "Those lines aren't in this diff"),
        Err(RangeError::CrossHunk) => return err(StatusCode::UNPROCESSABLE_ENTITY, "cross_hunk", "A note covers lines of one hunk"),
        Err(RangeError::TooBig) => return err(StatusCode::UNPROCESSABLE_ENTITY, "range_too_big", "A note covers at most 200 lines"),
    };
    // Store the ends in diff order, each on the side the person picked for it
    let (first, last) = if is_at(&code[0], &b.start) { (b.start.clone(), b.end.clone()) } else { (b.end.clone(), b.start.clone()) };
    let now = now_ms();
    let row = LineNoteRow {
        id: new_id(),
        space_id: pr.current.space_id.clone(),
        repo: pr.current.repo.clone(),
        ref_: pr.current.ref_.clone(),
        card_id: pr.current.id.clone(),
        path: b.path.clone(),
        start_side: first.side,
        start_line: first.line as i64,
        end_side: last.side,
        end_line: last.line as i64,
        code: serde_json::to_string(&code).unwrap_or_else(|_| "[]".into()),
        text,
        state: DRAFT.into(),
        head_sha: pr.current.head_sha.clone(),
        created_at: now,
        updated_at: now,
        sent_at: None,
        message_id: None,
        checked_sha: None,
        deleted_at: None,
    };
    if let Err(e) = state.db.insert_line_note(&row).await {
        return internal(e);
    }
    let _ = state.db.purge_line_notes().await;
    changed_event(&state, &pr);
    let files = pr.files();
    (StatusCode::CREATED, Json(note_json(&row, Some((&pr.current.head_sha, &files))))).into_response()
}

/// Whether `l` is the line `e` names.
fn is_at(l: &DLine, e: &End) -> bool {
    match e.side.as_str() {
        "new" => l.new == Some(e.line),
        "old" => l.old == Some(e.line),
        _ => false,
    }
}

/// A note of a card's PR, with the PR.
async fn note_of(state: &AppState, headers: &HeaderMap, note_id: &str) -> Result<(LineNoteRow, Pr), Response> {
    super::super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await?;
    let row = match state.db.line_note(note_id.trim()).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, "not_found", "no such note")),
        Err(e) => return Err(internal(e)),
    };
    let pr = pr_of(state, headers, &row.card_id).await?;
    Ok((row, pr))
}

#[derive(Deserialize)]
pub struct EditNote {
    pub text: String,
}

/// `PATCH /api/dev/notes/:note_id`
pub async fn edit(State(state): State<AppState>, headers: HeaderMap, Path(note_id): Path<String>, body: Option<Json<Value>>) -> Response {
    let Some(Ok(b)) = body.map(|Json(v)| serde_json::from_value::<EditNote>(v)) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "send {text}");
    };
    let (row, pr) = match note_of(&state, &headers, &note_id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if row.deleted_at.is_some() {
        return err(StatusCode::NOT_FOUND, "not_found", "no such note");
    }
    let text = match check_text(&b.text) {
        Ok(t) => t,
        Err(r) => return r,
    };
    match state.db.set_line_note_text(&row.id, &text).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::CONFLICT, "not_draft", "Only a draft can be changed"),
        Err(e) => return internal(e),
    }
    changed_event(&state, &pr);
    one_note(&state, &pr, &row.id).await
}

async fn one_note(state: &AppState, pr: &Pr, id: &str) -> Response {
    match state.db.line_note(id).await {
        Ok(Some(r)) => {
            let files = pr.files();
            Json(note_json(&r, Some((&pr.current.head_sha, &files)))).into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "not_found", "no such note"),
        Err(e) => internal(e),
    }
}

/// `DELETE /api/dev/notes/:note_id`: a soft delete, restorable for 60 s.
pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(note_id): Path<String>) -> Response {
    let (row, pr) = match note_of(&state, &headers, &note_id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if row.deleted_at.is_some() {
        return err(StatusCode::NOT_FOUND, "not_found", "no such note");
    }
    match state.db.delete_line_note(&row.id).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::CONFLICT, "not_draft", "Only a draft can be deleted"),
        Err(e) => return internal(e),
    }
    changed_event(&state, &pr);
    Json(json!({"id": row.id, "deleted": true})).into_response()
}

/// `POST /api/dev/notes/:note_id/restore`
pub async fn restore(State(state): State<AppState>, headers: HeaderMap, Path(note_id): Path<String>) -> Response {
    let (row, pr) = match note_of(&state, &headers, &note_id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if row.state != DRAFT {
        return err(StatusCode::CONFLICT, "not_draft", "This note was sent, so there's nothing to restore");
    }
    if row.deleted_at.is_some() {
        match state.db.restore_line_note(&row.id).await {
            Ok(true) => changed_event(&state, &pr),
            Ok(false) => {
                let _ = state.db.purge_line_notes().await;
                return err(StatusCode::NOT_FOUND, "not_found", "That note can't be restored any more");
            }
            Err(e) => return internal(e),
        }
    }
    one_note(&state, &pr, &row.id).await
}

/// `DELETE /api/dev/cards/:id/notes`: soft-deletes every draft of the PR.
pub async fn discard(State(state): State<AppState>, headers: HeaderMap, Path(card_id): Path<String>) -> Response {
    let pr = match pr_of(&state, &headers, &card_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match state.db.discard_line_note_drafts(&pr.current.space_id, &pr.current.repo, &pr.current.ref_).await {
        Ok(n) => {
            if n > 0 {
                changed_event(&state, &pr);
            }
            Json(json!({"discarded": n})).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct SendBody {
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    pub send_id: String,
}

/// The ids a send names, sorted, as stored to compare a repeat.
fn ids_key(ids: &Option<Vec<String>>) -> Option<String> {
    ids.as_ref().map(|v| {
        let mut v: Vec<&String> = v.iter().collect::<HashSet<_>>().into_iter().collect();
        v.sort();
        serde_json::to_string(&v).unwrap_or_default()
    })
}

/// `POST /api/dev/cards/:id/notes/send`
pub async fn send(State(state): State<AppState>, headers: HeaderMap, Path(card_id): Path<String>, body: Option<Json<Value>>) -> Response {
    let Some(Ok(b)) = body.map(|Json(v)| serde_json::from_value::<SendBody>(v)) else {
        return err(StatusCode::BAD_REQUEST, "bad_request", "send {send_id, ids?}");
    };
    let send_id = b.send_id.trim().to_string();
    if send_id.is_empty() || send_id.len() > 100 {
        return err(StatusCode::BAD_REQUEST, "bad_request", "send_id is the client's UUID");
    }
    let pr = match pr_of(&state, &headers, &card_id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let lock = card_lock(&pr_key(&pr.current));
    let _held = lock.lock().await;
    // A repeat: the first result, nothing posted
    match state.db.line_note_send(&send_id).await {
        Ok(Some(s)) if s.created_at >= now_ms() - SEND_ID_HOURS * 3_600_000 => {
            if s.ids != ids_key(&b.ids) || s.space_id != pr.current.space_id || s.repo != pr.current.repo || s.ref_ != pr.current.ref_ {
                return err(StatusCode::CONFLICT, "send_id_reused", "That send_id was used for other notes");
            }
            let first: Value = serde_json::from_str(&s.result).unwrap_or(Value::Null);
            return Json(first).into_response();
        }
        Ok(_) => {}
        Err(e) => return internal(e),
    }
    if let Err(r) = writable(&state, &pr).await {
        return r;
    }
    let kwants = state.db.list_kwants(&pr.space).await.unwrap_or_default();
    let Some(bot) = author_bot(&pr.facts, &kwants).cloned() else {
        return err(StatusCode::CONFLICT, "no_author", "Notes go to bots in this space, and this PR isn't by one");
    };
    let rows = match state.db.line_notes_of_pr(&pr.current.space_id, &pr.current.repo, &pr.current.ref_).await {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let drafts: Vec<&LineNoteRow> = match &b.ids {
        Some(ids) => {
            let want: HashSet<&str> = ids.iter().map(String::as_str).collect();
            let found: Vec<&LineNoteRow> = rows.iter().filter(|r| want.contains(r.id.as_str())).collect();
            if found.len() != want.len() {
                return err(StatusCode::NOT_FOUND, "not_found", "One of those notes doesn't exist on this PR");
            }
            if found.iter().any(|r| r.state != DRAFT) {
                return err(StatusCode::CONFLICT, "not_draft", "One of those notes was already sent");
            }
            found
        }
        None => rows.iter().filter(|r| r.state == DRAFT).collect(),
    };
    if drafts.is_empty() {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "empty", "There are no drafts to send");
    }
    if drafts.len() > SEND_MAX {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "too_many", "Up to 20 notes per send");
    }
    let outgoing: Vec<Outgoing> =
        drafts.iter().map(|r| Outgoing { path: r.path.clone(), code: serde_json::from_str(&r.code).unwrap_or_default(), text: r.text.clone() }).collect();
    let text = message_text(pr.facts.pr, pr.facts.head_branch.as_deref(), &pr.current.head_sha, &outgoing);
    let meta_notes: Vec<Value> = drafts
        .iter()
        .zip(&outgoing)
        .map(|(r, o)| {
            json!({
                "id": r.id, "at": at_text(&o.path, &o.code), "path": o.path,
                "start": End { side: r.start_side.clone(), line: r.start_line as u32 },
                "end": End { side: r.end_side.clone(), line: r.end_line as u32 },
                "code": o.code, "text": o.text,
            })
        })
        .collect();
    let meta = json!({
        "kind": "pr_notes",
        "notes": meta_notes,
        "repo": pr.facts.repo,
        "pr": pr.facts.pr,
        "head_sha": pr.current.head_sha,
        "card_id": pr.current.id,
        "pr_url": pr.facts.links.pr,
        "author_bot_id": bot.id,
    });
    let payload = crate::server::routes::SendChatPayload {
        channel_id: None,
        kwant_id: Some(bot.id.clone()),
        team_id: None,
        thread_id: None,
        prompt: text,
        sender_id: "user".into(),
        attachment_ids: Vec::new(),
        reply_to_message_id: None,
        source: None,
        meta: Some(meta),
        client_message_id: None,
        message_id: None,
    };
    let sent = match crate::server::routes::send_chat_inner(state.clone(), payload, true).await {
        Ok(Json(v)) => v,
        Err(status) => {
            tracing::warn!(card = %pr.current.id, %status, "dev: line notes: the message didn't go");
            return err(StatusCode::BAD_GATEWAY, "send_failed", "The notes didn't send. They're still here.");
        }
    };
    let message_id = sent["user_message_id"].as_str().unwrap_or_default().to_string();
    let sent_at = now_ms();
    let ids: Vec<String> = drafts.iter().map(|r| r.id.clone()).collect();
    if let Err(e) = state.db.mark_line_notes_sent(&ids, &message_id, sent_at).await {
        return internal(e);
    }
    let rows = state.db.line_notes_of_pr(&pr.current.space_id, &pr.current.repo, &pr.current.ref_).await.unwrap_or_default();
    let files = pr.files();
    let notes: Vec<Value> = rows.iter().filter(|r| ids.contains(&r.id)).map(|r| note_json(r, Some((&pr.current.head_sha, &files)))).collect();
    let result = json!({
        "sent": ids.len(),
        "message_id": message_id,
        "kwant_id": bot.id,
        "kwant_name": bot.name,
        "sent_at": Utc.timestamp_millis_opt(sent_at).single(),
        "notes": notes,
    });
    let remembered = LineNoteSendRow {
        send_id,
        space_id: pr.current.space_id.clone(),
        repo: pr.current.repo.clone(),
        ref_: pr.current.ref_.clone(),
        ids: ids_key(&b.ids),
        result: result.to_string(),
        created_at: sent_at,
    };
    if let Err(e) = state.db.insert_line_note_send(&remembered).await {
        tracing::warn!("dev: line notes: send not remembered: {e}");
    }
    changed_event(&state, &pr);
    Json(result).into_response()
}

// ---------------------------------------------------------------------------------------------
// After a push

/// Checks the PR's sent notes against its newest head, when that head is newer than the one they
/// were written on. True when any changed state.
async fn check_sent(state: &AppState, pr: &Pr) -> bool {
    let Ok(rows) = state.db.line_notes_of_pr(&pr.current.space_id, &pr.current.repo, &pr.current.ref_).await else { return false };
    let mut any = false;
    for r in rows.iter().filter(|r| r.state == SENT && r.head_sha != pr.current.head_sha) {
        let Some(c) = changed(r, &pr.facts.files) else { continue };
        let to = if c { RESOLVED } else { OPEN };
        if state.db.check_line_note(&r.id, to, &pr.current.head_sha).await.unwrap_or(false) {
            any = true;
        }
    }
    any
}

/// A new review card for a PR (the poller, after it saved it): the PR's sent notes are checked
/// against it, and the card's event fires when any changed.
pub async fn after_push(state: AppState, row: DevCardRow) {
    if row.kind != REVIEW {
        return;
    }
    let Ok(facts) = serde_json::from_str::<DevFacts>(&row.facts) else { return };
    let Ok(space) = active_space(&state, &row.space_id).await else { return };
    let pr = Pr { space, current: row, facts };
    if check_sent(&state, &pr).await {
        changed_event(&state, &pr);
    }
}

/// The review card payload's `notes: {drafts, total} | null` (g1-api.md, Iris ask 1), for the
/// `rows` of `space` whose JSON is `cards` (same order). Notes are the PR's, whichever of its cards
/// this is; soft-deleted notes don't count.
pub async fn add_counts(state: &AppState, space: &str, rows: &[DevCardRow], cards: &mut [Value]) {
    if !rows.iter().any(|r| r.kind == REVIEW) {
        return;
    }
    let Ok(counts) = state.db.line_note_counts(space).await else { return };
    let by: HashMap<(String, String), (i64, i64)> = counts.into_iter().map(|(repo, r, d, t)| ((repo, r), (d, t))).collect();
    for (row, c) in rows.iter().zip(cards.iter_mut()).filter(|(r, _)| r.kind == REVIEW) {
        let v = by.get(&(row.repo.clone(), row.ref_.clone())).map(|(d, t)| json!({"drafts": d, "total": t})).unwrap_or(Value::Null);
        if let Some(m) = c.as_object_mut() {
            m.insert("notes".into(), v);
        }
    }
}

/// The review card payload's `changes: {files, additions, deletions} | null`: from the files the
/// review read for its head; null when it read none.
pub fn changes_stat(files: &[FileDiff]) -> Value {
    if files.is_empty() {
        return Value::Null;
    }
    json!({
        "files": files.len(),
        "additions": files.iter().map(|f| f.additions).sum::<u64>(),
        "deletions": files.iter().map(|f| f.deletions).sum::<u64>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "@@ -38,6 +38,7 @@ export function InboxEmpty\n   const x = 1;\n-  return (\n+  return (\n+    <section aria-label=\"Inbox\">\n   <div>\n   </div>\n@@ -80,3 +81,3 @@\n a\n-b\n+B\n c\n\\ No newline at end of file";

    fn new(line: u32) -> End {
        End { side: "new".into(), line }
    }
    fn old(line: u32) -> End {
        End { side: "old".into(), line }
    }

    #[test]
    fn a_patch_parses_into_numbered_lines() {
        let h = parse_patch(PATCH);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].header, "@@ -38,6 +38,7 @@ export function InboxEmpty");
        let l = &h[0].lines;
        assert_eq!(l[0], DLine { kind: "ctx".into(), old: Some(38), new: Some(38), text: "  const x = 1;".into() });
        assert_eq!(l[1], DLine { kind: "del".into(), old: Some(39), new: None, text: "  return (".into() });
        assert_eq!(l[2], DLine { kind: "add".into(), old: None, new: Some(39), text: "  return (".into() });
        assert_eq!(l[3].new, Some(40));
        assert_eq!((l[4].old, l[4].new), (Some(40), Some(41)));
        assert_eq!(h[1].lines.len(), 4, "the no-newline marker isn't a line");
        assert_eq!((h[1].lines[1].old, h[1].lines[2].new), (Some(81), Some(82)));
    }

    #[test]
    fn a_range_is_every_line_between_its_ends_in_one_hunk() {
        let h = parse_patch(PATCH);
        // del 39 to add 40: a mixed range
        let r = select(&h, &old(39), &new(40)).unwrap();
        assert_eq!(r.iter().map(|l| l.kind.as_str()).collect::<Vec<_>>(), ["del", "add", "add"]);
        // dragged upwards: same lines
        assert_eq!(select(&h, &new(40), &old(39)).unwrap(), r);
        // one line
        assert_eq!(select(&h, &new(38), &new(38)).unwrap().len(), 1);
        assert_eq!(select(&h, &new(38), &new(82)), Err(RangeError::CrossHunk));
        assert_eq!(select(&h, &new(500), &new(500)), Err(RangeError::NotInDiff));
        // a removed line has no new number
        assert_eq!(select(&h, &new(39), &new(39)).unwrap()[0].kind, "add");
        assert_eq!(select(&h, &old(39), &old(39)).unwrap()[0].kind, "del");
    }

    #[test]
    fn a_range_over_200_lines_is_refused() {
        let mut p = String::from("@@ -1,0 +1,201 @@\n");
        for i in 0..201 {
            p.push_str(&format!("+l{i}\n"));
        }
        let h = parse_patch(&p);
        assert_eq!(select(&h, &new(1), &new(201)), Err(RangeError::TooBig));
        assert_eq!(select(&h, &new(1), &new(200)).unwrap().len(), 200);
    }

    #[test]
    fn noted_lines_are_found_in_a_newer_push() {
        let h = parse_patch(PATCH);
        let code = select(&h, &new(40), &new(41)).unwrap();
        // Two lines added above them: they moved down by 2
        let moved = PATCH.replace("@@ -38,6 +38,7 @@ export function InboxEmpty\n", "@@ -38,6 +38,9 @@ export function InboxEmpty\n+// one\n+// two\n");
        let (s, e) = locate(&parse_patch(&moved), &code, "new", "new", 40).unwrap();
        assert_eq!((s, e), (new(42), new(43)));
        // One of them edited: gone
        let edited = PATCH.replace("aria-label=\"Inbox\"", "aria-labelledby=\"h\"");
        assert_eq!(locate(&parse_patch(&edited), &code, "new", "new", 40), None);
    }

    #[test]
    fn of_two_copies_the_nearest_is_picked() {
        let p = "@@ -1,0 +1,2 @@\n+x\n+y\n@@ -50,0 +50,2 @@\n+x\n+y\n";
        let code = vec![DLine { kind: "add".into(), old: None, new: Some(1), text: "x".into() }];
        assert_eq!(locate(&parse_patch(p), &code, "new", "new", 48).unwrap().0, new(50));
        assert_eq!(locate(&parse_patch(p), &code, "new", "new", 3).unwrap().0, new(1));
    }

    #[test]
    fn at_names_new_lines_else_old() {
        let h = parse_patch(PATCH);
        assert_eq!(at_text("a.tsx", &select(&h, &old(39), &new(40)).unwrap()), "a.tsx:39-40");
        assert_eq!(at_text("a.tsx", &select(&h, &old(39), &old(39)).unwrap()), "a.tsx:39");
        assert_eq!(at_text("a.tsx", &select(&h, &new(41), &new(41)).unwrap()), "a.tsx:41");
    }

    fn file(path: &str, adds: u64, dels: u64, patch: Option<&str>, status: &str) -> FileDiff {
        FileDiff { path: path.into(), status: status.into(), additions: adds, deletions: dels, patch: patch.map(str::to_string), previous_path: None }
    }

    #[test]
    fn omitted_files_say_why() {
        assert_eq!(omitted(&file("a.png", 0, 0, None, "added")), Some("binary"));
        assert_eq!(omitted(&file("a.rs", 900, 2, None, "modified")), Some("too_large"));
        assert_eq!(omitted(&file("a.rs", 0, 0, None, "renamed")), None);
        assert_eq!(omitted(&file("a.rs", 1, 0, Some("@@ -1,0 +1 @@\n+a"), "modified")), None);
        assert!(is_big(&file("x/Cargo.lock", 3, 1, Some(""), "modified")));
        assert!(is_big(&file("a.rs", 300, 101, Some(""), "modified")));
        assert!(!is_big(&file("a.rs", 300, 100, Some(""), "modified")));
    }

    #[test]
    fn the_message_is_mira_s_block() {
        let h = parse_patch(PATCH);
        let notes = vec![
            Outgoing { path: "src/a.tsx".into(), code: select(&h, &new(40), &new(41)).unwrap(), text: "Use aria-labelledby.".into() },
            Outgoing { path: "src/a.tsx".into(), code: select(&h, &old(39), &old(39)).unwrap(), text: "Why remove this?\nIt was fine.".into() },
        ];
        let m = message_text(Some(212), Some("pixel/inbox-empty"), "9f3c2e1abcdef", &notes);
        assert_eq!(
            m,
            "2 notes on #212 pixel/inbox-empty (at 9f3c2e1), from the review:\n\
             \n1. src/a.tsx:40-41\n   40 |     <section aria-label=\"Inbox\">\n   41 |   <div>\n   → Use aria-labelledby.\n\
             \n2. src/a.tsx:39\n   39 (removed) |   return (\n   → Why remove this?\n     It was fine.\n\
             \nPush to the same branch when done; kwant marks each note resolved when its lines change."
        );
        let one = message_text(None, None, "abc", &notes[..1]);
        assert!(one.starts_with("1 note on the branch (at abc), from the review:\n"), "{one}");
    }

    #[test]
    fn long_lines_and_long_ranges_are_cut() {
        let long = "x".repeat(200);
        let code: Vec<DLine> = (1..=20).map(|i| DLine { kind: "add".into(), old: None, new: Some(i), text: if i == 1 { long.clone() } else { format!("l{i}") } }).collect();
        let q = quoted(&code);
        assert_eq!(q.len(), 6 + 1 + 3);
        assert_eq!(q[0], format!("   1 | {}…", "x".repeat(160)));
        assert_eq!(q[6], "   …");
        assert_eq!(q[7], "   18 | l18");
        assert_eq!(q[9], "   20 | l20");
        assert_eq!(quoted(&code[..12]).len(), 12);
    }

    fn row(state: &str, head: &str, code: &[DLine], start: u32, end: u32) -> LineNoteRow {
        LineNoteRow {
            id: "ln_1".into(),
            space_id: "s".into(),
            repo: "r".into(),
            ref_: "pr:1".into(),
            card_id: "c".into(),
            path: "src/a.tsx".into(),
            start_side: "new".into(),
            start_line: start as i64,
            end_side: "new".into(),
            end_line: end as i64,
            code: serde_json::to_string(code).unwrap(),
            text: "t".into(),
            state: state.into(),
            head_sha: head.into(),
            created_at: 1,
            updated_at: 1,
            sent_at: None,
            message_id: None,
            checked_sha: None,
            deleted_at: None,
        }
    }

    #[test]
    fn a_push_changes_a_note_when_any_of_its_lines_changed() {
        let h = parse_patch(PATCH);
        let code = select(&h, &new(40), &new(41)).unwrap();
        let r = row(SENT, "aaa", &code, 40, 41);
        let same = [file("src/a.tsx", 2, 1, Some(PATCH), "modified")];
        assert_eq!(changed(&r, &same), Some(false));
        let edited = PATCH.replace("<div>", "<main>");
        assert_eq!(changed(&r, &[file("src/a.tsx", 2, 1, Some(&edited), "modified")]), Some(true));
        // The file's diff wasn't read for that head: can't tell yet
        assert_eq!(changed(&r, &[file("src/a.tsx", 2000, 1, None, "modified")]), None);
        // The file left the PR: the change is gone
        assert_eq!(changed(&r, &[file("src/b.tsx", 1, 0, Some("@@ -1,0 +1 @@\n+a"), "modified")]), Some(true));
    }

    #[test]
    fn a_note_says_where_it_is_in_the_newest_head() {
        let h = parse_patch(PATCH);
        let code = select(&h, &new(40), &new(41)).unwrap();
        let r = row(DRAFT, "aaa", &code, 40, 41);
        let files: HashMap<String, Vec<DHunk>> = [("src/a.tsx".to_string(), h.clone())].into();
        let v = note_json(&r, Some(("aaa", &files)));
        assert_eq!(v["at_head"]["start"], json!({"side": "new", "line": 40}));
        assert_eq!(v["seen_sha"], Value::Null);
        let moved = PATCH.replace("@@ -38,6 +38,7 @@ export function InboxEmpty\n", "@@ -38,6 +38,8 @@ export function InboxEmpty\n+// one\n");
        let files: HashMap<String, Vec<DHunk>> = [("src/a.tsx".to_string(), parse_patch(&moved))].into();
        let v = note_json(&r, Some(("bbb", &files)));
        assert_eq!(v["at_head"], json!({"start": {"side": "new", "line": 41}, "end": {"side": "new", "line": 42}}));
        assert_eq!(v["seen_sha"], "bbb");
        let files: HashMap<String, Vec<DHunk>> = HashMap::new();
        let v = note_json(&r, Some(("bbb", &files)));
        assert_eq!(v["at_head"], Value::Null);
        assert_eq!(v["seen_sha"], "bbb");
        assert_eq!(v["code"][0]["kind"], "add");
    }

    #[test]
    fn the_summary_counts_the_newest_check() {
        let mut a = row(RESOLVED, "a", &[], 1, 1);
        a.checked_sha = Some("b".into());
        a.updated_at = 5;
        let mut b = row(OPEN, "a", &[], 1, 1);
        b.checked_sha = Some("b".into());
        b.updated_at = 6;
        let c = row(DRAFT, "b", &[], 1, 1);
        assert_eq!(summary(&[a, b, c.clone()]).unwrap(), json!({"total": 2, "changed": 1, "open": 1, "sha": "b"}));
        assert_eq!(summary(&[c]), None);
    }
}
