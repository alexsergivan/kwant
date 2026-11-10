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
    /// The driver waits for room before its engine can start: an isolated Claude reply behind
    /// others in the VM (`reason` "vm_busy"; `position` 1 is next; `slots` run at once). The
    /// server stops the turn's stall watchdog and shows the turn as queued (`turn_queued`) until
    /// [`RuntimeEvent::RoomTaken`]. Never broadcast as is.
    WaitingForRoom {
        reason: String,
        position: usize,
        slots: usize,
    },
    /// The wait in [`RuntimeEvent::WaitingForRoom`] is over: the stall watchdog starts again from
    /// now and the server sends `turn_started` again. Never broadcast as is.
    RoomTaken,
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
    /// A pending approval is closed: answered (in kwant or from Telegram), or it can no longer
    /// be answered (its turn ended, or it expired). Both fields are absent on events stored
    /// before they existed, so `{id}` alone still reads.
    ApprovalWithdrawn {
        id: String,
        /// Where it was answered: "telegram", or absent (null) when answered in kwant or when
        /// nobody answered it. The same value as the approval's `answered_via` in `GET /api/approvals`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered_via: Option<String>,
        /// The approval's stored status after closing: "allowed_once", "always_allowed",
        /// "denied" (answered), "expired" (nobody answered in time, or an audit/generate card
        /// closed) or "cancelled" (its turn ended or was stopped while waiting).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
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
        /// No `turn_finished` notification for this turn: a mission runner turn, whose runner
        /// notifies once for the task's outcome instead (an AutoFix retry stays silent).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        quiet: bool,
    },
    /// A turn is waiting for a free slot: its agent already runs as many turns as allowed
    /// (`reason` "agent_busy") or the whole server does ("server_busy"). It starts on its own,
    /// with `turn_started`; it is never dropped. `position` is 1 for the next to start.
    ///
    /// `reason` "vm_busy": a turn that had started waits for a free spot in the VM, because
    /// `slots` isolated Claude replies already run there. It is sent after that turn's
    /// `turn_started`, and a second `turn_started` follows when it gets its spot.
    #[serde(rename = "turn_queued", alias = "TurnQueued")]
    TurnQueued {
        turn_id: String,
        kwant_id: String,
        context_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        position: usize,
        reason: String,
        /// "vm_busy" only: how many isolated Claude replies run at once.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slots: Option<usize>,
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
    /// A Show-it-once recording started, stopped, was written or lost its steps; `recording` is
    /// the recording without any step (steps never go out on SSE). See `crate::db::show_once`.
    #[serde(rename = "skill_recording_updated", alias = "RecordingUpdated")]
    RecordingUpdated { recording: Value },
    /// Show it once (A29): a Try it once turn ended. `outcome` is `done`, `failed` or `cancelled`;
    /// `bot` is the bot whose chat ran it.
    #[serde(rename = "show_once_trial_ended", alias = "TrialEnded")]
    TrialEnded { skill_id: String, recording_id: Option<String>, bot: String, outcome: String },
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
    /// A close call the person was asked to settle was asked or settled; `choice` is its card's
    /// meta (see `crate::server::rerouting`).
    #[serde(rename = "route_choice_updated", alias = "RouteChoiceUpdated")]
    RouteChoiceUpdated { choice: Value },
    /// Flight School (C4): a bot proposed a tier, or the person granted, declined or kwant marked
    /// a proposal stale. `capability` is `{space_id, kwant_id, task_class, proposal?}`; clients
    /// reload `GET /api/capabilities`. See docs/mission-control.md §14.
    #[serde(rename = "capability_changed", alias = "CapabilityChanged")]
    CapabilityChanged { capability: Value },
    /// A Truth Ledger fact changed status (written, accepted, disputed, superseded, expired);
    /// `fact` is `{id, status, space_id, scope, scope_id, kwant_id}`. Clients refetch it. See
    /// docs/mission-control.md §21.
    #[serde(rename = "fact_changed", alias = "FactChanged")]
    FactChanged { fact: Value },
    /// A dispute between two facts opened, was resolved or was dismissed; `dispute` is
    /// `{id, status, space_id, fact_a, fact_b, cause, winner}`.
    #[serde(rename = "fact_dispute", alias = "FactDispute")]
    FactDispute { dispute: Value },
    /// An event from inside one turn, tagged with the turn and its chat. On the SSE wire it is
    /// the inner event with `turn_id`, `kwant_id`, `context_id` and `thread_id` added at the top
    /// level (see [`RuntimeEvent::to_wire`]), so clients that ignore the tags still work.
    #[serde(rename = "scoped")]
    Scoped {
        tag: TurnTag,
        event: Box<RuntimeEvent>,
    },
    /// A turn moved to another account because the first ran out of quota or was signed out,
    /// on the same engine or another one (`from_engine` ≠ `to_engine`). `class` is "quota" or
    /// "auth". Sent once per chat per outage. Engines are "claude", "codex", "grok" or "custom".
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
        #[serde(default)]
        from_engine: String,
        #[serde(default)]
        to_engine: String,
        /// The model the turn was on, when it had started one; the one it moved to.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_model: Option<String>,
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
    /// A bot's status, its reason and since when changed (H4); `status` is the whole status
    /// object of `release-090/h4-contract.md` §1 (see `crate::server::bot_status`).
    #[serde(rename = "bot_status", alias = "BotStatus")]
    BotStatus { kwant_id: String, status: Value },
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
    /// A goal changed (status, checks, schedule, its fix task); `GET /api/goals/:id` has the rest.
    #[serde(rename = "goal_updated", alias = "GoalUpdated")]
    GoalUpdated { id: String, status: String, space_id: String },
    /// A Dev mode card was added or changed (plan §3.2); refetch `GET /api/spaces/:id/dev/…`.
    /// `kind` is `landed`, `review` or `release_notes`; `status` `pending`, `ready`, `failed` or `stale`.
    #[serde(rename = "dev_card", alias = "DevCard")]
    DevCard { space_id: String, id: String, kind: String, status: String },
    /// B2: a bot's pre-push card was made or changed (fresh-eyes-audit.md §4); refetch
    /// `GET /api/dev/prepush/:id`. `status` is `checking`, `blocked`, `clear`, `overridden` or `failed`.
    #[serde(rename = "dev_prepush", alias = "DevPrepush")]
    DevPrepush { space_id: String, id: String, kwant_id: String, status: String },
    /// Dev mode was turned on or off in a space.
    #[serde(rename = "dev_mode", alias = "DevMode")]
    DevMode { space_id: String, on: bool },
    /// A Jira ticket picked up by Dev mode changed state (jira-tickets J4–J14); refetch
    /// `GET /api/spaces/:id/dev/tickets/:key`. Carries no ticket text, only its key.
    #[serde(rename = "dev_ticket", alias = "DevTicket")]
    DevTicket { space_id: String, key: String, state: String },
    /// One line of a ticket's reading checklist (jira-tickets api.md 1.7): `step` is `ticket`,
    /// `comments`, `links`, `confluence` or `code`; `state` is `running`, `done` or `skipped`;
    /// `detail` is kwant's own short line ("2 comments"). Sent on the bus only, never stored.
    #[serde(rename = "dev_ticket_progress", alias = "DevTicketProgress")]
    DevTicketProgress { space_id: String, key: String, step: String, state: String, detail: String },
    /// A Telegram account paired with the space (next-077 §G). Never carries the bot token.
    #[serde(rename = "telegram.paired", alias = "TelegramPaired")]
    TelegramPaired { space_id: String, name: String, username: Option<String> },
    /// The space's Telegram pairing code ran out before anyone used it.
    #[serde(rename = "telegram.expired", alias = "TelegramExpired")]
    TelegramExpired { space_id: String },
    /// Something on the Telegram status changed (state, listening/offline, queue); refetch
    /// `GET /api/connections/telegram`.
    #[serde(rename = "telegram.status", alias = "TelegramStatus")]
    TelegramStatus { space_id: String },
    /// An idea in the idea jar changed (dropped in, peeked at, picked, edited, shelved, deleted:
    /// `status` is then `deleted`); `GET /api/ideas/:id` has the rest.
    #[serde(rename = "idea_updated", alias = "IdeaUpdated")]
    IdeaUpdated { id: String, status: String, space_id: String },
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
        /// `spend_stopped` only: what the stop held was the person's own message, not a routine,
        /// a mission or a Jira task, so Telegram may tell them to send it again.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        held_message: bool,
        /// The bot's conversation (thread) the notification is about, when it is one: a tap
        /// opens it instead of the bot's direct chat (0.8.1 F6). Absent on older events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        /// The card to show in that chat: an approval's id (0.8.1 F6).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        card_id: Option<String>,
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
    /// An `ApprovalWithdrawn` saying how the card closed: `outcome` is the approval's stored
    /// status, `answered_via` where it was answered ("telegram"; `None` in kwant or unanswered).
    pub fn approval_withdrawn(id: impl Into<String>, outcome: &str, answered_via: Option<&str>) -> Self {
        RuntimeEvent::ApprovalWithdrawn {
            id: id.into(),
            answered_via: answered_via.map(str::to_string),
            outcome: Some(outcome.to_string()),
        }
    }

    /// Whether an approval outcome is a person's answer (Allow once, Always allow, Deny), not a
    /// card that closed on its own (expired, its turn ended).
    pub fn is_answer(outcome: Option<&str>) -> bool {
        matches!(outcome, Some("allowed_once" | "always_allowed" | "denied"))
    }

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

#[cfg(test)]
mod approval_withdrawn_tests {
    use super::RuntimeEvent;
    use serde_json::json;

    #[test]
    fn an_old_id_only_withdrawn_event_still_reads() {
        let old: RuntimeEvent = serde_json::from_value(json!({"type": "ApprovalWithdrawn", "data": {"id": "a1"}})).unwrap();
        assert_eq!(old, RuntimeEvent::ApprovalWithdrawn { id: "a1".into(), answered_via: None, outcome: None });
        assert!(!RuntimeEvent::is_answer(None));
    }

    #[test]
    fn withdrawn_carries_where_and_how_it_closed() {
        let tg = serde_json::to_value(RuntimeEvent::approval_withdrawn("a1", "allowed_once", Some("telegram"))).unwrap();
        assert_eq!(tg, json!({"type": "ApprovalWithdrawn", "data": {"id": "a1", "answered_via": "telegram", "outcome": "allowed_once"}}));
        // answered in kwant: no answered_via key (reads as null)
        let app = serde_json::to_value(RuntimeEvent::approval_withdrawn("a2", "denied", None)).unwrap();
        assert_eq!(app, json!({"type": "ApprovalWithdrawn", "data": {"id": "a2", "outcome": "denied"}}));
        assert!(app["data"].get("answered_via").is_none());
        let back: RuntimeEvent = serde_json::from_value(tg).unwrap();
        assert_eq!(back, RuntimeEvent::approval_withdrawn("a1", "allowed_once", Some("telegram")));
        for o in ["allowed_once", "always_allowed", "denied"] {
            assert!(RuntimeEvent::is_answer(Some(o)));
        }
        for o in ["expired", "cancelled"] {
            assert!(!RuntimeEvent::is_answer(Some(o)));
        }
    }
}
