Recorded Composio answers (2026-11-13, Lin), trimmed: tool schemas cut to one, account ids and the
person's details replaced with `[redacted]`/`*_redacted`. Recorded through a Composio Tool Router MCP
server, which serves the same `COMPOSIO_SEARCH_TOOLS` and `COMPOSIO_MANAGE_CONNECTIONS` tools as
Composio Connect.

- `search_tools_*.json`: `COMPOSIO_SEARCH_TOOLS` with one `use_case` (the typed name). A real app's
  slug appears in `results[].toolkits` and `toolkit_connection_statuses`; a made-up name still gets
  unrelated toolkits back.
- `manage_list_real_and_made_up.json`: `COMPOSIO_MANAGE_CONNECTIONS` `list` for elevenlabs (connected),
  heygen (real, not connected) and notarealapp123 (made up). The real and the made-up one answer the
  same ("initiated"), so `list` can't say whether an app exists.

The composio.dev page check was recorded the same day with `curl -I`:
elevenlabs 200, google_search_console 200, notarealapp123 404, googlesearchconsole 404,
ElevenLabs 308 → /toolkits/elevenlabs.
