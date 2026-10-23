# kwant

**Your own team of AI bots, in one self-hosted binary.**

[![CI](https://github.com/alexsergivan/kwant/actions/workflows/ci.yml/badge.svg)](https://github.com/alexsergivan/kwant/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Docs](https://img.shields.io/badge/docs-kwant.si-orange.svg)](https://kwant.si/docs/quickstart)

kwant gives you a team of bots that share chats, hand work to each other with an @mention, keep working on a schedule and wait for your OK before anything risky. Keep separate spaces for work and home, each with its own bots and accounts. Run it on your laptop or a server, with the Claude, Codex or Grok CLI you already use, or your own API keys.

It's written in Rust (Tokio, Axum, SQLite) with a React web app embedded in the binary.

![A kwant channel: Designer answers a question about empty states, then @mentions QA, and QA checks it on a 360 pixel screen.](docs/media/handoff.webp)

**Website and docs: [kwant.si](https://kwant.si)**

---

## Why kwant

- **A team, not a chat tab.** Each bot has its own role, voice ("soul") and model. Bots @mention each other to hand work on, a team lead runs short discussions and posts one summary, and the [Laya](#laya-model-router) router picks who answers and with which model and effort.
- **Spaces.** Separate spaces for work, a side project and home. Each has its own bots, teams, channels, routines, accounts, approval rule and hiring rule.
- **Every account you have.** Several accounts per engine: Claude, Codex or Grok CLI logins, Anthropic, OpenAI or xAI keys, and your own OpenAI- or Anthropic-compatible server. Set a default and a priority order. When an account runs out of usage, the turn moves to the next one and the chat says so. You can pin a bot to one account.
- **Nothing risky without your OK.** Shell commands, file edits outside the bot's folder and tool calls show up as approval cards: *Allow once*, *Always allow* or *Deny*. Per space or per bot: ask every time, ask only when risky, or don't ask.
- **Works while you're away.** Routines on a schedule (bots can set them up when you ask, with an Undo button), follow-ups bots schedule for themselves, and a *While you were away* page with what's waiting on you, what finished and what failed.
- **Loose ends and letters.** kwant reads your chats and, if you add them, an IMAP inbox and your GitHub issues and pull requests, read-only, and finds what you started and never finished. Photograph a bill or a form and it pulls out the sender, the ask and the deadline and drafts a reply. It never sends anything.
- **Parallel Universe.** Compare up to three ways to deploy an app with local container tests and dated price estimates. Every number is tagged as verified, an estimate or speculation. No cloud account and no spending.
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

More in the [quickstart](https://kwant.si/docs/quickstart) and [install](https://kwant.si/docs/install) docs.

---

## Self-hosting on a VPS

### The binary

`kwnt` is self-contained: SQLite and the web app are inside it.

```bash
scp target/release/kwnt user@your-vps:/usr/local/bin/kwnt
ssh user@your-vps "kwnt serve --host 0.0.0.0 --port 8799"
```

On your own computer there is nothing to sign in to. From another machine, open the link `kwnt serve` prints at startup once (`http://<this-host>:8799/#kwnt_token=...`, with `<this-host>` swapped for the server's address); the browser stays signed in for a year. Lost it? `kwnt token` prints it again. Through `ssh -L 8799:127.0.0.1:8799` no sign-in is needed. The link signs in as you, so anyone who can read the server's output can use it. Put a TLS reverse proxy in front: see [browser service and VPS hosting](docs/browser-service.md).

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

### Docker Compose

`docker-compose.yml` runs kwnt with a Laya image that has the model baked in, so the server never downloads it at runtime:

```bash
docker compose up -d --build                  # UI on http://127.0.0.1:8799
docker compose logs kwnt | grep kwnt_token    # open that link once, with <this-host> as 127.0.0.1
```

Data persists in the `kwnt-data` volume. `docker compose exec kwnt kwnt token` prints the sign-in link again.

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

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in kwant by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
