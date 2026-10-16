# ⚡ KWNTS: Multi-Agent AI Platform in Rust

> Inspired by **Grok Bot** & **OpenMausBot**. Built with **Rust**, **Axum**, and **Laya**.

Kwants is a high-performance, self-hosted multi-agent AI platform packaged as a **single, standalone binary** with an embedded sleek dark-mode Web UI and SQLite storage. Every agent is a **Kwant**—a specialized autonomous companion with its own persona (`Architect`, `TechLead`, `DevOps`), model provider routing, and developer capabilities.

---

## ⚡ Key Highlights

- **Single-Binary Zero-Dependency Deploy**: No Node.js or Docker required in production. One compiled native binary (`kwnt`) serves both the Tokio/Axum asynchronous engine and the embedded modern React web application in under 25 MB of RAM on your laptop or a VPS.
- **Sub-50ms Intelligent Routing with Laya**: By default, task-based model selection queries [convaiinnovations/laya](https://huggingface.co/convaiinnovations/laya) over its `/v1/systemone` API in ~33ms, with seamless offline heuristic fallback and dynamic effort-level selection.
- **Hybrid Driver Fleet**: Run direct cloud & local APIs (xAI Grok, Anthropic Claude, OpenAI, OpenRouter, local Ollama) or supervise host CLI agents (`claude`, `codex`, `grok` CLIs).
- **Kwant-to-Kwant Handoffs**: Every kwant knows its teammates in the current channel or team. When a reply @mentions a teammate (or `@everyone`), that teammate answers right after in the same chat, up to 4 handoffs per message.
- **Moderated Team Discussions**: When a team question needs several perspectives, the lead opens with a short take (or first asks you a clarifying question and waits), teammates respond to each other by name in under ~120 words or pass if they have nothing to add, and the lead closes with one summary. The back-and-forth folds into a collapsible "Team discussed" thread so the chat shows the answer.
- **Persona Presets & Avatars**: Create agents from ready-made personas grouped into **Tech** (Product, Designer, Frontend, Backend, QA, Security, SRE, Data, Tech Writer) and **Non-tech** (Research, Marketing, Copywriter, Sales, Finance, Legal, Support, People, Coach, Tutor), or start blank, and pick from sixteen pixel avatar styles. The chosen avatar always wins over hints in the agent's name.
- **Editable Agents**: Hover any agent in the sidebar (team members or the Agent Fleet) and click the pencil, or use **Edit** in a direct chat header, to change its name, title, avatar, preferred model, or persona. Renames carry over to the agent's past messages.
- **Human-in-the-Loop Permission Broker**: Security-critical operations (shell execution, workspace file edits) require user approval via modern security cards: `[Allow Once]`, `[Always Allow]`, `[Deny]`.
- **Sleek Modern Grok Bot Interface**: Dark zinc aesthetics (`zinc-950`), clean typography (`Inter`, `JetBrains Mono`), collapsible reasoning disclosures (`Thought for Xs • Laya [Grok] • Low Effort`), role-based SVG/gradient avatars, and subtle modern micro-interaction click & completion sounds synthesized via the Web Audio API.

---

## 🚀 Quickstart

### 1. Run with Cargo
```bash
# Clone and run locally
cargo run -- serve
```
Open your browser to: **http://127.0.0.1:8799**

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

### Systemd Service (`/etc/systemd/system/kwnt.service`)
```ini
[Unit]
Description=Kwants 8-Bit AI Agent Platform
After=network.target

[Service]
Type=simple
User=root
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
```

The `laya` image (`docker/laya.Dockerfile`) uses CPU PyTorch and preloads the checkpoints listed in the `LAYA_MODELS` build arg. kwnt runs with `KWNT_LAYA_MANAGED=0` and `LAYA_URL=http://laya:8000`; data persists in the `kwnt-data` volume. Put a TLS reverse proxy in front as described in [browser service and VPS hosting](docs/browser-service.md).

---

## 🎮 The Starter Party

| Kwant | Title | Sprite | Specialty |
|---|---|---|---|
| **CyberMage** | Arcane Architect | `cyber_mage` | Architecture, refactoring, systems thinking |
| **PixelNinja** | Swift Blade | `pixel_ninja` | Fast algorithms, clean functions, unit testing |
| **TerminalKnight** | Guardian of the Shell | `terminal_knight` | Bash scripts, devops, file safety |

You can summon new Kwants with custom personas and sprites via the **+ SUMMON KWANT** button in the UI.

---

## 🧪 Testing

```bash
# Run all 14 integration and unit tests
cargo test
```
