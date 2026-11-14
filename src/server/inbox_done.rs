//! 0.9.5 B: Inbox "Done while you were away" (release-095/inbox-away/brief-b.md, Mira's
//! ux-audit-090 §1). What `GET /api/inbox` adds to each done item (finished, failed, to review):
//!
//! - `result_message_id`, `result_conversation_id` and `result_link`: the message that holds the
//!   answer. A routine run's is `routine_runs.message_id`, a hand-off's the delegation's result,
//!   a task's the routine run or hand-off it came from, else the owner's reply at the hand-in;
//!   a team job's is the lead's `Done:` reply.
//! - `answer_excerpt`: that message as plain text, at most [`EXCERPT_CHARS`], secrets masked.
//! - `trail` (`trail_dropped`): up to [`TRAIL_MAX`] `{at, kind, label, detail, status, by}`
//!   steps from the message's saved `meta.activity` (the activity store; no second one). `kind`
//!   is `page` (opened or read a web page), `message` (sent something to a person or a place
//!   outside this chat, from the fixed [`MESSAGE_TOOLS`] list), `write` (anything else that may
//!   have changed something) or `tool` (read only).
//! - `why` (`why_kind`): the routine's instruction, or the task's, hand-off's or request's first
//!   line, at most [`WHY_CHARS`].
//! - `changed` (Iris's name, part F) and `changed_nothing`: whether the job wrote anything.
//!   A task's side-effect ledger (`turn_tool_calls`) decides when it has rows; otherwise the trail
//!   does, with the ledger's rule that only reading tools change nothing. `null` when neither
//!   is known, and then the UI draws no "Read only" chip.
//! - `unread`: newer than the last-seen marker. `POST /api/away/seen` only clears this now; done
//!   items stay listed for [`DONE_DAYS`] days.
//!
//! - `checked` (0.9.7 done-check, autonomy/done-check/brief.md): what checked the result and how,
//!   from what kwant already stores, see [`checked`]. Nothing here runs a check.
//!
//! A lead's reply that ended with `Done: …` (keep_going's `chain_done` meta) is a done item of
//! kind `team_job`, with the lead, the members it handed work to for that request, and the fields
//! above.

use super::away::{self, rfc3339, DEFAULT_LOOKBACK_DAYS, MAX_ITEMS};
use super::inbox::{sort_key, Group};
use super::keep_going::DONE_KEY;
use super::AppState;
use crate::db::away::DoneReplyRow;
use crate::db::missions::{Evidence, OutcomeCheck, Task};
use crate::db::models::Message;
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::server::missions::autofix::{redact, redact_value};
use crate::server::missions::checkpoints::NO_EFFECT_TOOLS;
use crate::server::missions::contract::{Contract, Criterion, Part};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Done items stay this many days, seen or not.
pub const DONE_DAYS: i64 = DEFAULT_LOOKBACK_DAYS;
pub const EXCERPT_CHARS: usize = 600;
pub const WHY_CHARS: usize = 300;
pub const TRAIL_MAX: usize = 30;
const TITLE_CHARS: usize = 120;
/// How long after a task's hand-in its owner's reply still counts as the result.
const REPLY_SLACK_MINUTES: i64 = 30;

/// The oldest a finished or failed item may be: 7 days back, or a later `since` the caller asked for.
pub fn done_since(now: DateTime<Utc>, requested: Option<DateTime<Utc>>) -> DateTime<Utc> {
    let floor = now - Duration::days(DONE_DAYS);
    requested.map_or(floor, |r| r.max(floor))
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max - 1) {
        Some((at, _)) if text.chars().count() > max => format!("{}…", text[..at].trim_end()),
        _ => text.to_string(),
    }
}

/// One line of markdown as plain text: heading, quote and list markers, emphasis and code ticks gone.
fn plain_line(line: &str) -> String {
    let l = line.trim();
    let l = l.trim_start_matches(['#', '>']).trim_start();
    let l = match l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
        Some(rest) => rest,
        None => l,
    };
    l.replace("**", "").replace("__", "").replace('`', "")
}

/// The answer as plain text, at most [`EXCERPT_CHARS`], with secrets masked line by line; code
/// blocks dropped and runs of blank lines folded. `None` when there is no text.
pub fn excerpt(text: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut in_code = false;
    for raw in text.lines() {
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let line = away::mask_words(&plain_line(raw));
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let joined = lines.join("\n");
    (!joined.is_empty()).then(|| cut(&joined, EXCERPT_CHARS))
}

/// A routine's instruction (all of it, lines kept) or a task's first line, masked, at most [`WHY_CHARS`].
pub fn why_of(text: &str, whole: bool) -> Option<String> {
    let text = match whole {
        true => text.lines().map(|l| away::mask_words(l.trim())).collect::<Vec<_>>().join("\n").trim().to_string(),
        false => text.lines().map(plain_line).find(|l| !l.trim().is_empty()).map(|l| away::mask_words(&l)).unwrap_or_default(),
    };
    (!text.is_empty()).then(|| cut(&text, WHY_CHARS))
}

/// A tool's own name without kwnt's `mcp__kwnt__` prefix; other servers keep theirs.
fn own_tool(tool: &str) -> &str {
    tool.strip_prefix("mcp__kwnt__").or_else(|| tool.strip_prefix("mcp__kwant__")).unwrap_or(tool)
}

/// Tools that open or read a web page: a trail step of kind `page`.
const PAGE_TOOLS: [&str; 12] = [
    "WebFetch",
    "webfetch",
    "web_fetch",
    "fetch",
    "browser_open",
    "browser_navigate",
    "navigate",
    "open_url",
    "browser_page",
    "browser_snapshot",
    "browser_read",
    "browser_screenshot",
];

/// Reading tools beyond the ledger's [`NO_EFFECT_TOOLS`]: opening and reading pages, kwnt's own
/// task tools, asking the person. Anything else (a command, a file write, another server's tool,
/// clicking in the browser) may change something.
const READ_TOOLS: [&str; 6] = ["browser_cite", "web_search", "websearch", "ask_user", "mission_plan", "update_plan"];

fn is_page(tool: &str) -> bool {
    PAGE_TOOLS.contains(&own_tool(tool))
}

/// Tools that send something to a person or a place outside this chat: a trail step of kind
/// `message`, which counts as a change. Matched without case on the tool's own name (the part
/// after the last `__`, so `mcp__gmail__send_email` and Composio's `GMAIL_SEND_EMAIL` both match).
/// A fixed list: any other tool that may change something stays `write`.
const MESSAGE_TOOLS: [&str; 26] = [
    // mail
    "send_email",
    "send_mail",
    "sendmail",
    "reply_to_email",
    "gmail_send_email",
    "gmail_reply_to_thread",
    "outlook_send_email",
    // Slack
    "slack_post_message",
    "slack_send_message",
    "slack_sends_a_message_to_a_slack_channel",
    "chat_postmessage",
    // Discord
    "discord_send_message",
    "discordbot_create_message",
    // Telegram
    "telegram_send_message",
    "send_telegram_message",
    // X
    "post_tweet",
    "create_tweet",
    "twitter_creation_of_a_post",
    "twitter_create_post",
    "x_create_post",
    "x_post",
    // kwnt's own send and post tools
    "send_message",
    "post_message",
    "post_to_room",
    "message_person",
    "send_voice_note",
];

/// Whether a tool sends a message outside this chat ([`MESSAGE_TOOLS`]).
pub fn is_message(tool: &str) -> bool {
    let own = own_tool(tool);
    let name = own.rsplit("__").next().unwrap_or(own);
    MESSAGE_TOOLS.iter().any(|m| m.eq_ignore_ascii_case(name))
}

/// Whether a saved activity step may have changed something.
pub fn step_changes(step: &Value) -> bool {
    match step["kind"].as_str() {
        Some("tool") | Some("approval") => {}
        // thinking, notes and human steps run no tool
        _ => return false,
    }
    // A tool that never ran (declined, stopped before it started) changed nothing
    if matches!(step["status"].as_str(), Some("declined")) {
        return false;
    }
    let Some(tool) = step["tool"].as_str() else {
        return true;
    };
    let own = own_tool(tool);
    let kwnt_own = own.len() != tool.len();
    !(NO_EFFECT_TOOLS.contains(&own) || is_page(tool) || READ_TOOLS.contains(&own) || (kwnt_own && own.starts_with("task_")))
}

fn activity_steps(meta: &Value) -> (Vec<Value>, i64) {
    let a = &meta["activity"];
    let steps = a["steps"].as_array().cloned().unwrap_or_default();
    (steps, a["dropped"].as_i64().unwrap_or(0))
}

/// A message's meta as JSON (`Null` when it has none or it doesn't parse).
pub fn meta_of(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// A step's status in Iris's three words: `failed` is `error`; a step that never ran or never
/// finished (declined, stopped, still running or waiting when the reply was saved) is `skipped`.
fn trail_status(status: Option<&str>) -> &'static str {
    match status {
        Some("failed") => "error",
        Some("declined" | "stopped" | "running" | "waiting") => "skipped",
        _ => "ok",
    }
}

/// One message's steps as trail rows: tools, approvals and human steps (thinking and notes left
/// out), with who did them when it isn't the item's own bot. Already redacted when saved.
pub fn trail_rows(meta: &Value, by: Option<&str>) -> (Vec<Value>, i64) {
    let (steps, dropped) = activity_steps(meta);
    let rows = steps
        .iter()
        .filter(|s| matches!(s["kind"].as_str(), Some("tool" | "approval" | "human_step")))
        .map(|s| {
            let kind = match s["tool"].as_str() {
                Some(t) if is_page(t) => "page",
                Some(t) if is_message(t) && step_changes(s) => "message",
                _ if step_changes(s) => "write",
                _ => "tool",
            };
            json!({
                "at": s["started_at"],
                "kind": kind,
                "label": s["title"].as_str().map(away::mask_words).unwrap_or_default(),
                "detail": s["detail"].as_str().map(away::mask_words),
                "status": trail_status(s["status"].as_str()),
                "by": by,
            })
        })
        .collect();
    (rows, dropped)
}

/// Whether the steps saved on these messages changed anything; `None` with no message at all.
pub fn trail_changed(metas: &[Value]) -> Option<bool> {
    if metas.is_empty() {
        return None;
    }
    Some(metas.iter().any(|m| activity_steps(m).0.iter().any(step_changes)))
}

/// What a done item's fields come from.
#[derive(Debug, Default)]
pub struct DoneFacts {
    pub message: Option<Message>,
    pub why: Option<String>,
    /// `routine`, `task`, `ask` (a hand-off) or `request` (the person's message a team job answered).
    pub why_kind: Option<&'static str>,
    /// The task's side-effect ledger: `Some(true)` when a call that wasn't denied had an effect.
    pub ledger_changed: Option<bool>,
    /// Further result messages whose steps join the trail (a team job's members), with who.
    pub more: Vec<(Message, String)>,
    /// What the item came from (Iris's `source`): `{kind: "routine", routine_id, run_id, trigger}`,
    /// `{kind: "task", task_id}`, `{kind: "delegation", delegation_id}` or
    /// `{kind: "team_job", delegation_id}`; `null` when unknown.
    pub source: Value,
    /// What checked the result: the item's task (a review's own, a routine run's or hand-off's),
    /// or a team job's hand-off tasks. Empty when no task stands behind it.
    pub tasks: Vec<TaskChecks>,
}

/// The ledger's answer, else the trail's.
pub fn changed(ledger: Option<bool>, trail: Option<bool>) -> Option<bool> {
    match (ledger, trail) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), _) | (_, Some(false)) => Some(false),
        _ => None,
    }
}

fn link_of(m: &Message) -> Value {
    json!({"thread_id": m.thread_id, "channel_id": m.channel_id, "direct_kwant_id": m.direct_kwant_id})
}

fn context_of(m: &Message) -> Option<&str> {
    m.thread_id.as_deref().or(m.channel_id.as_deref()).or(m.direct_kwant_id.as_deref())
}

/// Adds the done fields and `unread` to an item.
pub fn apply(v: &mut Value, f: &DoneFacts, unread: bool) {
    let m = f.message.as_ref();
    let mut metas: Vec<Value> = m.map(meta_of).into_iter().collect();
    let (mut trail, mut dropped) = m.map(|m| trail_rows(&meta_of(m), None)).unwrap_or_default();
    for (msg, by) in &f.more {
        let meta = meta_of(msg);
        let (rows, d) = trail_rows(&meta, Some(by));
        trail.extend(rows);
        dropped += d;
        metas.push(meta);
    }
    trail.sort_by(|a, b| a["at"].as_str().unwrap_or("").cmp(b["at"].as_str().unwrap_or("")));
    if trail.len() > TRAIL_MAX {
        // The last steps: the ones that led to the answer
        dropped += (trail.len() - TRAIL_MAX) as i64;
        trail.drain(..trail.len() - TRAIL_MAX);
    }
    let changed = changed(f.ledger_changed, trail_changed(&metas));
    v["unread"] = json!(unread);
    v["result_message_id"] = json!(m.map(|m| m.id.clone()));
    v["result_conversation_id"] = json!(m.and_then(context_of));
    v["result_thread_id"] = json!(m.and_then(|m| m.thread_id.clone()));
    v["result_link"] = m.map(link_of).unwrap_or(Value::Null);
    v["source"] = f.source.clone();
    v["answer_excerpt"] = json!(m.and_then(|m| excerpt(&m.text)));
    v["trail"] = json!(trail);
    v["trail_dropped"] = json!(dropped);
    v["why"] = json!(f.why);
    v["why_kind"] = json!(f.why_kind);
    v["changed"] = json!(changed);
    v["changed_nothing"] = json!(changed.map(|c| !c));
    v["checked"] = checked(&f.tasks, m.map(meta_of).as_ref());
}

async fn message(state: &AppState, id: Option<&str>) -> Result<Option<Message>, DbError> {
    match id {
        Some(id) => state.db.get_message(id).await,
        None => Ok(None),
    }
}

/// A routine run's result and instruction.
async fn routine_facts(state: &AppState, run_id: &str) -> Result<DoneFacts, DbError> {
    let Some((message_id, instructions, routine_id, trigger)) = state.db.routine_run_result(run_id).await? else {
        return Ok(DoneFacts::default());
    };
    Ok(DoneFacts {
        message: message(state, message_id.as_deref()).await?,
        tasks: sourced_task(state, "routine", run_id).await?,
        why: why_of(&instructions, true),
        why_kind: Some("routine"),
        source: json!({"kind": "routine", "routine_id": routine_id, "run_id": run_id, "trigger": trigger}),
        ..Default::default()
    })
}

/// A hand-off's result and its ask.
async fn delegation_facts(state: &AppState, id: &str) -> Result<DoneFacts, DbError> {
    let Some(d) = state.db.get_delegation(id).await? else {
        return Ok(DoneFacts::default());
    };
    Ok(DoneFacts {
        message: message(state, d.result_message_id.as_deref()).await?,
        tasks: sourced_task(state, "delegation", &d.id).await?,
        why: why_of(&d.task, false),
        why_kind: Some("ask"),
        source: json!({"kind": "delegation", "delegation_id": d.id}),
        ..Default::default()
    })
}

/// A finished or failed away item (`delegation` or `routine_run`).
pub async fn away_facts(state: &AppState, kind: &str, id: &str) -> Result<DoneFacts, DbError> {
    match kind {
        "routine_run" => routine_facts(state, id).await,
        "delegation" => delegation_facts(state, id).await,
        _ => Ok(DoneFacts::default()),
    }
}

/// Whether the task's ledger saw a call with an effect go through; `None` with no rows.
async fn ledger_changed(state: &AppState, task_id: &str) -> Result<Option<bool>, DbError> {
    let calls = state.db.tool_calls_of(task_id).await?;
    if calls.is_empty() {
        return Ok(None);
    }
    Ok(Some(calls.iter().any(|c| c.effect != "none" && c.decision != "denied")))
}

/// A task in review: the routine run or hand-off it came from, else its owner's reply at the hand-in.
pub async fn task_facts(state: &AppState, task: &Task, owner_name: &str, in_review_since: DateTime<Utc>) -> Result<DoneFacts, DbError> {
    let mut f = match (task.source_kind.as_str(), task.source_id.as_deref()) {
        ("routine", Some(run)) => routine_facts(state, run).await?,
        ("delegation", Some(d)) => delegation_facts(state, d).await?,
        _ => DoneFacts::default(),
    };
    if f.message.is_none() {
        let context = task.thread_id.as_deref().or(task.channel_id.as_deref()).or(task.direct_kwant_id.as_deref());
        if let Some(c) = context {
            f.message = state.db.task_result_message(c, owner_name, in_review_since, Duration::minutes(REPLY_SLACK_MINUTES)).await?;
        }
    }
    if f.why.is_none() {
        f.why = why_of(&task.acceptance, false).or_else(|| why_of(&task.title, false));
        f.why_kind = f.why.as_ref().map(|_| "task");
    }
    f.ledger_changed = ledger_changed(state, &task.id).await?;
    // The review's own task (the one its routine run or hand-off made, when there is one)
    f.tasks = vec![task_checks(state, &task.id).await?];
    // A review links to its own task; the routine run it came from is still exact (`run_id`)
    f.source = match f.source.take() {
        Value::Object(mut o) if o.get("kind") == Some(&json!("routine")) => {
            o.insert("task_id".into(), json!(task.id));
            Value::Object(o)
        }
        _ => json!({"kind": "task", "task_id": task.id}),
    };
    Ok(f)
}

/// The arguments an approval's question names, in this order.
const ASK_FACTS: [&str; 3] = ["subject", "title", "name"];

/// An approval's own question for its Inbox title, from what the call is about (its masked
/// arguments' subject, title or name): `Approve “Spring sale”?`. `None` when they name nothing;
/// the title stays the tool's (`Use gmail`).
pub fn approval_question(details: &Value) -> Option<String> {
    let args = details["arguments"].as_object().or_else(|| details["input"].as_object())?;
    let fact = ASK_FACTS.iter().find_map(|k| args.get(*k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty() && *v != "[redacted]"))?;
    let fact = away::mask_words(fact);
    Some(format!("Approve “{}”?", cut(&fact, TITLE_CHARS - 12)))
}

// ---- Team jobs ----

/// The lead of a `Done:` reply in `space`: the direct chat's bot, else the bot of that name.
async fn lead_of(state: &AppState, r: &DoneReplyRow, space: &SpaceId) -> Result<Option<crate::db::models::Kwant>, DbError> {
    // Replies name their bot (older ones carried its id)
    let id = match r.direct_kwant_id.as_deref() {
        Some(k) => k.to_string(),
        None => state.db.kwant_id_named(&r.sender_id, space).await?.unwrap_or_else(|| r.sender_id.clone()),
    };
    let Some(k) = state.db.get_kwant(&id).await? else { return Ok(None) };
    let of = state.db.space_of_kwant(&k.id).await?.unwrap_or_else(SpaceId::default_space);
    Ok(((k.name == r.sender_id || k.id == r.sender_id) && &of == space).then_some(k))
}

fn done_at(meta: &Value, fallback: DateTime<Utc>) -> DateTime<Utc> {
    meta[DONE_KEY]["at"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(fallback)
}

/// A message row with only the stored columns this module reads.
fn bare_message(id: &str, text: &str, meta: Option<String>) -> Message {
    Message {
        id: id.to_string(),
        channel_id: None,
        direct_kwant_id: None,
        sender_role: "kwant".into(),
        sender_id: String::new(),
        text: text.to_string(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
        created_at: None,
        thread_id: None,
        meta,
        results: None,
        reply_to_message_id: None,
        reaction: None,
        reply_to: None,
    }
}

fn as_message(r: &DoneReplyRow) -> Message {
    Message {
        channel_id: r.channel_id.clone(),
        direct_kwant_id: r.direct_kwant_id.clone(),
        sender_id: r.sender_id.clone(),
        created_at: Some(r.created_at),
        thread_id: r.thread_id.clone(),
        ..bare_message(&r.id, &r.text, r.meta.clone())
    }
}

/// The space's `Done:` replies since `since` that count as team jobs (their lead is in the
/// space), with the lead. Newest first, at most [`MAX_ITEMS`].
async fn done_replies(state: &AppState, space: &SpaceId, since: DateTime<Utc>) -> Result<Vec<(DoneReplyRow, crate::db::models::Kwant)>, DbError> {
    let mut out = Vec::new();
    for r in state.db.done_replies(since, MAX_ITEMS * 4).await? {
        let meta: Value = r.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
        if !meta[DONE_KEY].is_object() {
            continue;
        }
        if let Some(lead) = lead_of(state, &r, space).await? {
            out.push((r, lead));
        }
        if out.len() >= MAX_ITEMS as usize {
            break;
        }
    }
    Ok(out)
}

/// The space's team jobs since `since`, as finished inbox items.
pub async fn team_jobs(state: &AppState, space: &SpaceId, since: DateTime<Utc>, seen: Option<DateTime<Utc>>) -> Result<Vec<Value>, DbError> {
    let mut items = Vec::new();
    for (r, lead) in done_replies(state, space, since).await? {
        let msg = as_message(&r);
        let meta = meta_of(&msg);
        let at = done_at(&meta, r.created_at);
        let context = context_of(&msg).unwrap_or(&lead.id).to_string();
        let request = state.db.last_person_message_before(&context, r.created_at).await?;
        let from = request.as_ref().and_then(|m| m.created_at);
        // The hand-offs the lead made from this chat for this request (with no request found,
        // the 7 days before the `Done:`)
        let from = from.unwrap_or(r.created_at - Duration::days(DONE_DAYS));
        let handoffs: Vec<_> = state
            .db
            .lead_delegations(&lead.id, from, r.created_at)
            .await?
            .into_iter()
            .filter(|d| [&d.origin_thread_id, &d.origin_channel_id, &d.origin_kwant_id].iter().any(|o| o.as_deref() == Some(context.as_str())))
            .collect();
        let mut members: Vec<Value> = Vec::new();
        let mut more = Vec::new();
        let mut tasks = Vec::new();
        for d in &handoffs {
            tasks.extend(sourced_task(state, "delegation", &d.id).await?);
            let name = state.db.get_kwant(&d.specialist_kwant_id).await?.map(|k| k.name).unwrap_or_else(|| "A bot".into());
            if !members.iter().any(|m| m["kwant_id"] == json!(d.specialist_kwant_id)) {
                members.push(json!({"kwant_id": d.specialist_kwant_id, "kwant_name": name}));
            }
            if let Some(m) = message(state, d.result_message_id.as_deref()).await? {
                more.push((m, name));
            }
        }
        let request_text = request.as_ref().map(|m| m.text.as_str()).unwrap_or("");
        let why = why_of(request_text, false);
        let done_text = meta[DONE_KEY]["text"].as_str().unwrap_or("").to_string();
        let title = why_of(request_text, false).map(|t| cut(&t, TITLE_CHARS)).unwrap_or_else(|| format!("Team job led by {}", lead.name));
        let unread = seen.is_none_or(|s| at > s);
        let mut v = json!({
            "kind": "team_job",
            "id": r.id,
            "kwant_id": lead.id,
            "kwant_name": lead.name,
            "title": title,
            "summary": away::summarize(&format!("Done: {done_text}")),
            "status": "completed",
            "timestamp": rfc3339(at),
            "link": link_of(&msg),
            "group": Group::Finished.key(),
            "sort_key": sort_key(Group::Finished, None, at.timestamp_millis(), unread, &lead.name, &r.id),
            "lead": {"kwant_id": lead.id, "kwant_name": lead.name},
            "members": members,
            "handoffs": handoffs.len(),
            "done_line": away::mask_words(&done_text),
        });
        let source = json!({"kind": "team_job", "delegation_id": handoffs.first().map(|d| d.id.clone())});
        let facts = DoneFacts { message: Some(msg), why: why.clone(), why_kind: why.as_ref().map(|_| "request"), ledger_changed: None, more, source, tasks };
        apply(&mut v, &facts, unread);
        items.push(v);
    }
    Ok(items)
}

// ---- Done, and here's the check (0.9.7 done-check) ----

/// Most `checked.items` shown; the rest are counted in `items_dropped`.
pub const CHECKED_MAX: usize = 8;
const CHECK_WHAT_CHARS: usize = 120;
const CHECK_EVIDENCE_CHARS: usize = 300;
const CHECK_LINE_CHARS: usize = 240;

/// One task's stored checks: its outcome checks, its contract's criteria and the parts of the
/// request they prove, the evidence still attached to it, and the names of the bots that reviewed it.
#[derive(Debug, Default, Clone)]
pub struct TaskChecks {
    pub checks: Vec<OutcomeCheck>,
    pub criteria: Vec<Criterion>,
    pub parts: Vec<Part>,
    pub evidence: Vec<Evidence>,
    pub names: HashMap<String, String>,
}

async fn task_checks(state: &AppState, task_id: &str) -> Result<TaskChecks, DbError> {
    let checks = state.db.list_checks(task_id).await?;
    let evidence = state.db.list_evidence(task_id).await?;
    let contract = state.db.get_contract(task_id).await?.and_then(|r| serde_json::from_str::<Contract>(&r.body).ok()).unwrap_or_default();
    let (criteria, parts) = (contract.criteria, contract.parts);
    let mut names = HashMap::new();
    for by in checks.iter().filter(|c| c.kind == "peer_review").filter_map(|c| c.checked_by.as_deref()) {
        if !names.contains_key(by) {
            if let Some(k) = state.db.get_kwant(by).await? {
                names.insert(by.to_string(), k.name);
            }
        }
    }
    Ok(TaskChecks { checks, criteria, parts, evidence, names })
}

/// The checks of the task a routine run or hand-off made, if it made one.
async fn sourced_task(state: &AppState, kind: &str, id: &str) -> Result<Vec<TaskChecks>, DbError> {
    match state.db.task_by_source(kind, id).await? {
        Some(t) => Ok(vec![task_checks(state, &t.id).await?]),
        None => Ok(Vec::new()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fed {
    Check,
    Criterion,
    Delivery,
}

/// One `checked.items` row before it is JSON; `claim` marks a criterion met with no evidence.
#[derive(Debug, Clone)]
struct CheckItem {
    from: Fed,
    what: String,
    how: &'static str,
    result: &'static str,
    evidence: Option<String>,
    claim: bool,
}

/// A text masked as the answer excerpt is, on one line, at most `max`.
fn masked_line(text: &str, max: usize) -> String {
    cut(mask_words_keep_urls(&redact(text)).trim(), max)
}

/// A check's output or evidence: masked line by line, blank lines dropped, at most
/// [`CHECK_EVIDENCE_CHARS`]; `None` when empty.
fn masked_text(text: &str) -> Option<String> {
    let lines: Vec<String> = redact(text).lines().map(|l| mask_words_keep_urls(l.trim())).filter(|l| !l.is_empty()).collect();
    (!lines.is_empty()).then(|| cut(&lines.join("\n"), CHECK_EVIDENCE_CHARS))
}

/// [`away::mask_words`], but an address stays readable: `mask_words` splits `https://host/a/42`
/// at the colon and masks the rest as a key once it is 20 characters with a digit, and an http
/// check's address is the very thing the card names. An address keeps its scheme; its host and
/// each piece of its path, query and fragment are masked on their own (sign-in details before an
/// `@` always), so a key in the path or a `token=` value still goes.
fn mask_words_keep_urls(text: &str) -> String {
    text.split_whitespace()
        .map(|w| match w.split_once("://") {
            Some((scheme, rest)) if scheme == "http" || scheme == "https" => mask_url(scheme, rest),
            _ => away::mask_words(w),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn mask_url(scheme: &str, rest: &str) -> String {
    let piece = |s: &str| if s.is_empty() || away::mask_words(s) == s { s.to_string() } else { "[redacted]".to_string() };
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(host_end);
    let host = match host.rsplit_once('@') {
        Some((_, h)) => format!("[redacted]@{}", piece(h)),
        None => piece(host),
    };
    let mut out = format!("{scheme}://{host}");
    let mut seg = String::new();
    for c in tail.chars() {
        if "/?&#".contains(c) {
            out.push_str(&piece(&seg));
            out.push(c);
            seg.clear();
        } else {
            seg.push(c);
        }
    }
    out.push_str(&piece(&seg));
    out
}

/// A stored check status as a result. Only `passed` passes; a check that couldn't run (`error`)
/// failed; one running or waiting on an outside service (#564's `waiting`) is `waiting`; anything
/// else (queued, or a status this code doesn't know) is `not_checked`, never `passed`.
fn check_result(status: &str) -> &'static str {
    match status {
        "passed" => "passed",
        "failed" | "error" => "failed",
        "running" | "waiting" => "waiting",
        _ => "not_checked",
    }
}

fn check_how(c: &OutcomeCheck) -> &'static str {
    match c.kind.as_str() {
        "command" => "ran the command",
        "http" => "opened the address",
        "file_exists" => "looked for the file",
        "file_contains" => "looked for the text in the file",
        "file_sha256" => "compared the file's fingerprint",
        "peer_review" if c.spec_value()["fresh_eyes"].as_bool() == Some(true) => "a fresh-eyes review",
        "peer_review" => "a teammate's review",
        "person" => "you checked it",
        _ => "a check",
    }
}

/// The check in words: its command, address or file; who reviewed; or the person's own check.
fn check_what(c: &OutcomeCheck, names: &HashMap<String, String>) -> String {
    let spec = redact_value(&c.spec_value());
    let field = |k: &str| spec[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    let what = match c.kind.as_str() {
        "command" => field("command").map(str::to_string),
        "http" => field("url").map(str::to_string),
        "file_contains" => field("path").map(|p| match field("text") {
            Some(t) => format!("{p} has “{t}”"),
            None => p.to_string(),
        }),
        "file_exists" | "file_sha256" => field("path").map(str::to_string),
        "peer_review" => Some(match c.checked_by.as_deref().and_then(|b| names.get(b)) {
            Some(n) => format!("Review by {n}"),
            None => "Review by a teammate".to_string(),
        }),
        "person" => Some("Your own check".to_string()),
        _ => None,
    };
    masked_line(&what.unwrap_or_else(|| c.kind.replace('_', " ")), CHECK_WHAT_CHARS)
}

fn check_items(t: &TaskChecks) -> Vec<CheckItem> {
    t.checks
        .iter()
        // A check waiting for approval never ran and doesn't count
        .filter(|c| c.approved)
        .map(|c| CheckItem {
            from: Fed::Check,
            what: check_what(c, &t.names),
            how: check_how(c),
            result: check_result(&c.status),
            evidence: c.output_excerpt.as_deref().and_then(masked_text),
            claim: false,
        })
        .collect()
}

/// A criterion's evidence in words: each attached evidence's label and first line, joined.
fn criterion_evidence(k: &Criterion, evidence: &[Evidence]) -> Option<String> {
    let parts: Vec<String> = k
        .evidence
        .iter()
        .filter_map(|id| evidence.iter().find(|e| &e.id == id))
        .map(|e| {
            let detail = e.uri.as_deref().or(e.path.as_deref()).or_else(|| e.excerpt.as_deref().and_then(|x| x.lines().find(|l| !l.trim().is_empty())));
            match detail {
                Some(d) => format!("{}: {}", e.label.trim(), d.trim()),
                None => e.label.trim().to_string(),
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; ")).and_then(|p| masked_text(&p))
}

/// The contract's criteria: met with evidence still on the task passed; met without it is a
/// claim, not checked (the contract's own rule); not met failed.
fn criterion_items(t: &TaskChecks) -> Vec<CheckItem> {
    let on_task: Vec<String> = t.evidence.iter().map(|e| e.id.clone()).collect();
    t.criteria
        .iter()
        .map(|k| {
            let claim = crate::server::missions::contract::is_claim(k, &on_task);
            let (how, result, evidence) = match k.status.as_str() {
                "met" if claim => ("task criterion, marked met with no evidence (a claim)", "not_checked", None),
                "met" => ("task criterion, met with evidence", "passed", criterion_evidence(k, &t.evidence)),
                "not_met" => ("task criterion, marked not met", "failed", criterion_evidence(k, &t.evidence)),
                _ => ("task criterion, not marked yet", "not_checked", None),
            };
            CheckItem { from: Fed::Criterion, what: masked_line(&k.text, CHECK_WHAT_CHARS), how, result, evidence, claim }
        })
        .collect()
}

/// What the person asked to see, in words, from what was missing or from the reply's plan.
fn delivery_thing(names: &[&str]) -> &'static str {
    match (names.contains(&"image"), names.contains(&"link")) {
        (true, true) => "picture and link",
        (true, false) => "picture",
        (false, true) => "link",
        _ => "result",
    }
}

/// The delivery check's verdict on the result message (`meta.delivery`, written by
/// `delivery_check::meta`): a hit passed, a miss failed with what is missing. No verdict (no plan
/// to check against) is no item.
fn delivery_item(meta: &Value) -> Option<CheckItem> {
    let d = &meta["delivery"];
    let hit = match d["outcome"].as_str()? {
        "hit" => true,
        "miss" => false,
        _ => return None,
    };
    let strs = |v: &Value| v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
    let missing = strs(&d["missing"]);
    let shown = strs(&meta[super::delivery_plan::META_KEY]["show"]);
    let names: Vec<&str> = if hit { shown.iter().map(String::as_str).collect() } else { missing.iter().map(String::as_str).filter(|m| *m != "follow_through").collect() };
    let thing = delivery_thing(&names);
    let evidence = match (hit, d["auto_shared"].as_bool() == Some(true)) {
        (true, true) => "kwant shared the bot's last screenshot under the reply".to_string(),
        (true, false) => format!("the {thing} is in the reply"),
        (false, _) => format!("missing from the reply: {thing}"),
    };
    Some(CheckItem {
        from: Fed::Delivery,
        what: format!("The {thing} you asked for"),
        how: "kwant compared the reply with what you asked for",
        result: if hit { "passed" } else { "failed" },
        evidence: Some(evidence),
        claim: false,
    })
}

fn state_of(items: &[CheckItem]) -> &'static str {
    let passed = items.iter().filter(|i| i.result == "passed").count();
    if items.iter().any(|i| i.result == "failed") {
        "failed"
    } else if passed > 0 && passed == items.len() {
        "checked"
    } else if passed > 0 {
        "partly"
    } else {
        "not_checked"
    }
}

/// The card's one line: the state, then a short phrase per source (checks, criteria, delivery).
/// With the request's parts written (`parts_lead`), they lead instead of the criteria.
fn checked_line(state: &str, items: &[CheckItem], parts_lead: Option<String>) -> String {
    if items.is_empty() && parts_lead.is_none() {
        return "Not checked: nothing verified this result.".into();
    }
    let count = |of: &[&CheckItem], r: &str| of.iter().filter(|i| i.result == r).count();
    let mut parts: Vec<String> = Vec::new();
    let checks: Vec<&CheckItem> = items.iter().filter(|i| i.from == Fed::Check).collect();
    match checks.len() {
        0 => {}
        1 => {
            let c = checks[0];
            let word = match c.result {
                "passed" => "passed",
                "failed" => "failed",
                "waiting" => "is still waiting",
                _ => "hasn't run",
            };
            parts.push(format!("{} {word}", cut(&c.what, 60)));
        }
        n => {
            let (p, f, w) = (count(&checks, "passed"), count(&checks, "failed"), count(&checks, "waiting"));
            parts.push(match (f, w) {
                (0, 0) => format!("{p} of {n} checks passed"),
                (0, w) => format!("{p} of {n} checks passed, {w} still waiting"),
                (f, _) => format!("{f} of {n} checks failed"),
            });
        }
    }
    let criteria: Vec<&CheckItem> = items.iter().filter(|i| i.from == Fed::Criterion).collect();
    let (met, not_met, claims) = (count(&criteria, "passed"), count(&criteria, "failed"), criteria.iter().filter(|i| i.claim).count());
    match criteria.len() {
        _ if parts_lead.is_some() => {}
        0 => {}
        1 if not_met == 1 => parts.push("the criterion is not met".into()),
        1 if met == 1 => parts.push("the criterion has evidence".into()),
        1 if claims == 1 => parts.push("the criterion is a claim with no evidence".into()),
        1 => parts.push("the criterion isn't checked".into()),
        n if not_met > 0 => parts.push(format!("{not_met} of {n} criteria not met")),
        n => parts.push(format!("{met} of {n} criteria have evidence")),
    }
    if let Some(d) = items.iter().find(|i| i.from == Fed::Delivery) {
        let thing = d.what.trim_start_matches("The ").to_string();
        parts.push(match d.result {
            "passed" => format!("the {thing} is in the reply"),
            _ => format!("the {thing} is missing"),
        });
    }
    let prefix = match state {
        "checked" => "Checked",
        "partly" => "Partly checked",
        "failed" => "Failed",
        _ => "Not checked",
    };
    let body = match (parts_lead, parts.is_empty()) {
        (Some(lead), true) => lead,
        (Some(lead), false) => format!("{lead}; {}", parts.join(", ")),
        (None, _) => parts.join(", "),
    };
    cut(&format!("{prefix}: {body}."), CHECK_LINE_CHARS)
}

/// One part of the request on the card (request-parts).
#[derive(Debug, Clone)]
struct PartRow {
    text: String,
    result: &'static str,
    criteria: Vec<Value>,
}

/// A task's parts with their results, and whether the lead or the person wrote them. A written
/// part passed only when every criterion linked to it passed by [`criterion_items`]' rules (so a
/// claim never counts); one criterion not met fails it; no linked criterion leaves it
/// `not_checked`. With no parts written each criterion is its own part, as before parts existed.
fn part_rows(t: &TaskChecks) -> (Vec<PartRow>, bool) {
    let rows: Vec<(&str, CheckItem)> = t.criteria.iter().map(|k| k.id.as_str()).zip(criterion_items(t)).collect();
    let crit = |id: &str, i: &CheckItem| json!({"id": id, "text": i.what, "result": i.result});
    if t.parts.is_empty() {
        let parts = rows.iter().map(|(id, i)| PartRow { text: i.what.clone(), result: i.result, criteria: vec![crit(id, i)] }).collect();
        return (parts, false);
    }
    let parts = t
        .parts
        .iter()
        .map(|p| {
            let linked: Vec<&(&str, CheckItem)> = p.criteria.iter().filter_map(|id| rows.iter().find(|(k, _)| k == id)).collect();
            let result = if linked.iter().any(|(_, i)| i.result == "failed") {
                "failed"
            } else if !linked.is_empty() && linked.iter().all(|(_, i)| i.result == "passed") {
                "passed"
            } else {
                "not_checked"
            };
            PartRow { text: masked_line(&p.text, CHECK_WHAT_CHARS), result, criteria: linked.iter().map(|(id, i)| crit(id, i)).collect() }
        })
        .collect();
    (parts, true)
}

/// The line's lead when parts are written: "3 of 4 parts checked; "send it to Ana": no check",
/// naming at most one part (a failed one first). `None` for a single part that passed: the
/// sources say it.
fn parts_phrase(parts: &[PartRow]) -> Option<String> {
    let named = parts
        .iter()
        .find(|p| p.result == "failed")
        .map(|p| format!("\"{}\": not met", cut(&p.text, 60)))
        .or_else(|| parts.iter().find(|p| p.result != "passed").map(|p| format!("\"{}\": no check", cut(&p.text, 60))));
    let passed = parts.iter().filter(|p| p.result == "passed").count();
    match (parts.len(), named) {
        (0, _) | (1, None) => None,
        (1, Some(n)) => Some(n),
        (total, None) => Some(format!("{passed} of {total} parts checked")),
        (total, Some(n)) => Some(format!("{passed} of {total} parts checked; {n}")),
    }
}

/// A done item's `checked`: `{state, items, items_dropped, line}`. `items` come only from what
/// kwant stored: each task's approved outcome checks and contract criteria, then the delivery
/// check's verdict on the result message. `state` is `failed` when any item failed, `checked`
/// when all passed, `partly` when some did, else `not_checked` (also with no items at all). At
/// most [`CHECKED_MAX`] items, failed first; the state and line count them all.
///
/// With criteria there are also `parts` (`{text, result, criteria}`, in the request's order),
/// `parts_checked`, `parts_total` and `parts_written`. A part of the request with no passing
/// check keeps the state from `checked`; written parts lead the line.
pub fn checked(tasks: &[TaskChecks], result_meta: Option<&Value>) -> Value {
    let mut items: Vec<CheckItem> = Vec::new();
    let (mut parts, mut written) = (Vec::new(), false);
    for t in tasks {
        items.extend(check_items(t));
        items.extend(criterion_items(t));
        let (p, w) = part_rows(t);
        parts.extend(p);
        written |= w;
    }
    items.extend(result_meta.and_then(delivery_item));
    let mut state = state_of(&items);
    if state == "checked" && parts.iter().any(|p| p.result != "passed") {
        state = "partly";
    }
    let line = checked_line(state, &items, if written { parts_phrase(&parts) } else { None });
    // Failed first, the rest in source order (a stable sort)
    let mut shown = items;
    shown.sort_by_key(|i| i.result != "failed");
    let dropped = shown.len().saturating_sub(CHECKED_MAX);
    shown.truncate(CHECKED_MAX);
    let rows: Vec<Value> = shown.iter().map(|i| json!({"what": i.what, "how": i.how, "result": i.result, "evidence": i.evidence})).collect();
    let mut out = json!({"state": state, "items": rows, "items_dropped": dropped, "line": line});
    if !parts.is_empty() {
        out["parts_checked"] = json!(parts.iter().filter(|p| p.result == "passed").count());
        out["parts_total"] = json!(parts.len());
        out["parts_written"] = json!(written);
        out["parts"] = Value::Array(parts.into_iter().map(|p| json!({"text": p.text, "result": p.result, "criteria": p.criteria})).collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_items_look_back_seven_days_whatever_was_seen() {
        let now = Utc::now();
        assert_eq!(done_since(now, None), now - Duration::days(7));
        // An older since is cut to the window; a later one narrows it
        assert_eq!(done_since(now, Some(now - Duration::days(30))), now - Duration::days(7));
        assert_eq!(done_since(now, Some(now - Duration::hours(1))), now - Duration::hours(1));
    }

    #[test]
    fn excerpts_are_plain_masked_and_short() {
        let e = excerpt("## Result\n\n\n**Site is up.** Key: sk-ant-api03-abcdefghijklmnop1234\n```\ncode\n```\n- `200` on /\n").unwrap();
        assert_eq!(e, "Result\n\nSite is up. Key: [redacted]\n200 on /");
        let long = excerpt(&"word ".repeat(300)).unwrap();
        assert_eq!(long.chars().count(), EXCERPT_CHARS);
        assert!(long.ends_with('…'));
        assert_eq!(excerpt("  \n```\n```"), None);
    }

    #[test]
    fn why_is_the_whole_instruction_or_a_first_line() {
        assert_eq!(why_of("Check booktun.ing is up.\nPost the status.", true).as_deref(), Some("Check booktun.ing is up.\nPost the status."));
        assert_eq!(why_of("\n# Ship the fix\nthen tell me", false).as_deref(), Some("Ship the fix"));
        assert_eq!(why_of(&"x".repeat(400), false).unwrap().chars().count(), WHY_CHARS);
        assert_eq!(why_of("  ", true), None);
    }

    #[test]
    fn reading_steps_change_nothing_and_writes_do() {
        let step = |tool: &str| json!({"kind": "tool", "status": "ok", "tool": tool, "title": "x", "started_at": "2026-11-12T10:00:00Z"});
        for t in ["Read", "Grep", "WebSearch", "WebFetch", "mcp__kwnt__browser_open", "mcp__kwnt__browser_page", "mcp__kwnt__task_done"] {
            assert!(!step_changes(&step(t)), "{t}");
        }
        for t in ["Bash", "Write", "Edit", "mcp__github__create_issue", "mcp__kwnt__browser_act", "mcp__kwnt__send_email"] {
            assert!(step_changes(&step(t)), "{t}");
        }
        assert!(!step_changes(&json!({"kind": "thinking", "status": "ok"})));
        assert!(!step_changes(&json!({"kind": "approval", "status": "declined", "tool": "Bash"})));
        assert!(step_changes(&json!({"kind": "tool", "status": "ok"})), "a step with no tool name may write");
    }

    #[test]
    fn the_trail_keeps_tools_and_pages_and_says_who() {
        let meta = json!({"activity": {"dropped": 2, "steps": [
            {"kind": "thinking", "status": "ok", "title": "Thinking", "started_at": "2026-11-12T10:00:00Z"},
            {"kind": "tool", "status": "ok", "tool": "WebFetch", "title": "Opening booktun.ing", "detail": "https://booktun.ing", "started_at": "2026-11-12T10:00:01Z"},
            {"kind": "tool", "status": "failed", "tool": "Bash", "title": "Running `curl`", "started_at": "2026-11-12T10:00:02Z"},
        ]}});
        let (rows, dropped) = trail_rows(&meta, Some("Ben"));
        assert_eq!(dropped, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], json!({"at": "2026-11-12T10:00:01Z", "kind": "page", "label": "Opening booktun.ing", "detail": "https://booktun.ing", "status": "ok", "by": "Ben"}));
        assert_eq!((rows[1]["kind"].as_str(), rows[1]["status"].as_str()), (Some("write"), Some("error")));
        assert_eq!(trail_status(Some("declined")), "skipped");
        assert_eq!(trail_status(Some("done")), "ok");
        assert_eq!(trail_changed(std::slice::from_ref(&meta)), Some(true));
        assert_eq!(trail_changed(&[json!({})]), Some(false), "a reply with no steps only answered");
        assert_eq!(trail_changed(&[]), None);
    }

    #[test]
    fn sending_tools_on_the_list_are_message_steps() {
        for t in [
            "mcp__kwnt__send_email",
            "mcp__gmail__send_email",
            "GMAIL_SEND_EMAIL",
            "SLACK_SENDS_A_MESSAGE_TO_A_SLACK_CHANNEL",
            "mcp__slack__slack_post_message",
            "TELEGRAM_SEND_MESSAGE",
            "TWITTER_CREATION_OF_A_POST",
            "mcp__agents__post_to_room",
            "mcp__agents__message_person",
        ] {
            assert!(is_message(t), "{t}");
        }
        // Off the list: still a change, but `write`
        for t in ["mcp__github__create_issue", "Bash", "SLACK_LIST_CHANNELS", "send_money"] {
            assert!(!is_message(t), "{t}");
        }
        let step = |tool: &str, status: &str| json!({"kind": "tool", "status": status, "tool": tool, "title": "t", "started_at": "2026-11-12T10:00:00Z"});
        let meta = json!({"activity": {"steps": [
            step("GMAIL_SEND_EMAIL", "ok"),
            step("mcp__github__create_issue", "ok"),
            step("mcp__agents__post_to_room", "failed"),
        ]}});
        let (rows, _) = trail_rows(&meta, None);
        let kinds: Vec<_> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["message", "write", "message"]);
        assert_eq!(trail_changed(std::slice::from_ref(&json!({"activity": {"steps": [step("TELEGRAM_SEND_MESSAGE", "ok")]}}))), Some(true), "a message counts as changed");
        // A send that was declined never ran: not a message, nothing changed
        let declined = json!({"kind": "approval", "status": "declined", "tool": "send_email"});
        assert!(!step_changes(&declined));
    }

    #[test]
    fn an_approval_asks_about_what_the_call_names() {
        let d = json!({"tool": "send_email", "arguments": {"to": "all@example.com", "subject": "Spring sale: 15% off"}});
        assert_eq!(approval_question(&d).as_deref(), Some("Approve “Spring sale: 15% off”?"));
        assert_eq!(approval_question(&json!({"arguments": {"title": "Fix login"}})).as_deref(), Some("Approve “Fix login”?"));
        assert_eq!(approval_question(&json!({"arguments": {"command": "rm -rf build"}})), None);
        assert_eq!(approval_question(&json!({"arguments": {"name": "[redacted]"}})), None);
        assert_eq!(approval_question(&Value::Null), None);
    }

    #[test]
    fn the_ledger_wins_over_the_trail() {
        assert_eq!(changed(Some(true), Some(false)), Some(true));
        assert_eq!(changed(Some(false), None), Some(false));
        assert_eq!(changed(None, Some(true)), Some(true));
        assert_eq!(changed(None, None), None);
    }

    #[test]
    fn apply_caps_the_trail_at_the_last_thirty_steps() {
        let steps: Vec<Value> = (0..40)
            .map(|i| json!({"kind": "tool", "status": "ok", "tool": "Read", "title": format!("Reading f{i:02}"), "started_at": format!("2026-11-12T10:00:{i:02}Z")}))
            .collect();
        let msg = Message { direct_kwant_id: Some("k1".into()), ..bare_message("m1", "All fine.", Some(json!({"activity": {"steps": steps}}).to_string())) };
        let mut v = json!({});
        apply(&mut v, &DoneFacts { message: Some(msg), why: Some("Check it".into()), why_kind: Some("routine"), ..Default::default() }, false);
        assert_eq!(v["trail"].as_array().unwrap().len(), TRAIL_MAX);
        assert_eq!(v["trail"][0]["label"], "Reading f10");
        assert_eq!(v["trail_dropped"], 10);
        assert_eq!((v["changed"].clone(), v["changed_nothing"].clone()), (json!(false), json!(true)));
        assert_eq!(v["result_message_id"], "m1");
        assert_eq!(v["result_conversation_id"], "k1");
        assert_eq!((v["result_thread_id"].clone(), v["source"].clone()), (Value::Null, Value::Null));
        assert_eq!(v["answer_excerpt"], "All fine.");
        assert_eq!(v["unread"], false);
        // No message: nothing invented
        let mut v = json!({});
        apply(&mut v, &DoneFacts::default(), true);
        assert_eq!((v["result_message_id"].clone(), v["changed"].clone(), v["trail"].clone()), (Value::Null, Value::Null, json!([])));
    }

    // ---- done-check ----

    fn check(kind: &str, spec: Value, status: &str, output: Option<&str>) -> OutcomeCheck {
        let now = Utc::now();
        OutcomeCheck {
            id: format!("chk-{kind}-{status}"),
            task_id: "t1".into(),
            kind: kind.into(),
            spec: spec.to_string(),
            required: true,
            approved: true,
            status: status.into(),
            output_excerpt: output.map(str::to_string),
            checked_by: Some("kwant".into()),
            requested_by: "person".into(),
            approved_by: Some("person".into()),
            evidence_id: None,
            created_at: now,
            started_at: Some(now),
            finished_at: Some(now),
            waiting_since: None,
            next_run_at: None,
            wait_until: None,
            runs: 0,
            wait_round: 0,
            timed_out_at: None,
        }
    }

    fn criterion(id: &str, text: &str, status: &str, evidence: &[&str]) -> Criterion {
        Criterion { id: id.into(), text: text.into(), status: status.into(), evidence: evidence.iter().map(|e| e.to_string()).collect(), by: "owner".into(), round_added: 1 }
    }

    fn evidence(id: &str, label: &str, uri: Option<&str>) -> Evidence {
        Evidence {
            id: id.into(),
            task_id: "t1".into(),
            kind: "link".into(),
            label: label.into(),
            uri: uri.map(str::to_string),
            path: None,
            sha256: None,
            bytes: None,
            excerpt: None,
            captured_by: "k1".into(),
            run_id: None,
            captured_at: Utc::now(),
        }
    }

    fn results(v: &Value) -> Vec<&str> {
        v["items"].as_array().unwrap().iter().map(|i| i["result"].as_str().unwrap()).collect()
    }

    #[test]
    fn a_passed_command_and_a_criterion_with_evidence_read_checked() {
        let t = TaskChecks {
            checks: vec![check("command", json!({"command": "cargo test"}), "passed", Some("running 12 tests\ntest result: ok. 12 passed"))],
            criteria: vec![criterion("c1", "The site answers 200", "met", &["ev1"])],
            evidence: vec![evidence("ev1", "Status page", Some("https://booktun.ing/health"))],
            ..Default::default()
        };
        let v = checked(&[t], None);
        assert_eq!(v["state"], "checked");
        assert_eq!(v["items"][0], json!({"what": "cargo test", "how": "ran the command", "result": "passed", "evidence": "running 12 tests\ntest result: ok. 12 passed"}));
        assert_eq!(v["items"][1], json!({"what": "The site answers 200", "how": "task criterion, met with evidence", "result": "passed", "evidence": "Status page: https://booktun.ing/health"}));
        assert_eq!(v["line"], "Checked: cargo test passed, the criterion has evidence.");
        assert_eq!(v["items_dropped"], 0);
    }

    #[test]
    fn a_criterion_met_without_evidence_is_a_claim_not_a_pass() {
        let t = TaskChecks {
            checks: vec![check("command", json!({"command": "cargo test"}), "passed", None)],
            criteria: vec![
                criterion("c1", "Tests pass", "met", &["ev1"]),
                criterion("c2", "Docs updated", "met", &[]),
                // Evidence that was taken off the task no longer counts
                criterion("c3", "Changelog entry", "met", &["gone"]),
            ],
            evidence: vec![evidence("ev1", "CI run", None)],
            ..Default::default()
        };
        let v = checked(&[t], None);
        assert_eq!(results(&v), ["passed", "passed", "not_checked", "not_checked"]);
        assert_eq!(v["items"][2]["how"], "task criterion, marked met with no evidence (a claim)");
        assert_eq!(v["items"][2]["evidence"], Value::Null);
        assert_eq!(v["state"], "partly");
        assert_eq!(v["line"], "Partly checked: cargo test passed, 1 of 3 criteria have evidence.");
        // Only a claim: nothing checked it
        let t = TaskChecks { criteria: vec![criterion("c1", "Docs updated", "met", &[])], ..Default::default() };
        let v = checked(&[t], None);
        assert_eq!((v["state"].as_str(), v["line"].as_str()), (Some("not_checked"), Some("Not checked: the criterion is a claim with no evidence.")));
    }

    #[test]
    fn a_failed_check_fails_the_item_and_comes_first() {
        let t = TaskChecks {
            checks: vec![
                check("http", json!({"url": "https://booktun.ing"}), "passed", Some("GET https://booktun.ing → 200")),
                check("file_exists", json!({"path": "dist/index.html"}), "failed", Some("dist/index.html is missing")),
            ],
            criteria: vec![criterion("c1", "Page is live", "met", &["ev1"])],
            evidence: vec![evidence("ev1", "Screenshot", None)],
            ..Default::default()
        };
        let v = checked(&[t], None);
        assert_eq!(v["state"], "failed");
        assert_eq!(results(&v), ["failed", "passed", "passed"]);
        assert_eq!(v["items"][0]["what"], "dist/index.html");
        assert_eq!(v["items"][0]["how"], "looked for the file");
        assert_eq!(v["line"], "Failed: 1 of 2 checks failed, the criterion has evidence.");
        // A check that couldn't run (`error`) and a criterion marked not met fail too
        let t = TaskChecks { checks: vec![check("command", json!({"command": "make"}), "error", Some("the mission's folder doesn't exist"))], ..Default::default() };
        assert_eq!(checked(&[t], None)["line"], "Failed: make failed.");
        let t = TaskChecks { criteria: vec![criterion("c1", "Under budget", "not_met", &[])], ..Default::default() };
        assert_eq!(checked(&[t], None)["line"], "Failed: the criterion is not met.");
    }

    #[test]
    fn a_routine_run_nothing_checked_says_so_plainly() {
        let msg = Message { direct_kwant_id: Some("k1".into()), ..bare_message("m1", "Posted the daily summary.", Some(json!({"activity": {"steps": []}}).to_string())) };
        let mut v = json!({});
        apply(&mut v, &DoneFacts { message: Some(msg), why: Some("Post the summary".into()), why_kind: Some("routine"), ..Default::default() }, true);
        assert_eq!(v["checked"], json!({"state": "not_checked", "items": [], "items_dropped": 0, "line": "Not checked: nothing verified this result."}));
        // No message and no task: the same
        let mut v = json!({});
        apply(&mut v, &DoneFacts::default(), true);
        assert_eq!(v["checked"]["state"], "not_checked");
        // A plan that wasn't checked (no outcome) is no item
        assert!(delivery_item(&json!({"delivery": {"outcome": null, "missing": ["follow_through"]}})).is_none());
    }

    #[test]
    fn a_delivery_miss_fails_and_names_what_is_missing() {
        let meta = json!({"delivery": {"outcome": "miss", "missing": ["image", "follow_through"], "auto_shared": false}, "delivery_plan": {"show": ["image", "text"]}});
        let v = checked(&[], Some(&meta));
        assert_eq!(v["state"], "failed");
        assert_eq!(v["items"][0], json!({"what": "The picture you asked for", "how": "kwant compared the reply with what you asked for", "result": "failed", "evidence": "missing from the reply: picture"}));
        assert_eq!(v["line"], "Failed: the picture you asked for is missing.");
        // A hit reads from the plan what was asked for
        let hit = json!({"delivery": {"outcome": "hit", "missing": []}, "delivery_plan": {"show": ["link"]}});
        let v = checked(&[], Some(&hit));
        assert_eq!((v["state"].as_str(), v["line"].as_str()), (Some("checked"), Some("Checked: the link you asked for is in the reply.")));
        assert_eq!(v["items"][0]["evidence"], "the link is in the reply");
    }

    #[test]
    fn an_unknown_or_unfinished_check_status_never_reads_as_passed() {
        for (status, want) in [("waiting", "waiting"), ("running", "waiting"), ("pending", "not_checked"), ("needs_approval", "not_checked"), ("passed_maybe", "not_checked"), ("", "not_checked")] {
            assert_eq!(check_result(status), want, "{status}");
        }
        let t = TaskChecks { checks: vec![check("http", json!({"url": "https://deutero.example/study/42"}), "waiting", None)], ..Default::default() };
        let v = checked(&[t], None);
        assert_eq!((v["state"].as_str(), v["line"].as_str()), (Some("not_checked"), Some("Not checked: https://deutero.example/study/42 is still waiting.")));
        // A check still waiting for approval never ran: no item
        let mut unapproved = check("command", json!({"command": "rm -rf build"}), "needs_approval", None);
        unapproved.approved = false;
        let v = checked(&[TaskChecks { checks: vec![unapproved], ..Default::default() }], None);
        assert_eq!(v["items"], json!([]));
    }

    #[test]
    fn an_address_stays_readable_but_a_key_in_it_is_masked() {
        let key = "Zq8VwK3mT9pL2rX7nB4c";
        assert_eq!(masked_line("https://github.com/alexsergivan/kwant/pull/566", 200), "https://github.com/alexsergivan/kwant/pull/566");
        assert_eq!(masked_line("opened https://deutero.example/study/42?round=2#top", 200), "opened https://deutero.example/study/42?round=2#top");
        assert_eq!(masked_line(&format!("https://hooks.example/in/{key}/x"), 200), "https://hooks.example/in/[redacted]/x");
        let q = masked_line("https://api.example/v1?token=hunter2hunter2&page=3", 200);
        assert!(q.starts_with("https://api.example/v1?") && q.ends_with("&page=3") && !q.contains("hunter2"), "{q}");
        let u = masked_line("https://bob:s3cret@db.example/", 200);
        assert!(u.starts_with("https://") && u.ends_with("@db.example/") && !u.contains("s3cret"), "{u}");
        assert_eq!(masked_line("key sk-ant-api03-abcdefghijklmnop1234 and http://x.example", 200), "key [redacted] and http://x.example");
        // Through an http check and a criterion's evidence
        let mut t = TaskChecks { checks: vec![check("http", json!({"url": "https://kwant.si/r/2026/11/report-42"}), "passed", Some("HTTP 200"))], ..Default::default() };
        t.evidence = vec![evidence("ev1", "Pull request", Some("https://github.com/alexsergivan/kwant/pull/566"))];
        t.criteria = vec![criterion("c1", "The pull request is open", "met", &["ev1"])];
        let v = checked(&[t], None);
        assert_eq!(v["items"][0]["what"], "https://kwant.si/r/2026/11/report-42");
        assert_eq!(v["items"][1]["evidence"], "Pull request: https://github.com/alexsergivan/kwant/pull/566");
    }

    #[test]
    fn checked_texts_are_masked_and_capped_at_eight_failed_first() {
        let key = "sk-ant-api03-abcdefghijklmnop1234";
        let mut checks: Vec<OutcomeCheck> = (0..9).map(|i| check("command", json!({"command": format!("test -f f{i}")}), "passed", None)).collect();
        checks.push(check("command", json!({"command": format!("curl -H 'x-api-key: {key}' https://api.example")}), "failed", Some(&format!("token {key} refused"))));
        let v = checked(&[TaskChecks { checks, ..Default::default() }], None);
        let text = v.to_string();
        assert!(!text.contains(key), "{text}");
        assert_eq!(v["items"].as_array().unwrap().len(), CHECKED_MAX);
        assert_eq!(v["items_dropped"], 2);
        assert_eq!(v["items"][0]["result"], "failed");
        assert_eq!(v["line"], "Failed: 1 of 10 checks failed.");
        // A teammate's review says who; the person's own check says so
        let mut review = check("peer_review", json!({"note": "Looks right", "fresh_eyes": true}), "passed", Some("Looks right"));
        review.checked_by = Some("k2".into());
        let names = HashMap::from([("k2".to_string(), "Rigel".to_string())]);
        let v = checked(&[TaskChecks { checks: vec![review, check("person", json!({"note": "ok"}), "passed", None)], names, ..Default::default() }], None);
        assert_eq!((v["items"][0]["what"].as_str(), v["items"][0]["how"].as_str()), (Some("Review by Rigel"), Some("a fresh-eyes review")));
        assert_eq!((v["items"][1]["what"].as_str(), v["items"][1]["how"].as_str()), (Some("Your own check"), Some("you checked it")));
        assert_eq!(v["line"], "Checked: 2 of 2 checks passed.");
    }

    // ---- request-parts ----

    fn part(id: &str, text: &str, criteria: &[&str]) -> Part {
        Part { id: id.into(), text: text.into(), criteria: criteria.iter().map(|c| c.to_string()).collect() }
    }

    fn part_results(v: &Value) -> Vec<&str> {
        v["parts"].as_array().unwrap().iter().map(|p| p["result"].as_str().unwrap()).collect()
    }

    #[test]
    fn a_part_with_no_check_keeps_the_card_from_reading_checked() {
        let t = TaskChecks {
            criteria: vec![criterion("c1", "PDF is in the folder", "met", &["ev1"])],
            parts: vec![part("p1", "make the PDF", &["c1"]), part("p2", "send it to Ana", &[])],
            evidence: vec![evidence("ev1", "report.pdf", None)],
            ..Default::default()
        };
        let v = checked(std::slice::from_ref(&t), None);
        assert_eq!(v["state"], "partly", "every item passed, but a part has no check");
        assert_eq!(v["line"], "Partly checked: 1 of 2 parts checked; \"send it to Ana\": no check.");
        assert_eq!((v["parts_checked"].as_i64(), v["parts_total"].as_i64(), v["parts_written"].as_bool()), (Some(1), Some(2), Some(true)));
        assert_eq!(v["parts"][0], json!({"text": "make the PDF", "result": "passed", "criteria": [{"id": "c1", "text": "PDF is in the folder", "result": "passed"}]}));
        assert_eq!(v["parts"][1], json!({"text": "send it to Ana", "result": "not_checked", "criteria": []}));
        // The same when part 2 has a criterion with no evidence yet, and the checks follow the parts
        let mut t2 = t.clone();
        t2.criteria.push(criterion("c2", "Ana has the email", "not_checked", &[]));
        t2.parts[1].criteria = vec!["c2".into()];
        t2.checks = vec![check("command", json!({"command": "test -f report.pdf"}), "passed", None)];
        let v = checked(&[t2], None);
        assert_eq!(v["state"], "partly");
        assert_eq!(v["line"], "Partly checked: 1 of 2 parts checked; \"send it to Ana\": no check; test -f report.pdf passed.");
        // Every part proved: checked
        let mut t3 = t;
        t3.criteria.push(criterion("c2", "Ana has the email", "met", &["ev2"]));
        t3.evidence.push(evidence("ev2", "Sent mail", None));
        t3.parts[1].criteria = vec!["c2".into()];
        let v = checked(&[t3], None);
        assert_eq!((v["state"].as_str(), v["line"].as_str()), (Some("checked"), Some("Checked: 2 of 2 parts checked.")));
    }

    #[test]
    fn a_part_proved_only_by_a_claim_is_not_checked() {
        let t = TaskChecks {
            criteria: vec![criterion("c1", "PDF is in the folder", "met", &["ev1"]), criterion("c2", "Ana has the email", "met", &[])],
            parts: vec![part("p1", "make the PDF", &["c1"]), part("p2", "send it to Ana", &["c2"])],
            evidence: vec![evidence("ev1", "report.pdf", None)],
            ..Default::default()
        };
        let v = checked(&[t], None);
        assert_eq!(part_results(&v), ["passed", "not_checked"]);
        assert_eq!(v["parts"][1]["criteria"][0]["result"], "not_checked");
        assert_eq!(v["state"], "partly");
        assert_eq!(v["line"], "Partly checked: 1 of 2 parts checked; \"send it to Ana\": no check.");
        // Nothing passed at all: not checked
        let t = TaskChecks { criteria: vec![criterion("c1", "Ana has the email", "met", &[])], parts: vec![part("p1", "make the PDF", &[]), part("p2", "send it to Ana", &["c1"])], ..Default::default() };
        let v = checked(&[t], None);
        assert_eq!((v["state"].as_str(), v["line"].as_str()), (Some("not_checked"), Some("Not checked: 0 of 2 parts checked; \"make the PDF\": no check.")));
    }

    #[test]
    fn a_part_with_a_criterion_not_met_fails() {
        let t = TaskChecks {
            criteria: vec![criterion("c1", "PDF is in the folder", "met", &["ev1"]), criterion("c2", "Ana has the email", "met", &["ev1"]), criterion("c3", "Under 2 MB", "not_met", &[])],
            parts: vec![part("p1", "make the PDF", &["c1", "c3"]), part("p2", "send it to Ana", &["c2"]), part("p3", "by Friday", &[])],
            evidence: vec![evidence("ev1", "report.pdf", None)],
            ..Default::default()
        };
        let v = checked(&[t], None);
        assert_eq!(part_results(&v), ["failed", "passed", "not_checked"]);
        assert_eq!(v["state"], "failed");
        assert_eq!(v["line"], "Failed: 1 of 3 parts checked; \"make the PDF\": not met.");
        // One written part: it is named, not counted
        let t = TaskChecks { criteria: vec![criterion("c1", "PDF is in the folder", "met", &["ev1"])], parts: vec![part("p1", "make the PDF and send it to Ana", &[])], evidence: vec![evidence("ev1", "report.pdf", None)], ..Default::default() };
        assert_eq!(checked(&[t], None)["line"], "Partly checked: \"make the PDF and send it to Ana\": no check.");
    }

    #[test]
    fn no_parts_written_reads_one_part_per_criterion_and_the_same_line() {
        let t = TaskChecks { criteria: vec![criterion("c1", "The site answers 200", "met", &["ev1"])], evidence: vec![evidence("ev1", "Status page", Some("https://booktun.ing/health"))], ..Default::default() };
        let v = checked(&[t], None);
        assert_eq!((v["state"].as_str(), v["line"].as_str()), (Some("checked"), Some("Checked: the criterion has evidence.")));
        assert_eq!(v["parts"], json!([{"text": "The site answers 200", "result": "passed", "criteria": [{"id": "c1", "text": "The site answers 200", "result": "passed"}]}]));
        assert_eq!((v["parts_checked"].as_i64(), v["parts_total"].as_i64(), v["parts_written"].as_bool()), (Some(1), Some(1), Some(false)));
        let t = TaskChecks { criteria: vec![criterion("c1", "Tests pass", "met", &["ev1"]), criterion("c2", "Docs updated", "met", &[])], evidence: vec![evidence("ev1", "CI run", None)], ..Default::default() };
        let v = checked(&[t], None);
        assert_eq!(v["line"], "Partly checked: 1 of 2 criteria have evidence.");
        assert_eq!(part_results(&v), ["passed", "not_checked"]);
        // No contract at all: no parts keys
        let v = checked(&[TaskChecks { checks: vec![check("command", json!({"command": "make"}), "passed", None)], ..Default::default() }], None);
        assert!(v.get("parts").is_none() && v.get("parts_total").is_none(), "{v}");
    }
}
