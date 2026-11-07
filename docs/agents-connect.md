# Use kwant from your coding agent

Claude Code, Codex, Cursor and other MCP clients can ask your kwant bots for work ("ask Vega to check the
release notes") and get the reply back where you're working. You make one connection per tool in
**Settings → Coding agents**. Each connection has its own token, only reaches the spaces and bots you pick,
and can be revoked at once.

The outside agent is never you. A bot sees its messages marked as coming from that agent, and the agent can't
approve anything.

## Connect

1. In **Settings → Coding agents**, choose **Connect a coding agent**. Name it (for example "Claude Code on my
   laptop"), then pick the spaces and either all of their bots or some of them.
2. Copy the token. It starts with `kag_` and **is shown only once**.
3. Add kwant to your tool. In the examples below, `https://<server>` is the address you open kwant at.

**The token is like a password.** Anyone who has it can ask your bots for work, within the connection's
spaces and bots, until you revoke it. It doesn't expire. Keep it out of repos and shared dotfiles. If it
leaks, revoke the connection and make a new one.

### Claude Code

```sh
claude mcp add --transport http kwant https://<server>/api/mcp/agents --header "Authorization: Bearer kag_…"
```

This adds it for you in the current project (`--scope local`, the default). Add `--scope user` to have it in
every project. Check it with `claude mcp get kwant`.

### Codex

In `~/.codex/config.toml`:

```toml
[mcp_servers.kwant]
url = "https://<server>/api/mcp/agents"
bearer_token_env_var = "KWANT_AGENT_TOKEN"
```

Then set the token in your shell, for example `export KWANT_AGENT_TOKEN=kag_…` in your shell profile.
That keeps it out of the config file. Streamable HTTP servers need no feature flag in current Codex.

### Cursor

In `~/.cursor/mcp.json`, or `.cursor/mcp.json` in a project you don't commit:

```json
{
  "mcpServers": {
    "kwant": {
      "url": "https://<server>/api/mcp/agents",
      "headers": { "Authorization": "Bearer ${env:KWANT_AGENT_TOKEN}" }
    }
  }
}
```

Cursor fills in `${env:KWANT_AGENT_TOKEN}` from your environment, so set it in your shell profile as for
Codex. You can also paste the token itself in place of `${env:KWANT_AGENT_TOKEN}`.

The endpoint uses MCP's streamable HTTP transport with plain JSON responses (no event stream). Every call
needs the token, including calls from the same machine.

These setups were checked against each tool's own docs on 2026-11-07:
<https://code.claude.com/docs/en/mcp>, <https://developers.openai.com/codex/mcp> and
<https://cursor.com/docs/context/mcp>.

## What the agent can do

| Tool | What it does |
| --- | --- |
| `list_bots` | Lists the bots this connection may ask, with each one's id, name, role, team, space, and whether it's busy or idle. |
| `ask` | Sends a bot a message (up to 8,000 characters) and waits up to `wait_seconds` (0–300, default 120) for the reply. |
| `get_reply` | Checks a turn that `ask` started, by `turn_id`. |
| `list_threads` | Lists the threads this connection started, newest first, with the bot's last reply to this connection's own asks (first 500 characters). Replies to what the person types in the thread are left out. |

`ask` posts into a thread with the bot called **From \<connection name\>**. There's one standing thread per
connection and bot, so the bot keeps the context. `thread: "new"` starts a fresh thread, and a `thread_id`
continues one this connection started earlier. You see these threads in kwant like any other, and you can
read along or reply. When you reply there, the bot's answer to you stays with you: the agent only ever gets
the bot's reply to its own ask.

Each answer is `{status, reply_text, attachments, thread_id, turn_id}`. `status` is one of:

- **`done`**: the bot replied. `reply_text` is the reply, with passwords, keys and home-folder paths hidden.
- **`working`**: the bot is still going. Call `get_reply` with the `turn_id` later.
- **`waiting_on_person`**: the bot needs you first, to approve a step or answer a question. `reply_text`
  says which in one line. You answer in kwant as usual, and the turn carries on.
- **`failed`**: the turn stopped. `reply_text` says why.

## What it can't do

- **Approve anything.** Approvals, questions and anything else that needs you go to you in kwant, exactly as
  they do today. The agent only sees that the turn is waiting.
- **Act as you.** Its messages are stored and shown as the agent's, and the bot is told they aren't from you.
- **Reach anything else.** The token only works on `/api/mcp/agents`. On every other kwant address it gets a
  403, even from the same machine. It can't read passwords, change settings, manage connections or send chat
  messages as you.
- **Reach other bots or threads.** Only the bots and spaces you picked, and only threads this connection
  started.
- **Run unlimited work.** Each connection can make up to 60 asks an hour and have 3 turns working at once.
  Past either limit it gets HTTP 429 with `retry_after` (in seconds). Its turns also count toward the
  chain budget and the daily spend stop, like any other turn.

Every call is logged with the connection's name, the tool, the bot and whether it was allowed. Logs never
include the token. Each connection keeps its newest 1,000 entries; a revoked connection keeps its log.

## Revoke

In **Settings → Coding agents**, choose **Revoke** next to the connection. It takes effect at once: the next
call with that token gets a 401. Its threads stay in kwant. You can have up to 10 live connections.
