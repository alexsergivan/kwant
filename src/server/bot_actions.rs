//! Agents changing the app from a chat: routines, bots, non-secret settings, their space and its
//! channels. Each tool wraps the HTTP handler the app itself uses, scoped to the turn's agent and
//! its space. A change applies at once, is logged in `bot_actions` and shows as a card in the
//! chat; the person can undo it (`POST /api/bot-actions/:id/undo`, which refuses agents' tokens).
//! Deleting a bot, archiving a space or removing an account only files a request the person
//! confirms. Secrets, sign-ins, approvals, MCP installs and the computer stay with the person.
//! Contract: docs/bot-actions.md.

// Helpers return the ready-made error `Response`
#![allow(clippy::result_large_err)]

use super::continuity::SelfScope;
use super::scheduler::RoutinePayload;
use super::spaces::Scope;
use super::AppState;
use crate::db::bot_actions::{BotAction, NewBotAction, ACTION_STATUSES, MAX_PER_TURN};
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use crate::routines::Schedule;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The setting that holds the person's time zone (IANA); routines default to it.
pub const TIMEZONE_SETTING: &str = "timezone";

/// Settings agents may read and change; everything else in Settings is the person's.
const SETTINGS: [&str; 5] = ["timezone", "max_turns_per_agent", "max_turns_global", "delegation_follow_up_minutes", "notify_on"];

const AVATARS: [&str; 22] = [
    "default", "architect", "tech_lead", "devops", "security", "fast", "designer", "data", "qa", "product", "writer", "research",
    "teacher", "marketing", "legal", "coach", "traveler", "chef", "fitness", "party", "builder", "zen",
];

const TOOL_NAMES: [&str; 23] = [
    "routine_list",
    "routine_create",
    "follow_up_create",
    "routine_update",
    "routine_pause",
    "routine_resume",
    "routine_run_now",
    "routine_delete",
    "bot_list",
    "bot_create",
    "bot_update",
    "bot_delete",
    "settings_get",
    "settings_update",
    "space_get",
    "space_list",
    "space_create",
    "space_update",
    "space_archive",
    "channel_list",
    "channel_create",
    "channel_update",
    "profile_delete",
];

pub fn handles(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

fn schedule_schema() -> Value {
    json!({
        "type": "object",
        "description": "{\"type\":\"daily\",\"time\":\"07:00\"} | {\"type\":\"weekly\",\"time\":\"09:30\",\"weekdays\":[\"mon\",\"fri\"]} \
            | {\"type\":\"monthly\",\"time\":\"09:00\",\"days\":[1]} (days 1-31 or \"last\") \
            | {\"type\":\"cron\",\"expression\":\"0 9 * * mon#2\"} (5 fields; L, mon#2, 5L allowed) \
            | {\"type\":\"interval\",\"every_minutes\":60} (5-10080) | {\"type\":\"once\",\"at\":\"2026-10-20T07:00:00+02:00\"}. \
            Use monthly or cron for calendar rules (\"1st of each month\"), never a single once. daily, weekly, monthly and cron \
            take an IANA `tz` (e.g. Europe/Berlin); without one the person's time zone from Settings is used.",
        "properties": {
            "type": {"type": "string", "enum": ["once", "daily", "weekly", "monthly", "cron", "interval"]},
            "time": {"type": "string", "description": "HH:MM, 24-hour"},
            "weekdays": {"type": "array", "items": {"type": "string", "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]}},
            "days": {"type": "array", "description": "monthly: days of the month, 1-31 or \"last\"", "items": {"type": ["integer", "string"]}},
            "expression": {"type": "string", "description": "cron: minute hour day-of-month month day-of-week"},
            "tz": {"type": "string"},
            "every_minutes": {"type": "integer"},
            "at": {"type": "string", "description": "RFC 3339 time with offset"},
        },
        "required": ["type"],
    })
}

pub fn tools() -> Vec<Value> {
    let routine_ref = json!({"type": "string", "description": "The routine's id, or its exact name"});
    let bot_ref = json!({"type": "string", "description": "A bot in your space: its id or name"});
    vec![
        json!({
            "name": "routine_list",
            "description": "List the routines (scheduled runs) in your space, with their schedule, bot, chat and next run.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "routine_create",
            "description": "Schedule a routine: at each time, `instructions` are posted as a message to the bot, and the bot replies \
                in the chat. Use it whenever the person asks for something to happen on its own (\"every morning\", \"each Friday\", \
                \"remind me at 5\"). It applies at once and the person sees a card with Undo. By default the routine runs as you and \
                delivers into this chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Short name, e.g. \"Morning motivation\" (max 100)"},
                    "instructions": {"type": "string", "description": "What the bot should do at each run, written as a request to it (max 4000)"},
                    "schedule": schedule_schema(),
                    "bot": bot_ref,
                    "channel": {"type": "string", "description": "Where it delivers: a channel's id or name, or \"direct\" for the bot's direct chat. Default: this chat"},
                },
                "required": ["name", "instructions", "schedule"],
            },
        }),
        json!({
            "name": "follow_up_create",
            "description": "Schedule a one-off check-in by you, in this same chat (or thread), at `at`. At that time you get \
                `check` plus the exchange it follows up on, and you write to the person. Use it when you promised to check back, \
                or the task has a natural later moment (after a first workout, before a deadline). The person sees a card with Undo.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "at": {"type": "string", "description": "When, RFC 3339 with offset, e.g. 2026-10-22T18:00:00+02:00"},
                    "check": {"type": "string", "description": "What to ask or check then, written as a note to yourself (max 4000)"},
                    "name": {"type": "string", "description": "Short name; default \"Follow-up: <check>\""},
                },
                "required": ["at", "check"],
            },
        }),
        json!({
            "name": "routine_update",
            "description": "Change a routine in your space; give only what changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "routine": routine_ref,
                    "name": {"type": "string"},
                    "instructions": {"type": "string"},
                    "schedule": schedule_schema(),
                    "bot": bot_ref,
                    "channel": {"type": "string", "description": "A channel's id or name, or \"direct\""},
                },
                "required": ["routine"],
            },
        }),
        json!({"name": "routine_pause", "description": "Pause a routine in your space until it is resumed.", "inputSchema": {"type": "object", "properties": {"routine": routine_ref}, "required": ["routine"]}}),
        json!({"name": "routine_resume", "description": "Resume a paused routine in your space.", "inputSchema": {"type": "object", "properties": {"routine": routine_ref}, "required": ["routine"]}}),
        json!({"name": "routine_run_now", "description": "Run a routine in your space once now, besides its schedule (e.g. so the person can see what it will send).", "inputSchema": {"type": "object", "properties": {"routine": routine_ref}, "required": ["routine"]}}),
        json!({"name": "routine_delete", "description": "Delete a routine in your space (the person can undo it).", "inputSchema": {"type": "object", "properties": {"routine": routine_ref}, "required": ["routine"]}}),
        json!({
            "name": "bot_list",
            "description": "List the bots in your space, plus the models and accounts a bot can use.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "bot_create",
            "description": "Add a new bot to your space. It applies at once and the person sees a card with Undo.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "One word, no spaces or @, unique in the space, e.g. Chef"},
                    "title": {"type": "string", "description": "Its role, e.g. \"Recipe helper\""},
                    "instructions": {"type": "string", "description": "Its standing instructions: who it is and how it works"},
                    "soul": {"type": "string", "description": "Optional personality and voice"},
                    "model": {"type": "string", "description": "\"auto\" (default), an engine (claude, codex, grok, ollama, custom) or engine/model, from bot_list"},
                    "profile_id": {"type": "string", "description": "Optional account (from bot_list) for that engine"},
                    "avatar": {"type": "string", "enum": AVATARS},
                },
                "required": ["name", "title", "instructions"],
            },
        }),
        json!({
            "name": "bot_update",
            "description": "Change a bot in your space (yourself included); give only what changes. `mcp_servers` sets exactly which \
                already-installed tool servers it has, by name or id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bot": bot_ref,
                    "name": {"type": "string"},
                    "title": {"type": "string"},
                    "instructions": {"type": "string"},
                    "soul": {"type": "string"},
                    "model": {"type": "string"},
                    "profile_id": {"type": ["string", "null"], "description": "An account from bot_list, or null for the engine's default"},
                    "avatar": {"type": "string", "enum": AVATARS},
                    "mcp_servers": {"type": "array", "items": {"type": "string"}},
                },
                "required": ["bot"],
            },
        }),
        json!({
            "name": "bot_delete",
            "description": "Ask the person to delete a bot in your space: they get a card to confirm. Only a bot with no history yet can be deleted.",
            "inputSchema": {"type": "object", "properties": {"bot": bot_ref, "reason": {"type": "string"}}, "required": ["bot"]},
        }),
        json!({
            "name": "settings_get",
            "description": "Read the app settings you may change (time zone, parallel work limits, delegation follow-ups, which events notify the person).",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "settings_update",
            "description": format!("Change app settings: {}. They apply to every space. Secrets, engines, sign-ins, approvals, \
                the computer and tool servers stay with the person.", SETTINGS.join(", ")),
            "inputSchema": {
                "type": "object",
                "properties": {"settings": {"type": "object", "description": "e.g. {\"timezone\": \"Europe/Berlin\"}"}},
                "required": ["settings"],
            },
        }),
        json!({"name": "space_get", "description": "Your space: its name, icon and colour.", "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "space_list", "description": "The spaces (separate sets of bots and chats) the person has.", "inputSchema": {"type": "object", "properties": {}}}),
        json!({
            "name": "space_create",
            "description": "Create a new, empty space. You can't act inside it: the person opens it and adds bots there.",
            "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "icon": {"type": "string"}, "color": {"type": "string"}}, "required": ["name"]},
        }),
        json!({
            "name": "space_update",
            "description": "Rename or restyle your space.",
            "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "icon": {"type": "string"}, "color": {"type": ["string", "null"]}}},
        }),
        json!({
            "name": "space_archive",
            "description": "Ask the person to archive your space: they get a card to confirm.",
            "inputSchema": {"type": "object", "properties": {"reason": {"type": "string"}}},
        }),
        json!({"name": "channel_list", "description": "The channels in your space.", "inputSchema": {"type": "object", "properties": {}}}),
        json!({
            "name": "channel_create",
            "description": "Create a channel in your space. Members are bots of your space; you lead it unless `lead` says otherwise.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "description": {"type": "string"},
                    "lead": bot_ref,
                    "members": {"type": "array", "items": {"type": "string"}, "description": "Bots' ids or names"},
                },
                "required": ["name"],
            },
        }),
        json!({
            "name": "channel_update",
            "description": "Set a channel's pinned instructions, which every bot there follows; \"\" clears them.",
            "inputSchema": {
                "type": "object",
                "properties": {"channel": {"type": "string", "description": "Its id or name"}, "instructions": {"type": "string"}},
                "required": ["channel", "instructions"],
            },
        }),
        json!({
            "name": "profile_delete",
            "description": "Ask the person to remove an engine account (from bot_list) in your space: they get a card to confirm.",
            "inputSchema": {"type": "object", "properties": {"profile_id": {"type": "string"}, "reason": {"type": "string"}}, "required": ["profile_id"]},
        }),
    ]
}

// Refusals: what stays with the person, and where they do it

const SECRETS: &str = "Keys, tokens, passwords and webhook addresses are secrets you never handle. Ask the person to enter it \
    themselves: API keys and engine accounts in Space settings → Accounts, the notification webhook in App settings → Notifications.";
const ENGINES: &str = "Engines and their sign-in are the person's. Ask them to sign in, sign out or point to Ollama in Space settings → Accounts \
    or App settings.";
const AUTH: &str = "How people sign in to kwnt is the person's alone; ask them to change it on the computer kwnt runs on.";
const APPROVALS: &str = "Approvals and questions are answered by the person, never by a bot: ask them to answer the card in the chat.";
const COMPUTER: &str = "Where bots browse and run commands is the person's choice: ask them in App settings → Computer for browsing and commands.";
const AGENT_TOOLS: &str = "Only the person can switch bots' own tools on or off, in App settings.";
const MCP: &str = "Tool servers are installed by the person. Use mcp_suggest to offer one; they see an Install card. \
    Their keys and headers are entered by the person in Space settings → Tools.";

/// Why an agent may not change setting `key`, with where the person does it; `None` when it may.
fn refused_setting(key: &str) -> Option<String> {
    let k = key.trim().to_ascii_lowercase();
    if SETTINGS.contains(&k.as_str()) {
        return None;
    }
    let has = |words: &[&str]| words.iter().any(|w| k.contains(w));
    let why = if has(&["key", "token", "password", "secret", "webhook", "credential"]) {
        SECRETS
    } else if has(&["auth", "login_", "sign_in", "signin"]) {
        AUTH
    } else if has(&["ollama", "engine", "login", "logout", "profile", "account"]) {
        ENGINES
    } else if has(&["approv", "permission", "question"]) {
        APPROVALS
    } else if has(&["computer", "machine", "sandbox", "vm"]) {
        COMPUTER
    } else if k == "agent_tools" {
        AGENT_TOOLS
    } else if has(&["mcp"]) {
        MCP
    } else {
        return Some(format!("There is no setting \"{key}\" you can change; settings_get lists them: {}.", SETTINGS.join(", ")));
    };
    Some(why.to_string())
}

// Calling the app's own handlers

/// The handler's JSON body, or its error message.
async fn read(resp: Response) -> Result<Value, String> {
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20).await.unwrap_or_default();
    let body: Value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).trim().to_string()));
    if status.is_success() {
        return Ok(body);
    }
    let message = body["error"]
        .as_str()
        .map(str::to_string)
        .or_else(|| body.as_str().filter(|s| !s.is_empty()).map(str::to_string))
        .unwrap_or_else(|| match status {
            StatusCode::NOT_FOUND => "not found".into(),
            StatusCode::BAD_REQUEST => "that isn't valid".into(),
            StatusCode::CONFLICT => "that clashes with something that already exists".into(),
            _ => status.canonical_reason().unwrap_or("failed").to_ascii_lowercase(),
        });
    Err(message)
}

fn space_uri(path: &str, space: &SpaceId) -> Uri {
    format!("{path}?space={space}").parse().unwrap_or_else(|_| Uri::from_static("/"))
}

/// One tool call: the turn's agent and the space it may act in.
struct Ctx<'a> {
    state: &'a AppState,
    scope: &'a SelfScope,
    space: SpaceId,
}

type ToolResult = (String, bool);

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

fn other_space(what: &str) -> ToolResult {
    fail(format!(
        "That {what} is in another space. You can only change your own space; ask the person to switch to that space and do it there."
    ))
}

fn arg<'v>(args: &'v Value, key: &str) -> Option<&'v str> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

pub async fn call(state: &AppState, scope: &SelfScope, name: &str, args: &Value) -> ToolResult {
    let space = match state.db.space_of_kwant(&scope.kwant_id).await {
        Ok(Some(s)) => s,
        _ => return fail("Could not find your space."),
    };
    let cx = Ctx { state, scope, space };
    let changes = !matches!(
        name,
        "routine_list" | "bot_list" | "settings_get" | "space_get" | "space_list" | "channel_list"
    );
    if changes {
        if let Err(e) = super::spaces::ensure_active(state, &cx.space).await {
            return fail(read(e).await.err().unwrap_or_else(|| "your space is archived".into()));
        }
        match state.db.bot_actions_in_turn(&scope.kwant_id, scope.turn_id.as_deref()).await {
            Ok(n) if n >= MAX_PER_TURN => {
                return fail(format!(
                    "You already made {MAX_PER_TURN} changes this turn, the most one turn may make. Tell the person what is left \
                    and let them ask again."
                ))
            }
            Ok(_) => {}
            Err(_) => return fail("Could not check your changes this turn."),
        }
    }
    match name {
        "routine_list" => cx.routine_list().await,
        "routine_create" => cx.routine_create(args).await,
        "follow_up_create" => cx.follow_up_create(args).await,
        "routine_update" => cx.routine_update(args).await,
        "routine_pause" => cx.routine_enable(args, false).await,
        "routine_resume" => cx.routine_enable(args, true).await,
        "routine_run_now" => cx.routine_run_now(args).await,
        "routine_delete" => cx.routine_delete(args).await,
        "bot_list" => cx.bot_list().await,
        "bot_create" => cx.bot_create(args).await,
        "bot_update" => cx.bot_update(args).await,
        "bot_delete" => cx.bot_delete(args).await,
        "settings_get" => cx.settings_get().await,
        "settings_update" => cx.settings_update(args).await,
        "space_get" => cx.space_get().await,
        "space_list" => cx.space_list().await,
        "space_create" => cx.space_create(args).await,
        "space_update" => cx.space_update(args).await,
        "space_archive" => cx.space_archive(args).await,
        "channel_list" => cx.channel_list().await,
        "channel_create" => cx.channel_create(args).await,
        "channel_update" => cx.channel_update(args).await,
        "profile_delete" => cx.profile_delete(args).await,
        _ => fail(format!("Unknown tool {name}.")),
    }
}

/// A schedule in words, for cards and replies: "daily 07:00 Europe/Berlin".
pub fn schedule_text(s: &Value) -> String {
    let str_of = |k: &str| s[k].as_str().unwrap_or("").to_string();
    match s["type"].as_str() {
        Some("once") => format!("once at {}", str_of("at")),
        Some("daily") => format!("daily {} {}", str_of("time"), str_of("tz")),
        Some("weekly") => {
            let days: Vec<&str> = s["weekdays"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            format!("weekly on {} at {} {}", days.join(", "), str_of("time"), str_of("tz"))
        }
        Some("interval") => format!("every {} minutes", s["every_minutes"].as_i64().unwrap_or(0)),
        Some("monthly") => {
            let days: Vec<String> = s["days"]
                .as_array()
                .map(|a| a.iter().map(|d| d.as_str().map_or_else(|| d.to_string(), |x| format!("the {x} day"))).collect())
                .unwrap_or_default();
            format!("monthly on day {} at {} {}", days.join(", ").replace("day the last day", "the last day"), str_of("time"), str_of("tz"))
        }
        Some("cron") => format!("cron \"{}\" {}", str_of("expression"), str_of("tz")),
        _ => "an unknown schedule".into(),
    }
}

impl Ctx<'_> {
    fn db(&self) -> &crate::db::Database {
        &self.state.db
    }

    // Lookups inside the agent's space

    async fn bot(&self, reference: Option<&str>) -> Result<Kwant, ToolResult> {
        let Some(r) = reference else {
            return match self.db().get_kwant(&self.scope.kwant_id).await {
                Ok(Some(k)) => Ok(k),
                _ => Err(fail("Could not load you.")),
            };
        };
        if let Ok(Some(k)) = self.db().get_kwant(r).await {
            return if k.space_id == self.space.as_str() { Ok(k) } else { Err(other_space("bot")) };
        }
        let r = r.trim_start_matches('@');
        let all = self.db().list_kwants(&self.space).await.unwrap_or_default();
        all.into_iter()
            .find(|k| k.name.eq_ignore_ascii_case(r))
            .ok_or_else(|| fail(format!("There is no bot \"{r}\" in your space; bot_list shows them.")))
    }

    async fn channel(&self, reference: &str) -> Result<crate::db::models::Channel, ToolResult> {
        if let Ok(Some(c)) = self.db().get_channel(reference).await {
            return if c.space_id == self.space.as_str() { Ok(c) } else { Err(other_space("channel")) };
        }
        let r = reference.trim_start_matches('#');
        let all = self.db().list_channels(&self.space).await.unwrap_or_default();
        all.into_iter()
            .find(|c| c.name.eq_ignore_ascii_case(r))
            .ok_or_else(|| fail(format!("There is no channel \"{reference}\" in your space; channel_list shows them.")))
    }

    async fn routine(&self, args: &Value) -> Result<crate::db::continuity::RoutineRow, ToolResult> {
        let Some(r) = arg(args, "routine") else {
            return Err(fail("routine is required: its id or name"));
        };
        if let Ok(Some(row)) = self.db().get_routine(r).await {
            return if row.space_id == self.space.as_str() { Ok(row) } else { Err(other_space("routine")) };
        }
        let all = self.db().list_routines(&self.space).await.unwrap_or_default();
        let found: Vec<_> = all.into_iter().filter(|x| x.name.eq_ignore_ascii_case(r)).collect();
        match found.len() {
            1 => Ok(found.into_iter().next().unwrap_or_else(|| unreachable!())),
            0 => Err(fail(format!("There is no routine \"{r}\" in your space; routine_list shows them."))),
            _ => Err(fail(format!("Several routines are called \"{r}\"; give its id from routine_list."))),
        }
    }

    /// The chat this turn is in, for routines that deliver "here": `(channel, thread)`; both
    /// empty for a direct chat. A thread is only "here" for its own bot.
    fn here(&self, target: &Kwant) -> (Option<String>, Option<String>) {
        let origin = self.scope.origin.as_ref();
        match origin.and_then(|o| o.thread_id.clone()) {
            Some(t) if target.id == self.scope.kwant_id => (None, Some(t)),
            Some(_) => (None, None),
            None => (origin.and_then(|o| o.channel_id.clone()), None),
        }
    }

    /// The latest message from the person in this turn's chat: what a follow-up follows up on.
    async fn anchor(&self) -> Option<String> {
        let origin = self.scope.origin.as_ref()?;
        let chat = origin.thread_id.clone().or_else(|| origin.channel_id.clone()).or_else(|| origin.direct_kwant_id.clone())?;
        let recent = self.db().list_messages(&chat, 10).await.ok()?;
        recent.into_iter().rev().find(|m| m.sender_role == "user" && !m.sender_id.starts_with("Routine: ")).map(|m| m.id)
    }

    /// Where a routine delivers: `Some(None)` for the bot's direct chat; `None` when not given.
    async fn delivery(&self, args: &Value, target: &Kwant, default_here: bool) -> Result<Option<Option<String>>, ToolResult> {
        match arg(args, "channel") {
            Some(c) if c.eq_ignore_ascii_case("direct") => Ok(Some(None)),
            Some(c) => Ok(Some(Some(self.channel(c).await?.id))),
            None if !default_here => Ok(None),
            None => {
                let origin = self.scope.origin.as_ref();
                match origin.and_then(|o| o.channel_id.clone()) {
                    Some(c) => Ok(Some(Some(c))),
                    None => {
                        let _ = target;
                        Ok(Some(None))
                    }
                }
            }
        }
    }

    /// A schedule from the tool's arguments, with the person's time zone filled in.
    async fn schedule(&self, raw: &Value) -> Result<Schedule, ToolResult> {
        let mut raw = raw.clone();
        if !raw.is_object() {
            return Err(fail("schedule must be an object such as {\"type\":\"daily\",\"time\":\"07:00\"}"));
        }
        let needs_tz = matches!(raw["type"].as_str(), Some("daily" | "weekly" | "monthly" | "cron"));
        if needs_tz && raw["tz"].as_str().map(str::trim).unwrap_or("").is_empty() {
            match self.db().get_setting(TIMEZONE_SETTING).await.ok().flatten().filter(|t| !t.trim().is_empty()) {
                Some(tz) => raw["tz"] = json!(tz),
                None => {
                    return Err(fail(
                        "The person's time zone isn't set. Pass schedule.tz (an IANA name like Europe/Berlin) if you know it, \
                        or ask the person which time zone they are in; settings_update {\"timezone\": …} saves it for next time.",
                    ))
                }
            }
        }
        let schedule: Schedule =
            serde_json::from_value(raw).map_err(|e| fail(format!("schedule is not valid: {e}. Example: {{\"type\":\"daily\",\"time\":\"07:00\"}}")))?;
        schedule.validate().map_err(|e| fail(format!("schedule is not valid: {e}")))?;
        Ok(schedule)
    }

    fn where_text(&self, channel_id: &Option<String>, channels: &[crate::db::models::Channel], bot: &str) -> String {
        match channel_id {
            Some(c) => match channels.iter().find(|x| &x.id == c) {
                Some(ch) => format!("#{}", ch.name),
                None => "a channel".into(),
            },
            None => format!("the direct chat with {bot}"),
        }
    }

    async fn routine_details(&self, view: &Value) -> Value {
        let kwant_id = view["kwant_id"].as_str().unwrap_or("");
        let bot = self.db().get_kwant(kwant_id).await.ok().flatten().map(|k| k.name).unwrap_or_default();
        let channels = self.db().list_channels(&self.space).await.unwrap_or_default();
        let channel_id = view["channel_id"].as_str().map(str::to_string);
        let delivers_to = match view["thread_id"].as_str() {
            Some(t) => match self.db().get_thread(t).await.ok().flatten() {
                Some(t) => format!("the thread \"{}\"", t.title),
                None => "a thread".into(),
            },
            None => self.where_text(&channel_id, &channels, &bot),
        };
        json!({
            "routine_id": view["id"],
            "name": view["name"],
            "kwant_id": kwant_id,
            "bot_name": bot,
            "schedule": view["schedule"],
            "schedule_text": schedule_text(&view["schedule"]),
            "enabled": view["enabled"],
            "next_run_at": view["next_run_at"],
            "channel_id": channel_id,
            "thread_id": view["thread_id"],
            "follow_up": view["context_message_id"].is_string(),
            "delivers_to": delivers_to,
        })
    }

    // Recording and the card

    #[allow(clippy::too_many_arguments)]
    async fn record(
        &self,
        action: &str,
        target_kind: &str,
        target_id: Option<String>,
        summary: String,
        before: Option<Value>,
        after: Option<Value>,
        status: &str,
    ) -> Result<BotAction, String> {
        let origin = self.scope.origin.as_ref();
        let new = NewBotAction {
            space_id: self.space.as_str().to_string(),
            kwant_id: self.scope.kwant_id.clone(),
            turn_id: self.scope.turn_id.clone(),
            channel_id: origin.and_then(|o| o.channel_id.clone()),
            direct_kwant_id: origin.and_then(|o| o.direct_kwant_id.clone()),
            thread_id: origin.and_then(|o| o.thread_id.clone()),
            action: action.into(),
            target_kind: target_kind.into(),
            target_id,
            summary,
            before,
            after,
            status: status.into(),
        };
        let sender = self.scope.kwant_name.clone();
        let has_chat = origin.is_some_and(|o| o.channel_id.is_some() || o.direct_kwant_id.is_some() || o.thread_id.is_some());
        let recorded = self
            .db()
            .record_bot_action(&new, |a| {
                if !has_chat {
                    return None;
                }
                Some((
                    NewMessage {
                        // A thread's messages carry only the thread
                        channel_id: a.channel_id.clone().filter(|_| a.thread_id.is_none()),
                        direct_kwant_id: a.direct_kwant_id.clone().filter(|_| a.thread_id.is_none()),
                        sender_role: "kwant".into(),
                        sender_id: sender,
                        text: card_text(a),
                        reasoning_content: None,
                        tool_calls: None,
                        tokens_used: 0,
                    },
                    MessageExtra { thread_id: a.thread_id.clone(), meta: Some(card_meta(a)) },
                ))
            })
            .await;
        let (a, card) = recorded.map_err(|e| {
            tracing::warn!(error = %e, action, "could not record a bot action");
            "The change was made but could not be logged.".to_string()
        })?;
        if let Some(card) = card {
            if let Some(t) = &a.thread_id {
                let _ = self.db().touch_thread(t).await;
            }
            let _ = self.state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
        }
        let _ = self.state.event_sender.send(RuntimeEvent::BotActionUpdated { action: action_json(&a) });
        tracing::info!(action_id = %a.id, kwant_id = %a.kwant_id, turn_id = ?a.turn_id, action = %a.action,
            target = ?a.target_id, status = %a.status, "bot changed the app");
        Ok(a)
    }

    /// Records an applied change and words the tool's reply.
    #[allow(clippy::too_many_arguments)]
    async fn applied(
        &self,
        action: &str,
        kind: &str,
        id: Option<String>,
        summary: String,
        before: Option<Value>,
        after: Value,
        reply: String,
    ) -> ToolResult {
        match self.record(action, kind, id, summary, before, Some(after), "applied").await {
            Ok(_) => (format!("{reply} The person sees a card with Undo. Tell them in one sentence what will happen."), false),
            Err(e) => fail(format!("{reply} {e}")),
        }
    }

    async fn requested(&self, action: &str, kind: &str, id: String, summary: String, after: Value) -> ToolResult {
        match self.record(action, kind, Some(id), summary.clone(), None, Some(after), "pending").await {
            Ok(_) => (
                format!("Asked the person: {summary}. Nothing changes until they confirm the card. Tell them it is waiting for them."),
                false,
            ),
            Err(e) => fail(e),
        }
    }

    // Routines

    async fn routine_list(&self) -> ToolResult {
        let list = match read(super::scheduler::list(State(self.state.clone()), Scope(self.space.clone())).await).await {
            Ok(Value::Array(l)) => l,
            Ok(_) => Vec::new(),
            Err(e) => return fail(e),
        };
        if list.is_empty() {
            return ("No routines in your space yet.".into(), false);
        }
        let mut lines = Vec::new();
        for r in &list {
            let d = self.routine_details(r).await;
            lines.push(format!(
                "- [{}] \"{}\" by {}: {}{}, delivers to {}; next run {}",
                r["id"].as_str().unwrap_or(""),
                r["name"].as_str().unwrap_or(""),
                d["bot_name"].as_str().unwrap_or(""),
                d["schedule_text"].as_str().unwrap_or(""),
                if r["enabled"].as_bool() == Some(true) { "" } else { " (paused)" },
                d["delivers_to"].as_str().unwrap_or(""),
                r["next_run_at"].as_str().unwrap_or("none"),
            ));
        }
        (lines.join("\n"), false)
    }

    async fn routine_create(&self, args: &Value) -> ToolResult {
        let target = match self.bot(arg(args, "bot")).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let schedule = match self.schedule(&args["schedule"]).await {
            Ok(s) => s,
            Err(e) => return e,
        };
        let mut channel_id = match self.delivery(args, &target, true).await {
            Ok(c) => c,
            Err(e) => return e,
        };
        // "Here" in a thread is that thread, when the routine runs as the thread's bot
        let thread_id = match arg(args, "channel") {
            None => self.here(&target).1,
            Some(_) => None,
        };
        if thread_id.is_some() {
            channel_id = Some(None);
        }
        // A one-off reminder made here carries the exchange that asked for it
        let context_message_id = match (&schedule, arg(args, "channel")) {
            (Schedule::Once { .. }, None) if target.id == self.scope.kwant_id => self.anchor().await,
            _ => None,
        };
        let payload = RoutinePayload {
            kwant_id: Some(target.id.clone()),
            channel_id,
            name: args["name"].as_str().map(str::to_string),
            instructions: args["instructions"].as_str().map(str::to_string),
            schedule: Some(schedule),
            enabled: Some(true),
            thread_id: Some(thread_id),
            context_message_id: Some(context_message_id),
        };
        let resp = super::scheduler::create(
            State(self.state.clone()),
            Scope(self.space.clone()),
            space_uri("/api/routines", &self.space),
            Json(payload),
        )
        .await;
        let view = match read(resp).await {
            Ok(v) => v,
            Err(e) => return fail(format!("The routine was not created: {e}")),
        };
        let d = self.routine_details(&view).await;
        let summary = format!(
            "created routine '{}', {}, delivering to {}",
            view["name"].as_str().unwrap_or(""),
            d["schedule_text"].as_str().unwrap_or(""),
            d["delivers_to"].as_str().unwrap_or("")
        );
        let reply = format!(
            "Created routine [{}] '{}': {} as {}, delivering to {}; first run {}.",
            view["id"].as_str().unwrap_or(""),
            view["name"].as_str().unwrap_or(""),
            d["schedule_text"].as_str().unwrap_or(""),
            target.name,
            d["delivers_to"].as_str().unwrap_or(""),
            view["next_run_at"].as_str().unwrap_or("not scheduled"),
        );
        self.applied("routine_create", "routine", view["id"].as_str().map(str::to_string), summary, None, d, reply).await
    }

    async fn follow_up_create(&self, args: &Value) -> ToolResult {
        let me = match self.bot(None).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        if self.scope.origin.is_none() {
            return fail("A follow-up needs a chat to come back to; this turn has none.");
        }
        let Some(check) = arg(args, "check") else {
            return fail("check is required: what to ask or check then");
        };
        let Some(at) = arg(args, "at") else {
            return fail("at is required: an RFC 3339 time with offset, e.g. 2026-10-22T18:00:00+02:00");
        };
        let at = match chrono::DateTime::parse_from_rfc3339(at) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(_) => return fail(format!("at \"{at}\" is not an RFC 3339 time with offset, e.g. 2026-10-22T18:00:00+02:00")),
        };
        if at <= chrono::Utc::now() {
            return fail("at must be in the future.");
        }
        let (channel, thread) = self.here(&me);
        let name = match arg(args, "name") {
            Some(n) => n.to_string(),
            None => format!("Follow-up: {}", check.chars().take(60).collect::<String>()),
        };
        let payload = RoutinePayload {
            kwant_id: Some(me.id.clone()),
            channel_id: Some(channel),
            name: Some(name),
            instructions: Some(check.to_string()),
            schedule: Some(Schedule::Once { at }),
            enabled: Some(true),
            thread_id: Some(thread),
            context_message_id: Some(self.anchor().await),
        };
        let resp = super::scheduler::create(
            State(self.state.clone()),
            Scope(self.space.clone()),
            space_uri("/api/routines", &self.space),
            Json(payload),
        )
        .await;
        let view = match read(resp).await {
            Ok(v) => v,
            Err(e) => return fail(format!("The follow-up was not scheduled: {e}")),
        };
        let d = self.routine_details(&view).await;
        let summary = format!("scheduled a follow-up '{}', {}, in {}", view["name"].as_str().unwrap_or(""), d["schedule_text"].as_str().unwrap_or(""), d["delivers_to"].as_str().unwrap_or(""));
        let reply = format!(
            "Scheduled follow-up [{}] for {} in {}; you'll get the note and the exchange it follows up on.",
            view["id"].as_str().unwrap_or(""),
            view["next_run_at"].as_str().unwrap_or(""),
            d["delivers_to"].as_str().unwrap_or(""),
        );
        // Recorded as a routine creation, so Undo removes it the same way
        self.applied("routine_create", "routine", view["id"].as_str().map(str::to_string), summary, None, d, reply).await
    }

    async fn routine_patch(&self, action: &str, row: crate::db::continuity::RoutineRow, payload: RoutinePayload, verb: &str) -> ToolResult {
        let before = serde_json::to_value(super::scheduler::RoutineView::from(row.clone())).unwrap_or(Value::Null);
        let resp = super::scheduler::update(State(self.state.clone()), Path(row.id.clone()), Json(payload)).await;
        let view = match read(resp).await {
            Ok(v) => v,
            Err(e) => return fail(format!("The routine was not changed: {e}")),
        };
        let d = self.routine_details(&view).await;
        let summary = format!("{verb} routine '{}', {}", view["name"].as_str().unwrap_or(""), d["schedule_text"].as_str().unwrap_or(""));
        let reply = format!(
            "{} routine '{}': {}, delivering to {}; next run {}.",
            capitalize(verb),
            view["name"].as_str().unwrap_or(""),
            d["schedule_text"].as_str().unwrap_or(""),
            d["delivers_to"].as_str().unwrap_or(""),
            view["next_run_at"].as_str().unwrap_or("none (paused)"),
        );
        self.applied(action, "routine", Some(row.id), summary, Some(before), d, reply).await
    }

    async fn routine_update(&self, args: &Value) -> ToolResult {
        let row = match self.routine(args).await {
            Ok(r) => r,
            Err(e) => return e,
        };
        let target = match arg(args, "bot") {
            Some(b) => match self.bot(Some(b)).await {
                Ok(k) => Some(k),
                Err(e) => return e,
            },
            None => None,
        };
        let delivery_bot = match &target {
            Some(k) => k.clone(),
            None => match self.db().get_kwant(&row.kwant_id).await {
                Ok(Some(k)) => k,
                _ => return fail("Could not load the routine's bot."),
            },
        };
        let schedule = if args["schedule"].is_null() {
            None
        } else {
            match self.schedule(&args["schedule"]).await {
                Ok(s) => Some(s),
                Err(e) => return e,
            }
        };
        let channel_id = match self.delivery(args, &delivery_bot, false).await {
            Ok(c) => c,
            Err(e) => return e,
        };
        let payload = RoutinePayload {
            kwant_id: target.map(|k| k.id),
            channel_id,
            name: args["name"].as_str().map(str::to_string),
            instructions: args["instructions"].as_str().map(str::to_string),
            schedule,
            enabled: None,
            ..Default::default()
        };
        self.routine_patch("routine_update", row, payload, "updated").await
    }

    async fn routine_enable(&self, args: &Value, on: bool) -> ToolResult {
        let row = match self.routine(args).await {
            Ok(r) => r,
            Err(e) => return e,
        };
        if row.enabled == on {
            return (format!("Routine '{}' is already {}.", row.name, if on { "on" } else { "paused" }), false);
        }
        let payload = RoutinePayload { enabled: Some(on), ..Default::default() };
        let (action, verb) = if on { ("routine_resume", "resumed") } else { ("routine_pause", "paused") };
        self.routine_patch(action, row, payload, verb).await
    }

    async fn routine_run_now(&self, args: &Value) -> ToolResult {
        let row = match self.routine(args).await {
            Ok(r) => r,
            Err(e) => return e,
        };
        let before = serde_json::to_value(super::scheduler::RoutineView::from(row.clone())).unwrap_or(Value::Null);
        let d = self.routine_details(&before).await;
        let run = match read(super::scheduler::run_now(State(self.state.clone()), Path(row.id.clone())).await).await {
            Ok(r) => r,
            Err(e) => return fail(format!("The routine did not run: {e}")),
        };
        let skipped = run["status"].as_str() == Some("skipped");
        let mut after = d.clone();
        after["run_id"] = run["id"].clone();
        after["run_status"] = run["status"].clone();
        let summary = format!("ran routine '{}' now", row.name);
        let reply = if skipped {
            format!("Routine '{}' is still running from before, so this run was skipped.", row.name)
        } else {
            format!("Started routine '{}' now; its reply arrives in {}.", row.name, d["delivers_to"].as_str().unwrap_or("its chat"))
        };
        match self.record("routine_run_now", "routine", Some(row.id), summary, None, Some(after), "applied").await {
            Ok(_) => (reply, false),
            Err(e) => fail(format!("{reply} {e}")),
        }
    }

    async fn routine_delete(&self, args: &Value) -> ToolResult {
        let row = match self.routine(args).await {
            Ok(r) => r,
            Err(e) => return e,
        };
        let before = serde_json::to_value(super::scheduler::RoutineView::from(row.clone())).unwrap_or(Value::Null);
        let d = self.routine_details(&before).await;
        if let Err(e) = read(super::scheduler::remove(State(self.state.clone()), Path(row.id.clone())).await).await {
            return fail(format!("The routine was not deleted: {e}"));
        }
        let summary = format!("deleted routine '{}' ({})", row.name, d["schedule_text"].as_str().unwrap_or(""));
        let reply = format!("Deleted routine '{}'.", row.name);
        self.applied("routine_delete", "routine", Some(row.id), summary, Some(before), d, reply).await
    }

    // Bots

    async fn bot_list(&self) -> ToolResult {
        let bots = self.db().kwant_views(&self.space).await.unwrap_or_default();
        let profiles = self.db().list_profiles(&self.space).await.unwrap_or_default();
        let servers = self.db().list_mcp_servers(&self.space).await.unwrap_or_default();
        let mut out = String::from("Bots in your space:\n");
        for v in &bots {
            let k = &v.kwant;
            let tools: Vec<String> = match self.db().get_kwant_mcp_server_ids(&k.id).await {
                Ok(ids) => servers.iter().filter(|s| ids.contains(&s.id)).map(|s| s.name.clone()).collect(),
                Err(_) => Vec::new(),
            };
            out.push_str(&format!(
                "- [{}] {} ({}), model {}{}{}{}\n",
                k.id,
                k.name,
                k.title,
                k.preferred_model,
                v.profile_id.as_deref().map(|p| format!(", account {p}")).unwrap_or_default(),
                if k.id == self.scope.kwant_id { " — you" } else { "" },
                if tools.is_empty() { String::new() } else { format!(", tool servers: {}", tools.join(", ")) },
            ));
        }
        out.push_str(
            "\nModels: \"auto\" lets kwnt pick; or an engine — claude, codex, grok, ollama, custom — optionally with a model, \
            e.g. claude/claude-sonnet-5-5.\n",
        );
        if profiles.is_empty() {
            out.push_str("Accounts: none in this space yet (the person adds them in Space settings → Accounts).\n");
        } else {
            out.push_str("Accounts (profile_id for that engine):\n");
            for p in profiles.iter().filter(|p| p.enabled) {
                out.push_str(&format!("- [{}] {} ({}){}\n", p.id, p.name, p.engine, if p.is_default { ", default" } else { "" }));
            }
        }
        if !servers.is_empty() {
            out.push_str(&format!(
                "Installed tool servers: {}\n",
                servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        (out, false)
    }

    fn check_model(model: &str) -> Result<(), ToolResult> {
        check_model(model).map_err(fail)
    }

    async fn bot_snapshot(&self, id: &str) -> Value {
        bot_snapshot(self.state, id).await
    }
}

/// `Err` with the reason when `model` isn't "auto", an engine or engine/model.
pub(super) fn check_model(model: &str) -> Result<(), String> {
    let pin = crate::router::parse_pin(model);
    match pin.family.as_deref() {
        None | Some("claude" | "codex" | "grok" | "custom" | "local_ollama") => Ok(()),
        Some(_) => Err(format!(
            "Unknown model \"{model}\". Use \"auto\", an engine (claude, codex, grok, ollama, custom) or engine/model."
        )),
    }
}

/// What a bot is now, for cards and Undo.
pub(super) async fn bot_snapshot(state: &AppState, id: &str) -> Value {
    let view = state.db.kwant_view(id).await.ok().flatten();
    let mcp = state.db.get_kwant_mcp_server_ids(id).await.unwrap_or_default();
    match view {
        Some(v) => json!({
            "kwant_id": v.kwant.id,
            "name": v.kwant.name,
            "title": v.kwant.title,
            "instructions": v.kwant.persona_prompt,
            "soul": v.kwant.soul,
            "avatar": v.kwant.avatar_sprite,
            "model": v.kwant.preferred_model,
            "profile_id": v.profile_id,
            "profile_failover": v.profile_failover,
            "mcp_server_ids": mcp,
        }),
        None => Value::Null,
    }
}

/// Creates a bot in `space` through the app's own handler. `spec`: `name`, `title`,
/// `instructions`, `soul?`, `avatar?`, `model?`, `profile_id?`. Returns its view.
pub(super) async fn create_bot(state: &AppState, space: &SpaceId, spec: &Value) -> Result<Value, String> {
    let mut body = json!({
        "name": spec["name"],
        "title": spec["title"],
        "persona_prompt": spec["instructions"],
        "soul": spec["soul"].as_str().unwrap_or(""),
        "avatar_sprite": spec["avatar"].as_str().unwrap_or("default"),
        "driver_type": "api",
        "preferred_model": spec["model"].as_str().unwrap_or("auto"),
    });
    if let Some(p) = spec["profile_id"].as_str().filter(|p| !p.is_empty()) {
        body["profile_id"] = json!(p);
    }
    let payload = serde_json::from_value(body).map_err(|e| e.to_string())?;
    let resp = super::routes::create_kwant(State(state.clone()), Scope(space.clone()), Json(payload)).await.into_response();
    match read(resp).await {
        Ok(v) => Ok(v),
        Err(e) if e == "that isn't valid" => Err("the account doesn't exist in your space or is for another engine.".into()),
        Err(e) => Err(e),
    }
}

/// Records an applied change that already shows on another card (a hire), without a card of its
/// own; Undo goes through `POST /api/bot-actions/:id/undo` as usual.
pub(super) async fn record_quiet(state: &AppState, new: &NewBotAction) -> Result<BotAction, String> {
    let (a, _) = state.db.record_bot_action(new, |_| None).await.map_err(|e| {
        tracing::warn!(error = %e, action = %new.action, "could not record a bot action");
        "the change could not be logged".to_string()
    })?;
    let _ = state.event_sender.send(RuntimeEvent::BotActionUpdated { action: action_json(&a) });
    tracing::info!(action_id = %a.id, kwant_id = %a.kwant_id, turn_id = ?a.turn_id, action = %a.action,
        target = ?a.target_id, status = %a.status, "bot changed the app");
    Ok(a)
}

impl Ctx<'_> {
    async fn bot_create(&self, args: &Value) -> ToolResult {
        if let Some(r) = refuse_extra(args, &["mcp_env", "env", "headers", "api_key", "password", "token"]) {
            return r;
        }
        let model = arg(args, "model").unwrap_or("auto").to_string();
        if let Err(e) = Self::check_model(&model) {
            return e;
        }
        let avatar = arg(args, "avatar").unwrap_or("default");
        if !AVATARS.contains(&avatar) {
            return fail(format!("Unknown avatar \"{avatar}\"; use one of {}.", AVATARS.join(", ")));
        }
        let (Some(name), Some(title), Some(instructions)) = (arg(args, "name"), arg(args, "title"), arg(args, "instructions")) else {
            return fail("name, title and instructions are required");
        };
        let spec = json!({
            "name": name,
            "title": title,
            "instructions": instructions,
            "soul": arg(args, "soul").unwrap_or(""),
            "avatar": avatar,
            "model": model,
            "profile_id": arg(args, "profile_id"),
        });
        let view = match create_bot(self.state, &self.space, &spec).await {
            Ok(v) => v,
            Err(e) => return fail(format!("The bot was not created: {e}")),
        };
        let id = view["id"].as_str().unwrap_or("").to_string();
        let after = self.bot_snapshot(&id).await;
        let summary = format!("created bot {} ({}), model {}", name, title, after["model"].as_str().unwrap_or("auto"));
        let reply = format!("Created bot {name} [{id}]: {title}. The person can chat with it right away.");
        self.applied("bot_create", "bot", Some(id), summary, None, after, reply).await
    }

    async fn bot_update(&self, args: &Value) -> ToolResult {
        if let Some(r) = refuse_extra(args, &["mcp_env", "env", "headers", "api_key", "password", "token"]) {
            return r;
        }
        let target = match self.bot(arg(args, "bot")).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let before = self.bot_snapshot(&target.id).await;
        let mcp_ids = match args.get("mcp_servers") {
            None | Some(Value::Null) => None,
            Some(Value::Array(list)) => {
                let servers = self.db().list_mcp_servers(&self.space).await.unwrap_or_default();
                let mut ids = Vec::new();
                for item in list {
                    let r = item.as_str().unwrap_or("").trim();
                    match servers.iter().find(|s| s.id == r || s.name.eq_ignore_ascii_case(r)) {
                        Some(s) => ids.push(s.id.clone()),
                        None => {
                            return fail(format!(
                                "No tool server \"{r}\" is installed in your space. {MCP}"
                            ))
                        }
                    }
                }
                Some(ids)
            }
            Some(_) => return fail("mcp_servers must be a list of installed tool servers' names or ids"),
        };
        let model = arg(args, "model").map(str::to_string).unwrap_or_else(|| target.preferred_model.clone());
        if let Err(e) = Self::check_model(&model) {
            return e;
        }
        if let Some(a) = arg(args, "avatar") {
            if !AVATARS.contains(&a) {
                return fail(format!("Unknown avatar \"{a}\"; use one of {}.", AVATARS.join(", ")));
            }
        }
        let mut body = json!({
            "name": arg(args, "name").unwrap_or(&target.name),
            "title": arg(args, "title").unwrap_or(&target.title),
            "persona_prompt": arg(args, "instructions").unwrap_or(&target.persona_prompt),
            "soul": args["soul"].as_str().map(str::trim).unwrap_or(&target.soul),
            "avatar_sprite": arg(args, "avatar").unwrap_or(&target.avatar_sprite),
            "preferred_model": model,
        });
        if args.get("profile_id").is_some() {
            body["profile_id"] = args["profile_id"].clone();
        }
        let payload = match serde_json::from_value(body) {
            Ok(p) => p,
            Err(e) => return fail(format!("The bot was not changed: {e}")),
        };
        let resp = super::routes::update_kwant(State(self.state.clone()), Path(target.id.clone()), Json(payload)).await.into_response();
        match read(resp).await {
            Ok(_) => {}
            Err(e) if e == "that isn't valid" => {
                return fail("The bot was not changed: the account doesn't exist in your space or is for another engine.")
            }
            Err(e) => return fail(format!("The bot was not changed: {e}")),
        }
        if let Some(ids) = &mcp_ids {
            if self.db().set_kwant_mcp_servers(&target.id, ids).await.is_err() {
                return fail("The bot was changed, but its tool servers could not be set.");
            }
        }
        let after = self.bot_snapshot(&target.id).await;
        let changed: Vec<&str> = ["name", "title", "instructions", "soul", "avatar", "model", "profile_id", "mcp_server_ids"]
            .into_iter()
            .filter(|k| before[*k] != after[*k])
            .collect();
        if changed.is_empty() {
            return (format!("Nothing changed for {}.", target.name), false);
        }
        let summary = format!("updated bot {}: {}", after["name"].as_str().unwrap_or(&target.name), changed.join(", ").replace("mcp_server_ids", "tool servers"));
        let reply = format!("Updated {} ({}).", after["name"].as_str().unwrap_or(&target.name), changed.join(", "));
        self.applied("bot_update", "bot", Some(target.id), summary, Some(before), after, reply).await
    }

    async fn bot_delete(&self, args: &Value) -> ToolResult {
        let target = match self.bot(arg(args, "bot")).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        if let Ok(Some(why)) = self.db().kwant_in_use(&target.id).await {
            return fail(format!(
                "{} can't be deleted: {why}, and deleting would lose it. kwnt only deletes bots with no history yet.",
                target.name
            ));
        }
        let mut after = self.bot_snapshot(&target.id).await;
        after["reason"] = json!(arg(args, "reason"));
        self.requested("bot_delete", "bot", target.id.clone(), format!("delete bot {}", target.name), after).await
    }

    // Settings

    async fn settings_values(&self) -> Result<Map<String, Value>, String> {
        let all = read(super::routes::get_settings(State(self.state.clone())).await.into_response()).await?;
        let mut out = Map::new();
        for k in SETTINGS {
            out.insert(k.to_string(), all.get(k).cloned().unwrap_or(Value::Null));
        }
        Ok(out)
    }

    async fn settings_get(&self) -> ToolResult {
        match self.settings_values().await {
            Ok(v) => (
                format!(
                    "{}\nThey apply to every space. timezone \"\" means not set; notify_on lists events that notify the person.",
                    Value::Object(v)
                ),
                false,
            ),
            Err(e) => fail(e),
        }
    }

    async fn settings_update(&self, args: &Value) -> ToolResult {
        let Some(changes) = args["settings"].as_object().filter(|m| !m.is_empty()) else {
            return fail("settings must be an object such as {\"timezone\": \"Europe/Berlin\"}");
        };
        let refusals: Vec<String> = changes.keys().filter_map(|k| refused_setting(k).map(|why| format!("{k}: {why}"))).collect();
        if !refusals.is_empty() {
            return fail(format!("Nothing was changed. {}", refusals.join(" ")));
        }
        let before = match self.settings_values().await {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        let mut payload = Map::new();
        for (k, v) in changes {
            let k = k.trim().to_ascii_lowercase();
            payload.insert(k, v.clone());
        }
        if let Err(e) = save_settings(self.state, Value::Object(payload.clone())).await {
            return fail(format!("Nothing was changed: {e}"));
        }
        let after_all = match self.settings_values().await {
            Ok(v) => v,
            Err(e) => return fail(e),
        };
        let mut before_changed = Map::new();
        let mut after_changed = Map::new();
        for k in payload.keys() {
            if before.get(k) != after_all.get(k) {
                before_changed.insert(k.clone(), before.get(k).cloned().unwrap_or(Value::Null));
                after_changed.insert(k.clone(), after_all.get(k).cloned().unwrap_or(Value::Null));
            }
        }
        if after_changed.is_empty() {
            return ("Those settings already had those values.".into(), false);
        }
        let words: Vec<String> = after_changed.iter().map(|(k, v)| format!("{k} = {v}")).collect();
        let summary = format!("changed settings: {}", words.join(", "));
        let reply = format!("Saved {} (for every space).", words.join(", "));
        self.applied("settings_update", "settings", None, summary, Some(Value::Object(before_changed)), Value::Object(after_changed), reply)
            .await
    }

    // Spaces

    async fn space_json(&self, id: &SpaceId) -> Value {
        match self.db().get_space(id).await.ok().flatten() {
            Some(s) => json!({"space_id": s.id, "name": s.name, "icon": s.icon, "color": s.color, "archived": s.archived_at.is_some()}),
            None => Value::Null,
        }
    }

    async fn space_get(&self) -> ToolResult {
        (self.space_json(&self.space).await.to_string(), false)
    }

    async fn space_list(&self) -> ToolResult {
        let spaces = self.db().list_spaces().await.unwrap_or_default();
        let lines: Vec<String> = spaces
            .iter()
            .map(|s| {
                format!(
                    "- [{}] {}{}{}",
                    s.id,
                    s.name,
                    if s.archived_at.is_some() { " (archived)" } else { "" },
                    if s.id == self.space.as_str() { " — yours" } else { "" }
                )
            })
            .collect();
        (format!("{}\nYou can only change your own space.", lines.join("\n")), false)
    }

    async fn space_create(&self, args: &Value) -> ToolResult {
        let Some(name) = arg(args, "name") else {
            return fail("name is required");
        };
        let body = super::spaces::CreateBody {
            name: name.to_string(),
            icon: arg(args, "icon").map(str::to_string),
            color: arg(args, "color").map(str::to_string),
            this_machine_enabled: None,
        };
        let created = match read(super::spaces::create(State(self.state.clone()), Json(body)).await).await {
            Ok(v) => v,
            Err(e) => return fail(format!("The space was not created: {e}")),
        };
        let id = created["id"].as_str().unwrap_or("").to_string();
        let after = self.space_json(&SpaceId::new(&id)).await;
        let reply = format!("Created the space {name} [{id}]. It is empty; the person opens it from the space switcher to add bots.");
        self.applied("space_create", "space", Some(id), format!("created space {name}"), None, after, reply).await
    }

    async fn space_update(&self, args: &Value) -> ToolResult {
        if args.get("archived").is_some() {
            return fail("Use space_archive to ask the person to archive the space.");
        }
        let before = self.space_json(&self.space).await;
        let mut body = json!({});
        for k in ["name", "icon"] {
            if let Some(v) = arg(args, k) {
                body[k] = json!(v);
            }
        }
        if args.get("color").is_some() {
            body["color"] = args["color"].clone();
        }
        if body.as_object().is_some_and(|m| m.is_empty()) {
            return fail("Give a name, icon or color to change.");
        }
        let patch = match serde_json::from_value(body) {
            Ok(p) => p,
            Err(e) => return fail(format!("The space was not changed: {e}")),
        };
        if let Err(e) = read(super::spaces::update(State(self.state.clone()), Path(self.space.to_string()), Json(patch)).await).await {
            return fail(format!("The space was not changed: {e}"));
        }
        let after = self.space_json(&self.space).await;
        let summary = format!("updated space {}", after["name"].as_str().unwrap_or(""));
        self.applied("space_update", "space", Some(self.space.to_string()), summary.clone(), Some(before), after, format!("{}.", capitalize(&summary)))
            .await
    }

    async fn space_archive(&self, args: &Value) -> ToolResult {
        if self.space.is_default() {
            return fail("The Default space can't be archived.");
        }
        let mut after = self.space_json(&self.space).await;
        let name = after["name"].as_str().unwrap_or("").to_string();
        after["reason"] = json!(arg(args, "reason"));
        self.requested("space_archive", "space", self.space.to_string(), format!("archive the space {name}"), after).await
    }

    // Channels

    async fn channel_list(&self) -> ToolResult {
        let channels = self.db().list_channels(&self.space).await.unwrap_or_default();
        let bots = self.db().list_kwants(&self.space).await.unwrap_or_default();
        let name_of = |id: &str| bots.iter().find(|k| k.id == id).map(|k| k.name.clone()).unwrap_or_else(|| id.to_string());
        let lines: Vec<String> = channels
            .iter()
            .map(|c| {
                format!(
                    "- [{}] #{} led by {}, members: {}",
                    c.id,
                    c.name,
                    name_of(&c.lead_kwant_id),
                    serde_json::from_str::<Vec<String>>(&c.member_kwants).unwrap_or_default().iter().map(|m| name_of(m)).collect::<Vec<_>>().join(", ")
                )
            })
            .collect();
        (if lines.is_empty() { "No channels yet.".into() } else { lines.join("\n") }, false)
    }

    async fn channel_create(&self, args: &Value) -> ToolResult {
        let Some(name) = arg(args, "name") else {
            return fail("name is required");
        };
        let lead = match self.bot(arg(args, "lead")).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let mut members = vec![lead.id.clone()];
        if let Some(list) = args["members"].as_array() {
            for m in list {
                match self.bot(m.as_str()).await {
                    Ok(k) if !members.contains(&k.id) => members.push(k.id),
                    Ok(_) => {}
                    Err(e) => return e,
                }
            }
        }
        let input = crate::db::models::NewChannel {
            name: name.to_string(),
            description: arg(args, "description").map(str::to_string),
            lead_kwant_id: lead.id.clone(),
            member_kwants: members,
            routing_mode: "auto".into(),
        };
        let resp = super::routes::create_channel(State(self.state.clone()), Scope(self.space.clone()), Json(input)).await.into_response();
        let created = match read(resp).await {
            Ok(v) => v,
            Err(e) => return fail(format!("The channel was not created: {e}")),
        };
        let id = created["id"].as_str().unwrap_or("").to_string();
        let after = json!({"channel_id": id, "name": created["name"], "lead_kwant_id": lead.id, "member_kwants": created["member_kwants"]});
        let reply = format!("Created #{} [{id}], led by {}.", created["name"].as_str().unwrap_or(name), lead.name);
        self.applied("channel_create", "channel", Some(id), format!("created channel #{name}"), None, after, reply).await
    }

    async fn channel_update(&self, args: &Value) -> ToolResult {
        let Some(r) = arg(args, "channel") else {
            return fail("channel is required");
        };
        let channel = match self.channel(r).await {
            Ok(c) => c,
            Err(e) => return e,
        };
        let Some(text) = args["instructions"].as_str() else {
            return fail("instructions is required (\"\" clears them)");
        };
        let before = json!({"channel_id": channel.id, "name": channel.name, "instructions": self.db().channel_instructions(&channel.id).await.ok().flatten()});
        let payload = super::continuity::InstructionsPayload { instructions: Some(text.to_string()) };
        if let Err(e) = read(super::continuity::patch_channel(State(self.state.clone()), Path(channel.id.clone()), Json(payload)).await).await {
            return fail(format!("The channel was not changed: {e}"));
        }
        let after = json!({"channel_id": channel.id, "name": channel.name, "instructions": self.db().channel_instructions(&channel.id).await.ok().flatten()});
        let verb = if text.trim().is_empty() { "cleared" } else { "set" };
        let summary = format!("{verb} the instructions of #{}", channel.name);
        self.applied("channel_update", "channel", Some(channel.id), summary.clone(), Some(before), after, format!("{}.", capitalize(&summary)))
            .await
    }

    // Accounts

    async fn profile_delete(&self, args: &Value) -> ToolResult {
        let Some(id) = arg(args, "profile_id") else {
            return fail("profile_id is required (from bot_list)");
        };
        let profile = match self.db().get_profile(id).await {
            Ok(Some(p)) => p,
            Ok(None) => return fail("There is no such account; bot_list shows them."),
            Err(_) => return fail("Could not load the account."),
        };
        if profile.space_id != self.space.as_str() {
            return other_space("account");
        }
        if profile.kind == "system" {
            return fail("The built-in account can't be removed.");
        }
        let after = json!({"profile_id": profile.id, "name": profile.name, "engine": profile.engine, "reason": arg(args, "reason")});
        self.requested("profile_delete", "profile", profile.id.clone(), format!("remove the {} account {}", profile.engine, profile.name), after)
            .await
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A refusal when the agent passed an argument that would carry a secret or install something.
fn refuse_extra(args: &Value, keys: &[&str]) -> Option<ToolResult> {
    let found = keys.iter().find(|k| args.get(**k).is_some())?;
    let why = match *found {
        "mcp_env" | "env" | "headers" => MCP,
        _ => SECRETS,
    };
    Some(fail(format!("Nothing was changed: {found} is not something you can set. {why}")))
}

async fn save_settings(state: &AppState, payload: Value) -> Result<(), String> {
    let payload = serde_json::from_value(payload).map_err(|e| e.to_string())?;
    read(super::routes::save_settings(State(state.clone()), Ok(Json(payload))).await.into_response()).await.map(|_| ())
}

// The card

fn card_text(a: &BotAction) -> String {
    let lead = match a.status.as_str() {
        "pending" => "Needs your OK",
        _ => "Done",
    };
    format!("{lead}: {}.", capitalize(&a.summary))
}

/// Whether the person can undo the action now.
fn can_undo(a: &BotAction) -> bool {
    a.status == "applied" && a.action != "routine_run_now"
}

/// The card's `meta` (docs/bot-actions.md).
pub fn card_meta(a: &BotAction) -> Value {
    json!({
        "kind": "bot_action",
        "action_id": a.id,
        "action": a.action,
        "kwant_id": a.kwant_id,
        "target_kind": a.target_kind,
        "target_id": a.target_id,
        "summary": a.summary,
        "details": a.after_json(),
        "status": a.status,
        "can_undo": can_undo(a),
        "needs_confirmation": a.status == "pending",
        "created_at": a.created_at,
        "resolved_at": a.resolved_at,
    })
}

fn action_json(a: &BotAction) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
    v["card"] = card_meta(a);
    v
}

async fn changed(state: &AppState, a: &BotAction) {
    if let Some(mid) = &a.message_id {
        let _ = state.db.set_message_meta(mid, &card_meta(a)).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::BotActionUpdated { action: action_json(a) });
}

// Routes (the person's)

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub kwant_id: Option<String>,
}

/// `GET /api/bot-actions?space=&kwant_id=&status=`: the space's actions, newest first (max 200).
pub async fn list(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ListQuery>) -> Response {
    let status = q.status.filter(|s| !s.is_empty());
    if status.as_deref().is_some_and(|s| !ACTION_STATUSES.contains(&s)) {
        return error(StatusCode::BAD_REQUEST, "unknown status");
    }
    let kwant = q.kwant_id.filter(|k| !k.is_empty());
    match state.db.list_bot_actions(&space, kwant.as_deref(), status.as_deref()).await {
        Ok(list) => Json(list.iter().map(action_json).collect::<Vec<_>>()).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn load(state: &AppState, id: &str) -> Result<BotAction, Response> {
    match state.db.get_bot_action(id).await {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err(error(StatusCode::NOT_FOUND, "action not found")),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

/// Moves the action from `from` to `to`, runs `work`, and puts it back if `work` fails.
async fn resolve<F, Fut>(state: &AppState, headers: &HeaderMap, id: &str, from: &str, to: &str, work: F) -> Response
where
    F: FnOnce(BotAction) -> Fut,
    Fut: std::future::Future<Output = Result<Option<String>, Response>>,
{
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(state, headers, "only the person can undo, confirm or decline a bot's change").await {
        return r;
    }
    let a = match load(state, id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if a.status != from {
        return error(StatusCode::CONFLICT, format!("this change is {}", a.status));
    }
    match state.db.resolve_bot_action(id, from, to).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::CONFLICT, "this change was just resolved by someone else"),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    match work(a).await {
        Ok(new_target) => {
            if let Some(t) = new_target {
                let _ = state.db.retarget_bot_action(id, &t).await;
            }
            let now = match load(state, id).await {
                Ok(a) => a,
                Err(r) => return r,
            };
            changed(state, &now).await;
            tracing::info!(action_id = %now.id, status = %now.status, "the person resolved a bot action");
            Json(action_json(&now)).into_response()
        }
        Err(r) => {
            let _ = state.db.reset_bot_action(id, from).await;
            r
        }
    }
}

/// `POST /api/bot-actions/:id/undo`: puts back what an applied change replaced. 409 when a newer
/// change to the same thing would be overwritten, or the thing can't be put back safely.
pub async fn undo(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Ok(Some(a)) = state.db.get_bot_action(&id).await {
        if a.status == "applied" && !can_undo(&a) {
            return error(StatusCode::CONFLICT, "a run that already started can't be undone");
        }
        if a.status == "applied" {
            if let Ok(Some(newer)) = state.db.newer_bot_action(&a).await {
                return error(StatusCode::CONFLICT, format!("undo the newer change first: {}", newer.summary));
            }
        }
    }
    let st = state.clone();
    resolve(&state, &headers, &id, "applied", "undone", |a| async move { undo_action(&st, &a).await }).await
}

/// `POST /api/bot-actions/:id/confirm`: the person carries out a requested deletion.
pub async fn confirm(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let st = state.clone();
    resolve(&state, &headers, &id, "pending", "confirmed", |a| async move { confirm_action(&st, &a).await.map(|_| None) }).await
}

/// `POST /api/bot-actions/:id/decline`: the person turns a request down; nothing changes.
pub async fn decline(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    resolve(&state, &headers, &id, "pending", "declined", |_| async { Ok(None) }).await
}

async fn ok_or_conflict(resp: Response, what: &str) -> Result<Value, Response> {
    read(resp).await.map_err(|e| error(StatusCode::CONFLICT, format!("{what}: {e}")))
}

async fn delete_unused_bot(state: &AppState, id: &str) -> Result<(), Response> {
    match state.db.kwant_in_use(id).await {
        Ok(None) => {}
        Ok(Some(why)) => return Err(error(StatusCode::CONFLICT, format!("the bot can't be removed: {why}"))),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
    match state.db.delete_unused_kwant(id).await {
        Ok(true) => {
            state.scheduler_wake.notify_one();
            Ok(())
        }
        Ok(false) => Err(error(StatusCode::CONFLICT, "the bot no longer exists")),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

fn routine_payload(before: &Value) -> Result<RoutinePayload, Response> {
    let schedule = serde_json::from_value(before["schedule"].clone())
        .map_err(|_| error(StatusCode::CONFLICT, "the routine's old schedule is unreadable"))?;
    Ok(RoutinePayload {
        kwant_id: before["kwant_id"].as_str().map(str::to_string),
        channel_id: Some(before["channel_id"].as_str().map(str::to_string)),
        name: before["name"].as_str().map(str::to_string),
        instructions: before["instructions"].as_str().map(str::to_string),
        schedule: Some(schedule),
        enabled: before["enabled"].as_bool(),
        thread_id: Some(before["thread_id"].as_str().map(str::to_string)),
        context_message_id: Some(before["context_message_id"].as_str().map(str::to_string)),
    })
}

/// Puts back what `a` replaced; `Some(id)` when that made a new row (an undone delete).
async fn undo_action(state: &AppState, a: &BotAction) -> Result<Option<String>, Response> {
    let target = a.target_id.clone().unwrap_or_default();
    let before = a.before_json();
    let space = SpaceId::new(&a.space_id);
    match a.action.as_str() {
        "routine_create" => {
            ok_or_conflict(super::scheduler::remove(State(state.clone()), Path(target)).await, "the routine can't be removed").await?;
        }
        "routine_update" | "routine_pause" | "routine_resume" => {
            let p = routine_payload(&before)?;
            ok_or_conflict(super::scheduler::update(State(state.clone()), Path(target), Json(p)).await, "the routine can't be put back")
                .await?;
        }
        "routine_delete" => {
            let p = routine_payload(&before)?;
            let resp = super::scheduler::create(State(state.clone()), Scope(space.clone()), space_uri("/api/routines", &space), Json(p)).await;
            let created = ok_or_conflict(resp, "the routine can't be recreated").await?;
            return Ok(created["id"].as_str().map(str::to_string));
        }
        "bot_create" => delete_unused_bot(state, &target).await?,
        "bot_hire" => {
            delete_unused_bot(state, &target).await?;
            if let Some(hire) = a.after_json()["hire_id"].as_str() {
                super::hires::undone(state, hire).await;
            }
        }
        "bot_update" => {
            if state.db.get_kwant(&target).await.ok().flatten().is_none() {
                return Err(error(StatusCode::CONFLICT, "the bot no longer exists"));
            }
            let body = json!({
                "name": before["name"],
                "title": before["title"],
                "persona_prompt": before["instructions"],
                "soul": before["soul"],
                "avatar_sprite": before["avatar"],
                "preferred_model": before["model"],
                "profile_id": before["profile_id"],
                "profile_failover": before["profile_failover"],
            });
            let payload = serde_json::from_value(body).map_err(|_| error(StatusCode::CONFLICT, "the bot's old settings are unreadable"))?;
            ok_or_conflict(
                super::routes::update_kwant(State(state.clone()), Path(target.clone()), Json(payload)).await.into_response(),
                "the bot can't be put back",
            )
            .await?;
            let ids: Vec<String> = serde_json::from_value(before["mcp_server_ids"].clone()).unwrap_or_default();
            let installed: Vec<String> = state.db.list_mcp_servers(&space).await.unwrap_or_default().into_iter().map(|s| s.id).collect();
            let ids: Vec<String> = ids.into_iter().filter(|i| installed.contains(i)).collect();
            state.db.set_kwant_mcp_servers(&target, &ids).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
        }
        "settings_update" => {
            let mut payload = before.as_object().cloned().unwrap_or_default();
            // An unset time zone reads as "", which clears it again
            if let Some(Value::Null) = payload.get("timezone") {
                payload.insert("timezone".into(), json!(""));
            }
            save_settings(state, Value::Object(payload)).await.map_err(|e| error(StatusCode::CONFLICT, e))?;
        }
        "space_create" => {
            let patch = serde_json::from_value(json!({"archived": true})).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
            ok_or_conflict(super::spaces::update(State(state.clone()), Path(target), Json(patch)).await, "the space can't be archived")
                .await?;
        }
        "space_update" => {
            let patch = serde_json::from_value(json!({"name": before["name"], "icon": before["icon"], "color": before["color"]}))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
            ok_or_conflict(super::spaces::update(State(state.clone()), Path(target), Json(patch)).await, "the space can't be put back")
                .await?;
        }
        "channel_create" => match state.db.delete_empty_channel(&target).await {
            Ok(true) => {}
            Ok(false) => return Err(error(StatusCode::CONFLICT, "the channel already has messages, so it is kept")),
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        },
        "channel_update" => {
            let text = before["instructions"].as_str().unwrap_or("").to_string();
            match state.db.set_channel_instructions(&target, &text).await {
                Ok(true) => {}
                Ok(false) => return Err(error(StatusCode::CONFLICT, "the channel no longer exists")),
                Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            }
        }
        _ => return Err(error(StatusCode::CONFLICT, "this change can't be undone")),
    }
    Ok(None)
}

async fn confirm_action(state: &AppState, a: &BotAction) -> Result<(), Response> {
    let target = a.target_id.clone().unwrap_or_default();
    match a.action.as_str() {
        "bot_delete" => delete_unused_bot(state, &target).await,
        "space_archive" => {
            let patch = serde_json::from_value(json!({"archived": true})).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
            ok_or_conflict(super::spaces::update(State(state.clone()), Path(target), Json(patch)).await, "the space can't be archived")
                .await
                .map(|_| ())
        }
        "profile_delete" => {
            ok_or_conflict(super::profile_routes::remove(State(state.clone()), Path(target)).await, "the account can't be removed")
                .await
                .map(|_| ())
        }
        _ => Err(error(StatusCode::CONFLICT, "this request can't be confirmed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_refusals_name_the_place() {
        assert!(refused_setting("timezone").is_none());
        assert!(refused_setting("anthropic_api_key").unwrap().contains("Accounts"));
        assert!(refused_setting("notify_webhook_url").unwrap().contains("Notifications"));
        assert!(refused_setting("ollama_url").unwrap().contains("Accounts"));
        assert!(refused_setting("auth_token").unwrap().contains("secret"));
        assert!(refused_setting("auth_mode").unwrap().contains("sign in to kwnt"));
        assert!(refused_setting("auto_approve").unwrap().contains("Approvals"));
        assert!(refused_setting("computer_mode").unwrap().contains("Computer"));
        assert!(refused_setting("agent_tools").unwrap().contains("own tools"));
        assert!(refused_setting("mcp_servers").unwrap().contains("mcp_suggest"));
        assert!(refused_setting("colour").unwrap().contains("no setting"));
    }

    #[test]
    fn schedules_read_as_words() {
        assert_eq!(schedule_text(&json!({"type": "daily", "time": "07:00", "tz": "Europe/Berlin"})), "daily 07:00 Europe/Berlin");
        assert_eq!(
            schedule_text(&json!({"type": "weekly", "time": "09:30", "weekdays": ["mon", "fri"], "tz": "UTC"})),
            "weekly on mon, fri at 09:30 UTC"
        );
        assert_eq!(schedule_text(&json!({"type": "interval", "every_minutes": 60})), "every 60 minutes");
    }
}
