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

/// Fields that name the chat, agent or team an event belongs to, most specific first.
const SPACE_KEYS: [&str; 5] = ["context_id", "channel_id", "thread_id", "kwant_id", "team_id"];

/// The event's wire JSON with the `space_id` it belongs to, or `null` for global events
/// (accounts, models, …). `cache` maps ids already looked up to their space for one connection.
pub async fn wire_event(db: &Database, cache: &mut HashMap<String, Option<String>>, event: &RuntimeEvent) -> Value {
    let mut v = event.to_wire();
    let Value::Object(map) = &mut v else {
        return v;
    };
    // Turn tags sit at the top; an event's own ids are in its `data`
    let ids: Vec<String> = SPACE_KEYS
        .iter()
        .flat_map(|key| [map.get(*key), map.get("data").and_then(|d| d.get(*key))])
        .flatten()
        .filter_map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
        .collect();
    let mut space = None;
    for id in &ids {
        let id = id.as_str();
        if !cache.contains_key(id) {
            // A long-lived connection sees ids come and go; start over rather than grow forever
            if cache.len() >= 10_000 {
                cache.clear();
            }
            let found = db.space_of_context(id).await.ok().flatten().map(|s| s.as_str().to_string());
            cache.insert(id.to_string(), found);
        }
        if let Some(s) = cache.get(id).cloned().flatten() {
            space = Some(s);
            break;
        }
    }
    map.insert("space_id".into(), space.map(Value::String).unwrap_or(Value::Null));
    v
}

/// `GET /api/events`: every event, each tagged with its `space_id`. With `?space=<id>`, only
/// that space's events and the global ones (`space_id` null); an unknown space is a 404 and an
/// archived one a 409, as for the collections.
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
pub fn in_space(wire: &Value, space: Option<&str>) -> bool {
    match (space, wire.get("space_id").and_then(Value::as_str)) {
        (None, _) | (_, None) => true,
        (Some(want), Some(got)) => want == got,
    }
}
