# LLM profiles and API access

## Profiles

A profile is one account for an engine (`claude`, `codex` or `grok`), for example a work Claude
subscription and a personal one. Every engine starts with a **system** profile, "This machine",
which runs exactly as kwnt did before profiles: the CLI's own login, or the key from Settings or
the environment. Existing setups keep working without any change.

| Kind | Runs with | Signed in by |
|---|---|---|
| `system` | the CLI's default config dir, Settings/env key | whatever you did on the host |
| `cli_home` | its own config dir (`CLAUDE_CONFIG_DIR`, `CODEX_HOME` or `GROK_HOME`); host credential env vars are removed | the in-app sign-in, or your own dir |
| `api_key` | the provider API with the profile's key only (never a host env key) | the key you paste |

A `cli_home` profile without `home_dir` gets a kwnt-managed dir, `<data_dir>/profiles/<id>`
(mode 0700). A chosen `home_dir` must be an existing, absolute, non-symlinked directory owned by
kwnt's user, and must not be `/`, `$HOME`, the engine's default dir or another profile's dir.
Deleting a profile never deletes its dir.

**Sign-in.** `POST /api/profiles/:id/login` runs the CLI's own login with the profile's home:
`claude auth login --claudeai` (open the link, then paste the code shown back into kwnt),
`codex login --device-auth` and `grok login --device-auth` (open the link and enter the code).
Only links to the providers' sign-in hosts are passed on, one sign-in runs per home at a time and
it expires after 10 minutes.

**Which profile a turn uses.** A kwant may be pinned to a profile (`profile_id`); otherwise the
engine's default profile runs. When that profile is out of usage or signed out, and nothing has
been shown or done yet in the turn, the turn moves to the engine's next enabled profile by
priority (at most 3 attempts), the chat shows a notice and a `profile_switched` event is sent.
Set `profile_failover: false` on a kwant to stay on its profile and get an explanation instead.
A profile that ran out of usage is skipped until its reset time (or one hour when the provider
didn't say). Overloads and rate limits are retried, not failed over.

Keys are write-only: no API response contains one, only `has_key`. Provider error text is kept
only as a class (`quota`, `auth`).

## API access (token)

Browsers may call `/api` only from kwnt's own page: requests with an `Origin` other than
`http://127.0.0.1|localhost|[::1]` on kwnt's port or 5199 (Vite), the page's own host, or an entry
in `KWNT_ALLOWED_ORIGINS` (comma separated) get `403`.

Requests that don't come from this machine need the API token. "This machine" means a loopback
peer, a loopback `Host` and no proxy headers (`X-Forwarded-*`, `Forwarded`, `X-Real-IP`). So:

- `kwnt serve` on your laptop, or a VPS reached through `ssh -L`: nothing changes.
- A VPS reached directly or through a reverse proxy, and Docker (the published port arrives from
  the bridge network): the token is needed.

Get the token with `kwnt token` (Docker: `docker compose exec kwnt kwnt token`). It lives in
`<data_dir>/api-token` (mode 0600), or set `KWNT_API_TOKEN` (16+ characters). Open
`http://<host>:<port>/#kwnt_token=<token>` once: the UI posts it to `/api/auth/session`, which sets an
HttpOnly, SameSite=Strict cookie for a year, and clears the address bar. Scripts can send
`Authorization: Bearer <token>` instead.

`KWNT_REQUIRE_TOKEN=always` requires the token for local requests too; `never` turns the check
off (only behind your own authentication). Always open: `/api/health`, `/api/auth/*`, the MCP
OAuth callback and the MCP endpoints, which carry their own per-turn tokens.

## API reference

Errors are `{"error": "<message>"}`. Bodies with unknown fields are refused.

### Auth

- `GET /api/auth/status` → `{"auth_required": bool, "authenticated": bool}`
- `POST /api/auth/session` `{"token": "..."}` → `204` + `Set-Cookie: kwnt_token=...`; `401` when wrong
- `DELETE /api/auth/session` → `204`, clears the cookie

### Profiles

- `GET /api/profiles` → `[Profile]`, by engine, default first, then priority
- `POST /api/profiles` `{"engine", "name", "kind": "cli_home"|"api_key", "home_dir"?, "priority"?, "enabled"?, "is_default"?, "api_key"?}` → `201 Profile`; `409` on a duplicate name or home
- `PATCH /api/profiles/:id` `{"name"?, "priority"?, "enabled"?, "is_default"?}` → `Profile`
- `DELETE /api/profiles/:id` → `204` (not for system profiles; pinned kwants go back to the default)
- `PUT /api/profiles/:id/key` `{"api_key"}` → `204` (api_key profiles)
- `DELETE /api/profiles/:id/key` → `204`
- `POST /api/profiles/:id/login` → `Login` (cli_home profiles; returns the running one if any)
- `GET /api/profiles/:id/login/:flow_id` → `Login`
- `POST /api/profiles/:id/login/:flow_id/code` `{"code"}` → `Login` (Claude)
- `DELETE /api/profiles/:id/login/:flow_id` → `204`
- `POST /api/profiles/:id/logout` → `204`

```jsonc
// Profile
{
  "id": "prf_…" | "prf_system_claude",
  "engine": "claude" | "codex" | "grok",
  "name": "Work",
  "kind": "system" | "cli_home" | "api_key",
  "enabled": true,
  "is_default": false,
  "priority": 100,
  "home_dir": "/home/me/.kwnt/profiles/prf_…" | null,
  "has_key": false,
  "signed_in": true | false | null,      // null: couldn't tell (CLI missing)
  "account_label": "me@example.com" | "API key" | null,
  "limited_until": "2026-10-16T17:00:00Z" | null,
  "last_error": "quota" | "auth" | null
}
// Login
{
  "flow_id": "login_…",
  "profile_id": "prf_…",
  "status": "starting" | "waiting" | "succeeded" | "failed" | "expired" | "cancelled",
  "url": "https://claude.ai/oauth/authorize?…" | null,
  "user_code": "ABCD-1234" | null,
  "needs_code": true,                    // Claude: paste the code from the page
  "error": null,
  "expires_at": "2026-10-16T12:10:00Z"
}
```

### Kwants

`GET /api/kwants`, `POST /api/kwants` and `PUT /api/kwants/:id` return kwants with two more
fields, and the writes accept them: `"profile_id": "prf_…" | null` (null: engine default; omit to
leave unchanged) and `"profile_failover": bool`. A profile for another engine than the kwant's
model gets `400`.

### Events (`/api/events`)

- `turn_started` `{"kwant_id", "context_id"}`: the kwant is working
- `turn_ended` `{"kwant_id", "context_id", "outcome": "completed"|"failed"|"cancelled"}`: always follows `turn_started`
- `profile_switched` `{"kwant_id", "from_profile_id", "from_name", "to_profile_id", "to_name", "class": "quota"|"auth", "retry_at"?}`
- `TurnComplete` gains `"kwant_id"` and `"profile_id"`; `Error` gains `"kwant_id"`
