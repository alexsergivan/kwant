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
tiffutil -cathidpicheck dmg/background.png dmg/background@2x.png -out dmg/background.tiff
npx @tauri-apps/cli@2 build --target universal-apple-darwin --bundles app,dmg
```

The `.dmg` lands in `target/universal-apple-darwin/release/bundle/dmg/`. It is ad-hoc signed, so
the first launch needs right-click → Open. `.github/workflows/mac.yml` builds the same on CI.

The install window's layout (background, 660x400 window, 128px icons, kwant on the left and
Applications on the right) comes from `bundle.macOS.dmg` in `tauri.conf.json`. Finder writes it
into the image's `.DS_Store` through AppleScript, which Tauri skips when `CI=true` unless
`TAURI_BUNDLER_DMG_IGNORE_CI=true` is set, as the workflow does. The same script moves hidden
files (`.VolumeIcon.icns`, `.background`, …) outside the window, so they stay out of view even
with hidden files shown. The art is drawn by `dmg/make-background.py`; `background.tiff` is built
from the two PNGs and not committed.
