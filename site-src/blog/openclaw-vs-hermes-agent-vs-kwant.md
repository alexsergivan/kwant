title: OpenClaw vs Hermes Agent vs kwant: self-hosted AI agents
description: A fair look at OpenClaw, Hermes Agent and kwant, three self-hosted AI agents. What each is built for, where each is the better pick, and how they install.
date: 2026-11-01
tags: openclaw, hermes-agent, kwant, self-hosted, comparison, ai-agents
slug: openclaw-vs-hermes-agent-vs-kwant
card: OpenClaw, Hermes Agent or kwant: which one for what?
draft: no
---

OpenClaw and Hermes Agent are the two names that come up most when people look for a self-hosted AI agent. kwant, the open-source AI agent team we build at kwant.si, is a much smaller project. This post compares the three using what each project says about itself, and it says plainly where OpenClaw or Hermes Agent is the better pick. We are one of the three, so treat our own paragraphs with that in mind, and check the linked sources.

Every fact about OpenClaw and Hermes Agent below comes from their own repositories and docs, read on November 1, 2026. Star counts and latest versions are from GitHub on the same day.

## The short answer

- You want your agent inside the chat apps you already use, with native apps on every platform: **OpenClaw**.
- You want an agent that builds up skills and memory from experience and can run on several kinds of backend, including serverless ones: **Hermes Agent**.
- You want a team of several bots on your own machine, with leads, hand-offs, approvals and a task board, and you're happy with the CLIs you already use: **kwant**.

## What each one is

**OpenClaw** describes itself as "an open-source AI assistant that runs on your own computer and meets you in the channels you already use: Discord, iMessage, Slack, Teams, Telegram, WhatsApp, and 20+ more, plus native apps for macOS, iOS, Android, Windows, and Linux." One Gateway is the local control plane, and it can run as a personal assistant on a laptop or as a shared team deployment. Models and agent harnesses (Claude, Codex, local models) are plugins you can swap. The project is developed in the open by the OpenClaw Foundation, an independent 501(c)(3), under the MIT licence. Latest release v2026.10.9 (October 29, 2026); 393,569 stars on GitHub. ([source](https://github.com/openclaw/openclaw), as of Nov 1, 2026)

**Hermes Agent** describes itself as "the self-improving AI agent built by Nous Research", with a built-in learning loop: it creates skills from experience, improves them during use, and searches its own past conversations. It has a built-in cron scheduler with delivery to any platform, can spawn subagents, and offers six built-in terminal backends (local, Docker, SSH, Modal, Daytona and Vercel Sandbox). It works with many model providers, including Nous Portal and OpenRouter. It installs on Linux, macOS, native Windows and Android (Termux), and there is also a Hermes Desktop app. MIT licence. Latest release v0.22.5 (October 23, 2026); 259,223 stars. ([source](https://github.com/NousResearch/hermes-agent), as of Nov 1, 2026)

**kwant** is an open-source (AGPL-3.0, with a commercial option) self-hosted team of AI bots in one binary. Bots share channels and teams and hand work to each other with @mentions. It runs on your laptop, a VPS or Docker, and has a Mac app and a web app that works on a phone. Latest release: Mac app 0.6.0 (November 1, 2026); 16 stars on GitHub. ([kwant.si](https://kwant.si/), [GitHub](https://github.com/alexsergivan/kwant), as of Nov 1, 2026)

## Comparison table

```compare
caption: As each project describes itself, checked November 1, 2026
| | OpenClaw | Hermes Agent | kwant |
| --- | --- | --- | --- |
| Built around | One assistant reached through chat channels | One self-improving agent with a learning loop | A team of bots with leads and hand-offs |
| How you talk to it | Messaging channels (Discord, iMessage, Slack, Teams, Telegram, WhatsApp and 20+ more), Control UI, CLI, TUI | TUI, Hermes Desktop, and one gateway for Telegram, Discord, Slack, WhatsApp, Signal and email | Web app, Mac app |
| From your phone | Native iOS and Android apps, or any of its chat channels | Its chat channels; Termux install on Android | The web app, in the phone's browser |
| Where it runs | Your computer (macOS, Linux, Windows); one Gateway can serve a team | Local, Docker, SSH, Modal, Daytona or Vercel Sandbox backends | Laptop, VPS or Docker; Mac app |
| Models | Swappable Claude, Codex or local models | Many providers (Nous Portal, OpenRouter, OpenAI, your own endpoint) | Claude, Codex or Grok CLI logins, API keys, local Ollama, or any OpenAI-compatible server |
| Scheduling | Built-in scheduler ("Automations") that can deliver to a chat channel or webhook | Built-in cron scheduler with delivery to any platform | Routines and cron rules, on your own machine |
| Approvals and safety | Exec approvals with allowlists; tools run on the host for the main session unless you configure sandboxing | Dangerous-command approval (manual, smart or off) and container isolation | Three approval modes (ask, smart, off); payments always ask; optional Flight School tiers |
| Licence | MIT | MIT | AGPL-3.0, commercial option |
| Latest release (Nov 1, 2026) | v2026.10.9 | v0.22.5 | Mac 0.6.0 |
| GitHub stars (Nov 1, 2026) | 393,569 | 259,223 | 16 |
```

## Pick OpenClaw if…

- You want to message your agent from WhatsApp, iMessage, Telegram or Slack. kwant's interface is its own web and Mac apps, so it isn't the pick for that.
- You want native apps on macOS, iOS, Android, Windows and Linux. kwant's desktop app is Mac only today.
- You want a very large community around the project. By GitHub stars, OpenClaw is far larger than kwant (393,569 against 16), and that is a real advantage for help, plugins and examples.

Check the [OpenClaw security guide](https://docs.openclaw.ai/gateway/security) before you connect other people: the README notes that tools run on the host for the main session unless you configure sandboxing.

## Pick Hermes Agent if…

- You want an agent that learns: skills created from experience, searchable past sessions, a model of you across sessions.
- You want to run it somewhere other than your own box, such as a Docker container, over SSH, or on Modal, Daytona or Vercel Sandbox.
- You want a built-in scheduler that delivers results to a chat platform.
- You're on Windows. Hermes Agent installs natively there; kwant doesn't have a Windows app yet.

kwant also has memory and skills: bots remember what matters, and a skill a bot proposes is only used once you approve it ([memory and skills](https://kwant.si/docs/memory-skills)). Learning is the centre of Hermes Agent's design, though, and we don't claim kwant matches it.

## Pick kwant if…

- You want several bots with different roles, working as a team: a lead, @mention hand-offs, and a task board (off until you switch it on) where a task counts as Verified only when its checks pass ([bots and teams](https://kwant.si/docs/bots-and-teams), [mission board](https://kwant.si/docs/mission-board)).
- You want to decide how much each bot may do without asking: three [approval modes](https://kwant.si/docs/approvals), plus optional [Flight School](https://kwant.si/docs/flight-school) tiers (Observe, Sandbox, Act) per bot and per kind of work on board tasks. Since 0.6, each bot gets a scorecard, and a bot that qualifies can ask for a higher tier, which only you can grant. In every mode, anything that looks like a payment asks first.
- You want your bots to agree on facts. kwant 0.6 adds an optional Truth Ledger: when two facts disagree, such as "4 GB" and "12 GB" of RAM, they're flagged until you settle them. Like Flight School, it's off by default ([Truth Ledger docs](https://kwant.si/docs/truth-ledger)).
- You want your bots to run on the CLIs and accounts you already have, with several accounts per engine and failover ([accounts](https://kwant.si/docs/accounts)).
- You want routines that run on your own machine ([routines](https://kwant.si/docs/routines)).

kwant's own docs are direct about limits: its middle approval mode "guards against accidents. It isn't a sandbox." And it has the smallest community of the three, by a wide margin.

## Install effort

- **OpenClaw:** `curl -fsSL https://openclaw.ai/install.sh | bash` on macOS, Linux or WSL2, or `iwr -useb https://openclaw.ai/install.ps1 | iex` in Windows PowerShell. The installer provides Node.js if needed. If you already manage Node (24.22.1+ or 26.12.1+), `npm install -g openclaw@latest --allow-scripts=openclaw` (leave out the last flag on npm 11.15 and earlier). ([source](https://github.com/openclaw/openclaw), as of Nov 1, 2026)
- **Hermes Agent:** `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash` on Linux, macOS or WSL2, or `iex (irm https://hermes-agent.nousresearch.com/install.ps1)` in Windows PowerShell. Android has its own Termux guide. ([source](https://github.com/NousResearch/hermes-agent), as of Nov 1, 2026)
- **kwant:** download the Mac app (a universal .dmg, signed ad hoc and not notarized by Apple, so macOS asks you to click Open Anyway the first time), or on a server use Docker Compose or build the binary. The binary build needs Rust and Node on the build machine; the machine that runs it needs no Node.js. See the [install guide](https://kwant.si/docs/install).

## What we haven't compared

We did not test any of the three side by side, and we have not measured speed, cost or reliability. We left out hosted products, and agent frameworks you program yourself such as CrewAI. If we have a detail wrong about OpenClaw or Hermes Agent, their repositories are the authority.

## So which one?

If you are choosing one assistant and want it in your chat apps, start with OpenClaw. If you want an agent that learns and can live on several backends, start with Hermes Agent. If you want a team of bots with approvals on your own machine, try kwant. They don't exclude each other.

A note on the name: "kwant" here is the open-source AI agent team at kwant.si, not the Kwant physics package or Kwant.ai. It isn't affiliated with or endorsed by Anthropic, OpenAI, xAI, Google or Apple. See also [self-hosted AI agents](https://kwant.si/self-hosted-ai-agents) for what running your own agent team involves.
