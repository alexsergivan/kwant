# fastbrowse

[fastbrowse](https://github.com/agent-labs-dev/fastbrowse) (MIT) is a browser agent: you give it
a whole task ("find the top story on Hacker News") and it returns an answer with citations. kwnt
offers it as an optional connection in "+ Add connection". kwnt's own browser
(`src/sandbox/browser.rs`) stays the default and is unchanged.

## Setup

The preset runs:

```
uvx --from fastbrowse fastbrowse-mcp --local --max-dollars 0.25 --max-seconds 540
```

with a 600 s timeout. It needs:

- **uv** (`curl -LsSf https://astral.sh/uv/install.sh | sh`, or `brew install uv`). kwnt also
  looks in `~/.local/bin`, `~/.cargo/bin`, `/opt/homebrew/bin` and `/usr/local/bin`, so a Mac app
  started from the Dock finds it.
- **Chrome or Chromium** on the computer kwnt runs on, or `FASTBROWSE_CHROME` set to its path.
- **Its own model key**: `OPENROUTER_API_KEY` or `AI_GATEWAY_API_KEY`. It is saved with the
  connection like any MCP server secret: masked in the API, removed from tool results, and never
  put in a bot's prompt or claude's MCP config. Every run costs money on that key; the preset caps
  a run at $0.25.

When one is missing, starting or testing the connection says which ("fastbrowse needs uv, …").

## What kwnt enforces

These hold for any server whose command or arguments name `fastbrowse-mcp`/`fastbrowse`, not just
the preset (`src/mcp/fastbrowse.rs`):

- **Local only.** `--local` is required; `--cloud-profile`, `--transport` and
  `BROWSER_USE_API_KEY` are refused when the server is saved and again before it starts.
- **Never authorize.** `--allow-authorize` is refused, and kwnt removes `authorize` from every
  call. Without it fastbrowse stops before an irreversible step (submit, send, delete, pay) with
  `needs_confirmation`.
- **Its stop is a card.** kwnt shows an approval card `mcp__<server>__confirm_step` with the task,
  the page it stopped on and what it says. The bot reads the answer under the result: approved
  means finish that step in kwnt's own browser (which asks again if it pays); declined means don't
  take it another way. The card follows the bot's approval mode, except that a paying step always
  asks.
- **Payments ask first.** A task that mentions paying gets the payment card before fastbrowse
  starts, in every approval mode and Flight School tier ([payments](payments.md)).
- **One path for every engine.** API drivers, codex and grok call it through kwnt's tool session
  or gateway; claude gets it as `…/api/mcp/gateway/server/<name>`, a gateway scoped to that one
  server, so the same checks run.

## Limits

- fastbrowse's own steps aren't shown in kwnt's browser panel; the result has `final_url` and
  `steps`.
- Payment detection reads the task text, so "order the list by date" or "card layout" also asks.
- On claude turns the gateway opens the bot's other MCP servers too, so they run twice.

## Tests

`cargo test --test fastbrowse_test` and `cargo test --lib fastbrowse`.
