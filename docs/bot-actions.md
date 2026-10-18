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
- Without `tz`, the `timezone` setting is used. If that isn't set either, the tool returns an error asking the bot to pass `tz` or ask the person.
- Schedule and validation errors come back as tool errors (`isError: true`), and nothing changes.
- A routine runs as its bot and delivers into its channel, or into that bot's direct chat. Created from a thread, it delivers to the thread's channel or direct chat, not into the thread.

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
