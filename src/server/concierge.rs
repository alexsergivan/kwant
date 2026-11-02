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

/// Similar requests the Concierge must have had (this one included) within [`SIMILAR_WINDOW_DAYS`]
/// before it suggests a specialist.
pub const SIMILAR_FOR_HIRE: usize = 3;
pub const SIMILAR_WINDOW_DAYS: i64 = 14;

/// Records that the Concierge answers `message_id` and, when it is the [`SIMILAR_FOR_HIRE`]th
/// similar request in [`SIMILAR_WINDOW_DAYS`] days, returns their ids, oldest first, for one hire
/// suggestion (see [`hire_note`]). Requests a hire was already suggested from don't count again,
/// and nothing is suggested while a Concierge proposal is pending.
pub async fn similar_requests(app: &AppState, space: &SpaceId, message_id: &str, text: &str, concierge_id: &str) -> Option<Vec<String>> {
    if let Err(e) = app.db.add_route_example(space.as_str(), message_id, text, concierge_id, "concierge").await {
        tracing::warn!(error = %e, "could not save the Concierge request");
        return None;
    }
    let since = chrono::Utc::now() - chrono::Duration::days(SIMILAR_WINDOW_DAYS);
    let recent = app.db.concierge_examples(space.as_str(), since).await.ok()?;
    let ids = similar_ids(text, message_id, &recent);
    if ids.len() < SIMILAR_FOR_HIRE {
        return None;
    }
    let pending = app.db.list_hires(space, Some("pending")).await.unwrap_or_default();
    if pending.iter().any(|h| h.kwant_id == concierge_id) {
        return None;
    }
    let _ = app.db.mark_hire_suggested(&ids).await;
    Some(ids)
}

/// `message_id` and the earlier requests in `recent` (newest first) that are like `text` and no
/// hire was suggested from, oldest first.
pub fn similar_ids(text: &str, message_id: &str, recent: &[crate::db::routing::RouteExample]) -> Vec<String> {
    let words = crate::router::team::content_words(text);
    let mut ids: Vec<String> = recent
        .iter()
        .filter(|e| e.message_id != message_id && e.hire_suggested_at.is_none())
        .filter(|e| crate::router::team::alike(&words, &crate::router::team::content_words(&e.text)))
        .map(|e| e.message_id.clone())
        .collect();
    ids.reverse();
    ids.push(message_id.to_string());
    ids
}

/// Added to the Concierge's turn when similar requests keep coming: answer, then suggest one
/// specialist through the hire policy, which still refuses a role the person turned down.
pub fn hire_note(ids: &[&str]) -> String {
    format!(
        "(kwnt routing note: this is request {} of a kind you have answered in the last {SIMILAR_WINDOW_DAYS} days; message \
        ids: {}. Answer it first. Then, if one specialist bot would clearly do this kind of request better, call \
        propose_hire once with evidence {{\"basis\": \"repeated\", \"similar_requests\": [those ids]}}. Check hire_status \
        first and don't propose a role the person turned down. If propose_hire refuses, don't retry.)",
        ids.len(),
        ids.join(", ")
    )
}

#[cfg(test)]
mod hire_tests {
    use super::*;
    use crate::db::routing::RouteExample;

    fn ex(id: &str, text: &str, suggested: bool) -> RouteExample {
        RouteExample {
            id: format!("e-{id}"),
            space_id: "s".into(),
            message_id: id.into(),
            text: text.into(),
            kwant_id: "conc".into(),
            source: "concierge".into(),
            hire_suggested_at: suggested.then(chrono::Utc::now),
            created_at: chrono::Utc::now(),
        }
    }

    /// The third similar request makes the count; unlike ones and ones already suggested from don't.
    #[test]
    fn the_third_similar_request_suggests_a_hire() {
        let now = "Best street food stalls in Munich this weekend?";
        let one = vec![ex("m1", "Where is the best street food in Munich?", false), ex("m0", "What's the weather tomorrow?", false)];
        assert_eq!(similar_ids(now, "m3", &one), vec!["m1", "m3"]);
        assert!(similar_ids(now, "m3", &one).len() < SIMILAR_FOR_HIRE);
        let two = [vec![ex("m2", "Good street food markets in Munich?", false)], one.clone()].concat();
        assert_eq!(similar_ids(now, "m3", &two), vec!["m1", "m2", "m3"]);
        let done = [vec![ex("m2", "Good street food markets in Munich?", true)], one].concat();
        assert_eq!(similar_ids(now, "m3", &done).len(), 2, "a request a hire was suggested from counts once");
        assert!(hire_note(&["m1", "m2", "m3"]).contains("\"similar_requests\""));
    }
}
