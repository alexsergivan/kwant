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
    /// A driver is still working: it printed something that isn't reply text or a tool call.
    /// Only feeds the turn's stall watchdog; it is never broadcast.
    Progress,
    /// The turn's fresh tokens so far (see `TurnUsage::fresh`), when the engine reports usage
    /// mid-turn, so a task the bot hands in during the turn can pay for it then. Never broadcast.
    UsageSoFar {
        fresh: u64,
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
        /// This turn; matches `turn_id` on the turn's streamed events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        /// Set when the turn runs in a thread (then `context_id` is the thread id too).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    /// A kwant's turn is over, however it ended: "completed", "failed" or "cancelled".
    #[serde(rename = "turn_ended", alias = "TurnEnded")]
    TurnEnded {
        kwant_id: String,
        context_id: Option<String>,
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    /// A turn is waiting for a free slot: its agent already runs as many turns as allowed
    /// (`reason` "agent_busy") or the whole server does ("server_busy"). It starts on its own,
    /// with `turn_started`; it is never dropped. `position` is 1 for the next to start.
    #[serde(rename = "turn_queued", alias = "TurnQueued")]
    TurnQueued {
        turn_id: String,
        kwant_id: String,
        context_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        position: usize,
        reason: String,
    },
    /// A thread was created, renamed or archived.
    #[serde(rename = "thread_updated", alias = "ThreadUpdated")]
    ThreadUpdated { thread: Value },
    /// A delegation changed state; `delegation` is the whole record (see `crate::db::threads`).
    #[serde(rename = "delegation_updated", alias = "DelegationUpdated")]
    DelegationUpdated { delegation: Value },
    /// kwnt wrote a message outside a streamed turn (delegation notes and results); `message` is
    /// the saved message, with `channel_id` / `direct_kwant_id` / `thread_id` saying which chat.
    #[serde(rename = "message_added", alias = "MessageAdded")]
    MessageAdded { message: Value },
    /// A skill was proposed, reviewed, edited or deleted; `skill` is the whole record (or
    /// `{id, kwant_id, deleted: true}`). See `crate::db::skills`.
    #[serde(rename = "skill_updated", alias = "SkillUpdated")]
    SkillUpdated { skill: Value },
    /// A question an agent asked the user was asked, answered or cancelled.
    #[serde(rename = "question_updated", alias = "QuestionUpdated")]
    QuestionUpdated { question: Value },
    /// A step of a running turn started or changed (thinking, a tool run, a wait for an approval
    /// or for the person). Only sent inside a turn (`Scoped`); `step` is the whole step, so a
    /// client replaces the one with the same `id`. See [`crate::drivers::activity::Step`].
    #[serde(rename = "activity", alias = "Activity")]
    Activity { step: Value },
    /// A bot asked the person to do a step in the browser (sign in, solve a CAPTCHA), or the
    /// step was done, declined or cancelled; `step` is the whole record. See
    /// `crate::db::human_steps`.
    #[serde(rename = "human_step_updated", alias = "HumanStepUpdated")]
    HumanStepUpdated { step: Value },
    /// An MCP server an agent suggested was suggested, installed or dismissed; `suggestion` is the
    /// whole record. See `crate::db::mcp_suggestions`.
    #[serde(rename = "mcp_suggestion_updated", alias = "McpSuggestionUpdated")]
    McpSuggestionUpdated { suggestion: Value },
    /// A change an agent made from a chat was made, undone, confirmed or declined; `action` is the
    /// whole record with its card. See `crate::db::bot_actions` and docs/bot-actions.md.
    #[serde(rename = "bot_action_updated", alias = "BotActionUpdated")]
    BotActionUpdated { action: Value },
    /// A bot proposed hiring a new bot, or the person decided on (or undid) a hire; `hire` is the
    /// whole proposal with its `card`. See `crate::db::hires` and docs/bot-actions.md → "Hiring".
    #[serde(rename = "hire_updated", alias = "HireUpdated")]
    HireUpdated { hire: Value },
    /// Flight School (C4): a bot proposed a tier, or the person granted, declined or kwant marked
    /// a proposal stale. `capability` is `{space_id, kwant_id, task_class, proposal?}`; clients
    /// reload `GET /api/capabilities`. See docs/mission-control.md §14.
    #[serde(rename = "capability_changed", alias = "CapabilityChanged")]
    CapabilityChanged { capability: Value },
    /// An event from inside one turn, tagged with the turn and its chat. On the SSE wire it is
    /// the inner event with `turn_id`, `kwant_id`, `context_id` and `thread_id` added at the top
    /// level (see [`RuntimeEvent::to_wire`]), so clients that ignore the tags still work.
    #[serde(rename = "scoped")]
    Scoped {
        tag: TurnTag,
        event: Box<RuntimeEvent>,
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
        /// The channel or team the discussion ran in, so the event reaches only its space.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_id: Option<String>,
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
    /// A reply with open choices is held while its bot decides them (see `crate::server::decide`):
    /// `state` is "deciding", then "done" with `outcome` "decided", "posted_original" or "cancelled".
    #[serde(rename = "decide_updated", alias = "DecideUpdated")]
    DecideUpdated {
        kwant_id: String,
        context_id: Option<String>,
        thread_id: Option<String>,
        state: String,
        held_turn_id: String,
        consulted: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
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
        /// How many agents were considered.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        candidate_count: Option<usize>,
        /// True when Laya was unreachable and keyword matching picked instead.
        #[serde(default)]
        fallback: bool,
        /// Prompt words that matched the picked agent's name, title, persona or role.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        matched_terms: Vec<String>,
        /// How the pick was made: only, keywords, skills, lead, several, or discuss.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        basis: Option<String>,
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
    /// A Mission Control task changed (state, claim, checks, evidence). `task` is the board's Task
    /// shape and carries `space_id`.
    #[serde(rename = "task_updated", alias = "TaskUpdated")]
    TaskUpdated { task: Value },
    /// A mission changed (status, budget, counts). `mission` carries `space_id`.
    #[serde(rename = "mission_updated", alias = "MissionUpdated")]
    MissionUpdated { mission: Value },
    /// Something the user asked to be told about. `kind` is one of `continuity::NOTIFY_KINDS`
    /// ("turn_finished", "approval_needed", "human_step", …). `body` never holds chat text, tool arguments or secrets.
    #[serde(rename = "notify", alias = "Notify")]
    Notify {
        kind: String,
        kwant_id: String,
        kwant_name: String,
        channel_id: Option<String>,
        title: String,
        body: String,
        /// The space the kwant lives in, so the UI can label and filter the notification.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space_name: Option<String>,
    },
}

/// Which turn and chat an event belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TurnTag {
    pub turn_id: String,
    pub kwant_id: String,
    /// Channel id, kwant id (direct chat) or thread id.
    pub context_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl RuntimeEvent {
    /// The JSON sent to SSE clients.
    pub fn to_wire(&self) -> Value {
        match self {
            RuntimeEvent::Scoped { tag, event } => {
                let mut v = event.to_wire();
                if let Value::Object(map) = &mut v {
                    map.insert("turn_id".into(), Value::String(tag.turn_id.clone()));
                    map.entry("kwant_id").or_insert_with(|| Value::String(tag.kwant_id.clone()));
                    map.insert("context_id".into(), Value::String(tag.context_id.clone()));
                    if let Some(t) = &tag.thread_id {
                        map.insert("thread_id".into(), Value::String(t.clone()));
                    }
                }
                v
            }
            other => serde_json::to_value(other).unwrap_or(Value::Null),
        }
    }

    /// The event without its turn tag.
    pub fn unscoped(&self) -> &RuntimeEvent {
        match self {
            RuntimeEvent::Scoped { event, .. } => event.unscoped(),
            other => other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingCandidate {
    pub id: String,
    pub name: String,
    pub title: String,
}
