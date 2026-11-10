# Browser checks

- `native-drop.mjs`: drops real files on the chat through Chromium's own drag path, against a real `kwnt serve` (bot chat, room, #general, @mention, /idea refusal, side pane, Import dialog). Runs in CI's `fresh-machine` job. By hand: `cd frontend && npm run build && cargo build --bin kwnt && node e2e/native-drop.mjs ../target/debug/kwnt` (needs `playwright-core` and a Chromium; `CHROMIUM=/path/to/chrome` picks one, `DROP=synthetic BROWSER=webkit` runs the page-made drag events in WebKit).
