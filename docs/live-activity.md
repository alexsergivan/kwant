# Live activity and human steps: API

Backend for two UI features on branch `live-activity` (the third, Connections, is in
[connections.md](connections.md)):

1. **Live activity**: what a bot is doing while it works (thinking, each tool it runs, waits for
   approval or for the person), streamed over SSE and kept on the reply after a reload.
2. **Human step**: a bot (or kwnt itself, on a CAPTCHA) asks the person to do one step in the
   bot browser (sign in, solve a CAPTCHA, something else), then waits for Done or Decline.

All events arrive on the existing SSE stream as `{ "type": ..., "data": ... }`. Turn events are
*scoped*: `turn_id`, `kwant_id`, `context_id` and (in threads) `thread_id` sit next to `type`.

## 1. Live activity

### SSE `activity` (scoped)

Sent every time a step starts or changes. Replace the step with the same `data.step.id` in the
turn's list (keyed by `turn_id`); append it if it's new.

```json
{
  "type": "activity",
  "turn_id": "…", "kwant_id": "…", "context_id": "…", "thread_id": "…",
  "data": {
    "step": {
      "id": "s3",
      "kind": "tool",
      "status": "failed",
      "title": "Running `cargo test`",
      "tool": "Bash",
      "detail": "exit code 101",
      "started_at": "2026-10-22T10:00:01.120Z",
      "ended_at": "2026-10-22T10:00:09.480Z",
      "duration_ms": 8360
    }
  }
}
```

| field | meaning |
|---|---|
| `id` | `s1`, `s2`, … in start order, unique within the turn |
| `kind` | `thinking`, `tool`, `approval`, `human_step`, `note` |
| `status` | `running` (thinking/tool), `waiting` (approval/human_step), `ok`, `failed`, `done`, `declined`, `stopped` (the turn ended while it was open, or an approval timed out) |
| `title` | short, human, already redacted: "Running `cargo test`", "Opening github.com", "Reading …/src/main.rs", "Searching the web for “rust sse”", "Using tool (github)", "Waiting for your approval: Running `rm -rf build`", "Waiting for you: Log in to GitHub" |
| `tool` | optional. The raw tool name (`Bash`, `mcp__github__create_issue`, `command_execution`) |
| `detail` | optional. The full command or address, or why the tool failed (redacted, ≤500 chars) |
| `text` | optional. The thinking step's whole text, set once the step ends |
| `ref_id` | optional. For `approval` it is the approval ticket, and for `human_step` it is the human-step id. Use it to link to the card |
| `started_at`, `ended_at` | RFC 3339 with milliseconds. `ended_at` is missing while the step is open |
| `duration_ms` | set once the step ends |

**Thinking text streams live** as the existing `ThinkingDelta` event (`data` is a string chunk,
scoped to the turn). The `activity` step of kind `thinking` only marks start (`running`) and end
(`ok`, with `text`). While it's running, show the `ThinkingDelta` text under it.

A `note` step (status `ok`) explains a limit. It is the first step of a turn on an API engine:
"This engine shares its thinking only when its model returns it; otherwise you'll see what it says
between steps and each tool it runs." Show it as a muted line, not an error.

### After a reload: `meta.activity` on the reply

The saved bot reply (message `meta`, a JSON string in the message row) gets:

```json
{ "activity": { "steps": [ <step>, … ], "dropped": 12 } }
```

`dropped` appears only when steps past the cap were left out. The caps are 200 steps per reply and
16,000 thinking characters per reply (the rest is cut with "…"). Titles are cut at 160 characters
and details at 500. Steps still open when the turn ended are saved as `stopped`.

### Loading mid-turn: `activity` on `GET /api/turns`

A turn that is still running has no saved reply yet. `GET /api/turns` lists each running turn with
its steps so far under `activity`, in the same shape (`{ "steps": [ <step>, … ] }`). Queued turns
and turns with no steps yet have no `activity`. Clients seed their live steps from it on load and
on every reconnect, so the working line shows the open step instead of "getting started". Steps
here never carry `text`: thinking text that streamed before the load is not included (a running
thinking step's text isn't redacted until the step ends), and the thinking step's title still
shows. Thinking that streams after the load arrives as `ThinkingDelta` as usual.

### Redaction

Titles, details and thinking text pass through redaction before they leave the server. That
includes the live stream: each `ThinkingDelta` chunk, every string in a `ToolCallPending`'s `args`
and in a `ToolCallCompleted`'s `result` is masked before it is broadcast, and an `args`/`result`
field named like a credential (`api_key`, `password`, `token`, …) loses its value whole, whatever
its type: a string, a number, an object or a list becomes `"[redacted]"` (`true` and `null` stay).
A plural name (`tokens`, `passwords`) loses its value when it holds any text, so counts such as
`"max_tokens": 100` stay readable, as does `"key": "Enter"`. Reply text (`TextDelta`) is not
touched.

So that a secret split across two `ThinkingDelta` chunks is matched whole, each chunk goes out
without its last two words (at most 256 characters); they are joined to the next chunk and
redacted with it. A credential whose value has not fully arrived is held with it: `password:` or
`password =` until the next word, `Authorization: Basic` / `Bearer` until the credential after
it, a bare `Bearer` before a word that is not all lowercase, and a quoted value
(`password = "two words"`) until its closing quote, all within the same 256-character cap. A
credential word in prose ("the password field is empty", "Bearer tokens expire") is held no
longer than any other word. By design, a bare `Bearer` before an all-lowercase word
(`Bearer abcdefghij`) is not held, so that word shows live. The saved text treats it as prose
too and leaves it unmasked, so the two match; `Authorization: Bearer` masks any word. This
keeps prose such as "Bearer tokens expire" or "bearer of bad news" from stalling the stream. Held text is per turn (and per delegated turn), and it is all sent, redacted,
before the thinking step ends: on the next reply text or tool call, an approval or human step, and
when the turn finishes, fails or is cancelled. Chunks therefore split at spaces, a word or two
behind the model. The thinking text saved on the reply is redacted again as a whole when the step
ends. Masked:

- the turn's MCP secret values are masked as `********`
- `key=value` / `"key": "value"` credentials (password, token, secret, api_key, …)
- `Bearer …` headers
- passwords inside URLs
- Slack and AWS keys
- whole words that look like provider keys: `sk-…`, `xai-…`, `ghp_…`/`gho_…`/`ghu_…`/`ghs_…`/`ghr_…`, `github_pat_…`, `glpat-…`, `AIza…`, `hf_…`, `npm_…`

### Per-engine coverage

| engine | thinking | tool steps | approvals |
|---|---|---|---|
| claude (CLI) | yes. Streamed `thinking_delta` blocks (or whole `thinking` blocks), when the model thinks | yes, from `tool_use` / `tool_result`. Failures carry the error text | yes |
| codex (CLI) | yes, from reasoning summaries (`codex exec --json` `reasoning` items) | yes: commands (with exit code), file changes, MCP calls, web searches, plan updates | yes (kwnt MCP tools) |
| grok (CLI) | yes, from `thought` stream events | yes, from `tool_call` / `tool_call_update` (failed status carries the error) | yes (kwnt MCP tools) |
| API engines (anthropic_api, openai_api, xai_api, ollama, custom_api) | **when the model returns it**, in full: OpenAI-style `reasoning_content` / `reasoning` (DeepSeek, xAI, Ollama, OpenRouter, vLLM…) and Anthropic `thinking` blocks, streamed (`thinking_delta`, `delta.reasoning_content`) or whole per reply. Redacted thinking is skipped. Turns get the `note` step first. The text the model writes before a tool call also arrives as `ThinkingDelta` | yes | yes |

kwnt doesn't switch on Anthropic extended thinking for API accounts itself (it costs tokens and
changes replies), so Anthropic API turns show thinking only for models that think by default.

## 2. Human step

### The agent tool

`request_human_step` on the agents' own MCP server (offered in chat turns that have a bot
browser):

```json
{ "reason": "Log in to GitHub so I can open the PR", "kind": "sign_in", "url": "https://github.com/login" }
```

`kind` is one of `sign_in`, `captcha`, `two_factor` (a one-time code, an authenticator or a
security key) or `other`. `reason` is required and at most 300
characters. `url` is optional, must be http(s), and is stored without user info, query or
fragment.

### Timeouts

- The turn waits **290 seconds** (the 300-second tool-call limit less 10 seconds) for the person to
  answer, unless the turn is stopped first (then the step is `cancelled`).
- If the wait runs out, the step **stays open** and the card stays live; the agent is told to end
  its turn and say what it is waiting for. The bot's browser stays held for the person.
- Open steps **never expire** on their own. When the person answers later, Done or Skip starts a
  **new turn** in the same chat with the outcome (`resumed: "new_turn"`), so the bot carries on
  from where it stopped.
- "Skip" in the UI is `POST /api/human-steps/:id/decline`: the bot is told the person skipped the
  step and continues without it.

**Auto-raise:** when a `browser_open`, `browser_page` or `browser_act` tool lands on a CAPTCHA or
anti-bot wall (`detect_bot_wall`), kwnt raises a `captcha` step itself (`source: "auto"`), waits
the same way, and after Done shows the agent the page again. This also happens when the tool call
itself fails (a click that stalls, is covered or times out because a check slid over the page):
kwnt reads the page and raises the step if it shows a wall. Login pages are **not**
auto-detected. Agents ask for those with the tool.

kwnt also raises the step where no tool call can wait on it. It does not wait; the person's answer
(Done, Skip, solved or timed out) starts a **new turn** in the same chat:

- **The person's link.** When kwnt opens a link from the person's message before the turn starts
  and the page is a wall, the card is up before the bot runs. The bot's prompt says the person was
  asked and tells it to end its turn in one line.
- **The shell browser API.** A bot without kwnt's tools (agent tools off, or the tools didn't
  start) drives the browser with curl. `POST /navigate`, `GET /page` and `POST /act` on
  `/api/sandbox/:id/` raise the step in the chat of the turn running on that browser. The answer
  gets `"human_step": "asked"` and a `notice` telling the bot to end its turn. While the step is
  open, `navigate` and `act` answer **409** ("The browser is waiting on the person …"). With no
  turn on the browser (someone curling it by hand), nobody is asked and the answer keeps the plain
  `blocked` notice.

Every bot's prompt says that a CAPTCHA met by the engine's own web fetch, search or built-in
browser goes to the person too. The bot opens that address in kwnt's browser, where the card comes
up, and never solves, bypasses or routes around the check. A bot with no browser at all is told to
name the page to the person.

**What counts as a wall** (`detect_bot_wall`, plus the in-page probe):

- Google's "unusual traffic" page.
- Cloudflare's "Just a moment", "Attention Required!" and "Checking your browser" pages, including
  Turnstile drawn in a closed shadow root.
- Pages that ask you to verify, confirm or prove you are human or not a robot (curly apostrophes
  included). This covers Amazon's "Enter the characters you see", PerimeterX "Press & Hold",
  DataDome, DuckDuckGo's "bots use DuckDuckGo too" and "solve the CAPTCHA" forms.
- An address whose host or path names a CAPTCHA.
- Widgets from reCAPTCHA, hCaptcha, Turnstile, Arkose, DataDome, GeeTest, PerimeterX and TikTok.

Phrases and addresses count only on a short page (at most 3000 characters of text) or in the
title. An article or a search about CAPTCHAs is not a wall.

While a step is open in a bot browser:

- the bot's browser tools are refused ("The browser is waiting on the person …")
- the browser controller is held for the person, so live-view input goes through even without `take_control` (and `take_control` answers `{"type":"control","active":true}`)

The person can use the live view freely.

**Bots never see typed passwords.** The agent's browser tools are `browser_open`, `browser_page`
and `browser_act` (click by ref or text, type, key, scroll); there is no script tool. Page reads use
`innerText` and element labels, which never include the value of an input (only submit and button
labels), and all three tools are refused while the step is open. This covers kwnt's own browser
tools; a CLI engine's own built-in browser or shell is outside it.

A turn that asks again while its step is open gets the same step back, so no second card is
created. Each agent can have at most 5 open steps.

### SSE `human_step_updated` (not scoped; routed to the step's chat)

Sent when a step opens and when it closes.

```json
{ "type": "human_step_updated", "data": { "step": <HumanStep> } }
```

`HumanStep`:

```json
{
  "id": "uuid",
  "kwant_id": "…",
  "channel_id": null,
  "direct_kwant_id": "…",
  "thread_id": null,
  "turn_id": "…",
  "kind": "sign_in",
  "reason": "Log in to GitHub so I can open the PR",
  "url": "https://github.com/login",
  "sandbox_target": "…",
  "source": "bot",
  "status": "open",
  "message_id": "…",
  "created_at": "2026-10-22T10:00:00Z",
  "closed_at": null
}
```

`status` is one of `open`, `done`, `declined` or `cancelled` (the turn was stopped). `source` is
`bot` (the agent asked) or `auto` (kwnt saw a CAPTCHA). `sandbox_target` is the bot-browser id
(team, channel or kwant), which is the same id the live browser panel uses.

### The card message

Opening a step also adds a chat message (`message_added`) from the bot. The first line names the kind: "Sign in needed", "Human check needed",
"Sign-in code needed" (two_factor) or "Your help is needed". Its text is something like:

> Sign in needed: Log in to GitHub so I can open the PR
> https://github.com/login
>
> Do it in the browser panel, then press Done.

Its `meta` holds:

```json
{ "kind": "human_step", "human_step_id": "…", "step_kind": "sign_in", "reason": "…", "url": "…",
  "sandbox_target": "…", "source": "bot", "status": "open" }
```

The card's `meta.status` is updated in place when the step closes. Render it as a card with
**Done** and **Decline** buttons while `status == "open"`, plus "Open browser" (the panel for
`sandbox_target`). When the person answers, a user message `Done` / `Declined` is added with
`meta: {"kind":"human_step_reply","human_step_id":"…","status":"done"}`.

### Notification

The standard `Notify` event and webhook use `kind: "human_step"`. The title is "<Bot> needs you in
the browser". The body is "<Sign in needed | Human check needed | Sign-in code needed | Your help is needed>. Open the
chat, do it in the browser panel, then press Done." The body never includes the reason or URL.

`human_step` was added to `NOTIFY_KINDS`. People who saved a custom notification list must turn it
on, so the settings UI needs a label for it, e.g. "A bot needs you in the browser".

### Live activity

While the step is open, the turn shows an `activity` step with kind `human_step`, status `waiting`,
title "Waiting for you: <reason>" and `ref_id` set to the step id. When the step closes, that
activity step settles as `done`, `declined` or `stopped`.

### Endpoints

| method | path | body | result |
|---|---|---|---|
| GET | `/api/human-steps?status=&context_id=&space=` | | `[HumanStep]`, newest first, at most 100. `context_id` is a channel, thread or direct-chat kwant id. Use `status=open` after a reload |
| GET | `/api/human-steps/:id` | | `{ "step": HumanStep, "waiting": bool }`. `waiting` is true while a live turn waits on it |
| POST | `/api/human-steps/:id/done` | none | `{ "step": HumanStep, "resumed": "turn" \| "new_turn" }` |
| POST | `/api/human-steps/:id/decline` | none | same |

Errors:

- `404` when the step is unknown
- `409 {"error":"the step is already done"}` when it is no longer open
- `400` for an unknown `status` in the list query

Done and Decline hand the browser back to the bot (once no other step is open in it). Then
either the waiting turn continues with the outcome (`resumed: "turn"`), or a new turn starts in
the same chat (`resumed: "new_turn"`).

### Known limits

- The person-hold on the browser lives in memory. After a kwnt restart an open step still blocks
  the bot's browser tools (that check reads the database), but live-view input needs
  `take_control` again.
- If the browser wasn't running when the step opened, only the database check protects it: bot
  tools are refused, and the person takes control as usual.
