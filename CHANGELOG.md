# Changelog

Notable changes to kwant, newest first. Version numbers follow the Mac app releases
([GitHub releases](https://github.com/alexsergivan/kwant/releases)). The server (`kwnt`) and the web
app ship from `main`, so changes under **Unreleased** are already on `main` but not yet in a Mac download.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Added
- **Performance Market (report-only).** kwant now records how each finished task went: who did it, on
  which model, how long it took, its tokens and cost, whether it ended verified, and how often you had
  to step in. Earlier finished tasks are filled in once on upgrade. `GET /api/performance` groups this
  by task class and bot (and model) with a 95% confidence range on the verified rate and flags groups
  with fewer than 5 tasks. It doesn't change who gets work: Laya still routes.
- **Performance on the Mission board.** Under the board (All work), a Performance section shows, per
  kind of work and bot (or model), how many tasks finished, the verified rate with its likely range
  ("56% (27–81%)"), median time and tokens, cost per task and how often you stepped in. Filter by kind
  of work, bot and period (30 days, 90 days, all time); groups with fewer than 5 tasks say "few tasks".
  It's report-only and only you can see it.
- **Swarm Auditor (off by default).** With `auditor` on in Settings, a high-priority task, or one no
  outside check will judge, gets one challenge when it reaches review: a teammate outside the owner's
  lead loop lists concrete risks with evidence, the owner answers once, and what stays unresolved is
  listed on the task until you close it. An audit never verifies, fails or moves a task, and "no risks
  raised" isn't a pass. Both turns ask before changing anything; at most 5 audits a day by default.
  `GET /api/tasks/:id/risks`, `GET /api/risks`, `POST /api/tasks/:id/audit`, and
  `POST /api/tasks/:id/risks/:risk_id/resolve`.
- **Open risks on tasks.** An audited task's panel lists each audit (who challenged, how far it got,
  its tokens) and each risk with its evidence and the owner's answer, with Resolve and Dismiss for you.
  Board cards show how many risks are still open. While an audit waits on one of your approval cards,
  the task says "Audit waiting for your answer" with a link to it. Settings has the Auditor switch and
  its daily limit.
- **Skill Foundry (off by default).** With `skill_foundry` on in Settings, when a bot has 3 or more
  verified tasks of one kind of work (70% of them verified, in 60 days), it drafts a skill from them:
  the steps, the inputs, checks that judge the result, the example tasks and the tools it uses. Each
  draft is replayed as a dry run on up to 3 of its examples in a scratch copy of their folder, where
  nothing outside that folder runs, and the report sits on the draft for you to enable, edit or reject.
  Listed tools are shown, never granted. Versions of a skill can be rolled back.
  `GET /api/skills/foundry`, `POST /api/skills/foundry/:cluster/draft`, `POST /api/skills/:id/replay`,
  `POST /api/skills/:id/rollback`, `GET /api/skills/:id/foundry`. At most 2 drafts a day by default.
- **Audit model in Settings.** With the auditor on, an **Audit model** picker sets which model audits
  use: Fast (the default and cheapest), Balanced, Deep, or the same model as the bot. A value the
  server rejects is flagged next to the picker in plain words.

### Changed
- **The hand-off token limit is now off by default.** A request no longer pauses after 200k fresh
  tokens of bots handing work to each other; the hand-off count caps (4 per bot, 16 per request, with
  Continue) still stop loops. Set a limit in Settings → Usage → Hand-offs per request (No limit, a quick
  pick or Custom) and it pauses and offers Continue as before. On upgrade, a stored limit still at the
  old 200k default is switched off once; any other number you set stays.
- **Audits run on the fast model by default.** The Swarm Auditor's challenge and rebuttal turns now use
  the engine's cheap, fast model (for Claude, Haiku at medium effort) instead of the bot's default. The
  new `auditor_model` setting picks `fast` (default), `balanced`, `deep`, or `bot` for the bot's usual
  model.
- **Kinds of work read as words.** The Performance section and Flight School show a task class such as
  `code-review` as "Code review" instead of its raw name.

### Fixed
- **One approval card per audit challenge.** When the auditor's call was allowed but then refused for
  its input (usually the task's own id given as evidence), the bot's corrected call asked again, so one
  challenge could need two cards. The corrected call now goes through on the first card (at most two
  corrections per stage), and the task's own id is no longer refused as evidence.
- **No raw `[DECISIONS]` text in older channel replies.** Replies saved in a team channel could end with
  the bot's raw `[DECISIONS] … [/DECISIONS]` block. The chat now hides it and shows its choices as
  read-only chips (a line it can't read is dropped, never shown); a block being typed live is hidden too.
- **Choices in channels show as chips, not raw text.** A bot's `[DECISIONS] …` block in a team
  discussion or a "While you were away" post now becomes choice chips you can change, as in direct
  chats. A garbled block (two choices on one line, no closing tag, in a code fence) is removed, never shown.
- **The bot browser no longer opens addresses it guessed.** A message with "open" or "browser" and a
  folder path like `/Users/…/dev/KORB` sent the browser to that path on the site already open
  (`github.com/Users/…`, a 404), and a bot reply that mentioned a server like `registry-1.docker.io`
  opened it on its own. Before a turn the browser now opens only a full `http://` or `https://` link in
  the message, and a reply never moves it. A file path or `file://` address given to the browser is
  refused with a note to read local files with file tools.
- **Hand-off cards no longer pile up in Review.** When a teammate finishes a hand-off, the bot that
  handed it out reviews the result in its next reply: a pass verifies the card, a fail sends the work
  back to the teammate with the reason. Only that bot's review counts (the teammate still can't approve
  its own work), and until it reviews, the card says "Waiting for <bot> to review". A bot that doesn't
  review gets one reminder (one for all the cards of a batch); a card it never got to see (one blocked when the hand-off finished, or one
  caught by a restart) gets its own review turn. A one-way hand-off, whose result nobody is asked about,
  is closed ("Notice, no review needed", shown as Closed, not Verified) instead of waiting in Review. Each hand-off's card
  is named after what was asked of that teammate, not after your message, so two hand-offs to one bot
  no longer look like duplicates.
- **Bots can see the page, not just read it.** Bots now have a `browser_screenshot` tool that returns
  the browser page as a picture, so a designer bot can check layout, colours and spacing. It can take
  the shot at a set width (390 for a phone, 1280 for a desktop) and puts the browser back to its own size
  afterwards. Like the other browser tools, it waits while you sign in, solve a CAPTCHA or have control.
- **A teammate's result now hands on its @mentions.** In a team channel, when a teammate's result
  @mentions another teammate ("@Frontend could switch to …"), that teammate gets the task even if the
  lead doesn't pass it on. It follows the same caps as the lead's mentions (Continue past them), never
  hands work back to someone earlier in the same chain, and skips mentions that only credit someone.
  The lead is resumed once with all of these results.
- **"@Frontend why didn't you answer QA" goes to Frontend.** A message that @mentions exactly one
  teammate goes to that teammate even if it names others without an @. Two @mentions still go to
  the lead.
- **A restart no longer leaves an audit running.** An audit cut off mid-turn by a restart ends
  `interrupted` and says so in the task's history; one still waiting to start runs after the restart
  (or is skipped if the auditor was turned off).
- **A read letter always shows its updated case.** Opening a document right as it finished reading
  could show the case still titled "Letter (reading…)".
- **"Answer in chat" works when that chat is already open.** On the Mission board, "Answer in chat" and
  "Open the chat it came from" now close the board and show the chat even when it's the one already
  selected underneath; before, nothing visible happened.
- **An expired card on the Away page is a plain notice.** Answering a card the bot stopped waiting for
  shows the server's "This card expired: … Ask the bot again." as a neutral notice instead of an error.

## [Mac 0.3.0] - 2026-10-29

### Added
- **Add a connection from the Connections page.** A new **+ Add connection** button in the header offers
  a tool server (opens Tools straight on adding one), an engine account, or an app through Composio. Empty
  groups link to adding one there, and the row buttons now say **Manage tools** or **Manage accounts**.

### Changed
- **kwant.si covers 0.2.0.** The home page has a "New in 0.2.0" strip, the features page has sections
  for the Mission board, Flight School and connecting apps with new screenshots and three short clips
  (Sandbox, AutoFix, Connect), and there are new docs pages for the Mission board and Flight School.
- **No token limit by default.** New tasks and missions no longer get a hidden 50k (task) or 200k
  (mission) token budget, so ordinary work stops landing in Needs you as "Out of budget". A budget of
  0 means no limit; you can still set one on a task or mission, or as a default in Settings
  (`mission_default_task_budget_tokens`, `mission_default_budget_tokens`). Existing tasks and missions
  still on the old automatic limit have it removed; limits you set yourself stay. A task already
  blocked for its budget stays in Needs you until you remove its limit and unblock it.
- **Token limits on screen.** With no limit, a task or mission shows plain usage ("64.9k tokens used")
  and no bar. **Set a limit** next to Cost on a task, and under the mission's cost, offers 50k, 200k,
  1M or a custom amount; **Change** and **Remove limit** take it from there. A task stopped by its
  limit gets **Remove limit and continue**, on its board card and in its panel, plus **Raise limit**.
  Settings → Usage has a **Token limits** group for the default on new tasks and missions, "No limit"
  unless you pick one.
- **Unblocking or retrying a hand-off card carries the work on.** A hand-off card sent back to Ready
  used to sit there because nothing runs hand-offs from the board. Now the teammate picks it up again in
  the hand-off's own thread ("carry on from where you stopped") and the card is back to working, or it
  goes to review if the hand-off had already finished while the card was blocked. A hand-off cut short
  by two restarts now fails its card so it offers Retry, and a card that can't carry on is blocked with
  the reason. Cards already stuck in Ready are picked up automatically.
- **Teams can hand on more before kwant pauses them.** One teammate can now be asked by mention up to 4
  times per message (was 2), and one message can start up to 16 hand-offs (was 8). Past that, instead of
  "Send a new message to go on", the chat shows a notice with a Continue button (or say "continue").
- **The pause notice reads plainly and continues in one click.** It now says "Paused after 16 hand-offs
  so it doesn't loop. Continue?", lists who's waiting, and its **Continue** button hands the turn on
  straight away. Once continued (by the button or by typing "continue") it says who it went to.
- **Mission board owners show their avatar.** Board cards, the task panel and "How tasks connect" show
  the owner's bot avatar, the same one as in the sidebar and chat, instead of a letter circle. A bot
  that no longer exists still shows its initials.

### Fixed
- **Board notes aren't lost when kwant is busy.** A task's notes (such as "the hand-off finished;
  unblock to review its result") could be dropped with "database is locked" while other work was
  saving at the same moment. A hand-off that fails before the board sees it start now gets its card
  too, failed so it offers Retry.
- **"Answer in chat" on a task opens the right chat.** It used to open the chat the task came from
  (usually the lead's), not the chat where the task's owner asked; on a hand-off task it was missing.
  Now it opens the owner's hand-off thread or chat where the card is.
- **Cards survive a restart.** An approval card still waiting when kwant restarted used to give an
  error when you answered it. Now Allow resumes the work in the chat that asked, and a card more than
  a day old says it expired so you can ask the bot again. Payments still ask every time.
- **"What went wrong" names the token limit.** When a task stops at its token limit, the box now says
  "Remove or raise the task's token limit, then retry it." instead of "Raise the task's budget".

## [Mac 0.2.0] - 2026-10-28

### Added
- **Flight School: give each bot only as much freedom as it has earned.** With the new `flight_school`
  setting on (off by default), you can set a bot's tier per kind of task: Observe (every change asks),
  Sandbox (it may change files in its task folder when kwant took a checkpoint it can undo; commands that
  can change things ask, while read-only ones in its own folder, such as `ls` or `cat`, run without
  asking) or Act (its normal approvals). A tier only adds approval cards, even with approvals off, and
  payments still always ask. Lowering a tier applies from the bot's next action. Bots already there keep
  Act; bots hired after you switch it on start at Sandbox. A bot on Codex or a custom engine that is below
  Act doesn't run its task, because kwant can't see its own tools.
- **Flight School on screen.** Settings has the Flight School switch and the tier new bots start at, and
  says plainly that bots you already have stay at Act and payments always ask. A bot's settings show its
  tier for all work and per kind of work, where each tier comes from, the recent changes and a one-click
  Revoke. With it on, a task shows its kind of work (which you can change) and its bot's tier, an
  approval card says when the bot's tier asked, and a task whose engine can't be held to its tier sits in
  Needs you and says what to do. With it off, nothing on screen changes.
- **kwant can tell when an app finished connecting through Composio.** A new person-only
  `GET /api/connections/composio/apps/:slug/status` answers whether you've signed in to the app, so the
  connection card can show "Connected" once you have.
- **Bots try the way round before saying "not possible".** Bots can now read a public page's HTML with a
  new `page_source` tool: meta description, canonical, robots, Open Graph tags, headings, image alt text
  and structured data, which reading a page normally leaves out. When a bot lacks access to an app such
  as Google Search Console, it offers you a connection card instead of stopping, and carries on with the
  rest. Bots are told to try the fallback first, such as the PageSpeed website when the API is
  rate-limited.
- **Connect an app from a bot's card.** When a bot offers a connection (such as Google Search Console),
  its card has a Connect button. It opens the app's sign-in in your browser (in the Mac app too), shows
  "Connected" once the sign-in finishes, and offers to open the sign-in again. Without a usable Composio
  connector, the card says what's missing and opens Connections instead.
- **Payments always ask.** A bot's tool call that looks like paying now shows an approval card even
  with approvals off, even when "Ask only when risky" finds it safe, and even after "Always this
  session". Allowing a payment covers that one call. kwant recognises payment apps and Composio
  actions (Stripe, PayPal, Square, Wise, Coinbase and others), commands that post to payment
  services or send crypto, and in its browser pay buttons, any click on a checkout page and typed
  card numbers. It can't catch everything, such as a plain "Continue" on an unusual checkout page or a
  card a site already saved: see `docs/payments.md`. Sending, deleting, publishing and signing in still
  run without asking when approvals are off.
- **Approvals: the risks are said before they turn off.** Choosing "Don't ask" for a space or a bot
  now says plainly that bots can then send messages and emails, delete files or data, publish and sign
  in to your accounts without asking, that payments still ask you first, and that bots with access to
  money or accounts are better kept on "Ask" or "Ask only when risky". Nothing changes until you press
  Turn off approvals; Cancel (or Escape) keeps the current setting. While approvals are off, a note next
  to the setting keeps saying so.
- **Approvals: payment cards say why.** A card for something that looks like a payment says so and
  why ("This looks like a payment: …"), and offers only Allow Once and Deny: allowing a payment covers
  that one payment, so there is no "Always" button on it. Other cards are unchanged.
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
- **Only you change what goes into a bot's prompt.** Approving, rejecting, making or editing a skill,
  writing a bot's notes, and changing team or channel instructions are now refused when a bot calls the
  API with its own token; bots keep proposing skills and saving their own notes with their tools.
  Notes that came in with an import are labelled "imported" for the bots instead of "written by the
  person".
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
- **A waiting approval card's Deny / Allow buttons no longer hide under the message box.** When a
  card's details arrived a moment after it appeared, or the message box grew, the chat didn't scroll
  with it, so on a phone the buttons sat below the box until you scrolled. While you follow the chat it
  now stays pinned to the bottom. If you've scrolled up to read, it stays where you are and shows an
  "Approval waiting" button above the message box that takes you to the card.
- **A task no longer sits in "Working" with no bot on it after you answer a card.** When you allowed an
  action on a task's card, the bot's "done, please review" right after it could be refused as "waiting
  on you", and the task then stayed in Working with nothing running it. Answering a card now moves the
  task back at once, and a bot's hand-in or the end of its turn no longer waits for that.
- **VM browser containers no longer pile up and come back after a reboot.** Each kwant data folder
  started its own browser container set to restart with Docker, and nothing removed it, so test and
  eval instances left dozens running. The container now never restarts on its own and is stopped when
  kwant shuts down. At startup and every 10 minutes kwant removes browser containers whose data folder
  is gone and stops ones unused for 30 minutes (`KWNT_BROWSER_IDLE_MINUTES`). It starts again on the
  next browser request; bots' cookies are kept.
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
- **Mac app: sign-in pages open in your browser.** Outside pages, such as a Google or Cloudflare
  sign-in, no longer take over the kwant window with no way back. The window only shows kwant; every
  other link opens in your default browser. Connecting an MCP server, a Composio app or Claude still
  finishes: the app notices the sign-in by itself, and the browser tab says to go back to kwant.
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

[Mac 0.3.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.3.0
[Mac 0.2.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.2.0
[Mac 0.1.3]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.3
[Mac 0.1.2]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.2
[Mac 0.1.1]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.1
[Mac 0.1.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.0
