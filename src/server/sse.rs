use axum::{
    extract::State,
    http::Uri,
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::BroadcastStream;
use super::AppState;
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;

/// Event types that belong to no space and so reach every scoped stream when their `space_id`
/// is null. Empty today: every event names a kwant, chat, team, routine, skill or approval. An
/// event that can't be placed in a space and isn't listed here never reaches a scoped stream.
pub const GLOBAL_EVENTS: &[&str] = &[];

/// Something an event names that decides its space.
#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    /// The space itself, carried by the event.
    Space(String),
    /// A channel, kwant (its direct chat), thread or team id.
    Context(String),
    Routine(String),
    Skill(String),
    Approval(String),
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn contexts<'a>(ids: impl IntoIterator<Item = &'a Option<String>>) -> Vec<Owner> {
    ids.into_iter().flatten().filter(|s| !s.is_empty()).cloned().map(Owner::Context).collect()
}

fn fields(v: &Value, keys: &[&str], to: fn(String) -> Owner) -> Vec<Owner> {
    keys.iter().filter_map(|k| str_field(v, k)).map(to).collect()
}

/// What decides `event`'s space, most specific first. Every variant is listed with no catch-all,
/// so a new event has to say here how it is placed. An empty list means the event can't be
/// placed: it reaches scoped streams only if its type is in [`GLOBAL_EVENTS`].
pub fn owners(event: &RuntimeEvent) -> Vec<Owner> {
    use RuntimeEvent as E;
    match event {
        E::Scoped { tag, .. } => contexts([&Some(tag.context_id.clone()), &Some(tag.kwant_id.clone())]),
        // Only ever sent inside a turn (`Scoped`); bare, they belong nowhere
        E::TextDelta(_)
        | E::ThinkingDelta(_)
        | E::ToolCallPending { .. }
        | E::ToolCallCompleted { .. }
        | E::Activity { .. } => Vec::new(),
        // Never broadcast: the turn's stall watchdog and spend meter take them
        E::Progress | E::UsageSoFar { .. } => Vec::new(),
        E::ApprovalRequired { id, .. } | E::ApprovalWithdrawn { id } => vec![Owner::Approval(id.clone())],
        E::TurnComplete { kwant_id, .. } => contexts([kwant_id]),
        E::TurnStarted { kwant_id, context_id, thread_id, .. }
        | E::TurnEnded { kwant_id, context_id, thread_id, .. }
        | E::TurnQueued { kwant_id, context_id, thread_id, .. } => {
            contexts([context_id, thread_id, &Some(kwant_id.clone())])
        }
        E::ThreadUpdated { thread } => fields(thread, &["kwant_id", "id"], Owner::Context),
        E::DelegationUpdated { delegation } => fields(
            delegation,
            &["lead_kwant_id", "specialist_kwant_id", "thread_id", "origin_channel_id"],
            Owner::Context,
        ),
        E::MessageAdded { message } => {
            fields(message, &["channel_id", "direct_kwant_id", "thread_id"], Owner::Context)
        }
        E::SkillUpdated { skill } => {
            let mut out = fields(skill, &["space_id"], Owner::Space);
            out.extend(fields(skill, &["kwant_id"], Owner::Context));
            out.extend(fields(skill, &["id"], Owner::Skill));
            out
        }
        E::QuestionUpdated { question: v } | E::HumanStepUpdated { step: v } => fields(
            v,
            &["kwant_id", "channel_id", "direct_kwant_id", "thread_id"],
            Owner::Context,
        ),
        E::McpSuggestionUpdated { suggestion } => {
            let mut out = fields(suggestion, &["space_id"], Owner::Space);
            out.extend(fields(suggestion, &["kwant_id", "channel_id", "direct_kwant_id", "thread_id"], Owner::Context));
            out
        }
        E::BotActionUpdated { action: v }
        | E::HireUpdated { hire: v }
        | E::CapabilityChanged { capability: v }
        | E::FactChanged { fact: v }
        | E::FactDispute { dispute: v } => {
            let mut out = fields(v, &["space_id"], Owner::Space);
            out.extend(fields(v, &["kwant_id", "channel_id", "direct_kwant_id", "thread_id"], Owner::Context));
            out
        }
        E::ProfileSwitched { kwant_id, .. } | E::MemoryChanged { kwant_id } => contexts([&Some(kwant_id.clone())]),
        E::DiscussionStarted { team_id, .. }
        | E::BrowserNavigated { team_id, .. }
        | E::BrowserScreenshot { team_id, .. }
        | E::SandboxStatusChanged { team_id, .. } => contexts([&Some(team_id.clone())]),
        E::DiscussionTurnStarted { speaker_id, .. } => contexts([&Some(speaker_id.clone())]),
        E::DiscussionCompleted { context_id, .. } | E::RoutingStarted { context_id, .. } => contexts([context_id]),
        E::RoutingDecided { context_id, kwant_id, .. } => contexts([context_id, &Some(kwant_id.clone())]),
        E::DecideUpdated { kwant_id, context_id, thread_id, .. } => contexts([context_id, thread_id, &Some(kwant_id.clone())]),
        E::Error { kwant_id, .. } => contexts([kwant_id]),
        E::RoutineRun { routine_id, .. } => vec![Owner::Routine(routine_id.clone())],
        E::TaskUpdated { task: v } | E::MissionUpdated { mission: v } => fields(v, &["space_id"], Owner::Space),
        E::Notify { space_id, kwant_id, channel_id, .. } => {
            let mut out: Vec<Owner> = space_id.iter().cloned().map(Owner::Space).collect();
            out.extend(contexts([channel_id, &Some(kwant_id.clone())]));
            out
        }
    }
}

async fn resolve(db: &Database, owner: &Owner) -> Option<String> {
    let found = match owner {
        Owner::Space(s) => return Some(s.clone()),
        Owner::Context(id) => db.space_of_context(id).await,
        Owner::Routine(id) => db.space_of_routine(id).await,
        Owner::Skill(id) => db.space_of_skill(id).await,
        Owner::Approval(id) => db.space_of_approval(id).await,
    };
    found.ok().flatten().map(|s| s.as_str().to_string())
}

/// The event's wire JSON with the `space_id` it belongs to, or `null` when it can't be placed.
/// `cache` maps owners already looked up to their space for one connection.
pub async fn wire_event(db: &Database, cache: &mut HashMap<String, Option<String>>, event: &RuntimeEvent) -> Value {
    let mut v = event.to_wire();
    let Value::Object(map) = &mut v else {
        return v;
    };
    let mut space = None;
    for owner in owners(event) {
        let key = format!("{owner:?}");
        if !cache.contains_key(&key) {
            // A long-lived connection sees ids come and go; start over rather than grow forever
            if cache.len() >= 10_000 {
                cache.clear();
            }
            let found = resolve(db, &owner).await;
            cache.insert(key.clone(), found);
        }
        if let Some(s) = cache.get(&key).cloned().flatten() {
            space = Some(s);
            break;
        }
    }
    map.insert("space_id".into(), space.map(Value::String).unwrap_or(Value::Null));
    v
}

/// `GET /api/events`: every event, each tagged with its `space_id`. With `?space=<id>`, only
/// that space's events and the [`GLOBAL_EVENTS`]; an event that can't be placed is dropped. An
/// unknown space is a 404 and an archived one a 409, as for the collections.
pub async fn sse_handler(State(state): State<AppState>, uri: Uri) -> Response {
    let only = match super::spaces::space_param(uri.query()) {
        None => None,
        Some(id) => match super::spaces::active_space(&state, &id).await {
            Ok(space) => Some(space.as_str().to_string()),
            Err(r) => return r,
        },
    };
    let rx = state.event_sender.subscribe();
    let cache = Arc::new(Mutex::new(HashMap::new()));
    let stream = BroadcastStream::new(rx)
        .filter_map(|msg| async move { msg.ok() })
        .then(move |event| {
            let db = state.db.clone();
            let cache = cache.clone();
            async move { wire_event(&db, &mut *cache.lock().await, &event).await }
        })
        .filter_map(move |wire| {
            let keep = in_space(&wire, only.as_deref());
            async move { keep.then(|| Ok::<_, Infallible>(Event::default().data(wire.to_string()))) }
        });

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

/// Whether a wire event goes to a connection scoped to `space` (every event when unscoped).
/// Fails closed: on a scoped stream an event with no space passes only if it is global.
pub fn in_space(wire: &Value, space: Option<&str>) -> bool {
    match (space, wire.get("space_id").and_then(Value::as_str)) {
        (None, _) => true,
        (Some(want), Some(got)) => want == got,
        (Some(_), None) => wire.get("type").and_then(Value::as_str).is_some_and(|t| GLOBAL_EVENTS.contains(&t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::events::{RoutingCandidate, TurnTag};
    use serde_json::json;

    /// One of every event, each naming kwant `k`, chat `c`, team `t`, routine `r`, skill `s` or
    /// approval `a` wherever it can name something. The match has no catch-all, so a new event
    /// fails to compile here until it gets a sample, and then the test below places it.
    fn samples() -> Vec<RuntimeEvent> {
        use RuntimeEvent as E;
        let s = |x: &str| x.to_string();
        let all = vec![
            E::TextDelta(s("x")),
            E::ThinkingDelta(s("x")),
            E::ToolCallPending { id: s("x"), name: s("x"), args: json!({}) },
            E::ToolCallCompleted { id: s("x"), result: json!({}) },
            E::Progress,
            E::UsageSoFar { fresh: 1 },
            E::ApprovalRequired { id: s("a"), tool_name: s("x"), details: json!({}) },
            E::ApprovalWithdrawn { id: s("a") },
            E::TurnComplete { tokens: 1, duration_ms: 1, kwant_id: Some(s("k")), profile_id: None },
            E::TurnStarted { kwant_id: s("k"), context_id: Some(s("c")), turn_id: None, thread_id: None },
            E::TurnEnded { kwant_id: s("k"), context_id: None, outcome: s("completed"), turn_id: None, thread_id: None },
            E::TurnQueued { turn_id: s("x"), kwant_id: s("k"), context_id: None, thread_id: None, position: 1, reason: s("x") },
            E::ThreadUpdated { thread: json!({"id": "th", "kwant_id": "k"}) },
            E::DelegationUpdated { delegation: json!({"id": "d", "lead_kwant_id": "k", "thread_id": "th"}) },
            E::MessageAdded { message: json!({"id": "m", "channel_id": "c"}) },
            E::SkillUpdated { skill: json!({"id": "s", "kwant_id": null, "deleted": true}) },
            E::QuestionUpdated { question: json!({"id": "q", "kwant_id": "k"}) },
            E::Activity { step: json!({"id": "s1"}) },
            E::HumanStepUpdated { step: json!({"id": "h", "kwant_id": "k"}) },
            E::McpSuggestionUpdated { suggestion: json!({"id": "g", "space_id": "spc_default", "kwant_id": "k"}) },
            E::BotActionUpdated { action: json!({"id": "b", "space_id": "spc_default", "kwant_id": "k"}) },
            E::HireUpdated { hire: json!({"id": "h", "space_id": "spc_default", "kwant_id": "k"}) },
            E::CapabilityChanged { capability: json!({"space_id": "spc_default", "kwant_id": "k", "task_class": "general"}) },
            E::FactChanged { fact: json!({"id": "f", "space_id": "spc_default", "kwant_id": "k"}) },
            E::FactDispute { dispute: json!({"id": "d", "space_id": "spc_default"}) },
            E::Scoped {
                tag: TurnTag { turn_id: s("x"), kwant_id: s("k"), context_id: s("c"), thread_id: None },
                event: Box::new(E::TextDelta(s("x"))),
            },
            E::ProfileSwitched {
                kwant_id: s("k"),
                from_profile_id: s("x"),
                from_name: s("x"),
                to_profile_id: s("x"),
                to_name: s("x"),
                class: s("quota"),
                retry_at: None,
            },
            E::DiscussionStarted { team_id: s("t"), team_name: s("x"), total_turns: 1, speakers: vec![], plan: vec![], kind: s("x") },
            E::DiscussionTurnStarted {
                speaker_id: s("k"),
                speaker_name: s("x"),
                turn_index: 0,
                total_turns: 1,
                prompt: s("x"),
                asked_by: None,
                phase: None,
            },
            E::DiscussionCompleted { total_turns: 1, context_id: Some(s("t")) },
            E::BrowserNavigated { team_id: s("t"), url: s("x"), title: s("x"), status_code: 200 },
            E::BrowserScreenshot { team_id: s("t"), url: s("x"), image_base64: s("x"), timestamp: s("x") },
            E::SandboxStatusChanged { team_id: s("t"), container_name: s("x"), is_running: true },
            E::RoutingStarted {
                context_id: Some(s("c")),
                candidates: vec![RoutingCandidate { id: s("k"), name: s("x"), title: s("x") }],
            },
            E::RoutingDecided {
                context_id: Some(s("c")),
                kwant_id: s("k"),
                kwant_name: s("x"),
                kwant_title: s("x"),
                reason: s("x"),
                mode: s("direct"),
                candidate_count: None,
                fallback: false,
                matched_terms: vec![],
                basis: None,
            },
            E::DecideUpdated {
                kwant_id: s("k"),
                context_id: Some(s("c")),
                thread_id: None,
                state: s("deciding"),
                held_turn_id: s("x"),
                consulted: vec![],
                outcome: None,
            },
            E::Error { code: s("x"), message: s("x"), kwant_id: Some(s("k")) },
            E::MemoryChanged { kwant_id: s("k") },
            E::RoutineRun { routine_id: s("r"), run_id: s("x"), status: s("ok") },
            E::TaskUpdated { task: json!({"id": "t", "space_id": "spc_x"}) },
            E::MissionUpdated { mission: json!({"id": "m", "space_id": "spc_x"}) },
            E::Notify {
                kind: s("x"),
                kwant_id: s("k"),
                kwant_name: s("x"),
                channel_id: None,
                title: s("x"),
                body: s("x"),
                space_id: Some(s("spc_x")),
                space_name: None,
            },
        ];
        for e in &all {
            match e {
                E::TextDelta(_)
                | E::ThinkingDelta(_)
                | E::ToolCallPending { .. }
                | E::ToolCallCompleted { .. }
                | E::Progress
                | E::UsageSoFar { .. }
                | E::ApprovalRequired { .. }
                | E::ApprovalWithdrawn { .. }
                | E::TurnComplete { .. }
                | E::TurnStarted { .. }
                | E::TurnEnded { .. }
                | E::TurnQueued { .. }
                | E::ThreadUpdated { .. }
                | E::DelegationUpdated { .. }
                | E::MessageAdded { .. }
                | E::SkillUpdated { .. }
                | E::QuestionUpdated { .. }
                | E::Activity { .. }
                | E::HumanStepUpdated { .. }
                | E::McpSuggestionUpdated { .. }
                | E::BotActionUpdated { .. }
                | E::HireUpdated { .. }
                | E::CapabilityChanged { .. }
                | E::FactChanged { .. }
                | E::FactDispute { .. }
                | E::Scoped { .. }
                | E::ProfileSwitched { .. }
                | E::DiscussionStarted { .. }
                | E::DiscussionTurnStarted { .. }
                | E::DiscussionCompleted { .. }
                | E::BrowserNavigated { .. }
                | E::BrowserScreenshot { .. }
                | E::SandboxStatusChanged { .. }
                | E::RoutingStarted { .. }
                | E::RoutingDecided { .. }
                | E::DecideUpdated { .. }
                | E::Error { .. }
                | E::MemoryChanged { .. }
                | E::RoutineRun { .. }
                | E::TaskUpdated { .. }
                | E::MissionUpdated { .. }
                | E::Notify { .. } => {}
            }
        }
        all
    }

    fn wire_type(e: &RuntimeEvent) -> String {
        e.to_wire()["type"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn every_event_is_placed_in_a_space_or_is_global() {
        // Only ever sent inside a turn; bare, they are dropped from scoped streams on purpose
        let turn_only = ["TextDelta", "ThinkingDelta", "ToolCallPending", "ToolCallCompleted", "Progress", "UsageSoFar", "activity"];
        for e in samples() {
            let t = wire_type(&e);
            if turn_only.contains(&t.as_str()) && !matches!(e, RuntimeEvent::Scoped { .. }) {
                assert!(owners(&e).is_empty(), "{t}");
                continue;
            }
            assert!(
                !owners(&e).is_empty() || GLOBAL_EVENTS.contains(&t.as_str()),
                "{t} names nothing to find its space by and isn't global, so scoped streams drop it"
            );
        }
    }

    #[test]
    fn nested_ids_place_records() {
        let msg = RuntimeEvent::MessageAdded { message: json!({"channel_id": null, "direct_kwant_id": "k1"}) };
        assert_eq!(owners(&msg), vec![Owner::Context("k1".into())]);
        let del = RuntimeEvent::DelegationUpdated { delegation: json!({"lead_kwant_id": "k2"}) };
        assert_eq!(owners(&del)[0], Owner::Context("k2".into()));
        let skill = RuntimeEvent::SkillUpdated { skill: json!({"id": "s1", "space_id": "spc_b", "kwant_id": null}) };
        assert_eq!(owners(&skill)[0], Owner::Space("spc_b".into()));
    }

    #[test]
    fn a_scoped_stream_drops_events_with_no_space() {
        let placed = json!({"type": "message_added", "space_id": "spc_a"});
        let unplaced = json!({"type": "message_added", "space_id": null});
        assert!(in_space(&placed, Some("spc_a")));
        assert!(!in_space(&placed, Some("spc_b")));
        assert!(!in_space(&unplaced, Some("spc_a")), "fails closed");
        assert!(in_space(&unplaced, None), "the unscoped stream still gets everything");
    }
}
