# kwant for Mac

A Tauri v2 shell around kwant. The app links the `kwnt` crate and runs the same server as
`kwnt serve` inside its own process, on 127.0.0.1, and shows the UI in a window (WKWebView).

- **Data:** `~/.kwnt` (or `KWNT_DATA_DIR`), the same as the CLI. App logs: `~/.kwnt/mac-app.log`.
- **Port:** 8799, or the `port` in `config.toml`. If kwant already answers `/api/health` there, the
  app opens a window on it and starts nothing. If something else holds the port, it uses a free one.
- **PATH:** apps opened from Finder get a bare PATH, so the app merges the login shell's PATH with
  `/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin` to find `claude`, `codex` and `grok`.
- **Links:** each window only shows its own place: This Mac's loopback page, or exactly one
  server's origin, plus the bundled pages. A link to another listed place opens (or focuses) that
  place's window. Any other web or mail address, including sign-in pages and `target=_blank`
  links, opens in the default browser. Other schemes (`file:`, app links) are ignored. The page
  sees `window.__KWANT_APP__ = 'mac'`, so sign-ins say to finish in the browser and watch the
  server for the result. See `src/links.rs`.
- **Places, side by side** (0.7.5): This Mac and every added server each get their own window
  ("kwant – This Mac", "kwant – host"; just "kwant" while This Mac is alone), all running at once.
  Nothing relaunches. The list lives in `~/.kwnt/desktop-places.json` (origins and which windows
  were open, never a token, mode 0600); an older `desktop-target.json` is migrated once and
  removed. Closing a window hides it and its place keeps running; the Dock click shows the window
  used last. See `src/places.rs`.
- **Menu bar:** with no server, as before (Open kwant, a status line, Show nudges, Quit) plus
  **Add a server…**. With servers: one row per place with what waits there, **Add a server…**,
  **Remove a server ▸** (asks first, then closes its window, clears its sign-in cookie and drops
  it), **Run kwant on this Mac** (off stops the kwant this app started and closes its window; it
  can't go off with no server) and **Show nudges**. On macOS the Window menu lists the places
  with ⌘1, ⌘2…. With more than one place, a nudge names its place ("host · …") and opens in that
  place's window. See `src/menu.rs`.
- **Updates:** 15 seconds after start and then every 6 hours the app reads
  `releases/latest/download/latest.json` (prereleases are never offered). A newer version shows in
  the web app (`GET /api/app/update`), as one notification per version and as **Update to x.y.z…**
  at the top of the menu-bar menu (**Update this app to x.y.z…** once a server is listed).
  Installing downloads `kwant_universal.app.tar.gz`, checks its signature against
  `plugins.updater.pubkey` in `tauri.conf.json`, replaces the app, stops kwant and relaunches. If
  anything fails, the running app stays as it was. See `src/update.rs`.
- **Add a server:** a bundled page (`loading/connect.html`) checks an address (`GET /api/health`
  must answer as kwant, 10 s, no redirects), adds its origin (scheme, host and port only) and
  opens its window; a server already listed is focused. The app checks the server natively, then
  opens it. http is allowed only on 127.0.0.1, localhost and [::1] (an SSH tunnel). A pasted
  one-time sign-in link is opened once in that server's window and is never saved or logged. A
  server's count comes from the server with the windows' sign-in cookie, held in memory only. Only
  the bundled pages can call the app's commands (`capabilities/connect.json`, and each command
  checks the page itself); a server's pages can call none. The log masks `kwnt_token` values and
  the fragment of any address. Commands: `src/connect.rs`.
- `kwant --check` starts kwant headless on a free port with a throwaway data dir, waits for
  `/api/health` and exits 0. CI runs it on the built app.

**On Linux** (first builds, beta) the same code runs with these differences, all behind
`cfg(target_os = "macos")` so the Mac app is unchanged:
- links open with `xdg-open` instead of `/usr/bin/open`;
- the page sees `window.__KWANT_APP__ = 'linux'` (the frontend treats `'mac'` and `'linux'` alike,
  `frontend/src/externalLinks.ts` `DESKTOP_APPS`);
- the tray shows the coloured app icon (`icons/32x32.png`), not the black menu-bar template, and
  the approval count next to it shows only where the panel draws AppIndicator labels;
- a second launch from the app menu focuses the running window (`tauri-plugin-single-instance`,
  over D-Bus; without a session bus it starts normally), the way a Dock click does on macOS;
- `$SHELL` falls back to `/bin/bash` for the login PATH, and the log is `~/.kwnt/desktop-app.log`.

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

## Build on Linux (x86_64, beta)

On Debian/Ubuntu (22.04 or newer):

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf
(cd ../../frontend && npm ci && npm run build)   # kwnt embeds frontend/dist
npx @tauri-apps/cli@2 build --bundles deb,appimage
```

`tauri.linux.conf.json` (merged over `tauri.conf.json` on Linux only) sets the targets, the .deb's
`Depends` (python3 and python3-venv for the Laya router; Tauri adds WebKitGTK, GTK and the tray
library itself) and `Recommends` (zenity for the folder picker, xdg-utils for `xdg-open`). The
packages land in `target/release/bundle/{deb,appimage}/`. `.github/workflows/linux.yml` builds both
on ubuntu-22.04 (the AppImage's glibc floor) for pull requests, signs the updater artifacts with a
throwaway key and checks them, installs the .deb and opens the app and the AppImage on a virtual
display. Linux isn't released yet: it will ship in the same `mac-v*` GitHub release as the Mac app,
so the one `latest.json` serves both.

Both packages carry AppStream metainfo, `linux/si.kwant.kwant.appdata.xml`, installed as
`/usr/share/metainfo/si.kwant.kwant.appdata.xml`. linux.yml stamps its `<release>` entry with the
version it builds. The AppImage carries the update information
`gh-releases-zsync|alexsergivan|kwant|latest|kwant_x86_64.AppImage.zsync`, set through
`LDAI_UPDATE_INFORMATION` before Tauri signs it. The release ships that `.zsync`, which linux.yml
makes over the AppImage's exact bytes. linux.yml's "catalog checks" job validates the metainfo, the
desktop file, the update information and the `.zsync`, the same checks the AppImage catalog runs.

## Releases and the updater key

A release commit sets one version in `apps/mac/Cargo.toml`, `apps/mac/tauri.conf.json` and the
root `Cargo.toml` (kwnt's own version, shown by `kwnt --version` and `/api/health`), with both
`Cargo.lock` files. `tests/version_test.rs` fails when they disagree, and mac.yml refuses a
`mac-v*` tag that doesn't match all three.

`mac.yml` signs the updater bundle on `mac-v*` tags with the repository secrets
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, checks the signature
(`scripts/minisign_verify.py`) and, for a full release, publishes `latest.json` built by
`scripts/mac_latest_json.py` from the version's `## [Mac x.y.z]` CHANGELOG section. Pull requests
sign with a throwaway key, so they never see the secret.

The public half of the key is built into every app. If the private key is lost, installed apps can
never update themselves again and everyone has to download the next version by hand, so keep a
backup of the key and its password outside GitHub. Replacing the key works the same way: ship one
release signed with the old key that carries the new public key.
