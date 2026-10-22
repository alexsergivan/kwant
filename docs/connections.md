# Connections: API

One list of everything kwnt's bots reach outside the app, with its status, last check and the
bots that use it:

- MCP servers
- engine accounts
- website logins saved in the vault
- the mail (IMAP) and GitHub sources of Loose ends

Composio is offered as an **optional** MCP connector. It is off by default, and it is labelled
everywhere as storing the person's app sign-ins on Composio's servers. Every other login stays in
kwnt's own vault or database.

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
| `logins_stored` | `vault` (kwnt's password vault or Bitwarden), `kwnt` (kwnt's database or the engine CLI's own login folder), or `composio` (Composio's servers) |
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
| not tested since kwnt started, but signed in with OAuth | `connected` |
| not tested since kwnt started | `unknown` |

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

- Always `connected`. kwnt doesn't try a saved password on its own.
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
| account | bots bound to it, plus bots with no binding on its engine while it is the engine's default |
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
  "warning": "Composio keeps your app sign-ins on Composio's servers, not in kwnt's vault, and your bots' tool calls through it pass through Composio.",
  "add": "/api/connections/composio"
}
```

Show `warning` next to the add button and on the connection once it is added. Once added,
`connection_id` points at the MCP connection.

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

## Known limits

- MCP `last_check` is kept in memory only (see the MCP status rules).
- Loose ends sources are global in kwnt rather than per space, so every space lists them, and
  `used_by` names the current space's Loose ends bot.
- Engine CLI logins (`~/.claude`, Codex, Grok) live in each CLI's own folder, reported as
  `logins_stored: "kwnt"`. kwnt doesn't copy them into the vault.
