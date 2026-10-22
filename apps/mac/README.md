# kwant for Mac

A Tauri v2 shell around kwant. The app links the `kwnt` crate and runs the same server as
`kwnt serve` inside its own process, on 127.0.0.1, and shows the UI in a window (WKWebView).

- **Data:** `~/.kwnt` (or `KWNT_DATA_DIR`), the same as the CLI. App logs: `~/.kwnt/mac-app.log`.
- **Port:** 8799, or the `port` in `config.toml`. If kwant already answers `/api/health` there, the
  app opens a window on it and starts nothing. If something else holds the port, it uses a free one.
- **PATH:** apps opened from Finder get a bare PATH, so the app merges the login shell's PATH with
  `/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin` to find `claude`, `codex` and `grok`.
- **Menu bar:** Open kwant, a status line (approvals waiting), Quit. Closing the window keeps kwant
  running; Quit stops it.
- `kwant --check` starts kwant headless on a free port with a throwaway data dir, waits for
  `/api/health` and exits 0. CI runs it on the built app.

This crate is its own Cargo workspace with its own `Cargo.lock`, so Tauri never enters the build
of the `kwnt` crate.

## Build (macOS only)

```sh
(cd ../../frontend && npm ci && npm run build)   # kwnt embeds frontend/dist
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npx @tauri-apps/cli@2 build --target universal-apple-darwin --bundles app,dmg
```

The `.dmg` lands in `target/universal-apple-darwin/release/bundle/dmg/`. It is ad-hoc signed, so
the first launch needs right-click → Open. `.github/workflows/mac.yml` builds the same on CI.
