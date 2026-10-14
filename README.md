# 🕹️ KWNTS: 8-Bit AI Agent Platform in Rust

> Inspired by **Grok Bot** & **OpenMausBot**. Built with **Rust**, **Axum**, and **Laya**.

Kwants is a high-performance, self-hosted AI agent platform packaged as a **single, standalone 10 MB binary** with an embedded retro 8-bit arcade Web UI and SQLite storage. Every agent is a **Kwant**—a specialized autonomous companion with its own personality, 8-bit sprite, stats (HP, MP/Mana context gauge, Level), model provider, and developer abilities.

---

## ⚡ Key Highlights

- **Single-Binary Zero-Dependency Deploy**: No Node.js or Docker required in production. One compiled native binary (`kwnt`) serves both the Tokio/Axum asynchronous engine and the embedded 8-bit React web application in under 25 MB of RAM on your laptop or a VPS.
- **Sub-50ms Intelligent Routing with Laya**: By default, task-based model selection queries [convaiinnovations/laya](https://huggingface.co/convaiinnovations/laya) over its Jev-compatible `/v1/systemone` API in ~33ms, with seamless offline heuristic fallback.
- **Hybrid Driver Fleet**: Run direct cloud & local APIs (xAI Grok, Anthropic Claude, OpenAI, OpenRouter, local Ollama) or supervise host CLI agents (`claude`, `codex`, `grok` CLIs).
- **Human-in-the-Loop RPG Permission Broker**: Risky operations (shell execution, file writes) trigger retro RPG encounter cards: `[ALLOW ONCE]`, `[ALWAYS ALLOW]`, `[DENY]`.
- **Authentic 8-Bit Arcade Aesthetics**: Pixel fonts (*Press Start 2P*, *Silkscreen*), toggleable CRT scanlines, animated pixel avatars, and procedural chiptune sound effects synthesized in pure code via the **Web Audio API** (zero audio files needed).

---

## 🚀 Quickstart

### 1. Run with Cargo
```bash
# Clone and run locally
cargo run -- serve
```
Open your browser to: **http://127.0.0.1:8799**

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

When a Kwant's model is set to `"auto"` (the default), `kwnt` queries Laya's Jev-compatible non-autoregressive decision model on `/v1/systemone`:

```bash
# Start Laya service sidecar (optional)
pip install "laya[serve]"
laya-serve --port 8000
```

If Laya is offline, `kwnt` automatically engages a local heuristic router with zero interruption.

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
