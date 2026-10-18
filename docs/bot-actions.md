# Bots changing the app (bot actions)

Agents with their own tools (`kwnt_tools`, the self MCP server) can set up routines and manage the
app when the person asks, in the same turn. Every tool wraps the HTTP handler the app uses, scoped
to the turn's bot and its space. Code: `src/server/bot_actions.rs`, `src/db/bot_actions.rs`.

## Tools

| Tool | Arguments | Effect |
|---|---|---|
| `routine_list` | – | Routines in the bot's space |
| `routine_create` | `name`, `instructions`, `schedule`, `bot?`, `channel?` | Applied, undo deletes it |
| `routine_update` | `routine`, `name?`, `instructions?`, `schedule?`, `bot?`, `channel?` | Applied, undo restores |
| `routine_pause` / `routine_resume` | `routine` | Applied, undo restores |
| `routine_run_now` | `routine` | Logged with a card, no undo |
| `routine_delete` | `routine` | Applied, undo recreates it (new id; the action is retargeted) |
| `follow_up_create` | `at` (RFC 3339 with offset), `check`, `name?` | A one-off check-in by the calling bot in this chat or thread. Applied as `routine_create`, so undo deletes it |
| `bot_list` | – | Bots, models, accounts, installed tool servers |
| `bot_create` | `name`, `title`, `instructions`, `soul?`, `model?`, `profile_id?`, `avatar?` | Applied, undo deletes it if it has no history yet |
| `bot_update` | `bot`, `name?`, `title?`, `instructions?`, `soul?`, `model?`, `profile_id?`, `avatar?`, `mcp_servers?` | Applied, undo restores fields, account and tool servers |
| `bot_delete` | `bot`, `reason?` | **Pending** until the person confirms; only bots with no history |
| `settings_get` | – | `timezone`, `max_turns_per_agent`, `max_turns_global`, `delegation_follow_up_minutes`, `notify_on` |
| `settings_update` | `settings: {…}` (those keys only) | Applied, undo restores |
| `space_get` / `space_list` | – | |
| `space_create` | `name`, `icon?`, `color?` | Applied, undo archives it |
| `space_update` | `name?`, `icon?`, `color?` (own space) | Applied, undo restores |
| `space_archive` | `reason?` | **Pending** |
| `channel_list` | – | |
| `channel_create` | `name`, `description?`, `lead?`, `members?` | Applied, undo deletes it while it has no messages |
| `channel_update` | `channel`, `instructions` | Applied, undo restores |
| `profile_delete` | `profile_id`, `reason?` | **Pending** |

- `routine`, `bot` and `channel` take an id or a name. `bot` defaults to the calling bot.
- `channel` defaults to the chat the turn is in. `"direct"` means the bot's direct chat.
- `schedule` can be one of:
  - `{"type":"daily","time":"07:00","tz?"}`
  - `{"type":"weekly","time":"09:30","weekdays":["mon"],"tz?"}`
  - `{"type":"interval","every_minutes":60}`
  - `{"type":"once","at":"<RFC 3339>"}`
  - `{"type":"monthly","time":"09:00","days":[1, 15, "last"],"tz?"}`. A day past a month's end runs on that month's last day.
  - `{"type":"cron","expression":"0 9 * * MON#2","tz?"}`. Five fields (minute hour day-of-month month weekday), with lists, ranges, steps, names, `L` (last day of month), `MON#2` and `5L`. When both day fields are set, either one matching is enough, as in classic cron. Not supported: seconds, years, `@daily`-style macros, or rules that fire more than once every 5 minutes. A rule that never fires (`0 9 30 2 *`) is refused.
- Local times follow the zone across DST. A time that falls in the spring-forward gap runs at the first valid moment after it, and an ambiguous autumn time runs once, at its first occurrence.
- Without `tz`, the `timezone` setting is used. If that isn't set either, the tool returns an error asking the bot to pass `tz` or ask the person.
- Schedule and validation errors come back as tool errors (`isError: true`), and nothing changes.
- A routine runs as its bot and delivers into its channel, its thread (`thread_id`, one of that bot's own threads), or that bot's direct chat. Created by a bot for itself from a thread, with no `channel` given, it delivers into that thread.
- Follow-ups: a `once` routine a bot makes for itself (`follow_up_create`, or `routine_create` with a once schedule and no `channel`) stores `context_message_id`, the person's latest message in that chat. When it runs, if that message is no longer in the 20 messages the turn sees anyway, the prompt gets up to 6 messages around it (600 characters each), from the same chat only. The API refuses a `context_message_id` from another chat, and a `thread_id` that belongs to another bot.
- `GET /api/routines` items also carry `schedule_text` (for example "monthly on day 1 at 09:00 Europe/Berlin"), `thread_id` and `context_message_id`.

New setting: `GET/POST /api/settings` has `timezone` (IANA name; `""` means not set and clears it). Legacy names are canonicalised (Europe/Kiev → Europe/Kyiv).

## Limits and scope

- Each turn can make at most **10 changes**. Read-only tools don't count. Without a turn id, the limit is 10 changes per bot per 10 minutes.
- Only the bot's own space is reachable. A target in another space gets: "That … is in another space … ask the person to switch to that space and do it there."
- An archived space refuses changes.
- Each change is logged in the `bot_actions` table with bot, turn, chat, before and after, and also written to `tracing::info!("bot changed the app")`.

## Refused (the tool says where the person does it)

| Category | Where the person goes |
|---|---|
| API keys, tokens, passwords, webhook URL (any `*key*`, `*token*`, `*secret*`, `*webhook*` setting) | Space settings → Accounts / App settings → Notifications |
| Engines, sign-in and sign-out, Ollama URL, profiles | Space settings → Accounts / App settings |
| How people sign in to kwnt (auth) | The host computer |
| Approvals and questions | The card in the chat (there is no tool for these) |
| Computer, sandbox or VM | App settings → Computer for browsing and commands |
| `agent_tools` | App settings, person only |
| Installing MCP servers, and MCP env or header values | `mcp_suggest` card; Space settings → Tools |

`bot_update.mcp_servers` only assigns servers that are already installed in the space. Passing `env`, `headers`, `api_key`, `password` or `token` to `bot_create`/`bot_update` is refused.

## The card

Each action posts a message from the bot (`sender_role: "kwant"`, `sender_id: <bot name>`) into the chat of the turn: the channel, the direct chat or the thread. Example text: `Done: Created routine 'Morning motivation', daily 07:00 Europe/Berlin, delivering to #general.` A pending action reads `Needs your OK: …`.

`message.meta`:

```json
{
  "kind": "bot_action",
  "action_id": "…",
  "action": "routine_create",
  "kwant_id": "…",
  "target_kind": "routine",
  "target_id": "…",
  "summary": "created routine 'Morning motivation', daily 07:00 Europe/Berlin, delivering to #general",
  "details": {
    "routine_id": "…", "name": "Morning motivation", "kwant_id": "…", "bot_name": "Coach",
    "schedule": {"type": "daily", "time": "07:00", "tz": "Europe/Berlin"},
    "schedule_text": "daily 07:00 Europe/Berlin", "enabled": true,
    "next_run_at": "…", "channel_id": "…", "delivers_to": "#general"
  },
  "status": "applied",
  "can_undo": true,
  "needs_confirmation": false,
  "created_at": "…",
  "resolved_at": null
}
```

- `status` is one of `applied | undone | pending | confirmed | declined`.
- `target_kind` is one of `routine | bot | settings | space | channel | profile`.
- `details` depends on the target:
  - routine: as above, plus `run_id` and `run_status` for `routine_run_now`;
  - bot: `kwant_id, name, title, instructions, soul, avatar, model, profile_id, profile_failover, mcp_server_ids`;
  - settings: only the changed keys;
  - space: `space_id, name, icon, color, archived`;
  - channel: `channel_id, name, instructions` or `lead_kwant_id, member_kwants`.
- Pending actions also have `details.reason`.

Suggested buttons:
- **Undo** when `can_undo`.
- **Confirm** and **Decline** when `needs_confirmation`.
- For routines, **Edit** and **Pause** can open the routine in Space settings → Routines (`target_id`), or call `PUT /api/routines/:id` directly. Those edits are the person's own and are not bot actions.

## Routes (person only)

- `GET /api/bot-actions?space=&kwant_id=&status=`: the space's actions, newest first, at most 200. Each is a `BotAction` plus `card` (the meta above).
- `POST /api/bot-actions/:id/undo`: returns 200 with the action.
  - 409 if it isn't applied, if a newer applied change to the same target exists ("undo the newer change first: …"), or if it can't be put back safely (for example, a bot that already has history, or a channel with messages).
- `POST /api/bot-actions/:id/confirm` and `POST /api/bot-actions/:id/decline`: only for pending actions; 409 otherwise.
- All three return **403** for a turn token (self tools or MCP session) or any non-API bearer.
- After a change, the card's meta is updated and SSE sends `bot_action_updated { action }`. That event is routed to the space and the chat context (channel, direct chat or thread).

## Hiring

A bot does every request itself first, including suggesting a missing tool server with `mcp_suggest`. It may propose hiring a new bot only when:
- a specialist with its own instructions, tools, schedule or model would clearly do this kind of work better; or
- similar requests from the person have come up at least 3 times in this space.

The bot then carries on with the request itself. A proposal creates nothing on its own.

### Tools

- `propose_hire {name, role, instructions, model?, tools?, team?, reason, evidence, requested_by_person?}` records a pending proposal tied to the bot, turn and space, and posts a card in the turn's chat.
  - `evidence` is required and is checked by the server:
    - `{basis: "specialist", needs: [some of "instructions","tools","schedule","model"], gap: "<at least 20 chars>"}`; or
    - `{basis: "repeated", similar_requests: [message ids]}`, with at least 3 distinct messages from the person (`sender_role` user) in this space.
  - `tools` must be tool servers already installed in the space. `team` must be in the space; it defaults to the proposer's team.
  - `model` follows `bot_create`'s rules.
  - Refused:
    - more than 1 proposal per turn;
    - keys or accounts (`env`, `headers`, `api_key`, `password`, `token`, `mcp_env`, `profile_id`);
    - a name already used in the space;
    - a bot whose title is already a similar role;
    - a similar proposal already pending;
    - a role similar to one the person denied in this space (unless `requested_by_person`, which still always asks).
  - Roles are compared by their words, ignoring case, plurals and filler such as "bot" or "assistant". They are similar when more than half the words match.
- `hire_status {}` is read-only. It returns the policy, the automatic hires left, recent proposals and the denials.

### Decisions (person only; a turn token or non-API bearer gets 403)

- `POST /api/hires/:id/accept {}` (**Accept once**) creates the bot through the `bot_create` path, with title = role, its tools and no keys. It adds the bot to the team and the team's `team-<name>` channel, and logs a `bot_action` `bot_hire` that can be undone. The policy doesn't change, so the next proposal asks again.
- `POST /api/hires/:id/accept {"always": true}` (**Always accept**) does the same and sets the space's policy to `auto`. Later proposals in the space are then created at once (status `auto_accepted`) with an Undo card. The cap is 3 automatic hires per space in 24 hours; after that, proposals wait for the person again, with a `note` on the card.
- `POST /api/hires/:id/deny` creates nothing. It is remembered for the space, and the space's bots are told not to propose that kind of bot again unless the person asks.
- A proposal that was already decided returns 409.
- Undo goes through `POST /api/bot-actions/:action_id/undo`. It removes the hired bot and its team memberships while the bot has no history, and the proposal becomes `undone`.

### Routes

| Route | Who | Returns |
|---|---|---|
| `GET /api/hires?space=&status=` | anyone with the API | proposals newest first (≤200), each a `Hire` plus `card` |
| `POST /api/hires/:id/accept {always?}` | person | the `Hire` plus `card`; 409 when not pending or creation failed |
| `POST /api/hires/:id/deny` | person | the `Hire` plus `card`; 409 when not pending |
| `GET /api/spaces/:id/hire-policy` | anyone with the API (bots read it through `hire_status`) | `{space_id, policy: "ask"\|"auto", auto_hires_last_24h, auto_hires_per_day: 3, denied: [{hire_id,name,role,denied_at}], denials_cleared_at, updated_at}`; 404 for an unknown space |
| `PUT /api/spaces/:id/hire-policy {policy?, clear_denials?}` | person | same as GET; 400 for another policy value. `clear_denials: true` forgets the denials made so far |

`Hire` fields: `id, space_id, kwant_id` (the proposer), `turn_id, channel_id, direct_kwant_id, thread_id, name, role, role_key, instructions, model, tools` (server ids), `team_id, reason, evidence, requested_by_person, auto, status` (`pending|accepted|auto_accepted|denied|undone`), `hired_kwant_id, action_id, message_id, created_at, resolved_at`.

### The card

A message from the proposing bot carries `meta`:

```
{kind: "hire_proposal", hire_id, kwant_id, proposer_name, name, role, instructions, model,
 tools: [{id, name}], team: {id, name} | null, reason, evidence, requested_by_person,
 status, auto, choices: ["deny","accept_once","always_accept"] (pending only, else []),
 hired_kwant_id, action_id, can_undo, policy, note, created_at, resolved_at}
```

- Buttons:
  - **Deny** → `/deny`.
  - **Accept once** → `/accept {}`.
  - **Always accept** → `/accept {"always": true}`.
  - **Undo** when `can_undo` → `/api/bot-actions/{action_id}/undo`.
- The message text also changes with the status ("Wants to hire…", "Hired…", "Not hired…", "Hire undone…"), so bots reading the history see the decision.
- Each change sends SSE `hire_updated { hire }` (the `Hire` plus `card`), routed like `bot_action_updated`. A new card also sends `message_added`.

### What bots see

While a bot has its own tools, every turn includes a `[Hiring new bots in this space]` block. It contains:
- the bot-first rule;
- the policy, and the automatic hires left;
- up to 8 recent proposals with their status;
- every denial still in force.

Each space sees only its own block.
