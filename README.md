# ⚡ KWNTS: Multi-Agent AI Platform in Rust

> Inspired by **Grok Bot** & **OpenMausBot**. Built with **Rust**, **Axum**, and **Laya**.

kwant is your team of AI bots: a self-hosted platform packaged as a **single, standalone binary** with an embedded dark-mode web app and SQLite storage. Every bot has its own persona (a fresh install starts with `Architect`, `TechLead` and `DevOps`), picks the right model for each message, and can work in its own folder, shell and browser.

---

## ⚡ Key Highlights

- **Single-Binary Deploy**: The built binary (`kwnt`) needs no Node.js: it serves both the Tokio/Axum engine and the embedded React web app in under 25 MB of RAM on your laptop or a VPS. Docker is only needed for VM mode and the bots' shared browser.
- **Sub-50ms Intelligent Routing with Laya**: By default, task-based model selection queries [convaiinnovations/laya](https://huggingface.co/convaiinnovations/laya) over its `/v1/systemone` API in ~33ms, with seamless offline heuristic fallback and dynamic effort-level selection.
- **Hybrid Driver Fleet**: Run direct cloud & local APIs (xAI Grok, Anthropic Claude, OpenAI, OpenRouter, local Ollama) or supervise host CLI agents (`claude`, `codex`, `grok` CLIs).
- **Agent-to-Agent Handoffs**: Every agent knows its teammates in the current channel or team. When a reply @mentions a teammate (or `@everyone`), that teammate answers right after in the same chat, up to 4 handoffs per message.
- **Moderated Team Discussions**: When a team question needs several perspectives, the lead opens with a short take (or first asks you a clarifying question and waits), teammates respond to each other by name in under ~120 words or pass if they have nothing to add, and the lead closes with one summary. The back-and-forth folds into a collapsible "Team discussed" thread so the chat shows the answer.
- **Persona Presets & Avatars**: Create agents from ready-made personas grouped into **Tech** (Product, Designer, Frontend, Backend, QA, Security, SRE, Data, Tech Writer) and **Non-tech** (Research, Marketing, Copywriter, Sales, Finance, Legal, Support, People, Coach, Tutor) and **Lifestyle** (Travel Planner, Chef, Fitness Trainer, Event Planner, Home & DIY, Language Partner, Budget Buddy, Wellbeing Companion), or start blank, and pick from twenty-two pixel avatar styles. The chosen avatar always wins over hints in the agent's name.
- **Agent Souls**: Besides its persona (what it does), every agent has an optional **Soul**: its personality, voice, values, and quirks. The soul is added to the system prompt, so it shapes how the agent speaks. It is not used for routing. Every preset and built-in agent comes with one.
- **Editable Agents**: Hover any agent in the sidebar (team members or the Agent Fleet) and click the pencil, or use **Edit** in a direct chat header, to change its name, title, avatar, preferred model, persona, or soul. Renames carry over to the agent's past messages.
- **Human-in-the-Loop Permission Broker**: Security-critical operations (shell execution, workspace file edits) require user approval via modern security cards: `[Allow Once]`, `[Always Allow]`, `[Deny]`.
- **Sleek Modern Grok Bot Interface**: Dark zinc aesthetics (`zinc-950`), clean typography (`Inter`, `JetBrains Mono`), collapsible reasoning disclosures (`Thought for Xs • Laya [Grok] • Low Effort`), role-based SVG/gradient avatars, and subtle modern micro-interaction click & completion sounds synthesized via the Web Audio API.

---

## 🚀 Quickstart

### What you need first
- **Rust** (stable), installed with [rustup](https://rustup.rs).
- **Node.js 20 or newer** with npm, to build the web app. The finished binary doesn't need it.
- **At least one AI account** for your bots to run on. Any one of these is enough:
  - the [`claude`](https://docs.anthropic.com/en/docs/claude-code) CLI, installed. You don't need to sign it in first: kwant has a **Sign in with Claude** button;
  - the `codex` or `grok` CLI, signed in on the same machine (`codex login`, `grok login`);
  - an Anthropic, OpenAI or xAI API key, which you add in the app under **Space settings → Accounts**.
- Optional: **Docker** for VM mode and the bots' browser, and [`uv`](https://docs.astral.sh/uv/) or `python3` for the Laya router (see below).

### 1. Run from a fresh clone
```bash
git clone https://github.com/alexsergivan/kwant.git
cd kwant
make run          # builds the web app (frontend/dist), then runs `cargo run -- serve`
```
Open your browser to: **http://127.0.0.1:8799**. If no account works yet, kwant shows a **Get ready** banner with an **Add an account** button.

`cargo run -- serve` on its own only works once `frontend/dist` exists, because the binary embeds it. After the first `make run` (or `make frontend`) you can use either.

**First start takes a few minutes:** kwant installs its Laya router and downloads its ~420 MB model in the background (see [Laya](#-laya-model-router-convaiinnovationslaya)). Bots answer meanwhile, picked by keyword. To skip it, run `cargo run -- serve --no-laya`.

Bot browsing uses a shared headless Chromium service with live WebSocket frames and separate sessions. Code execution containers start independently. See [browser service and VPS hosting](docs/browser-service.md) for Docker setup, HTTPS proxy configuration, control handoff, and session persistence limits.

### 2. Build Single Release Binary
```bash
make build
# Output located at: target/release/kwnt (~10MB)
./target/release/kwnt serve
```

---

## 🌐 Self-Hosting on a VPS

Because `kwnt` is a completely self-contained binary embedding SQLite and the frontend assets, deploying to any VPS takes seconds:

```bash
# 1. Copy binary to your VPS
scp target/release/kwnt user@your-vps-ip:/usr/local/bin/kwnt

# 2. Run on VPS
ssh user@your-vps-ip "kwnt serve --host 0.0.0.0 --port 8799"
```

Requests from other machines need the API token: run `kwnt token` on the VPS (or `docker compose exec kwnt kwnt token` with Docker Compose) and open `http://your-vps-ip:8799/#kwnt_token=<token>` once. Through `ssh -L 8799:127.0.0.1:8799` no token is needed. See [LLM profiles and API access](docs/llm-profiles.md).

### Systemd Service (`/etc/systemd/system/kwnt.service`)
```ini
[Unit]
Description=Kwant AI agent platform
After=network.target

[Service]
Type=simple
# A dedicated user (useradd --system --create-home --home-dir /var/lib/kwnt kwnt); sign the
# claude/codex/grok CLIs in as this user, or add API keys in the app
User=kwnt
ExecStart=/usr/local/bin/kwnt serve --host 0.0.0.0 --port 8799
Restart=always
Environment=KWNT_DATA_DIR=/var/lib/kwnt
Environment=LAYA_URL=http://127.0.0.1:8000
# kwnt installs and supervises Laya under KWNT_DATA_DIR/laya; set KWNT_LAYA_MANAGED=0 to run your own
Environment=XAI_API_KEY=your_xai_key_here
Environment=ANTHROPIC_API_KEY=your_claude_key_here

[Install]
WantedBy=multi-user.target
```
Enable and start:
```bash
systemctl daemon-reload
systemctl enable --now kwnt
```

---

## 🔮 Laya Model Router (`convaiinnovations/laya`)

Laya picks which agent answers in a channel and which model and effort each agent uses when its model is `"auto"`. It runs as a small Python service speaking `/v1/systemone`.

**`kwnt serve` manages Laya for you.** If nothing answers at `laya_url` (default `http://127.0.0.1:8000`), kwnt:

1. installs `laya[serve]==0.4.8` into `~/.kwnt/laya/venv` on first run, using [`uv`](https://docs.astral.sh/uv/) when present (it fetches Python too) or `python3 -m venv` otherwise;
2. starts `laya-serve` on the configured port, downloading the English checkpoint (~420 MB) into `~/.kwnt/laya/hf` on first start;
3. restarts it with backoff if it exits, and stops it when kwnt shuts down.

The header shows the router status (green online, amber installing or loading, red offline), and `GET /api/laya/status` returns it with the failure reason. Logs are in `~/.kwnt/laya/laya.log` and `install.log`. While Laya is unavailable, kwnt routes by keyword and says so in the routing card.

| Setting | `config.toml` | Environment | Default |
|---|---|---|---|
| Laya URL | `laya_url` | `LAYA_URL` | `http://127.0.0.1:8000` |
| Manage a local sidecar | `laya_managed` | `KWNT_LAYA_MANAGED=0` to disable, or `kwnt serve --no-laya` | `true` |
| Checkpoints to preload | `laya_models` | `KWNT_LAYA_MODELS` | `english` |
| Use an existing `laya-serve` binary | | `KWNT_LAYA_SERVE_BIN` | |

kwnt only starts Laya for loopback URLs. If Laya already answers, for example from Docker, kwnt uses it as is.

**Networks that block Hugging Face.** The first start needs `huggingface.co`. If a proxy blocks it, start kwnt once on an open network, or copy an existing `~/.kwnt/laya/hf` from another machine; after that Laya starts offline.

---

## 🐳 Docker Compose (VPS)

`docker-compose.yml` runs kwnt with a Laya image that has the weights baked in, so the server never downloads them at runtime:

```bash
docker compose up -d --build   # UI on http://127.0.0.1:8799
docker compose exec kwnt kwnt token   # open http://127.0.0.1:8799/#kwnt_token=<token> once
```

The `laya` image (`docker/laya.Dockerfile`) uses CPU PyTorch and preloads the checkpoints listed in the `LAYA_MODELS` build arg. kwnt runs with `KWNT_LAYA_MANAGED=0` and `LAYA_URL=http://laya:8000`; data persists in the `kwnt-data` volume. Put a TLS reverse proxy in front as described in [browser service and VPS hosting](docs/browser-service.md).

---

## 🎮 The Starter Team

A fresh install's Default space starts with three bots in #general, on the `@core-engineering` team:

| Bot | Title | Avatar | Specialty |
|---|---|---|---|
| **Architect** | Systems Architect | `architect` | Architecture, software design, multi-file engineering |
| **TechLead** | Staff Engineer & Prototyper | `tech_lead` | Fast reasoning, prototyping, clear explanations |
| **DevOps** | Systems & Security Engineer | `devops` | Shell, servers, containers, security |

Make more with **New bot**: pick a ready-made persona (Tech, Non-tech, Lifestyle) or start blank, and choose a name, avatar, persona and soul.

---

## 🧪 Testing

```bash
cargo test                      # Rust unit and integration tests (needs frontend/dist: run `make frontend` first)
cd frontend && npm test         # web app tests
```
