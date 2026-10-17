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
    /// A pending approval can no longer be answered (its turn ended, or it expired).
    ApprovalWithdrawn {
        id: String,
    },
    TurnComplete {
        tokens: usize,
        duration_ms: u64,
        /// Kwant whose turn ended; set on the events the server broadcasts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kwant_id: Option<String>,
        /// Profile that served the turn, when the engine has profiles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_id: Option<String>,
    },
    /// A kwant started working on a turn (its avatar can show it's busy until `turn_ended`).
    #[serde(rename = "turn_started", alias = "TurnStarted")]
    TurnStarted {
        kwant_id: String,
        context_id: Option<String>,
    },
    /// A kwant's turn is over, however it ended: "completed", "failed" or "cancelled".
    #[serde(rename = "turn_ended", alias = "TurnEnded")]
    TurnEnded {
        kwant_id: String,
        context_id: Option<String>,
        outcome: String,
    },
    /// A turn moved to another profile of the same engine because the first ran out of quota
    /// or was signed out. `class` is "quota" or "auth".
    #[serde(rename = "profile_switched", alias = "ProfileSwitched")]
    ProfileSwitched {
        kwant_id: String,
        from_profile_id: String,
        from_name: String,
        to_profile_id: String,
        to_name: String,
        class: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    #[serde(rename = "discussion_started", alias = "DiscussionStarted")]
    DiscussionStarted {
        team_id: String,
        team_name: String,
        total_turns: usize,
        speakers: Vec<String>,
        plan: Vec<String>,
        /// "discussion" for a planned team discussion, "handoff" when kwants @mentioned each other.
        #[serde(default)]
        kind: String,
    },
    #[serde(rename = "discussion_turn_started", alias = "DiscussionTurnStarted")]
    DiscussionTurnStarted {
        speaker_id: String,
        speaker_name: String,
        turn_index: usize,
        total_turns: usize,
        prompt: String,
        /// Kwant that handed this speaker the turn with an @mention (handoffs only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asked_by: Option<String>,
        /// "open", "reply" or "summary" for team discussions; None for handoffs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kwant_id: Option<String>,
    },
    /// A kwant's memory changed (by the user or by the kwant's own tools); refetch it.
    #[serde(rename = "memory_changed", alias = "MemoryChanged")]
    MemoryChanged { kwant_id: String },
    /// A routine run started or finished. `status` is "running", "ok", "error" or "skipped".
    #[serde(rename = "routine_run", alias = "RoutineRun")]
    RoutineRun {
        routine_id: String,
        run_id: String,
        status: String,
    },
    /// Something the user asked to be told about. `kind` is "turn_finished", "approval_needed"
    /// or "routine_failed". `body` never holds chat text, tool arguments or secrets.
    #[serde(rename = "notify", alias = "Notify")]
    Notify {
        kind: String,
        kwant_id: String,
        kwant_name: String,
        channel_id: Option<String>,
        title: String,
        body: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingCandidate {
    pub id: String,
    pub name: String,
    pub title: String,
}
