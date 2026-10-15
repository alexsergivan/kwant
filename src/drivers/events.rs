use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum RuntimeEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallPending {
        id: String,
        name: String,
        args: Value,
    },
    ToolCallCompleted {
        id: String,
        result: Value,
    },
    ApprovalRequired {
        id: String,
        tool_name: String,
        details: Value,
    },
    TurnComplete {
        tokens: usize,
        duration_ms: u64,
    },
    #[serde(rename = "discussion_started", alias = "DiscussionStarted")]
    DiscussionStarted {
        team_id: String,
        team_name: String,
        total_turns: usize,
        speakers: Vec<String>,
        plan: Vec<String>,
    },
    #[serde(rename = "discussion_turn_started", alias = "DiscussionTurnStarted")]
    DiscussionTurnStarted {
        speaker_id: String,
        speaker_name: String,
        turn_index: usize,
        total_turns: usize,
        prompt: String,
    },
    #[serde(rename = "discussion_completed", alias = "DiscussionCompleted")]
    DiscussionCompleted {
        total_turns: usize,
    },
    #[serde(rename = "browser_navigated", alias = "BrowserNavigated")]
    BrowserNavigated {
        team_id: String,
        url: String,
        title: String,
        status_code: u16,
    },
    #[serde(rename = "browser_screenshot", alias = "BrowserScreenshot")]
    BrowserScreenshot {
        team_id: String,
        url: String,
        image_base64: String,
        timestamp: String,
    },
    #[serde(rename = "sandbox_status", alias = "SandboxStatusChanged")]
    SandboxStatusChanged {
        team_id: String,
        container_name: String,
        is_running: bool,
    },
    #[serde(rename = "routing_started", alias = "RoutingStarted")]
    RoutingStarted {
        context_id: Option<String>,
        candidates: Vec<RoutingCandidate>,
    },
    #[serde(rename = "routing_decided", alias = "RoutingDecided")]
    RoutingDecided {
        context_id: Option<String>,
        kwant_id: String,
        kwant_name: String,
        kwant_title: String,
        reason: String,
        /// "direct" for a single responder, "discuss" for a multi-turn team discussion.
        mode: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingCandidate {
    pub id: String,
    pub name: String,
    pub title: String,
}
