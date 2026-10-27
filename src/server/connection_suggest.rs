//! `connection_suggest`: a bot that lacks access to an app (Google Search Console, Google
//! Analytics, HubSpot…) offers the person the connection instead of reporting "not possible".
//! It posts a card in the chat naming the app and why; nothing connects until the person signs in
//! through Connections (`POST /api/connections/composio/apps/:slug/connect`, person-only).
//! The card's `meta.kind` is `connection_suggestion`; its text reads on its own where the app has
//! no card for it.

use super::continuity::SelfScope;
use super::AppState;
use crate::db::models::{MessageExtra, NewMessage};
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use crate::mcp::composio;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const TOOL: &str = "connection_suggest";
/// Most connection cards one turn may post.
pub const MAX_PER_TURN: usize = 3;
const MAX_REASON_CHARS: usize = 300;
const MAX_NAME_CHARS: usize = 60;

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": TOOL,
        "description": "Offer the user a one-tap connection to an app you need but can't reach (for example Google \
            Search Console, Google Analytics, Google Ads, HubSpot, Notion, Slack), shown as a card in this chat. \
            Connections go through Composio; `app` is its app slug, lower case with underscores \
            (google_search_console, google_analytics, googleads, hubspot). This connects nothing: the user signs in \
            themselves, so post it rather than offering to or asking whether to. Use it instead of reporting that something is \
            impossible for lack of access, then carry on \
            with everything you can do without it. At most 3 per turn.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "Composio app slug, e.g. google_search_console"},
                "name": {"type": "string", "description": "The app's name as the user knows it, e.g. Google Search Console"},
                "reason": {"type": "string", "description": format!("What it would let you do for this request (max {MAX_REASON_CHARS} characters)")},
            },
            "required": ["app", "name", "reason"],
        },
    })]
}

/// Whether the space's Composio connector can start a sign-in: added, with its key, switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposioReady {
    Ready,
    Off,
    NeedsKey,
    NotAdded,
}

impl ComposioReady {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Off => "off",
            Self::NeedsKey => "needs_key",
            Self::NotAdded => "not_added",
        }
    }

    /// The step before the sign-in, in the card's words; `None` when there is none.
    fn first_step(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Off => Some("switch the Composio connector on in Connections"),
            Self::NeedsKey => Some("add your Composio API key to the Composio connector in Connections"),
            Self::NotAdded => Some("add Composio in Connections (free account at composio.dev)"),
        }
    }
}

async fn composio_ready(state: &AppState, space: &SpaceId) -> ComposioReady {
    let servers = state.db.list_mcp_servers(space).await.unwrap_or_default();
    let Some(server) = servers.iter().find(|s| super::connections::is_composio(s)) else {
        return ComposioReady::NotAdded;
    };
    if super::connections::composio_key(server).is_none() {
        ComposioReady::NeedsKey
    } else if !server.enabled {
        ComposioReady::Off
    } else {
        ComposioReady::Ready
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The card's text, readable without a card: what, why, and the steps.
pub fn card_text(name: &str, reason: &str, ready: ComposioReady) -> String {
    let steps = match ready.first_step() {
        Some(first) => format!("To connect it, {first}, then open Connections → Composio apps, find {name} and press Connect."),
        None => format!("To connect it, open Connections → Composio apps, find {name} and press Connect."),
    };
    format!("Suggested connection: {name}. {reason} {steps} You sign in on the app's own page; nothing is connected until you do.")
}

/// Which (turn, app) pairs already got a card, so a turn posts each app once and at most
/// [`MAX_PER_TURN`] in all. In memory: a restart ends every turn anyway.
fn posted() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static POSTED: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    POSTED.get_or_init(Default::default)
}

/// Records `app` for `turn`; `Err` says why it isn't posted again.
fn claim_slot(turn: &str, app: &str) -> Result<(), String> {
    let mut map = posted().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() > 1000 {
        map.clear();
    }
    let apps = map.entry(turn.to_string()).or_default();
    if apps.iter().any(|a| a == app) {
        return Err(format!("You already suggested {app} this turn. Carry on with your reply."));
    }
    if apps.len() >= MAX_PER_TURN {
        return Err(format!("You already suggested {MAX_PER_TURN} connections this turn. Carry on with your reply."));
    }
    apps.push(app.to_string());
    Ok(())
}

pub async fn tool_call(state: &AppState, scope: &SelfScope, args: &Value) -> (String, bool) {
    let app = args["app"].as_str().map(str::trim).unwrap_or("").to_ascii_lowercase();
    let name = one_line(args["name"].as_str().unwrap_or(""));
    let reason = one_line(args["reason"].as_str().unwrap_or(""));
    if !composio::valid_slug(&app) {
        return ("app must be a Composio app slug: lower-case letters, digits, _ or -, e.g. google_search_console".into(), true);
    }
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return (format!("name is required, up to {MAX_NAME_CHARS} characters"), true);
    }
    if reason.is_empty() || reason.chars().count() > MAX_REASON_CHARS {
        return (format!("reason is required, up to {MAX_REASON_CHARS} characters"), true);
    }
    let Some(origin) = scope.origin.as_ref() else {
        return (
            "There is no chat to show a card in. Say in your reply which connection you need and that the user adds it in Connections.".into(),
            true,
        );
    };
    let Ok(Some(space)) = state.db.space_of_kwant(&scope.kwant_id).await else {
        return ("Could not post the suggestion.".into(), true);
    };
    if let Some(turn) = &scope.turn_id {
        if let Err(why) = claim_slot(turn, &app) {
            return (why, false);
        }
    }
    let ready = composio_ready(state, &space).await;
    let meta = json!({
        "kind": "connection_suggestion",
        "kwant_id": scope.kwant_id,
        "provider": "composio",
        "app": app,
        "name": name,
        "reason": reason,
        "composio": ready.as_str(),
        "connect": format!("/api/connections/composio/apps/{app}/connect"),
    });
    let message = NewMessage {
        channel_id: origin.channel_id.clone(),
        direct_kwant_id: origin.direct_kwant_id.clone(),
        sender_role: "kwant".into(),
        sender_id: scope.kwant_id.clone(),
        text: card_text(&name, &reason, ready),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = MessageExtra { thread_id: origin.thread_id.clone(), meta: Some(meta) };
    let card = match state.db.insert_message_with(&message, &extra).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "connection_suggest: could not post the card");
            return ("Could not post the suggestion.".into(), true);
        }
    };
    if let Some(t) = &origin.thread_id {
        let _ = state.db.touch_thread(t).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
    tracing::info!(kwant_id = %scope.kwant_id, app = %app, composio = ready.as_str(), "agent suggested a connection");
    let first = ready.first_step().map(|s| format!(" First the user has to {s}.")).unwrap_or_default();
    (
        format!(
            "Posted a card suggesting {name}.{first} Nothing is connected yet; once the user connects it, its tools reach you \
            through the Composio server (mcp__composio__*) from a later turn. Don't wait: carry on with everything you can do \
            without it, and in your reply say what connecting it would add."
        ),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_text_names_the_app_the_reason_and_the_steps() {
        let t = card_text("Google Search Console", "It shows which pages Google indexed.", ComposioReady::Ready);
        assert!(t.starts_with("Suggested connection: Google Search Console. It shows which pages Google indexed."));
        assert!(t.contains("Connections → Composio apps, find Google Search Console and press Connect"));
        assert!(!t.contains("API key"));
        let t = card_text("Google Search Console", "x", ComposioReady::NotAdded);
        assert!(t.contains("add Composio in Connections"), "{t}");
        assert!(card_text("A", "x", ComposioReady::NeedsKey).contains("Composio API key"));
        assert!(card_text("A", "x", ComposioReady::Off).contains("switch the Composio connector on"));
    }

    #[test]
    fn a_turn_suggests_each_app_once_and_three_at_most() {
        let turn = "turn-connection-suggest-unit";
        assert!(claim_slot(turn, "google_search_console").is_ok());
        assert!(claim_slot(turn, "google_search_console").unwrap_err().contains("already suggested google_search_console"));
        assert!(claim_slot(turn, "google_analytics").is_ok());
        assert!(claim_slot(turn, "hubspot").is_ok());
        assert!(claim_slot(turn, "notion").unwrap_err().contains("3 connections"));
        assert!(claim_slot("another-turn-unit", "notion").is_ok());
    }
}
