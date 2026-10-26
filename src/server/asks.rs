//! One "what I need from you" list: when the bots of one chain (started by one person message)
//! asked the person 2+ things, the channel's lead (or the bot that started the chain) gets one
//! short extra turn to gather them into a single numbered list, and the person's answer to that
//! list goes to it, which passes each part on.

use super::routes::{try_kwant_turn, TurnOpts, TurnScope};
use super::AppState;
use crate::db::models::{Kwant, Message};
use crate::drivers::RuntimeEvent;
use serde_json::{json, Value};

/// The key of a consolidated list's `meta`: `{kwant_id, askers, question_ids, message_ids}`.
pub const META_KEY: &str = "consolidated_asks";
/// The key an asker's message (a reply or a question card) gets: the list's message id.
pub const INTO_KEY: &str = "consolidated_into";

/// Something a bot asked the person in this chain.
#[derive(Debug, Clone)]
struct Ask {
    kwant: Kwant,
    question: String,
    /// An open question row (a card), if it was one.
    question_id: Option<String>,
    /// The message that shows it (a reply, or the card).
    message_id: Option<String>,
}

fn meta_of(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// The consolidated list among `tail` (messages since the person's last one), with its meta.
pub fn consolidated_in(tail: &[Message]) -> Option<(&Message, Value)> {
    tail.iter().rev().find_map(|m| {
        let meta = meta_of(m);
        (m.sender_role == "kwant" && meta.get(META_KEY).is_some()).then(|| (m, meta[META_KEY].clone()))
    })
}

/// The messages after the person's last one, and when that was. `None` when the person never wrote.
fn since_last_user(recent: &[Message]) -> Option<(&[Message], chrono::DateTime<chrono::Utc>)> {
    let i = recent.iter().rposition(|m| m.sender_role == "user")?;
    Some((&recent[i + 1..], recent[i].created_at?))
}

/// Whether a reply's last line hands on to a teammate ("@Writer, can you…?"): that question is
/// for the teammate, not the person.
pub(super) fn hands_on(text: &str, names: &[&str]) -> bool {
    let last = text.trim_end().lines().last().unwrap_or("").to_lowercase();
    names.iter().any(|n| last.contains(&format!("@{}", n.to_lowercase())))
}

/// At a chain's end in a channel: if 2+ bots asked the person something since their last
/// message, the lead (or the bot that started the chain) posts one list of everything needed.
/// Nothing happens while a delegation from this channel still runs, when the list was already
/// posted, or when fewer than two bots asked.
pub(super) async fn consolidate(scope: &TurnScope) {
    let (Some(cid), None) = (&scope.channel_id, &scope.thread_id) else { return };
    if scope.cancel_token.is_cancelled() {
        return;
    }
    let state = &scope.state;
    let Ok(Some(channel)) = state.db.get_channel(cid).await else { return };
    let recent = state.db.list_messages(cid, 100).await.unwrap_or_default();
    let Some((tail, since)) = since_last_user(&recent) else { return };
    if consolidated_in(tail).is_some() {
        return;
    }
    let delegations: Vec<_> = state
        .db
        .list_delegations(None, None, 100)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.origin_channel_id.as_deref() == Some(cid.as_str()) && d.created_at >= since)
        .collect();
    // The chain is still going: the last one to end consolidates
    if delegations.iter().any(|d| d.is_open() && !super::delegation::is_parked(d)) {
        return;
    }
    let members = &scope.teammates;
    let by_ref = |r: &str| members.iter().find(|k| k.id == r || k.name == r).cloned();
    let names: Vec<&str> = members.iter().map(|k| k.name.as_str()).collect();

    let mut asks: Vec<Ask> = Vec::new();
    // Question cards in the channel
    for q in state.db.list_questions(Some("open"), Some(cid), None).await.unwrap_or_default() {
        if q.channel_id.as_deref() != Some(cid.as_str()) || q.created_at < since {
            continue;
        }
        if let Some(kwant) = by_ref(&q.kwant_id) {
            asks.push(Ask { kwant, question: q.question.clone(), question_id: Some(q.id.clone()), message_id: q.message_id.clone() });
        }
    }
    // Questions specialists asked in the threads of this chain's delegations
    for d in &delegations {
        for q in state.db.list_questions(Some("open"), Some(&d.thread_id), None).await.unwrap_or_default() {
            if q.kwant_id != d.specialist_kwant_id || q.created_at < since {
                continue;
            }
            if let Some(kwant) = by_ref(&q.kwant_id) {
                asks.push(Ask { kwant, question: q.question.clone(), question_id: Some(q.id.clone()), message_id: q.message_id.clone() });
            }
        }
    }
    // Plain replies whose last line asks the person
    for m in tail.iter().filter(|m| m.sender_role == "kwant") {
        let meta = meta_of(m);
        if meta["kind"] == "ask_user" || !crate::router::sticky::asks_the_person(&m.text) || hands_on(&m.text, &names) {
            continue;
        }
        let Some(kwant) = by_ref(&m.sender_id) else { continue };
        if asks.iter().any(|a| a.kwant.id == kwant.id) {
            continue;
        }
        let question = m.text.trim_end().lines().last().unwrap_or("").trim().to_string();
        asks.push(Ask { kwant, question, question_id: None, message_id: Some(m.id.clone()) });
    }
    let mut askers: Vec<&Kwant> = Vec::new();
    for a in &asks {
        if !askers.iter().any(|k| k.id == a.kwant.id) {
            askers.push(&a.kwant);
        }
    }
    if askers.len() < 2 {
        return;
    }
    // The lead, else whoever answered the person first
    let coordinator = members
        .iter()
        .find(|k| k.id == channel.lead_kwant_id)
        .cloned()
        .or_else(|| tail.iter().filter(|m| m.sender_role == "kwant").find_map(|m| by_ref(&m.sender_id)))
        .unwrap_or_else(|| askers[0].clone());

    let lines: Vec<String> = asks.iter().map(|a| format!("- {}: “{}”", a.kwant.name, clip(&a.question, 300))).collect();
    let prompt = format!(
        "[kwnt: since the person's last message, {n} teammates asked them something:\n{list}\n\n\
        Post ONE short message with everything the person needs to answer, as one numbered list (one line each; say \
        whose question it is and merge duplicates), so they can answer it all in one reply. Don't @mention anyone, \
        don't answer the questions yourself and do no other work. When the person answers, you get the answer and pass \
        each part on.]",
        n = askers.len(),
        list = lines.join("\n"),
    );
    tracing::info!(channel = %cid, coordinator = %coordinator.id, askers = askers.len(), "consolidating the person's open questions");
    // Saved times have whole seconds: the list is the coordinator's newest message not seen before
    let seen: std::collections::HashSet<&str> = recent.iter().map(|m| m.id.as_str()).collect();
    let mut opts = TurnOpts::default();
    opts.label = Some("One list of what the team needs from you".into());
    if try_kwant_turn(scope, &coordinator, prompt, opts).await.is_err() {
        return;
    }
    let after = state.db.list_messages(cid, 20).await.unwrap_or_default();
    let Some(list) = after
        .iter()
        .rev()
        .find(|m| m.sender_role == "kwant" && m.sender_id == coordinator.name && !seen.contains(m.id.as_str()))
    else {
        return;
    };
    let link = json!({
        "kwant_id": coordinator.id,
        "askers": askers.iter().map(|k| k.id.clone()).collect::<Vec<_>>(),
        "question_ids": asks.iter().filter_map(|a| a.question_id.clone()).collect::<Vec<_>>(),
        "message_ids": asks.iter().filter_map(|a| a.message_id.clone()).collect::<Vec<_>>(),
    });
    set_meta_key(state, list, META_KEY, link).await;
    for a in &asks {
        let Some(mid) = &a.message_id else { continue };
        if let Some(m) = after.iter().chain(recent.iter()).find(|m| &m.id == mid) {
            set_meta_key(state, m, INTO_KEY, json!(list.id)).await;
        } else if let Ok(Some(m)) = state.db.get_message(mid).await {
            set_meta_key(state, &m, INTO_KEY, json!(list.id)).await;
        }
    }
}

/// Adds `key` to a message's meta, keeping the rest, and tells the clients.
async fn set_meta_key(state: &AppState, m: &Message, key: &str, value: Value) {
    let mut meta = match meta_of(m) {
        Value::Object(o) => Value::Object(o),
        _ => json!({}),
    };
    meta[key] = value;
    if state.db.set_message_meta(&m.id, &meta).await.is_ok() {
        let mut updated = m.clone();
        updated.meta = Some(meta.to_string());
        let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&updated).unwrap_or(Value::Null) });
    }
}

/// The person answered a consolidated list (their message goes to its lead): the questions on
/// cards close with that answer, and a specialist that asked in a delegation's thread continues
/// there with it. Returns the note for the lead's turn: who already has the answer and whom it
/// should pass it on to.
pub(super) async fn deliver_answer(state: &AppState, link: &Value, answer: &str, members: &[Kwant]) -> String {
    let mut continued: Vec<String> = Vec::new();
    let mut answered_cards: Vec<String> = Vec::new();
    for qid in link["question_ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let Ok(Some(q)) = state.db.get_question(qid).await else { continue };
        if q.status != "open" {
            continue;
        }
        let name = members.iter().find(|k| k.id == q.kwant_id).map(|k| k.name.clone()).unwrap_or_default();
        if q.thread_id.is_some() {
            // Its specialist continues in the thread, and its new result comes back here
            if super::questions::answer_with(state, qid, answer).await.is_some() && !continued.contains(&name) {
                continued.push(name);
            }
        } else if super::questions::answer_by_message(state, qid, answer).await {
            // A turn still waiting on its card took the answer
            if !continued.contains(&name) {
                continued.push(name);
            }
        } else if !answered_cards.contains(&name) {
            answered_cards.push(name);
        }
    }
    let others: Vec<String> = link["askers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|&id| Some(id) != link["kwant_id"].as_str())
        .filter_map(|id| members.iter().find(|k| k.id == id).map(|k| k.name.clone()))
        .filter(|n| !continued.contains(n))
        .collect();
    let mut note = String::from("\n\n[kwnt: this message answers the list of questions you posted for the team.");
    if !continued.is_empty() {
        note.push_str(&format!(" {} already got the answer directly and continue on their own; don't hand it to them again.", continued.join(", ")));
    }
    if !others.is_empty() {
        note.push_str(&format!(
            " Pass the part that answers each of these teammates on to them by @mention, with what they asked for: {}.",
            others.join(", ")
        ));
    }
    note.push(']');
    note
}

fn clip(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_to_a_teammate_is_not_for_the_person() {
        let names = ["Writer", "Analyst"];
        assert!(hands_on("Here's the draft.\n@Analyst, can you check the numbers?", &names));
        assert!(!hands_on("Here's the draft.\nWhich tone do you want?", &names));
    }
}
