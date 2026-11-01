//! The Concierge: a generalist bot that answers in #general when no specialist fits the message.
//!
//! It is made on the first message nothing fits (with a card the person can undo), never counts
//! against the daily cap on bots made without a card, and is never picked by role scoring. Its id
//! is kept in the `concierge:<space>` setting: set and present is the Concierge; set but gone means
//! the person removed it, so #general goes back to the lead and it is never made again unasked.

use serde_json::json;

use super::AppState;
use crate::db::bot_actions::NewBotAction;
use crate::db::models::Kwant;
use crate::db::spaces::SpaceId;

pub const NAME: &str = "Concierge";
pub const TITLE: &str = "Generalist for everyday questions";
pub const INSTRUCTIONS: &str = "You answer the questions in #general that none of the specialist bots in this space \
    are for: everyday questions, recommendations, quick facts, planning and anything general. Answer directly and \
    briefly. When a question clearly belongs to a specialist here, say who in one line and answer what you can.";
/// What it covers, seeded when it is made so the person sees (and can edit) what it is for. Role
/// scoring never picks it, so these only describe it.
pub const COVERS: &[&str] = &[
    "everyday questions", "recommendations", "food", "restaurants", "travel", "places", "events", "quick facts",
    "planning", "shopping", "general knowledge",
];
pub const SOUL: &str = "Warm, quick and plain-spoken. You'd rather give a useful short answer than a long hedge.";

fn key(space: &SpaceId) -> String {
    format!("concierge:{}", space.as_str())
}

/// Serialises making the Concierge, so two messages at once don't make two.
static MAKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Where the space stands with its Concierge.
#[derive(Debug, Clone)]
pub enum State {
    /// Never made: the first message nothing fits makes it.
    None,
    Present(Box<Kwant>),
    /// The person removed it: never made again unasked.
    Removed,
}

pub async fn state(app: &AppState, space: &SpaceId, all_kwants: &[Kwant]) -> State {
    match app.db.get_setting(&key(space)).await.ok().flatten() {
        Some(id) => match all_kwants.iter().find(|k| k.id == id) {
            Some(k) => State::Present(Box::new(k.clone())),
            None => State::Removed,
        },
        None => State::None,
    }
}

/// The Concierge's id, if the space has one now: role scoring leaves it out.
pub async fn id(app: &AppState, space: &SpaceId, all_kwants: &[Kwant]) -> Option<String> {
    match state(app, space, all_kwants).await {
        State::Present(k) => Some(k.id),
        _ => None,
    }
}

/// The Concierge for a #general message nothing fits: the existing one, else a new one made now
/// with a card in `channel_id` the person can undo. `None` when the person removed it (or it
/// couldn't be made): the message stays with today's pick.
pub async fn for_no_match(app: &AppState, space: &SpaceId, channel_id: &str, all_kwants: &[Kwant]) -> Option<Kwant> {
    let _making = MAKING.lock().await;
    let fresh = app.db.list_kwants(space).await.unwrap_or_else(|_| all_kwants.to_vec());
    match state(app, space, &fresh).await {
        State::Present(k) => return Some(*k),
        State::Removed => return None,
        State::None => {}
    }
    // A bot the person already named Concierge is adopted rather than doubled
    if let Some(k) = fresh.iter().find(|k| k.name.trim().eq_ignore_ascii_case(NAME)) {
        let _ = app.db.set_setting(&key(space), &k.id).await;
        return Some(k.clone());
    }
    let spec = json!({
        "name": NAME,
        "title": TITLE,
        "instructions": INSTRUCTIONS,
        "soul": SOUL,
        "avatar": "default",
        "model": "auto",
    });
    let view = match super::bot_actions::create_bot(app, space, &spec).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, space = %space.as_str(), "could not make the Concierge");
            return None;
        }
    };
    let id = view["id"].as_str().unwrap_or_default().to_string();
    if id.is_empty() {
        return None;
    }
    let _ = app.db.set_setting(&key(space), &id).await;
    let covers: Vec<String> = COVERS.iter().map(|c| c.to_string()).collect();
    let _ = app.db.set_scope(&id, Some(&covers), None).await;
    let new = NewBotAction {
        space_id: space.as_str().to_string(),
        kwant_id: id.clone(),
        turn_id: None,
        channel_id: Some(channel_id.to_string()),
        direct_kwant_id: None,
        thread_id: None,
        // Not "bot_create": the daily cap on bots made without a card doesn't count it
        action: "concierge_create".into(),
        target_kind: "bot".into(),
        target_id: Some(id.clone()),
        summary: format!("added {NAME}, a generalist that answers in #general when no specialist fits"),
        before: None,
        after: Some(json!({"kwant_id": id, "name": NAME, "title": TITLE})),
        status: "applied".into(),
    };
    let _ = super::bot_actions::record_with_card(app, &new, NAME).await;
    tracing::info!(kwant_id = %id, space = %space.as_str(), "made the Concierge for a message no specialist fits");
    app.db.get_kwant(&id).await.ok().flatten()
}
