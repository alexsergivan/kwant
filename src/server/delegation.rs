//! Threads and delegations: an agent's extra conversations, and a lead handing a task to a
//! specialist that runs in its own thread in the background and reports back to the chat it came
//! from. See `design-A.md` (batch 2) for the API and SSE contract.

use super::routes::{continue_once_if_capped, scope_for, try_kwant_turn, TurnOpts};
use super::AppState;
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use crate::db::spaces::SpaceId;
use crate::db::threads::{Delegation, NewDelegation, Thread, MAX_TASK_CHARS};
use crate::drivers::RuntimeEvent;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

/// A specialist may delegate once more; deeper chains are refused (no loops, bounded cost).
pub const MAX_DEPTH: i64 = 2;
/// Unfinished delegations one lead may have at a time.
pub const MAX_OPEN_PER_LEAD: i64 = 5;
/// An identical delegation (same lead, specialist, chat and task) within this many seconds is refused,
/// unless the earlier one failed, was cancelled or was interrupted.
pub const DEDUPE_SECS: i64 = 60;
/// How much of the specialist's reply is copied into the origin chat; the thread keeps it all.
pub const RESULT_CHARS: usize = 8000;
pub const FOLLOW_UP_SETTING: &str = "delegation_follow_up_minutes";
pub const DEFAULT_FOLLOW_UP_MINUTES: i64 = 15;
pub const MAX_FOLLOW_UP_MINUTES: i64 = 24 * 60;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn internal() -> Response {
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A specialist's reply as posted to the origin chat: clipped, saying where the rest is.
fn result_text(reply: &str) -> String {
    let full = reply.trim();
    let mut text = clip(full, RESULT_CHARS);
    // Counted in chars like `clip`: in bytes, the added "…" can make a clipped text look longer
    if text.chars().count() < full.chars().count() {
        text.push_str("\n\n(The full answer is in the delegation thread.)");
    }
    text
}

/// Minutes after which an unfinished delegation gets one "still working" note; 0 turns it off.
pub async fn follow_up_minutes(state: &AppState) -> i64 {
    state
        .db
        .get_setting(FOLLOW_UP_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .map(|m| m.clamp(0, MAX_FOLLOW_UP_MINUTES))
        .unwrap_or(DEFAULT_FOLLOW_UP_MINUTES)
}

fn emit_delegation(state: &AppState, d: &Delegation) {
    let _ = state.event_sender.send(RuntimeEvent::DelegationUpdated {
        delegation: serde_json::to_value(d).unwrap_or(Value::Null),
    });
}

fn emit_thread(state: &AppState, t: &Thread) {
    let _ = state.event_sender.send(RuntimeEvent::ThreadUpdated { thread: serde_json::to_value(t).unwrap_or(Value::Null) });
}

/// Posts a message from kwnt or an agent into the chat a delegation came from.
async fn post_to_origin(
    state: &AppState,
    d: &Delegation,
    role: &str,
    sender: &str,
    text: String,
    meta: Value,
) -> Option<String> {
    let msg = NewMessage {
        channel_id: d.origin_channel_id.clone(),
        direct_kwant_id: d.origin_kwant_id.clone(),
        sender_role: role.into(),
        sender_id: sender.into(),
        text,
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = MessageExtra { thread_id: d.origin_thread_id.clone(), meta: Some(meta) };
    let saved = state.db.insert_message_with(&msg, &extra).await.ok()?;
    if let Some(t) = &d.origin_thread_id {
        let _ = state.db.touch_thread(t).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    Some(saved.id)
}

fn meta(kind: &str, d: &Delegation, specialist: &str) -> Value {
    json!({
        "kind": kind,
        "delegation_id": d.id,
        "thread_id": d.thread_id,
        "specialist_id": d.specialist_kwant_id,
        "specialist_name": specialist,
        "lead_id": d.lead_kwant_id,
        "status": d.status,
        "title": d.title,
    })
}

async fn name_of(state: &AppState, id: &str) -> String {
    state.db.get_kwant(id).await.ok().flatten().map(|k| k.name).unwrap_or_else(|| "A bot".into())
}

// Starting

/// Where a delegation's result goes: exactly one of a channel, the lead's direct chat or a thread.
#[derive(Debug, Clone, Default)]
pub struct Origin {
    pub channel_id: Option<String>,
    pub kwant_id: Option<String>,
    pub thread_id: Option<String>,
}

pub struct StartRequest {
    pub lead: Kwant,
    pub specialist: Kwant,
    pub task: String,
    pub title: Option<String>,
    pub origin: Origin,
    pub continue_lead: bool,
    /// Depth of the chat the lead is in (0 for user chats).
    pub origin_depth: i64,
    /// Agents the lead may delegate to, when its chat has a member list.
    pub allowed: Option<Vec<String>>,
    /// Another open delegation of the same lead and chat whose result this task waits for: it is
    /// created parked and starts with that result once it completes (see [`release_dependents`]).
    pub after: Option<String>,
    /// The ended delegation whose result @mentioned this teammate (see
    /// [`Delegation::relayed_from_delegation_id`]).
    pub relayed_from: Option<String>,
}

/// Checks a request, creates the thread and the delegation, and starts the specialist's turn
/// in the background. Returns at once; the error is a message fit for the user or the agent.
pub async fn start(state: &AppState, req: StartRequest) -> Result<Delegation, (StatusCode, String)> {
    let bad = |m: &str| Err((StatusCode::BAD_REQUEST, m.to_string()));
    if req.lead.id == req.specialist.id {
        return bad("A bot can't delegate to itself.");
    }
    let task = req.task.trim();
    if task.is_empty() {
        return bad("task is required");
    }
    if task.chars().count() > MAX_TASK_CHARS {
        return bad(&format!("task is longer than {MAX_TASK_CHARS} characters"));
    }
    if req.origin_depth + 1 > MAX_DEPTH {
        return bad("This task was already delegated; it can't be delegated further.");
    }
    // Never across spaces, with or without a member list
    if req.specialist.space_id != req.lead.space_id {
        return Err((StatusCode::FORBIDDEN, format!("{} isn't in this space.", req.specialist.name)));
    }
    let space = SpaceId::new(&req.lead.space_id);
    if let Some(allowed) = &req.allowed {
        if !allowed.iter().any(|id| id == &req.specialist.id) {
            return Err((StatusCode::FORBIDDEN, format!("{} isn't in this chat.", req.specialist.name)));
        }
    }
    // No handing a task back to the bot that gave it: the reply in the thread already goes back to it
    if let Some(tid) = &req.origin.thread_id {
        if let Ok(Some(parent)) = state.db.delegation_for_thread(tid).await {
            if parent.lead_kwant_id == req.specialist.id {
                return bad(&format!(
                    "{} gave you this task, so don't hand it back. Do the work here; your reply goes back to {} automatically.",
                    req.specialist.name, req.specialist.name
                ));
            }
        }
    }
    let o = &req.origin;
    let set = [&o.channel_id, &o.kwant_id, &o.thread_id].iter().filter(|x| x.is_some()).count();
    if set != 1 {
        return bad("give exactly one of channel_id, kwant_id or thread_id");
    }
    // The origin must exist, and a direct chat or thread origin must be the lead's own
    let team_id = if let Some(cid) = &o.channel_id {
        let channel = state.db.get_channel(cid).await.ok().flatten().ok_or((StatusCode::NOT_FOUND, "channel not found".into()))?;
        if channel.space_id != space.as_str() {
            return Err((StatusCode::NOT_FOUND, "channel not found".into()));
        }
        match channel.name.strip_prefix("team-") {
            Some(name) => state.db.list_teams(&space).await.unwrap_or_default().into_iter().find(|t| t.name.eq_ignore_ascii_case(name)).map(|t| t.id),
            None => None,
        }
    } else if let Some(kid) = &o.kwant_id {
        if kid != &req.lead.id {
            return bad("kwant_id must be the lead's own direct chat");
        }
        None
    } else {
        let tid = o.thread_id.as_deref().unwrap_or_default();
        let thread = state.db.get_thread(tid).await.ok().flatten().ok_or((StatusCode::NOT_FOUND, "thread not found".into()))?;
        if thread.kwant_id != req.lead.id {
            return bad("thread_id must be one of the lead's own threads");
        }
        thread.team_id
    };
    // A repeat of the same request (a retried tool call, a reply naming the bot twice) starts nothing
    let since = Utc::now() - chrono::Duration::seconds(DEDUPE_SECS);
    if let Ok(Some(same)) = state
        .db
        .find_recent_delegation(&req.lead.id, &req.specialist.id, o.channel_id.as_deref(), o.kwant_id.as_deref(), o.thread_id.as_deref(), task, since)
        .await
    {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "{} was already given this exact task [{}] less than a minute ago ({}); it wasn't sent again.",
                req.specialist.name, same.id, same.status
            ),
        ));
    }
    let open = state.db.count_open_delegations(&req.lead.id).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error".into()))?;
    if open >= MAX_OPEN_PER_LEAD {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("{} already has {open} unfinished delegations; wait for one to finish.", req.lead.name),
        ));
    }
    let minutes = follow_up_minutes(state).await;
    let title = req.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(task);
    let (thread, d) = state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: req.lead.id.clone(),
            specialist_kwant_id: req.specialist.id.clone(),
            origin_channel_id: o.channel_id.clone(),
            origin_kwant_id: o.kwant_id.clone(),
            origin_thread_id: o.thread_id.clone(),
            team_id,
            task: task.to_string(),
            title: title.to_string(),
            continue_lead: req.continue_lead,
            depth: req.origin_depth + 1,
            follow_up_at: (minutes > 0).then(|| Utc::now() + chrono::Duration::minutes(minutes)),
        })
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error".into()))?;

    // Linked before it runs, so the lead's batch waits for it (see `finished_batch`)
    let d = match &req.relayed_from {
        Some(from) => {
            let _ = state.db.set_delegation_relayed_from(&d.id, from).await;
            state.db.get_delegation(&d.id).await.ok().flatten().unwrap_or(d)
        }
        None => d,
    };

    // Waits for a sibling of the same lead and chat that is still open; anything else is ignored
    let prereq = match &req.after {
        Some(id) => state
            .db
            .get_delegation(id)
            .await
            .ok()
            .flatten()
            .filter(|p| p.lead_kwant_id == d.lead_kwant_id && p.origin_context() == d.origin_context() && p.is_open()),
        None => None,
    };
    let d = match &prereq {
        Some(p) => {
            let _ = state.db.set_delegation_after(&d.id, &p.id).await;
            state.db.get_delegation(&d.id).await.ok().flatten().unwrap_or(d)
        }
        None => d,
    };

    // The task opens the thread, from the lead, so the specialist's history shows who asked
    let task_msg = NewMessage {
        channel_id: None,
        direct_kwant_id: None,
        sender_role: "kwant".into(),
        sender_id: req.lead.name.clone(),
        text: task.to_string(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let _ = state
        .db
        .insert_message_with(&task_msg, &MessageExtra { thread_id: Some(thread.id.clone()), meta: None })
        .await;
    let started = match &prereq {
        Some(p) => format!(
            "{} asked {} to work on “{}” once {} has finished “{}”. It waits until then, runs in its own thread, and the result will be posted here.",
            req.lead.name,
            req.specialist.name,
            d.title,
            name_of(state, &p.specialist_kwant_id).await,
            p.title
        ),
        None => format!("{} asked {} to work on “{}”. It runs in its own thread; the result will be posted here.", req.lead.name, req.specialist.name, d.title),
    };
    let mut m = meta("delegation_started", &d, &req.specialist.name);
    if let Some(p) = &prereq {
        m["after_delegation_id"] = json!(p.id);
    }
    post_to_origin(state, &d, "system", "kwnt", started, m).await;
    emit_thread(state, &thread);
    emit_delegation(state, &d);
    tracing::info!(delegation = %d.id, lead = %req.lead.id, specialist = %req.specialist.id, depth = d.depth, after = ?d.after_delegation_id, "delegation started");
    if let Some(p) = prereq {
        // It may have ended while this one was being set up
        if !state.db.get_delegation(&p.id).await.ok().flatten().is_some_and(|p| p.is_open()) {
            release_dependents(state, &p.id).await;
        }
        return Ok(state.db.get_delegation(&d.id).await.ok().flatten().unwrap_or(d));
    }

    spawn_run(state, d.clone(), req.lead, req.specialist);
    Ok(d)
}

/// Starts a delegation's run in the background. A plain function, so the futures of [`run`] and
/// [`release_dependents`] (which starts runs) don't contain each other.
fn spawn_run(state: &AppState, d: Delegation, lead: Kwant, specialist: Kwant) {
    let job = state.turns.start_job([d.id.clone(), d.thread_id.clone()]);
    let runner_state = state.clone();
    tokio::spawn(async move { run(runner_state, job, d, lead, specialist, RunAs::default()).await });
}

/// What a parked task gets of the result it waited for, at most.
const WAITED_RESULT_CHARS: usize = 4000;

/// The delegations parked until `prereq_id` ended (see [`StartRequest::after`]) go on, and so on
/// down the chain:
/// - it completed and asked the person nothing: each starts, with the result added to its task;
/// - it completed but waits for the person's answer: they stay parked, and the chat says so
///   (the answer continues it, and [`resume_after_answer`] calls this again);
/// - it failed, was stopped or cut short: they don't start and end as failed, saying why.
///
/// Returns how many started.
pub(super) async fn release_dependents(state: &AppState, prereq_id: &str) -> usize {
    let mut started = 0;
    let mut work = vec![prereq_id.to_string()];
    while let Some(id) = work.pop() {
        let Ok(Some(p)) = state.db.get_delegation(&id).await else { continue };
        if p.is_open() {
            continue;
        }
        let waiting = state.db.list_dependent_delegations(&p.id).await.unwrap_or_default();
        if waiting.is_empty() {
            continue;
        }
        let who = name_of(state, &p.specialist_kwant_id).await;
        let asked = open_question(state, &p).await;
        // A result whose last line asks the person: there is no card to answer, so nothing
        // would start the waiting task later; it ends as not started, saying why
        let result = if p.status == "completed" { result_of(state, &p).await } else { String::new() };
        let asked_in_reply = asked.is_none() && crate::router::sticky::asks_the_person(&result);
        for d in waiting {
            let (Ok(Some(lead)), Ok(Some(specialist))) =
                (state.db.get_kwant(&d.lead_kwant_id).await, state.db.get_kwant(&d.specialist_kwant_id).await)
            else {
                continue;
            };
            match (p.status.as_str(), &asked) {
                ("completed", None) if !asked_in_reply => {
                    let result = clip(&result, WAITED_RESULT_CHARS);
                    let task = clip(&format!("{}\n\n{who}'s result, which this task waited for:\n{result}", d.task), MAX_TASK_CHARS);
                    if !state.db.unpark_delegation(&d.id, &task).await.unwrap_or(false) {
                        continue;
                    }
                    let Ok(Some(d)) = state.db.get_delegation(&d.id).await else { continue };
                    post_to_origin(
                        state,
                        &d,
                        "system",
                        "kwnt",
                        format!("{who} has finished, so {} starts on “{}” now, with {who}'s result.", specialist.name, d.title),
                        meta("delegation_started", &d, &specialist.name),
                    )
                    .await;
                    emit_delegation(state, &d);
                    tracing::info!(delegation = %d.id, after = %p.id, "parked delegation started");
                    spawn_run(state, d, lead, specialist);
                    started += 1;
                }
                ("completed", Some(q)) => {
                    let mut m = meta("delegation_waiting", &d, &specialist.name);
                    m["after_delegation_id"] = json!(p.id);
                    post_to_origin(
                        state,
                        &d,
                        "system",
                        "kwnt",
                        format!(
                            "{} hasn't started “{}”: it waits for {who}'s result, and {who} asked you “{}” first. Once you answer, {who} continues and {} starts.",
                            specialist.name,
                            d.title,
                            clip(&q.question, 200),
                            specialist.name
                        ),
                        m,
                    )
                    .await;
                }
                (status, _) => {
                    let why = if asked_in_reply {
                        format!("not started: it waits for {who}'s result, and {who} asked you something first. Answer {who}, then ask for it again")
                    } else {
                        format!("not started: it waited for {who}'s result, and {who}'s task was {status}")
                    };
                    if !state.db.finish_delegation(&d.id, "failed", Some(&why), None).await.unwrap_or(false) {
                        continue;
                    }
                    let Ok(Some(done)) = state.db.get_delegation(&d.id).await else { continue };
                    if let Some(mid) = post_to_origin(
                        state,
                        &done,
                        "system",
                        "kwnt",
                        format!("{} didn't start “{}”: {}.", specialist.name, done.title, why.trim_start_matches("not started: ")),
                        meta("delegation_result", &done, &specialist.name),
                    )
                    .await
                    {
                        let _ = state.db.set_delegation_result_message(&done.id, &mid).await;
                    }
                    emit_delegation(state, &done);
                    work.push(done.id);
                }
            }
        }
    }
    started
}

/// Whether `d` is parked behind another delegation and hasn't started.
pub(super) fn is_parked(d: &Delegation) -> bool {
    d.status == "queued" && d.after_delegation_id.is_some()
}

/// The background part: the specialist's turn in the thread, then the report in the origin chat.
async fn run(state: AppState, job: super::turns::JobGuard, d: Delegation, lead: Kwant, specialist: Kwant, how: RunAs) {
    let RunAs { retried, after_restart, carry_on } = how;
    let carried_on = carry_on.is_some();
    let scope = match scope_for(&state, &specialist, None, Some(d.thread_id.clone()), &job, d.depth).await {
        Ok(s) => s,
        Err(why) => return finish(&state, &d, &specialist, Err(why), carried_on).await,
    };
    // Queued until the specialist's turn gets its slot: a busy specialist runs one turn at a time
    let (started_state, started_id) = (state.clone(), d.id.clone());
    let on_started = Box::pin(async move {
        if started_state.db.mark_delegation_running(&started_id).await.unwrap_or(false) {
            if let Ok(Some(running)) = started_state.db.get_delegation(&started_id).await {
                emit_delegation(&started_state, &running);
            }
        }
    });
    let opening = match &carry_on {
        Some(why) => format!(
            "{lead} delegated this task to you earlier, and it stopped before it was done ({why}). The person sent it \
            back to you: carry on from where you stopped. Your earlier work is above in this thread, so check what is \
            already done instead of starting over, and don't repeat a step that can't be undone (sending, posting, deleting, \
            paying) unless you've confirmed it didn't happen. When you finish, your reply is posted back to {lead} and the chat it \
            came from, so make it a complete, self-contained answer.",
            lead = lead.name,
            why = clip(why, 300),
        ),
        None => format!(
            "{lead} delegated this task to you. Work on it here; when you finish, your reply is posted back to {lead} \
            and the chat it came from, so make it a complete, self-contained answer.",
            lead = lead.name,
        ),
    };
    let prompt = format!(
        "{opening}\n\n{task}{origin}{again}",
        task = crate::drivers::peer_message(&lead.name, &format!("Task: {}\n\n{}", d.title, d.task)),
        origin = origin_brief(&state, &d).await,
        again = if after_restart { RESTARTED_NOTE } else { "" },
    );
    let mut opts = TurnOpts::default();
    opts.on_started = Some(on_started);
    opts.retried = retried;
    opts.delegation_id = Some(d.id.clone());
    let outcome = try_kwant_turn(&scope, &specialist, prompt, opts).await;
    let outcome = continue_once_if_capped(&scope, &specialist, outcome, false).await;
    let outcome = match outcome {
        Err(_) if job.token.is_cancelled() => Err("stopped".to_string()),
        other => other,
    };
    drop(job);
    // A usage limit stopped it before anything happened: it stays open and runs again when the
    // limit resets
    if outcome.is_err() {
        if let Ok(Some(p)) = state.db.parked_for_delegation(&d.id).await {
            post_to_origin(
                &state,
                &d,
                "system",
                "kwnt",
                format!(
                    "{} hit a usage limit on “{}”. {}",
                    specialist.name,
                    d.title,
                    super::parked::retry_line(p.retry_at, Utc::now())
                ),
                meta("delegation_started", &d, &specialist.name),
            )
            .await;
            return;
        }
    }
    finish(&state, &d, &specialist, outcome, carried_on).await;
}

/// How [`run`] runs a delegation's task.
#[derive(Default)]
struct RunAs {
    /// The run after a usage limit reset, which never waits for one again.
    retried: bool,
    /// A restart cut an earlier run short, so it may have done part of the work already (the
    /// prompt says so).
    after_restart: bool,
    /// The person sent an ended task back (unblocked or retried its board card): why it stopped.
    /// The specialist carries on in its thread, and the lead hears the new result on its own.
    carry_on: Option<String>,
}

/// Most of the person's request, and of the origin chat's instructions, a delegated task quotes.
const BRIEF_CHARS: usize = 2000;
/// Most files of the origin chat a delegated task lists.
const BRIEF_FILES: i64 = 10;

/// The chat a delegation came from, when its specialist may see it: the lead's own direct chat or
/// thread, or a channel the specialist is a member of (the members a turn there would have).
async fn visible_origin(state: &AppState, d: &Delegation) -> Option<String> {
    if let Some(id) = &d.origin_channel_id {
        let channel = state.db.get_channel(id).await.ok().flatten()?;
        let space = SpaceId::new(&channel.space_id);
        let all = state.db.list_kwants(&space).await.ok()?;
        let team = match channel.name.strip_prefix("team-") {
            Some(name) => state.db.list_teams(&space).await.ok()?.into_iter().find(|t| t.name.eq_ignore_ascii_case(name)),
            None => None,
        };
        let members = super::routes::channel_member_ids(&channel, team.as_ref(), &all);
        return members.contains(&d.specialist_kwant_id).then(|| id.clone());
    }
    if let Some(id) = &d.origin_thread_id {
        let thread = state.db.get_thread(id).await.ok().flatten()?;
        return (thread.kwant_id == d.lead_kwant_id).then(|| id.clone());
    }
    d.origin_kwant_id.clone().filter(|k| *k == d.lead_kwant_id)
}

/// The origin chat of the delegation in `thread_id`, when `kwant_id` is its specialist and may
/// see that chat: its files are readable from the task's thread.
pub(super) async fn shared_origin(state: &AppState, kwant_id: &str, thread_id: &str) -> Option<String> {
    let d = state.db.delegation_for_thread(thread_id).await.ok().flatten()?;
    if d.specialist_kwant_id != kwant_id {
        return None;
    }
    visible_origin(state, &d).await
}

/// What the specialist should know of the chat the task came from: the person's last request
/// before it, that chat's instructions and its files. Empty when the specialist may not see it.
async fn origin_brief(state: &AppState, d: &Delegation) -> String {
    let Some(ctx) = visible_origin(state, d).await else {
        return String::new();
    };
    let mut parts = Vec::new();
    let recent = state.db.list_messages(&ctx, 20).await.unwrap_or_default();
    let asked = recent
        .iter()
        .rev()
        .find(|m| m.sender_role == "user" && m.created_at.is_none_or(|t| t <= d.created_at) && !m.text.trim().is_empty());
    if let Some(m) = asked {
        parts.push(format!("[What the person asked in the chat this task came from]\n{}", clip(m.text.trim(), BRIEF_CHARS)));
    }
    if let Some(id) = &d.origin_channel_id {
        if let Ok(Some(text)) = state.db.channel_instructions(id).await {
            if !text.trim().is_empty() {
                parts.push(format!("[That chat's pinned instructions]\n{}", clip(text.trim(), BRIEF_CHARS)));
            }
        }
    }
    let files = state.db.list_attachments_in(&ctx, BRIEF_FILES).await.unwrap_or_default();
    if !files.is_empty() {
        let lines: Vec<String> = files.iter().map(|a| format!("- {} ({}) id {}", a.name, a.mime, a.id)).collect();
        parts.push(format!(
            "[Files in that chat; read one with `read_attachment` and its id]\n{}",
            lines.join("\n")
        ));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("\n\n{}", parts.join("\n\n"))
}

/// Added to a delegated task that runs again because kwnt restarted in the middle of it.
pub const RESTARTED_NOTE: &str = "\n\n[kwnt restarted while an earlier run of this task was in progress, so it may have partly run: \
files may already be written, messages sent or settings changed. Check what is already done before redoing anything, \
and don't repeat a step that can't be undone (sending, posting, deleting, paying) unless you've confirmed it didn't happen.]";

/// Ends `d` with `outcome`: the result in the origin chat, then the lead (with its batch, or on
/// its own when the person sent the task back: `carried_on`).
async fn finish(state: &AppState, d: &Delegation, specialist: &Kwant, outcome: Result<String, String>, carried_on: bool) {
    let (status, error) = match &outcome {
        Ok(_) => ("completed", None),
        Err(e) if e == "stopped" => ("cancelled", Some("stopped by the user".to_string())),
        Err(e) => ("failed", Some(clip(e, 500))),
    };
    // Someone may have cancelled it meanwhile; then the cancel already reported it
    if !state.db.finish_delegation(&d.id, status, error.as_deref(), None).await.unwrap_or(false) {
        return;
    }
    let Ok(Some(done)) = state.db.get_delegation(&d.id).await else {
        return;
    };
    let text = match &outcome {
        Ok(reply) => result_text(reply),
        Err(_) if status == "cancelled" => format!("Stopped work on “{}”.", done.title),
        Err(e) => format!("Couldn't finish “{}”: {}", done.title, clip(e, 500)),
    };
    let result_id = post_to_origin(state, &done, "kwant", &specialist.name, text, meta("delegation_result", &done, &specialist.name)).await;
    if let Some(id) = &result_id {
        let _ = state.db.set_delegation_result_message(&done.id, id).await;
    }
    // The specialist asked the person something and ended its turn waiting: say where to answer
    if let Some(q) = open_question(state, &done).await {
        post_to_origin(
            state,
            &done,
            "system",
            "kwnt",
            format!(
                "{} is waiting for your answer to “{}” in the thread of “{}”. Answer it there; the result will come back here.",
                specialist.name,
                clip(&q.question, 200),
                done.title
            ),
            meta("delegation_waiting", &done, &specialist.name),
        )
        .await;
    }
    let done = state.db.get_delegation(&d.id).await.ok().flatten().unwrap_or(done);
    emit_delegation(state, &done);
    tracing::info!(delegation = %done.id, status, "delegation finished");
    let lead_name = name_of(state, &done.lead_kwant_id).await;
    super::continuity::notify(
        state,
        super::continuity::Notification {
            kind: "delegation_finished",
            kwant_id: specialist.id.clone(),
            kwant_name: specialist.name.clone(),
            channel_id: done.origin_channel_id.clone(),
            title: format!("{} finished a task for {lead_name}", specialist.name),
            body: format!("“{}”: {status}.", clip(&done.title, 80)),
        },
    )
    .await;
    release_dependents(state, &done.id).await;
    if !carried_on {
        // Teammates the result @mentions start first, so the lead's batch waits for them too
        if let Ok(reply) = &outcome {
            relay_mentions(state, &done, specialist, reply).await;
        }
        return report_to_lead(state, &done, None).await;
    }
    // Its batch already went to the lead: this result goes on its own (a stop was the person's)
    if done.continue_lead && status != "cancelled" {
        let opening = format!("{} carried on with the task you delegated (“{}”) after the person sent it back.", specialist.name, done.title);
        continue_lead(state, &done, std::slice::from_ref(&done), Some(opening)).await;
    }
}

/// How many relays back [`relay_chain`] looks; a chain is bounded by the hand-off caps anyway.
const MAX_RELAY_CHAIN: usize = 32;

/// Hands on the @mentions of other teammates in a completed result posted to a team channel
/// (Oleks, 2026-10-29: QA's result said "@Frontend could switch to …" and Frontend never heard).
/// Same rules as a lead's mentions ([`super::routes::delegate_mentions`]): per-bot and
/// per-request caps, a notice for each one not sent, never back to a bot in this chain.
async fn relay_mentions(state: &AppState, done: &Delegation, specialist: &Kwant, reply: &str) {
    let Some(cid) = done.origin_channel_id.clone() else { return };
    if !done.continue_lead || done.status != "completed" || !reply.contains('@') {
        return;
    }
    let Ok(Some(lead)) = state.db.get_kwant(&done.lead_kwant_id).await else { return };
    let job = state.turns.start_job([done.id.clone()]);
    let Ok(scope) = scope_for(state, &lead, Some(cid), None, &job, done.depth - 1).await else { return };
    if !scope.teammates.iter().any(|k| k.id == specialist.id) {
        return;
    }
    let user_prompt = state
        .db
        .list_messages(done.origin_context().unwrap_or(&lead.id), 100)
        .await
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|m| m.sender_role == "user")
        .map(|m| m.text)
        .unwrap_or_else(|| done.task.clone());
    let end = |d: &Delegation| d.ended_at.unwrap_or(d.created_at);
    let alongside = state
        .db
        .list_delegations(Some(&done.lead_kwant_id), None, 50)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.continue_lead && s.origin_context() == done.origin_context())
        .filter(|s| s.is_open() || (s.created_at <= end(done) && end(s) >= done.created_at))
        .map(|s| s.specialist_kwant_id)
        .collect();
    let relay = super::routes::Relay { from: specialist, source: done, chain: relay_chain(state, done).await, alongside };
    super::routes::delegate_mentions(&scope, &lead, reply, &user_prompt, &[], None, Some(&relay)).await;
}

/// The teammates of `d` and of every delegation it was relayed from.
async fn relay_chain(state: &AppState, d: &Delegation) -> Vec<String> {
    let mut chain = vec![d.specialist_kwant_id.clone()];
    let mut from = d.relayed_from_delegation_id.clone();
    while let Some(id) = from.take() {
        if chain.len() > MAX_RELAY_CHAIN {
            break;
        }
        let Ok(Some(p)) = state.db.get_delegation(&id).await else { break };
        if !chain.contains(&p.specialist_kwant_id) {
            chain.push(p.specialist_kwant_id.clone());
        }
        from = p.relayed_from_delegation_id.clone();
    }
    chain
}

/// Resumes the lead once `d` ended, if it was the last of its batch to end (see
/// [`finished_batch`]). `opening` replaces the first line of the lead's prompt.
async fn report_to_lead(state: &AppState, d: &Delegation, opening: Option<String>) {
    if !d.continue_lead {
        return;
    }
    if let Some(batch) = finished_batch(state, d).await {
        continue_lead(state, d, &batch, opening).await;
    }
}

/// [`report_to_lead`] in the background, for callers that must not wait for the lead's turn.
fn spawn_report_to_lead(state: &AppState, d: Delegation) {
    let state = state.clone();
    tokio::spawn(async move { report_to_lead(&state, &d, None).await });
}

/// A question the specialist asked in the delegation's thread that the person hasn't answered.
async fn open_question(state: &AppState, d: &Delegation) -> Option<crate::db::questions::Question> {
    state
        .db
        .list_questions(Some("open"), Some(&d.thread_id), None)
        .await
        .ok()?
        .into_iter()
        .find(|q| q.kwant_id == d.specialist_kwant_id && q.thread_id.as_deref() == Some(d.thread_id.as_str()))
}

/// A late answer to a question the specialist asked in an ended delegation's thread: the
/// specialist continues there, its new result is posted to the origin chat, and the lead gets
/// a turn with it. `false` when the delegation can't continue (a bot is gone, or it is still
/// running and the answer is just a new turn in its thread).
pub async fn resume_after_answer(state: &AppState, d: Delegation, question_id: &str, prompt: String) -> bool {
    if d.is_open() {
        return false;
    }
    let (Ok(Some(_)), Ok(Some(specialist))) =
        (state.db.get_kwant(&d.lead_kwant_id).await, state.db.get_kwant(&d.specialist_kwant_id).await)
    else {
        return false;
    };
    let job = state.turns.start_job([d.id.clone(), d.thread_id.clone(), question_id.to_string()]);
    let state = state.clone();
    tokio::spawn(async move {
        let scope = match scope_for(&state, &specialist, None, Some(d.thread_id.clone()), &job, d.depth).await {
            Ok(s) => s,
            Err(_) => return,
        };
        let outcome = try_kwant_turn(&scope, &specialist, prompt, TurnOpts::default()).await;
        let outcome = continue_once_if_capped(&scope, &specialist, outcome, false).await;
        let stopped = job.token.is_cancelled();
        drop(job);
        let reply = match outcome {
            Ok(reply) => reply,
            // A failed turn already said why in the thread; a stopped one was the person's choice
            Err(e) if stopped => return tracing::info!(delegation = %d.id, error = %e, "late answer's turn stopped"),
            Err(e) => format!("Couldn't finish “{}” after your answer: {}", d.title, clip(&e, 500)),
        };
        let text = result_text(&reply);
        let mut m = meta("delegation_result", &d, &specialist.name);
        m["late"] = json!(true);
        if let Some(id) = post_to_origin(&state, &d, "kwant", &specialist.name, text, m).await {
            let _ = state.db.set_delegation_result_message(&d.id, &id).await;
        }
        tracing::info!(delegation = %d.id, "delegation continued after a late answer");
        // A teammate waited for this result: it starts now, and the lead hears from both when it ends
        if release_dependents(&state, &d.id).await > 0 {
            return;
        }
        if d.continue_lead {
            let opening = format!(
                "{} got the person's answer to its question and continued the task you delegated (“{}”).",
                specialist.name, d.title
            );
            continue_lead(&state, &d, std::slice::from_ref(&d), Some(opening)).await;
        }
    });
    true
}

/// Batches whose lead was already resumed, by their sorted ids (see [`finished_batch`]).
static RESUMED_BATCHES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();

/// The delegations a lead sent from one chat that ran at the same time as `d`, once all of them
/// have ended. `None` while one is still open, or when the batch was already claimed: the first
/// finisher to see every one of them ended resumes the lead, once, with every result.
///
/// Not "the one with the latest `ended_at`": that time is taken before the row is written, so
/// under write contention the latest one could check while a sibling was still open, the
/// sibling then saw it wasn't the latest, and nobody resumed the lead (1 run in 5 of a 4-bot
/// team on 2026-10-24: every teammate replied and the lead never answered).
async fn finished_batch(state: &AppState, d: &Delegation) -> Option<Vec<Delegation>> {
    let siblings: Vec<Delegation> = state
        .db
        .list_delegations(Some(&d.lead_kwant_id), None, 50)
        .await
        .ok()?
        .into_iter()
        // A task parked behind one that waits for the person isn't running: the lead reports
        // the rest now, and that task comes back with its own batch once it ran
        .filter(|s| s.continue_lead && s.origin_context() == d.origin_context() && !is_parked(s))
        .collect();
    if siblings.iter().any(Delegation::is_open) {
        return None;
    }
    let end = |s: &Delegation| s.ended_at.unwrap_or(s.created_at);
    // Grow from `d` to every delegation whose run overlaps the batch's span, and to the results
    // that relayed work to a batch member or were relayed from one (they ran one after the other)
    let mut batch = vec![siblings.iter().find(|s| s.id == d.id)?.clone()];
    let (mut lo, mut hi) = (d.created_at, end(d));
    let linked = |s: &Delegation, batch: &[Delegation]| {
        batch.iter().any(|b| {
            s.relayed_from_delegation_id.as_deref() == Some(b.id.as_str()) || b.relayed_from_delegation_id.as_deref() == Some(s.id.as_str())
        })
    };
    loop {
        let more: Vec<Delegation> = siblings
            .iter()
            .filter(|s| !batch.iter().any(|b| b.id == s.id) && ((s.created_at <= hi && end(s) >= lo) || linked(s, &batch)))
            .cloned()
            .collect();
        if more.is_empty() {
            break;
        }
        for s in more {
            lo = lo.min(s.created_at);
            hi = hi.max(end(&s));
            batch.push(s);
        }
    }
    // A batch the person stopped entirely stays stopped; anything else (a result, a failure, a
    // restart cutting it short) is the lead's to report
    if batch.iter().all(|b| b.status == "cancelled") {
        return None;
    }
    let mut ids: Vec<&str> = batch.iter().map(|b| b.id.as_str()).collect();
    ids.sort_unstable();
    let mut claimed = RESUMED_BATCHES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if claimed.len() > 10_000 {
        claimed.clear();
    }
    if !claimed.insert(ids.join(",")) {
        return None;
    }
    drop(claimed);
    batch.sort_by_key(|b| b.created_at);
    Some(batch)
}

/// The specialist's last reply in a delegation thread.
async fn result_of(state: &AppState, d: &Delegation) -> String {
    let lead = name_of(state, &d.lead_kwant_id).await;
    state
        .db
        .list_messages(&d.thread_id, 100)
        .await
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|m| m.sender_role == "kwant" && m.sender_id != lead)
        .map(|m| m.text)
        .unwrap_or_else(|| "(The result is in the chat above.)".into())
}

/// Gives the lead one turn in the origin chat with every result of the batch, so it checks them
/// and gives the user one answer. Tasks that didn't complete say what the lead should do about it.
async fn continue_lead(state: &AppState, d: &Delegation, batch: &[Delegation], opening: Option<String>) {
    let Ok(Some(lead)) = state.db.get_kwant(&d.lead_kwant_id).await else {
        return;
    };
    let context = d.origin_context().unwrap_or(&lead.id).to_string();
    let job = state.turns.start_job([context, d.id.clone()]);
    let Ok(scope) = scope_for(state, &lead, d.origin_channel_id.clone(), d.origin_thread_id.clone(), &job, d.depth - 1).await else {
        return;
    };
    let budget = 4000 / batch.len().max(1);
    let mut results = String::new();
    for b in batch {
        let who = name_of(state, &b.specialist_kwant_id).await;
        let body = match b.status.as_str() {
            "completed" => clip(&result_of(state, b).await, budget),
            _ => format!("({}: {})", b.status, b.error.as_deref().unwrap_or("no result")),
        };
        results.push_str(&format!("### {who}: “{}”\n{}\n", b.title, crate::drivers::peer_message(&who, &body)));
        if let Some(from) = b.relayed_from_delegation_id.as_ref().and_then(|id| batch.iter().find(|x| &x.id == id)) {
            results.push_str(&format!(
                "({who} got this task because {}'s result @mentioned it; you didn't send it yourself.)\n",
                name_of(state, &from.specialist_kwant_id).await
            ));
        }
        if let Some(q) = open_question(state, b).await {
            results.push_str(&format!(
                "(Waiting: {who} asked the person “{}” in its thread and has no answer yet. Don't redo this task: tell the \
                person the question is waiting in that thread. When they answer, {who} continues and you get its result.)\n",
                clip(&q.question, 200)
            ));
        }
        if let Some(next) = status_guidance(&b.status) {
            results.push_str(next);
            results.push('\n');
        }
        for parked in state.db.list_dependent_delegations(&b.id).await.unwrap_or_default() {
            results.push_str(&format!(
                "(Not started yet: {} waits for {who}'s result on “{}”. It starts by itself once {who} has the person's answer; \
                don't hand it on again.)\n",
                name_of(state, &parked.specialist_kwant_id).await,
                parked.title
            ));
        }
        results.push('\n');
    }
    let opening = match (opening, batch) {
        (Some(o), _) => o,
        (None, [one]) if one.status == "completed" => {
            format!("{} finished the task you delegated (“{}”).", name_of(state, &one.specialist_kwant_id).await, one.title)
        }
        (None, [one]) => format!(
            "The task you delegated to {} (“{}”) ended without a result: {}.",
            name_of(state, &one.specialist_kwant_id).await,
            one.title,
            one.status
        ),
        (None, _) => format!("All {} tasks you delegated have ended.", batch.len()),
    };
    let prompt = format!(
        "{opening} The results are in the chat above, quoted here:\n\n{results}\
        Check each result against what the user asked. If something is missing or wrong, hand that teammate a concrete \
        correction by @mention (below). Otherwise continue with the user's request and give them one answer \
        that uses the results. If a task failed, was stopped or was cut short, say so plainly. Don't repeat the results in full.\n\n\
        {rules}",
        rules = handoff_rules(),
    );
    // The follow-up may hand on by @mention, like any lead reply: to the next teammate, or back to
    // one that just reported with a correction (not a mention that only credits it), within the
    // usual limits.
    if let Ok(reply) = try_kwant_turn(&scope, &lead, prompt, TurnOpts::default()).await {
        let mut spoken = vec![lead.id.clone()];
        spoken.extend(batch.iter().map(|b| b.specialist_kwant_id.clone()));
        let user_prompt = state
            .db
            .list_messages(d.origin_context().unwrap_or(&lead.id), 100)
            .await
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find(|m| m.sender_role == "user")
            .map(|m| m.text)
            .unwrap_or_else(|| d.task.clone());
        super::routes::run_handoffs_boxed(&scope, &spoken, &[(lead.clone(), reply)], &user_prompt).await;
        // The chain may be over now: several open questions for the person become one list
        super::asks::consolidate(&scope).await;
    }
}

/// How a lead hands work on from its results follow-up, so it never announces a handoff that
/// doesn't happen (Oleks's screenshot, 2026-10-24: "Handing off now: @Social …" and nothing ran).
pub fn handoff_rules() -> String {
    format!(
    "How handing on works: to give a teammate a correction or the next step, @mention them in this reply with the \
    concrete task (\"@Writer, draft Pillar 1: …\"); kwnt sends it to them and resumes you with their result. That works for \
    a teammate who just reported, too. Only say you're handing off, asking or waiting for someone when you @mention them \
    with a task in this reply; to merely refer to a teammate, write the name without @. Limits for this user message: each \
    teammate gets at most {per_bot} tasks by mention and all teammates at most {total} in total. Past that the mention is \
    not sent yet: a notice asks the person to press Continue, so tell them what's waiting instead of claiming a handoff.",
    per_bot = super::routes::MAX_MENTION_DELEGATIONS_PER_BOT,
    total = super::routes::MAX_DELEGATIONS_PER_REQUEST,
    )
}

/// What the lead should do about a task that ended without a result.
fn status_guidance(status: &str) -> Option<&'static str> {
    match status {
        "failed" => Some("(It failed. Decide whether to retry it, give it to someone else or do it yourself, and tell the person which.)"),
        "cancelled" => Some("(It was stopped. Don't restart it unless the person asks.)"),
        "interrupted" => Some("(kwnt restarted twice while it ran, so it may be partly done. Tell the person and offer to run it again.)"),
        _ => None,
    }
}

// Cancelling, follow-ups, restarts

/// Stops a delegation and reports it; `None` if it was already over.
pub async fn cancel(state: &AppState, id: &str) -> Option<Delegation> {
    let d = state.db.get_delegation(id).await.ok().flatten()?;
    state.turns.cancel(Some(&d.id));
    if !state.db.finish_delegation(&d.id, "cancelled", Some("stopped by the user"), None).await.unwrap_or(false) {
        return None;
    }
    let done = state.db.get_delegation(id).await.ok().flatten()?;
    let specialist = name_of(state, &done.specialist_kwant_id).await;
    if let Some(mid) = post_to_origin(
        state,
        &done,
        "system",
        "kwnt",
        format!("Stopped {specialist}'s work on “{}”.", done.title),
        meta("delegation_result", &done, &specialist),
    )
    .await
    {
        let _ = state.db.set_delegation_result_message(&done.id, &mid).await;
    }
    emit_delegation(state, &done);
    release_dependents(state, &done.id).await;
    // A stopped task alone doesn't wake the lead, but other results of its batch must not be lost
    spawn_report_to_lead(state, done.clone());
    Some(done)
}

/// A Stop for a thread also ends the delegation running in it.
pub async fn cancel_thread_delegation(state: &AppState, thread_id: &str) {
    if let Ok(Some(d)) = state.db.delegation_for_thread(thread_id).await {
        if d.is_open() {
            cancel(state, &d.id).await;
        }
    }
}

/// Posts one "still working" note for each delegation past its follow-up time. Called by the
/// routines scheduler loop; no agent turn is started, so it costs nothing.
pub async fn follow_ups(state: &AppState, now: chrono::DateTime<Utc>) -> usize {
    let due = state.db.take_due_follow_ups(now).await.unwrap_or_default();
    for d in &due {
        let specialist = name_of(state, &d.specialist_kwant_id).await;
        let minutes = (now - d.created_at).num_minutes().max(1);
        let queued = d.status == "queued";
        let text = if queued {
            format!("“{}” is still waiting for {specialist}, who is busy with other work ({minutes} min so far).", d.title)
        } else {
            format!("{specialist} is still working on “{}” ({minutes} min so far). Open the thread to follow along or stop it.", d.title)
        };
        post_to_origin(state, d, "system", "kwnt", text, meta("delegation_stalled", d, &specialist)).await;
        super::continuity::notify(
            state,
            super::continuity::Notification {
                kind: "delegation_stalled",
                kwant_id: d.specialist_kwant_id.clone(),
                kwant_name: specialist.clone(),
                channel_id: d.origin_channel_id.clone(),
                title: format!("{specialist} is still on a delegated task"),
                body: format!("“{}”: {minutes} min so far.", clip(&d.title, 80)),
            },
        )
        .await;
    }
    due.len()
}

/// Delegations a restart cut short are restarted once, with a note where they came from; one
/// cut short again becomes `interrupted`.
pub async fn interrupt_open(state: &AppState) {
    let Ok((again, cut)) = state.db.restart_open_delegations().await else {
        return;
    };
    for d in cut {
        let specialist = name_of(state, &d.specialist_kwant_id).await;
        let Ok(Some(done)) = state.db.get_delegation(&d.id).await else {
            continue;
        };
        post_to_origin(
            state,
            &done,
            "system",
            "kwnt",
            format!("kwnt restarted again before {specialist} finished “{}”, after one retry. Ask again to restart it.", done.title),
            meta("delegation_result", &done, &specialist),
        )
        .await;
        release_dependents(state, &done.id).await;
        spawn_report_to_lead(state, done);
    }
    // Tasks parked behind one that ended while kwnt was down go on (or give up) now
    for parked in state.db.list_parked_delegations().await.unwrap_or_default() {
        if let Some(after) = &parked.after_delegation_id {
            if !state.db.get_delegation(after).await.ok().flatten().is_some_and(|p| p.is_open()) {
                release_dependents(state, after).await;
            }
        }
    }
    for d in again {
        let Ok(Some(d)) = state.db.get_delegation(&d.id).await else {
            continue;
        };
        let specialist = name_of(state, &d.specialist_kwant_id).await;
        post_to_origin(
            state,
            &d,
            "system",
            "kwnt",
            format!("kwnt restarted before {specialist} finished “{}”: retrying it once now.", d.title),
            meta("delegation_started", &d, &specialist),
        )
        .await;
        restart(state, d, false, true).await;
    }
}

/// The retry of a delegation a usage limit stopped (see [`super::parked`]).
pub async fn retry_parked(state: &AppState, delegation_id: &str) -> bool {
    let Ok(Some(d)) = state.db.get_delegation(delegation_id).await else {
        return false;
    };
    if !d.is_open() {
        return false;
    }
    let specialist = name_of(state, &d.specialist_kwant_id).await;
    post_to_origin(
        state,
        &d,
        "system",
        "kwnt",
        format!("The usage limit has reset: {specialist} is retrying “{}” once.", d.title),
        meta("delegation_started", &d, &specialist),
    )
    .await;
    restart(state, d, true, false).await
}

/// Runs an open delegation's task again in its thread. `after_restart`: kwnt restarted during
/// the earlier run (see [`RESTARTED_NOTE`]); a usage-limit retry ran nothing before it parked.
async fn restart(state: &AppState, d: Delegation, retried: bool, after_restart: bool) -> bool {
    let (Ok(Some(lead)), Ok(Some(specialist))) =
        (state.db.get_kwant(&d.lead_kwant_id).await, state.db.get_kwant(&d.specialist_kwant_id).await)
    else {
        if state.db.finish_delegation(&d.id, "failed", Some("the bot no longer exists"), None).await.unwrap_or(false) {
            if let Ok(Some(done)) = state.db.get_delegation(&d.id).await {
                let specialist = name_of(state, &done.specialist_kwant_id).await;
                post_to_origin(
                    state,
                    &done,
                    "system",
                    "kwnt",
                    format!("Couldn't restart “{}”: the bot no longer exists.", done.title),
                    meta("delegation_result", &done, &specialist),
                )
                .await;
                emit_delegation(state, &done);
                spawn_report_to_lead(state, done);
            }
        }
        return false;
    };
    let job = state.turns.start_job([d.id.clone(), d.thread_id.clone()]);
    tracing::info!(delegation = %d.id, retried, "delegation restarted");
    tokio::spawn(run(state.clone(), job, d, lead, specialist, RunAs { retried, after_restart, carry_on: None }));
    true
}

/// The person sent an ended hand-off back from its board card (unblocked or retried it): the
/// specialist carries on in the delegation's own thread from where it stopped, and the new
/// result goes to the origin chat and the lead. `false` when it can't: it is still open or
/// completed (a completed one has its result already), a bot is gone, or another call reopened it
/// first.
pub async fn carry_on(state: &AppState, d: &Delegation) -> bool {
    if d.is_open() || d.status == "completed" {
        return false;
    }
    let (Ok(Some(lead)), Ok(Some(specialist))) =
        (state.db.get_kwant(&d.lead_kwant_id).await, state.db.get_kwant(&d.specialist_kwant_id).await)
    else {
        return false;
    };
    let why = d.error.clone().filter(|e| !e.trim().is_empty()).unwrap_or_else(|| format!("it {}", d.status));
    if !state.db.reopen_delegation(&d.id).await.unwrap_or(false) {
        return false;
    }
    let Ok(Some(open)) = state.db.get_delegation(&d.id).await else { return false };
    emit_delegation(state, &open);
    post_to_origin(
        state,
        &open,
        "system",
        "kwnt",
        format!("{} is carrying on with “{}”.", specialist.name, open.title),
        meta("delegation_started", &open, &specialist.name),
    )
    .await;
    let job = state.turns.start_job([open.id.clone(), open.thread_id.clone()]);
    tracing::info!(delegation = %open.id, "delegation carried on");
    tokio::spawn(run(state.clone(), job, open, lead, specialist, RunAs { carry_on: Some(why), ..Default::default() }));
    true
}

// Routes

/// `GET /api/turns?space=`: the turns running for that space's kwants. The limits are app-wide.
pub async fn list_turns(State(state): State<AppState>, super::spaces::Scope(space): super::spaces::Scope) -> Response {
    let (per_agent, global) = state.turns.limiter.limits();
    let mut turns = Vec::new();
    for t in state.turns.active_turns() {
        let of = state.db.space_of_kwant(&t.kwant_id).await.ok().flatten().unwrap_or_else(crate::db::spaces::SpaceId::default_space);
        if of == space {
            turns.push(t);
        }
    }
    Json(json!({ "turns": turns, "max_turns_per_agent": per_agent, "max_turns_global": global })).into_response()
}

#[derive(Deserialize)]
pub struct ThreadsQuery {
    #[serde(default)]
    pub include_archived: Option<String>,
}

async fn thread_view(state: &AppState, t: &Thread) -> Value {
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    let delegation = state.db.delegation_for_thread(&t.id).await.ok().flatten();
    let active: Vec<_> = state.turns.active_turns().into_iter().filter(|a| a.thread_id.as_deref() == Some(&t.id)).collect();
    v["delegation"] = serde_json::to_value(delegation).unwrap_or(Value::Null);
    v["active"] = serde_json::to_value(active).unwrap_or(Value::Null);
    v
}

pub async fn list_threads(State(state): State<AppState>, Path(kwant_id): Path<String>, Query(q): Query<ThreadsQuery>) -> Response {
    match state.db.get_kwant(&kwant_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "bot not found"),
        Err(_) => return internal(),
    }
    let archived = matches!(q.include_archived.as_deref(), Some("1" | "true"));
    let Ok(threads) = state.db.list_threads(&kwant_id, archived, 200).await else {
        return internal();
    };
    let mut out = Vec::with_capacity(threads.len());
    for t in &threads {
        out.push(thread_view(&state, t).await);
    }
    Json(out).into_response()
}

#[derive(Deserialize, Default)]
pub struct NewThreadPayload {
    pub title: Option<String>,
}

pub async fn create_thread(
    State(state): State<AppState>,
    Path(kwant_id): Path<String>,
    body: Option<Json<NewThreadPayload>>,
) -> Response {
    match state.db.get_kwant(&kwant_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "bot not found"),
        Err(_) => return internal(),
    }
    let title = body.and_then(|b| b.0.title).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "New conversation".into());
    match state.db.create_thread(&kwant_id, &title, "chat", None, "user").await {
        Ok(t) => {
            emit_thread(&state, &t);
            (StatusCode::CREATED, Json(thread_view(&state, &t).await)).into_response()
        }
        Err(_) => internal(),
    }
}

pub async fn get_thread(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_thread(&id).await {
        Ok(Some(t)) => Json(thread_view(&state, &t).await).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "thread not found"),
        Err(_) => internal(),
    }
}

#[derive(Deserialize)]
pub struct PatchThreadPayload {
    pub title: Option<String>,
    pub archived: Option<bool>,
}

pub async fn patch_thread(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<PatchThreadPayload>) -> Response {
    if p.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
        return error(StatusCode::BAD_REQUEST, "title can't be empty");
    }
    match state.db.update_thread(&id, p.title.as_deref(), p.archived).await {
        Ok(Some(t)) => {
            emit_thread(&state, &t);
            Json(thread_view(&state, &t).await).into_response()
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "thread not found"),
        Err(_) => internal(),
    }
}

pub async fn thread_messages(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_thread(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "thread not found"),
        Err(_) => return internal(),
    }
    match state.db.list_messages(&id, 100).await {
        Ok(m) => Json(m).into_response(),
        Err(_) => internal(),
    }
}

pub async fn cancel_thread(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_thread(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "thread not found"),
        Err(_) => return internal(),
    }
    let cancelled = state.turns.cancel(Some(&id));
    cancel_thread_delegation(&state, &id).await;
    Json(json!({ "status": "cancelled", "cancelled": cancelled })).into_response()
}

#[derive(Deserialize)]
pub struct DelegatePayload {
    pub lead_kwant_id: String,
    pub specialist_kwant_id: String,
    pub task: String,
    pub title: Option<String>,
    pub channel_id: Option<String>,
    pub kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub continue_lead: Option<bool>,
}

pub async fn create_delegation(State(state): State<AppState>, Json(p): Json<DelegatePayload>) -> Response {
    let (lead, specialist) = match (state.db.get_kwant(&p.lead_kwant_id).await, state.db.get_kwant(&p.specialist_kwant_id).await) {
        (Ok(Some(l)), Ok(Some(s))) => (l, s),
        (Ok(_), Ok(_)) => return error(StatusCode::NOT_FOUND, "bot not found"),
        _ => return internal(),
    };
    let origin_depth = match &p.thread_id {
        Some(t) => state.db.delegation_for_thread(t).await.ok().flatten().map(|d| d.depth).unwrap_or(0),
        None => 0,
    };
    let req = StartRequest {
        lead,
        specialist,
        task: p.task,
        title: p.title,
        origin: Origin { channel_id: p.channel_id, kwant_id: p.kwant_id, thread_id: p.thread_id },
        continue_lead: p.continue_lead.unwrap_or(true),
        origin_depth,
        allowed: None,
        after: None,
        relayed_from: None,
    };
    match start(&state, req).await {
        Ok(d) => (StatusCode::ACCEPTED, Json(d)).into_response(),
        Err((status, m)) => error(status, m),
    }
}

#[derive(Deserialize)]
pub struct DelegationsQuery {
    pub lead_kwant_id: Option<String>,
    pub status: Option<String>,
}

pub async fn list_delegations(State(state): State<AppState>, Query(q): Query<DelegationsQuery>) -> Response {
    match state.db.list_delegations(q.lead_kwant_id.as_deref(), q.status.as_deref(), 50).await {
        Ok(d) => Json(d).into_response(),
        Err(_) => internal(),
    }
}

pub async fn get_delegation(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_delegation(&id).await {
        Ok(Some(d)) => Json(d).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "delegation not found"),
        Err(_) => internal(),
    }
}

pub async fn cancel_delegation(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_delegation(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "delegation not found"),
        Err(_) => return internal(),
    }
    cancel(&state, &id).await;
    match state.db.get_delegation(&id).await {
        Ok(Some(d)) => Json(d).into_response(),
        _ => internal(),
    }
}

// Agent tools

/// What a lead's `delegate` tool call becomes; the scope says who is calling and from where.
pub async fn tool_delegate(state: &AppState, scope: &super::continuity::SelfScope, args: &Value) -> (String, bool) {
    let Some(origin) = &scope.origin else {
        return ("Delegation isn't available in this turn.".into(), true);
    };
    let to = args["to"].as_str().map(str::trim).unwrap_or("").trim_start_matches('@');
    let task = args["task"].as_str().unwrap_or("").to_string();
    let Some(lead) = state.db.get_kwant(&scope.kwant_id).await.ok().flatten() else {
        return ("Unknown bot.".into(), true);
    };
    // Only the lead's own space, even when the chat has no member list
    let kwants = state.db.list_kwants(&SpaceId::new(&lead.space_id)).await.unwrap_or_default();
    let pool: Vec<&Kwant> = if origin.member_ids.is_empty() {
        kwants.iter().filter(|k| k.id != lead.id).collect()
    } else {
        kwants.iter().filter(|k| k.id != lead.id && origin.member_ids.contains(&k.id)).collect()
    };
    let Some(specialist) = kwants.iter().find(|k| k.id == to || k.name.eq_ignore_ascii_case(to)).cloned() else {
        let names: Vec<&str> = pool.iter().map(|k| k.name.as_str()).collect();
        return (format!("No bot named '{to}'. You can delegate to: {}.", names.join(", ")), true);
    };
    let req = StartRequest {
        lead,
        specialist,
        task,
        title: args["title"].as_str().map(str::to_string),
        origin: Origin {
            channel_id: origin.channel_id.clone(),
            kwant_id: origin.direct_kwant_id.clone(),
            thread_id: origin.thread_id.clone(),
        },
        continue_lead: true,
        origin_depth: origin.depth,
        allowed: (!origin.member_ids.is_empty()).then(|| origin.member_ids.clone()),
        after: None,
        relayed_from: None,
    };
    match start(state, req).await {
        Ok(d) => (
            format!(
                "Delegated as [{}]. It runs in the background; its result will be posted to this chat and you'll get a turn to continue. \
                Tell the user who is working on it, then carry on.",
                d.id
            ),
            false,
        ),
        Err((_, m)) => (m, true),
    }
}

pub async fn tool_status(state: &AppState, scope: &super::continuity::SelfScope, args: &Value) -> (String, bool) {
    let list = match args["id"].as_str().filter(|s| !s.is_empty()) {
        Some(id) => match state.db.get_delegation(id).await.ok().flatten() {
            Some(d) if d.lead_kwant_id == scope.kwant_id => vec![d],
            _ => return ("No delegation of yours has that id.".into(), true),
        },
        None => state.db.list_delegations(Some(&scope.kwant_id), None, 10).await.unwrap_or_default(),
    };
    if list.is_empty() {
        return ("You have no delegations.".into(), false);
    }
    let mut out = String::new();
    for d in list {
        let who = name_of(state, &d.specialist_kwant_id).await;
        out.push_str(&format!("- [{}] {who}: “{}” — {}", d.id, clip(&d.title, 80), d.status));
        if let Some(e) = &d.error {
            out.push_str(&format!(" ({})", clip(e, 120)));
        }
        out.push('\n');
    }
    (out, false)
}

pub async fn tool_cancel(state: &AppState, scope: &super::continuity::SelfScope, args: &Value) -> (String, bool) {
    let Some(id) = args["id"].as_str().filter(|s| !s.is_empty()) else {
        return ("id is required".into(), true);
    };
    match state.db.get_delegation(id).await.ok().flatten() {
        Some(d) if d.lead_kwant_id == scope.kwant_id => match cancel(state, id).await {
            Some(_) => (format!("Stopped [{id}]."), false),
            None => (format!("[{id}] had already finished ({}).", d.status), false),
        },
        _ => ("No delegation of yours has that id.".into(), true),
    }
}
