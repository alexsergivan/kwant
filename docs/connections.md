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
  "description": "Hundreds of apps (Gmail, Slack, Notion, Linear…) through one MCP server you set up at composio.dev.",
  "optional": true,
  "added": false,
  "connection_id": null,
  "enabled": false,
  "logins_stored": "composio",
  "warning": "Composio keeps your app sign-ins on Composio's servers, not in kwant's vault, and your bots' tool calls through it pass through Composio.",
  "add": "/api/connections/composio",
  "can_search": false,
  "can_connect": false,
  "search": "/api/connections/composio/apps"
}
```

Show `warning` next to the add button and on the connection once it is added. Once added,
`connection_id` points at the MCP connection. `can_search` is true once Composio is added with an
API key (`x-api-key` header); `can_connect` also needs a `user_id` in its URL.

## `POST /api/connections/composio?space=`

```json
{ "url": "https://backend.composio.dev/v3/mcp/<id>/mcp?user_id=…", "api_key": "…", "enabled": false }
```

| field | rules |
|---|---|
| `url` | required. Must be `https` on `composio.dev` or one of its subdomains |
| `api_key` | optional. Sent to Composio as the `x-api-key` header, and stored and masked like any MCP header |
| `enabled` | optional, default **false**. Only an enabled server reaches bots. Turn it on later with `PATCH /api/mcp/servers/:id` |

Result: `{ "connection_id": "mcp:…", "server": McpServerView, "warning": "…" }`. `McpServerView`
masks header values.

Errors:

| status | when |
|---|---|
| `400` | the URL is not Composio's, or the server is invalid |
| `409` | Composio is already added (a server named `composio`, or one with a Composio URL). Change that server instead |
| `403` / `401` | not the person (see above) |

Composio appears as an ordinary MCP server in `/api/mcp/servers`, so switching it on or off, the
per-bot opt-outs, testing and removing all use those routes.

## Find an app on Composio

Both routes are person-only, refuse bot tokens, and use the person's own Composio key from the
connector's `x-api-key` header. Until Composio is added with a key they answer `409` **without
contacting Composio**; nothing else in kwant calls Composio's API. The key is never echoed.
`KWNT_COMPOSIO_API_URL` points them at another API address (tests).

### `GET /api/connections/composio/apps?q=&space=`

`q` is 1–60 characters. kwant calls Composio's `GET /api/v3.1/toolkits?search=…&sort_by=usage&limit=20`.

```json
{ "apps": [{ "slug": "gmail", "name": "Gmail", "description": "…", "logo": "https://…", "tools_count": 23, "no_auth": false }],
  "can_connect": true, "warning": "…" }
```

Apps without a plain slug are left out; `logo` is kept only when it is `https`.

### `POST /api/connections/composio/apps/:slug/connect?space=`

Creates a Composio session for the `user_id` in the connector's URL
(`POST /api/v3.1/tool_router/session`), then a sign-in link for the app
(`POST /api/v3.1/tool_router/session/:id/link`). Result: `{ "redirect_url": "https://…", "warning": "…" }`.
The person finishes signing in on Composio's site. A Composio MCP server set up with a fixed list
of apps may need the new app added to it on composio.dev before its tools reach bots.

| status | when |
|---|---|
| `400` | `q` is empty or too long, or `slug` isn't `[a-z0-9_-]` |
| `409` | Composio isn't added, has no API key, Composio refused the key, or (connect) the URL has no `user_id` |
| `502` | Composio couldn't be reached or answered with an error |
| `403` / `401` | not the person |

## Known limits

- MCP `last_check` is kept in memory only (see the MCP status rules).
- Loose ends sources are global in kwant rather than per space, so every space lists them, and
  `used_by` names the current space's Loose ends bot.
- Engine CLI logins (`~/.claude`, Codex, Grok) live in each CLI's own folder, reported as
  `logins_stored: "kwnt"`. kwant doesn't copy them into the vault.
