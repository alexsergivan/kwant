//! Bots proposing to hire a new bot, and the person deciding. `propose_hire` creates nothing: it
//! records a proposal tied to the bot, turn and space and posts a card with three choices for
//! the person: Deny, Accept once or Always accept (`POST /api/hires/:id/accept|deny`, person
//! only). Accepting creates the bot through the same path as `bot_create`, adds it to the team and
//! logs a `bot_hire` action the person can undo. "Always accept" sets the space's hire policy to
//! `auto`, so later proposals there are created at once (at most [`AUTO_HIRES_PER_DAY`] a day).
//! A Deny is remembered per space and blocks proposing that kind of bot again. Every bot in the
//! space sees the policy and recent decisions on its next turn ([`prompt_block`]).
//! Contract: docs/bot-actions.md → "Hiring".

#![allow(clippy::result_large_err)]

use super::continuity::SelfScope;
use super::spaces::Scope;
use super::AppState;
use crate::db::bot_actions::NewBotAction;
use crate::db::hires::{role_key, similar_roles, Hire, NewHire, AUTO_HIRES_PER_DAY, HIRE_STATUSES, MAX_PROPOSALS_PER_TURN};
use crate::db::models::{MessageExtra, NewMessage};
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

type ToolResult = (String, bool);

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// What a bot can name as the gap a new bot fills.
const NEEDS: [&str; 4] = ["instructions", "tools", "schedule", "model"];
/// Earlier similar requests from the person that count as "this keeps coming up".
const MIN_SIMILAR_REQUESTS: usize = 3;
/// Bots one team proposal may hold.
pub const MAX_TEAM_SIZE: usize = 6;

pub fn handles(name: &str) -> bool {
    matches!(name, "propose_hire" | "hire_status")
}

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "propose_hire",
            "description": "Propose hiring a new bot for your space; the person decides on a card (Deny, Accept once, Always accept). \
                Only after you tried the request yourself (and mcp_suggest for missing tools), and only when (a) a specialist with \
                its own instructions, tools, schedule or model would clearly do this kind of work better, or (b) similar requests \
                have come up at least 3 times in this space. It creates nothing by itself; carry on with the request yourself. \
                One proposal per turn, which may hold a whole team: when the person asks for a team, put the first bot in \
                name/role/instructions and the others in members (up to 6 bots in all), with new_team to create their team \
                (and its channel) on the same card. A kind of bot the person turned down is refused unless they asked for one.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "One word, no spaces or @, unique in the space, e.g. Chef"},
                    "role": {"type": "string", "description": "Its role, e.g. \"Recipe planner\""},
                    "instructions": {"type": "string", "description": "Its standing instructions: who it is and how it works"},
                    "model": {"type": "string", "description": "\"auto\" (default), an engine (claude, codex, grok, ollama, custom) or engine/model"},
                    "tools": {"type": "array", "items": {"type": "string"}, "description": "Tool servers already installed in the space (names or ids) it should have"},
                    "team": {"type": "string", "description": "The team it joins (id or name). Default: your team"},
                    "reason": {"type": "string", "description": "Why a new bot does this better than you, in one or two sentences for the person"},
                    "evidence": {
                        "type": "object",
                        "description": "What the proposal rests on. basis \"specialist\": needs (what it needs of its own: instructions, tools, schedule, model) and gap (what you can't do well). basis \"repeated\": similar_requests, the message ids of at least 3 earlier requests from the person in this space (search_history finds them).",
                        "properties": {
                            "basis": {"type": "string", "enum": ["specialist", "repeated"]},
                            "needs": {"type": "array", "items": {"type": "string", "enum": NEEDS}},
                            "gap": {"type": "string"},
                            "similar_requests": {"type": "array", "items": {"type": "string"}},
                        },
                        "required": ["basis"],
                    },
                    "requested_by_person": {"type": "boolean", "description": "true only when the person asked for this kind of bot in this conversation"},
                    "members": {
                        "type": "array",
                        "description": "For a team: the other bots, each like the first one. The person approves them all on one card.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "role": {"type": "string"},
                                "instructions": {"type": "string"},
                                "model": {"type": "string"},
                                "tools": {"type": "array", "items": {"type": "string"}},
                            },
                            "required": ["name", "role", "instructions"],
                        },
                    },
                    "new_team": {
                        "type": "object",
                        "description": "Create a new team (with its own channel) for these bots instead of joining one; lead names the bot that leads it (default: the first).",
                        "properties": {"name": {"type": "string"}, "lead": {"type": "string"}},
                        "required": ["name"],
                    },
                },
                "required": ["name", "role", "instructions", "reason", "evidence"],
            },
        }),
        json!({
            "name": "hire_status",
            "description": "Your space's hire policy (ask each time, or always accept), how many automatic hires are left today, \
                and recent proposals with the person's decisions. Read-only: only the person changes the policy.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
    ]
}

pub async fn call(state: &AppState, scope: &SelfScope, name: &str, args: &Value) -> ToolResult {
    let space = match state.db.space_of_kwant(&scope.kwant_id).await {
        Ok(Some(s)) => s,
        _ => return fail("Could not find your space."),
    };
    match name {
        "propose_hire" => propose(state, scope, &space, args).await,
        "hire_status" => (status_text(state, &space).await, false),
        _ => fail(format!("Unknown tool {name}.")),
    }
}

fn arg<'v>(args: &'v Value, key: &str) -> Option<&'v str> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// The checked `evidence`, or why it doesn't hold up.
async fn check_evidence(state: &AppState, space: &SpaceId, raw: &Value) -> Result<Value, String> {
    const TRY_FIRST: &str = "Do the request yourself first; propose a hire only with real evidence.";
    match raw["basis"].as_str() {
        Some("specialist") => {
            let needs: Vec<String> = raw["needs"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            if needs.is_empty() || needs.iter().any(|n| !NEEDS.contains(&n.as_str())) {
                return Err(format!(
                    "evidence.needs must say what the new bot needs of its own: some of {}. {TRY_FIRST}",
                    NEEDS.join(", ")
                ));
            }
            let gap = raw["gap"].as_str().map(str::trim).unwrap_or("");
            if gap.chars().count() < 20 {
                return Err(format!("evidence.gap must say in a sentence what you can't do well yourself. {TRY_FIRST}"));
            }
            Ok(json!({"basis": "specialist", "needs": needs, "gap": gap}))
        }
        Some("repeated") => {
            let ids: Vec<String> = raw["similar_requests"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().or_else(|| v["message_id"].as_str()))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let mut found = Vec::new();
            for id in &ids {
                if found.iter().any(|f: &Value| f["message_id"] == json!(id)) {
                    continue;
                }
                match state.db.message_space_and_sender(id).await {
                    Ok(Some((s, role))) if &s == space && role == "user" => {
                        let at = state.db.message_created_at(id).await.ok().flatten();
                        found.push(json!({"message_id": id, "at": at}));
                    }
                    Ok(Some((s, _))) if &s == space => {
                        return Err(format!("Message {id} isn't a request from the person; give the person's own messages."))
                    }
                    _ => return Err(format!("There is no message {id} in your space; search_history finds earlier requests.")),
                }
            }
            if found.len() < MIN_SIMILAR_REQUESTS {
                return Err(format!(
                    "evidence.similar_requests needs at least {MIN_SIMILAR_REQUESTS} different earlier requests from the person in \
                    this space (message ids); you gave {}. {TRY_FIRST}",
                    found.len()
                ));
            }
            Ok(json!({"basis": "repeated", "similar_requests": found}))
        }
        _ => Err(format!(
            "Nothing was proposed: evidence.basis must be \"specialist\" (with needs and gap) or \"repeated\" (with \
            similar_requests). {TRY_FIRST}"
        )),
    }
}

/// Tools: only servers already installed in the space, as their ids.
async fn resolve_tools(state: &AppState, space: &SpaceId, raw: Option<&Value>) -> Result<Vec<String>, String> {
    let mut tools = Vec::new();
    match raw {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            let servers = state.db.list_mcp_servers(space).await.unwrap_or_default();
            for item in list {
                let r = item.as_str().unwrap_or("").trim();
                match servers.iter().find(|s| s.id == r || s.name.eq_ignore_ascii_case(r)) {
                    Some(s) => tools.push(s.id.clone()),
                    None => {
                        return Err(format!(
                            "Nothing was proposed: no tool server \"{r}\" is installed in your space. Suggest it with mcp_suggest; \
                            the person installs it."
                        ))
                    }
                }
            }
        }
        Some(_) => return Err("Nothing was proposed: tools must be a list of installed tool servers' names or ids.".into()),
    }
    Ok(tools)
}

/// Why a bot of this name and role can't be proposed in the space, if it can't.
fn clash(name: &str, key: &str, bots: &[crate::db::models::Kwant], denials: &[Hire], pending: &[Hire], requested: bool) -> Option<String> {
    if let Some(b) = bots.iter().find(|b| b.name.eq_ignore_ascii_case(name)) {
        return Some(format!("There is already a bot called {} in your space; ask it, or pick another name.", b.name));
    }
    if let Some(b) = bots.iter().find(|b| similar_roles(&role_key(&b.title), key)) {
        return Some(format!(
            "{} ({}) already does this kind of work in your space. Hand the work to it, or update it with bot_update, instead of \
            hiring.",
            b.name, b.title
        ));
    }
    if let (Some(d), false) = (denials.iter().find(|d| d.name.eq_ignore_ascii_case(name) || similar_roles(&d.role_key, key)), requested) {
        return Some(format!(
            "The person turned down hiring {} ({}) in this space on {}. Don't propose this kind of bot again unless they ask for \
            one; carry on with the request yourself.",
            d.name,
            d.role,
            d.resolved_at.map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default()
        ));
    }
    if let Some(p) = pending.iter().find(|p| p.name.eq_ignore_ascii_case(name) || similar_roles(&p.role_key, key)) {
        return Some(format!(
            "A proposal to hire {} ({}) is already waiting for the person. Don't propose it again; carry on with the request.",
            p.name, p.role
        ));
    }
    None
}

async fn propose(state: &AppState, scope: &SelfScope, space: &SpaceId, args: &Value) -> ToolResult {
    if let Err(e) = super::spaces::ensure_active(state, space).await {
        let _ = e;
        return fail("Your space is archived, so nothing can be added to it.");
    }
    if let Some(k) = ["env", "headers", "api_key", "password", "token", "mcp_env", "profile_id"].iter().find(|k| args.get(**k).is_some()) {
        return fail(format!(
            "Nothing was proposed: {k} is not something you can give a new bot. A new bot gets no keys or accounts; the person adds \
            them in Space settings → Accounts or Tools."
        ));
    }
    if let Some(t) = &scope.turn_id {
        match state.db.hires_in_turn(t).await {
            Ok(n) if n >= MAX_PROPOSALS_PER_TURN => {
                return fail("You already proposed a hire this turn; one per turn. Carry on with the request yourself.")
            }
            Ok(_) => {}
            Err(_) => return fail("Could not check your proposals this turn."),
        }
    }
    let (Some(name), Some(role), Some(instructions), Some(reason)) =
        (arg(args, "name"), arg(args, "role"), arg(args, "instructions"), arg(args, "reason"))
    else {
        return fail("Nothing was proposed: name, role, instructions, reason and evidence are required.");
    };
    if reason.chars().count() < 10 {
        return fail("Nothing was proposed: reason must tell the person in a sentence why a new bot does this better than you.");
    }
    let name = name.trim_start_matches('@');
    if name.chars().any(char::is_whitespace) {
        return fail("Nothing was proposed: name must be one word, e.g. Chef.");
    }
    let evidence = match check_evidence(state, space, &args["evidence"]).await {
        Ok(v) => v,
        Err(e) => return fail(format!("Nothing was proposed. {e}")),
    };
    let model = arg(args, "model").unwrap_or("auto").to_string();
    if let Err(e) = super::bot_actions::check_model(&model) {
        return fail(format!("Nothing was proposed. {e}"));
    }
    let tools = match resolve_tools(state, space, args.get("tools")).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    // Team: the one named, or the proposer's
    let team = match arg(args, "team") {
        Some(t) => {
            let by_id = state.db.get_team(t).await.ok().flatten();
            let team = match by_id {
                Some(team) => Some(team),
                None => state.db.list_teams(space).await.unwrap_or_default().into_iter().find(|x| x.name.eq_ignore_ascii_case(t)),
            };
            match team {
                Some(team) if team.space_id == space.as_str() => Some(team),
                Some(_) => return fail("That team is in another space. You can only hire into your own space."),
                None => return fail(format!("There is no team \"{t}\" in your space.")),
            }
        }
        None => state.db.team_of_kwant(space, &scope.kwant_id).await.ok().flatten(),
    };
    // Bot first: an existing bot that already does this
    let key = role_key(role);
    let bots = state.db.list_kwants(space).await.unwrap_or_default();
    let requested = args["requested_by_person"].as_bool() == Some(true);
    // A kind of bot the person turned down stays turned down in this space
    let denials = state.db.active_denials(space).await.unwrap_or_default();
    let pending = state.db.list_hires(space, Some("pending")).await.unwrap_or_default();
    if let Some(why) = clash(name, &key, &bots, &denials, &pending, requested) {
        return fail(why);
    }
    let mut denied = denials.iter().find(|d| d.name.eq_ignore_ascii_case(name) || similar_roles(&d.role_key, &key));

    // A team: the other bots, checked like the first, and the team they form
    let mut members = Vec::new();
    let mut names = vec![name.to_lowercase()];
    match args.get("members") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() + 1 > MAX_TEAM_SIZE {
                return fail(format!("Nothing was proposed: a team proposal holds at most {MAX_TEAM_SIZE} bots."));
            }
            for m in list {
                let (Some(mname), Some(mrole), Some(minstr)) = (arg(m, "name"), arg(m, "role"), arg(m, "instructions")) else {
                    return fail("Nothing was proposed: every member needs a name, role and instructions.");
                };
                let mname = mname.trim_start_matches('@');
                if mname.chars().any(char::is_whitespace) {
                    return fail(format!("Nothing was proposed: member name \"{mname}\" must be one word."));
                }
                if names.contains(&mname.to_lowercase()) {
                    return fail(format!("Nothing was proposed: two bots in the proposal are called {mname}; give each its own name."));
                }
                let mkey = role_key(mrole);
                if let Some(why) = clash(mname, &mkey, &bots, &denials, &pending, requested) {
                    return fail(format!("Nothing was proposed. {why}"));
                }
                denied = denied.or(denials.iter().find(|d| d.name.eq_ignore_ascii_case(mname) || similar_roles(&d.role_key, &mkey)));
                let mmodel = arg(m, "model").unwrap_or("auto").to_string();
                if let Err(e) = super::bot_actions::check_model(&mmodel) {
                    return fail(format!("Nothing was proposed ({mname}). {e}"));
                }
                let mtools = match resolve_tools(state, space, m.get("tools")).await {
                    Ok(t) => t,
                    Err(e) => return fail(e),
                };
                names.push(mname.to_lowercase());
                members.push(json!({"name": mname, "role": mrole, "instructions": minstr, "model": mmodel, "tools": mtools}));
            }
        }
        Some(_) => return fail("Nothing was proposed: members must be a list of bots, each with name, role and instructions."),
    }
    let new_team = match args.get("new_team") {
        None | Some(Value::Null) => None,
        Some(t) => {
            let Some(tname) = arg(t, "name") else {
                return fail("Nothing was proposed: new_team needs a name.");
            };
            if arg(args, "team").is_some() {
                return fail("Nothing was proposed: give either team (join one) or new_team (create one), not both.");
            }
            let teams = state.db.list_teams(space).await.unwrap_or_default();
            if let Some(x) = teams.iter().find(|x| x.name.eq_ignore_ascii_case(tname)) {
                return fail(format!("Nothing was proposed: there is already a team called {} in your space; join it with team.", x.name));
            }
            let lead = match arg(t, "lead").map(|l| l.trim_start_matches('@')) {
                Some(l) if names.contains(&l.to_lowercase()) => Some(l.to_string()),
                Some(l) => return fail(format!("Nothing was proposed: the lead {l} must be one of the bots in the proposal.")),
                None => None,
            };
            Some(json!({"name": tname, "lead": lead}))
        }
    };
    let team = if new_team.is_some() { None } else { team };

    let origin = scope.origin.as_ref();
    let new = NewHire {
        space_id: space.as_str().into(),
        kwant_id: scope.kwant_id.clone(),
        turn_id: scope.turn_id.clone(),
        channel_id: origin.and_then(|o| o.channel_id.clone()),
        direct_kwant_id: origin.and_then(|o| o.direct_kwant_id.clone()),
        thread_id: origin.and_then(|o| o.thread_id.clone()),
        name: name.into(),
        role: role.into(),
        instructions: instructions.into(),
        model,
        tools,
        team_id: team.as_ref().map(|t| t.id.clone()),
        reason: reason.into(),
        evidence,
        requested_by_person: requested,
        members,
        new_team,
    };
    let is_team = !new.members.is_empty() || new.new_team.is_some();
    let has_chat = origin.is_some_and(|o| o.channel_id.is_some() || o.direct_kwant_id.is_some() || o.thread_id.is_some());
    let sender = scope.kwant_name.clone();
    let mut card_meta = Value::Null;
    let recorded = state
        .db
        .record_hire(&new, |h| {
            if !has_chat {
                return None;
            }
            card_meta = json!({"kind": "hire_proposal", "hire_id": h.id, "status": "pending"});
            Some((
                NewMessage {
                    channel_id: h.channel_id.clone().filter(|_| h.thread_id.is_none()),
                    direct_kwant_id: h.direct_kwant_id.clone().filter(|_| h.thread_id.is_none()),
                    sender_role: "kwant".into(),
                    sender_id: sender,
                    text: card_text(h, ""),
                    reasoning_content: None,
                    tool_calls: None,
                    tokens_used: 0,
                },
                MessageExtra { thread_id: h.thread_id.clone(), meta: Some(card_meta.clone()) },
            ))
        })
        .await;
    let (hire, card) = match recorded {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "could not record a hire proposal");
            return fail("Nothing was proposed: it could not be saved.");
        }
    };
    if let Some(card) = card {
        if let Some(t) = &hire.thread_id {
            let _ = state.db.touch_thread(t).await;
        }
        let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
    }
    tracing::info!(hire_id = %hire.id, kwant_id = %hire.kwant_id, turn_id = ?hire.turn_id, role = %hire.role, "bot proposed a hire");

    // The space's policy may decide at once
    let policy = state.db.hire_policy(space).await.map(|p| p.policy).unwrap_or_else(|_| "ask".into());
    let mut note = None;
    if is_team {
        // Several bots at once always ask, on one card
        if policy == "auto" {
            note = Some("A team proposal always asks you, even with \"always accept\".".to_string());
        }
    } else if policy == "auto" && denied.is_none() {
        let used = state.db.auto_hires_today(space).await.unwrap_or(i64::MAX);
        if used < AUTO_HIRES_PER_DAY {
            return match accept(state, &hire, true).await {
                Ok(h) => (
                    format!(
                        "Hired {} ({}) at once: the person chose \"always accept\" for this space. They see a card with Undo. Tell \
                        them in one sentence, then carry on with the request yourself; {} can take this kind of work from now on.",
                        h.name, h.role, h.name
                    ),
                    false,
                ),
                Err((_, e)) => {
                    refresh(state, &hire.id, Some(&format!("couldn't be created at once: {e}"))).await;
                    (format!("The proposal is waiting for the person: {e}. Carry on with the request yourself."), false)
                }
            };
        }
        note = Some(format!("{AUTO_HIRES_PER_DAY} bots were already hired automatically in the last 24 hours, so this one asks you."));
    } else if denied.is_some() {
        note = Some("You turned this kind of bot down before; the bot says you asked for one now.".to_string());
    }
    if is_team && denied.is_some() {
        note = Some("You turned one of these kinds of bot down before; the bot says you asked for them now.".to_string());
    }
    refresh(state, &hire.id, note.as_deref()).await;
    (
        format!(
            "Proposed hiring {}. Nothing is created until the person chooses on the card (Deny, Accept once or Always accept). \
            Tell them in one sentence, then carry on with the request yourself.",
            who(&hire)
        ),
        false,
    )
}

// The card

/// "Chef (Recipe planner)", or for a team "a team of 3 (team Trading): Ana (Lead), Bo (Backend), Cy (QA)".
fn who(h: &Hire) -> String {
    let members = h.members_json();
    let mut list = vec![format!("{} ({})", h.name, h.role)];
    list.extend(members.iter().map(|m| format!("{} ({})", m["name"].as_str().unwrap_or(""), m["role"].as_str().unwrap_or(""))));
    let team = h.new_team_json().and_then(|t| t["name"].as_str().map(|n| format!(" (new team {n})"))).unwrap_or_default();
    if list.len() == 1 && team.is_empty() {
        list.remove(0)
    } else {
        format!("a team of {}{team}: {}", list.len(), list.join(", "))
    }
}

fn card_text(h: &Hire, proposer: &str) -> String {
    let who = who(h);
    let team = h.members.is_some() || h.new_team.is_some();
    match h.status.as_str() {
        "pending" if team => format!("Wants to hire {who}. Waiting for your choice: Deny, or Approve all."),
        "pending" => format!("Wants to hire {who}. Waiting for your choice: Deny, Accept once or Always accept."),
        "accepted" => format!("Hired {who}; you accepted. Undo removes it while it has no history."),
        "auto_accepted" => format!("Hired {who} at once, as you chose \"always accept\". Undo removes it."),
        "denied" => format!("Not hired: {who}. {proposer} won't propose this kind of bot again in this space."),
        _ => format!("Hire undone: {who} was removed."),
    }
}

/// The card's `meta` (docs/bot-actions.md → "Hiring").
pub async fn card_meta(state: &AppState, h: &Hire, note: Option<&str>) -> Value {
    let space = SpaceId::new(&h.space_id);
    let proposer = state.db.get_kwant(&h.kwant_id).await.ok().flatten().map(|k| k.name).unwrap_or_default();
    let team = match &h.team_id {
        Some(t) => state.db.get_team(t).await.ok().flatten().map(|t| json!({"id": t.id, "name": t.name})),
        None => None,
    }
    .unwrap_or(Value::Null);
    let servers = state.db.list_mcp_servers(&space).await.unwrap_or_default();
    let tools: Vec<Value> = h
        .tools_json()
        .iter()
        .map(|id| json!({"id": id, "name": servers.iter().find(|s| &s.id == id).map(|s| s.name.clone())}))
        .collect();
    let action_applied = match &h.action_id {
        Some(a) => state.db.get_bot_action(a).await.ok().flatten().is_some_and(|a| a.status == "applied"),
        None => false,
    };
    let policy = state.db.hire_policy(&space).await.map(|p| p.policy).unwrap_or_else(|_| "ask".into());
    let tool_view = |ids: &Value| -> Vec<Value> {
        ids.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|id| json!({"id": id, "name": servers.iter().find(|s| s.id == id).map(|s| s.name.clone())}))
            .collect()
    };
    let is_team = h.members.is_some() || h.new_team.is_some();
    let mut members = vec![json!({"name": h.name, "role": h.role, "model": h.model, "tools": tools, "instructions": h.instructions})];
    members.extend(h.members_json().iter().map(|m| {
        json!({"name": m["name"], "role": m["role"], "model": m["model"], "tools": tool_view(&m["tools"]), "instructions": m["instructions"]})
    }));
    let choices = match (h.status.as_str(), is_team) {
        ("pending", true) => json!(["deny", "accept_once"]),
        ("pending", false) => json!(["deny", "accept_once", "always_accept"]),
        _ => json!([]),
    };
    json!({
        "kind": "hire_proposal",
        "hire_id": h.id,
        "kwant_id": h.kwant_id,
        "proposer_name": proposer,
        "name": h.name,
        "role": h.role,
        "instructions": h.instructions,
        "model": h.model,
        "tools": tools,
        "team": team,
        "reason": h.reason,
        "evidence": h.evidence_json(),
        "requested_by_person": h.requested_by_person,
        "status": h.status,
        "auto": h.auto,
        "choices": choices,
        "members": members,
        "new_team": h.new_team_json().map(|t| json!({"name": t["name"], "lead": t["lead"]})),
        "hired_kwant_ids": h.hired_ids(),
        "hired_kwant_id": h.hired_kwant_id,
        "action_id": h.action_id,
        "can_undo": action_applied && matches!(h.status.as_str(), "accepted" | "auto_accepted"),
        "policy": policy,
        "note": note,
        "created_at": h.created_at,
        "resolved_at": h.resolved_at,
    })
}

async fn hire_json(state: &AppState, h: &Hire, note: Option<&str>) -> Value {
    let mut v = serde_json::to_value(h).unwrap_or(Value::Null);
    v["card"] = card_meta(state, h, note).await;
    v
}

/// Re-reads the proposal, updates its card in place and tells clients.
async fn refresh(state: &AppState, id: &str, note: Option<&str>) -> Option<Value> {
    let h = state.db.get_hire(id).await.ok().flatten()?;
    let v = hire_json(state, &h, note).await;
    if let Some(mid) = &h.message_id {
        let _ = state.db.set_message_meta(mid, &v["card"]).await;
        let proposer = state.db.get_kwant(&h.kwant_id).await.ok().flatten().map(|k| k.name).unwrap_or_else(|| "The bot".into());
        let _ = state.db.set_message_text(mid, &card_text(&h, &proposer)).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::HireUpdated { hire: v.clone() });
    Some(v)
}

/// After the person undid a `bot_hire` action: the proposal reads "undone".
pub(super) async fn undone(state: &AppState, id: &str) {
    for from in ["accepted", "auto_accepted"] {
        if matches!(state.db.resolve_hire(id, from, "undone").await, Ok(true)) {
            break;
        }
    }
    refresh(state, id, None).await;
}

/// Creates the bot for a pending proposal: same path as `bot_create`, plus its tools and team,
/// logged as a `bot_hire` action for Undo. Puts the proposal back to pending on failure.
async fn accept(state: &AppState, h: &Hire, auto: bool) -> Result<Hire, (StatusCode, String)> {
    let to = if auto { "auto_accepted" } else { "accepted" };
    match state.db.resolve_hire(&h.id, "pending", to).await {
        Ok(true) => {}
        Ok(false) => return Err((StatusCode::CONFLICT, "this proposal was already decided".into())),
        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, "could not save the decision".into())),
    }
    let space = SpaceId::new(&h.space_id);
    let back = |e: String| async move {
        let _ = state.db.resolve_hire(&h.id, to, "pending").await;
        Err((StatusCode::CONFLICT, e))
    };
    if super::spaces::ensure_active(state, &space).await.is_err() {
        return back("the space is archived".into()).await;
    }
    // Every bot of the proposal (the first, then a team's others); all or none
    let installed: Vec<String> = state.db.list_mcp_servers(&space).await.unwrap_or_default().into_iter().map(|s| s.id).collect();
    let first = json!({"name": h.name, "role": h.role, "instructions": h.instructions, "model": h.model, "tools": h.tools_json()});
    let mut ids: Vec<String> = Vec::new();
    for m in std::iter::once(first).chain(h.members_json()) {
        let spec = json!({"name": m["name"], "title": m["role"], "instructions": m["instructions"], "model": m["model"]});
        let view = match super::bot_actions::create_bot(state, &space, &spec).await {
            Ok(v) => v,
            Err(e) => {
                let _ = remove_made(state, &ids, None).await;
                return back(format!("{} could not be created: {e}", m["name"].as_str().unwrap_or("a bot"))).await;
            }
        };
        let id = view["id"].as_str().unwrap_or("").to_string();
        let tools: Vec<String> = m["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|t| installed.iter().any(|i| i == t))
            .map(str::to_string)
            .collect();
        if !tools.is_empty() {
            let _ = state.db.set_kwant_mcp_servers(&id, &tools).await;
        }
        ids.push(id);
    }
    let id = ids[0].clone();
    // Their team: a new one (with its channel), or the one they join
    let mut made_team = None;
    if let Some(t) = h.new_team_json() {
        let names: Vec<String> =
            std::iter::once(h.name.clone()).chain(h.members_json().iter().map(|m| m["name"].as_str().unwrap_or("").to_string())).collect();
        let lead = t["lead"].as_str().and_then(|l| names.iter().zip(&ids).find(|(n, _)| n.eq_ignore_ascii_case(l)).map(|(_, i)| i.clone()));
        let new_team = crate::db::models::NewTeam {
            name: t["name"].as_str().unwrap_or("").to_string(),
            description: Some(format!("Hired together: {}", who(h))),
            icon: None,
            lead_kwant_id: lead.unwrap_or_else(|| id.clone()),
            member_kwants: ids.clone(),
            workspace_path: None,
            repo_url: None,
        };
        match state.db.create_team(&space, &new_team).await {
            Ok(team) => {
                let channel = state.db.list_channels(&space).await.unwrap_or_default().into_iter().find(|c| c.name == format!("team-{}", team.name));
                made_team = Some(json!({"id": team.id, "name": team.name, "channel_id": channel.map(|c| c.id)}));
            }
            Err(e) => {
                let _ = remove_made(state, &ids, None).await;
                return back(format!("the team could not be created: {e}")).await;
            }
        }
    } else if let Some(t) = &h.team_id {
        for i in &ids {
            let _ = state.db.add_team_member(t, i).await;
        }
    }
    if ids.len() > 1 || made_team.is_some() {
        let _ = state.db.set_hired_kwant_ids(&h.id, &ids).await;
    }
    let mut after = super::bot_actions::bot_snapshot(state, &id).await;
    after["hire_id"] = json!(h.id);
    after["team_id"] = json!(h.team_id);
    after["hired_kwant_ids"] = json!(ids);
    after["made_team"] = made_team.unwrap_or(Value::Null);
    let new = NewBotAction {
        space_id: h.space_id.clone(),
        kwant_id: h.kwant_id.clone(),
        turn_id: h.turn_id.clone(),
        channel_id: h.channel_id.clone(),
        direct_kwant_id: h.direct_kwant_id.clone(),
        thread_id: h.thread_id.clone(),
        action: "bot_hire".into(),
        target_kind: "bot".into(),
        target_id: Some(id.clone()),
        summary: format!("hired {}", who(h)),
        before: None,
        after: Some(after),
        status: "applied".into(),
    };
    let action = super::bot_actions::record_quiet(state, &new).await.ok();
    let _ = state.db.set_hire_result(&h.id, Some(&id), action.as_ref().map(|a| a.id.as_str())).await;
    if auto {
        let _ = state.db.mark_hire_auto(&h.id).await;
    }
    state.scheduler_wake.notify_one();
    refresh(state, &h.id, None).await;
    tracing::info!(hire_id = %h.id, kwant_id = %id, auto, "a hire was accepted");
    state.db.get_hire(&h.id).await.ok().flatten().ok_or((StatusCode::INTERNAL_SERVER_ERROR, "lost the proposal".into()))
}

/// Removes bots (and the team and channel) a hire made: a failed accept, or the person's Undo.
/// Refuses, removing nothing, when any of them already has history.
pub(super) async fn remove_made(state: &AppState, ids: &[String], made_team: Option<&Value>) -> Result<(), String> {
    for id in ids {
        match state.db.kwant_in_use(id).await {
            Ok(None) | Ok(Some("it leads a channel")) | Ok(Some("it leads a team")) if made_team.is_some() => {}
            Ok(None) => {}
            Ok(Some(why)) => {
                let name = state.db.get_kwant(id).await.ok().flatten().map(|k| k.name).unwrap_or_else(|| "a bot".into());
                return Err(format!("{name} can't be removed: {why}"));
            }
            Err(_) => return Err("could not check the bots".into()),
        }
    }
    if let Some(t) = made_team {
        if let Some(c) = t["channel_id"].as_str() {
            if !matches!(state.db.delete_empty_channel(c).await, Ok(true)) {
                return Err("the team's channel already has messages".into());
            }
        }
        if let Some(t) = t["id"].as_str() {
            let _ = state.db.delete_team(t).await;
        }
    }
    for id in ids {
        let _ = state.db.delete_unused_kwant(id).await;
    }
    state.scheduler_wake.notify_one();
    Ok(())
}

// What bots see

async fn status_text(state: &AppState, space: &SpaceId) -> String {
    let block = prompt_block(state, space).await;
    if block.is_empty() {
        "Hire policy: ask the person each time. No proposals in this space yet.".into()
    } else {
        block.trim().to_string()
    }
}

/// The space's hire policy and recent decisions, for every bot's next turn. Compact: at most 8
/// recent proposals plus the denials that still count.
pub async fn prompt_block(state: &AppState, space: &SpaceId) -> String {
    let policy = state.db.hire_policy(space).await.map(|p| p.policy).unwrap_or_else(|_| "ask".into());
    let recent = state.db.list_hires(space, None).await.unwrap_or_default();
    let denials = state.db.active_denials(space).await.unwrap_or_default();
    let policy_line = if policy == "auto" {
        let used = state.db.auto_hires_today(space).await.unwrap_or(0);
        format!(
            "Hire policy: always accept (the person chose it); {} of {AUTO_HIRES_PER_DAY} automatic hires left in the last 24 hours, \
            then proposals ask again.",
            (AUTO_HIRES_PER_DAY - used).max(0)
        )
    } else {
        "Hire policy: ask the person each time.".to_string()
    };
    let mut lines = Vec::new();
    let mut names = std::collections::HashMap::new();
    for h in recent.iter().filter(|h| h.status != "denied").take(8) {
        let proposer = match names.get(&h.kwant_id) {
            Some(n) => n,
            None => {
                let n = state.db.get_kwant(&h.kwant_id).await.ok().flatten().map(|k| k.name).unwrap_or_else(|| "a bot".into());
                names.entry(h.kwant_id.clone()).or_insert(n)
            }
        };
        let when = h.resolved_at.unwrap_or(h.created_at).format("%Y-%m-%d");
        let what = match h.status.as_str() {
            "pending" => "waiting for the person",
            "accepted" => "hired (the person accepted once)",
            "auto_accepted" => "hired automatically",
            _ => "hired, then undone by the person",
        };
        lines.push(format!("- {} ({}): {what}, {when}, proposed by {proposer}", h.name, h.role));
    }
    for d in denials.iter().take(10) {
        lines.push(format!(
            "- {} ({}): DENIED {}; don't propose this kind of bot again unless the person asks for one",
            d.name,
            d.role,
            d.resolved_at.map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default()
        ));
    }
    let mut out = format!(
        "\n\n[Hiring new bots in this space]\n{policy_line}\n\
        Do every request yourself first (with mcp_suggest for a missing tool). Use `propose_hire` only when a specialist with its \
        own instructions, tools, schedule or model would clearly do this kind of work better, or when similar requests have come \
        up at least {MIN_SIMILAR_REQUESTS} times here; give the reason and the evidence, then carry on with the request yourself. \
        When the person asks for a team, propose the whole team in one call: the first bot plus `members`, and `new_team` to \
        create their team and channel; they approve it on one card."
    );
    if !lines.is_empty() {
        out.push_str("\nProposals so far:\n");
        out.push_str(&lines.join("\n"));
    }
    out
}

// Routes (the person's)

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

/// `GET /api/hires?space=&status=`: the space's proposals with their cards, newest first.
pub async fn list(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ListQuery>) -> Response {
    let status = q.status.filter(|s| !s.is_empty());
    if status.as_deref().is_some_and(|s| !HIRE_STATUSES.contains(&s)) {
        return error(StatusCode::BAD_REQUEST, "unknown status");
    }
    match state.db.list_hires(&space, status.as_deref()).await {
        Ok(list) => {
            let mut out = Vec::new();
            for h in &list {
                out.push(hire_json(&state, h, None).await);
            }
            Json(out).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn load_pending(state: &AppState, headers: &HeaderMap, id: &str) -> Result<Hire, Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, "only the person decides on hiring a bot").await?;
    match state.db.get_hire(id).await {
        Ok(Some(h)) if h.status == "pending" => Ok(h),
        Ok(Some(h)) => Err(error(StatusCode::CONFLICT, format!("this proposal is {}", h.status))),
        Ok(None) => Err(error(StatusCode::NOT_FOUND, "proposal not found")),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

#[derive(Deserialize, Default)]
pub struct AcceptBody {
    /// "Always accept": also set the space's policy to `auto`.
    #[serde(default)]
    pub always: bool,
}

/// `POST /api/hires/:id/accept {always?}`: Accept once, or Always accept.
pub async fn accept_route(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Option<Json<AcceptBody>>) -> Response {
    let h = match load_pending(&state, &headers, &id).await {
        Ok(h) => h,
        Err(r) => return r,
    };
    let always = body.map(|b| b.always).unwrap_or(false);
    match accept(&state, &h, false).await {
        Ok(h) => {
            if always {
                let _ = state.db.set_hire_policy(&SpaceId::new(&h.space_id), "auto", false).await;
            }
            match refresh(&state, &h.id, None).await {
                Some(v) => Json(v).into_response(),
                None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err((status, e)) => {
            refresh(&state, &h.id, Some(&e)).await;
            error(status, e)
        }
    }
}

/// `POST /api/hires/:id/deny`: nothing is created, and the space remembers it.
pub async fn deny_route(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let h = match load_pending(&state, &headers, &id).await {
        Ok(h) => h,
        Err(r) => return r,
    };
    match state.db.resolve_hire(&h.id, "pending", "denied").await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::CONFLICT, "this proposal was just decided"),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    tracing::info!(hire_id = %h.id, "the person denied a hire");
    match refresh(&state, &h.id, None).await {
        Some(v) => Json(v).into_response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn policy_json(state: &AppState, space: &SpaceId) -> Result<Value, Response> {
    let p = state.db.hire_policy(space).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let used = state.db.auto_hires_today(space).await.unwrap_or(0);
    let denied: Vec<Value> = state
        .db
        .active_denials(space)
        .await
        .unwrap_or_default()
        .iter()
        .map(|d| json!({"hire_id": d.id, "name": d.name, "role": d.role, "denied_at": d.resolved_at}))
        .collect();
    Ok(json!({
        "space_id": space.as_str(),
        "policy": p.policy,
        "auto_hires_last_24h": used,
        "auto_hires_per_day": AUTO_HIRES_PER_DAY,
        "denied": denied,
        "denials_cleared_at": p.denials_cleared_at,
        "updated_at": p.updated_at,
    }))
}

async fn existing_space(state: &AppState, id: &str) -> Result<SpaceId, Response> {
    let space = SpaceId::new(id);
    match state.db.get_space(&space).await {
        Ok(Some(_)) => Ok(space),
        Ok(None) => Err(error(StatusCode::NOT_FOUND, "space not found")),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    }
}

/// `GET /api/spaces/:id/hire-policy`
pub async fn get_policy(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let space = match existing_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match policy_json(&state, &space).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
pub struct PolicyBody {
    pub policy: Option<String>,
    #[serde(default)]
    pub clear_denials: bool,
}

/// `PUT /api/spaces/:id/hire-policy {policy?: "ask"|"auto", clear_denials?}`: the person only.
pub async fn put_policy(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(body): Json<PolicyBody>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person sets the hire policy").await {
        return r;
    }
    let space = match existing_space(&state, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let current = match state.db.hire_policy(&space).await {
        Ok(p) => p.policy,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let policy = body.policy.unwrap_or(current);
    if !matches!(policy.as_str(), "ask" | "auto") {
        return error(StatusCode::BAD_REQUEST, "policy must be \"ask\" or \"auto\"");
    }
    if state.db.set_hire_policy(&space, &policy, body.clear_denials).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    tracing::info!(space = %space, policy = %policy, clear_denials = body.clear_denials, "the person set the hire policy");
    match policy_json(&state, &space).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}
