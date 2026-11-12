# Connections: API

One list of everything kwant's bots reach outside the app, with its status, last check and the
bots that use it:

- MCP servers
- engine accounts
- website logins saved in the vault
- the mail (IMAP) and GitHub sources of Loose ends

Composio is offered as an **optional** MCP connector. It is off by default, and it is labelled
everywhere as storing the person's app sign-ins on Composio's servers. Every other login stays in
kwant's own vault or database.

Both routes are **person-only**, because the list names saved logins:

- A bot's turn token gets `403`.
- From another machine, the API token is required (`401` without it).

## `GET /api/connections?space=`

```json
{
  "connections": [ <Connection>, … ],
  "counts": { "connected": 4, "error": 1, "off": 1 },
  "vault": { "status": "ok", "logins": 3 },
  "catalog": [ <CatalogEntry> ]
}
```

`connections` is ordered by kind (MCP servers, accounts, logins, then Loose ends sources) in the
order each list comes from. Sort or group it on the client.

### Connection

```json
{
  "id": "mcp:7c1e…",
  "kind": "mcp",
  "name": "github",
  "detail": "api.githubcopilot.com",
  "status": "needs_sign_in",
  "status_detail": "Sign-in required",
  "last_check": "2026-10-22T10:04:11Z",
  "used_by": [ { "id": "k1", "name": "Ada" } ],
  "logins_stored": "kwnt",
  "manage": "/api/mcp/servers/7c1e…"
}
```

| field | meaning |
|---|---|
| `id` | `mcp:<server id>`, `account:<profile id>`, `login:<owner id>:<credential id>`, `source:<source id>`. Stable across calls |
| `kind` | `mcp`, `account`, `browser_login`, `email` or `github` |
| `name` | the server or account name, the login's host, or the source label |
| `detail` | what it points at. It is never a secret. See the per-kind list below |
| `status` | `connected`, `needs_sign_in`, `limited`, `error`, `off`, `locked` or `unknown` |
| `status_detail` | optional readable reason ("Out of quota until …", the last error) |
| `last_check` | RFC 3339, or `null` when never checked |
| `used_by` | the bots that use it, sorted by name. Empty while it is off |
| `logins_stored` | `vault` (kwant's password vault or Bitwarden), `kwnt` (kwant's database or the engine CLI's own login folder; the value keeps the binary's name and is not shown as is), or `composio` (Composio's servers) |
| `manage` | the existing API for changing it (link the UI's edit screen from the kind) |

What `detail` holds for each kind:

| kind | `detail` |
|---|---|
| MCP over HTTP | the host |
| MCP over stdio | the command only. Its arguments can carry tokens, so they are left out |
| account | the engine, plus the account label when known. For a Custom API account, the server host |
| browser_login | `username @ origin · saved for <bot / team … / channel …>` |
| email | `username @ imap host` |
| github | the API host |

### Status rules

**MCP server**

| condition | status |
|---|---|
| switched off | `off` |
| last test passed | `connected` |
| last test needed a sign-in | `needs_sign_in` |
| last test failed | `error`, with the error |
| not tested since kwant started, but signed in with OAuth | `connected` |
| not tested since kwant started | `unknown` |

`last_check` is the last `POST /api/mcp/servers/:id/test`. It is kept **in memory**, so it is
empty again after a restart. Call the test route to refresh it ("Check now").

**Account** (engine profile)

| condition | status |
|---|---|
| switched off | `off` |
| out of quota | `limited` |
| Custom API account, by its last check | `ready` → `connected`, `key_rejected` → `needs_sign_in`, `unreachable` or `error` → `error` |
| last turn refused for auth, or not signed in | `needs_sign_in` |
| signed in | `connected` |
| anything else | `unknown` |

`last_check` is the Custom API check time. Other accounts have none.

**Saved login**

- Always `connected`. kwant doesn't try a saved password on its own.
- `last_check` is the time it was last filled into a page.

**Loose ends source**

| condition | status |
|---|---|
| switched off | `off` |
| last scan ok | `connected` |
| last scan failed | `error`, with the error |
| never scanned | `unknown` |

`last_check` is the last scan.

### Who uses it (`used_by`)

| kind | bots |
|---|---|
| MCP server | every bot in the space, minus bots switched off for that server. Nobody while the server is off |
| account | bots whose turns on its engine start on it: a bot bound to it, or a bot with no binding (or bound to another engine's account) while it is the engine's default. Bots on "auto" count for every engine, pinned bots only for theirs. Nobody while it is off |
| browser_login | the bot it was saved for, or the lead and members of the team or channel it was saved for (they share that browser) |
| email, github | the space's Loose ends bot, when Loose ends ownership is on |

### `vault`

| `status` | meaning |
|---|---|
| `ok` | the logins are listed. `logins` is how many there are |
| `locked` | Bitwarden is locked. Logins are **not** listed. Show "Unlock Bitwarden to list saved logins" and link the Passwords settings |
| `unavailable` | the store couldn't be read. Logins are not listed |

### Catalog entry

```json
{
  "id": "composio",
  "kind": "mcp",
  "name": "Composio",
  "description": "Gmail, Slack, Notion, Linear and hundreds more apps, with your free Composio API key. Your bots find the tools they need themselves.",
  "optional": true,
  "added": true,
  "connection_id": "mcp:…",
  "enabled": false,
  "logins_stored": "composio",
  "warning": "Composio keeps your app sign-ins on Composio's servers, not in kwant's vault, and your bots' tool calls through it pass through Composio.",
  "add": "/api/connections/composio",
  "key_url": "https://dashboard.composio.dev/~/project/settings/api-keys",
  "mode": "session",
  "user_id": "kwant-spc_default",
  "apps_connected": [{ "slug": "gmail", "name": "Gmail" }],
  "problem": null,
  "can_search": true,
  "can_connect": true,
  "can_switch": false,
  "search": "/api/connections/composio/apps",
  "switch": "/api/connections/composio/switch",
  "replace_key": "/api/connections/composio/key"
}
```

Show `warning` next to the add button and on the connection once it is added. Once added,
`connection_id` points at the MCP connection.

| field | meaning |
|---|---|
| `mode` | `null` before Composio is added. `session`: the one-key setup, where kwant made the Composio session. `url`: an MCP URL pasted from composio.dev (older setups) |
| `user_id` | the Composio user whose apps the bots use: the session's, else the `user_id` in the URL. In the one-key setup it is `kwant-<space id>` |
| `apps_connected` | apps signed in on Composio for that user. One Composio call, cached for a minute per space (an app search or status check refreshes it). Names come from catalogue searches, else the slug in words. Empty without a key or user, or if Composio couldn't be asked |
| `problem` | `null`, or `{message, action}` when something needs the person: a missing or refused key (`replace_key`), Composio down (`try_again`), a URL setup without a `user_id` (`switch`) |
| `can_search` | Composio is added with an API key |
| `can_connect` | also knows the user (always in the one-key setup) |
| `can_switch` | a `url` setup: offer Switch to the one-key setup |
| `key_url` | where the person copies their project API key (signed out it goes through Composio's sign-in) |

## `POST /api/connections/composio?space=`

The one-key setup, which the app uses:

```json
{ "api_key": "ak_…", "enabled": false }
```

kwant checks the key by creating a Composio Tool Router session for the space's own Composio user
(`POST /api/v3.1/tool_router/session` with `{"user_id": "kwant-<space id>", "manage_connections": {"enable": true}}`
and no toolkit list, so every app is available and the bots find tools themselves). It saves the
session's MCP address as the MCP server `composio` with the key as its `x-api-key` header, and
keeps `{session_id, user_id, mcp_url}` in settings under `composio_session:<space id>`. The setup
counts as `session` only while the server's URL is still that `mcp_url`: a server edited or
re-added by hand is a `url` setup again. If Composio refuses the key, **nothing is saved**.

The older body still works and makes a `url` setup without calling Composio:

```json
{ "url": "https://backend.composio.dev/v3/mcp/<id>/mcp?user_id=…", "api_key": "…", "enabled": false }
```

| field | rules |
|---|---|
| `api_key` | required without `url`. Trimmed. Sent to Composio as the `x-api-key` header, and stored and masked like any MCP header |
| `url` | optional. Must be `https` on `composio.dev` or one of its subdomains |
| `enabled` | optional, default **false**. Only an enabled server reaches bots. Turn it on later with `PATCH /api/mcp/servers/:id` |

Result: `{ "connection_id": "mcp:…", "server": McpServerView, "mode": "session" | "url", "user_id": "kwant-…" | null, "tools": ["COMPOSIO_SEARCH_TOOLS", …], "warning": "…" }`.
`McpServerView` masks header values. `tools` are the session's meta tools (empty for `url`).

## Errors from the Composio routes

Every error is `{ "error": "<words for the person>", "action": "…", "key_url"?: "…" }`. `action`
is the one button that fixes it:

| `action` | button | when |
|---|---|---|
| `add_composio` | Add Composio | Composio isn't added yet, or the add body has no key |
| `replace_key` | Replace key | no key saved, Composio refused the key (`409`), or Composio is already added (`409` on add) |
| `try_again` | Try again | Composio couldn't be reached or failed (`502`) |
| `switch` | Switch to one key | a `url` setup without `user_id` can't connect apps (`409`) |

`add_composio` and `replace_key` errors carry `key_url`. The texts:

- refused key (401): "Composio didn't accept this key. Copy the API key from your Composio dashboard, under Project settings › API keys. It starts with ak_."
- a user key (`uak_…`) refused: "That's a Composio user key (it starts with uak_). kwant needs the project API key: …"
- a scoped key without the permission (403): "This Composio key isn't allowed to do this. Copy a full-access API key from your Composio dashboard, under Project settings › API keys."
- down, timed out or 5xx: "Composio couldn't be reached; try again in a minute."

`400` is a bad body, query or app name; `403` / `401` is not the person.

## `POST /api/connections/composio/switch?space=`

Body optional: `{ "api_key": "ak_…" }` (else the saved key). Turns a `url` setup into the one-key
setup: a session for the URL's `user_id` (so apps connected before stay connected), else for
`kwant-<space id>`. The same MCP server is updated in place, so its id, its on/off switch and the
per-bot opt-outs don't change. A refused key changes nothing. Idempotent: a `session` setup answers
`switched: false` without calling Composio.

Result: `{ "connection_id", "server", "mode": "session", "user_id", "tools"?, "switched": bool }`.

## `PUT /api/connections/composio/key?space=`

Body `{ "api_key": "ak_…" }`. The new key is checked before anything is saved. In the one-key setup
kwant reads the saved session with the new key: a key of the same Composio project keeps the
session; a key of another project (or a deleted session) gets a new session for the **same user**,
on the same MCP server. In a `url` setup the key is checked with one account call and only the
header changes.

Result: `{ "connection_id", "server", "mode", "user_id", "session_renewed": bool }`.

Composio appears as an ordinary MCP server in `/api/mcp/servers`, so switching it on or off, the
per-bot opt-outs, testing and removing all use those routes.

## Find an app on Composio

These routes are person-only, refuse bot tokens, and use the person's own Composio key from the
connector's `x-api-key` header. Until Composio is added with a key they answer `409` **without
contacting Composio**. The key is never echoed or logged. `KWNT_COMPOSIO_API_URL` points them at
another API address (tests).

### `GET /api/connections/composio/apps?space=` (no `q`): the whole catalogue

For autocomplete: the UI loads this once and matches each keystroke in the browser, with no call
per key. kwant pages Composio's `GET /api/v3.1/toolkits?sort_by=usage&limit=500[&cursor=…]`
(at most 20 pages, duplicates dropped) and keeps the list **24 hours per key** in memory. A list
older than that is still served at once while one background fetch replaces it. The `connected`
flags come from the one-minute connected-apps cache, so a listing costs at most one
`connected_accounts` call.

```json
{ "apps": [{ "slug": "gmail", "name": "Gmail", "logo": "https://…", "tools_count": 23, "connected": true }],
  "count": 1, "fetched_at": "2026-11-12T22:30:00Z", "connected_checked": true,
  "mode": "session", "can_connect": true, "warning": "…" }
```

Rows are compact: no description; `logo` (https only) and `tools_count` are left out when
Composio has none. The order is Composio's usage order. The reply carries `ETag` and
`Cache-Control: private, no-cache`; send it back as `If-None-Match` (weak `W/` tags and lists are
accepted) and an unchanged list answers `304` with no body. The tag changes when the list is
refetched, the connected apps change, or the setup's mode or user changes. Composio needs a key
even to list apps (`401` without one), so before setup the UI uses its own built-in tiles. The
server does not gzip; a reverse proxy can.

### `GET /api/connections/composio/apps?q=&space=`

`q` is 1–60 characters. kwant calls Composio's `GET /api/v3.1/toolkits?search=…&sort_by=usage&limit=20`,
then **one** `GET /api/v3.1/connected_accounts?user_ids=<user>&statuses=ACTIVE&limit=200` for the
whole page.

```json
{ "apps": [{ "slug": "gmail", "name": "Gmail", "description": "…", "logo": "https://…", "tools_count": 23, "no_auth": false, "connected": true }],
  "connected_checked": true, "mode": "session", "can_connect": true, "warning": "…" }
```

Apps without a plain slug are left out; `logo` is kept only when it is `https`. If the account call
fails, the search still answers with every `connected: false` and `connected_checked: false`.

### `POST /api/connections/composio/apps/:slug/connect?space=`

In the one-key setup: a sign-in link from the saved session
(`POST /api/v3.1/tool_router/session/:id/link`). In a `url` setup: a new session for the URL's
`user_id`, then the link. Result: `{ "redirect_url": "https://…", "warning": "…" }`. The person
finishes signing in on Composio's site. A Composio MCP server set up with a fixed list of apps may
need the new app added to it on composio.dev before its tools reach bots; the one-key session has
every app.

### `GET /api/connections/composio/apps/:slug/status?space=`

`{ "connected": bool }` for the setup's user, and it updates `apps_connected`.

## Known limits

- MCP `last_check` is kept in memory only (see the MCP status rules).
- Loose ends sources are global in kwant rather than per space, so every space lists them, and
  `used_by` names the current space's Loose ends bot.
- Engine CLI logins (`~/.claude`, Codex, Grok) live in each CLI's own folder, reported as
  `logins_stored: "kwnt"`. kwant doesn't copy them into the vault.
