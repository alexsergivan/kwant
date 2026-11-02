//! A channel message that doesn't go straight to one bot: a close call the person settles on a
//! card ("ask"), and a bot handing a message it got back as not its area ("handback").
//!
//! - Ask: a fit from [`ASK_FROM`] up to [`ROUTE_AT`] with two candidates posts a card offering
//!   those two bots and a general answer (the Concierge in #general, else the channel lead). The
//!   message waits; `POST /api/route-choices/:id/resolve` (person only) runs the chosen bot's turn.
//! - Hand-back: the `hand_back` tool, in a routed channel turn, sends the message that turn answers
//!   on to the next best bot here, else the Concierge (or the lead). One hop per message, ever.
//!
//! Contract: /home/maus/shared/ship-software-safely/router-concierge/contract.md

use super::continuity::SelfScope;
use super::AppState;
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use crate::db::routing::RouteChoice;
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use crate::router::team::{Hints, Learned, ROUTE_AT};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub const HAND_BACK: &str = "hand_back";

/// The space's covers/avoids lists for routing among `members`, and what the person taught it.
pub(super) async fn hints_for(state: &AppState, space: &SpaceId, members: &[Kwant]) -> Hints {
    let ids: Vec<String> = members.iter().map(|k| k.id.clone()).collect();
    let learned = state
        .db
        .learned_examples(space.as_str())
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|e| ids.contains(&e.kwant_id))
        .map(|e| Learned::new(&e.kwant_id, &e.text))
        .collect();
    Hints { scopes: state.db.scopes_for(&ids).await.unwrap_or_default(), learned }
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"error": msg}))).into_response()
}

/// Who answers "Answer generally": the Concierge in #general (made now if needed, unless the
/// person removed it), else the channel's lead. `None` only when neither exists.
async fn general_answerer(state: &AppState, space: &SpaceId, channel: &crate::db::models::Channel, all: &[Kwant]) -> Option<Kwant> {
    if channel.name == "general" {
        if let Some(c) = super::concierge::for_no_match(state, space, &channel.id, all).await {
            return Some(c);
        }
    }
    all.iter().find(|k| k.id == channel.lead_kwant_id).cloned()
}

/// The words on the general option, before anyone is made: "Concierge" when #general has one or
/// would make one, else the lead's name.
async fn general_label(state: &AppState, space: &SpaceId, channel: &crate::db::models::Channel, all: &[Kwant]) -> (String, Option<String>) {
    if channel.name == "general" {
        match super::concierge::state(state, space, all).await {
            super::concierge::State::Present(k) => return (k.name.clone(), Some(k.id.clone())),
            super::concierge::State::None => return (super::concierge::NAME.to_string(), None),
            super::concierge::State::Removed => {}
        }
    }
    match all.iter().find(|k| k.id == channel.lead_kwant_id) {
        Some(k) => (k.name.clone(), Some(k.id.clone())),
        None => (String::new(), None),
    }
}

fn card_meta(c: &RouteChoice, offered: &[Kwant], general: &(String, Option<String>), reason: &str) -> Value {
    json!({
        "kind": "route_choice",
        "route_choice_id": c.id,
        "space_id": c.space_id,
        "channel_id": c.channel_id,
        "user_message_id": c.user_message_id,
        "status": c.status,
        "reason": reason,
        "choices": offered.iter().map(|k| json!({"kwant_id": k.id, "name": k.name, "title": k.title})).collect::<Vec<_>>(),
        "general": {"label": "Answer generally", "name": general.0, "kwant_id": general.1},
        "chosen_kwant_id": c.chosen_kwant_id,
        "chose_general": c.general,
    })
}

fn card_text(offered: &[Kwant]) -> String {
    let names: Vec<String> = offered.iter().map(|k| format!("{} ({})", k.name, k.title)).collect();
    format!("Who should answer this? It could be {}, or a general answer.", names.join(" or "))
}

/// Posts the card asking who answers `user_message_id` and returns the send route's response
/// (`status: "asking"`). No turn runs until the person chooses.
#[allow(clippy::too_many_arguments)]
pub(super) async fn ask(
    state: &AppState,
    space: &SpaceId,
    channel: &crate::db::models::Channel,
    all: &[Kwant],
    user_message_id: &str,
    prompt: &str,
    offered: &[Kwant],
    reason: &str,
) -> Result<Value, StatusCode> {
    let ids: Vec<String> = offered.iter().map(|k| k.id.clone()).collect();
    let c = state
        .db
        .add_route_choice(space.as_str(), &channel.id, user_message_id, prompt, &ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let general = general_label(state, space, channel, all).await;
    let meta = card_meta(&c, offered, &general, reason);
    let msg = NewMessage {
        channel_id: Some(channel.id.clone()),
        direct_kwant_id: None,
        sender_role: "system".into(),
        sender_id: "kwnt".into(),
        text: card_text(offered),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let card = state
        .db
        .insert_message_with(&msg, &MessageExtra { thread_id: None, meta: Some(meta.clone()) })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = state.db.set_route_choice_card(&c.id, &card.id).await;
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
    let _ = state.event_sender.send(RuntimeEvent::RouteChoiceUpdated { choice: meta.clone() });
    tracing::info!(route_choice = %c.id, channel = %channel.id, "asked the person who answers a close call");
    Ok(json!({
        "status": "asking",
        "user_message_id": user_message_id,
        "route_choice_id": c.id,
        "card_message_id": card.id,
        "choices": meta["choices"],
        "general": meta["general"],
    }))
}

#[derive(Debug, Deserialize)]
pub struct ResolveBody {
    #[serde(default)]
    pub kwant_id: Option<String>,
    #[serde(default)]
    pub general: bool,
}

/// `POST /api/route-choices/:id/resolve` `{kwant_id}` or `{general: true}`: the person settles a
/// close call and the chosen bot answers the waiting message. Person only. 409 when it was
/// already settled, 400 for a bot that wasn't offered.
pub async fn resolve_route(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<ResolveBody>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person chooses who answers").await {
        return r;
    }
    let Ok(Some(c)) = state.db.get_route_choice(&id).await else {
        return error(StatusCode::NOT_FOUND, "no such choice");
    };
    if c.status != "open" {
        return error(StatusCode::CONFLICT, "already chosen");
    }
    let space = SpaceId::new(&c.space_id);
    let Ok(Some(channel)) = state.db.get_channel(&c.channel_id).await else {
        return error(StatusCode::NOT_FOUND, "the channel is gone");
    };
    let all = state.db.list_kwants(&space).await.unwrap_or_default();
    let offered: Vec<Kwant> = c.choice_ids().iter().filter_map(|i| all.iter().find(|k| &k.id == i).cloned()).collect();
    let kwant = match (&body.kwant_id, body.general) {
        (Some(_), true) | (None, false) => return error(StatusCode::BAD_REQUEST, "send kwant_id or general: true"),
        (Some(k), false) => match offered.iter().find(|o| &o.id == k) {
            Some(o) => o.clone(),
            None => return error(StatusCode::BAD_REQUEST, "that bot wasn't offered"),
        },
        (None, true) => match general_answerer(&state, &space, &channel, &all).await {
            Some(k) => k,
            None => return error(StatusCode::CONFLICT, "no one here can answer generally"),
        },
    };
    match state.db.resolve_route_choice(&id, &kwant.id, body.general).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::CONFLICT, "already chosen"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "could not save the choice"),
    }
    let c = state.db.get_route_choice(&id).await.ok().flatten().unwrap_or(c);
    let general = general_label(&state, &space, &channel, &all).await;
    let reason = if body.general {
        format!("You asked for a general answer, so {} answers", kwant.name)
    } else {
        format!("You chose {} ({}) for this", kwant.name, kwant.title)
    };
    let meta = card_meta(&c, &offered, &general, &reason);
    if let Some(mid) = &c.card_message_id {
        let _ = state.db.set_message_meta(mid, &meta).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::RouteChoiceUpdated { choice: meta });
    // The person's pick teaches routing (a general answer says nothing about the bots)
    if !body.general {
        learn(&state, &space, &c.user_message_id, &kwant.id, "ask").await;
    }
    let candidates = offered.len() + 1;
    let mut routing = super::routes::routing_meta(&kwant, &reason, candidates, false, &[], "ask", &c.user_message_id);
    routing["route_choice_id"] = json!(c.id);
    let (st, k, um, ch, prompt) = (state.clone(), kwant.clone(), c.user_message_id.clone(), c.channel_id.clone(), c.prompt.clone());
    tokio::spawn(async move {
        if let Err(e) = super::routes::run_routed_turn(&st, &k, Some(ch), None, prompt, Some(routing), vec![um]).await {
            tracing::warn!(error = %e, kwant = %k.id, "the chosen bot's turn didn't finish");
        }
    });
    Json(json!({"status": "dispatched", "user_message_id": c.user_message_id, "kwant": kwant.name, "kwant_id": kwant.id})).into_response()
}

/// Saves "messages like `message_id` go to `kwant_id`" for routing to learn from.
async fn learn(state: &AppState, space: &SpaceId, message_id: &str, kwant_id: &str, source: &str) {
    let Ok(Some(m)) = state.db.get_message(message_id).await else { return };
    if let Err(e) = state.db.add_route_example(space.as_str(), message_id, &m.text, kwant_id, source).await {
        tracing::warn!(error = %e, "could not save a routing example");
    }
}

#[derive(Debug, Deserialize)]
pub struct CorrectionBody {
    pub message_id: String,
    pub kwant_id: String,
}

/// `POST /api/route-corrections` `{message_id, kwant_id}`: the person says who should have
/// answered their channel message; similar messages go to that bot from now on (basis
/// "learned"). Person only. It doesn't rerun the message.
pub async fn correct_route(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<CorrectionBody>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person corrects routing").await {
        return r;
    }
    let msg = match state.db.get_message(&body.message_id).await {
        Ok(Some(m)) => m,
        _ => return error(StatusCode::NOT_FOUND, "no such message"),
    };
    let (Some(channel_id), None, "user") = (msg.channel_id.as_deref(), msg.thread_id.as_deref(), msg.sender_role.as_str()) else {
        return error(StatusCode::BAD_REQUEST, "only your own messages in a channel can be corrected");
    };
    let Ok(Some(channel)) = state.db.get_channel(channel_id).await else {
        return error(StatusCode::NOT_FOUND, "the channel is gone");
    };
    let space = SpaceId::new(&channel.space_id);
    let all = state.db.list_kwants(&space).await.unwrap_or_default();
    let concierge = super::concierge::id(&state, &space, &all).await;
    let members = super::routes::routing_members(&channel, &all, concierge.as_deref());
    let Some(kwant) = members.iter().find(|k| k.id == body.kwant_id) else {
        return error(StatusCode::BAD_REQUEST, "that bot doesn't answer in this channel");
    };
    learn(&state, &space, &msg.id, &kwant.id, "correction").await;
    Json(json!({"status": "learned", "message_id": msg.id, "kwant_id": kwant.id, "kwant": kwant.name})).into_response()
}

pub fn tool() -> Value {
    json!({
        "name": HAND_BACK,
        "description": "Hand the person's message you were routed in this channel back when it is clearly not your area, so kwnt sends it \
            to the bot here that fits it best (or a generalist). Use it only when someone else is clearly better placed, not to \
            avoid work. It works once per message. After it succeeds, end your turn with one short line saying who takes it; \
            don't answer the request yourself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "reason": {"type": "string", "description": "Why this isn't your area, in one sentence for the person"},
            },
            "required": ["reason"],
        },
    })
}

type ToolResult = (String, bool);

/// The `hand_back` tool: one hop to the next best bot, else the Concierge, else the lead.
pub async fn tool_call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    let reason = args["reason"].as_str().map(str::trim).filter(|r| !r.is_empty());
    let Some(reason) = reason else {
        return ("Say why in `reason`.".into(), true);
    };
    let reason: String = reason.chars().take(300).collect();
    let Some(origin) = scope.origin.as_ref().filter(|o| o.depth == 0 && o.thread_id.is_none()) else {
        return ("hand_back works only for a message the person sent in a channel.".into(), true);
    };
    let Some(channel_id) = origin.channel_id.clone() else {
        return ("hand_back works only for a message the person sent in a channel.".into(), true);
    };
    // The message this turn answers, never just the newest one: in a busy channel that can be
    // another person's message routed to someone else
    let Some(message_id) = origin.message_id.clone() else {
        return ("hand_back works only for a message kwnt routed to you; this one was sent to you directly, so answer it.".into(), true);
    };
    match hand_back(state, &scope.kwant_id, &channel_id, &message_id, &reason).await {
        Ok(to) => (format!("Handed to {to}. End your turn now with one short line saying {to} will take it; don't answer the request."), false),
        Err(why) => (why, true),
    }
}

/// Sends the person's message `message_id` in `channel_id` on from `from_id`; returns who takes it.
pub(super) async fn hand_back(state: &AppState, from_id: &str, channel_id: &str, message_id: &str, reason: &str) -> Result<String, String> {
    let Ok(Some(channel)) = state.db.get_channel(channel_id).await else {
        return Err("Could not find this channel.".into());
    };
    let Ok(Some(from)) = state.db.get_kwant(from_id).await else {
        return Err("Could not find you.".into());
    };
    let space = SpaceId::new(&from.space_id);
    let msg = match state.db.get_message(message_id).await {
        Ok(Some(m)) if m.sender_role == "user" && m.channel_id.as_deref() == Some(channel_id) => m,
        _ => return Err("Could not find the person's message you were answering, so it can't be handed back.".into()),
    };
    if state.db.handed_back(&msg.id).await.unwrap_or(true) {
        return Err("This message was already handed back once; answer it as well as you can.".into());
    }
    let all = state.db.list_kwants(&space).await.unwrap_or_default();
    let concierge = super::concierge::id(state, &space, &all).await;
    let members: Vec<Kwant> = super::routes::routing_members(&channel, &all, concierge.as_deref()).into_iter().filter(|k| k.id != from.id).collect();
    let to = next_after_handback(state, &space, &channel, &all, &members, &from, &msg.text, concierge.as_deref()).await;
    let Some((to, fit)) = to else {
        return Err("No one else here fits it better; answer it as well as you can.".into());
    };
    match state.db.add_route_handback(&msg.id, &from.id, &to.id, reason).await {
        Ok(true) => {}
        Ok(false) => return Err("This message was already handed back once; answer it as well as you can.".into()),
        Err(_) => return Err("Could not hand it back.".into()),
    }
    let why = format!("{} handed this back (\"{reason}\"), so {} ({}) takes it", from.name, to.name, to.title);
    let mut routing = super::routes::routing_meta(&to, &why, members.len() + 1, false, &[], "handback", &msg.id);
    routing["fit"] = json!(fit);
    routing["handed_back_by"] = json!(from.id);
    let prompt = format!(
        "{}\n\n(kwnt routing note: {} handed this message to you as not their area: {reason}. Answer it.)",
        msg.text, from.name
    );
    tracing::info!(from = %from.id, to = %to.id, message = %msg.id, "a bot handed a message back");
    let (st, k, um) = (state.clone(), to.clone(), msg.id.clone());
    let ch = channel.id.clone();
    tokio::spawn(async move {
        if let Err(e) = super::routes::run_routed_turn(&st, &k, Some(ch), None, prompt, Some(routing), vec![um]).await {
            tracing::warn!(error = %e, kwant = %k.id, "the hand-back turn didn't finish");
        }
    });
    Ok(to.name)
}

/// Who takes a handed-back message: the next best member when it fits (at least [`ROUTE_AT`]),
/// else the Concierge (#general, made if needed; anywhere it already exists), else the lead.
/// Never the bot that handed it back.
#[allow(clippy::too_many_arguments)]
async fn next_after_handback(
    state: &AppState,
    space: &SpaceId,
    channel: &crate::db::models::Channel,
    all: &[Kwant],
    members: &[Kwant],
    from: &Kwant,
    text: &str,
    concierge: Option<&str>,
) -> Option<(Kwant, f32)> {
    if !members.is_empty() {
        let hints = hints_for(state, space, members).await;
        let lead = if channel.lead_kwant_id == from.id { members[0].id.clone() } else { channel.lead_kwant_id.clone() };
        let route = state.router.route_channel_query_hinted(text, &channel.name, &lead, members, None, &hints).await;
        if route.fit >= ROUTE_AT {
            if let Some(k) = members.iter().find(|k| k.id == route.kwant_id) {
                return Some((k.clone(), route.fit));
            }
        }
    }
    if concierge != Some(from.id.as_str()) {
        let c = if channel.name == "general" {
            super::concierge::for_no_match(state, space, &channel.id, all).await
        } else {
            concierge.and_then(|id| all.iter().find(|k| k.id == id)).cloned()
        };
        if let Some(c) = c {
            return Some((c, 0.0));
        }
    }
    let lead = all.iter().find(|k| k.id == channel.lead_kwant_id && k.id != from.id && members.iter().any(|m| m.id == k.id));
    lead.cloned().map(|k| (k, 0.0))
}
