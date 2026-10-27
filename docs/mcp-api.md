# MCP servers: HTTP API contract

Kwants can use tools from [Model Context Protocol](https://modelcontextprotocol.io) servers.
An MCP server is registered once, then assigned to individual kwants. A kwant only ever sees the
servers assigned to it, never the host user's own MCP configuration.

All endpoints are JSON. Errors use the status code plus `{"error": "<message>"}`.

## Server object

```json
{
  "id": "6f1c…",
  "name": "github",
  "transport": "stdio",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-github"],
  "url": null,
  "env": { "GITHUB_TOKEN": "********" },
  "headers": {},
  "enabled": true,
  "timeout_secs": 30,
  "created_at": "2026-10-16T12:00:00Z",
  "oauth": { "connected": false, "expires_at": null }
}
```

| Field | Type | Notes |
|---|---|---|
| `id` | string | Server-generated UUID. |
| `name` | string | Unique. 1–48 chars, `[A-Za-z0-9_-]`, starts with a letter or digit, no `__`. `kwnt` is reserved. Used in tool names (`mcp__<name>__<tool>`), so renaming changes tool names. |
| `transport` | `"stdio"` \| `"http"` | `stdio`: kwnt (or the agent CLI) starts `command` as a child process. `http`: streamable HTTP MCP endpoint at `url`. |
| `command` | string \| null | Required for `stdio`, must be null/absent for `http`. Resolved via `PATH`; no shell is involved. |
| `args` | string[] | `stdio` only. Default `[]`. |
| `url` | string \| null | Required for `http` (`http://` or `https://`), null for `stdio`. |
| `env` | object<string,string> | `stdio` only. Extra environment variables for the process. Keys `[A-Za-z_][A-Za-z0-9_]*`. |
| `headers` | object<string,string> | `http` only. Extra request headers, e.g. `Authorization`. |
| `enabled` | bool | Disabled servers are kept but never offered to any kwant. Default `true`. |
| `timeout_secs` | int | Per-request timeout (initialize, list, call), 1–600. Default `30`. |
| `created_at` | string \| null | RFC 3339. |
| `oauth` | object | Read-only sign-in status (see [Sign-in](#sign-in-oauth)): `connected` bool, `expires_at` RFC 3339 or null. Never contains tokens. Always `{"connected": false, …}` for stdio servers. |

### Secrets

`env` and `headers` values are treated as secrets. They are stored, but **every value is
returned as the mask `"********"`**; only the keys are visible. On update:

- a value of exactly `"********"` keeps the stored value for that key,
- any other value replaces it,
- a key that is left out is deleted,
- leaving the whole `env` / `headers` field out (or `null`) keeps all stored values.

Secret values never appear in API responses, logs, or the agent CLI's command line. Text returned
by `POST /test` (errors, stderr) has every stored secret value replaced with `********`.

## Endpoints

### `GET /api/mcp/servers`
`200` → `Server[]`, ordered by name.

### `POST /api/mcp/servers`
Body: `{name, transport, command?, args?, url?, env?, headers?, enabled?, timeout_secs?}`.
`200` → `Server`. `400` validation error. `409` name already taken.

### `GET /api/mcp/servers/:id`
`200` → `Server`. `404`.

### `PUT /api/mcp/servers/:id`
Body: same as create (`name` and `transport` required), with the secret-mask rules above.
`200` → `Server`. `400`, `404`, `409`.

### `DELETE /api/mcp/servers/:id`
`200` → `{"status": "deleted"}`. `404`. Also removes the server from every kwant.

### `POST /api/mcp/servers/:id/test`
Connects with the stored config (works for disabled servers too), runs `initialize` and
`tools/list`, then shuts the server down. Always `200` once the server exists (`404` otherwise):

```json
{ "ok": true, "server_info": {"name": "echo", "version": "1.0"}, "duration_ms": 41,
  "tools": [{"name": "echo", "description": "…", "input_schema": {"type": "object"}}] }
```
```json
{ "ok": false, "error": "timed out after 30s waiting for initialize", "stderr_tail": "…", "duration_ms": 30004 }
```

An http server that answers 401 while kwnt has no usable sign-in (and the user set no
`Authorization` header of their own) gives:

```json
{ "ok": false, "auth_required": true, "error": "Sign-in required", "duration_ms": 120 }
```

## Sign-in (OAuth)

http servers that require sign-in (e.g. `https://mcp.notion.com/mcp`) use the MCP authorization
spec (2025-06-18): kwnt reads the 401's `WWW-Authenticate: Bearer resource_metadata=…` (or
`/.well-known/oauth-protected-resource`), then the authorization server's metadata
(`/.well-known/oauth-authorization-server`, falling back to OpenID discovery), registers itself
as client `kwnt` (dynamic client registration, RFC 7591), and runs the authorization code flow
with PKCE S256 and the `resource` parameter (RFC 8707). Authorization-server endpoints must be
https (plain http only on loopback).

### `POST /api/mcp/servers/:id/oauth/start`
Body: `{"origin": "https://kwnt.example.com"}`: the browser's `location.origin`. It must be http(s)
with no path, and its host must equal the request's `Host` or `X-Forwarded-Host`. The callback is
`<origin>/api/mcp/oauth/callback`, so the same build works on localhost and behind a proxy.
`200` → `{"authorize_url": "https://…"}`: open it in a popup. `400` bad origin or a stdio server,
`404` unknown server, `502` discovery or registration failed (`{"error": "…"}`).
The sign-in must finish within 10 minutes; starting again for the same server replaces the
earlier attempt. Registrations are reused per server, callback URL and authorization server.

### `GET /api/mcp/oauth/callback?code=…&state=…` (or `?error=…&state=…`)
Where the provider redirects the popup. kwnt exchanges the code and stores the tokens, then
returns a small HTML page that runs
`window.opener.postMessage({type: "kwnt-mcp-oauth", server_id, ok, error?}, location.origin)`
and closes itself (or goes to `/` when it has no opener). `server_id` is null when the `state`
is unknown, expired or already used. `error` is a short readable reason.

### `DELETE /api/mcp/servers/:id/oauth`
Forgets the tokens. `200` → `{"status": "disconnected"}`. `404` unknown server.

### How tokens are used
- Before every connection (test, API-agent turns, the claude CLI config) kwnt refreshes a
  token that expires within 60 s and sends `Authorization: Bearer <token>`.
- A user-set `Authorization` header always wins; kwnt's token is then not sent.
- If the authorization server rejects a refresh, the token is dropped and the server needs
  sign-in again. A network failure keeps it (and uses it while it is still valid).
- On `POST /test`, a 401 despite a token triggers one forced refresh and retry; if that fails
  too, the token is dropped and the answer is `auth_required`.
- Tokens are stored in the database like other secrets, never returned or logged, and redacted
  from tool output and errors. Changing a server's `url` or `transport`, or deleting it, drops
  its tokens and registrations.
- The claude CLI gets the current access token in its per-turn config file; a turn longer than
  the token's lifetime is not refreshed mid-turn.

For local checks, `kwnt mcp-oauth-mock-server [--port 7790] [--expires-in 3600]` runs an
OAuth-protected mock MCP server at `http://127.0.0.1:<port>/mcp` whose sign-in page has an
"Allow" link (no credentials).

### `GET /api/kwants/:id/mcp-servers`
`200` → `{"server_ids": ["…"]}`. `404` unknown kwant.

### `PUT /api/kwants/:id/mcp-servers`
Body: `{"server_ids": ["…"]}` (replaces the whole assignment; `[]` clears it).
`200` → `{"server_ids": [...]}`. `400` unknown server id. `404` unknown kwant.

## Suggestions (agents find servers, the person installs them)

Agents get two tools on `kwnt_tools` (always on, every turn):

- `mcp_search {query}`: searches the official MCP Registry (`KWNT_MCP_REGISTRY_URL`, default
  `https://registry.modelcontextprotocol.io`; 8 s timeout, results cached 10 min). Each hit says what would run
  (`transport`, `runs`, `runs_on_users_computer`), what the person must provide (`needs`), or `not_one_click` with the
  reason, and `status` (`available` with the tool prefix, `installed_but_off`, `suggested`) when the space already has it.
  Registry text is returned as untrusted data.
- `mcp_suggest {registry_id, reason}`: records a pending suggestion and posts a card into the chat the turn runs in.
  It installs nothing. At most 3 per turn, 10 pending per agent, one pending per registry entry per agent; an entry
  that is already installed or isn't one-click is refused with a note.

Only these become one-click plans (the plan is stored with the suggestion; install runs exactly that):
remote `streamable-http` (https, or http on loopback, no templated URL); `npm` → `npx -y <id>@<version>`;
`pypi` → `uvx <id>==<version>`; `oci` → `docker run -i --rm -e NAME… <image>:<version>`. Exact versions only,
default npm/PyPI registries only, no shell, arguments only from fixed registry values. stdio plans are marked
`runs_on_your_computer: true`.

### `GET /api/mcp/suggestions?space=&status=&kwant_id=`
Newest first. Each item is the suggestion record plus `card` (same shape as the message meta below).

### `POST /api/mcp/suggestions/:id/install` `{env?: {NAME: value}, headers?: {Name: value}}`
Person only: a bearer that is an agent turn/gateway token or isn't the API token gets 403. Keys must be the plan's
`inputs`; required ones must be present (400 otherwise). For an input with a `template` such as `Bearer {api_key}`
the person types just the value. Creates the server (named after the entry, `-2`… if taken), enables it only for the
suggesting agent, then tests it.
- 200 `{suggestion, server, test, oauth_required}`; `server` is the masked server object with `disabled_for_kwants`.
  When `oauth_required` is true the server is kept; continue with `POST /api/mcp/servers/:id/oauth/start`.
- 422 `{error, suggestion, test}`: the test failed; the server is removed, the suggestion stays `pending` with `last_error`.
- 404 unknown, 409 not pending (or the agent was deleted).
The agent's next turn has the tools; no restart.

### `POST /api/mcp/suggestions/:id/dismiss`
Person only (as above). 200 with the suggestion, 409 if not pending.

### Card
A message from the agent with `meta.kind = "mcp_suggestion"`: `suggestion_id, kwant_id, registry_name, version, title,
description, reason, repository_url, transport, url, command, args, package, runs_on_your_computer, fixed_env,
fixed_headers` (names only), `inputs[{name, kind: "env"|"header", description, required, secret, template, placeholder}],
status (pending|installing|installed|dismissed), server_id, last_error`. Changes arrive as SSE `mcp_suggestion_updated
{suggestion}` and the message meta is updated in place.

## Fallbacks before "not possible"

Two of kwnt's own tools exist so a bot tries the obvious way round, or offers the fix, instead of
reporting a dead end. The prompt tells every bot with kwnt's tools to use them first
(`server::continuity::prompt_block`, "Before you call anything impossible").

### `page_source {url, full?}`
A public page's HTML and the facts an audit reads from it: `title`, `lang`, `meta` (description, robots,
viewport…), `canonical`, `hreflang`, `social` (`og:*`, `twitter:*`), `h1_count`, `headings`, `images`
(`total`, `missing_alt`, `empty_alt`, `missing_alt_src`), `links`, `json_ld` types, plus the response's
`status`, `redirects`, `content_type` and `x_robots_tag`, and the raw `<head>` (12,000 characters; the whole
HTML up to 60,000 with `full: true`). claude's WebFetch returns a model's summary of a page's text and the
browser tools return visible text, so neither shows tags or attributes.

It runs without a card, like WebFetch: it reads public addresses only. Every hop, redirects included
(at most 5), is checked with `fetch_risk::public_only` and fetched from the addresses just checked, so a
page on this computer or a private network is refused and the bot is told to use WebFetch or a shell
command, which ask the person. Pages are read up to 2 MB.

### `connection_suggest {app, name, reason}` (chats only)
Posts a card offering the person a connection to an app the bot can't reach, through Composio (`app` is
its toolkit slug, e.g. `google_search_console`). It connects nothing. A turn posts each app once and at
most 3 in all. The card is a message from the bot with `meta.kind = "connection_suggestion"`:
`kwant_id, provider: "composio", app, name, reason, composio: ready|off|needs_key|not_added, connect`
(the person-only `POST /api/connections/composio/apps/:slug/connect` that starts the sign-in). Its text
names the steps, so it reads on its own where the app has no card for it.

## Agent runtime

- **Tool names** are `mcp__<server>__<tool>`.
- **CLI agents (`claude`)**: each turn kwnt writes a private config file (mode 0600) under
  `<data_dir>/mcp/` with the kwant's enabled servers and passes it with
  `--mcp-config <file> --strict-mcp-config`; the file is removed after the turn. Secrets are
  only in that file, never on the command line. claude's permission prompts go to kwnt
  (`--permission-prompt-tool mcp__kwnt__approve`, served at `POST /api/mcp/permission/<token>`,
  valid only while that turn runs), so only the kwant's own MCP tools can be approved; claude's
  built-in tools stay denied as before.
- **CLI agents (`codex`)**: codex gets exactly one MCP server, kwnt's per-turn gateway
  (`POST /api/mcp/gateway`), via
  `-c mcp_servers={kwnt={url=…,bearer_token_env_var="KWNT_MCP_GATEWAY_TOKEN",default_tools_approval_mode="approve",…}}`.
  A `-c` table is merged into codex's config rather than replacing it, so kwnt first asks codex
  which servers it would load for the turn (`codex mcp list --json`, run in the turn's working
  directory: user config, profile, trusted-project `.codex/config.toml`, system config) and adds
  `"<name>"={enabled=false}` for each one into the same table, so they are neither offered nor
  started. If that listing fails, kwnt doesn't run codex for the turn. `kwnt` is reserved here
  too: a user codex server named `kwnt` is switched off like the others on turns without the
  gateway, but on a gateway turn the two entries would merge, so kwnt refuses the turn and asks
  the user to rename that server. The turn's token is passed
  in the `KWNT_MCP_GATEWAY_TOKEN` env variable and sent as `Authorization: Bearer`, never on the
  command line, and blanked for codex's own shell commands
  (`-c shell_environment_policy.set.KWNT_MCP_GATEWAY_TOKEN=""`); other tokens and finished turns
  get `401`. The gateway connects to the kwant's servers
  on first use, lists their tools as `<server>__<tool>` (codex shows them under `mcp__kwnt`),
  and asks for approval (below) before forwarding each call, so codex's own approval is turned
  off for it. Without servers (or for a kwant that opted out) codex gets only the
  `enabled=false` entries: `-c mcp_servers={}` when the user has no servers of their own.
- **CLI agents (`grok`)**: every grok turn runs with a fresh private `GROK_HOME` (mode 0700,
  under the system temp dir, removed when the turn ends, is cancelled or grok is killed; leftovers
  older than 24 h are swept on the next turn). Its `config.toml` (0600) holds only the
  `[models]`/`[model.*]` settings from the user's own config, so user MCP servers, plugins,
  hooks, permission grants and folder trust don't apply, and the user's `~/.grok` is never
  written. `GROK_{CLAUDE,CURSOR}_{MCPS,HOOKS}_ENABLED=0` and `GROK_MANAGED_MCPS_ENABLED=0` stop
  grok importing Claude, Cursor and managed servers and hooks. The login is read in place via
  `GROK_AUTH_PATH`; if grok says it isn't signed in, the turn is retried once with a 0600 copy of
  `auth.json` inside the temp home. With servers, the temp config adds kwnt's gateway as its only
  server, `[mcp_servers.kwnt]` at `POST /api/mcp/gateway/grok`, with
  `Authorization = "Bearer ${KWNT_MCP_GATEWAY_TOKEN}"` (grok expands it from its env; the token is
  never in the file or argv) and `shell_environment_policy.exclude` hiding the variable from
  grok's shell. grok runs with `--allow 'MCPTool(kwnt__*)' --output-format streaming-json`.
  grok rejects tool names containing `__`, so this endpoint lists tools as `<server>-<tool>`
  (characters other than letters, digits and `-` become `_`; clashes get `-2`, `-3`, …) and maps
  calls back; approval cards still show `mcp__<server>__<tool>`. When grok ends without a reply
  (a tool it needed approval for, no login, an error, a cancel), the turn's reply says why
  instead of being empty.
- **API agents** (OpenAI-compatible chat endpoints, incl. Ollama): kwnt connects to the servers,
  offers their tools in the `tools` field, executes `tool_calls` and feeds results back, for at
  most `mcp_max_tool_iterations` model calls per turn (setting or env
  `KWNT_MCP_MAX_TOOL_ITERATIONS`, default 8). The Anthropic Messages endpoint is not given tools.

### Approvals

Every MCP tool call needs approval through the permission broker, for both agent kinds, unless the
bot's approvals are `off`, or `smart` and the server marks the tool `readOnlyHint: true` without
`destructiveHint: true` (see [approvals.md](approvals.md)):

1. kwnt emits the SSE event
   `{"type":"ApprovalRequired","data":{"id":"<ticket>","tool_name":"mcp__echo__echo","details":{"source":"mcp","server":"echo","tool":"echo","arguments":{…},"kwant_id":"…","kwant_name":"…"}}}`
   and stores the ticket in `approvals`.
2. The UI resolves it with the existing `POST /api/approvals/:id/resolve`
   `{"decision": "allow_once" | "always_allow" | "deny"}`.
   `always_allow` allows that tool for that kwant until kwnt restarts.
3. Unanswered tickets are denied after 300 s, or when the turn ends or is cancelled; kwnt then
   emits `{"type":"ApprovalWithdrawn","data":{"id":"<ticket>"}}` so the UI can drop the card.
   Resolving a withdrawn (or unknown) ticket returns `409` and changes nothing: its tool never
   runs, and the ticket keeps its `cancelled` or `expired` status. A call the turn ended under is
   recorded with status `cancelled`.

API agents also emit `ToolCallPending {id, name, args}` before running a call and
`ToolCallCompleted {id, result: {status, content}}` after it (`content` cut to 4000 characters).
A server that fails to start is skipped for that turn with a `ThinkingDelta` note; tools whose
namespaced name is not a valid function name (`^[A-Za-z0-9_-]{1,64}$`) are not offered. When the
model still asks for tools on its last allowed call, those calls are not run and the reply ends
with a note about the limit.

### Recorded tool calls

The kwant's reply message stores its MCP calls in `tool_calls` (JSON array, `null` when none):

```json
[{ "id": "call_1", "name": "mcp__echo__echo", "server": "echo", "tool": "echo",
   "arguments": {"text": "hi"}, "status": "ok", "result": "hi", "duration_ms": 12 }]
```

`status` is `ok`, `error` or `denied`. `result` is the tool's text output (or error), cut to
4000 characters. For CLI agents kwnt only sees the approval step, so their records have
status `allowed` or `denied` and no `result`.

## Deployment notes

- Nothing beyond kwnt itself is required: stdio servers are plain child processes, so whatever
  runtime a server needs (`node`, `python`, `uvx` …) must be installed by the user, as usual.
- Stdio servers started by kwnt get a minimal environment (`PATH`, `HOME`, `USER`, `LANG`,
  `TMPDIR`) plus the server's `env`, not kwnt's own environment (which may hold API keys).
  Stdio servers of a `claude` CLI agent are started by claude itself and inherit claude's
  environment (and so kwnt's) plus the server's `env`.
- CLI agents reach kwnt's permission endpoint at `http://127.0.0.1:<port>` (or the configured
  host when it is a specific address), so the CLI must run on the same machine as kwnt.
- Registering a stdio server lets the caller run any command as the kwnt user. The kwnt API has no
  authentication of its own: on a VPS keep it bound to `127.0.0.1` or behind an authenticating
  reverse proxy.
