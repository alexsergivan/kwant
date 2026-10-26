# Changelog

Notable changes to kwant, newest first. Version numbers follow the Mac app releases
([GitHub releases](https://github.com/alexsergivan/kwant/releases)). The server (`kwnt`) and the web
app ship from `main`, so changes under **Unreleased** are already on `main` but not yet in a Mac download.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

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

[Mac 0.1.2]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.2
[Mac 0.1.1]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.1
[Mac 0.1.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.0
