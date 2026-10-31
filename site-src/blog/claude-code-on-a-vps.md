title: Run Claude Code on a VPS and reach it from your phone
description: Three ways to run Claude Code on a VPS and reach it from your phone: SSH and tmux, Anthropic's Remote Control, or a self-hosted AI agent team with kwant.
date: 2026-10-31
tags: claude-code, vps, self-hosted, how-to, ai-agents
slug: claude-code-on-a-vps
card: SSH and tmux, Remote Control, or a kwant bot team.
---

A laptop that sleeps is a poor home for a coding agent. A small server that stays on is better, and the question that follows is how to reach it from your phone. This guide covers three routes, what each one needs, and when each is the right pick. One of them is kwant, the self-hosted AI agent team we build. It is not the right answer for everyone, and we say where it isn't.

## What you need before you start

- A VPS you can SSH into. Anything small that runs Linux works for the first two options. The Docker route for kwant needs about 4 GB of free RAM ([install guide](https://kwant.si/docs/install), checked 2026-10-31).
- Claude Code installed on that server. Anthropic's recommended install is `curl -fsSL https://claude.ai/install.sh | bash` ([Anthropic docs](https://code.claude.com/docs/en/overview), as of 2026-10-31).
- A Claude login the CLI can use. Which kind matters for option 2; see below.

## Option 1: SSH and tmux

This is the simplest route and it needs nothing beyond Claude Code.

1. SSH into the server and start a session that survives disconnects: `tmux new -s claude`.
2. Run `claude` inside it and detach with `Ctrl-b d`.
3. Reconnect later with `tmux attach -t claude`.

On a phone, use any SSH app. The cost is the experience: you are typing into a terminal on a small screen, and approving a command means reading it in a terminal.

**Pick this if** you already live in a terminal and want no extra software.

## Option 2: Claude Code's Remote Control

Anthropic's own route is [Remote Control](https://code.claude.com/docs/en/remote-control). It connects claude.ai/code or the Claude mobile app to a Claude Code session running on your machine. Run `claude remote-control` in your project directory; the process stays running and shows a session URL, and you can press the spacebar to show a QR code for your phone (Anthropic docs, as of 2026-10-31).

Two requirements from the docs are worth knowing before you plan around it:

- It needs a claude.ai subscription login on a Pro, Max, Ultra, Team or Enterprise plan. The docs say API keys are not supported, and on Team and Enterprise an Owner must first turn on the Remote Control toggle in Claude Code admin settings.
- The session runs on your machine, so "your computer has to stay on and the claude process has to keep running." A VPS is such a machine, but we have not tested this route on a headless server, so check the docs for the sign-in step before relying on it.

**Pick this if** you pay for a Claude subscription, want the official mobile app experience, and mainly work with one Claude Code session at a time.

## Option 3: kwant on the same server, with a phone-friendly web app

[kwant](https://kwant.si/) is an open-source, self-hosted team of AI agents ("bots" in its own wording) in one binary, `kwnt`. It runs the bots with the CLIs you already use, and its web app works on a phone, so you can read the team's summary or approve an action away from your desk ([home page](https://kwant.si/), checked 2026-10-31). kwant is licensed AGPL-3.0, with a commercial option.

This is a bigger step than the other two: you are adding a small server application, not only reaching a CLI. It makes sense if you want several bots, approvals in one place, and scheduled work on your own server. If you only want one Claude Code session on your phone, option 1 or 2 is less to set up.

## Set up kwant on a VPS

The [install guide](https://kwant.si/docs/install) documents a Docker Compose route that starts kwant, its router and a private browser for the bots:

```
git clone https://github.com/alexsergivan/kwant.git && cd kwant
KWNT_CLIS=claude,codex docker compose up -d --build
docker compose exec kwnt kwnt doctor
docker compose exec kwnt claude auth login
docker compose exec kwnt kwnt signin-link
```

`kwnt doctor` reports what works and the fix for what doesn't. `kwnt signin-link` prints a one-time sign-in link and a QR code that work once, for 15 minutes; set `KWNT_ALLOWED_ORIGINS` first (below) so the link has your HTTPS address. The guide also mentions `kwnt token --raw`, which prints the permanent API token: keep that for scripts and treat it like a password. Instead of a CLI sign-in you can add an API key in the app under Space settings → Accounts ([install guide](https://kwant.si/docs/install)). Read the [accounts page](https://kwant.si/docs/accounts) for how several accounts and failover work, and check your provider's terms for whatever you sign in with.

The guide also describes a single-binary route. Build with `make build`, copy the binary to the server, and run `kwnt serve --host 0.0.0.0 --port 8799`. `make build` needs Rust and Node, and makes a binary for the machine it runs on, so build on the server itself or on a Linux machine with the same CPU type and the same or an older distribution (a binary built on a Mac won't run on Linux). Or use the Docker route. The server prints a one-time sign-in link that works once, for 15 minutes.

### Put HTTPS in front

With Docker, the web app is published on 127.0.0.1:8799 only. Put an HTTPS reverse proxy in front and set `KWNT_ALLOWED_ORIGINS` to your HTTPS address. The guide's Caddy example is two lines: `your.domain { reverse_proxy 127.0.0.1:8799 }`. HTTPS also matters for the phone: the guide notes that browsers only install the web app from pages served over https or from 127.0.0.1.

If you would rather not expose anything, the guide shows an SSH tunnel: `ssh -N -L 8799:127.0.0.1:8799 user@your-vps`, then open `http://127.0.0.1:8799` on your computer.

### Sign in from your phone

Once you are signed in, open Settings → Signed-in devices and choose Add a phone or browser. kwant shows a one-time QR code that works once, for 15 minutes. Scan it with your phone's camera. The page then offers to install kwant as a web app ([install guide](https://kwant.si/docs/install)).

### Keep it running

The guide includes a systemd unit that runs `kwnt serve` as its own user with `Restart=always` and `KWNT_DATA_DIR=/var/lib/kwnt`, so kwant starts again after a reboot.

## Security checklist for a server that runs a coding agent

- **Decide what the agent may do without asking.** kwant has three approval modes (ask, smart, off) and, in every mode, anything that looks like a payment asks first ([approvals](https://kwant.si/docs/approvals)). Its own docs say the middle mode "guards against accidents. It isn't a sandbox." Use the strictest mode for bots or folders you don't trust.
- **Never expose Chromium's debugging port or the Docker socket.** The install guide says so in as many words.
- **Keep the sign-in link private.** Anyone who can read the server's output can use it.
- **Use a dedicated user.** The systemd example creates one for kwant.
- **Limit freedom per bot.** [Flight School](https://kwant.si/docs/flight-school) lets you set Observe, Sandbox or Act per bot and per kind of work. It is off until you switch it on, and tiers apply to tasks on the Mission board.

## Which option should you pick?

| If you… | Pick |
|---|---|
| live in a terminal and want nothing extra | SSH and tmux |
| pay for Claude and want Anthropic's mobile experience | Remote Control |
| want several bots, approvals and scheduled work on your own server | kwant |
| want a hosted service with no server to look after | None of these; look at hosted options |

A note on the name: "kwant" here means the open-source AI agent team at kwant.si. It is not the Kwant physics package or Kwant.ai, and kwant is not affiliated with or endorsed by Anthropic, OpenAI, xAI, Google or Apple.

Ready to try it? Start with the [install guide](https://kwant.si/docs/install) or the [quickstart](https://kwant.si/docs/quickstart).
