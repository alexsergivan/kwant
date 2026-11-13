# Changelog

Notable changes to kwant, newest first. Version numbers follow the Mac app releases
([GitHub releases](https://github.com/alexsergivan/kwant/releases)). The server (`kwnt`) and the web
app ship from `main`, so changes under **Unreleased** are already on `main` but not yet in a Mac download.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Added
- A bot's own browser now saves what it downloads straight into its folder (`downloads/`, never overwriting an existing file) and reports progress as it reads the page; a new `browser_upload` tool answers a file picker the page itself opens, with a file from the bot's folder or a chat attachment.

### Fixed
- In the live browser view, a page's own file picker (TikTok Studio's "Select video", for one) now opens kwant's own **Choose file…** sheet instead of doing nothing, since the screencast can't show Chrome's native dialog. Choose a file, watch it upload, and the page gets it as if you'd picked it yourself.
- The page's own file picker: dropping a file onto the sheet, a focused OK when the page itself closed the picker, a Cancel while uploading, a byte-count progress bar, and a failed answer (not just a lost picker) can now be retried instead of always going "gone". (#578)

## [Mac 0.9.7] - 2026-11-13

kwant 0.9.7: your bot itself at work while a reply is coming, a mark on each bot's avatar for the AI it runs on and one search box to find a tool and add it.

### Added
- **Your bot at work while a reply is coming.** Before the first words of a reply arrive, the bubble shows the bot's own avatar at a small pixel laptop, typing, with "Ben is working on it" next to it, instead of three grey lines. In a team chat it's the bot that is speaking. The first words fade in where the picture was, at the same height, so nothing jumps. With reduced motion on, the picture stands still, and screen readers hear "Ben is working on the answer". Phones keep the status strip. (#562)
- **Each bot's AI on its avatar.** A small mark at the bottom left of a bot's avatar shows the AI it runs on: Anthropic, OpenAI, xAI, Ollama, a chip for a custom engine, or ✦ when Laya picks the model. While a bot works, the mark shows what it runs on right now and changes if the bot moves to another engine. A resting bot that Laya picks for shows its last reply's mark, faded, with a small ✦. A pinned bot whose engine is signed out, off or not set up is greyed, and the tooltip says why, for example "Runs on Claude · Sonnet · signed out"; running out of usage doesn't grey it. The sidebar name tooltip says the same. For apps: a `turn_runs_on` event when a turn starts and on failover, and `last_ran_on` for each bot in `GET /api/kwants`. (#554, #560)
- **Find a tool.** One search box finds a tool to add, in Add a tool and on Apps & accounts. Suggestions start from the first letter and forgive typos and short names ("gmial", "11labs", "pg"). Results come in three groups: Built in, Through Composio, and From the public directory, a copy of the official MCP Registry that kwant refreshes every 6 hours. Add & test installs a tool from there in one step. Connecting a Composio app before Composio is set up asks for the key right there. The Composio card shows the apps connected and which bots use them, with Replace key, Turn off and Remove. (#552, #555)

### Fixed
- Apps & accounts › Another app › Through Composio now finds any app Composio has, such as ElevenLabs, not only its popular ones. (#568)
- Signing in to tool servers that accept a client ID document instead of letting apps register themselves, such as ElevenLabs, works now. kwant uses kwant.si/oauth/client.json when it runs on this computer, or serves its own at /api/mcp/oauth/client.json over https. A server it still can't sign in to says why and what to do instead, with no Retry. (#574)

### Fixed
- **Find any Composio app by its name.** Apps & accounts › Another app only searched Composio's 120 most popular apps, so "ElevenLabs" said "No Composio apps match". Now a name the popular list doesn't have is checked with Composio itself, through Composio Connect, or on the app's public page on composio.dev when Connect can't answer (only the app's name is sent). "ElevenLabs", "elevenlabs" and "Eleven Labs" all find ElevenLabs, ready to connect. When nothing is found, the line says why and what to try: the app's full name, trying again if Composio couldn't be reached, or asking a bot. Answers are kept for 10 minutes.

## [Mac 0.9.6] - 2026-11-13

kwant 0.9.6: a team suggested from your own words, an Inbox that shows what your bots finished while you were away, bots that carry on or ask you when they stop, one Settings card that says if you're on the latest version and a Chrome that sites see as normal Chrome, with a window for Google sign-ins.

### Added
- **A team from your words.** Write what your bots should do in the "What should your bots do?" box on the Get ready card, from **Describe your own** in the sidebar, or with "Describe what this team should do" in New team, and press **Suggest a team** (10 to 1000 characters). One of your ready AI accounts drafts it, without tools, usually in under a minute. The **Suggested team** sheet shows 2 to 6 bots with the lead first: untick or rename them, or **Try again** with the same words, then **Create team**, with Undo. A drafted team never gets tools, accounts or routines from the draft, and its pinned instructions quote your words. (#545, #551)
- **Done while you were away.** The Inbox has two groups, **Needs you** and **Done while you were away**. Done items stay for 7 days, seen or not, and the badge counts only the ones you haven't read. Each one shows what ran, when and why, then the answer with **Open in chat**, a **What Ben did** trail with times, and **Why**: what was asked, or the routine's instruction with **Edit routine**. Chips say "Read only · changed nothing" or "No checks set · Add one". **Looks good** and **Ask Ben to redo…** answer it, and Roll back is hidden when nothing changed. A routine's run folds into its review, a lead's `Done:` line brings a team job in, and an approval's title is its own question. In a routine's History, each run has **Open result ›**. (#539, #543)
- **A stopped bot carries on or asks you.** When a reply in your own chat stops before it's done, it either continues by itself, a limited number of times, or leaves you exactly one ask: one line in the chat with **Retry** or **Continue**, and one item in the Inbox's **Needs you** that says what stopped and why. The item closes when you write in that chat, when the bot's next reply there finishes, or when Retry or Continue runs. When kwant restarts in the middle of a reply in your own chat, that reply continues by itself once. If it can't, you get "Ben's reply was cut off when kwant restarted. Retry in the chat sends your message again." A failed hand-off or routine run keeps its one Inbox entry (a routine's now says why), and a mission task kwant stops gets one line in its chat. (#541)

### Changed
- **One card for your kwant version.** Settings has one **kwant bots** card in place of About and the list of earlier versions. It says "You're on the latest version" with **Check again**, "kwant bots X is available" with **Update**, or "Couldn't check for updates" with **Try again**; it never says you're on the latest version when it couldn't check. In the Mac app, **Update** installs the new version. On a server it opens **How to update**, with one command and **Copy**; on the Windows beta it links the new installer. A kwant server outside the Mac app now checks for a new version 5 seconds after it starts and then every 6 hours (`KWNT_UPDATE_CHECK=off` turns this off). The Mac app checks every hour, and again when you open Settings. (#547)
- **Google sign-in in kwant's Chrome.** kwant's Chrome now tells sites it is Chrome, with Chrome's own version details, and no longer marks itself as automated. kwant prefers an installed Google Chrome over Chromium, and nothing is injected into pages. Google still refuses sign-ins in a Chrome without a window, so the live view then says "Google only allows sign-in in a normal Chrome window." with **Sign in in Chrome**. On a computer with a screen, that opens Chrome in a window for the sign-in, and it goes back to no window once you're signed in. (#549)
- The Mac app build in CI gets 65 minutes instead of 45 (the whole job 105), because the universal build now takes 38-42 minutes and kept hitting the old limit, mac-v0.9.5's tag build included. (#561)

### Fixed
- A new space no longer shows the main space's Get started card with "Keep the starter bots: Architect, TechLead and DevOps". Outside the main space the card shows only while nothing can run there yet, and its step 3 reads **Keep the bots you have** with that space's own bots. (#553)
- When kwant's own Chrome closes right after it starts (a missing library, a sandbox refusal), kwant stops waiting at once and says "kwant's Chrome closed right after starting" with Chrome's exit code and last lines, instead of waiting out the debugging port and reporting a timeout. (#557)
- When a mission task's browser step finds kwant's browser busy, the task waits and tries again by itself, after 1 minute and then 2 minutes, and says so in the mission's chat ("retry 1 of 2"), instead of failing at once. Steps that act outward, such as sending, are never re-run. If the browser is still busy after both tries, the task fails as before, with one notification. (#559)

## [Mac 0.9.5] - 2026-11-13

kwant 0.9.5: apps and AI accounts on one settings page, a monthly budget for each bot, one clear line when a bot's connection stops working, and a team browser that recovers when it's busy.

### Added
- **Apps & accounts.** Connections, Tools and Accounts are now one settings page. Apps are tiles (Gmail, Google Calendar, Slack, Notion, Shopify, GitHub, and any other app you find through Composio), AI accounts say which bots use them, and MCP servers and Telegram sit under **Advanced**. Connecting an app takes 3 steps in one sheet: open Composio, paste your Composio link, and pick the bots that may use it. No bot is picked for you, and picking one turns Composio on for that bot. Old addresses, such as Settings › Connections, open the new page. (#542)
- **A monthly budget for each bot.** In a bot's settings, **Usage › Monthly budget** sets a limit in tokens, in dollars, or both. It's off until you set it. At 80% the bot's sidebar row gets a small amber meter, its chat gets a note, and the Inbox gets a Needs you item. At 100% the bot starts nothing new: its row and chat say "Paused: monthly budget reached", with **Raise the limit** and **Continue this month**. Settings › All spaces › Usage lists every bot's budget in this space. (#536)
- **One line when a connection stops working.** When a bot's tool fails because of the connection, the chat gets one line for it, for example "GitHub stopped working for Ben: it needs you to sign in again. Reconnect". It covers an expired sign-in, a Composio app that needs connecting again, a server that didn't start, and one that stopped responding. There's at most one line per connection per turn, and none when a retry within 20 seconds fixed it. Lines only you can fix also show in the Inbox, and the line adds "It works again." once a later call works. (#537)

### Changed
- **The team browser recovers when it's busy.** A page nobody has used for 10 minutes stops its videos and sounds and is frozen, and an idle team page is cleared after 2 hours, with its sign-ins kept in the team's cookie jar. When the browser is busy, kwant tries again for up to 45 seconds and closes other idle pages to make room. If it still can't, the bot gets "The browser isn't responding right now… retry the same step in a minute" instead of a Docker error, and the live view says "The browser isn't responding. Reconnecting…". The Docker message now appears only when the browser really can't be reached, and checking the browser's status never opens a page. (#544)
- **No daily cap on Dev mode model calls.** Dev mode no longer stops at 30 model calls per space per day ("Today's 30 Dev mode model calls for this space are used up"). Its calls count toward the space's daily spending stop and each bot's monthly budget, like any other turn. Cards that failed on the old cap in the last 3 days are written again when kwant starts. A server can still set a cap with `KWANT_DEV_CALL_CAP`. (#550)
- **Fix it works on a person's PR.** On a Dev mode card for a PR that a person wrote, **Fix it** and line notes used to go nowhere. Now they go to the lead of the team that owns the repo. The lead is told to push new commits and never rewrite the person's, or to open a draft PR that links the original when it can't push there. If no team owns the repo, the card says to add it under Manage team › Git Repo. (#546)
- **Bots ask you to connect an app with a card.** When a bot needs an app you haven't connected through Composio, it posts a card for you to connect it instead of pasting a sign-in link into the chat. kwant's server can now also set up Composio from your API key alone and list Composio's whole app catalogue; a key Composio refuses gets a clear message and nothing is saved. (#548)

## [Mac 0.9.4] - 2026-11-12

kwant 0.9.4: bots don't stop after a failed step. They try again or another way, and when nothing works they say what's blocked and what would unblock it.

### Changed
- **Bots try again after a failed step.** When a step fails or is refused, for example a command that errors or a terminal that's turned off, a bot tries it once more, then tries another way to get the same result. A step you declined isn't tried again. Before, a bot could end its turn on "I'll check the file sizes next" with nothing run after it.
- **A clear ending when a bot is stuck.** If nothing works, the bot's last line starts with `Blocked:` and says what is blocked and what would unblock it, for example "Blocked: terminal commands are off for this bot on the shared Mac. Allow Terminal for this team on that computer, then say continue." When a team's lead ends this way, you get a question card with that line, and your answer lets the team carry on (#535).
- **One more turn when a reply stops on a plan.** If a step failed and the reply still ends on "I'll…" or "Let me…", kwant gives the bot one more turn to do it or say what blocks it. If that turn only promises again, the reply ends with a short kwant note that it stopped after a failed step.

## [Mac 0.9.3] - 2026-11-12

kwant 0.9.3: each bot job gets its own page in its team's browser, so a sign-in that one bot waits on no longer stops the team's other bots.

### Changed
- **Each job gets its own browser page.** Before, a team's bots shared one browser page, and a sign-in or CAPTCHA step held it for the whole team: while one bot waited for you to sign in to TikTok, another bot in the same team couldn't open Google Flow. Now each bot gets its own page in the team's browser for each chat it works in, and a step holds only the page it's on, so the team's other bots keep working. The pages share the team's sign-ins, so signing in on one page counts for all of them. A team has at most 4 job pages at once. A page nobody has used for 30 minutes closes, and a page whose bot is still working is never closed. A 5th job waits up to a minute for a page, then says "All 4 of this team's browser pages are in use by other jobs." Your own browsing, and bots working outside a chat, use the team's shared page as before. (#531)
- **The browser view shows the page that needs you.** When a team has 2 or more pages, a strip above the address bar lists them by bot and site, for example Architect on tiktok.com. The team's own page is "Shared", and a page that needs you has a yellow dot. The view goes to the page of the newest open step, and **Open browser** or **Solve it** on a card opens that card's page. Pick a tab to watch another page: a new step then marks its tab without moving you. While you have control, the other tabs wait ("Hand back to switch pages"). The address bar opens sites on the page you're looking at. (#532)

## [Mac 0.9.2] - 2026-11-12

kwant 0.9.2: teams keep going until the job is done, "Continue with Google" and other sign-in popups open in the bot's live browser view, and Bitwarden accounts with two-step login connect.

### Changed
- **Teams keep going until the job is done.** You no longer have to type "continue" when a lead stops halfway through a job. After a teammate reports back, each turn of the lead now ends in one of three ways: it hands the next step on (or does it itself), it asks you with a question card, or its last line starts with `Done:` and says what it checked. If a lead stops without any of these, kwant gives it another turn that quotes your request, at most 2 for each message you send. After that you get one card, "<Lead> stopped before the job was done: … Keep going?", with **Keep going** and **That's enough for now**. A question on the last line of a lead's reply becomes a question card straight away. Writing in the chat or pressing Stop always wins. There is no switch in Settings for this yet: a `keep_going_turns` row in kwant's settings table changes the number of extra turns (0 turns them off, at most 5). The 64 hand-off **Continue**, the loop guards, budgets and approvals for sending, paying and signing in work as before. (#526)

### Fixed
- **"Continue with Google" and other sign-in popups open in the live view.** When a page opens a popup window, such as "Continue with Google" on TikTok, the bot's live browser view and its browser tools now move to the popup, so you or the bot can sign in there. When the popup closes, both go back to the page that opened it. Before, the popup opened out of sight and the button seemed to do nothing. Pages opened by anything other than the bot's own page are never followed. (#524)
- **A lead no longer misses a teammate's result.** If a new hand-off started just before an earlier teammate's result was reported, the lead could wait for the new one and never hear about the earlier one. (#526)
- **Bitwarden accounts with two-step login connect.** kwant now uses the same device for every try while you connect, so the code Bitwarden emails to confirm a new device is accepted. A two-step code that Bitwarden refuses now says the code wasn't accepted and keeps your master password, instead of reporting a wrong master password. Thanks to skullsdev. (#527)
- **A Telegram approval card no longer keeps an Allow once button that does nothing.** Before, if you switched off Allow approvals from Telegram while a card was still on its way, the card kept its Allow once button, and a tap was refused. Now the button is removed as soon as the card arrives. (#529)

## [Mac 0.9.1] - 2026-11-12

kwant 0.9.1: bots hand work to each other without stopping to ask you, hand-off cards catch up after a lost connection, and TikTok's QR sign-in no longer freezes the bot's browser.

### Fixed
- **Hand-offs keep going without you.** A lead can @mention the same teammate as often as the work needs. The old limits of 4 hand-offs to one teammate and 16 for one message are gone, so a chain no longer pauses for **Continue** at the 5th mention. Loops stop on their own instead: the same task sent word for word to the same teammate again is skipped with "already asked X exactly this in this request", and a teammate whose last 3 tasks here failed is skipped with "X failed its last 3 tasks here: do it yourself or give it to someone else". The lead sees why and carries on; your next message resets it. A task that failed may be sent again as it was. Only 64 hand-offs for one message still pause with a **Continue** notice, and after you continue, the count starts again. (#520)
- **Hand-off cards catch up after a lost connection.** When the app lost its live connection while a teammate worked, its card could stay on "Waiting for X" after the hand-off had finished. Now the app checks every open hand-off again when it reconnects, without a page reload. (#521)
- **TikTok's QR sign-in no longer freezes the bot's browser.** TikTok's sign-in page keeps asking to open its own app (`bytedance:`) from a hidden frame. kwant reloaded the page each time, which made a new QR code and asked again, until it gave up and the live view stopped taking clicks. kwant now blocks frames that try to open another app before Chrome asks, so the page keeps working, and it reloads a page at most once per app a minute. Web, `srcdoc`, `data:` and `blob:` frames still load. (#522)

## [Mac 0.9.0] - 2026-11-11

kwant 0.9.0: a second check and one place to review a bot's work, roll back even after you approved it, notes on diff lines and on the page a bot is looking at, a worktree for each bot task, and one order for what needs you.

### Added
- **Review a bot's finished work in one place.** Inbox › New has a **To review** group, highest risk first, and on a wide screen each item opens beside the list. An item shows its verdict (Pass, Fail, Needs you or Not checked yet), who checked it, a **Couldn't check** list with the reason for each, and folds for **Decisions**, **What done looks like** and **Evidence**. Press **Approve**, **Send back to Ann** (your note becomes a new criterion) or **Roll back**; **Review in Inbox** on the Mission board and on a Dev desk review card opens the same item. If an action is held, you Deny it or Allow it once before Approve goes through. If the files changed after the check, nothing is approved and the item moves to Failed or stuck. (#503, #504, #506)
- **Approve the low-risk passes together.** When two or more low-risk items passed every check with high confidence, a **Low risk, checks passed** card offers **Approve all N**. It approves them one at a time, and **Stop after this one** stops it. An item with a held action, a Needs you verdict or anything it couldn't check is never in the batch. (#504, #506)
- **A second checker looks at the work before you do.** When a Mission board task moves to review, a teammate on another model (or on another tier of the same engine when no other is set up, and it says so) runs its checks again in a clean copy of the files. It never writes to the task's folder, and for high-risk work it also makes small changes to the code to see if the tests notice. It says Pass only when kwant ran a check again itself; a first Fail goes back to the bot, and a second Fail or a Needs you comes to you. It's on by default, up to 10 checker turns a day; to change that, set `verifier` and `verifier_daily_cap` with `POST /api/settings`, as there's no switch in the app yet. (#502)
- **Say what done means and what must not change.** A task can carry a contract: criteria marked Met, Not met, Not checked or Claim only, paths that must not change, open issues and risk flags. The bot and its lead fill it in with the `task_contract` tool; the bot can't drop or reword a must-not-change path, and a lead won't pass a criterion that has a claim but no evidence. You see the criteria in the Inbox item and can set the contract with `PUT /api/tasks/:id/contract`, as there's no editor in the app yet. On a low-risk task, changing a must-not-change path doesn't fail the check on its own: the Mission board task view shows an amber line, "Ann changed `config/prod.yml`, which this task said must not change. It was still approved because the task is small." (#502, #514)
- **Roll back work, even after you approved it.** **Roll back** puts back only the files the task changed, as they were before the run, and lists what already left this computer and can't be undone. A safety checkpoint is taken first, so you can **Undo the roll back**. For a verified task the approved record stays: a copy of the task is made from before the run and waits on hold until you press Run. Rolling back approved work shows when it was approved and moves the bot down one Flight School tier for that kind of work, once per approval, and only you move it back up; a task whose PR was merged from the Dev desk can't be rolled back here. (#502, #503, #515, #516)
- **Point at the part of the page you mean.** In Watch live, press **Point** in the browser pane, then click the part of the page you mean (tap on a phone; ↑ picks a wider part, Esc stops). It goes into your message as a chip, up to 5 per message, and the bot gets the page address, a selector, the element's HTML (trimmed), its size and styles, and a picture of it. Point only reads: it needs no control, never clicks or types in the page, and the page's own scripts can't see it. It works in Local Chrome too, is hidden while you've taken over, and doesn't reach inside shadow DOM. (#487, #489)
- **Leave notes on diff lines and send them together.** On a Dev desk review card, press the button beside a line, or shift-click or drag for a range, to write a note. Notes stay on the card as drafts until you press **Send 3 notes to Ann**, and the bot gets them in its chat with you as one message, with each file, line and code. After its next push, each note says "Changed in …" or "Still open". Nothing is posted to GitHub. (#488, #491)
- **CI failures and review comments go back to the bot that made the PR.** When a check fails or someone leaves a review comment on a bot's pull request, kwant sends it to that bot as one Fix-it item in a **Fix-it: owner/name #N** chat, with the end of the failing check's output or each comment with its file and line. Each thing is sent once, at most 3 Fix-it turns per PR a day; past that it waits in Needs you, and archiving the chat stops it for that PR. When the PR goes green, a fresh-eyes reviewer that didn't write it posts MERGE, HOLD or DO NOT MERGE in that chat as advice. Nothing here merges, pushes or writes to the PR. (#490, #509)
- **A worktree for each bot task.** The Dev desk has a **Worktrees** tab, grouped by repo, with a count of what needs you and what's working on each repo's header. **New worktree** makes one from a new or existing branch, for a bot or for "Nobody (just me)"; with a bot, a chat named after the branch opens, and in that chat the bot works in the worktree's folder. **Remove** never deletes the branch, and a checkout with changes lists them and needs "Remove anyway and lose these N changes" ticked. Only you can make or remove a worktree, and ones made outside kwant are listed but can't be removed here. (#486, #493, #510)
- **Build folders are cleaned an hour after a task.** In a worktree kwant made, an hour after the task finishes, kwant deletes only `node_modules`, `dist`, `target`, `.next` and `.turbo` folders that git ignores and that hold no tracked file. The worktree's line says when, with **Keep it** to skip it and **Clean now**. It never cleans inside `.git`, through a symlink, or while a bot works there. (#486, #493)
- **Watch a mission and check what it claims.** On the Mission board, press **Watch** on a mission and pick the bot that checks. When the mission stops, that bot gets one turn to check each claim with the browser, page source, attachments and facts, and anything that would write asks you first. A claim that doesn't hold sends its task back, the report shows "Claimed vs checked" and what the watcher did, and anything that needs you is in the Inbox. From the chip you can **Check again now**, **Change who checks** or **Stop watching**. (#511, #513)
- **Bots leave notes about their tools.** When a tool didn't work, has a tip, or is missing, a bot can file a note, and a grey line in the chat says so. Settings › Connections starts with **Notes from bots**: filter by Didn't work, Tip, Missing or Fixed, **Add a note** yourself, and **Mark fixed**, edit or reopen one. Bots with the same tools get the open notes in their later turns, at most 10, marked as hints, and your edit wins over the bot's. A bot can add 5 new notes a day. (#507, #512)
- **Set a monthly budget for a bot (API only).** A bot can have a monthly limit in tokens, dollars or both, set with `POST /api/kwants/:id/budget` and `{"budget_tokens": …, "budget_usd": …}`, where 0 turns a limit off; there's no setting in the app yet. At 80% you get a note in your chat with the bot. At 100% it finishes the turn it's on, then starts nothing new, and later messages wait until you raise the budget, continue for this month (`POST /api/kwants/:id/budget/continue`) or the next month starts. Other bots keep working, and one long turn can go past the limit. (#495, #508)
- **Messages to a busy or stuck bot wait, and say why.** When a bot is waiting on you in another chat, on an approval or a browser step, or is stuck, your message to it is saved at once and its turn waits. A line under your message says why, such as "Queued for Ann: Ann is waiting for your answer to its question (14:02). It goes through when you answer." If the bot stopped after 2 tries, the line has **Try again**. Hand-offs from other bots wait the same way, and the bot that sent one is told. (#499, #510)
- **See when a bot is stuck or quiet.** A bot stuck on its AI is listed under Stuck with the reason, such as "Stuck: Claude stopped answering" or "Stuck: stopped after 2 tries · 14:05". When a turn shows no step for 15 minutes, you get one notification, "Ann is quiet". For scripts, `GET /api/bots/status` gives each bot's status with its reason, since when and its last step. (#499, #510)

### Changed
- **One order for what needs you.** The sidebar, the Inbox and the phone's Chats tab sort bots the same way: needs you, stuck or failed, to review, finished but not read, working, waiting. On the desktop a **Needs attention** block sits above Channels with up to 5 rows, then "and 3 more in Inbox", and team headers show dots for what's inside. In Needs you, the oldest ask comes first. (#494, #510)
- **A bot's other chats wait while it waits on your answer.** While a bot has a question open for you, messages to it in its other chats wait. A message in the chat where it asked is still the answer. (#499)
- **Review on the Mission board goes through the Inbox.** A task in Review shows its verdict and a **Review** button that opens it in the Inbox, in place of Mark verified and Send back. Dismiss and Try again stay on the card. (#506)
- **Changing a bot's model starts its Flight School scorecard again.** The scorecard counts only tasks since the change, or the last 30 days if that is shorter. (#502)

### Fixed
- **A retried send arrives once.** For apps and scripts that use `POST /api/chat/send`: with a `client_message_id`, a second send with the same id in the same chat within 10 minutes answers `already_delivered` and saves nothing, even when the two sends race. kwant's own composer doesn't send one yet. (#499, #509)

## [Mac 0.8.2] - 2026-11-10

kwant 0.8.2: files dragged in from Finder or Explorer land in the chat again.

### Fixed
- **Dropping files on a chat works again.** In 0.8.0 and 0.8.1, dragging files from Finder, Explorer or your desktop onto a chat showed the drop card, but letting go did nothing: no file chips and no upload. The card closed a moment too early, before the chat got the files. Now the files land in a bot's chat, a team chat, #general and a message to an @mention. A new test drops real files through the browser's own drag path, which the old tests didn't. (#497)

## [Mac 0.8.1] - 2026-11-10

kwant 0.8.1: fixes from the 0.8.0 checks. A "needs you" notification opens the chat it came from, a team chat points to the pre-push checks its bots are waiting on, and each bot looks the same everywhere.

### Changed
- **Pre-push checks show in team chats.** In a team chat or channel, each member bot's newest pre-push card from the last 24 hours shows as one line: the ones that need you first, at most 3, then "+n more in the Inbox". Open goes to the card in the bot's own chat, where its buttons stay. (#483)

### Fixed
- **A "needs you" notification opens the chat it came from.** Clicking a notification that a bot needs you (an approval, a question or a step only you can do) now opens the team chat, channel, thread or direct chat where it asked, and an approval scrolls to its card. This works for the web app's notifications, a phone's and the Mac menu-bar pill's. If that chat is gone, it opens the bot's direct chat and says so. (#485)
- **One look per bot.** A bot now has the same avatar in the sidebar, its replies, the working line, the cards and the Ask chips. Before, a bot whose name suggested another look could look different in its replies, as TechLead did in teams made from the coding template, which saves it with the Architect look. (#492)
- **A follow-up no longer asks again for a picture already shared.** When a change takes more than one turn, kwant counts the pictures any of them shared, so a continuation doesn't end with "No picture was attached". (#482)
- **"Try again" after a change picks the work up again**, even after you cancelled it. So do "again", "retry" and "do it". "Stop", "never mind", "cancel" and "no" don't. (#484)
- **The working line names the bot** ("Ann is getting started") when you open a chat while it's replying, instead of the chat's name, and on a phone it sits in the message box as on the desktop. A queued message's last word no longer sits alone on its own line. (#481)

## [Mac 0.8.0] - 2026-11-10

kwant 0.8.0: bots that do the work and show it, bring your bots from more places, a second pair of eyes before code is pushed, and drag and drop into any chat.

### Added
- **Bots that show their work.** When you ask to see something, the bot shares the screenshot in the chat as its own picture, under its reply, once (no repeats, no "@space" sender). A change you can see, such as "the button isn't aligned", gets a before and an after picture. A reply that says "here's a screenshot" but shared none gets the turn's last screenshot added, or says plainly that it has none. Before a bot starts on a message in a team chat, kwant works out what you expect back (a picture, a link, a file or an answer) and checks the reply delivered it; a miss is picked up once on its own when your words asked for it. (#468, #469, #474, #477)
- **Bots act instead of asking.** The approvals setting you chose now reaches every engine (Claude, Codex, Grok and API keys). With approvals off, bots do the work instead of offering it, look first instead of asking you what they can find, and ask one decision at a time. They still ask for sign-ins, keys, payments and anything that reaches other people or can't be undone. (#470)
- **Follow-ups keep going.** A short follow-up such as "show me" or "again" keeps the effort of the work it continues. A second message to a bot that is still answering in the same chat waits its turn, and a grey "#1 in line." line under the running reply says so. A turn that stalls is retried once on its own. (#467, #476)
- **Drag and drop into any chat.** Drag files from Finder, Explorer or your desktop anywhere onto a chat: the whole chat is the drop target, and it says which bot gets them and what can't be sent before you let go. It works in the web app and in the Mac, Linux and Windows apps. (#478, #479)
- **Bring your bots from more places.** Import now reads OpenClaw and Hermes Agent workspaces, Codex's AGENTS.md, Paperclip companies and ai-employees, as well as everything it read before. **Found on this computer** (desktop apps) lists the agents, skills and workspaces in the usual folders by name and count only, and reads nothing until you pick one. **Folder** packs only the files an import reads, never sign-in or key files. **GitHub link** imports from a public GitHub repo and shows its licence first. Imported routines arrive paused, and the preview says what was left out. (#455, #459, #471)
- **Checked with fresh eyes.** Dev mode reviews and the Swarm Auditor now run in a fresh turn with no chat history and no reviewer memories, so a reviewer judges the work, not the conversation. Before a bot's `git push`, a **pre-push card** runs the checks (secrets, new packages, weakened tests) and lets the push go only when they're clear. **Push anyway** needs your reason, covers that one push only, and is saved on the card. See docs/dev-mode.md. (#456, #458, #475)
- **Pre-push card follow-ups.** kwant's own GitHub tools that write files to a branch (`push_files`, `create_or_update_file`, `delete_file` and Composio's GitHub file tools) now wait for a pre-push card too, and must name their branch. An override now lets exactly one push go: the next push of the same commit waits for the person again. A plain `git push <remote> [<branch>]` of a branch runs pinned to the commit kwant checked; a tag or any other ref (`refs/notes/…`, `refs/pull/…`) runs as written, gated by its card. GitHub tools now meet the production floor like `git push`: a file write to a protected branch, every pull-request merge, moving a protected branch's ref (`GITHUB_UPDATE_A_REFERENCE`), creating or moving a tag ref (`GITHUB_CREATE_A_REFERENCE`, `GITHUB_UPDATE_A_REFERENCE`), and deleting a protected branch or a tag ask the person. A check that errored (not a reviewer's own finding) runs once more on its own. A lead's review of a hand-off is labelled "Checked with fresh eyes" when it ran in a fresh-eyes turn, and the review kwant posts for a PR says when the person pushed anyway, with their reason. Fresh-eyes turns now get only an allowlist of kwant's tools, so a new tool is off in them until it is added. **Known limits:** the card doesn't see shells kwant doesn't run (Codex's own shell, VM mode), git aliases, `gh`, `git.exe`, `"$(which git)"` or scripts that push, or a person's own Claude settings that allow `git push`; tokenless loopback calls to person-only routes need `KWNT_REQUIRE_TOKEN=always` to close; a command substitution runs before the push it sits in; and a bot's non-kwant MCP servers stay on in fresh-eyes turns. See docs/dev-mode.md. (#456 follow-up)
- **kwant's own browser on Windows and Linux ARM.** When no Chrome is found, kwant can now download Google's Chrome for Testing on 64-bit Windows and on Linux ARM (aarch64) too, as it already did on Mac and Linux x64. On Windows it's kept in `AppData\Local\kwant\chrome` in your user folder. (#457)

### Changed
- **The working line sits in the composer.** On the desktop, "Architect is getting started · 0:04" is one quiet line just above the message box, lined up with it, instead of a card above the composer. (#464)
- The queued chip is grey, like the queued line, and the "Files changed since this preview" message stays until you act on it. The key protection setting is named **Protect your keys from tricked Claude bots**. (#460)

### Security
- **On a Mac, Claude can't read your sign-ins or kwant's secrets.** When a Claude turn runs on the Mac itself, kwant denies its tools the Claude, Codex and Grok sign-in folders, the Keychain folder and kwant's own vault, token and database. A stricter sandbox is available with `KWNT_MAC_SEATBELT=1`. (#463)
- Folder walks for undo points stop at a depth and size limit, only kwant's helper may start git for a bot, a folder an isolated bot used can be forgotten, and a bot in key protection gets read-only git hooks. (#462)

### Fixed
- **A bot's Enter now submits forms.** When a bot pressed Enter in kwant's browser, the page saw the key but a search box or login form never sent. Enter, Tab, Backspace, Space, the arrows and single characters now go out as a real keyboard sends them. A key kwant can't press, such as `Control+a` or a whole word, is refused with a hint to type text instead. (#457)
- Every pull request now builds the web part of `docker compose up --build`, so a source build can't break unnoticed again as it did in 0.7.6. (#457)
- **Deleting a bot no longer fails with "database error".** A team or channel the bot led could list a bot that no longer exists, or the chat search index could be out of step with the chat. Either one stopped the delete. kwant now skips the missing bot when it picks the new lead, and repairs the search index and tries again. If a delete still fails, the dialog says why in plain words. (#465)
- **Undo of a ready-made team or an import** works even when the chat search index is out of step. (#473)

## [Mac 0.7.7] - 2026-11-09

### Added
- **Run Claude inside the VM (Linux, off by default).** In VM mode, a new switch under the VM card, **Run Claude inside the VM**, runs each bot's Claude replies in a container of its own with no network. Your real key or sign-in stays outside: kwant's broker passes on only the Anthropic calls Claude needs and refuses the rest. Each reply may make 200 model requests. Claude's own web search and fetch are off while isolated, so bots use kwant's browser tools instead. Codex, Grok and Claude API-key accounts still run on this computer, and the chips under the switch show where each one runs: Isolated, Stopped or Not isolated. If isolation can't start, the reply stops with a notice card that says why and links to the setting, and nothing runs unprotected. When Docker isn't running the switch is greyed out, but a switch that's already on can still be turned off. A conversation waiting for a bot's other replies now shows a grey "In line" line. See docs/agents-in-vm.md for what it does and doesn't protect against. (#446, #450)
- **kwant for Windows (beta).** kwant now has a Windows app for 64-bit Windows. Each release has an installer, `kwant_X.Y.Z_windows_x64_beta_setup.exe`, with its SHA-256. It installs for your user only, so it needs no admin rights, and it runs kwant on your PC with the same data folder as `kwnt serve` (`.kwnt` in your user folder). The installer isn't code-signed yet, so the first time you run it Windows SmartScreen warns about an unrecognized app: click **More info**, then **Run anyway**. In the beta, the app doesn't update itself (a new version is a new installer), and Grok, the isolated computer (bots work on your own computer instead) and looking inside videos aren't available on Windows yet. Dev mode works, with one difference: in a folder that OneDrive syncs with "Files On-Demand", kwant can't safely read the repository's git files, so Dev mode asks you before every push from that folder. (#410, #451)
- **Get started offers to bring your bots.** Step 3, "Pick a team", has a new **Bring your bots** tile next to "Keep the starter bots". It opens the same importer as Space settings › Import bots, so you can bring a bot from Grok Bot, ChatGPT or Claude, or from a file, and check a preview first. A finished import completes step 3. Closing the importer without importing leaves step 3 as it was. (#449)
- **Import several bots at once.** The importer's "Upload a file" now takes several files in one pick or drop, for example every Grok Bot export you have. You don't need to zip them yourself: kwant packs them into one zip in your browser and shows one preview with every bot, plus the names of any files it couldn't read. A pick is limited to 500 files and 20 MB in all, and a zip can only be imported on its own. (#449)

### Changed
- **Dev mode reviews only fresh work.** A branch or PR gets a review card only if it moved in the last 7 days, so old branches no longer fill the desk or spend model calls. Cards on branches that went quiet go stale, and a new push brings a branch back. The newest branches are reviewed first, and Dev mode's status says how many old ones it skipped. A card that couldn't read a file now says "Couldn't read N files (no diff available)" instead of "(no diff from GitHub)". (#447, #445)
- **Every release is tested on a brand-new computer.** Before a release ships, CI starts kwant as a brand-new user on an empty machine and checks the first minutes: Get started shows, Claude installs in one click, and its sign-in starts. (#418)

### Fixed
- **A type change no longer leaves every file unread.** When a symlink in a branch turned into a file, Dev mode's review card marked every file in the change as unread. Now only that file is affected. (#448)
- **A checkpoint's git commit only comes from the task's own git files.** When a mission takes a checkpoint, kwant notes the commit the task's folder is on. It now never follows a symlink inside `.git`, ignores a `ref:` that points outside it, skips FIFOs and oversized files, and keeps the value only when it's a real 40- or 64-character commit id. Before, a bot that could write its task folder could make a checkpoint show the first 64 characters of another file kwant can read. Restoring a checkpoint now opens the task's folder strictly, so a folder swapped for a link is refused and nothing changes. (#443, #446)
- **Dev mode reads the checked-out branch safely.** The production safeguard reads which branch a bot's folder is on. A planted FIFO could stall that check, and a symlink, a hard link or a `.git` file could point it at another file. It now reads `.git` only in the bot's folder and below, never through a symlink, counts a `.git` folder as a repo only when it has `objects` and `refs` as git does, and doesn't follow a worktree's pointer out of the folder. When it can't tell the branch, a plain `git push` asks, as before. Checkpoints also skip a hard-linked git file. (#444)
- A flaky test is steady again. (#452)

### Known issue
- Undo points saved by kwant 0.7.6 or earlier in a folder reached through a linked (symlinked) parent folder can't be restored in 0.7.7. kwant refuses safely and changes nothing. New undo points work.

## [Mac 0.7.6] - 2026-11-08

### Added
- **Recipes as cards.** When a reply holds one recipe, with an ingredients list and the method, it shows as a recipe card. The card has the ingredients in their groups, numbered steps, and any times and notes. The text before and after the recipe stays as it is. The servings stepper (1 to 24) rescales every amount it can read, in kitchen fractions such as ¼, ⅓ and ½, and highlights each changed amount, with a **Back to N** button. Oven temperatures, and anything it can't read, stay as written. A screen reader hears the old amount after each change. On a wide window the ingredients and steps sit side by side. If a reply doesn't match a recipe's shape exactly, it stays plain text, and **Show as text** always gives the reply back word for word. (#424, #417)
- **Get started.** A new person's #general now starts with one Get started card in three steps: **Choose your AI** (Claude, ChatGPT, Grok or an API key), **Sign in**, and **Pick a team**. Choosing an AI installs it when it's missing. The card replaces the old welcome. (#425)
- **Sign in to ChatGPT and Grok inside the app.** ChatGPT (Codex) and Grok now sign in with a device code. kwant shows the link and a short code, you approve in your browser, and the sign-in finishes on its own, with no Terminal needed. You can sign out from the app too. Claude's sign-in works as before. (#419)
- **Codex without Node.** On a Mac or Linux computer without Node, kwant installs Codex from OpenAI's standalone release. It checks the download's SHA-256 before anything is unpacked, and refuses a mismatch without changing anything. (#420)
- **The bots' private browser without Node.** When the computer has no Chrome, kwant downloads Chrome for Testing into its own cache the first time a bot needs the browser. Bots that need the browser before it's ready wait for it. It stays quiet unless the download fails, and then Settings says why. (#422)
- **kwnt on a Linux server in one line.** `curl -fsSL https://kwant.si/install.sh | sh` installs the kwant server on Linux, on x86_64 or arm64, with Ubuntu 22.04 or newer, or Debian 12 or newer. It never uses sudo. It checks the download's SHA-256 before installing anything, starts kwnt as a user service (or in the background), and ends with your sign-in link. For a server that only listens on itself, it also prints the SSH tunnel command to reach it from your laptop. The container image is `ghcr.io/alexsergivan/kwnt`. (#436)
- **What's new, in the app.** After an update, a sheet shows what's new and what's fixed, once on each device, and on the web app when its server updates. **See what's new** on the "Updated" banner opens it, and Settings › About keeps every version's notes. (#421)

### Changed
- **kwnt reports its real version.** `kwnt --version`, the server's health check and its update status now show the release's version instead of 0.1.0. (#423)
- **First time on a Mac?** On kwant.si, Download for Mac now opens the three Open Anyway steps for the first launch, with a picture. The install guide shows the same steps. (#413)

### Fixed
- **Take over says when you're already controlling the browser in another window.** It tells you to press Hand back there, instead of retrying every second. When other chats are browsing, the Hand back button's tooltip says "Hand back first to switch browser". (#429, #434, #412)
- **Undo for a ready-made team names the exact archived chat it keeps**, including a -2 or -3 when that name was taken. (#433, #435)
- **A Jira ticket whose lead can't prepare a brief says why.** A brief is prepared only on Claude or Codex with tool checks on, and the card says so. The brief toggle and See the review now name their ticket. (#427, #431)
- **Show it once.** A failed draft shows a plain sentence instead of the engine's error. An empty trial reply says so. A draft that fails its checks says what went wrong in plain words. A start page that doesn't load records the page you were on. A password or code field you leave by typing an address is still recorded as a masked step. Readable path names in recorded addresses are no longer scrubbed. (#426, #430, #437)
- **A GitLab address without .git now works**, just as a GitHub address does. (#428)
- **After "folder in use", a ready-made team's main button says Pick another folder** instead of Try again. (#411)
- **Telegram says "your message wasn't run" only for your own message**, not when a routine or a Jira task hit the spending stop. (#416)
- Three flaky tests are steady again. (#414, #415, #432)

## [Mac 0.7.5] - 2026-11-07

### Added
- **This Mac and your servers, side by side.** The Mac and Linux app now opens This Mac and every server you add, each in its own window, all at the same time. Nothing relaunches to change place. **Add a server…** in the menu bar takes an address or a sign-in link. A server stays listed, and signed in, until you remove it, so you never type it again. The menu bar lists each place with what's waiting there, or "Sign in to see what's waiting" or "Couldn't reach it". A link to another listed place opens that place's window; other links open in your browser. **Remove a server** asks first, then signs the app out on that server, so it disappears from that server's signed-in devices. When a server can't be reached, its window says why, with Try again and Remove this server…. **Run kwant on this Mac** (on by default) can be turned off if you only use servers. The list lives in `~/.kwnt/desktop-places.json` and holds addresses only, never a token. (#388, #389)
- **Ready-made teams.** Create an Inbox, Sales or Coding team in one step. The preview shows who's in it, lets you rename them, and names any clashes, its skills, its routine (off until you turn it on) and what the team will never do. Create makes the bots, the team chat, the starter skills and the routine together, and one Undo removes all of it. **Suggest members from the code** reads a team's repo and suggests 2 to 6 bots, each with a reason and the paths it saw, and says plainly when it can't read or can't tell. (#396, #398)
- **Work from Jira tickets.** Dev mode can connect Jira Cloud or Data Center (off by default; until you connect it, nothing changes). Tickets that match your rule appear on the Dev desk's Tickets tab. A bot reads each one, writes a brief with its sources and any questions, and waits for your OK before it works. The card follows the ticket through review to landed, and shows **Matches the ticket** only when every done-when line is met. A token Jira rejects, an address with no Jira, a token that sees no projects, and Jira asking kwant to slow down each get their own message. (#393, #394, #395)
- **Watch live.** A globe on a chat's row shows that its bot is using the browser right now; click it to watch the page. **Watch** on the working strip does the same. Each chat's live view has its own `/live` address, and a notification can open it directly. With two or more chats browsing, the panel's name becomes a switcher, with the chats that need you first. (#397)
- **Telegram.** Pair one Telegram chat with a space (off by default). Write to your bots from Telegram, and get their replies and what needs you (questions, approvals, steps only you can do, and stuck bots) on your phone. Answer questions with a tap or a reply. With **Allow approvals from Telegram** on, some approvals get **Allow once** on the phone, checked again when you tap; the rest open in kwant, with the reason. The bot token stays in the vault and never appears in replies, logs or a bot's context. When the daily spending stop holds a message you sent from Telegram, the chat says so once: press Continue in kwant, then send it again. Unpair has an Undo, and Disconnect deletes the token. (#399, #403)
- **Show it once.** Hold Take over and show a bot a task once while kwant records the steps. Passwords, one-time codes, card details, secrets and anything filled in by a saved login or a script are recorded as placeholders, never as their text. kwant drafts a skill from the steps, you review and edit them, and **Try it once** runs it in the bot's chat before you approve it. The trial never changes the skill. Picks from a dropdown are recorded like any other step, and each field is named by its own label. (#402, #400, #407)
- **Install Claude Code in one click.** When Claude Code, Codex or Grok's command-line tool is missing and kwant can install it, the start picker and the engine cards show **Install** instead of a link and a Re-check. When it's done, sign-in follows straight away. If it fails, kwant says so, with Try again and the old link. (#406)

### Changed
- **A quieter working line on the phone.** The "… is getting started" line is now one small line with space above the message box, and the Ask buttons above the box are smaller. Both are just as easy to tap. Desktop is unchanged. (#405)

### Fixed
- **The Dev mode switch no longer gets stuck in "settling."** A timer that fired a moment early left the switch ignoring every press. (#391)
- **After "Which is it?" in a forge connection form, keyboard focus lands on the first choice** every time, even on a slow machine. (#401)

## [Mac 0.7.4] - 2026-11-07

### Added
- **Daily spending stop.** Settings › All spaces › Usage can cap what all bots together use in a day, in fresh tokens and in dollars (dollars count only engines that report a cost). Both are off by default. Over the cap no new turn starts, from a person message, a hand-off, a routine, a goal, a mission task, AutoFix or a connected agent; turns already running finish. Chats get one notice a day with Continue, routines, missions and AutoFix skip with `spend_stopped` without counting as failures, Needs you shows one item, and the first hold each day sends one notification. Goal checks keep running, since they use no tokens. A turn that fails mid-way counts the tokens its engine reported so far (Claude CLI and API engines). The day follows your time zone and survives a restart. In the app, the two limits sit beside today's use, the chat notice and the Needs-you item have Continue today, a tap on the notification opens that item (any other notification about no bot opens home), and a run the stop skipped reads as a skip, not an error. (#375, #382)
- **Use kwant from your coding agent.** Claude Code, Codex, Cursor and other MCP clients can ask your bots for work through `POST /api/mcp/agents` and get the reply back. Each connection is made in Settings › All spaces › Coding agents, which lists them with their spaces, bots, created date and last use, shows recent activity, and gives copyable setup for Claude Code, Codex and Cursor. Each has its own `kag_` token (shown once, stored only as a hash), reaches only the spaces and bots you pick, and is revoked at once. The agent is never the person: its messages are marked as the agent's, it can't approve anything (it sees `waiting_on_person`), and its token gets a 403 on every other route. Each connection gets up to 60 asks an hour and 3 turns at once, and its turns count toward the chain budget and the spend stop. See [docs/agents-connect.md](docs/agents-connect.md). (#376, #378)
- **Dev mode on GitLab, Gitea, Forgejo and Codeberg.** Add the server as a connection in Loose ends (kwant asks it which forge it runs) and its merge requests or pull requests get review cards with their CI, like GitHub's. A repo with no forge, or no token for it, gets a review card for each pushed branch that's ahead of the watched one, with no CI and no Post. Posting reviews is still GitHub only. Forge requests take the same address checks as git fetches and never follow redirects. (#377)
- **GitLab, Gitea and Forgejo in the app.** Loose ends adds a GitLab, Gitea or Forgejo server with its address and a token (with an optional "Which is it?" when the server can't tell), and says plainly when that server is already connected. Dev desk repo rows show their forge, and a GitLab, Gitea or Forgejo repo with no connection gets a Connect GitLab (or Gitea, or Forgejo) button that opens the same form in place. Review cards show GitLab's MR !7, say that posting there comes in a later update, and a branch with no forge reads as a pushed branch. Team settings' line under Git Repo URL names GitLab, Gitea and Forgejo too. (#379)

### Changed
- **Bots don't paste your keys back to you.** When you give a bot a key, password or token, it still does what you asked with it, but in its reply it calls it by name or by its last 4 characters instead of writing it out in full. (#381)

### Fixed
- GitHub Enterprise: paste your server's address as it is (github.yourcompany.com or yourcompany.ghe.com); kwant finds the API itself. Repos on yourcompany.ghe.com now match their connection in Dev mode. (#380)
- **"Ask only when risky" now asks before a bot adds a new package.** `npm install left-pad` used to run with no card, even though it downloads code that runs on your computer. Now adding a package the project doesn't list yet asks first: `npm install <name>`, `pnpm add`, `yarn add`, `bun add`, `cargo add`, `uv add`, `poetry add`, `go get`. A plain `npm install` of what the project already lists still runs on its own, and the approvals guide now says the same thing in both places. `npm exec`, `pnpm dlx`, `yarn dlx` and `bun x` also ask when an option comes first (`npm --prefix app exec …` used to slip through); `npm run x` now asks too. It also asks when an unusual option or a workspace name sits in front (`npm --loglevel silent install left-pad`, `yarn workspace app add x`), for starters that download and run a package (`npm create vite`, `npm init vite`, `yarn create …`), for `deno run` of a web address, `uv run --with`, `uv pip sync` and `dotnet add package`, and for a pip or npm the bot set up inside its own folder (`v/bin/pip install x`). (#381)

## [Mac 0.7.3] - 2026-11-07

### Added
- NVIDIA preset for custom servers: free models from build.nvidia.com (#365).
- **Dev mode reads any git repo.** Repos on GitLab, Gitea or any git server, and team folders with no remote, are watched with git, and what lands there shows on the Dev desk. Reviews, CI checks and Post to GitHub still work on GitHub repos only. The server needs git 2.31 or newer; without it, the desk says "Needs git on the server". (#366, #368)
- **Connect GitHub in one place.** Every Connect GitHub (the Dev desk's status line and empty state, the Dev mode toast, Dev mode setup and Team settings) opens the same GitHub form right where you pressed it: a popover on the desktop, a sheet on the phone. It no longer jumps to Loose ends. The GitHub row in Loose ends says "Also used by Dev mode".
- **Team settings say what Dev mode does with the repo.** Under Git Repo URL, one line: watched with GitHub connected, watched but needs Connect GitHub for reviews, can be watched once Dev mode is on, or read with git (reviews need GitHub).
- **Videos in chat, without the wait.** MP4, MOV and WebM files up to 100 MB can be attached and shared by bots (other files stay at 20 MB), and they play right in the chat (no autoplay), falling back to the file card when the browser can't play them. The type comes from the file's bytes, not its name. A file kwant can't take is refused the moment it's dropped, with one line naming what works; anything that gets past the browser is refused by the server right after its first bytes instead of after the whole upload. A dropped connection or a 502/503/504 retries by itself twice (after 1 s, then 3 s) before showing Retry. Upload chips show "5.2 of 8.0 MB". With ffmpeg on the server, a bot reading a video gets its length and size and up to 6 frames; without ffmpeg it is told to install it. (#372, #373)

### Changed
- **Dev mode reads commit history with git, not GitHub's API.** Each watched repo gets a copy in kwant's data folder, up to 1 GB each; a bigger repo reads "Too big to read". Repo addresses that point at this server, link-local addresses or cloud metadata addresses are refused, and https reads are pinned to the address that was checked. A team folder whose .git points somewhere else is refused. GitHub PRs, checks and logins still come from GitHub. (#366)
- **Saving a token or password while some bots don't ask first is a choice, not a wall.** The card "Before you save the token" explains the risk and offers **Save it on this server**, **Use Bitwarden** or **Make every bot ask first**. You make the "Save it on this server" choice once for the server, and a bot can never make it for you; the warning that such passwords are exposed stays. Archived spaces, and the bots in them, no longer count. Turning approvals off, or restoring an archived space that doesn't ask, with passwords on this server offers the same one-time choice. (#370, #371)
- After a wrong master password on bitwarden.com or bitwarden.eu, the Bitwarden form asks whether the account is on the other one. (#371)
- A token form waiting on Bitwarden or approvals checks again when you come back to the window, so connecting Bitwarden in another tab unblocks it. (#371)

### Fixed
- **Connect in the Bitwarden form works again.** Inside the GitHub, email and approvals forms, Connect (or Enter) reloaded the page and sent nothing; it now connects. (#371)
- **Connecting Bitwarden works with accounts whose key salt isn't their email.** kwant now uses the salt Bitwarden's server reports, and also reads the account key from Bitwarden's newer login reply. Each failed connect or unlock logs one line with what went wrong, the HTTP status and the server's host. That line never includes the email, password or codes. (#370)
- A custom server's models that have no name now show their id in the model picker instead of "[redacted]" (#365).
- A watched repo whose server lost git now reads "Needs git on the server", with the install tip, instead of "Can't read it right now. Trying again." (#374)

## [Mac 0.7.2] - 2026-11-06

### Added
- **Dev mode watches every team's repo.** A space no longer has one watched repo. Dev mode watches each team's
  repo (its repo link, or its folder when that folder is a git checkout), up to 10 per space. When there's more
  than one, every card shows its repo, and Landed and Reviews filter by repo (All · app · site). Each repo is
  checked on its own: a repo that loses its GitHub connection or can't be found shows on its own row with one fix
  (**Connect GitHub**, **Link a repo** or **Remove**), and the others keep going. The production floor, the command
  guards, approvals and the daily review limit stay the same for the whole space. **Switch** is gone, because
  there's nothing to switch between. (#359, #356, #362)
- **What Dev mode is, in the app.** Dev mode and Approvals have their own rows under the space header. The **?**
  next to Dev mode says in plain words what it does, as a popover on the desktop and a sheet on the phone, with a
  link to the guide. (#360)

### Changed
- **A new Dev mode switch.** It's a track with a knob and the word Off, On or **Off?**. The first press to turn
  it off shows **Off?**, and a second press within 5 seconds turns it off. Escape or waiting keeps it on. (#360)
- **The Approvals row shows the approval mode,** and "Off for N bots" when some bots skip approvals. The amber
  Dev mode pill in the sidebar and on the desk is gone. (#360)
- **GitLab, Gitea and other git hosts are read from 0.7.3,** not 0.7.2 as 0.7.1 said. Until then the desk names
  them, and the production floor and guards work for them.
- **Once 0.7.2 runs, don't go back to 0.7.1:** Dev mode stops making new cards there (nothing is lost; updating
  again fixes it). (#359)

### Fixed
- **The Dev desk badge and the "what landed" rollup each keep their own seen time.** Opening the desk clears the
  new reviews badge, and **Mark seen** starts the rollup over. Before, opening the desk also emptied the rollup, so
  it almost always said "Nothing landed since you last looked". (#357)
- **Opening the Dev desk from a link clears the new-reviews badge** without a reload, and a review that arrives
  while the desk is open still counts. (#361)

## [Mac 0.7.1] - 2026-11-06

### Added
- **Post to GitHub.** A Dev mode review card can go onto its pull request as one GitHub review comment, posted as
  you and pinned to the commit that was reviewed. Findings you didn't dismiss become comments on their lines, and
  the rest go in the review's text. Only your press posts: kwant never approves, requests changes or merges, and a
  bot can't post. Secrets are never posted. If your GitHub token can only read, the card says so, and if the pull
  request got new commits, nothing is posted and a new review is on its way. Posting needs **Pull requests: Read and
  write** on the token; reading still works with read access. (#340, #338) Guide: docs/dev-mode.md.
- **A new reviews badge.** The Dev desk row shows how many reviews are ready since you last opened the desk, on the
  phone and the desktop alike. (#344, #348)

### Changed
- **Dev mode turns on with one press and finds the repo itself.** There's no setup dialog. kwant looks at the
  space's teams: each team's repo link, or its folder when that folder is a git checkout. It reads this on your
  machine with no network, and a token in a remote URL is dropped, never logged or stored. A short note says what it
  found, with one fix and **Undo**. If no GitHub connection can read the repo, **Connect GitHub** takes you to
  Connections once, and Dev mode picks the repo up after that with no off and on. **Switch** lists the repos it
  found, with **Type a repo** last. GitLab, Gitea and other hosts are named, and kwant reads them from 0.7.2.
  Turning on never fails: with no repo, the production floor and the command guards still work. (#348, #344)
- **Turning Dev mode off takes two presses.** The first lifts the cover, and the second, within 5 seconds, turns it
  off. Escape, clicking elsewhere or waiting keeps it on. The off note has **Turn back on**. (#343, #344)
- **The Dev desk has a new look.** Landed cards show the commit, the pull request link (or "Direct push"), a
  diffstat for each file and a tests line. Review cards show the branch flow, a "reading the diff" line while the
  review is written, a light for each security check, and an LGTM stamp when there's nothing to fix. The production
  floor card, the desk header and its status light match. (#339, #341, #346, #347)

### Fixed
- **Payments always ask for crypto wallet sends and swaps.** A tool whose arguments carry an amount and an address
  or a crypto asset now counts as a payment, so it asks in every approval mode, even with approvals off or after
  "always allow". This catches wallet tools such as Incentiv's `send_transfer` and `send_swap`, which could run
  without a card before. Tools named with `token`, `coin` or `asset` no longer ask by their name alone. (#342, #349)
  Guide: docs/payments.md.
- **A reply that waits on your approval says "Waiting for your OK"** instead of showing a loading skeleton. (#336)
- **The production floor card reads as a sentence**, such as "git push sends new commits to main.", instead of "This
  is runs deploy.sh…". (#335)

## [Mac 0.7.0] - 2026-11-05

### Added
- **Dev mode.** Point a space at one GitHub repo and branch with the switch in the space header. It has a safety
  cover, so turning it on takes two presses, and the first time it opens a short setup. kwant reads the repo
  **read only** and writes nothing to GitHub. The space gets a **Dev desk** at the top of the sidebar with two tabs:
  - **What landed**: one plain-words card per merge to the watched branch, a **Since you last looked** roll-up,
    **Mark all seen**, and **Draft release notes** from everything since the last tag.
    A commit that reached the branch without a pull request gets a normal card with a link to the commit. (#331)
  - **Reviews**: one card per open pull request, written by a bot that didn't write the code (on a different
    engine when the space has one). It shows a rule-based risk badge with its reasons, whether the PR does what
    its task asked, whether tests were weakened, and findings you can **Fix it** (sent to the bot that wrote the
    PR) or **Dismiss**. A merge gets the **Verified** sticker only when CI is green, another bot reviewed it in
    kwant, and every finding is fixed or dismissed. (#313, #312, #314, #315, #321, #333)
  - **Before you turn it on:** Dev mode reads GitHub through the Loose ends GitHub connection. kwant's built-in
    password store only saves that token while every space and bot is on **Ask before running commands** (new
    installs start on **Ask only when risky**). Switch them to Ask, or connect Bitwarden. While the token is
    saved, approvals can't go back to Off or Ask only when risky. (docs/dev-mode.md, #329)
- **Secret and package checks on reviews.** Each review runs Gitleaks over the PR (pinned and checksum-verified)
  and checks that every new package really exists in its registry. A found secret or a made-up package always
  makes the card High risk, and the secret itself is never shown. (#316)
- **The production floor.** In a Dev mode space, pushes to the watched or a protected branch, force pushes,
  deleting branches or tags, deploy commands, destructive data commands and using a connection you marked
  **production** always ask first. There's no "always allow". A step that can change production data can't be
  allowed until the space has a backup from the last 10 minutes: the card's **Back up first** takes one. (#313,
  #314)
- **The Idea jar.** Drop an idea in from the sidebar's **+**, the phone's **Jar** tab or `/idea` in any composer.
  When a bot is free, it peeks at one idea and writes its take: a verdict, why, the effort, the risks, a short plan
  and its questions. **Do it** turns a take into a draft mission that still waits for your OK. **Later**,
  **Edit**, **Look now** and **Delete** (with Undo) are on every idea. Bots look at up to 3 ideas per space a day
  by default (Settings changes it, 0 turns it off). (#320, #324, #323, #330)
  Guide: docs/idea-jar.md. (#329)

### Changed
- **Bots can't act on text from a pull request or an idea.** Dev mode cards and idea takes run with no tools at
  all, on every engine: no MCP servers, no browser, none of the engine's own tools. A turn that can't run that way
  fails instead. (#319, #322)
- **The board's Live now row fades at the edge on a phone** while there's more to scroll. (#318)

### Fixed
- **The phone's Undo pop-up no longer covers the tab bar** in the Idea jar, and the jar's Settings row always
  shows. (#330) Screen readers hear "Undo" once on that pop-up, not twice. (#332)
- **Ideas sent with `?space_id=` land in that space.** (#329)
- **Small links on the phone Dev desk are easier to tap.** (#331)
- **codex runs at the reasoning effort you chose.** kwant passed the effort under a key codex ignores, so every
  codex turn ran at codex's default. (#325)

## [Mac 0.6.13] - 2026-11-05

### Changed
- **More room to read on a phone.** While you type, the thread bar, pins and hints step aside and the header shrinks
  to one line. The composer is a single row, **Message {name}**, that opens up when you tap it. The header's second
  line is a thread picker (**Direct chat ▾**) that opens a sheet with the bot's threads and **New thread**. The hint
  row under the composer is gone on touch screens. (#307)
- **The phone Chats tab lists everything you talk to.** **Needs you** comes first, in amber, saying what each bot
  wants (**Wants to open a pull request**, **Asked you a question**, **Needs you in the browser**). Then come
  Working, Channels, Team chats and Direct chats, in taller rows. Each bot row has one **⋯** that opens its member
  sheet. (#308)
- **Approval and question cards fit a phone.** The approval card says **Approval needed** in one sentence, with a
  full-width **Allow**. Tool names are written in words, and a bot's steps fold to **Steps · N**. Question cards
  have larger choices and **Your own answer…**. Inbox titles wrap, the engine chip shows only the model, and inline
  code wraps instead of running off the screen. (#309)
- **The mission board on a phone.** **Live now** comes first, then the lanes as tabs, then **This week** folded to
  one line. The **Needs you now** tile is now **Board tasks waiting on you**. (#311)

### Fixed
- **Taking over a bot's browser on a phone.** The page takes the size of the overlay, with a **Pinch to zoom · drag
  to move** hint. The bot gets its own page size back however you close it: hand back, Back, tapping outside, or a
  dropped connection. With two views open, the last resize wins. (#310)

## [Mac 0.6.12] - 2026-11-05

### Fixed
- **A CAPTCHA is asked in the right chat.** The **Human check needed** card now comes up in the chat of the bot whose
  browser call met the check, even when a team's bots share one browser, and in the thread it was working in when a
  bot runs two threads at once (else its channel or direct chat). It never goes to another bot or thread. Trying to
  use a browser that is waiting on you now says why. (#298, #299)
- **The CAPTCHA card is shorter.** Once the bot's turn has ended, an open card shows one line: the bot carries on by
  itself once it's solved, and nothing you do there is passed to it. On a phone the card's top line wraps as a
  sentence and its icons keep their size. (#302, #304)
- **Bot names no longer break inside a word.** In the sidebar and the **Add member** list, a name wraps only between
  words or after a hyphen, on up to two lines, and a word too long for the line ends in "…". "Needs you" on a bot's
  row is now a compact amber hand with a count, and the line under the name says **Needs you in the browser**.
  (#301, #305)
- **A 👍 or 👎 is told to a bot once.** A reaction made in the same second as the bot's reply could be told again on
  its later turns. (#303)

## [Mac 0.6.11] - 2026-11-04

### Added
- **👍 and 👎 on a bot's messages, and Reply.** Every finished bot message has 👍, 👎 and **Reply** next to
  **Copy markdown**. When the bot's newest message asks you something, the thumbs read **Yes, go ahead** and **No**,
  and picking one answers it: the bot carries on without you typing. Anywhere else a reaction is feedback the bot
  reads on its next turn in that chat; after a 👎 it says what it would change. A 👍 or 👎 never approves an approval
  card, a payment, a human step or a decision card, which keep their own buttons. **Reply** puts the message in a
  "Replying to" chip in the composer, and the bot is told which message you mean. Your message shows a one-line
  quote; click it to jump to the original. In a room, a reply goes to the bot that wrote the message unless you
  @mention another. (#296, #292)
- **Every CAPTCHA a bot meets is handed to you.** A **Human check needed** card now comes up wherever a check stops a
  bot, including a page it opened from a link you sent, a bot using its own web tools (it reopens the page in its
  browser so you get the card) and a browser step that failed on the check. Detection catches more walls (Amazon,
  "make sure you're not a robot", new Cloudflare pages, DuckDuckGo, Turnstile) and no longer fires on articles or
  addresses that merely mention a CAPTCHA. While you're solving it, the bot stays off that browser. kwant never
  solves or bypasses a check itself. (#295)

### Fixed
- **An audit no longer shows up in a bot's own chat.** An audit's challenge and rebuttal follow your Approvals setting,
  an unanswered audit card closes after 5 minutes, and answering it late no longer starts an ordinary turn in that
  bot's chat. The challenger is picked from the owner's team and sees the owner's result. (#291)
- **Typing during a browser take-over in the Mac app.** Keys no longer carry a Windows-only key code, and the page is
  told it has focus while you're in control. If typing still does nothing for you on a Mac, please tell us. (#293)
- **Live thinking and the activity list keep secrets masked.** A running step's thinking text and raw tool arguments
  stay out of the activity list and the live stream, credential objects and lists are masked whole, and a secret
  split across chunks, or written with a space between the key and the value, is held back until it can be masked.
  Saved text was already masked. (#286, #288, #294)
- **Hovering a bot in the sidebar no longer wraps its name or makes the row jump.** The edit and remove buttons sit
  over the status line instead of pushing the name. (#290)
- **The selected space's ring and its unread badge are no longer cut off** at the top of the spaces rail. (#289)
- **Taking over a browser:** a background tab is no longer mistaken for a stuck one and reloaded, a frame inside a web
  component is checked like any other, an address like `localhost:5173` opens as a web page, and `tel:` and `sms:`
  links are refused as links to another app. (#287)

## [Mac 0.6.10] - 2026-11-04

### Fixed
- **Taking over a bot's browser works again after a page tries to open another app.** A link to an app, such as
  TikTok's QR sign-in or an app-store link, made Chrome wait on a hidden "Open this app?" prompt, so your clicks and
  keys did nothing, even after a reload. kwant now stops those links before Chrome asks and tells you so in a banner.
  If a tab is already stuck when you open it, kwant steps it to a blank page and straight back, at most 3 times a
  minute, and says the page was reloaded. A bot asked to open such a link is told it opens another app. (#280)
- **No extra click after you switch windows during a take-over.** A mouse button held while the window lost focus
  is now let go, so the next press doesn't give the page an extra click. (#280)
- **The take-over picture keeps up.** A picture Chrome drew while your connection was busy is sent as soon as it's
  free, so a page that then stays still no longer shows an old picture. The address and title follow the page as
  soon as it navigates, the cursor shape matches where the pointer stops, and input sent before you take control
  says "Take control of the browser first." (#275, #277)
- **A one-off follow-up card shows its date in your local time** ("once on Fri, Nov 6 at 03:00 PM"), not the raw
  ISO time. **"AutoFix retries it now." goes away** once the task is verified, waiting on a check or cancelled. (#283)
- **The working line no longer says "getting started" between steps,** and after a reload it still shows what the
  bot is using, with steps that ran before the reload. (#284)

## [Mac 0.6.9] - 2026-11-04

### Added
- **You can use a bot's browser when you take it over.** Click the bot's page, or press **Take over**, and it opens on
  a large stage at its real size, with back, forward, reload, the address and **Hand back**. Clicks, double-clicks,
  right-clicks, hover, scrolling and the full keyboard work, with copy, paste and input methods; on a Mac, Cmd
  shortcuts reach the page as Ctrl. Drop-down lists open as kwant's own menu and page dialogs as a card you answer.
  A sign-in card's **Open browser** goes straight into control. On a phone the page fills the screen. (#270, #274)
- **Bots open a website's sign-in page instead of refusing.** Asked to log in to an account of yours, a bot opens the
  site's sign-in page, carries on if you're already signed in, and otherwise posts a card so you sign in yourself.
  Bots never ask for a password or one-time code in chat. One card per site: a second turn waits on the open card,
  and after Decline the bot says what it couldn't do instead of asking again. (#269, #273)
- **A "needs you" nudge.** When a question or approval waits in a chat you aren't looking at, a pill drops from the
  top of the web app (Esc dismisses it, a click opens the chat), the tab shows the count and a "!" icon, and an
  installed app gets a badge. The Mac menu-bar icon plays a short animation once per new item and shows a small note
  under it; the Linux app changes its tray icon and shows a notification. **Show nudges** in the tray menu turns the
  app's nudges off. (#267, #268)
- **The Linux AppImage updates with AppImage tools and lists in software centers.** The AppImage carries update information for AppImageUpdate, and the release ships `kwant_x86_64.AppImage.zsync` next to it. The AppImage and the .deb carry AppStream metainfo (`si.kwant.kwant`). These were the two warnings from the AppImage catalog's test. The in-app updater is unchanged.

### Changed
- **A refreshed look.** A new app shell, sidebar and Bot home, avatar moods and stickers, and a new conversation
  stream: each reply's header shows the bot's avatar, who picked it and the engine, a live strip shows a bot at work,
  reply actions share one style, and decision cards say "Decided: …" with the reason. (#264, #266)

### Fixed
- **The bot's page stays 1280×800 after a navigate.** An error reply from Chrome no longer drops the page's
  connection, which shrank the page to 1280×713 until kwant restarted. (#272)

## [Mac 0.6.8] - 2026-11-03

### Added
- **Bots learn from their earlier tasks.** Before similar work, each bot sees what went wrong on its tasks in the last
  30 days: a check that failed after hand-in, a turn that stalled or was denied, and every time you sent a task back
  with a reason. Task briefs get up to 8 lines, chats, routines and hand-offs up to 5. The lines come from kwant's own
  records, so a bot can't write its own, and a check's command shows only when it has no link, path or access word.
  1 task in 5 goes without, so the effect can be compared. On by default; turn it off in Settings with
  **Learn from earlier work**. A bot's **Memory** tab lists its lessons, with ×N for repeats and **View task**.
- **Goals that stopped holding are lessons too.** When a "Keep this true" goal made from a bot's finished task is
  confirmed not met, that bot's briefs and chats say so until the goal holds again.
- **Bots check their work before hand-in.** A task with an approved check asks its bot to run the checks itself and
  fix what fails before moving it to review. The task's activity shows "It ran its checks before hand-in".
- **Bots keep your corrections.** When you correct how a bot did something in a way that would apply again ("use a
  table", "always cc Sam"), it saves that as a memory without asking and applies it later.

### Fixed
- **"Compare Lidl and Edeka" in #general no longer asks who should answer.** A close call with no matching words goes
  to the Concierge, and a message nobody's role matched no longer shows "matched" words.
- **A task's cost and model count only its own calls.** A chat with the same bot while it worked on a task is no
  longer billed to the task.

## [Mac 0.6.7] - 2026-11-03

### Changed
- **Comparing two options gets a table.** Bots are asked to answer "Compare A with B" as a table, a column each and a
  row per point, with every header cell filled, so kwant draws it as a comparison card.
- **A link in a table cell counts as its text.** The 200-character cell limit counts what you see, so a table with
  long source links still becomes a card. A cell holds at most 1,000 characters in all.

### Fixed
- **Smart Results draws fewer wrong cards.** Phone numbers and version numbers aren't number tiles, and a recap of
  weekdays isn't steps. `4,100` reads as four thousand one hundred when the whole column groups that way. Two series on
  very different scales aren't drawn on one axis, values out of range stay text, and a card link with a user name or
  password is refused.
- **A table or list a bot points at must match the reply exactly:** every row and step, in order, with the same
  numbers. Anything else falls back to the reply's own table.
- **Long table cells end in an ellipsis**, with the full text on hover or focus, and a chart's caption isn't repeated
  when the reply already says it.
- **A file preview counts only real lines,** so trailing blank lines no longer mark a short file as truncated.
- **Goals: clearer checks and fewer false alarms.** A page over 5 MB is "can't check", not "not met". A redirect is
  still not followed, and the reason names where it points. A changed file shows both the expected and the actual
  hash. One dropped connection no longer flips a met goal that passed last time; two in a row do, and an error right
  after a failure shows "can't check" at once. A notice you turned off isn't counted against the daily cap. Deleting a goal closes its open "Restore" mission and cancels its tasks, keeping the history.
  The goal tool's schema now matches what it accepts (`json_value` needs a `pointer`; `contains` is listed).

## [Mac 0.6.6] - 2026-11-03

### Changed
- **Accounts that can't take a turn sit apart in Order to use.** Engines that aren't installed or aren't signed in
  are listed under "Not in the order", with no number and no arrows. When you move an account, they're placed after
  the ones you ordered, so an engine you install or sign in later joins at the bottom instead of jumping to the top.
- **"Out until" names its time zone.** When every account is out, the message reads "Every model is out until
  09:00 CET; I'll answer then.", in the time zone saved in Settings (else the computer's), with its abbreviation. A
  zone kwant can't read falls back to UTC.
- **GitHub releases are named "kwant 0.6.6 for Mac and Linux"**, with separate Mac and Linux sections in the notes.

### Fixed
- **One notification per failing task.** With AutoFix on, a task that fails and is retried notifies once, for its
  final outcome, instead of once per attempt.
- **A wide file preview stays inside its message.** A shared file with a wide table scrolls inside the chat bubble
  instead of stretching it past the screen.
- **Goals refuse a latency check that can never pass.** "Latency under 0 ms" is refused with a plain message when you
  create or edit a goal, or when a bot proposes one.

## [Mac 0.6.5] - 2026-11-03

### Added
- **Smart Results.** When a bot's reply holds a table, a numbered list or a set of figures, kwant draws it as a card
  under the text: a comparison, a chart, metric tiles, facts, cards, findings, steps, a checklist or a timeline, a
  task's checks, or a preview of a shared file. Each card has Evidence (where the data came from), Show as text and
  Copy data, and "Ask about this" puts a row into your message. The reply text is never changed, and anything kwant
  can't draw cleanly stays as the original text.
- **Goals.** Above the Mission board, say what should stay true (a page answers, a command succeeds, a file exists)
  and kwant keeps checking it. One quiet line shows while everything is OK. When a goal fails, kwant opens one
  "Restore" mission, investigates on its own and offers a fix: Fix it, Not now, or Always fix this goal. Nothing runs
  until you create or approve a goal.
- **Automatic model failover.** A bot that hits a usage or session limit moves to the next account that still works,
  across Claude, Codex, Grok and Custom API, in the order you set in Settings › This space › Accounts › Order to use.
  The chat shows one quiet line per outage, and the bot goes back to its own account after the reset. When every
  account is out, your message waits and is answered at the earliest reset. A bot with failover off stays on its own
  account.

### Changed
- **Everything is on by default.** AutoFix, the Swarm Auditor, Flight School, Skill Foundry, Automation Discovery and
  the Truth Ledger now start on (the Ledger in full, not shadow), on a fresh install and on an existing one where you
  never changed them. Nothing you saved changes: if you switched one off, or picked "suggest" or "shadow", it stays
  that way. The caps are the same as before (AutoFix 25% recovery budget, 5 audits a day, the Foundry and Discovery
  daily caps), and token and task limits stay off.
- **Flight School starts every bot at Act.** New bots no longer start in Sandbox, so turning Flight School on adds no
  approval cards: it keeps scorecards and proposes tiers, and only you lower or raise a bot. You can still pick a
  lower start for new bots in Settings.

### Fixed
- **A limit message is never shown as the bot's reply.** "You've hit your session limit · resets 3am (Europe/Berlin)"
  and other clock-style resets are now read as a limit, with the reset time in its own time zone.
- **No task tool without the task board.** With the Truth Ledger on and the board switched off, bots were still
  offered `task_cite_fact`; it is now offered (and accepted) only while the board is on.

## [Mac 0.6.4] - 2026-11-02

### Added
- **Laya picks the right bot more often.** A message goes to a bot only when its words fit that bot's role, name,
  title or description, and in #general a question no specialist fits goes to the Concierge.
  On a close call between two bots you get a card to pick one, or "Answer generally". Each routed reply says in
  plain words why that bot answered, and a bot can hand back a message that isn't its area.
- **"Wrong bot?"** under a routed reply sends your message to the bot you choose. Laya remembers the correction, so a
  similar message later goes there too.
- **Good for / Not for.** Tell Laya what each bot should and shouldn't get, on the create form and in the bot's
  settings. The lists are kept when a bot is copied, exported or imported.
- **Browser sites.** In a bot's settings, choose which sites its browser may open, which it never opens, and whether
  it may reach local and private network addresses. A bot can't change its own rules.

### Changed
- When a team lead hands work to several teammates, it waits for every result and answers once, even when it hands
  off in separate steps or you press Stop.
- Escape keeps what you typed: an editor in Settings with unsaved changes stays open, and the bot editor and
  Manage team close on Escape. Desktop Settings sits beside the space rail, and removing a team member asks first.

### Fixed
- Saving two things at once no longer fails now and then with "database is locked".
- A link such as tiktok.com/@name is no longer taken for an @mention of a teammate.
- The updater uses a newer zip library, which clears three security alerts. They affected only Windows builds,
  which kwant doesn't ship yet.

## [Mac 0.6.3] - 2026-11-01

### Changed
- **One Inbox.** "While you were away" is now the **Inbox**, with the unread badge, and Loose ends is its second tab.
  The top of the sidebar is Search, Inbox, Mission board and **More** (Parallel Universe). Install app is a tip you
  can dismiss. Old links and bookmarks open the new places.
- **One bot list.** Bots are listed once, grouped by team with the lead first and bots without a team at the end.
  Long names wrap to two lines instead of being cut off, and Edit and Remove appear when you point at a row.
- **One space switcher.** The rail on the left switches spaces. The header shows the space's name, a short line
  ("4 bots · no account yet") and a gear for its settings. The phone has one switcher.
- **One Settings.** Settings has two tabs. **This space** holds its accounts, API keys, the Ollama address, Routines
  and Connections. **All spaces** holds usage, the Mission board, notifications, Flight School, the computer,
  signed-in devices and Install app. The gear in the header opens This space and the gear on the rail opens All spaces.
- Grok has an Install link when it isn't on this computer. When an account fails, the reply now points to
  **Settings › This space › Accounts**.

## [Mac 0.6.2] - 2026-11-01

### Changed
- **The Mission board is on from the start.** New installs, and installs that never changed the
  setting, now show the board, give bots the mission tools and put hand-offs, routine runs and Loose
  Ends on it. If you switched it off, it stays off (**Settings → Mission board**). Bots propose a
  mission only when you ask for a plan or tracked work, never for an ordinary request. A finished
  hand-off goes to Review and the bot that handed it out checks it in the turn it already takes when
  the result comes back (one reminder at most). Flight School, the Truth Ledger, AutoFix, automation
  suggestions and the auditor keep their own switches and stay off until you turn them on.
- **Pick one way to start.** Adding an account now opens with three choices: Sign in with Claude,
  Sign in with ChatGPT (through Codex) or Paste an API key (Anthropic, OpenAI or xAI). Engines that
  aren't installed are folded under "Not available on this computer" with an Install link, and their
  switches appear only once something works.
- **A welcome in the first chat.** An empty chat shows your bots, says that Laya picks who answers and
  has one Add an account button. Suggested prompts wait until an account works.
- **Replies start with one line.** A bot's reply shows its name and the time, with a Details arrow
  for the model, account, tokens and steps ("Details · a step failed" when one did). The note about
  when an engine shares its thinking is gone.
- **A cleaner bot list.** The Laya and engine tags are gone from every row in the sidebar and the
  phone's Bots tab, so names get the room; the engine is in the name's tooltip.
- **New installs start in "My space"** (violet, with a house). Existing spaces keep their name, icon and
  colour. The setup banner and the bar above the app no longer look like warnings.
- Line icons instead of emoji for Bot browser and Pin. "Out of quota until 8:15 PM" in your local time
  instead of a raw timestamp. A tool's command line sits behind Details in Connections. Monospace only
  for code.

## [Mac 0.6.1] - 2026-11-01

### Added
- **Linux app (beta).** The desktop app now also comes for Linux on x86_64, as a `.deb`
  (`kwant_amd64.deb`, Ubuntu 22.04+ or Debian 12+) and an AppImage (`kwant_x86_64.AppImage`), in the
  same GitHub release as the Mac app. It updates itself in the app like the Mac app does: it checks the
  same signed `latest.json`, which now also lists `linux-x86_64-appimage` and `linux-x86_64-deb`, and
  refuses an update whose signature doesn't match. The `.deb` asks for your password to install an
  update. Beta: tested in containers on Ubuntu 22.04, Ubuntu 24.04 and Debian 12, not yet on real
  desktops, so the tray icon, Wayland and HiDPI may vary. No ARM Linux and no Windows yet.

## [Mac 0.6.0] - 2026-11-01

### Added
- **Flight School: bots earn more freedom, and only you grant it (Mission Control Phase C4).** With
  `flight_school` on (off by default), each bot gets a scorecard per kind of work: verified tasks,
  first-try rate and what it still needs for the next tier. A bot that qualifies can ask for a higher
  tier; the request shows on the board under "Needs you" and in the bot's settings with its scorecard,
  and you Grant or Decline it (with an optional reason). Only you can decide; a bot gets a 403. If two
  tabs answer at once, one wins and the other says someone got there first. Taking a tier back is still
  one tap and applies at once. Payments and high-risk approvals always ask you, whatever the tier.
- **Truth Ledger on (Mission Control Phase C2).** With `truth_ledger` set to `on`, bots read their memory
  from the shared list of facts. When two facts disagree (for example "4 GB" vs "12 GB" of RAM) they are
  flagged as disputed until you settle them: the board shows both side by side with why they clash,
  and you pick "Keep this one" or "Not a conflict". Facts are labelled Unconfirmed or Disputed, show
  their evidence and the tasks that rely on them, and a task resting on a changed or disputed fact says
  so. Only you can confirm, reject or settle facts. With the ledger `off` or `shadow`, what bots see is
  byte-for-byte what 0.5.1 showed them, checked against a saved copy.
- **Insights: verified results per week and their cost.** Under the week strip, the board shows verified
  results per active week over the last 8 weeks, and the cost of each verified result. Failed runs count
  in the cost. When some runs reported no cost it says so ("Based on N verified results that reported a
  cost; M runs reported none"), and with no reported cost at all it says "Not reported", never $0.
- **kwant.si blog, ready for posts.** `scripts/build_site.py` now builds a blog from Markdown files in
  `site-src/blog/` (static, no new dependencies, CSP unchanged): `/blog/` lists the posts, each post has
  breadcrumbs, reading time, related posts and a try-kwant box, plus BlogPosting and breadcrumb JSON-LD,
  article meta, a sitemap entry and an item in `/blog/feed.xml` (RSS). A `compare` block makes comparison
  tables that turn into one card per row on phones. Posts marked `draft: yes` are left out everywhere;
  `--drafts` previews them locally. The Blog link in the header and footer appears with the first published
  post, so the live site is unchanged until then. `check_site.py` checks blog pages and the feed.
  Post links may only point to http(s), mailto, this site or an in-page anchor; anything else
  (`javascript:`, `data:`, `//host`, …) stays plain text. Links whose address has parentheses work.

### Changed
- **kwant.si covers 0.5.** The home page says "New in 0.5" (bots bring receipts, status dots, smarter
  browsing, suggested routines, off by default). The features page has "New in 0.5"
  sections for web citations, browser steps, numbered controls, asking before paying, status dots,
  suggested routines, routine checks, the Truth Ledger preview (off by default) and the star card. The
  Browser, Routines and Bots & teams docs cover them, and /llms.txt mentions citations.
- **kwant.si home page, focused.** One main button in the hero, Download for Mac; Linux, VPS and Docker
  get a text link to the quickstart. Under the install box, a release line ("0.5.1 released Oct 31 · 10
  releases since Oct 25 · AGPL-3.0") that `build_site.py` reads from the `## [Mac X.Y.Z] - date` headings in
  this file; `check_site.py` fails if the built page is behind them, so a release must rebuild the site.
  The home page is about 30% shorter: Parallel Universe, "Bots that grow with you", the phone section and
  "Works with what you already have" moved to the features page (`#parallel`, `#more`, `#phone`,
  `#works-with`), and a "More in kwant" strip on the home page links to each. Old home links to
  `#parallel`, `#more` and `#phone` land on that strip.

## [Mac 0.5.1] - 2026-10-31

### Added
- **Star on GitHub, asked nicely.** After real use (5 finished turns and 2 days since this device
  first opened the app) a small card says "kwant bots run on coffee and GitHub stars ⭐" with
  Star on GitHub, Maybe later (asks again in 2 weeks) and Don't ask again. It never shows on first
  run or while a bot needs you, a question or an approval waits. After a star or Don't ask again it
  never comes back. App settings has a quiet "Star kwant bots on GitHub" link. It is all kept in
  this device's storage; nothing is sent anywhere.
- **kwant.si:** every page's footer says "kwant runs on coffee and GitHub stars. Toss one our way ⭐",
  and the home page's closing panel has a Star on GitHub card. Plain links, no GitHub widget, so the
  Content-Security-Policy is unchanged.

## [Mac 0.5.0] - 2026-10-31

### Added
- **Bot status dots.** The dot on each bot's avatar in the sidebar now says what the bot is doing:
  green Available, a pulsing Working dot, amber Needs you, a hollow Queued ring, and grey Can't run.
  A bot with a running turn never shows green, in the open chat included, and hovering or a screen
  reader gives the status in words. The line under the name says "Working here" for the open chat.
- **Truth Ledger, shadow mode (Mission Control Phase C1).** A new `truth_ledger` setting (off by
  default; `shadow`, or `on`, which behaves like shadow for now) lets you see every bot note as a fact
  with where it came from. Notes a bot wrote after reading a web page, a file or another tool, and
  imported notes, show as proposed until you accept or reject them; your own and the bot's other notes
  are accepted. Nothing the bots see changes yet. Only you can read or decide facts; bots get a 403.
- **Routines can say what a good run is.** A routine can carry a Flight School class, a description
  of a good result and checks. Each run's board task gets them, with the checks already approved, so a
  run that passes its checks is verified on its own. Only the person sets the class and the checks; a
  bot that tries gets a 403. Routines without them run as before. This is the first step of
  Automation Discovery (Mission Control Phase D4).
- **Automation Discovery can preview routines from your habits.** With Automation Discovery on in
  Settings (off by default), a scan of a space looks at the board tasks you made over the last 120 days
  and finds ones you ask for daily, on weekdays, weekly on up to three days or monthly, in your time
  zone across daylight-saving changes. It says which routine it would suggest (bot, schedule, first
  run, checks, follow-up steps) and why every other repeated task gets none. Only habits that were
  mostly verified, mostly done by one bot and not already covered by a routine, on or off, qualify.
  Tasks made by bots, routines, hand-offs or chats never count. The scan is a preview: it creates no
  offer and no routine yet. This is step 2 of Automation Discovery.
- **Automation Discovery suggests routines you can accept or dismiss.** With it on, each space is
  scanned once an hour (or when you ask) and new suggestions are kept, at most 2 a day (you can change
  that) and 5 waiting per space, each with one notification. A suggestion shows its schedule, first
  run, checks, the tasks it came from and what the bot may do when it runs. Accept makes the routine,
  with your changes to the name, schedule, bot, delivery, acceptance or checks; Dismiss stops it being
  suggested for 30 days. Nothing is made without Accept, and only you can see, accept or dismiss
  suggestions. Suggestions whose habit stopped are withdrawn; one whose habit changed (another bot,
  days or time) is updated in place, without a second notification and without using up the day's
  suggestions. Undoing the delete of a
  suggested routine keeps it marked as suggested. Exports now carry a routine's class and acceptance,
  and its checks when they hold no secrets. This is step 3 of Automation Discovery; the screens come
  next.
- **Suggested routines in Routines, and a switch in Settings.** Settings > Mission board has "Suggest
  routines from repeated tasks" (off by default) and how many new suggestions a day (0–10). Routines
  lists each suggestion with its bot, rhythm, first run, the tasks it learned from (with their days),
  what the bot may do when it runs (its Flight School tier, when it asks you, what past runs changed),
  and which checks it keeps or leaves out and why. Accept opens the routine form to edit it first,
  then says when it first runs; checks can only be unticked, and masked check details are never
  editable. Dismiss says the suggestion is hidden for 30 days. If it was already handled elsewhere, the
  list refreshes and says so. Preview and Scan now show what was found and, in plain words, why other
  repeated tasks weren't suggested. When suggestions are paused (the switch or the board is off, or no
  time zone is set), the section says why and how to fix it. Routines added this way carry a
  "Suggested for you" badge, a small line shows the last 30 days, and "A routine was suggested" joins
  the notification toggles. This is step 4 of Automation Discovery.

### Changed
- **Bots cite what they read on the web, with links that open the page at the quote.** A bot passes
  the exact words (or a block id from its read) and kwant checks they are on the page once, word for
  word, and returns the quote, its address and a link that opens the page with it highlighted. A
  quote that isn't on the page, or is on it more than once, is refused with the reason, so a bot
  can't make one up. A link never opens at a lookalike higher up the page (the same words with
  other capitals or accents, like "résumé" for "resume"): kwant adds nearby words to the link, or
  refuses when nothing tells the two apart. Bots are told to back web facts with these links. No
  model calls.
- **Bots don't re-read a page that hasn't changed, and find controls on crowded pages.** When a bot
  reads its browser page again and nothing changed since its last look, it gets a one-line
  "unchanged" instead of the whole page. On a page with more than 40 controls, the bot can say what
  it looks for ("add to cart bike light") and the matching controls are listed first, by plain word
  matching with no extra model call; their numbers stay the same.
- **Bots see what each browser step changed, and stop going round in circles.** After every click,
  key or typing, the bot gets one line saying what changed: the page it went to, or the new and gone
  lines. If its last 3 steps changed nothing, or it asks a 3rd time in a row for the same click that
  changed nothing, it is told to stop and try something else. The page text bots read now keeps its line breaks.
- **Bots click page controls by number, and never through a banner.** The page a bot reads lists
  its buttons, links and fields as numbered refs with their role (`[9] button "Add to cart" · Sauce
  Labs Bike Light`), naming the product or row when several share a name. A bot clicks or types by
  ref, so the click hits exactly that control (scrolled into view, also inside frames). If it has
  gone, been hidden or renamed, or a cookie banner or dialog covers it, nothing is clicked and the bot
  gets a fresh list with the reason. Clicking by label still works.
- **Browser checkouts ask before paying, also behind a plain "Continue".** Before a bot clicks or
  presses Enter or Space in its browser, kwant reads the page: the button the click would really hit, card
  number or CVC fields, a payment provider's card frame (Stripe, Braintree, Adyen, …) and forms that
  post to a checkout. Any of these gets the payment card in every approval mode, as before for
  "Pay" buttons and checkout addresses. Bots using the browser through the shell can't show a card,
  so they leave that step to you. Clicks by screen position are now for your live view only.
- **kwant.si footer says "Made in Germany with ❤️".** The line sits under the trademark note on every
  page; screen readers read it as "Made in Germany with love".

### Fixed
- **A Review action survives closing the tab.** Mark verified, Send back or Dismiss on the Mission board
  waits 6 seconds for Undo. Closing, reloading or backgrounding the page in that window used to drop the
  action; now it is sent then, with `keepalive`, and still only once.
- **Only routine runs fold on the Review column.** Worked-ahead and loose-end cards with the same title
  can be different work, so they stay separate cards and "Dismiss N older" never covers them.
- **A hand-off card the lead reviewed no longer loses its fixed result.** When the lead's review sent
  work back and the lead's earlier results turn ran late, that turn put the card back in Review while the
  teammate was still fixing it, and the fix's result was never attached. The lead's turn now also makes a
  hand-off's card if the board hasn't seen it yet, so the lead reviews it right away instead of minutes
  later. Both races made `handoff_review_test` fail now and then on CI.
- **Truth Ledger: outside facts can't be laundered or erased.** A bot editing a note that ever came
  from outside text, or re-adding its exact text after deleting it, lands it as proposed again, even
  after you accepted an earlier version. Only you can take a fact out of "outside", and your decision
  can't be overruled afterwards. Facts and their history can no longer be deleted except by deleting
  their bot or undoing their import.
- **A CAPTCHA nobody solved is always reported as timed out.** When the time limit ran out, the bot
  could sometimes be told to keep waiting for a step that was already over, if the page watcher was
  slow on a busy machine. The bot's own wait now closes the step as timed out once the limit has
  passed, so the bot moves on straight away.

## [Mac 0.4.1] - 2026-10-30

### Changed
- **kwant.si covers 0.4.0.** The home page says "New in 0.4.0" (screenshots for bots, leads reviewing
  hand-offs, @mentions in results handing work on, no default token limits). The features page has
  sections for these and for Performance, the Swarm Auditor with open risks and the Skill Foundry. The
  Mission board, Bots & teams, Browser and Skills docs cover them, with the hand-off caps and Settings →
  Usage → Hand-offs per request. The site also gains /llms.txt and the Grok Bot comparison page.

### Fixed
- **Suggested routine names no longer keep the date.** A suggestion made from "Pay supplier invoices
  for October" is now called "Pay supplier invoices", and "Weekly status report 26 Oct" becomes
  "Weekly status report". Words every source task shares, like "Q4", stay. Skipped habits in a scan
  are named the same way. Suggestions already waiting keep their name until their habit changes.
- **One card per hand-off line.** A lead's message that hands the same teammate several lines (five
  `@Frontend, please …` lines) now makes one task and board card per line instead of one card for the
  whole message. The per-teammate cap (4) still applies; the rest wait for Continue and are sent once.
- **Team hand-offs share one folder for checks and audits.** A hand-off card from a team's channel
  now checks files, keeps checkpoints and runs its audit in the team's folder, where its owner wrote
  them. Before, it looked in the owner's own folder, and the Swarm Auditor's challenger in its own,
  so it could miss a file the owner had written (the challenger found no `index.html`).

## [Mac 0.4.0] - 2026-10-29

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

[Mac 0.5.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.5.0
[Mac 0.4.1]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.4.1
[Mac 0.4.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.4.0
[Mac 0.3.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.3.0
[Mac 0.2.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.2.0
[Mac 0.1.3]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.3
[Mac 0.1.2]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.2
[Mac 0.1.1]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.1
[Mac 0.1.0]: https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.0
