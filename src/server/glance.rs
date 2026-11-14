//! The desktop island's one call (0.9.13): `GET /api/glance` says, across every space that isn't
//! archived, what needs the person, what is working now and what just finished, in one cheap
//! response the app polls every 5 s per place with `If-None-Match`. `POST /api/glance/ask` sends a
//! quick ask into the person's chat with a bot, as if typed there. Both are the person's only.
//!
//! Needs you is exactly `GET /api/away`'s `waiting` list (the same builder), so the island and the
//! nudge agree. Each approval says whether the island may offer Allow once inline
//! ([`can_allow_inline`]): never for money, the production or push floor, a tier-forced ask, a
//! risky step or anything else whose only safe answer is the full card. Text from bots is
//! untrusted: titles are one masked line, a working bot's step is its activity log's own redacted
//! title, and no tool output, message body or argument value is ever sent.

use super::away::{self, mask_words, rfc3339};
use super::AppState;
use crate::db::spaces::{Space, SpaceId};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Most Needs-you items in one glance (the counts say how many there are in all).
pub const NEEDS_YOU_MAX: usize = 20;
/// Most running turns listed under Working now.
pub const WORKING_MAX: usize = 12;
/// Just finished: at most this many, from the last [`FINISHED_HOURS`].
pub const FINISHED_MAX: usize = 5;
pub const FINISHED_HOURS: i64 = 24;
/// Most bots offered for a quick ask.
pub const ASK_TARGETS_MAX: i64 = 8;
/// Most answer choices a question shows inline; a question with more says so (`choices_total`).
pub const CHOICES_MAX: usize = 6;
const CHOICE_CHARS: usize = 60;
const STEP_CHARS: usize = 80;
/// Longest quick ask. The chat composer has no cap of its own (only the 2 MB request limit); a
/// quick ask is one line from a small box, so it gets a generous fixed one.
pub const ASK_MAX_CHARS: usize = 8_000;

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({ "error": message, "code": code }))).into_response()
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Couldn't read what's happening. Try again.")
}

/// One line of at most `max` characters, with key-like words masked.
fn line(text: &str, max: usize) -> String {
    let one = mask_words(&text.split_whitespace().collect::<Vec<_>>().join(" "));
    match one.chars().count() > max {
        true => format!("{}…", one.chars().take(max - 1).collect::<String>().trim_end()),
        false => one,
    }
}

/// Percent-encodes everything but RFC 3986 unreserved characters (a path segment or query value),
/// as the desktop app's nudge does.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Where an item opens in the app (the nudge's `target_path`): its channel, its bot's
/// conversation, else its bot's chat, each with `?card=<id>` when a card should show; with no bot,
/// the space's Inbox.
pub fn app_path(space: &str, link: &Value, kwant_id: &str, card: Option<&str>) -> String {
    let base = format!("/s/{}", encode(space));
    let card = card.filter(|c| !c.is_empty()).map(|c| format!("?card={}", encode(c))).unwrap_or_default();
    if let Some(c) = link["channel_id"].as_str().filter(|c| !c.is_empty()) {
        return format!("{base}/c/{}{card}", encode(c));
    }
    let bot = link["direct_kwant_id"].as_str().filter(|b| !b.is_empty()).unwrap_or(kwant_id);
    if bot.is_empty() {
        return format!("{base}/inbox");
    }
    match link["thread_id"].as_str().filter(|t| !t.is_empty()) {
        Some(t) => format!("{base}/b/{}/t/{}{card}", encode(bot), encode(t)),
        None => format!("{base}/b/{}{card}", encode(bot)),
    }
}

/// GitHub tool names that merge, push or change a branch without a file write [`mcp_writes`]
/// would see (a merge, a branch update, a release). Any case, any prefix.
///
/// [`mcp_writes`]: super::dev::prepush::mcp_writes
const PUSH_WORDS: &[&str] = &["MERGE", "PUSH", "UPDATE_BRANCH", "UPDATE_A_REFERENCE", "UPDATE_REF", "CREATE_A_REFERENCE", "DELETE_A_REFERENCE", "RELEASE"];

/// Why an approval may only be answered on its full card, from its stored details alone; `None`
/// when the island may offer Allow once. Starts from Telegram's rule (money, the production floor,
/// a tier-forced ask, a turn that read outside text or that kwant started on its own, a shell
/// command, kwant's own tools, wrappers that run other actions, file tools outside the bot's
/// folder), then adds what a one-line peek can't show safely: a risky step (`smart_reason`), the
/// push floor (a prepush grant, a GitHub write, a `git push`, a merge) and details it can't read.
pub fn inline_refusal(details: &Value) -> Option<&'static str> {
    if !details.is_object() {
        return Some("Open it to see what it does.");
    }
    if let Some(why) = super::telegram::link_only_reason(details) {
        return Some(match why {
            "This kind of step can't be approved from Telegram." => "This kind of step needs the full card.",
            other => other,
        });
    }
    let set = |k: &str| details.get(k).is_some_and(|v| !v.is_null() && v != false);
    if set("smart_reason") {
        return Some("kwant flagged this step as risky.");
    }
    if set("prepush_grant") || set("strict_ask") {
        return Some("This pushes code, so it needs the full card.");
    }
    let tool = details["tool"].as_str().unwrap_or_default();
    let args = &details["arguments"];
    if !super::dev::prepush::mcp_writes(tool, args).is_empty() {
        return Some("This pushes code, so it needs the full card.");
    }
    let upper = tool.to_ascii_uppercase();
    if PUSH_WORDS.iter().any(|w| upper.contains(w)) {
        return Some("This pushes code, so it needs the full card.");
    }
    // A command or argument that pushes, wherever it hides
    let text = args.to_string().to_ascii_lowercase();
    if text.contains("git push") {
        return Some("This pushes code, so it needs the full card.");
    }
    None
}

/// [`inline_refusal`], plus the space's production marks (as Telegram's [`link_only_in`]).
///
/// [`link_only_in`]: super::telegram::link_only_in
pub async fn can_allow_inline(state: &AppState, space: &str, details: &Value) -> Result<(), &'static str> {
    if let Some(why) = inline_refusal(details) {
        return Err(why);
    }
    match super::telegram::link_only_in(state, space, details).await {
        None => Ok(()),
        Some("This is a production connection.") => Err("This is a production connection."),
        Some(_) => Err("This kind of step needs the full card."),
    }
}

/// One Needs-you item: `/api/away`'s waiting item in the island's shape.
async fn needs_you_item(state: &AppState, space: &Space, v: &Value) -> Value {
    let kind = match (v["kind"].as_str().unwrap_or(""), v.get("human_step_id").is_some_and(|h| !h.is_null())) {
        ("stopped", true) => "human_step",
        (k, _) => k,
    };
    let id = v["id"].as_str().unwrap_or("");
    let kwant_id = v["kwant_id"].as_str().unwrap_or("");
    let card = matches!(kind, "approval" | "question").then_some(id);
    let mut out = json!({
        "kind": kind,
        "id": id,
        "space_id": space.id,
        "space_name": line(&space.name, 60),
        "kwant_id": kwant_id,
        "kwant_name": line(v["kwant_name"].as_str().unwrap_or("A bot"), 60),
        "title": line(v["title"].as_str().unwrap_or(""), 120),
        "since": v["timestamp"],
        "path": app_path(&space.id, &v["link"], kwant_id, card),
        "approval": Value::Null,
        "question": Value::Null,
    });
    if kind == "approval" {
        // The stored details, unmasked, decide; none of them is sent
        let details = match state.db.get_approval(id).await {
            Ok(Some(a)) => serde_json::from_str(&a.parameters).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let verdict = can_allow_inline(state, &space.id, &details).await;
        out["approval"] = json!({
            "risk": if verdict.is_ok() { "normal" } else { "high" },
            "can_allow_inline": verdict.is_ok(),
            "why_open": verdict.err(),
        });
    }
    if kind == "question" {
        let all: Vec<&str> = v["choices"].as_array().map(|c| c.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let shown: Vec<String> = all.iter().take(CHOICES_MAX).map(|c| line(c, CHOICE_CHARS)).collect();
        out["question"] = json!({ "choices": shown, "choices_total": all.len(), "allow_text": true });
    }
    out
}

/// A running turn, as Working now shows it.
struct Working {
    since: DateTime<Utc>,
    value: Value,
}

async fn working(state: &AppState, spaces: &HashMap<String, &Space>, names: &mut HashMap<String, (String, String)>) -> Vec<Working> {
    let mut out = Vec::new();
    for t in state.turns.active_turns() {
        // The hidden decide-first round isn't work anyone asked for; a queued turn isn't working
        if t.purpose.as_deref() == Some("decide") || t.state != "running" {
            continue;
        }
        if !names.contains_key(&t.kwant_id) {
            let Ok(Some(k)) = state.db.get_kwant(&t.kwant_id).await else { continue };
            names.insert(t.kwant_id.clone(), (k.name, k.space_id));
        }
        let (name, space_id) = names[&t.kwant_id].clone();
        if !spaces.contains_key(&space_id) {
            continue;
        }
        let link = match &t.thread_id {
            Some(thread) => json!({"thread_id": thread}),
            None => match state.db.get_channel(&t.context_id).await {
                Ok(Some(_)) => json!({"channel_id": t.context_id}),
                _ => json!({"direct_kwant_id": t.context_id}),
            },
        };
        let (step, step_at) = super::activity::current_step(&t.turn_id).unwrap_or_else(|| ("Getting started".into(), rfc3339(t.since)));
        out.push(Working {
            since: t.since,
            value: json!({
                "turn_id": t.turn_id,
                "kwant_id": t.kwant_id,
                "kwant_name": line(&name, 60),
                "space_id": space_id,
                "step": line(&step, STEP_CHARS),
                "step_started_at": step_at,
                "started_at": rfc3339(t.since),
                "path": app_path(&space_id, &link, &t.kwant_id, None),
            }),
        });
    }
    out
}

/// One Just-finished item, with its done-check line when the item has one.
async fn finished_item(state: &AppState, space: &Space, row: crate::db::away::AwayRow, unseen: bool) -> Value {
    let facts = super::inbox_done::away_facts(state, &row.kind, &row.id).await.unwrap_or_default();
    let check = (!facts.tasks.is_empty()).then(|| {
        let c = super::inbox_done::checked(&facts.tasks, facts.message.as_ref().map(super::inbox_done::meta_of).as_ref());
        let status = match c["state"].as_str() {
            Some("checked") => "verified",
            Some("failed") => "failed",
            _ => "not_checked",
        };
        json!({ "status": status, "line": line(c["line"].as_str().unwrap_or(""), 120) })
    });
    let at = row.at;
    let v = away::item(row);
    let kwant_id = v["kwant_id"].as_str().unwrap_or("");
    let result_link = facts.message.as_ref().map(|m| json!({"thread_id": m.thread_id, "channel_id": m.channel_id, "direct_kwant_id": m.direct_kwant_id}));
    json!({
        "kind": v["kind"],
        "id": v["id"],
        "space_id": space.id,
        "kwant_id": kwant_id,
        "kwant_name": line(v["kwant_name"].as_str().unwrap_or("A bot"), 60),
        "title": line(v["title"].as_str().unwrap_or(""), 120),
        "status": v["status"],
        "finished_at": rfc3339(at),
        "unseen": unseen,
        "check": check,
        "path": app_path(&space.id, result_link.as_ref().unwrap_or(&v["link"]), kwant_id, None),
    })
}

/// The bots a quick ask can go to: the person's most recent direct chats in live spaces, then the
/// rest by name. There are no server-side pins (the sidebar's pins live in the browser), so the
/// app puts its own pins first.
async fn ask_targets(state: &AppState) -> Result<Vec<Value>, crate::db::DbError> {
    let rows = state.db.ask_targets(ASK_TARGETS_MAX).await?;
    Ok(rows.into_iter().map(|(id, name, space)| json!({"kwant_id": id, "name": line(&name, 60), "space_id": space})).collect())
}

/// The glance without `now`, which the ETag is taken over.
pub async fn build(state: &AppState, now: DateTime<Utc>) -> Result<Value, crate::db::DbError> {
    let spaces: Vec<Space> = state.db.list_spaces().await?.into_iter().filter(|s| s.archived_at.is_none()).collect();
    let live: HashMap<String, &Space> = spaces.iter().map(|s| (s.id.clone(), s)).collect();
    // Approvals and mission watches are read once for every space, not once per space
    let mut approvals = away::approvals_by_space(state, None).await?;
    let mut watch = crate::server::missions::watch::waiting_items_by_space(state).await;
    let mut waiting: Vec<(&Space, Value)> = Vec::new();
    let mut needs_you_count = 0;
    let mut finished_rows: Vec<(bool, &Space, crate::db::away::AwayRow)> = Vec::new();
    let mut finished_unseen = 0;
    let day_ago = now - Duration::hours(FINISHED_HOURS);
    let stuck_before = now - Duration::minutes(away::STUCK_AFTER_MINUTES);
    for space in &spaces {
        let id = SpaceId::new(space.id.clone());
        let mine = approvals.remove(&id).unwrap_or_default();
        let watching = watch.remove(&space.id).unwrap_or_default();
        let (items, count) = away::waiting_with(state, &id, mine, watching).await?;
        needs_you_count += count;
        waiting.extend(items.into_iter().take(NEEDS_YOU_MAX).map(|v| (space, v)));
        let seen = away::last_seen(state, &id).await?;
        let from = seen.map_or(day_ago, |s| s.max(day_ago));
        finished_unseen += state.db.away_counts(&id, from, stuck_before).await?.0;
        for row in state.db.away_finished(&id, day_ago, FINISHED_MAX as i64).await? {
            finished_rows.push((seen.is_none_or(|s| row.at > s), space, row));
        }
    }
    // Cut first, then build only the items that are sent
    waiting.sort_by(|(_, a), (_, b)| b["timestamp"].as_str().unwrap_or("").cmp(a["timestamp"].as_str().unwrap_or("")).then_with(|| b["id"].as_str().cmp(&a["id"].as_str())));
    waiting.truncate(NEEDS_YOU_MAX);
    let mut needs_you = Vec::with_capacity(waiting.len());
    for (space, v) in &waiting {
        needs_you.push(needs_you_item(state, space, v).await);
    }
    finished_rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.at.cmp(&a.2.at)));
    finished_rows.truncate(FINISHED_MAX);
    let mut finished = Vec::with_capacity(finished_rows.len());
    for (unseen, space, row) in finished_rows {
        finished.push(finished_item(state, space, row, unseen).await);
    }

    let mut names = HashMap::new();
    let mut work = working(state, &live, &mut names).await;
    let working_count = work.len();
    work.sort_by(|a, b| b.since.cmp(&a.since).then_with(|| b.value["turn_id"].as_str().cmp(&a.value["turn_id"].as_str())));
    work.truncate(WORKING_MAX);

    Ok(json!({
        "counts": {
            "needs_you": needs_you_count,
            "working": working_count,
            "finished_unseen": finished_unseen,
        },
        "needs_you": needs_you,
        "working": work.into_iter().map(|w| w.value).collect::<Vec<_>>(),
        "finished": finished,
        "ask_targets": ask_targets(state).await?,
    }))
}

/// A weak ETag over the glance's content (not its `now`).
pub fn etag_of(body: &Value) -> String {
    let digest = Sha256::digest(body.to_string().as_bytes());
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("W/\"g-{hex}\"")
}

/// Whether `If-None-Match` names this ETag (or `*`). Weak comparison, as for a GET.
pub fn matches(header: Option<&str>, etag: &str) -> bool {
    let Some(h) = header else { return false };
    let bare = |t: &str| t.trim().trim_start_matches("W/").to_string();
    h.split(',').any(|t| t.trim() == "*" || bare(t) == bare(etag))
}

/// `GET /api/glance`: see the module docs. 304 with no body when `If-None-Match` names the
/// current ETag.
pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let now = Utc::now();
    let Ok(mut body) = build(&state, now).await else {
        return internal();
    };
    let etag = etag_of(&body);
    let cache = [(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"))];
    let etag_header = HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("W/\"g\""));
    if matches(headers.get(header::IF_NONE_MATCH).and_then(|h| h.to_str().ok()), &etag) {
        return (StatusCode::NOT_MODIFIED, cache, [(header::ETAG, etag_header)]).into_response();
    }
    body["now"] = json!(rfc3339(now));
    (cache, [(header::ETAG, etag_header)], Json(body)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AskPayload {
    pub space_id: String,
    pub kwant_id: String,
    pub text: String,
    /// The app's own id for this ask, the same on every retry: sent once (as the composer's).
    #[serde(default)]
    pub client_message_id: Option<String>,
}

/// `POST /api/glance/ask {space_id, kwant_id, text, client_message_id?}`: sends `text` into the
/// person's chat with that bot, exactly as if typed there (the bot answers there; nothing opens).
/// 200 `{path, user_message_id}`; 400 for an empty or too long text, 404 for a bot not in that
/// space, 409 for an archived space.
pub async fn ask(State(state): State<AppState>, Json(p): Json<AskPayload>) -> Response {
    let text = p.text.trim();
    if text.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty", "Type something to ask.");
    }
    if text.chars().count() > ASK_MAX_CHARS {
        return error(StatusCode::BAD_REQUEST, "too_long", "That's too long for a quick ask. Open the chat to send it.");
    }
    let kwant = match state.db.get_kwant(&p.kwant_id).await {
        Ok(Some(k)) if k.space_id == p.space_id => k,
        Ok(_) => return error(StatusCode::NOT_FOUND, "not_found", "That bot isn't in this space."),
        Err(_) => return internal(),
    };
    let space = SpaceId::new(p.space_id.clone());
    if let Err(r) = super::spaces::ensure_active(&state, &space).await {
        return r;
    }
    let payload = super::routes::SendChatPayload {
        channel_id: None,
        kwant_id: Some(kwant.id.clone()),
        team_id: None,
        thread_id: None,
        prompt: text.to_string(),
        sender_id: "PlayerHero".into(),
        attachment_ids: Vec::new(),
        reply_to_message_id: None,
        source: None,
        meta: None,
        client_message_id: p.client_message_id,
        message_id: None,
    };
    match super::routes::send_chat_inner(state, payload, true).await {
        Ok(Json(v)) => Json(json!({
            "path": app_path(&p.space_id, &json!({"direct_kwant_id": kwant.id}), &kwant.id, None),
            "user_message_id": v["user_message_id"],
            "deduped": v["deduped"].as_bool().unwrap_or(false),
        }))
        .into_response(),
        Err(StatusCode::CONFLICT) => error(StatusCode::CONFLICT, "archived", "This space is archived."),
        Err(StatusCode::BAD_REQUEST) => error(StatusCode::BAD_REQUEST, "bad_request", "That ask couldn't be sent."),
        Err(_) => internal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp(server: &str, tool: &str, args: Value) -> Value {
        json!({"source": "mcp", "server": server, "tool": tool, "arguments": args})
    }

    #[test]
    fn a_plain_mcp_call_may_be_allowed_inline() {
        assert_eq!(inline_refusal(&mcp("notion", "create_page", json!({"title": "Notes"}))), None);
        let read = json!({"source": "claude_builtin", "tool": "Read", "outside_folder": false, "arguments": {"file_path": "a.txt"}});
        assert_eq!(inline_refusal(&read), None);
    }

    #[test]
    fn money_risk_and_the_floors_need_the_full_card() {
        let with = |k: &str, v: Value| {
            let mut d = mcp("stripe", "create_invoice", json!({}));
            d[k] = v;
            d
        };
        for (k, v) in [
            ("payment_reason", json!("Sends money")),
            ("prod_floor", json!({"server": "db"})),
            ("tier_reason", json!("Tier 3")),
            ("smart_reason", json!("Deletes records")),
            ("prepush_grant", json!({"repo": "x"})),
            ("tainted", json!(true)),
            ("always_ask", json!(true)),
        ] {
            assert!(inline_refusal(&with(k, v)).is_some(), "{k}");
        }
        // A shell command, kwant's own tools, wrappers and files outside the folder
        assert!(inline_refusal(&json!({"source": "local_shell", "tool": "shell", "arguments": {"command": "ls"}})).is_some());
        assert!(inline_refusal(&json!({"source": "claude_builtin", "tool": "Bash", "arguments": {"command": "ls"}})).is_some());
        assert!(inline_refusal(&json!({"source": "claude_builtin", "tool": "Write", "arguments": {}})).is_some());
        assert!(inline_refusal(&mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", json!({}))).is_some());
        assert!(inline_refusal(&Value::Null).is_some(), "unreadable details: false");
        assert!(inline_refusal(&json!({"tool": "x"})).is_some(), "no source: false");
    }

    #[test]
    fn pushes_and_merges_need_the_full_card() {
        let write = mcp("github", "create_or_update_file", json!({"owner": "o", "repo": "r", "path": "a", "content": "x", "message": "m", "branch": "main"}));
        assert!(inline_refusal(&write).is_some());
        assert!(inline_refusal(&mcp("github", "merge_pull_request", json!({"owner": "o", "repo": "r", "pull_number": 1}))).is_some());
        assert!(inline_refusal(&mcp("composio", "GITHUB_MERGE_A_PULL_REQUEST", json!({}))).is_some());
        assert!(inline_refusal(&mcp("runner", "run", json!({"cmd": "cd x && git push origin main"}))).is_some());
        assert!(inline_refusal(&mcp("github", "create_issue", json!({"owner": "o", "repo": "r", "title": "Bug"}))).is_none());
    }

    #[test]
    fn etags_match_weakly_and_ignore_now() {
        let a = json!({"counts": {"needs_you": 1}});
        let tag = etag_of(&a);
        assert!(tag.starts_with("W/\"g-"));
        assert_eq!(tag, etag_of(&a.clone()));
        assert_ne!(tag, etag_of(&json!({"counts": {"needs_you": 2}})));
        assert!(matches(Some(&tag), &tag));
        assert!(matches(Some(&tag.replace("W/", "")), &tag));
        assert!(matches(Some(&format!("\"other\", {tag}")), &tag));
        assert!(matches(Some("*"), &tag));
        assert!(!matches(Some("W/\"g-0\""), &tag));
        assert!(!matches(None, &tag));
    }

    #[test]
    fn paths_follow_the_nudge() {
        assert_eq!(app_path("s", &json!({"channel_id": "c 1"}), "k", Some("ap")), "/s/s/c/c%201?card=ap");
        assert_eq!(app_path("s", &json!({"thread_id": "t1"}), "k1", Some("ap")), "/s/s/b/k1/t/t1?card=ap");
        assert_eq!(app_path("s", &json!({"direct_kwant_id": "k2"}), "k1", None), "/s/s/b/k2");
        assert_eq!(app_path("s", &json!({}), "", None), "/s/s/inbox");
    }

    #[test]
    fn lines_are_short_one_line_and_masked() {
        assert_eq!(line("a\n  b", 10), "a b");
        assert!(!line("token sk-ant-api03-abcdefghijklmnop1234", 120).contains("sk-ant"));
        assert_eq!(line(&"x".repeat(200), 10).chars().count(), 10);
    }
}
