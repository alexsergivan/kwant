# kwant

**Your own team of AI bots, in one self-hosted binary.**

[![CI](https://github.com/alexsergivan/kwant/actions/workflows/ci.yml/badge.svg)](https://github.com/alexsergivan/kwant/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](#license)
[![Docs](https://img.shields.io/badge/docs-kwant.si-orange.svg)](https://kwant.si/docs/quickstart)

kwant gives you a team of bots that share chats, hand work to each other with an @mention and get it done on their own, on a schedule or while you're away. Keep separate spaces for work and home, each with its own bots and accounts. Run it on your laptop or a server, with the Claude, Codex or Grok CLI you already use, or your own API keys.

It's written in Rust (Tokio, Axum, SQLite) with a React web app embedded in the binary.

![A kwant channel: Designer answers a question about empty states, then @mentions QA, and QA checks it on a 360 pixel screen.](docs/media/handoff.webp)

**Website and docs: [kwant.si](https://kwant.si)** · **[Download for Mac](https://github.com/alexsergivan/kwant/releases/latest/download/kwant_universal.dmg)**

---

## Why kwant

- **A team, not a chat tab.** Each bot has its own role, voice ("soul") and model. Bots @mention each other to hand work on, a team lead runs short discussions and posts one summary, and the [Laya](#laya-model-router) router picks who answers and with which model and effort.
- **Spaces.** Separate spaces for work, a side project and home. Each has its own bots, teams, channels, routines, accounts, approval rule and hiring rule.
- **Every account you have.** Several accounts per engine: Claude, Codex or Grok CLI logins, Anthropic, OpenAI or xAI keys, and your own OpenAI- or Anthropic-compatible server. Set a default and a priority order. When an account runs out of usage, the turn moves to the next one and the chat says so. You can pin a bot to one account.
- **Works while you're away.** Routines on a schedule (bots can set them up when you ask, with an Undo button), follow-ups bots schedule for themselves, and a *While you were away* page with what's waiting on you, what finished and what failed.
- **Autonomous, with a safety net you control.** Ordinary work runs without stopping. On *Ask only when risky*, the default for new spaces, only risky steps such as `git push`, network calls, unknown commands or changes outside the bot's folder show an approval card with the exact command: *Allow once*, *Always allow* or *Deny*. Per space or per bot, switch to ask every time or don't ask.
- **Loose ends and letters.** kwant reads your chats and, if you add them, an IMAP inbox and your GitHub issues and pull requests, read-only, and finds what you started and never finished. Photograph a bill or a form and it pulls out the sender, the ask and the deadline and drafts a reply. It never sends anything.
- **Parallel Universe.** Compare up to three ways to deploy an app with local container tests and dated price estimates. Every number is tagged as verified, an estimate or speculation. No cloud account and no spending.
- **No telemetry.** kwant sends nothing to us, and its interface loads nothing from other servers. The one download you don't set up yourself is the [Laya](#laya-model-router) router on first start: its Python packages and its model from PyPI and Hugging Face (`--no-laya` turns it off). Everything else goes only to the AI providers and services you add.
- **And more:** memory and skills per bot (you approve new skills), bots that propose hiring a teammate, import a bot from a Claude Code subagent, a SKILL.md, a character card or a JSON export, search across a space's chats, MCP tool servers, a shared browser for bots, saved passwords with an optional Bitwarden store, and Local or VM computer mode.

See every feature, with screenshots, on [kwant.si/features](https://kwant.si/features).

<table>
  <tr>
    <td width="50%"><img src="docs/media/spaces.webp" alt="The space rail with three spaces, Trailhead, Pixel Quest and Home, each with its own icon and colour. Trailhead is open on #general with a weekly digest posted by a routine."><br><sub><b>Spaces</b>: each with its own bots, channels and routines.</sub></td>
    <td width="50%"><img src="docs/media/accounts.webp" alt="Space settings, Accounts tab. The Work login is the default, in use and ready. Personal Max is out of usage and back in 3 hours. This machine needs a sign-in."><br><sub><b>Accounts</b>: several per engine, with a default and fallbacks.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/approval.webp" alt="An approval card: TechLead wants to create a pull request on GitHub, with the full arguments shown and Deny, Allow Once and Always Allow buttons."><br><sub><b>Approvals</b>: see exactly what a bot wants to run.</sub></td>
    <td><img src="docs/media/routine.webp" alt="QA set up a routine when asked to check overnight checkout errors every morning: weekdays at 08:30, posting in the direct chat, with Undo, Edit and Pause buttons."><br><sub><b>Routines</b>: ask for "every morning" and get one, with Undo.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/away.webp" alt="While you were away. Waiting on you: a question with answer buttons. Finished: a handoff and a weekly digest. Failed or stuck: a price scan blocked with a 403 and a smoke test that stopped at 9 of 22."><br><sub><b>While you were away</b>: what's waiting, finished or failed.</sub></td>
    <td><img src="docs/media/loose-ends.webp" alt="Loose ends with Open, Waiting and Snoozed tabs. The top case is a letter from a water company, with the quote it's based on and a draft reply."><br><sub><b>Loose ends</b>: evidence and a draft. It never sends.</sub></td>
  </tr>
</table>

Screenshots come from a demo space with sample data.

---

## Quickstart

### What you need

- **Rust** (stable), installed with [rustup](https://rustup.rs).
- **Node.js 20 or newer** with npm, to build the web app. The finished binary doesn't need it.
- **At least one AI account.** Any one of these is enough:
  - the [`claude`](https://docs.anthropic.com/en/docs/claude-code) CLI, installed. You don't need to sign it in first: kwant has a **Sign in with Claude** button;
  - the `codex` or `grok` CLI, signed in on the same machine (`codex login`, `grok login`);
  - an Anthropic, OpenAI or xAI API key, which you add in the app under **Space settings → Accounts**.
- Optional: **Docker** for VM mode and the bots' shared browser, and [`uv`](https://docs.astral.sh/uv/) or `python3` for the Laya router.

`kwnt doctor` (`cargo run -- doctor` from a clone) checks all of these and prints the install command for whatever is missing on your machine; `kwnt setup` installs it (see [Without Docker](#without-docker-kwnt-setup)).

### Run from a fresh clone

```bash
git clone https://github.com/alexsergivan/kwant.git
cd kwant
make run          # builds the web app (frontend/dist), then runs `cargo run -- serve`
```

Open **http://127.0.0.1:8799**. If no account works yet, kwant shows a **Get ready** banner with an **Add an account** button. A fresh install starts with three bots in `#general`: Architect, TechLead and DevOps. Make more with **New bot**, from a ready-made persona or blank.

The binary embeds `frontend/dist`, so on a fresh clone `cargo build`, `cargo test` and `cargo run -- serve` fail to compile until you've run `make run` or `make frontend` once.

**The first start takes a few minutes:** kwant installs its Laya router and downloads its model (about 420 MB) in the background. Bots answer meanwhile, picked by keyword. To skip Laya, run `cargo run -- serve --no-laya`.

### Build a release binary

```bash
make build                 # web app + cargo build --release
./target/release/kwnt serve
```

### On a Mac: the app

[Download the Mac app](https://github.com/alexsergivan/kwant/releases/latest/download/kwant_universal.dmg) (a universal `.dmg` for Apple silicon and Intel) and drag kwant to Applications. It runs its own kwant on your Mac, with the same `~/.kwnt` data folder as `kwnt serve`, and stays in the menu bar when you close the window. Build it yourself from [`apps/mac`](apps/mac/README.md).

The app is signed ad hoc, not notarized by Apple, so macOS stops it the first time. Click **Done**, open **System Settings → Privacy & Security**, scroll down and click **Open Anyway**. On macOS 14 or earlier, right-click the app and choose **Open** instead.

### Install the web app

In Chrome or Edge, click **Install app** in the sidebar; in Safari on a Mac, **File → Add to Dock**; on a phone, kwant shows how. Browsers only install pages served over https or from `127.0.0.1`.

To get notifications on your phone with kwant closed, install it (on an iPhone, add it to the Home Screen first) and turn on **Settings → Notifications → Notify me on this device**. kwant then sends a Web Push notification for each kind you've ticked there, such as an approval or a question. It goes through your browser's push service (Google's for Chrome on Android, Apple's on an iPhone), encrypted so that service can't read it. Signing a browser out under **Signed-in devices** stops its notifications.

More in the [quickstart](https://kwant.si/docs/quickstart) and [install](https://kwant.si/docs/install) docs.

---

## Self-hosting on a VPS

### The binary

`kwnt` is self-contained: SQLite and the web app are inside it.

```bash
scp target/release/kwnt user@your-vps:/usr/local/bin/kwnt
ssh user@your-vps "kwnt serve --host 0.0.0.0 --port 8799"
```

On your own computer there is nothing to sign in to. From another machine, open the one-time link `kwnt serve` prints at startup (it works once, for 15 minutes; with `KWNT_ALLOWED_ORIGINS` set it is ready to open and comes with a QR code), or make a new one with `kwnt signin-link`. The browser stays signed in for a year. To add your phone or another browser later, open **Settings → Signed-in devices → Add a phone or browser** in a signed-in browser and scan the one-time QR code (it also works once, for 15 minutes); the same list lets you sign those browsers out. A new browser can also choose **Sign in from another device** on the sign-in screen: it shows a code for 10 minutes, and a signed-in browser shows **New device wants access** with Approve or Deny. Approve only a code that is on a screen you are holding. Prefer the token? On the server, as the user that runs kwnt, `cat ~/.kwnt/api-token` (or `$KWNT_DATA_DIR/api-token`, e.g. `sudo cat /var/lib/kwnt/api-token` with the systemd unit below) prints the token to paste (`kwnt token --raw` does the same; `./target/release/kwnt` if kwnt isn't on your PATH), and `kwnt token` prints a new one-time link like `kwnt signin-link`. Run them as the same user: as another user they fail, because that user can't read the token file (for the systemd unit use `sudo cat` as above). Each install has its own token, so one copied from your laptop won't open kwnt on the server. Or skip sign-in: `ssh -N -L 8799:127.0.0.1:8799 user@your-vps`, then open http://127.0.0.1:8799 on your computer (not with Docker, where requests always count as remote). A one-time link signs in as you, so keep it private until you've used it; the token never appears in the server's output. Put a TLS reverse proxy in front: see [browser service and VPS hosting](docs/browser-service.md).

<details>
<summary>A systemd unit</summary>

```ini
# /etc/systemd/system/kwnt.service
[Unit]
Description=kwant AI bot platform
After=network.target

[Service]
Type=simple
# A dedicated user (useradd --system --create-home --home-dir /var/lib/kwnt kwnt); sign the
# claude/codex/grok CLIs in as this user, or add API keys in the app
User=kwnt
ExecStart=/usr/local/bin/kwnt serve --host 0.0.0.0 --port 8799
Restart=always
Environment=KWNT_DATA_DIR=/var/lib/kwnt

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
systemctl enable --now kwnt
```

</details>

### VPS in 5 minutes (Docker Compose)

One command starts everything kwant needs: kwnt, **Laya** (picks which bot answers, model baked in) and a private **Chromium** for the bots (VM mode), with no Docker socket mounted. You need Docker with Compose and about 4 GB of free RAM (Chromium is capped at 1.5 GB).

```bash
git clone https://github.com/alexsergivan/kwant.git && cd kwant
KWNT_CLIS=claude,codex docker compose up -d --build   # any of claude,codex,grok; leave out for none
docker compose exec kwnt kwnt doctor                  # what works, and the fix for what doesn't
docker compose exec kwnt claude auth login            # sign a CLI in (codex login --device-auth, grok login --device-auth)
docker compose exec kwnt kwnt signin-link            # a one-time sign-in link and QR code (15 minutes)
```

Or skip the CLIs and add an Anthropic, OpenAI or xAI API key in the app under **Space settings → Accounts**. CLI sign-ins live in the `kwnt-home` volume and data in `kwnt-data`, so both survive rebuilds.

**HTTPS:** the UI is published on `127.0.0.1:8799` only. Put a TLS proxy in front, e.g. [Caddy](https://caddyserver.com) with this `Caddyfile`, and tell kwnt its address:

```
kwant.example.com {
	reverse_proxy 127.0.0.1:8799
}
```

```bash
echo KWNT_ALLOWED_ORIGINS=https://kwant.example.com >> .env && docker compose up -d
```

`docker compose exec kwnt kwnt signin-link` prints a one-time sign-in link for your domain and a QR code to scan with your phone. It works once, for 15 minutes; run it again for the next device. `docker compose exec kwnt kwnt token --raw` still prints the API token, as a fallback.

- **Image sizes** (measured): kwnt alone 0.2 GB; with `KWNT_CLIS=claude,codex,grok` 1.7 GB (the CLIs add about 0.9 GB, so they are off by default). Laya 3.2 GB, the browser 1.2 GB. Idle RAM: Laya about 1.7–2.3 GB, the browser about 0.4 GB, kwnt about 25 MB.
- **kwant's own `execute_shell` tool** runs in per-team Docker containers, so it isn't available inside this stack: that would need the host's Docker socket, which is root on the host. The CLIs' own shell tools work inside the kwnt container.
- **Upgrading from an older image:** kwnt now runs as uid 10001, not root. If it says `/data isn't writable`, run once: `docker compose run --rm -u root kwnt chown -R 10001:10001 /data`.

### Without Docker: `kwnt setup`

`kwnt setup` installs what kwant needs on this machine: Chromium or Chrome for the bots' browser, Laya (in kwant's data folder, with its own `uv` if you have no Python), and a CLI (`claude` unless you name others with `--clis`). It asks before each step (`--yes` skips the questions), never runs `sudo` itself (it prints the command for you instead), installs into your home folder where it can, skips Docker unless you pass `--docker`, and running it again changes nothing. `--dry-run` shows the plan. The app has the same checklist under **Setup**.

One line per platform, each followed by `kwnt doctor`:

| Platform | Install | Then |
|---|---|---|
| Mac app | Download the `.dmg` from [Releases](https://github.com/alexsergivan/kwant/releases) (test builds for now) and open it | Run `kwnt doctor` |
| Mac, command line | `git clone https://github.com/alexsergivan/kwant.git && cd kwant && make build && ./target/release/kwnt setup` (uses Homebrew when you have it) | Run `kwnt doctor` |
| Linux desktop or server | The same clone and `make build`, then `kwnt setup`, or `sh scripts/install-linux-deps.sh --clis claude` on Debian/Ubuntu | Run `kwnt doctor` |
| VPS with Docker | [Docker Compose](#vps-in-5-minutes-docker-compose) | Run `docker compose exec kwnt kwnt doctor` |

There's no prebuilt `kwnt` binary or Homebrew formula yet, so the command line needs a clone and a build (Rust and Node).

---

## Laya model router

[Laya](https://huggingface.co/convaiinnovations/laya) picks which bot answers in a channel, and which model and effort a bot uses when its model is `auto`. It runs as a small Python service, and **`kwnt serve` manages it for you**: on first run it installs `laya[serve]` into `~/.kwnt/laya/venv` (with `uv` when present, else `python3 -m venv`), starts it, and restarts it if it exits. The header shows its status. While Laya is unavailable, kwant routes by keyword and says so.

| Setting | `config.toml` | Environment | Default |
|---|---|---|---|
| Laya URL | `laya_url` | `LAYA_URL` | `http://127.0.0.1:8000` |
| Manage a local Laya | `laya_managed` | `KWNT_LAYA_MANAGED=0` to disable, or `kwnt serve --no-laya` | `true` |
| Checkpoints to preload | `laya_models` | `KWNT_LAYA_MODELS` | `english` |
| Use an existing `laya-serve` binary | | `KWNT_LAYA_SERVE_BIN` | |

The first start needs `huggingface.co`. Behind a proxy that blocks it, start kwant once on an open network or copy `~/.kwnt/laya/hf` from another machine.

---

## Documentation

- What changed in each release: [CHANGELOG.md](CHANGELOG.md)
- Website docs: [kwant.si/docs](https://kwant.si/docs/quickstart): quickstart, spaces, accounts, routines, approvals, loose ends, Parallel Universe, memory and skills, MCP.
- In this repo:
  - [Approvals](docs/approvals.md)
  - [Bot actions](docs/bot-actions.md)
  - [Browser service and VPS hosting](docs/browser-service.md)
  - [LLM profiles (accounts) and API access](docs/llm-profiles.md)
  - [MCP API](docs/mcp-api.md)
  - [Saved passwords](docs/saved-passwords.md)
  - [Loose ends (zombie hunter)](docs/zombie-hunter.md)
  - [Letters (camera to completed)](docs/camera-to-completed.md)
  - [Parallel Universe](docs/parallel-universe.md)

---

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and send a change, and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

```bash
make frontend                  # build the web app once (cargo test needs frontend/dist)
cargo test                     # Rust unit and integration tests
cd frontend && npm test        # web app tests
```

## Security

kwant runs commands and holds API keys and passwords, so please report vulnerabilities privately. See [SECURITY.md](SECURITY.md). Don't open a public issue for a security problem.

## License

kwant is open source under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only).

- **Free for everyone:** use it, change it and run it for yourself, your team or your company, at any size.
- **Share your changes if you serve them:** if you change kwant and let others use it over a network, for example as a hosted service, you must publish your changed source code under the AGPL too.
- **Commercial license:** if you want to sell kwant, host it or build it into your product without publishing your changes, you can buy a commercial license instead. See [COMMERCIAL.md](COMMERCIAL.md).

Code published before the switch, including the mac-v0.1.0 release, stays under MIT OR Apache-2.0.

Contributions are accepted under the terms in [CONTRIBUTING.md](CONTRIBUTING.md#license).
