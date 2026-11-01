title: Claude Code scheduled tasks: cloud vs desktop vs /loop
description: Claude Code can schedule work in the cloud, on your desktop or inside a session. Here is how the three differ, plus a self-hosted AI agent option.
date: 2026-10-31
tags: claude-code, scheduled-tasks, routines, self-hosted, comparison
slug: claude-code-scheduled-tasks-compared
card: Cloud, desktop or /loop, plus a self-hosted option.
draft: no
---

If you search for how to run Claude Code on a schedule, you will meet three names for what looks like one feature: scheduled tasks, routines and `/loop`. They are not the same thing, and picking the wrong one means a job that silently doesn't run when your laptop is shut. This post lays out Anthropic's three options from its own documentation, then adds a fourth that doesn't depend on Anthropic's cloud: routines in kwant, a self-hosted AI agent team.

Everything about Claude Code below comes from Anthropic's docs, read on 2026-10-31. The feature is new and changing, so check the linked pages before you decide.

## The short answer

- Job must run while your computer is off: **cloud scheduled tasks**.
- Job needs your local files and tools, and your computer is on: **desktop scheduled tasks**.
- Quick polling while you're working in a session: **`/loop`**.
- You want scheduled bots on your own machine or server, not on Anthropic's cloud, and you want several bots working together: **kwant routines**.

## What changed: routines are now "scheduled tasks"

Anthropic's cloud feature used to be called routines. Its docs now describe it as ["scheduled tasks, formerly routines"](https://code.claude.com/docs/en/routines), and say the earlier name "still appears". The same page calls the feature a research preview, meaning its behaviour and limits may change. That is why searches still mix the two names.

This matters because kwant also has something called routines. They are a different thing: kwant routines run inside kwant on your own machine or server, and kwant isn't affiliated with Anthropic.

## Cloud scheduled tasks

A cloud scheduled task is a saved Claude Code configuration (a prompt, one or more repositories and a set of connectors) that runs on Anthropic-managed cloud infrastructure, "so they keep working when your laptop is closed". They can run on a schedule, on API calls or on GitHub events. They're available on Pro, Max, Ultra, Team and Enterprise plans, and you create them at claude.ai/code/routines, in the Desktop app, or with `/schedule` in the CLI ([Anthropic docs](https://code.claude.com/docs/en/routines)).

Per the [comparison table](https://code.claude.com/docs/en/scheduled-tasks) in Anthropic's docs, cloud tasks don't need your machine on, have no access to your local files (each run gets a fresh clone), and have a minimum interval of one hour.

**Pick this if** the job is repo work that should run reliably without your computer, such as a nightly review or a PR check.

## Desktop scheduled tasks

Desktop tasks run on your machine, with access to your local files, and need that machine to be on. The minimum interval is one minute when Claude sets it and one hour in the forms ([Anthropic docs](https://code.claude.com/docs/en/scheduled-tasks)).

**Pick this if** the job needs local files or tools and your computer is on when it runs.

## /loop

`/loop` re-runs a prompt on an interval while a Claude Code session stays open. The minimum interval is one minute. It is meant for quick polling, like checking a deployment or watching a PR, and it is session-scoped: it needs an open session ([Anthropic docs](https://code.claude.com/docs/en/scheduled-tasks)).

**Pick this if** you're at your desk and want something checked every few minutes for the next hour.

## Comparison table

| | Cloud | Desktop | /loop |
|---|---|---|---|
| Runs on | Anthropic-managed cloud by default | Your machine | Your machine |
| Needs your machine on | No | Yes | Yes |
| Needs an open session | No | No | Yes |
| Access to local files | No (fresh clone) | Yes | Yes |
| Minimum interval | 1 hour | 1 minute when Claude sets it, 1 hour in the forms | 1 minute |

Source: Anthropic's ["Compare scheduling options"](https://code.claude.com/docs/en/scheduled-tasks), as of 2026-10-31.

## A self-hosted option: kwant routines

[kwant](https://kwant.si/) is an open-source (AGPL-3.0) team of AI agents that you run on your own laptop, VPS or Docker. A [routine](https://kwant.si/docs/routines) sends a prompt to one of your bots on a schedule and posts the answer in a channel, the bot's direct chat or a thread.

What a kwant routine gives you, per its docs:

- **Schedules:** Once, Daily, Weekly, Monthly, Every… (from every 5 minutes up to every week) and a Custom cron rule.
- **Run now**, a history of the last 200 runs, and an on/off switch for each routine.
- **Follow-ups:** a bot can set itself a one-off check-back, for example to look at a deploy in an hour.
- **Notifications:** a ping on your device or an HTTPS webhook such as ntfy, and a "While you were away" list of what finished or failed.

Routines run on your own machine, so they run while that machine is on. A small server that stays up avoids the sleeping-laptop problem; the [install guide](https://kwant.si/docs/install) shows a systemd setup.

kwant isn't a drop-in replacement for Anthropic's cloud tasks. Its routines start on a schedule or with Run now, not on GitHub events, and you look after the machine. What it adds is several bots working as a team, with approvals you control: three approval modes, plus optional [Flight School](https://kwant.si/docs/flight-school) tiers (Observe, Sandbox, Act) that set how much each bot may do on its own on board tasks ([approvals](https://kwant.si/docs/approvals)).

### New in kwant 0.5: routines that say what a good run is

The kwant 0.5 release notes describe routines that can carry a Flight School class, a description of a good result and checks, so with the Mission board switched on, a run that passes its checks counts as verified. The same release adds a setting that suggests routines from tasks you repeat. It's off by default, and nothing is created until you accept ([release notes](https://github.com/alexsergivan/kwant/blob/main/CHANGELOG.md), "Mac 0.5.0", 2026-10-31).

## Which should you use?

| Your situation | Use |
|---|---|
| Repo job that must run when your computer is off | Cloud scheduled tasks |
| Job that needs local files, computer is on | Desktop scheduled tasks |
| Watching something for the next hour | `/loop` |
| Scheduled work on your own server, with several bots and approvals | kwant routines |
| You want the least to maintain | Cloud scheduled tasks |

A note on the name: "kwant" here is the open-source AI agent team at kwant.si, not the Kwant physics package or Kwant.ai. It isn't affiliated with or endorsed by Anthropic.

To try kwant routines, start with the [quickstart](https://kwant.si/docs/quickstart) and the [routines guide](https://kwant.si/docs/routines).
