//! Jira tickets' HTTP API (api.md §2.3). Every route is the person's: `auth::person_only` covers
//! `/api/spaces/:id/dev/…`, and each handler also refuses a bot's turn token. Tokens come in in a
//! request body, go to the password vault, and never come back out: no answer, event or log line
//! carries one. Errors are `{"error", "code"}`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::pickup;
use super::source::{ask_text, Mention, SourceError, TicketWriter};
use super::store;
use crate::db::dev_jira::{self, JiraSiteRow};
use crate::db::spaces::SpaceId;
use crate::server::dev::atlassian::{self, check, client, jql};
use crate::server::spaces::active_space;
use crate::server::AppState;

pub const REFUSAL: &str = "only the person can read or change the Jira connection";

fn coded(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": message.into(), "code": code}))).into_response()
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!("jira: {e}");
    coded(StatusCode::INTERNAL_SERVER_ERROR, "internal", "kwant could not read or save that")
}

/// A source error as an answer. `Display` never carries a token, header or URL.
fn source_error(e: &SourceError) -> Response {
    match e {
        SourceError::NotFound => coded(StatusCode::NOT_FOUND, "not_found", "Jira has no such ticket"),
        SourceError::OutOfScope => coded(StatusCode::FORBIDDEN, "out_of_scope", e.to_string()),
        SourceError::Unauthorized => coded(StatusCode::BAD_GATEWAY, "auth_failed", "Jira didn't accept the saved token"),
        SourceError::Forbidden => coded(StatusCode::BAD_GATEWAY, "forbidden", "the token can't do that on this ticket"),
        SourceError::RateLimited { retry_after } => {
            let at = chrono::Utc::now() + chrono::Duration::from_std(retry_after.unwrap_or(std::time::Duration::from_secs(60))).unwrap_or(chrono::Duration::seconds(60));
            (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": "Jira asked us to slow down", "code": "rate_limited", "retry_at": at}))).into_response()
        }
        SourceError::Unavailable(m) => coded(StatusCode::BAD_GATEWAY, "unreachable", format!("couldn't reach Jira: {m}")),
        SourceError::NoSpaces | SourceError::Invalid(_) => coded(StatusCode::BAD_REQUEST, "bad_request", e.to_string()),
    }
}

#[allow(clippy::result_large_err)]
async fn guard(state: &AppState, headers: &HeaderMap, id: &str) -> Result<SpaceId, Response> {
    crate::server::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await?;
    active_space(state, id).await
}

#[allow(clippy::result_large_err)]
async fn site_or_404(state: &AppState, space: &SpaceId) -> Result<JiraSiteRow, Response> {
    match state.db.dev_jira_site(space.as_str()).await {
        Ok(Some(s)) => Ok(s),
        Ok(None) => Err(coded(StatusCode::NOT_FOUND, "no_site", "no Jira site is connected in this space")),
        Err(e) => Err(internal(e)),
    }
}

/// Takes a token out of a body: it never stays in a `Value` that could be logged or echoed.
fn take_secret(body: &mut Value, field: &str) -> Option<Zeroizing<String>> {
    let v = body.as_object_mut()?.remove(field)?;
    let s = v.as_str()?.trim();
    (!s.is_empty()).then(|| Zeroizing::new(s.to_string()))
}

fn str_of(body: &Value, field: &str) -> Option<String> {
    body[field].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn list_of(body: &Value, field: &str) -> Vec<String> {
    body[field].as_array().into_iter().flatten().filter_map(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

// ---------------------------------------------------------------------------------------------
// The site

/// `GET /api/spaces/:id/dev/jira` → `{site: row | null, token_saved}`. The row has no secret.
pub async fn get_site(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match state.db.dev_jira_site(space.as_str()).await {
        Ok(site) => Json(json!({"site": site, "tickets": state.db.dev_tickets(space.as_str()).await.map(|t| t.len()).unwrap_or(0)})).into_response(),
        Err(e) => internal(e),
    }
}

/// The token the person typed, else the saved one when the address (and kind) didn't change.
async fn token_or_saved(state: &AppState, existing: Option<&JiraSiteRow>, typed: Option<Zeroizing<String>>, url: &str) -> (Option<Zeroizing<String>>, Option<Zeroizing<String>>) {
    let same = existing.filter(|e| client::normalize_base(url).ok().as_deref() == Some(e.jira_url.as_str()));
    match (typed, same) {
        (Some(t), _) => (Some(t), None),
        (None, Some(e)) => match atlassian::secrets_of(state, e).await {
            Ok(s) => (Some(s.jira), s.confluence),
            Err(_) => (None, None),
        },
        (None, None) => (None, None),
    }
}

async fn check_input(state: &AppState, existing: Option<&JiraSiteRow>, body: &mut Value) -> check::CheckInput {
    let token = take_secret(body, "token");
    let conf_token = take_secret(body, "confluence_token");
    let url = str_of(body, "jira_url").unwrap_or_default();
    let (token, saved_conf) = token_or_saved(state, existing, token, &url).await;
    check::CheckInput {
        jira_url: url,
        kind: str_of(body, "kind"),
        email: str_of(body, "email"),
        token,
        confluence_url: str_of(body, "confluence_url"),
        confluence_token: conf_token.or(saved_conf),
        projects: list_of(body, "projects"),
        handover: str_of(body, "handover"),
        assignee_id: str_of(body, "assignee_id"),
        label: str_of(body, "label"),
        jql: str_of(body, "jql"),
    }
}

/// Branch names of this space's Dev mode cards (review and landed), for the suggested project.
async fn branch_names(state: &AppState, space: &str) -> Vec<String> {
    let mut out = Vec::new();
    for kind in ["review", "landed"] {
        for c in state.db.dev_cards(space, kind, None, 200).await.unwrap_or_default() {
            if let Some(b) = serde_json::from_str::<Value>(&c.facts).ok().and_then(|f| f["head_branch"].as_str().map(str::to_string)) {
                out.push(b);
            }
        }
    }
    out
}

/// `POST /api/spaces/:id/dev/jira/check`: the sheet's live check. Nothing is saved.
pub async fn check_site(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(mut body): Json<Value>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let existing = state.db.dev_jira_site(space.as_str()).await.ok().flatten();
    let input = check_input(&state, existing.as_ref(), &mut body).await;
    let mut out = check::check(input, atlassian::resolver()).await;
    if out.result == "found" {
        out.suggested_project = check::suggest_project(&out.projects, &branch_names(&state, space.as_str()).await);
    }
    Json(out).into_response()
}

/// A new site's id: `jira_` and 16 hex characters (also the vault entry's name).
fn new_site_id() -> String {
    format!("jira_{}", &uuid::Uuid::new_v4().simple().to_string()[..16])
}

/// `PUT /api/spaces/:id/dev/jira`: checks, then saves the token(s) in the vault and the settings
/// in the row. Body: the check's fields plus `spaces`, `team_id`, `assignee_key`,
/// `assignee_name`, `comment_progress`, `wait_for_ok`, `poll_secs`, `accept_risk`.
pub async fn put_site(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(mut body): Json<Value>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let existing = match state.db.dev_jira_site(space.as_str()).await {
        Ok(e) => e,
        Err(e) => return internal(e),
    };
    let input = check_input(&state, existing.as_ref(), &mut body).await;
    let token = input.token.clone();
    let conf_token = input.confluence_token.clone();
    let projects = input.projects.clone();
    if projects.is_empty() {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "pick at least one Jira project");
    }
    let out = check::check(input, atlassian::resolver()).await;
    if !out.passed() || out.jql_error.is_some() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": out.jql_error.clone().or(out.message.clone()).unwrap_or_else(|| "the check didn't pass".into()), "code": "check_failed", "check": out}))).into_response();
    }
    let Some(token) = token else {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "paste the token");
    };
    let spaces = list_of(&body, "spaces");
    let readable: Vec<&str> = out.confluence.as_ref().filter(|c| c.result == "ok").map(|c| c.spaces.iter().map(|s| s.key.as_str()).collect()).unwrap_or_default();
    if spaces.len() > jql::MAX_SPACES || spaces.iter().any(|s| !readable.contains(&s.as_str())) {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "pick Confluence spaces this token can read (at most 20)");
    }
    // The "Before you save the token" card: the built-in store's rules, and a locked vault.
    if let Err(r) = crate::server::passwords::accept_risk(&state, &headers, body["accept_risk"].as_bool().unwrap_or(false)).await {
        return r;
    }
    if let Some(r) = crate::server::passwords::refuse_local_save(&state).await {
        return r;
    }
    if state.credentials.is_locked().await {
        return (StatusCode::LOCKED, Json(json!({"error": "the password vault is locked: unlock it first", "locked": true}))).into_response();
    }

    let jira_url = out.jira_url.clone().unwrap_or_default();
    let site_id = existing.as_ref().map(|e| e.id.clone()).unwrap_or_else(new_site_id);
    if let Some(e) = &existing {
        if e.jira_url != jira_url || e.confluence_url != out.confluence_url {
            let _ = atlassian::forget_secrets(&state, &e.id).await;
        }
    }
    if let Err(e) = atlassian::save_secret(&state, &site_id, &jira_url, &token).await {
        return internal(format!("could not save the Jira token: {e}"));
    }
    if let (Some("dc"), Some(url), Some(t)) = (out.kind, out.confluence_url.as_deref(), conf_token.as_ref()) {
        if let Err(e) = atlassian::save_secret(&state, &atlassian::confluence_secret_id(&site_id), url, t).await {
            return internal(format!("could not save the Confluence token: {e}"));
        }
    }

    let mut row = existing.clone().unwrap_or_else(|| JiraSiteRow::blank(space.as_str(), &site_id));
    row.id = site_id;
    row.kind = out.kind.unwrap_or("cloud").to_string();
    row.jira_url = jira_url;
    row.confluence_url = out.confluence_url.clone();
    row.email = if row.kind == "cloud" { str_of(&body, "email") } else { None };
    row.auth = out.auth.unwrap_or("basic").to_string();
    row.cloud_id = out.cloud_id.clone();
    row.me_id = out.me.as_ref().map(|m| m.id.clone());
    row.me_name = out.me.as_ref().map(|m| m.display_name.clone());
    row.projects = serde_json::to_string(&projects).unwrap_or_else(|_| "[]".into());
    row.handover = str_of(&body, "handover").unwrap_or_else(|| "label".into());
    row.assignee_id = str_of(&body, "assignee_id");
    row.assignee_key = str_of(&body, "assignee_key").or_else(|| row.assignee_id.clone());
    row.assignee_name = str_of(&body, "assignee_name").map(|n| super::source::cut(&n, 120).0);
    row.label = str_of(&body, "label").unwrap_or_else(|| "kwant".into());
    row.jql = str_of(&body, "jql");
    row.team_id = str_of(&body, "team_id");
    row.spaces = serde_json::to_string(&spaces).unwrap_or_else(|_| "[]".into());
    row.comment_progress = body["comment_progress"].as_bool().unwrap_or(false);
    row.wait_for_ok = body["wait_for_ok"].as_bool().unwrap_or(false);
    row.poll_secs = body["poll_secs"].as_i64().unwrap_or(row.poll_secs).clamp(pickup::MIN_POLL_SECS, pickup::MAX_POLL_SECS);
    row.on_ = true;
    let rule_changed = existing.as_ref().is_none_or(|e| e.projects != row.projects || e.handover != row.handover || e.assignee_id != row.assignee_id || e.label != row.label || e.jql != row.jql);
    match state.db.save_dev_jira_site(&row, rule_changed).await {
        Ok(saved) => {
            pickup::poll_soon(space.as_str());
            Json(json!({"site": saved, "check": out})).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize, Default)]
pub struct DeleteQuery {
    /// Default true: the token goes too.
    pub forget_token: Option<bool>,
}

/// `DELETE /api/spaces/:id/dev/jira?forget_token=`: the site, its tickets and events (and token).
pub async fn delete_site(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<DeleteQuery>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match state.db.delete_dev_jira_site(space.as_str()).await {
        Ok(Some(site)) => {
            if q.forget_token.unwrap_or(true) {
                let _ = atlassian::forget_secrets(&state, &site.id).await;
            }
            super::hooks::site_deleted(&state, space.as_str(), &site.id).await;
            Json(json!({"deleted": true})).into_response()
        }
        Ok(None) => Json(json!({"deleted": false})).into_response(),
        Err(e) => internal(e),
    }
}

/// `POST /api/spaces/:id/dev/jira/on` `{on}`: turn pickup off (cards stay) or back on.
pub async fn set_on(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(on) = body["on"].as_bool() else {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "say on: true or false");
    };
    match state.db.set_dev_jira_on(space.as_str(), on).await {
        Ok(true) => {
            if on {
                pickup::poll_soon(space.as_str());
            }
            Json(json!({"on": on})).into_response()
        }
        Ok(false) => coded(StatusCode::NOT_FOUND, "no_site", "no Jira site is connected in this space"),
        Err(e) => internal(e),
    }
}

/// `POST /api/spaces/:id/dev/jira/poll`: poll now ("Check now"), answering what it did.
pub async fn poll_now(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = site_or_404(&state, &space).await {
        return r;
    }
    Json(pickup::poll_site(&state, space.as_str()).await).into_response()
}

#[derive(Deserialize, Default)]
pub struct UsersQuery {
    #[serde(default)]
    pub q: String,
}

/// `GET /api/spaces/:id/dev/jira/users?q=`: assignable users in the saved site's projects
/// ("Which Jira user stands for your bots?" when changing the rule after connecting).
pub async fn users(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<UsersQuery>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let site = match site_or_404(&state, &space).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let c = match atlassian::client_for(&state, &site).await {
        Ok(c) => c,
        Err(e) => return source_error(&e),
    };
    match c.assignable_users(&site.project_list(), &q.q).await {
        Ok(u) => Json(json!({"users": u})).into_response(),
        Err(e) => source_error(&e),
    }
}

// ---------------------------------------------------------------------------------------------
// Tickets

/// `GET /api/spaces/:id/dev/tickets` → `{tickets: [row]}` (the Tickets tab; newest first).
pub async fn list(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match state.db.dev_tickets(space.as_str()).await {
        Ok(t) => Json(json!({"tickets": t, "slots": pickup::SLOTS})).into_response(),
        Err(e) => internal(e),
    }
}

/// `GET /api/spaces/:id/dev/tickets/:key` → `{ticket, events}`.
pub async fn one(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match state.db.dev_ticket(space.as_str(), &key).await {
        Ok(Some(t)) => Json(json!({"ticket": t, "events": state.db.dev_ticket_events(space.as_str(), &key, 100).await.unwrap_or_default()})).into_response(),
        Ok(None) => coded(StatusCode::NOT_FOUND, "not_found", "no such ticket here"),
        Err(e) => internal(e),
    }
}

/// `POST /api/spaces/:id/dev/tickets/:key/start`: Start on a Waiting (backfill), Taken back or
/// Failed card. It joins the queue and starts when a slot is free.
pub async fn start(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let site = match site_or_404(&state, &space).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match store::set_state(&state, space.as_str(), &key, dev_jira::PICKED_UP, Some("started by the person")).await {
        Ok(true) => {
            let started = pickup::fill_slots(&state, &site).await;
            let t = state.db.dev_ticket(space.as_str(), &key).await.ok().flatten();
            Json(json!({"ticket": t, "started": started > 0 && t.as_ref().is_some_and(|t| t.start_called_at.is_some())})).into_response()
        }
        Ok(false) => coded(StatusCode::CONFLICT, "conflict", "this ticket can't be started from where it is"),
        Err(e) => internal(e),
    }
}

// ---------------------------------------------------------------------------------------------
// Ask on Jira (design §5): a preview of the exact comment, then one post.

/// Most questions one Ask on Jira comment carries.
pub const MAX_QUESTIONS: usize = 3;
pub const MAX_QUESTION_CHARS: usize = 500;
pub const MAX_CHOICES: usize = 5;
pub const MAX_CHOICE_CHARS: usize = 100;

#[derive(Debug, Deserialize, Clone)]
pub struct Question {
    pub text: String,
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AskBody {
    pub questions: Vec<Question>,
    /// The team's name for the sign-off ("— kwant, for the Shop team").
    #[serde(default)]
    pub team: Option<String>,
    /// From the preview: the comment posted must be the one the person saw.
    #[serde(default)]
    pub digest: Option<String>,
}

/// One line: control characters and runs of spaces collapsed.
fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    super::source::cut(&flat.split_whitespace().collect::<Vec<_>>().join(" "), max).0
}

/// The comment after `Hi @Name, ` (design §5). Err is a sentence for the person.
pub fn ask_rest(key: &str, questions: &[Question], team: Option<&str>) -> Result<String, String> {
    if questions.is_empty() || questions.len() > MAX_QUESTIONS {
        return Err(format!("ask 1 to {MAX_QUESTIONS} questions"));
    }
    let n = questions.len();
    let mut out = if n == 1 { format!("before we start {key} we have a question:\n\n") } else { format!("before we start {key} we have {n} questions:\n\n") };
    for (i, q) in questions.iter().enumerate() {
        let text = one_line(&q.text, MAX_QUESTION_CHARS);
        if text.is_empty() {
            return Err("a question is empty".into());
        }
        out.push_str(&format!("{}. {text}\n", i + 1));
        let choices: Vec<String> = q.choices.iter().map(|c| one_line(c, MAX_CHOICE_CHARS)).filter(|c| !c.is_empty()).take(MAX_CHOICES).collect();
        if !choices.is_empty() {
            out.push_str(&format!("   Suggested: {}\n", choices.join(" · ")));
        }
    }
    out.push_str("\nWe'll wait for your reply before building. Reply to this comment to answer.\n");
    match team.map(|t| one_line(t, 60)).filter(|t| !t.is_empty()) {
        Some(t) => out.push_str(&format!("— kwant, for the {t}")),
        None => out.push_str("— kwant"),
    }
    Ok(out)
}

/// What the person approves: ticket, addressee and the exact text.
pub fn ask_digest(key: &str, to: Option<&Mention>, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    h.update([0]);
    h.update(to.map(|m| m.mention_id.as_str()).unwrap_or("").as_bytes());
    h.update([0]);
    h.update(text.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The states a card can ask from: its questions block the build.
const ASK_STATES: &[&str] = &[dev_jira::NEEDS_ANSWER, dev_jira::READING];

struct Prepared {
    site: JiraSiteRow,
    writer: std::sync::Arc<client::AtlassianClient>,
    to: Option<Mention>,
    rest: String,
    text: String,
    digest: String,
}

#[allow(clippy::result_large_err)]
async fn prepare(state: &AppState, space: &SpaceId, key: &str, body: &AskBody) -> Result<Prepared, Response> {
    let site = site_or_404(state, space).await?;
    let ticket = match state.db.dev_ticket(space.as_str(), key).await {
        Ok(Some(t)) => t,
        Ok(None) => return Err(coded(StatusCode::NOT_FOUND, "not_found", "no such ticket here")),
        Err(e) => return Err(internal(e)),
    };
    if !ASK_STATES.contains(&ticket.state.as_str()) {
        return Err(coded(StatusCode::CONFLICT, "conflict", "this ticket has no open questions"));
    }
    if ticket.asked_comment_id.is_some() {
        return Err(coded(StatusCode::CONFLICT, "already_asked", "kwant already asked on this ticket; answer here, or wait for the reply"));
    }
    let rest = ask_rest(key, &body.questions, body.team.as_deref()).map_err(|m| coded(StatusCode::BAD_REQUEST, "bad_request", m))?;
    let writer = atlassian::client_for(state, &site).await.map_err(|e| source_error(&e))?;
    let to = writer.mention_target(key).await.map_err(|e| source_error(&e))?;
    let text = ask_text(to.as_ref(), &rest);
    if text.chars().count() > super::source::WRITE_CHARS {
        return Err(coded(StatusCode::BAD_REQUEST, "bad_request", "the comment is too long"));
    }
    let digest = ask_digest(key, to.as_ref(), &text);
    Ok(Prepared { site, writer, to, rest, text, digest })
}

fn as_whom(site: &JiraSiteRow) -> Value {
    json!({"email": site.email, "display_name": site.me_name})
}

/// `POST /api/spaces/:id/dev/tickets/:key/ask/preview` `{questions, team?}` → `{to, text, as,
/// digest}`. Nothing is posted.
pub async fn ask_preview(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>, Json(body): Json<AskBody>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match prepare(&state, &space, &key, &body).await {
        Ok(p) => Json(json!({"key": key, "to": p.to, "text": p.text, "as": as_whom(&p.site), "digest": p.digest})).into_response(),
        Err(r) => r,
    }
}

fn asking() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static S: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// `POST /api/spaces/:id/dev/tickets/:key/ask` `{questions, team?, digest}`: posts the previewed
/// comment once (409 `changed` when it no longer matches the preview) → `{comment_id, url, to, at}`.
pub async fn ask(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>, Json(body): Json<AskBody>) -> Response {
    let space = match guard(&state, &headers, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(digest) = body.digest.clone() else {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "look at the comment first (preview), then post it");
    };
    let lock = format!("{}/{key}", space.as_str());
    if !asking().lock().unwrap().insert(lock.clone()) {
        return coded(StatusCode::CONFLICT, "already_asked", "this comment is already being posted");
    }
    let res = post_ask(&state, &space, &key, &body, &digest).await;
    asking().lock().unwrap().remove(&lock);
    res
}

async fn post_ask(state: &AppState, space: &SpaceId, key: &str, body: &AskBody, digest: &str) -> Response {
    let p = match prepare(state, space, key, body).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if p.digest != digest {
        return (StatusCode::CONFLICT, Json(json!({"error": "the comment changed since you looked at it; look again", "code": "changed", "text": p.text, "digest": p.digest}))).into_response();
    }
    match p.writer.ask(key, p.to.as_ref(), &p.rest).await {
        Ok(posted) => {
            let _ = state.db.set_dev_ticket_asked(space.as_str(), key, &posted.comment.id).await;
            if let Ok(Some(t)) = state.db.dev_ticket(space.as_str(), key).await {
                store::notify(state, space.as_str(), key, &t.state);
            }
            Json(json!({"comment_id": posted.comment.id, "url": posted.url, "to": p.to, "at": posted.comment.created.unwrap_or_else(chrono::Utc::now), "text": p.text})).into_response()
        }
        Err(e) => {
            let _ = state.db.add_dev_ticket_event(space.as_str(), key, "comment", Some(&format!("couldn't post the question: {e}"))).await;
            source_error(&e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(t: &str, c: &[&str]) -> Question {
        Question { text: t.into(), choices: c.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn the_ask_text_is_the_designs() {
        let rest = ask_rest(
            "TRAIL-142",
            &[q("Which file should the export make?", &["CSV", "XLSX", "Both, CSV first"]), q("Who may export orders?\nThe file holds names.", &["Everyone with Orders access", "Only admins"])],
            Some("Shop team"),
        )
        .unwrap();
        assert_eq!(
            rest,
            "before we start TRAIL-142 we have 2 questions:\n\n1. Which file should the export make?\n   Suggested: CSV · XLSX · Both, CSV first\n2. Who may export orders? The file holds names.\n   Suggested: Everyone with Orders access · Only admins\n\nWe'll wait for your reply before building. Reply to this comment to answer.\n— kwant, for the Shop team"
        );
        assert!(ask_rest("TRAIL-1", &[q("One?", &[])], None).unwrap().starts_with("before we start TRAIL-1 we have a question:\n\n1. One?\n\nWe'll"));
        assert!(ask_rest("TRAIL-1", &[], None).is_err());
        assert!(ask_rest("TRAIL-1", &[q("a", &[]), q("b", &[]), q("c", &[]), q("d", &[])], None).is_err());
        assert!(ask_rest("TRAIL-1", &[q("  \n ", &[])], None).is_err());
    }

    #[test]
    fn the_digest_covers_ticket_addressee_and_text() {
        let m = Mention { mention_id: "a".into(), person_id: "a".into(), display_name: "A".into(), role: "reporter" };
        let d = ask_digest("TRAIL-1", Some(&m), "x");
        assert_eq!(d.len(), 64);
        assert_ne!(d, ask_digest("TRAIL-2", Some(&m), "x"));
        assert_ne!(d, ask_digest("TRAIL-1", None, "x"));
        assert_ne!(d, ask_digest("TRAIL-1", Some(&m), "y"));
    }
}
