# Browser service and VPS hosting

Kwnt runs one long-lived Chromium container per installation, with an isolated browser context for each bot, channel, or team. The web client connects to Kwnt over a same-origin WebSocket. Kwnt relays CDP screencast frames and input; the client never connects to Chromium directly.

Clicks, text, keys, and scroll events are acknowledged independently of frames. There is no 150 ms sleep or screenshot request in the streaming input path. Frames are acknowledged after image decoding; slow viewers do not accumulate an unlimited frame queue. Closing the pane stops its stream.

Take Control obtains an exclusive server-side lease. Bot navigation and selector actions return a conflict while the person holds it; other viewers cannot acquire it. Returning control, closing the pane, or disconnecting releases the lease. The connection has heartbeat and slow-client timeouts. This pauses browser actions, not the entire agent conversation or host CLI process.

## Computer mode: local or VM

Kwnt runs bots' browser and shell commands in one of two places, chosen in **Settings → Computer** or with the browser panel's mode toggle:

- **Local computer** (`local`, the default on macOS): Kwnt runs your installed Chrome or Chromium without a window (`--headless=new`, 1280×800) with its own profile, `<data dir>/local-chrome`, and shows it live in the app's browser panel, the same way as the VM. To sign in or solve a CAPTCHA yourself, use **Open as window** in the panel: Kwnt closes the windowless Chrome and opens the same profile as a real window, with each bot's page reopened there (one profile can't be open twice). **Show it here instead**, or closing that window, brings it back into the panel. Saved sign-ins carry over; cookies that only last for a browser session do not. The window choice is not saved, so a restart goes back to the panel. Switching to VM closes the windowless Chrome; a window you opened stays open. It never uses your everyday Chrome profile. Each bot gets its own Chrome window (headless Chrome only paints the front tab of a window, so tabs would stall the other bots' screenshots). Sign-ins are kept in the kwant profile and shared by all bots. The debugging port is chosen by Chrome and listens on 127.0.0.1 only. Shell commands run on your computer in the bot's working folder, and **every command asks for your approval first**. "Always allow" applies to that one command; it is not remembered. Docker is not needed.
- **VM (browser and built-in shell isolated)** (`vm`, the default everywhere else, including a VPS): the Docker browser and execution containers described below. Only the browser and kwnt's own `execute_shell` run in containers. Claude, Codex and Grok still run on this computer, with their own file and shell tools.

Switching takes effect at once, without a restart. Kwnt disconnects from the old browser: the VM container and a local Chrome window keep running, and switching back reuses them; only the windowless local Chrome is closed, as nothing would show it. Each mode keeps its own saved pages (`browser-sessions-local.json` and `browser-sessions.json`). If the local Chrome was closed, the next browser action starts it again. Kwnt does not close Chrome when it stops, and reuses a running one on the next start.

Precedence: a choice saved in Settings wins; otherwise `KWNT_COMPUTER_MODE=local|vm`; otherwise the platform default. `KWNT_CHROME_PATH` names the browser binary local mode launches. Without it, Kwnt looks for Google Chrome, Chromium or Chrome Canary in `/Applications` and `~/Applications` on macOS, then for `google-chrome`, `google-chrome-stable`, `chromium`, `chromium-browser` or `chrome` on `PATH`.

A mode that cannot run here is refused with the reason: local needs a Chrome binary; only opening it as a window needs a display (always present on macOS; `DISPLAY` or `WAYLAND_DISPLAY` on Linux). VM needs Docker, or a `KWNT_BROWSER_CDP_URL` for the browser.

API (same authentication as the other settings routes):

```text
GET /api/settings/computer
→ {"mode":"local"|"vm","available":{"local":bool,"vm":bool},"reasons":{"local":string|null,"vm":string|null},
   "isolates":["browser","kwnt_shell"],   // what vm mode runs in containers; "cli_agents" once the CLIs move
   "window":{"visible":bool,"available":bool,"reason":string|null}}

PUT /api/settings/computer  {"mode":"local"|"vm"}
→ the same shape after switching; 400 {"error":"<reason>"} for an unknown or unavailable mode

PUT /api/settings/computer/window  {"visible":bool}
→ the same shape after restarting Chrome as a window (true) or back in the app (false);
  400 {"error":...} outside local mode or with no display for a window; 502 if Chrome could not restart
```

Local mode gives up isolation: commands run with your user's access to your files, and the debugging port can be reached by other programs on the same computer while the kwant Chrome is running. Use VM mode for untrusted browsing and shell work. VM mode does not contain the Claude, Codex or Grok CLIs, which run on this computer in both modes.

## VM mode on a Mac or Linux VPS

Install Docker and start Kwnt normally. The first browser request builds the embedded browser image and starts a private container. Neither a desktop session nor a display server is required on the VPS. Subsequent requests reuse Chromium. When an update ships a new browser image tag, the first browser request builds it and replaces kwant's browser container, so the bots' open pages close once. The managed debugging port binds only to 127.0.0.1.

The container never restarts on its own (`--restart no`); kwant starts it on the next browser request. It carries the labels `kwnt.browser=1` and `kwnt.root=<absolute data folder>`. kwant stops it when it shuts down, and a running kwant looks for labelled browser containers at startup and every 10 minutes:

- a container whose data folder no longer exists is removed (each data folder gets its own container, so a deleted test or eval folder would otherwise leave one behind);
- a running container nobody has used for 30 minutes is stopped. Set `KWNT_BROWSER_IDLE_MINUTES` to change the limit. Use is recorded in `<data folder>/browser-last-used`.

The browser profile lives on the container's tmpfs, so stopping or removing it loses only the open pages; each bot's cookies are kept in `<data folder>/browser-cookies/`. A container made by an older kwant (no labels, `--restart unless-stopped`) is replaced the next time that kwant opens the browser. Containers from data folders no kwant will open again need a manual `docker rm`.

Prebuild images to avoid first-use latency:

```bash
docker build -t kwnt-browser:v2 -f docker/browser.Dockerfile docker
docker build -t kwnt-executor:v1 -f docker/executor.Dockerfile docker
./target/release/kwnt serve --host 127.0.0.1 --port 8799
```

Browser image recipes are embedded in the binary, so automatic builds work even when the VPS has only the binary. Build and run each image for the host CPU architecture; the Debian packages support amd64 and arm64.

The managed browser has a shared 1.5 GiB memory limit, rather than a separate Chromium allocation for each bot. This is a limit, not reserved memory. Actual memory and frame throughput depend on websites and concurrent sessions. Browser contexts separate cookies and local storage; they are not OS isolation between mutually untrusted customers.

## Existing/private browser service

The repository's `docker-compose.yml` already does this: its `browser` service builds `docker/browser.Dockerfile` (read-only, all capabilities dropped, 1.5 GiB memory limit, CDP reachable on the compose network only) and kwnt gets `KWNT_BROWSER_CDP_URL=http://browser:9222`. `kwnt doctor` checks that the service answers.


Set the server-side CDP HTTP endpoint when an operator manages Chromium separately:

```bash
KWNT_BROWSER_CDP_URL=http://browser:9222 ./kwnt serve --host 0.0.0.0 --port 8799
```

For a host binary, use a loopback-published service URL instead of a Docker DNS name. For a containerized Kwnt process, the service name must resolve on its private Docker network. CDP must remain private; it grants full browser control. The browser service is dedicated to Kwnt and must not be a person's normal browser profile.

## Reverse proxy

Expose Kwnt behind your existing authenticated HTTPS proxy. Kwnt currently has no built-in user authentication or tenant authorization; protect **all** its routes, including WebSocket upgrades. Forward the original Host header because the stream validates Origin against Host. The frontend automatically uses wss on HTTPS and uses the public domain and port, not localhost.

Example nginx location inside an HTTPS server with authentication already configured:

```nginx
location / {
    proxy_pass http://127.0.0.1:8799;
    proxy_http_version 1.1;
    proxy_set_header Host $http_host;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_buffering off;
    proxy_read_timeout 90s;
}
```

Only the HTTPS port needs external access. Do not publish the CDP port or Docker socket publicly. Shell tools require Docker access on the server; providing a browser URL alone does not provide an execution backend.

## Sessions and migration

Context and page IDs are saved under the data directory, so restarting Kwnt reconnects to existing browser sessions while Chromium remains running. Restart Browser disposes only the selected context. Restarting Chromium loses context cookies/local storage; saved vault credentials remain on disk. Existing per-sandbox Chromium profiles are not imported into shared contexts; sign in again as needed. This release does not persist incognito cookies across a Chromium restart.

Execution containers are separate and start only when the sandbox execute_shell tool is used. They contain shell/development utilities without Chromium. Existing host CLI drivers still run on the host; they are not converted into container-isolated agents by this change. Old kwnt-sandbox containers are not deleted automatically. Review and stop obsolete containers once their work is no longer needed.

## Validation

```bash
cargo test --test browser_stream_test --test browser_test --test server_test --test sandbox_tools_test
cd frontend
npm test
npm run build
```

The mock CDP suite covers context separation/reuse, recovery after browser restart, streamed frames, frame acknowledgement, input ordering, no screenshot per input, control ownership/release, and cross-origin rejection. Real Chromium/container validation additionally requires a functioning Docker backend.

After building both images, run the opt-in live check:

```bash
cargo test --test browser_stream_test live_chromium_stream_and_isolated_sessions -- --ignored --nocapture
```

This uses disposable contexts, verifies real mouse/text input and control release, and runs a Python command in the separate execution container. On the local arm64 validation run, click + type + click acknowledgements took 95 ms in total; this is a local smoke-test measurement, not a VPS latency guarantee.

The running service was also checked through a temporary local HTTPS reverse proxy: WSS frames, control acquisition/release, and HTTP 409 rejection of bot navigation during human control all passed. An actual remote VPS deployment was not part of this local validation.
