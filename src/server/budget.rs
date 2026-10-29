//! The chain token budget: one person message can start a chain of hand-offs, and every turn in it
//! adds its fresh tokens (see [`crate::drivers::TurnUsage::fresh`]) to the dispatch's job (see
//! [`super::turns`]). Once a chain is over the budget, no more teammates are handed the turn: the
//! person gets one notice in the chat, and the chain carries on only when they press Continue on it
//! or type "continue".

use super::routes::TurnScope;
use super::AppState;
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Setting key: fresh tokens one chain may use before hand-offs pause; 0 turns the guard off.
pub const SETTING: &str = "chain_token_budget";
/// Off unless the person sets a budget, like task and mission limits (docs/mission-control.md §15).
/// The hand-off count caps still stop a chain that loops.
pub const DEFAULT: u64 = 0;
/// The budget every install had before it became opt-in; a stored value equal to it is cleared
/// once ([`crate::db::Database::migrate_chain_budget_default`]).
pub const OLD_DEFAULT: u64 = 200_000;
pub const MAX: u64 = 100_000_000;
/// Message meta kind of the notice.
pub const KIND: &str = "budget_paused";
/// Message meta kind of the notice a lead's @mentions leave past the hand-off caps
/// ([`super::routes::MAX_MENTION_DELEGATIONS_PER_BOT`], [`super::routes::MAX_DELEGATIONS_PER_REQUEST`]).
/// Carried on like a paused chain: Continue on it, or "continue" typed after it.
pub const CAPPED_KIND: &str = "handoff_capped";

/// The budget in fresh tokens; 0 when the guard is off.
pub async fn limit(state: &AppState) -> u64 {
    state.db.get_setting(SETTING).await.ok().flatten().and_then(|v| v.parse::<u64>().ok()).unwrap_or(DEFAULT).min(MAX)
}

/// Whether the person's message asks a paused chain to carry on.
pub fn is_continue(text: &str) -> bool {
    let t = text.trim().trim_end_matches(['.', '!']).trim().to_lowercase();
    matches!(t.as_str(), "continue" | "continue please" | "please continue" | "go on" | "keep going" | "carry on" | "go ahead")
}

/// Short token counts for the notice: 950, 12k, 1.2M.
fn short(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{}k", n / 1_000),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Before handing the turn on: when this chain is over the budget and `replies` would hand it to
/// someone who hasn't spoken, saves one notice instead and returns true, so the caller stops.
/// The notice keeps what is needed to carry on: who spoke, and the replies whose @mentions
/// were waiting.
pub(super) async fn pause_if_over(scope: &TurnScope, spoken: &[String], replies: &[(String, String)], user_prompt: &str) -> bool {
    let budget = limit(&scope.state).await;
    let used = scope.state.turns.job_tokens(&scope.job_id);
    if budget == 0 || used <= budget || scope.teammates.is_empty() {
        return false;
    }
    let names: Vec<&str> = scope.teammates.iter().map(|k| k.name.as_str()).collect();
    let mut waiting: Vec<String> = Vec::new();
    for (from, text) in replies {
        for i in crate::router::handoff_mentions(text, &names).targets {
            let k = &scope.teammates[i];
            if &k.name != from && !spoken.contains(&k.id) && !waiting.contains(&k.name) {
                waiting.push(k.name.clone());
            }
        }
    }
    if waiting.is_empty() {
        return false;
    }
    tracing::info!(job = %scope.job_id, used, budget, waiting = ?waiting, "chain over its token budget: hand-offs paused");
    let text = format!(
        "Paused before handing on to {}: this request has used {} fresh tokens, over the {} budget. \
         Say \"continue\" to let them take their turn, or change the budget in App settings → Usage.",
        waiting.join(", "),
        short(used),
        short(budget),
    );
    let meta = json!({
        "kind": KIND,
        "used": used,
        "budget": budget,
        "waiting": waiting,
        "spoken": spoken,
        "replies": replies.iter().map(|(from, text)| json!({"from": from, "text": text})).collect::<Vec<_>>(),
        "user_prompt": user_prompt,
        "resumed": false,
    });
    let msg = scope.message("system", "kwant", text);
    scope.save(&msg, Some(meta)).await;
    true
}

/// A lead's @mentions went past a hand-off cap: saves one notice naming the teammates `capped`
/// (`sent` hand-offs so far for this request), which the person carries on with one press.
pub(super) async fn pause_capped(scope: &TurnScope, lead: &Kwant, capped: &[&Kwant], sent: usize, text: &str, user_prompt: &str) {
    let names: Vec<String> = capped.iter().map(|k| k.name.clone()).collect();
    tracing::info!(lead = %lead.id, sent, waiting = ?names, "hand-off caps reached: waiting for Continue");
    let line = format!(
        "Paused after {sent} hand-offs so it doesn't loop: {} not sent yet. Press Continue or say \"continue\" to hand it on.",
        names.join(", "),
    );
    let meta = json!({
        "kind": CAPPED_KIND,
        "lead": lead.id,
        "waiting": names,
        "waiting_ids": capped.iter().map(|k| k.id.clone()).collect::<Vec<_>>(),
        "sent": sent,
        "text": text,
        "user_prompt": user_prompt,
        "resumed": false,
    });
    if let Some(saved) = scope.save(&scope.message("system", "kwnt", line), Some(meta)).await {
        let _ = scope.state.event_sender.send(crate::drivers::events::RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or_default() });
    }
}

/// The newest notice in a channel that can be carried on (a paused chain or capped hand-offs) and
/// hasn't been, if it's the newest message from a bot or kwant (a later reply means the chat
/// moved on).
async fn open_pause(state: &AppState, channel_id: &str) -> Option<crate::db::models::Message> {
    let recent = state.db.list_messages(channel_id, 10).await.ok()?;
    let last = recent.into_iter().rev().find(|m| m.sender_role != "user")?;
    let meta: serde_json::Value = serde_json::from_str(last.meta.as_deref()?).ok()?;
    ((meta["kind"] == KIND || meta["kind"] == CAPPED_KIND) && meta["resumed"] != true).then_some(last)
}

/// Carries a paused chain on from its notice: marks it resumed and hands the turn to the bots
/// that were waiting, as a new dispatch with its own budget. `Err` says why it can't.
async fn resume(state: &AppState, notice_id: &str) -> Result<(), (StatusCode, &'static str)> {
    let notice = state.db.get_message(notice_id).await.ok().flatten().ok_or((StatusCode::NOT_FOUND, "no such message"))?;
    let mut meta: serde_json::Value =
        notice.meta.as_deref().and_then(|m| serde_json::from_str(m).ok()).ok_or((StatusCode::BAD_REQUEST, "not a paused chain"))?;
    if meta["kind"] != KIND && meta["kind"] != CAPPED_KIND {
        return Err((StatusCode::BAD_REQUEST, "not a paused chain"));
    }
    if meta["resumed"] == true {
        return Err((StatusCode::CONFLICT, "this chain was already continued"));
    }
    let channel_id = notice.channel_id.clone().ok_or((StatusCode::BAD_REQUEST, "not a channel message"))?;
    if meta["kind"] == CAPPED_KIND {
        return resume_capped(state, notice_id, meta, channel_id).await;
    }
    let channel = state.db.get_channel(&channel_id).await.ok().flatten().ok_or((StatusCode::NOT_FOUND, "the channel is gone"))?;
    let space = crate::db::spaces::SpaceId::new(&channel.space_id);
    let all: Vec<Kwant> = state.db.list_kwants(&space).await.map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not load the bots"))?;
    let replies: Vec<(Kwant, String)> = meta["replies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let from = all.iter().find(|k| r["from"].as_str().is_some_and(|f| k.name == f))?;
            Some((from.clone(), r["text"].as_str()?.to_string()))
        })
        .collect();
    let Some((first, _)) = replies.first().cloned() else {
        return Err((StatusCode::GONE, "the bots of this chain are gone"));
    };
    let spoken: Vec<String> = meta["spoken"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
    let user_prompt = meta["user_prompt"].as_str().unwrap_or_default().to_string();
    meta["resumed"] = json!(true);
    let _ = state.db.set_message_meta(notice_id, &meta).await;

    let job = state.turns.start_job([channel_id.clone(), notice_id.to_string()]);
    let scope = super::routes::scope_for(state, &first, Some(channel_id), None, &job, 0)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not set up the chat"))?;
    tokio::spawn(async move {
        let _job = job;
        super::routes::run_handoffs_boxed(&scope, &spoken, &replies, &user_prompt).await;
    });
    Ok(())
}

/// Carries capped hand-offs on: the lead hands the waiting teammates their work, with a fresh
/// allowance of hand-offs.
async fn resume_capped(state: &AppState, notice_id: &str, mut meta: serde_json::Value, channel_id: String) -> Result<(), (StatusCode, &'static str)> {
    let lead_id = meta["lead"].as_str().unwrap_or_default().to_string();
    let lead = state.db.get_kwant(&lead_id).await.ok().flatten().ok_or((StatusCode::GONE, "the lead of this chat is gone"))?;
    let waiting: Vec<String> = meta["waiting_ids"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
    let text = meta["text"].as_str().unwrap_or_default().to_string();
    let user_prompt = meta["user_prompt"].as_str().unwrap_or_default().to_string();
    meta["resumed"] = json!(true);
    let _ = state.db.set_message_meta(notice_id, &meta).await;

    let job = state.turns.start_job([channel_id.clone(), notice_id.to_string()]);
    let scope = super::routes::scope_for(state, &lead, Some(channel_id), None, &job, 0)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not set up the chat"))?;
    tokio::spawn(async move {
        let _job = job;
        super::routes::delegate_mentions(&scope, &lead, &text, &user_prompt, &[], Some(&waiting), None).await;
    });
    Ok(())
}

/// For `send_chat`: the person's "continue" right after a paused chain carries it on. Returns the
/// notice's id when it did. Bots' tokens never get here (`from_person` false).
pub async fn resume_if_continue(state: &AppState, channel_id: &str, prompt: &str, from_person: bool) -> Option<String> {
    if !from_person || !is_continue(prompt) {
        return None;
    }
    let notice = open_pause(state, channel_id).await?;
    resume(state, &notice.id).await.ok()?;
    Some(notice.id)
}

/// `POST /api/chains/:message_id/continue` (person only): the Continue button on the notice.
pub async fn continue_chain(State(state): State<AppState>, Path(message_id): Path<String>) -> Response {
    match resume(&state, &message_id).await {
        Ok(()) => {
            let msg = NewMessage {
                channel_id: state.db.get_message(&message_id).await.ok().flatten().and_then(|m| m.channel_id),
                direct_kwant_id: None,
                sender_role: "user".into(),
                sender_id: "user".into(),
                text: "continue".into(),
                reasoning_content: None,
                tool_calls: None,
                tokens_used: 0,
            };
            let _ = state.db.insert_message_with(&msg, &MessageExtra::default()).await;
            Json(json!({"status": "continued"})).into_response()
        }
        Err((status, why)) => (status, Json(json!({"error": why}))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_words() {
        for t in ["continue", "Continue.", " go on ", "Keep going!", "please continue", "go ahead"] {
            assert!(is_continue(t), "{t}");
        }
        for t in ["continue with the plan but shorter", "don't continue", "stop", ""] {
            assert!(!is_continue(t), "{t}");
        }
    }

    #[test]
    fn short_counts() {
        assert_eq!((short(950), short(12_400), short(1_250_000)), ("950".into(), "12k".into(), "1.2M".into()));
    }
}
