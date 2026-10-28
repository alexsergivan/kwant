# Changelog

Notable changes to kwant, newest first. Version numbers follow the Mac app releases
([GitHub releases](https://github.com/alexsergivan/kwant/releases)). The server (`kwnt`) and the web
app ship from `main`, so changes under **Unreleased** are already on `main` but not yet in a Mac download.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Added
- **Mission board: AutoFix, change approvals and restore.** A failed task shows why it failed in plain
  words, what AutoFix plans, its tries and what retries cost, with Retry now and Don't retry. Proposed
  changes to a check, the acceptance or a routine appear in Needs you with before and after, and only
  you can approve them. A task's checkpoints can be previewed and restored from its panel; a restore
  puts the task on hold until you press Run, or pick "Restore and run again". Settings has an AutoFix
  switch (off by default) and the retry budget; Insights counts what AutoFix fixed.
- **Mission Control: AutoFix (backend).** Every failed task now records why it failed, with secrets
  masked. Turn on the new `autofix` setting (off by default, or "suggest" to approve each retry) and
  kwant retries what is safe to retry: after a temporary error it waits 1, 5, then 25 minutes; after a
  failed check it runs again with the check's output. Retries stay within the task's attempts and a
  recovery budget (25% of the task's budget by default). Sign-in problems, budget overruns and denied
  actions are never retried; they ask you instead. Bots can propose changes to a check, the acceptance
  or a routine, and nothing changes until you approve. The board UI follows separately.
- **Mission Control: checkpoints and restore (backend).** kwant snapshots a task's folder before and
  after each run, and on demand. You can preview and restore a checkpoint: only the files that task's
  runs changed are written back, after a safety checkpoint you can restore in turn. What the bot did
  outside the folder (commands, web, messages) is listed as not undone. Every tool call of a task's
  turns is now recorded with its effect. Snapshots stay private on the server. The board UI follows
  separately.
- **Mission board: "May be invalid" flags, redo copies and how tasks connect.** A flagged task shows
  a "May be invalid" badge and the reason on its card, waits under **Needs you**, and stays Verified
  until you pick Still valid, Redo or Cancel on the task (Cancel is off for a verified task). Redo on a
  verified task makes a copy that starts running on its own; the copy links back with "Copied from". A
  new **How tasks connect** view lists a mission's tasks step by step, with open flags and how many
  tasks further down weren't flagged, plus a **Check files now** button. In Review, an optional check
  waiting for you now uses the same quiet grey row as on Verified.
- **Mission Control: work graph and "may be invalid" flags (backend).** When a verified task's evidence
  file changes or disappears, or a verified task is redone, kwant flags that task and the started tasks
  that directly depend on it as "may be invalid". Their state never changes, and only you decide:
  still valid, redo, or cancel. Redo forks the task, so verified history stays untouched. New
  endpoints serve the mission's graph (`/api/missions/:id/graph`), flags, forks and an on-demand drift
  scan. Fork, retry and resolving a flag accept an `Idempotency-Key`, so a double Redo makes one
  copy. The board UI follows separately.
- **Mission Control (preview, off by default).** One task board for everything bots work on: planned
  missions, team hand-offs, routine runs, Loose Ends you take on and work-ahead posts. A task counts as
  **verified** only when an outcome check passes (a command, a URL, a file, a teammate's review with
  evidence, or you), never on the bot's own word. Missions have a token budget that claims can't
  exceed, tasks wait for the tasks they depend on, and anything waiting on you shows under **Needs
  you**. Turn it on with `mission_board` in settings. API: `docs/mission-control.md`.
- **Mission board.** One board per space with Ready, Working, Needs you, Review and Verified. Each
  card shows its owner, cost and checks. Its evidence receipt lists every check with its result and
  says "Not checked" when nothing has run. A strip shows this week's verified tasks, false successes
  and blocked work. Turn it on in Settings → Mission board. It works on phone and desktop.

### Changed
- **Mission Control: a restore no longer starts the task again by itself.** A restored task that could
  run waits until you press Run; "restore and run again" keeps the old behaviour. A restore conflict
  names another task only when that task's own file edits wrote the file; otherwise it says "changed
  outside this task".
- **An optional check never shows as needing you.** While a required check is still running, an
  optional check waiting for your OK no longer counts in "Needs you now" or shows the amber "Waiting
  for you" block on the receipt. You can still allow it from its row.
- **A card says "Needs you" only when a check that counts waits for your OK.** A task whose required
  check is still running while an optional check waits now reads as still checking, with a grey
  "1 optional check waiting" note. The quiet "Not run" optional check on a verified task has a grey
  icon instead of the amber hand.
- **A task's spend shows up the moment it is verified.** When a bot hands a task in partway through
  its turn, the tokens used so far land on the task and mission right away; the rest of the turn is
  added when it ends, so nothing is counted twice. Before, a just-verified task showed 0 tokens for a
  few seconds. Engines that report usage only at the end (Codex, Grok, custom CLIs) still add it then.
- **Mission board checks read plainly.** A card counts only the checks that count, so a Verified
  task with an optional check still waiting says "Check passed" and "1 optional check waiting" instead
  of "1 of 2 checks passed". Once a task is Verified, that check's button is a quieter "Run this
  optional check".
- **Deciding for you costs fewer tokens.** Before deciding a choice itself, a bot now hears at most one
  teammate, and only when the choice or an open check names that teammate.
- **Mission API: `checks_summary.required_needs_approval`.** Counts the required checks waiting for
  your OK, so a card can tell "needs you" from "a required check is still running while an optional
  one waits".
- **"Run this optional check" runs it.** Approving an optional check on a Verified task now runs the
  check and records whether it passed or failed, with its output. The task stays Verified either way
  and its spend doesn't change. Before, the check was only approved and never ran.

### Fixed
- **Mission board: "To fix" no longer repeats the failure's name.** A usage-limit failure read "Usage
  limit reached" on its badge and again at the start of "To fix"; "To fix" now starts with what to do.
- **Mission board: AutoFix hides passwords without hiding command names.** A check's command name or a
  file path is no longer shown as `[redacted]`, short passwords after a flag like `--key` or `-p` (for
  mysql, sshpass, redis-cli) are hidden, and a stray `]` no longer appears. A sign-in or usage-limit
  card says what to do once instead of repeating the failure.
- **Mission board: "Needs you now" matches the Needs you column.** The tile counted only questions and
  approvals, so failed tasks waiting on a fix showed 0 next to a full column. It now counts the column's
  cards, and a task whose check waits for your OK sits in Needs you. A failure's panel says the summary,
  what to fix and whether AutoFix retries once each, instead of up to three times.
- **Mission Control: racing Redo requests make one copy, and restore refuses unknown forced paths.**
  Two Redo requests on the same flag at the same moment, without an `Idempotency-Key`, could each make
  a copy; now the flag is claimed in the same step as the copy, so one wins and the rest get 409. A
  restore whose `force_paths` names a path the preview doesn't list is now refused with 422 naming the
  path, instead of being ignored.

## [Mac 0.1.3] - 2026-10-26

### Added
- **Bots decide instead of asking you to pick.** When a reply ends by asking you to choose between
  options that can be undone (a design direction, wording, an approach), the bot first finishes the
  job: it runs the checks it said were still open, hears up to two teammates, picks, applies it, and
  tells you what it chose, why, and how to change it. Each choice shows as a chip with **Change**.
  Bots still ask before anything that can't be undone, reaches someone outside, costs money or needs a
  password or sign-in, and when you say you want to pick yourself. App settings has a switch for it.
- **Bots learn how you decide.** Your answers to question cards and your changes to the bots' choices
  are kept per space, and bots use them to predict your next pick instead of asking.
- **Bots work ahead in quiet chats.** When a chat has gone quiet for a couple of hours with open
  items, its lead bot does the next safe, undoable step on its own (a check, a draft, a summary) and
  leaves one "While you were away" message with what it did and what it expects you'll want next. It
  never sends, posts, pays or signs in, stays silent when nothing is worth doing, works each chat at
  most once until you write again, and is capped per space per day. App settings has a switch for it.
- **Questions say why only you can answer.** A bot's question card now says why it needs you and what
  it checked first; a question without a reason goes back to the bot to decide itself.
- **See and change what bots decided.** A bot's choice shows as a chip, for example "Decided: B: Wax
  seal", with the reason and who it asked first; **Change** lists the other options and the bot redoes
  what depended on it. While the team is deciding, the chat shows "Team is deciding…" instead of a
  half-written reply, and work done while you were away arrives as a quiet, folded message. App
  settings has a **Bots on their own** section with both switches and a daily limit.

### Fixed
- **The Gmail and Figma notes in Add a server no longer cover the next section.** Opening a "via
  Composio" app stretched its card and laid the explanation over the apps below it.

## [Mac 0.1.2] - 2026-10-26

### Added
- **Update banner in the Mac app.** When a new Mac version is out, a slim bar at the top says
  "kwant bots X.Y.Z is ready" with **Update and restart** and **What's new** (that version's notes).
  It shows download progress, then "Restarting…", then "Updated to X.Y.Z". If the update fails, it
  shows why, with **Try again** and **Download manually**, and the current app keeps working.
  Hiding the bar hides it for that version only. App settings → About shows the running version and
  has **Check for updates**. Nothing shows outside the Mac app, and phones get only the About row.
  In the Mac app, App settings → Notifications has an **App updates** switch for the update notification.
- **Vibration on Android phones.** The web app gives a short buzz when you send a message, answer a
  question or approval card, press **Solve it**, **Done**, **Skip** or **Hand back**, and when a puzzle
  is solved, with a different buzz for errors. A bot reply arriving while the app is on screen gives a
  light double buzz, once per reply and at most one every 3 seconds. A **Vibration** switch in App
  settings turns it off; it starts off when the phone asks for reduced motion. iPhones and desktops have no web vibration.
- **Hand-offs that wait for each other.** When a bot's reply asks one teammate to go "once @Analyst
  has the numbers" (also "after", "when … has finished", "wait for"), that teammate starts only when
  the other has finished, and gets its result. Other teammates in the same reply still start right
  away. If the first one fails or asks you something, the waiting one doesn't start, and the chat says why.
- **One list of questions.** When two or more bots ask you something after one message, the team
  lead (or the bot that started) posts one numbered list of everything they need. Your answer goes
  to the lead, which passes each part on; bots waiting on a question card continue directly.
- **Reminders when a bot waits on you.** When a bot's reply ends by asking you something, or it asks
  with a question card (also Claude's own question tool, which now shows as a card you can answer),
  it reminds you once in the same chat after 24 hours and sends a notification. Writing in that chat
  or answering the question cancels the reminder. Set the delay with `waiting_follow_up_hours` in
  settings (0 turns reminders off). `GET /api/waiting` (you only, never a bot) lists everything bots
  are waiting on you for.
- **See what a reply really cost.** Each reply now shows its fresh tokens (new input, cache writes
  and output) instead of a total padded with cheap cache reads; hover for the full split and the cost
  when the engine reports one. Claude sign-in bots now appear in a bot's **Usage** tab too, and
  **App settings → Usage** lists fresh and cached tokens and cost per day and bot.
- **Token budget per request.** When bots handing work to each other go over it (200k fresh tokens
  by default, 0 turns it off) for one of your messages, they pause with one notice in the chat;
  press **Continue** or say "continue" to let them go on.
- **Updates inside the Mac app.** kwant bots for Mac checks for a new version on start and every 6 hours
  and tells you once per version. **Update to x.y.z…** in the menu bar (or in the web app) downloads
  it, checks its signature, replaces the app and restarts it. If anything goes wrong, the running
  version keeps working. Versions before this one need one last download by hand.

### Changed
- A bot reply that ends with a closing line such as "Anything else?", "Should I continue?" or
  "Want me to also…?" (and now also "Happy to help further?", "Any other questions?", "What else can
  I do for you?") no longer pulls the person's next message back to that bot. Real questions
  ("Which city are you in?") still do, unless the next message clearly fits another bot better.
- UK places (London, Belfast, Jersey, Guernsey, Isle of Man) get metric units with road distances
  and speeds in miles and mph.
- A short message with an attachment ("what's the total on this?" plus a PDF) no longer gets the
  deepest, slowest model tier on its own; asking for depth ("do a deep dive") still does.

### Fixed
- Invisible characters (zero-width, bidi overrides, Unicode tags) are stripped from the city setting.
- A CAPTCHA step now holds the browser before the step appears, so a bot can't act on the page in between.

### Security
- Only the person can check for or install a Mac app update: `POST /api/app/update/check` and
  `/install` refuse a bot's turn token. Bots can still read `GET /api/app/update`.
- Only the person can switch which computer the bots run on (Settings → Computer) or open their
  browser as a window: `PUT /api/settings/computer` and `/api/settings/computer/window` now refuse
  a bot's turn token, like `POST /api/settings`.
- Changing app-wide settings (`POST /api/settings`) is person-only. Bots change the settings they
  are allowed to through their `settings_update` action.

## [Mac 0.1.1] - 2026-10-26

### Added
- **CAPTCHA hand-off.** When a bot hits a CAPTCHA (TikTok slider, reCAPTCHA, hCaptcha, Cloudflare
  Turnstile and others), it stops and notifies the person. **Solve it** opens the page full screen,
  zoomed in on the puzzle, with drag, pinch and keyboard support on phones. The bot continues by itself
  once the puzzle is solved, and skips the site after a time-out (10 minutes by default, in Settings).
  No solving services, browser disguises or proxies. Ordinary "I'm not a robot" boxes and look-alike
  pop-ups don't raise a step.
- Bots see the person's time zone and city, use local units (metric outside the US, Liberia and
  Myanmar) and local sources.
- New routines report their first run in the person's time zone, for example
  "first run: Mon 26 Oct 07:00 Europe/Berlin".
- Web Push: the installed web app gets notifications while it's closed.
- Sign in from another device: one-time sign-in links, a QR code under Settings → "Add a phone or
  browser", an approve prompt on a signed-in device, and a revocable list of signed-in devices.
- People can create and regroup channels; Laya routes every new channel.
- Team-aware suggested prompts in chat.

### Changed
- The kwant.si mascot is the icon of the Mac app, its menu bar and the web app.
- The Mac install window has a light background and a drag arrow to Applications.
- A person's answer goes to the bot that asked; an @mention still wins.
- Steadier routing: Laya gets 4 s before kwant falls back to keywords (`KWNT_LAYA_TIMEOUT_MS`), every
  fallback is logged with its reason, generic verbs such as "send" no longer pick a bot, and greetings
  go to the bot that spoke last or the channel's lead.
- Every screen fits phone widths.
- The web app bundles its fonts and loads nothing from other servers.
- License: AGPL-3.0 plus commercial licenses (see [COMMERCIAL.md](COMMERCIAL.md)). Earlier versions
  stay under MIT/Apache-2.0.

### Fixed
- Handles with a dot, such as "@mr.absurdia", are no longer cut short or answered with "Not sent to …".
- Bot replies show without reopening the chat.
- `kwnt token` prints a one-time link and never the API token.

### Security
- The browser cookie folder is readable only by its owner (`0700`).

## [Mac 0.1.0] - 2026-10-25

First public release of kwant for Mac: one universal app for Apple silicon and Intel Macs (macOS 11 or
later) that runs the kwant server and its interface, with a menu-bar icon. It uses the same data folder
as `kwnt serve` (`~/.kwnt`).

[Mac 0.1.3]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.3
[Mac 0.1.2]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.2
[Mac 0.1.1]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.1
[Mac 0.1.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.0
