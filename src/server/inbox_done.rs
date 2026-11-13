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
//! A lead's reply that ended with `Done: …` (keep_going's `chain_done` meta) is a done item of
//! kind `team_job`, with the lead, the members it handed work to for that request, and the fields
//! above.

use super::away::{self, rfc3339, DEFAULT_LOOKBACK_DAYS, MAX_ITEMS};
use super::inbox::{sort_key, Group};
use super::keep_going::DONE_KEY;
use super::AppState;
use crate::db::away::DoneReplyRow;
use crate::db::missions::Task;
use crate::db::models::Message;
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::server::missions::checkpoints::NO_EFFECT_TOOLS;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

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
        for d in &handoffs {
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
        let facts = DoneFacts { message: Some(msg), why: why.clone(), why_kind: why.as_ref().map(|_| "request"), ledger_changed: None, more, source };
        apply(&mut v, &facts, unread);
        items.push(v);
    }
    Ok(items)
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
}
