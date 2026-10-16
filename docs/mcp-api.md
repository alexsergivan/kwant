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
  "created_at": "2026-10-16T12:00:00Z"
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

### `GET /api/kwants/:id/mcp-servers`
`200` → `{"server_ids": ["…"]}`. `404` unknown kwant.

### `PUT /api/kwants/:id/mcp-servers`
Body: `{"server_ids": ["…"]}` (replaces the whole assignment; `[]` clears it).
`200` → `{"server_ids": [...]}`. `400` unknown server id. `404` unknown kwant.

## Agent runtime

- **Tool names** are `mcp__<server>__<tool>`.
- **CLI agents (`claude`)**: each turn kwnt writes a private config file (mode 0600) under
  `<data_dir>/mcp/` with the kwant's enabled servers and passes it with
  `--mcp-config <file> --strict-mcp-config`; the file is removed after the turn.
  `codex` and `grok` CLI agents do not get MCP servers yet (no way to route their tool approvals
  through kwnt); kwnt explicitly disables MCP for codex.
- **API agents** (OpenAI-compatible chat endpoints, incl. Ollama): kwnt connects to the servers,
  offers their tools in the `tools` field, executes `tool_calls` and feeds results back, for at
  most `mcp_max_tool_iterations` model calls per turn (setting or env
  `KWNT_MCP_MAX_TOOL_ITERATIONS`, default 8). The Anthropic Messages endpoint is not given tools.

### Approvals

Every MCP tool call needs approval through the permission broker, for both agent kinds:

1. kwnt emits the SSE event
   `{"type":"ApprovalRequired","data":{"id":"<ticket>","tool_name":"mcp__echo__echo","details":{"source":"mcp","server":"echo","tool":"echo","arguments":{…},"kwant_id":"…","kwant_name":"…"}}}`
   and stores the ticket in `approvals`.
2. The UI resolves it with the existing `POST /api/approvals/:id/resolve`
   `{"decision": "allow_once" | "always_allow" | "deny"}`.
   `always_allow` allows that tool for that kwant until kwnt restarts.
3. Unanswered tickets are denied after 300 s, or when the turn is cancelled.

API agents also emit `ToolCallPending {id, name, args}` before running a call and
`ToolCallCompleted {id, result}` after it.

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
- Registering a stdio server lets the caller run any command as the kwnt user. The kwnt API has no
  authentication of its own: on a VPS keep it bound to `127.0.0.1` or behind an authenticating
  reverse proxy.
