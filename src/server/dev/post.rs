//! Posting a review card to its pull request (0.7.1, plan/post-to-github.md P1–P8). The person
//! presses Post; kwant sends one GitHub review with event COMMENT, pinned to the card's head. It
//! never approves or requests changes, and nothing is posted on its own.
//!
//! What goes out: the risk level and reasons, the intent check, the test-tamper flags and the
//! security tiles in the body; one inline comment per finding that isn't dismissed and whose line
//! is inside the diff; everything else in the body. A gitleaks finding is only "Possible secret in
//! {file}:{line}". The body and every comment go through [`redact`] before they leave.
//!
//! [`post_review`] is the forge step (a forge trait can wrap it later, amendment of 21:50);
//! [`post`] is the route around it: guard, lock, record, notify.

use super::facts::DevFacts;
use super::routes::{card_json, REFUSAL};
use super::{notify_card, repo};
use crate::db::dev_mode::{DevCardRow, REVIEW, STALE};
use crate::server::missions::autofix::redact;
use crate::server::spaces::active_space;
use crate::server::AppState;
use crate::zombie::github::{GhError, GithubSource, PostError as GhPostError, ReviewComment, ReviewPost};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The only forge kwant posts to in 0.7.1.
pub const FORGE: &str = "github";

/// GitHub takes a review body up to 65,536 characters; kwant stays well under it.
const BODY_CHARS: usize = 60_000;
/// One inline comment's text at most.
const COMMENT_CHARS: usize = 4_000;
/// Most inline comments in one review; the rest go in the body.
const MAX_INLINE: usize = 50;

/// `posted` on the card (contract P1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posted {
    pub at: DateTime<Utc>,
    pub login: Option<String>,
    pub url: Option<String>,
    pub comments: usize,
    pub forge: String,
}

/// `post_error.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The token can read the repo but not post (403, 404 or a permission error, P7).
    ReadOnly,
    /// The PR head moved past the card (P5): nothing was posted.
    Stale,
    Failed,
}

/// `post_error` on the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostError {
    pub kind: ErrorKind,
    pub message: String,
}

impl PostError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        PostError { kind, message: message.into() }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Failed, message)
    }
}

pub const READ_ONLY: &str = "GitHub refused the review: this token can read the repo but not post to it. \
Give it Pull requests: Read and write (or the classic repo scope), then paste it again in Loose ends.";
pub const STALE_MESSAGE: &str = "This PR has new commits, so kwant didn't post this review. A new review is on its way.";

// ---------------------------------------------------------------------------------------------
// Building the review (pure)

/// New-side line numbers each file's diff hunks cover: the lines GitHub takes a `side: RIGHT`
/// comment on (added and context lines). A file with no patch (binary, too large, unread) has
/// none, so its findings go in the body.
pub fn diff_lines(facts: &DevFacts) -> HashMap<String, BTreeSet<u64>> {
    let mut out: HashMap<String, BTreeSet<u64>> = HashMap::new();
    for f in &facts.files {
        let Some(patch) = &f.patch else { continue };
        let lines = out.entry(f.path.clone()).or_default();
        let mut new_line: Option<u64> = None;
        for line in patch.split('\n') {
            if let Some(start) = super::scan::hunk_new_start(line) {
                new_line = Some(start);
                continue;
            }
            let Some(n) = new_line else { continue };
            match line.as_bytes().first() {
                Some(b'-') | Some(b'\\') => {}
                // `+` or a context line; an empty line is a context line whose space was trimmed
                _ => {
                    lines.insert(n);
                    new_line = Some(n + 1);
                }
            }
        }
    }
    out
}

/// A review ready to send: the body (redacted, without the marker) and the inline comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub body: String,
    pub comments: Vec<ReviewComment>,
}

/// The findings a review posts: every one not dismissed (open, or sent to the author bot).
pub fn postable(card: &Value) -> Vec<&Value> {
    card["findings"].as_array().map_or(Vec::new(), |fs| fs.iter().filter(|f| f["state"].as_str() != Some("dismissed")).collect())
}

fn is_secret(f: &Value) -> bool {
    f["source"].as_str() == Some("gitleaks")
}

fn location(f: &Value) -> String {
    let file = f["file"].as_str().unwrap_or("").trim();
    match f["line"].as_u64() {
        Some(l) if !file.is_empty() => format!("{file}:{l}"),
        _ => file.to_string(),
    }
}

fn severity(f: &Value) -> String {
    let s = f["severity"].as_str().unwrap_or("info");
    let mut c = s.chars();
    c.next().map_or(String::new(), |first| first.to_uppercase().chain(c).collect())
}

/// Text of one finding. A secret is only where it is (P4): no title, detail or fix, since gitleaks
/// rule names and model text may quote the value.
fn finding_text(f: &Value, with_location: bool) -> String {
    if is_secret(f) {
        return format!("Possible secret in {}", location(f));
    }
    let title = f["title"].as_str().unwrap_or("").trim();
    let mut out = format!("**{}** · {title}", severity(f));
    if with_location && !location(f).is_empty() {
        out.push_str(&format!(" (`{}`)", location(f)));
    }
    for (label, key) in [("", "detail"), ("Fix: ", "fix")] {
        let t = f[key].as_str().unwrap_or("").trim();
        if !t.is_empty() {
            out.push_str(&format!("\n\n{label}{t}"));
        }
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn tile_text(t: &Value, what: &str) -> String {
    let count = t["count"].as_u64().unwrap_or(0);
    match t["state"].as_str().unwrap_or("not_run") {
        "pass" => "nothing found".into(),
        "hit" => {
            let missing = t["missing"].as_u64().unwrap_or(0);
            let mut s = format!("{count} {what}{}", if count == 1 { "" } else { "s" });
            if missing > 0 {
                s.push_str(&format!(", {missing} not on the registry"));
            }
            s
        }
        "later" => "not checked yet".into(),
        _ => match t["reason"].as_str() {
            Some(r) if !r.trim().is_empty() => format!("not run ({})", r.trim()),
            _ => "not run".into(),
        },
    }
}

/// Who wrote the review, for the last line.
fn signature(card: &Value) -> String {
    let r = &card["reviewer"];
    match r["name"].as_str() {
        Some(name) if !name.trim().is_empty() => {
            let engine = r["engine"].as_str().filter(|e| !e.trim().is_empty()).unwrap_or("auto");
            format!("Reviewed by {} ({engine}) in kwant", name.trim())
        }
        _ => "Reviewed by kwant's rule checks (no reviewer bot) in kwant".into(),
    }
}

/// The review for `card` (P2, P3, P4). With `inline` false every finding goes in the body (the
/// 422 retry). Redacted throughout.
pub fn build(card: &Value, facts: &DevFacts, inline: bool) -> Built {
    let lines = diff_lines(facts);
    let mut comments = Vec::new();
    let mut in_body: Vec<&Value> = Vec::new();
    for f in postable(card) {
        let file = f["file"].as_str().unwrap_or("");
        let at = f["line"].as_u64().filter(|l| *l > 0 && lines.get(file).is_some_and(|ls| ls.contains(l)));
        match at {
            Some(line) if inline && comments.len() < MAX_INLINE => comments.push(ReviewComment {
                path: file.to_string(),
                line,
                side: "RIGHT",
                body: clip(&redact(&format!("{}\n\n<sub>kwant review</sub>", finding_text(f, false))), COMMENT_CHARS),
            }),
            _ => in_body.push(f),
        }
    }

    let mut b = String::new();
    let level = card["risk"]["level"].as_str().unwrap_or("unknown");
    b.push_str(&format!("### kwant review · Risk: {}\n", severity(&json!({"severity": level}))));
    for r in card["risk"]["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        b.push_str(&format!("- {r}\n"));
    }

    b.push_str("\n**Intent check**");
    let intent = &card["intent"];
    match intent["source"].as_str() {
        Some("task") => b.push_str(" (against the kwant task this PR came from)\n"),
        _ => b.push_str(" (against the PR's own title and description)\n"),
    }
    if let Some(asked) = intent["asked"].as_str().filter(|a| !a.trim().is_empty()) {
        b.push_str(&format!("Asked: {}\n", asked.trim()));
    }
    let rows: Vec<&Value> = intent["rows"].as_array().map_or(Vec::new(), |r| r.iter().collect());
    if rows.is_empty() {
        b.push_str("- No intent check: the reviewer didn't answer.\n");
    }
    for r in rows {
        let label = match r["state"].as_str() {
            Some("ok") => "Done",
            Some("missing") => "Missing",
            Some("extra") => "Not asked for",
            _ => "Note",
        };
        b.push_str(&format!("- {label}: {}\n", r["text"].as_str().unwrap_or("").trim()));
    }

    b.push_str("\n**Test tampering**\n");
    let flags: Vec<&Value> = card["tamper"]["flags"].as_array().map_or(Vec::new(), |f| f.iter().collect());
    if flags.is_empty() {
        b.push_str("- None found.\n");
    }
    for f in flags {
        b.push_str(&format!("- `{}:{}` {}\n", f["file"].as_str().unwrap_or(""), f["line"].as_u64().unwrap_or(0), f["why"].as_str().unwrap_or("").trim()));
        let excerpt = f["excerpt"].as_str().unwrap_or("").trim_end();
        if !excerpt.is_empty() {
            let fence = if excerpt.contains("```") { "~~~~" } else { "```" };
            b.push_str(&format!("  {fence}diff\n"));
            for l in excerpt.lines() {
                b.push_str(&format!("  {l}\n"));
            }
            b.push_str(&format!("  {fence}\n"));
        }
    }

    let sec = &card["security"];
    b.push_str("\n**Security**\n");
    b.push_str(&format!("- Secrets (gitleaks): {}\n", tile_text(&sec["secrets"], "possible secret")));
    b.push_str(&format!("- New packages: {}\n", tile_text(&sec["packages"], "package to check")));
    b.push_str(&format!("- Known vulnerabilities (OSV): {}\n", tile_text(&sec["osv"], "")));
    b.push_str(&format!("- Static analysis (Semgrep): {}\n", tile_text(&sec["semgrep"], "")));

    if !in_body.is_empty() {
        b.push_str(if comments.is_empty() { "\n**Findings**\n" } else { "\n**Other findings** (not on a line of this diff)\n" });
        for f in in_body {
            let text = finding_text(f, true).replace("\n\n", "\n  ");
            b.push_str(&format!("- {text}\n"));
        }
    }
    b.push_str(&format!("\n---\n{}\n", signature(card)));

    Built { body: clip(&redact(&b), BODY_CHARS), comments }
}

/// The hidden line that marks a review as this card's, so a post whose answer was lost isn't
/// sent twice. Added after redaction: it is kwant's own card id, which redaction would mask.
pub fn marker(card_id: &str) -> String {
    format!("<!-- kwant review card {card_id} -->")
}

// ---------------------------------------------------------------------------------------------
// The forge step

/// Whether GitHub's 422 is about comment lines (then the retry puts everything in the body).
fn about_lines(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    ["line", "path", "position", "diff", "hunk"].iter().any(|w| m.contains(w))
}

fn gh_error(e: GhError) -> PostError {
    match e {
        GhError::Forbidden | GhError::NotFound => PostError::new(ErrorKind::ReadOnly, READ_ONLY),
        e => PostError::failed(e.to_string()),
    }
}

/// The client whose connection can read PR `number`, and the PR. Same order as Dev mode's reads.
async fn client_for_pull(state: &AppState, repo_name: &str, number: u64) -> Result<(GithubSource, crate::zombie::github::GhPull), PostError> {
    let list = repo::clients(state).await.map_err(PostError::failed)?;
    let mut last = GhError::NotFound;
    for (_, c) in list {
        match c.dev_pull(repo_name, number).await {
            Ok(p) => return Ok((c, p)),
            Err(e @ GhError::Limited { .. }) => return Err(gh_error(e)),
            Err(e) => last = e,
        }
    }
    Err(gh_error(last))
}

/// Posts `card` (a review card row) to its PR: P1–P5, P7. Doesn't touch the database.
pub async fn post_review(state: &AppState, row: &DevCardRow) -> Result<Posted, PostError> {
    if row.status == STALE {
        return Err(PostError::new(ErrorKind::Stale, STALE_MESSAGE));
    }
    let facts: DevFacts = serde_json::from_str(&row.facts).map_err(|_| PostError::failed("kwant could not read this card's facts"))?;
    let card: Value = row.card.as_deref().and_then(|c| serde_json::from_str(c).ok()).ok_or_else(|| PostError::failed("this review isn't written yet"))?;
    let Some(number) = facts.pr else {
        return Err(PostError::failed("this card has no pull request"));
    };
    let (client, pull) = client_for_pull(state, &facts.repo, number).await?;
    if pull.head.sha != row.head_sha {
        return Err(PostError::new(ErrorKind::Stale, STALE_MESSAGE));
    }
    if pull.state != "open" {
        return Err(PostError::failed("this pull request is closed"));
    }

    // A post whose answer was lost (a timeout after GitHub took it): find it instead of posting twice
    let mark = marker(&row.id);
    if let Ok(reviews) = client.dev_reviews(&facts.repo, number).await {
        if let Some(r) = reviews.iter().find(|r| r.body.as_deref().is_some_and(|b| b.contains(&mark)) && r.commit_id.as_deref() == Some(row.head_sha.as_str())) {
            return Ok(Posted { at: Utc::now(), login: r.user.as_ref().map(|u| u.login.clone()), url: r.html_url.clone(), comments: 0, forge: FORGE.into() });
        }
    }

    let send = |b: Built| ReviewPost { commit_id: row.head_sha.clone(), body: format!("{}\n\n{mark}", b.body), event: "COMMENT", comments: b.comments };
    let first = build(&card, &facts, true);
    let mut sent = first.comments.len();
    let mut answer = client.dev_post_review(&facts.repo, number, &send(first)).await;
    if let Err(GhPostError::Rejected(m)) = &answer {
        if sent > 0 && about_lines(m) {
            tracing::info!(card = %row.id, "dev: GitHub refused the inline comments ({m}); posting them in the body");
            sent = 0;
            answer = client.dev_post_review(&facts.repo, number, &send(build(&card, &facts, false))).await;
        }
    }
    match answer {
        Ok(r) => {
            let login = r.user.as_ref().map(|u| u.login.clone());
            if let Some(l) = &login {
                remember_login(&client, l);
            }
            Ok(Posted { at: Utc::now(), login, url: r.html_url, comments: sent, forge: FORGE.into() })
        }
        Err(GhPostError::Rejected(m)) => Err(PostError::failed(format!("GitHub didn't take the review: {m}"))),
        // A permission error can also come back as a 422 on some GitHub Enterprise versions; the
        // 403 and 404 cases are the documented ones
        Err(GhPostError::Gh(e)) => Err(gh_error(e)),
    }
}

// ---------------------------------------------------------------------------------------------
// The route

/// One post at a time per card (P6): a double click waits, then finds `posted`.
fn card_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    // Drop locks nobody holds, so the map stays small
    map.retain(|_, l| Arc::strong_count(l) > 1);
    map.entry(id.to_string()).or_default().clone()
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!("dev: post: {e}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Dev mode could not read or save that")
}

/// `POST /api/dev/cards/:id/post`: the card, 200, with `posted` or `post_error` set. 404 for no
/// such card, 400 for a card that isn't a review, 409 for a review not written yet.
pub async fn post(State(state): State<AppState>, headers: HeaderMap, Path(card_id): Path<String>) -> Response {
    if let Err(r) = super::super::mcp_suggest::refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let card_id = card_id.trim().to_string();
    let read = |state: AppState, id: String| async move { state.db.dev_card(&id).await };
    let row = match read(state.clone(), card_id.clone()).await {
        Ok(Some(r)) => r,
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such card"),
        Err(e) => return internal(e),
    };
    if let Err(r) = active_space(&state, &row.space_id).await {
        return r;
    }
    if row.kind != REVIEW {
        return error(StatusCode::BAD_REQUEST, "only a review card can be posted");
    }
    if row.posted.is_some() {
        return Json(card_json(&row)).into_response();
    }
    let lock = card_lock(&row.id);
    let _held = lock.lock().await;
    // Read again under the lock: the press before this one may have posted
    let row = match read(state.clone(), row.id.clone()).await {
        Ok(Some(r)) => r,
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such card"),
        Err(e) => return internal(e),
    };
    if row.posted.is_some() {
        return Json(card_json(&row)).into_response();
    }
    if row.card.is_none() {
        return error(StatusCode::CONFLICT, "this review isn't written yet");
    }
    let saved = match post_review(&state, &row).await {
        Ok(p) => state.db.set_dev_card_posted(&row.id, &json!(p).to_string()).await.map(|_| ()),
        Err(e) => {
            tracing::info!(card = %row.id, kind = ?e.kind, "dev: review not posted: {}", e.message);
            state.db.set_dev_card_post_error(&row.id, Some(&json!(e).to_string())).await
        }
    };
    if let Err(e) = saved {
        return internal(e);
    }
    let row = match read(state.clone(), row.id.clone()).await {
        Ok(Some(r)) => r,
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such card"),
        Err(e) => return internal(e),
    };
    notify_card(&state, &row);
    Json(card_json(&row)).into_response()
}

// ---------------------------------------------------------------------------------------------
// Who posts (`forge_login`, P7)

/// How long a connection's login is kept.
const LOGIN_TTL: Duration = Duration::from_secs(15 * 60);
/// How long a failed read is kept, so the settings page doesn't ask GitHub on every open.
const LOGIN_MISS_TTL: Duration = Duration::from_secs(60);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5);

type Logins = Mutex<HashMap<(String, String), (Option<String>, Instant)>>;

fn logins() -> &'static Logins {
    static L: OnceLock<Logins> = OnceLock::new();
    L.get_or_init(Default::default)
}

fn login_key(c: &GithubSource) -> (String, String) {
    (c.connection_id().to_string(), c.api_base())
}

fn remember_login(c: &GithubSource, login: &str) {
    logins().lock().unwrap_or_else(|e| e.into_inner()).insert(login_key(c), (Some(login.to_string()), Instant::now()));
}

/// The account the first GitHub connection posts as (`GET /user`), cached per connection. None
/// with no connection, a locked vault, or a token GitHub refuses.
pub async fn forge_login(state: &AppState) -> Option<String> {
    let client = repo::clients(state).await.ok()?.into_iter().next()?.1;
    let key = login_key(&client);
    if let Some((login, at)) = logins().lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned() {
        let ttl = if login.is_some() { LOGIN_TTL } else { LOGIN_MISS_TTL };
        if at.elapsed() < ttl {
            return login;
        }
    }
    let login = tokio::time::timeout(LOGIN_TIMEOUT, client.dev_login()).await.ok().and_then(Result::ok);
    logins().lock().unwrap_or_else(|e| e.into_inner()).insert(key, (login.clone(), Instant::now()));
    login
}

/// `forge` and `forge_login` for the settings reply (amendment of 21:50): `github` when the space
/// watches a repo and a GitHub connection is set up, else null (no Post button).
pub async fn forge_json(state: &AppState, has_repo: bool) -> (Value, Value) {
    if !has_repo || repo::clients(state).await.is_err() {
        return (Value::Null, Value::Null);
    }
    (json!(FORGE), json!(forge_login(state).await))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::dev::facts::FileDiff;

    /// A fake AWS-style key, built at run time so no scanner reads it as a real one.
    fn fake_akia() -> String {
        format!("{}{}{}", "AK", "IA", "Q7XW3MZP2KD9RT4B")
    }

    fn facts() -> DevFacts {
        let mut f: DevFacts =
            serde_json::from_str(&std::fs::read_to_string(format!("{}/src/server/dev/fixtures/pr301.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
        f.files = vec![
            FileDiff {
                path: "src/a.rs".into(),
                status: "modified".into(),
                additions: 3,
                deletions: 1,
                // new side: 10 (ctx), 11 (+), 12 (+), 13 (ctx); then 40..=41
                patch: Some("@@ -10,3 +10,4 @@ fn a()\n ctx\n-old\n+new\n+newer\n ctx\n@@ -39,1 +40,2 @@\n+x\n \n\\ No newline at end of file".into()),
                previous_path: None,
            },
            FileDiff { path: "img.png".into(), status: "added".into(), additions: 0, deletions: 0, patch: None, previous_path: None },
            FileDiff { path: "gone.rs".into(), status: "removed".into(), additions: 0, deletions: 2, patch: Some("@@ -1,2 +0,0 @@\n-a\n-b".into()), previous_path: None },
        ];
        f
    }

    fn finding(id: &str, source: &str, file: &str, line: Option<u64>, state: &str) -> Value {
        json!({"id": id, "severity": "high", "title": format!("Title {id}"), "file": file, "line": line, "detail": format!("Detail {id}"),
               "fix": format!("Fix {id}"), "source": source, "state": state, "sent_at": null, "dismissed": null})
    }

    fn card(findings: Vec<Value>) -> Value {
        json!({
            "kind": "review",
            "reviewer": {"id": "kw_1", "name": "Vega", "engine": "codex", "model": "gpt"},
            "risk": {"level": "high", "reasons": ["Touches auth", "A test was switched off"]},
            "intent": {"source": "task", "asked": "Add a sidebar", "rows": [{"state": "ok", "text": "Sidebar added"}, {"state": "missing", "text": "No mobile layout"}, {"state": "extra", "text": "Changed the logo"}], "none": false},
            "tamper": {"flags": [{"kind": "skip", "file": "tests/a.rs", "line": 4, "excerpt": "-#[test]\n+#[ignore]", "why": "A test was switched off"}], "unread": []},
            "security": {"secrets": {"state": "hit", "count": 1, "missing": 0}, "packages": {"state": "pass", "count": 0, "missing": 0}, "osv": {"state": "later"}, "semgrep": {"state": "later"}},
            "findings": findings,
        })
    }

    #[test]
    fn hunks_give_the_new_side_lines() {
        let l = diff_lines(&facts());
        assert_eq!(l["src/a.rs"].iter().copied().collect::<Vec<_>>(), vec![10, 11, 12, 13, 40, 41]);
        assert!(!l.contains_key("img.png"));
        assert!(l["gone.rs"].is_empty());
    }

    #[test]
    fn findings_inside_the_diff_are_inline_and_the_rest_go_in_the_body() {
        let c = card(vec![
            finding("f1", "review", "src/a.rs", Some(11), "open"),
            finding("f2", "review", "src/a.rs", Some(25), "open"),
            finding("f3", "review", "img.png", Some(1), "open"),
            finding("f4", "review", "src/a.rs", Some(12), "dismissed"),
            finding("f5", "review", "gone.rs", Some(1), "sent"),
            finding("f6", "review", "src/a.rs", None, "open"),
        ]);
        let b = build(&c, &facts(), true);
        assert_eq!(b.comments.len(), 1, "{b:?}");
        assert_eq!((b.comments[0].path.as_str(), b.comments[0].line, b.comments[0].side), ("src/a.rs", 11, "RIGHT"));
        assert!(b.comments[0].body.contains("Title f1") && b.comments[0].body.contains("Fix f1"));
        for outside in ["Title f2", "`src/a.rs:25`", "Title f3", "Title f5", "Title f6"] {
            assert!(b.body.contains(outside), "{outside} missing from:\n{}", b.body);
        }
        assert!(!b.body.contains("Title f4") && !b.body.contains("Title f1"), "{}", b.body);
        assert!(b.body.contains("Other findings"));
    }

    #[test]
    fn the_body_has_risk_intent_tamper_security_and_the_signature() {
        let b = build(&card(vec![]), &facts(), true);
        for want in [
            "Risk: High",
            "- Touches auth",
            "- Done: Sidebar added",
            "- Missing: No mobile layout",
            "- Not asked for: Changed the logo",
            "`tests/a.rs:4` A test was switched off",
            "+#[ignore]",
            "Secrets (gitleaks): 1 possible secret",
            "New packages: nothing found",
            "Known vulnerabilities (OSV): not checked yet",
        ] {
            assert!(b.body.contains(want), "{want} missing from:\n{}", b.body);
        }
        assert!(b.body.trim_end().ends_with("Reviewed by Vega (codex) in kwant"), "{}", b.body);
        assert!(b.comments.is_empty());
    }

    #[test]
    fn without_inline_everything_goes_in_the_body() {
        let c = card(vec![finding("f1", "review", "src/a.rs", Some(11), "open"), finding("f2", "review", "src/a.rs", Some(40), "open")]);
        let b = build(&c, &facts(), false);
        assert!(b.comments.is_empty());
        assert!(b.body.contains("Title f1") && b.body.contains("Title f2") && b.body.contains("**Findings**"), "{}", b.body);
    }

    #[test]
    fn a_secret_is_only_where_it_is_and_nothing_secret_leaves() {
        let key = fake_akia();
        let mut leak = finding("s1", "gitleaks", "src/a.rs", Some(12), "open");
        leak["title"] = json!(format!("Possible secret (aws-access-token {key})"));
        leak["detail"] = json!(format!("rule aws-access-token matched {key}"));
        let mut chatty = finding("f1", "review", "src/a.rs", Some(40), "open");
        chatty["detail"] = json!(format!("The key {key} is hard-coded"));
        let mut outside = finding("s2", "gitleaks", "config/prod.env", Some(3), "open");
        outside["detail"] = json!(format!("value {key}"));
        let mut c = card(vec![leak, chatty, outside]);
        c["risk"]["reasons"] = json!([format!("Secret {key} found")]);
        c["tamper"]["flags"][0]["excerpt"] = json!(format!("-let k = \"{key}\";"));
        for inline in [true, false] {
            let b = build(&c, &facts(), inline);
            let all = format!("{}\n{}", b.body, b.comments.iter().map(|c| c.body.clone()).collect::<Vec<_>>().join("\n"));
            assert!(!all.contains(&key) && !all.contains("Q7XW3MZP2KD9RT4B"), "a secret left kwant:\n{all}");
            assert!(!all.contains("aws-access-token"), "the rule match left kwant:\n{all}");
            assert!(all.contains("Possible secret in src/a.rs:12") && all.contains("Possible secret in config/prod.env:3"), "{all}");
        }
        let b = build(&c, &facts(), true);
        let s1 = b.comments.iter().find(|c| c.line == 12).unwrap();
        assert_eq!(s1.body, "Possible secret in src/a.rs:12\n\n<sub>kwant review</sub>");
    }

    #[test]
    fn no_reviewer_signs_as_the_rule_checks() {
        let mut c = card(vec![]);
        c["reviewer"] = Value::Null;
        assert!(build(&c, &facts(), true).body.contains("Reviewed by kwant's rule checks (no reviewer bot) in kwant"));
    }

    #[test]
    fn only_line_errors_retry() {
        assert!(about_lines("Unprocessable Entity: Line could not be resolved"));
        assert!(about_lines("Pull request review thread line must be part of the diff"));
        assert!(about_lines("Path could not be resolved"));
        assert!(!about_lines("Validation Failed: commit_id is not part of the pull request"));
    }
}
