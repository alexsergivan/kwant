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
    post_to_origin(
        state,
        &d,
        "system",
        "kwnt",
        format!("{} asked {} to work on “{}”. It runs in its own thread; the result will be posted here.", req.lead.name, req.specialist.name, d.title),
        meta("delegation_started", &d, &req.specialist.name),
    )
    .await;
    emit_thread(state, &thread);
    emit_delegation(state, &d);
    tracing::info!(delegation = %d.id, lead = %req.lead.id, specialist = %req.specialist.id, depth = d.depth, "delegation started");

    let job = state.turns.start_job([d.id.clone(), thread.id.clone()]);
    let runner_state = state.clone();
    let (lead, specialist, delegation) = (req.lead, req.specialist, d.clone());
    tokio::spawn(async move { run(runner_state, job, delegation, lead, specialist, false).await });
    Ok(d)
}

/// The background part: the specialist's turn in the thread, then the report in the origin chat.
/// `retried`: the run after a usage limit reset, which never waits for one again.
async fn run(state: AppState, job: super::turns::JobGuard, d: Delegation, lead: Kwant, specialist: Kwant, retried: bool) {
    let scope = match scope_for(&state, &specialist, None, Some(d.thread_id.clone()), &job, d.depth).await {
        Ok(s) => s,
        Err(why) => return finish(&state, &d, &specialist, Err(why)).await,
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
    let prompt = format!(
        "{lead} delegated this task to you. Work on it here; when you finish, your reply is posted back to {lead} \
        and the chat it came from, so make it a complete, self-contained answer.\n\nTask: {title}\n\n{task}",
        lead = lead.name,
        title = d.title,
        task = d.task,
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
    finish(&state, &d, &specialist, outcome).await;
}

async fn finish(state: &AppState, d: &Delegation, specialist: &Kwant, outcome: Result<String, String>) {
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
        Ok(reply) => {
            let full = reply.trim();
            let mut text = clip(full, RESULT_CHARS);
            if text.len() < full.len() {
                text.push_str("\n\n(The full answer is in the delegation thread.)");
            }
            text
        }
        Err(_) if status == "cancelled" => format!("Stopped work on “{}”.", done.title),
        Err(e) => format!("Couldn't finish “{}”: {}", done.title, clip(e, 500)),
    };
    let result_id = post_to_origin(state, &done, "kwant", &specialist.name, text, meta("delegation_result", &done, &specialist.name)).await;
    if let Some(id) = &result_id {
        let _ = state.db.set_delegation_result_message(&done.id, id).await;
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
    if done.continue_lead {
        if let Some(batch) = finished_batch(state, &done).await {
            continue_lead(state, &done, &batch).await;
        }
    }
}

/// The delegations a lead sent from one chat that ran at the same time as `d`, once all of them
/// have ended. `None` while one is still open, or when another one of them ended later: the last
/// to end resumes the lead, once, with every result.
async fn finished_batch(state: &AppState, d: &Delegation) -> Option<Vec<Delegation>> {
    let siblings: Vec<Delegation> = state
        .db
        .list_delegations(Some(&d.lead_kwant_id), None, 50)
        .await
        .ok()?
        .into_iter()
        .filter(|s| s.continue_lead && s.origin_context() == d.origin_context())
        .collect();
    if siblings.iter().any(Delegation::is_open) {
        return None;
    }
    let end = |s: &Delegation| s.ended_at.unwrap_or(s.created_at);
    // Grow from `d` to every delegation whose run overlaps the batch's span
    let mut batch = vec![siblings.iter().find(|s| s.id == d.id)?.clone()];
    let (mut lo, mut hi) = (d.created_at, end(d));
    loop {
        let more: Vec<Delegation> = siblings
            .iter()
            .filter(|s| !batch.iter().any(|b| b.id == s.id) && s.created_at <= hi && end(s) >= lo)
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
    let last = batch.iter().max_by(|a, b| end(a).cmp(&end(b)).then_with(|| a.id.cmp(&b.id)))?;
    if last.id != d.id || !batch.iter().any(|b| b.status == "completed") {
        return None;
    }
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
/// and gives the user one answer.
async fn continue_lead(state: &AppState, d: &Delegation, batch: &[Delegation]) {
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
        results.push_str(&format!("### {who}: “{}”\n{body}\n\n", b.title));
    }
    let opening = match batch {
        [one] => format!("{} finished the task you delegated (“{}”).", name_of(state, &one.specialist_kwant_id).await, one.title),
        _ => format!("All {} tasks you delegated have ended.", batch.len()),
    };
    let prompt = format!(
        "{opening} The results are in the chat above, quoted here:\n\n{results}\
        Check each result against what the user asked. If something is missing or wrong, delegate a concrete \
        correction. Otherwise continue with the user's request and give them one answer that uses the results. \
        Don't repeat the results in full."
    );
    // The follow-up may hand on to the next teammate by @mention, like any lead reply. Not back to
    // the ones that just reported (the result is what it's about), and within the usual limits.
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
        restart(state, d, false).await;
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
    restart(state, d, true).await
}

/// Runs an open delegation's task again in its thread.
async fn restart(state: &AppState, d: Delegation, retried: bool) -> bool {
    let (Ok(Some(lead)), Ok(Some(specialist))) =
        (state.db.get_kwant(&d.lead_kwant_id).await, state.db.get_kwant(&d.specialist_kwant_id).await)
    else {
        let _ = state.db.finish_delegation(&d.id, "failed", Some("the bot no longer exists"), None).await;
        return false;
    };
    let job = state.turns.start_job([d.id.clone(), d.thread_id.clone()]);
    tracing::info!(delegation = %d.id, retried, "delegation restarted");
    tokio::spawn(run(state.clone(), job, d, lead, specialist, retried));
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
