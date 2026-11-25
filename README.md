<p align="center">
  <img src="docs/media/readme-hero.png" alt="kwant: a pixel-art night town where a team of bots hand work to each other" width="1280">
</p>

<h3 align="center">Your own team of AI bots that check each other’s work before saying “done”.</h3>

<p align="center">
  <a href="https://kwant.si">Website</a> ·
  <a href="https://kwant.si/docs/quickstart">Docs</a> ·
  <a href="https://kwant.si/features">Features</a> ·
  <a href="https://kwant.si/alternatives/grok-bot">Compare</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/alexsergivan/kwant/discussions">Discussions</a>
</p>

<p align="center">
  <a href="https://github.com/alexsergivan/kwant/releases/latest/download/kwant_universal.dmg"><img src="https://img.shields.io/badge/Download-macOS-5c3fd0?style=for-the-badge&logo=apple&logoColor=white&labelColor=160c3a" alt="Download kwant for macOS"></a>
  <a href="https://github.com/alexsergivan/kwant/releases/latest/download/kwant_amd64.deb"><img src="https://img.shields.io/badge/Download-Linux_.deb-5c3fd0?style=for-the-badge&logo=linux&logoColor=white&labelColor=160c3a" alt="Download kwant for Linux (.deb)"></a>
  <a href="https://github.com/alexsergivan/kwant/releases/latest/download/kwant_x86_64.AppImage"><img src="https://img.shields.io/badge/Download-Linux_AppImage-5c3fd0?style=for-the-badge&logo=linux&logoColor=white&labelColor=160c3a" alt="Download kwant for Linux (AppImage)"></a>
  <br>
  <sub>macOS: one app for Apple silicon and Intel · Linux x86_64 (beta): .deb or AppImage · A server: <a href="#self-hosting-on-a-vps">binary or Docker</a> · Phone: <a href="#on-your-phone-iphone-and-android">add it to your Home Screen</a> · <a href="https://github.com/alexsergivan/kwant/releases/latest">All downloads</a></sub>
</p>

<p align="center">
  <a href="https://github.com/alexsergivan/kwant/stargazers"><img src="https://img.shields.io/github/stars/alexsergivan/kwant?style=social&label=Star" alt="Star kwant on GitHub"></a>
  <a href="https://github.com/alexsergivan/kwant/actions/workflows/ci.yml"><img src="https://github.com/alexsergivan/kwant/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/alexsergivan/kwant/releases/latest"><img src="https://img.shields.io/github/v/release/alexsergivan/kwant?label=release&color=5c3fd0" alt="Latest release"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License: AGPL-3.0"></a>
  <br>
  <sub>Free and open source. A ⭐ helps others find kwant.</sub>
</p>

<p align="center"><b>kwant has no token.</b> Official X account is <a href="https://x.com/kwantbot">@kwantbot</a>.</p>

kwant gives you a team of bots that share chats, hand work to each other with an @mention and get it done on their own, on a schedule or while you're away. Each message goes to the bot that fits, and the reply says why. Work shows up as cards on a Mission board, and a task turns **Verified** only when kwant runs the check itself, never on a bot's word. Keep separate spaces for work and home, each with its own bots and accounts. Run it as an app on your Mac, Linux or Windows (beta) PC, or on a server, with the Claude, Codex or Grok CLI you already use, or your own API keys.

It's written in Rust (Tokio, Axum, SQLite) with a React web app embedded in the binary.

![A kwant channel: Designer answers a question about empty states, then @mentions QA, and QA checks it on a 360 pixel screen.](docs/media/handoff.webp)

---

## New in 0.9

- **Review a bot's work in the Inbox** (0.9.0). Inbox › New has a **To review** group, highest risk first. Each item shows a verdict (Pass, Fail, Needs you or Not checked yet), what done looks like, the evidence, and a **Couldn't check** list with the reason for each. Press **Approve**, **Send back** with a note, or **Roll back**. Low-risk items that passed every check can be approved together with **Approve all**.
- **A second check before you approve** (0.9.0). When a Mission board task moves to review, a teammate on another model runs its checks again in a clean copy of the files and never writes to the task's folder. For high-risk work it also changes the code in small ways to see if the tests notice. It says Pass only when kwant ran a check again itself. A task can say what done means and what must not change, and rolling back work you approved moves the bot down one Flight School tier.
- **Point at the part of the page you mean** (0.9.0). In Watch live, press **Point** and click part of the page. The bot gets the page address, a selector, the element's HTML, its size and styles, and a picture of it. Point only reads the page: it never clicks or types.
- **Notes on diff lines, and CI back to the bot** (0.9.0). On a Dev desk review card, add notes on lines and send them to the bot as one message; after its next push, each note says whether those lines changed. Failed CI checks and review comments on a bot's PR go back to that bot as one Fix-it item, and when the PR goes green a fresh-eyes reviewer gives its advice. Nothing is posted to GitHub, and nothing is merged.
- **A worktree for each bot task** (0.9.0). The Dev desk's **Worktrees** tab makes a worktree from a new or existing branch, for a bot or just for you, and the bot works in its folder. **Remove** never deletes the branch and asks before it loses changes. An hour after a task finishes, kwant deletes build folders git ignores, such as `node_modules` and `target`, unless you keep them.
- **Watch a mission** (0.9.0). Pick a bot to check a mission's claims when it stops. A claim that doesn't hold sends its task back, and the report shows what was claimed and what was checked.
- **One order for what needs you** (0.9.0). The sidebar, the Inbox and the phone's Chats tab sort the same way: needs you, stuck or failed, to review, finished, working, waiting. A message to a busy or stuck bot is kept and says why it waits, and you get one notification when a turn shows no step for 15 minutes. Also: bots leave notes about their tools in Settings › Connections, and a bot can have a monthly budget (set through the API for now).
- **Fixed in 0.9.1.** Bots hand work to each other as often as the work needs, with no stop at 4 hand-offs to one teammate or 16 per message. Exact repeats and a teammate that failed 3 times in a row are skipped with the reason. You press **Continue** only after 64 hand-offs for one message. Hand-off cards catch up after a lost connection, and TikTok's QR sign-in no longer freezes the bot's browser.
- **New in 0.9.2.** Teams keep going until the job is done, so you no longer type "continue" when a lead stops halfway. After a teammate reports back, a lead that stops without handing on, asking you or writing a `Done:` line gets up to 2 more turns, then you get one **Keep going?** card. "Continue with Google" and other sign-in popups now open in the bot's live browser view, and the view goes back to the page when the popup closes. Bitwarden accounts with two-step login connect, and a Telegram approval card sent as you switch off Allow approvals from Telegram no longer keeps a dead Allow once button.
- **New in 0.9.3.** Each bot job gets its own page in its team's browser, so a sign-in or CAPTCHA that one bot waits on no longer stops the team's other bots. The pages share the team's sign-ins. When a team has 2 or more pages, the browser view lists them by bot and site, marks the one that needs you and goes to it. A team has at most 4 job pages at once, and a page nobody has used for 30 minutes closes.
- **New in 0.9.4.** Bots don't stop after a failed step. When a step fails or is refused, a bot tries it again, then tries another way; a step you declined isn't tried again. If nothing works, its last line starts with `Blocked:` and says what would unblock it, and from a team's lead that line comes to you as a question card. A reply that still ends on "I'll…" after a failed step gets one more turn.
- **New in 0.9.5.** Apps & accounts puts apps, AI accounts, MCP servers and Telegram on one settings page, and connecting an app takes 3 steps: open Composio, paste your link, pick the bots. Each bot can have a monthly budget in tokens or dollars: at 80% you get a heads-up, and at 100% it pauses with **Raise the limit** and **Continue this month**. When a bot's connection fails, its chat gets one line that says which one and why, with **Reconnect** when only you can fix it. The team browser frees idle pages and retries for up to 45 seconds when it's busy, and Dev mode no longer stops at 30 model calls a day.
- **New in 0.9.6.** Describe what your bots should do, press **Suggest a team**, and get 2 to 6 bots to untick or rename before you create them. The Inbox has a **Done while you were away** group next to **Needs you**: each item shows the answer, what the bot did and why, with **Looks good**, or ask the bot to redo it. Items stay for 7 days. A reply in your chat that stops before it's done either continues by itself or leaves one line in the chat and one item in **Needs you**, with **Retry** or **Continue**. Settings has one **kwant bots** card that says whether you're on the latest version, with **Update** when you're not. kwant's Chrome now presents itself as normal Chrome, and when Google refuses a sign-in without a window, **Sign in in Chrome** opens one for it on a computer with a screen.
- **New in 0.9.7.** While a reply is coming, the chat shows the bot's own avatar typing at a pixel laptop, with **Ben is working on it**, and the words take its place without a jump. Each bot's avatar has a small mark for the AI it runs on, live while it works, and a greyed mark with the reason when its engine can't run. One **Find a tool** box searches built-in tools, Composio apps and the public MCP directory, forgives typos, and installs a directory tool in one step.
- **New in 0.9.8.** A fresh install starts with **@helpers** (Assistant, Researcher, Writer), and #general asks **What do you want done?**; Dev mode and Approvals move to Settings › This space › General. A done item in the Inbox says what **checked** it (Checked, Partly checked, Failed or Not checked), with the checks and their evidence. Pressing **Ask Ben to try again** on a pre-push card lets Ben's next clean push of that branch go with no second card; the card follows it to **Pushed**. A mission check can **wait** for an outside service (exit 75): it re-runs every 15 minutes without bot turns and tells you after 72 hours. A chat that only needs your answer says **Waiting for you** in one amber line, with **Answer**; press a choice or just type. A routine's message in a chat shows as a **Routine** row with its name, not as your own message. In a team discussion, an **@mentioned** teammate speaks next.
- **New in 0.9.9.** The bot's **browser panel** opens by itself while it browses, lists **Pages this reply** by title, and a chip on the reply brings it back. **AI use** in Settings › All spaces › Usage shows 7 or 30 days, with a bar per day and each model's share. **Routines** open from the sidebar, with a **Weekdays** chip and schedules in plain words.
- **New in 0.9.10.** Each Claude or Codex sign-in on **Apps & accounts** shows how much of its **5-hour and weekly limit** is left, with **Check now**. A job's last reply shows **Done** (or what checked it), and a reply that asks for your OK has **Approve** under it. The **Another app** search lists Composio apps and tool servers in one list, one row per app.
- **New in 0.9.11.** **Fix it** and line notes on a PR card go to the bot you pick, like **Goes to Ana · lead of Edge**. **Settings › This space › Documents** adds and removes the files bots quote, and a quote's chip opens the passage on its page. Done cards check each part of your request, in your words, and say **Partly checked** when a part has no check.
- **New in 0.9.12.** **Dev mode** is first in Space settings › General and says when it finds your team's repo, with a strip in #general that opens it.
- **New in 0.9.13.** Bots open **hover-only menus**, save **downloads** in their own folder and **upload** a file to a page's file picker (under Ask, each upload asks you first). A **mic** between @ and Send turns what you say into text in the message box (with an OpenAI key), and nothing is sent until you press Send. Done cards list each part of your request, in your words, as **Passed**, **Not met** or **No check**. Empty team chats open on a **team home** with the members, the lead and three things to ask. **Double-checks** run on a routine's first run, after a change and then weekly, and show who is checking which task and why.
- **New in 0.9.14.** **⋯ › Continue in a new chat from here** starts a new chat with the same bot and the conversation up to that message.
- **New in 0.9.15.** **The perch**, a small island at the top of your screen, shows what needs you and what your bots are doing, and lets you allow a safe step once or ask a bot without opening the app.
- **New in 0.9.16.** Your **answer to a bot's question** shows once, on its card, and phone chat headers name their buttons.
- **New in 0.9.17.** A card that puts kwant bots on your phone, a team chip that names the team once, steps that fold under a reply instead of pushing it down, "Pick a repo" that always lands in Repo URL, Allow once for notes that mention a release or a merge, Blocked when a reply asks you to pick after a closing line, Claude bots that stop repeating a command or file read that keeps failing, The island names a file, never its folders, dates and times that read one way everywhere, a bot that asks for a missing file in the chat, Keep going never passes a customer's words on as yours and Writes named like reads keep the full card.
- **New in 0.9.18.** A team named once, never "… team team", audio files up to 100 MB in the chat, a bot browser preview that never flashes white while a page loads, browser step cards that stop asking for you once they are closed, tidier reply buttons on a phone, a production deploy card that always has a way forward, A refused customer call remembers its chat and A step that keeps failing ends in one Blocked card.
- **New in 0.9.19.** Bot messages that never read as yours, bot messages that name the bot that sent them, a customer's refused call shown in that customer's chat, short Windows file paths in the chat's step titles, a stopped step that gets its tries back once the bot has fixed something, an island Allow once that no longer covers a deploy typed as plain text, a repeat guard that sees through changing timings, ids and ports, a stopped step that a read-only shell command no longer reopens, customer and bot messages that name who wrote them and requests to OpenRouter that name kwant as the app.
- **New in 0.9.20.** Needs attention rows that take you to the exact ask, Bots on API engines read files in their own folder, closed browser step cards headed by the step's name in grey, desktop settings that say so when they can't be saved, a Homebrew install for kwnt's server and command line, a bot browser that no longer runs out of connections after a long day, Home Assistant reads that run without asking, ready listings for Render, CasaOS, Umbrel and Coolify and customer and bot messages on the left, in their own bubble.
- **New in 0.9.21.** **Connect Home Assistant** with its address and an access token, and start the new **Home** team with House. Home Assistant locks, doors, gates and alarms always ask first, in every approval mode. **Always allow** lasts after a restart, and a bot's settings list each one under **Allowed without asking** with **Take back**.
- **New in 0.9.22.** One green and one purple across the app and Ollama bots that work again on their own when Ollama starts after kwant.
- **New in 0.9.23.** **Pictures from your tools** (like the new **FLUX** preset) are saved in the chat, and anything that spends credits asks first.
- **New in 0.9.24.** A Deny that reads as your no, not as a failed step, replies that keep the order you sent your messages in, no grey flash when the bot browser switches pages, a read-only check of what your bots ran without asking and address bars that show their own tab's page.
- **New in 0.9.25.** Fold titles centred in their rows on a phone, answered approval steps that say what you chose, a no that is saved as your no, also in routines, the link you send shown as the reply's first step, no blink when the bot browser panel opens after a run, a sign-in notice that follows its own tab's page, a run check that also catches local fetches with no card and failed grok steps that say why.
- **New in 0.9.26.** Routine runs you said no to shown in calm grey, finished steps you did for the bot that say so, home bots no longer told off for a true claim about another tool, false home claims caught before Home Assistant is tested, "You said no" only after a real Deny, steps nobody finished in time that say so, a bot browser panel that opens on the bot's own page, an address field that follows the tab you pick, one row per command you answered, step counts that match the rows in Details, an empty tab's address field that stays empty, a blocked push that says so on its own step, an address field that keeps what you type, a picked tab that leaves the bot's page alone, step summaries that wrap instead of being cut off on a phone, an address field that a page's title change leaves alone, a held push on Codex that says so on its own step, a picked tab's list entry that follows your own clicks, phone Details steps that open no taller than the rest, typed addresses that stay with their own tab, a held push on grok that says so on its own step, file steps that keep their full path, long grok tool errors shortened live, as on Claude, secrets in long tool errors hidden in full, phone step rows whose focus ring sits inside and whose paths wrap at a folder and no sign-in notes from a login word late in a long tool error.
- **New in 0.9.27.** **Deny can now say why**: tap **Say why** on an approval card, type what the bot should do instead, and it hears it right away.
- **New in 0.9.28.** **A runaway command can't take kwant down**: on Linux, when memory runs out, the system stops the bot's command or AI CLI, not kwant, and kwant says why. **Your reason for Deny shows in Details**, with the step it stopped.
- **New in 0.9.29.** **An approval asks once**: while a bot's approval card is on screen, the "needs you" bar above the message box steps aside instead of repeating it.
- **New in 0.9.30.** **Details shows the effort**: a reply made through a CLI no longer shows its "Executed via … CLI" line as reasoning; Details' Ran on row ends with the effort instead.
- **New in 0.9.31.** **The approval card stays put**: when Go to the approval flashes it, only the amber outline shows; the card no longer jumps. **Ran on keeps the effort together**: on a narrow window, Details' Ran on row keeps "· medium effort" on one line.
- **New in 0.9.32.** **Point stays out of the way**: its hover tag no longer covers the Point bar, and Point turns off once you send your picks.
- **New in 0.9.33.** **"Needs you" means it**: the sidebar says "Needs you" only when a bot asks, approves or failed, and a routine run counts once in the Inbox.
- **New in 0.9.34.** **Long logs fold**: a code block or log over 20 lines shows 16 with a "Show all" button, and Copy still copies it all. **Shared files preview again**: a CSV or text file a bot shares shows its table or text in the chat, not only a download button.
- **New in 0.9.35.** **Copy tells you when it fails**: a blocked clipboard shows "Couldn't copy" instead of "Copied". **Unpair clears Telegram buttons**: cards already in the chat lose their buttons when you unpair.
- **New in 0.9.36.** **Replies start clean**: no internal routing line flashes before a bot's answer. **Reply cards appear in one piece**: no empty card flashes before a bot's words.
- **New in 0.9.37.** **Stop stops the team**: a stopped teammate's work isn't restarted, and whoever waited on it shows Stopped. **Add-ons wait their turn**: what you add while a teammate works reaches them after their current task. **Team cards say when it's your turn**: a teammate waiting for your OK or answer says so in the team chat.
- **New in 0.9.38.** **One card per handoff**: a teammate that waits on another shows its handoff card once, not twice. **See when a teammate switched accounts**: a result that ran on another account says so in the team chat.
- **New in 0.9.39.** **The sales step waits for your answer**: a Sales team's "Tell the team what you sell" is ticked only once you write "What we sell:" and your answer. **Undo a ready-made team, then make it again**: its tile comes straight back, and a taken handle is flagged before you press Create.
- **New in 0.9.40.** **A failed AI download offers Try again**: in Get started, a failed Claude or Grok download stops with "Couldn't get Claude ready." and Try again instead of spinning. **Connect an app right from the bot's card**: a bot's "Connect Gmail" suggestion opens the Connect sheet for that app, with the bot already ticked. **The API key step waits for your key**: in Get started, Add account stays off until a key is pasted, and the account is named and made the default for you.
- **New in 0.9.41.** **Come back to what your bots finished**: the Inbox's "Done while you were away" now lists a single bot's finished job, under the request it answered. **Notifications say stopped or blocked**: while you're away, a bot that fails or is blocked tells you so, with the reason, instead of "replied" or nothing. **Can't reach the AI? It says which one**: with no connection, the reply names the provider and says to check the internet or proxy, instead of "connection failed". **Approvals you can read at a glance**: approval cards show ~/… folders, and Allow once leaves a short line in the chat where the bot asked.
- **New in 0.9.42.** **Tables and lists in what finished while away**: a finished job's answer in the Inbox shows its tables and lists formatted, and a one-bot job shows the bot's name instead of "1 bot". **Blocked bots show under Needs you**: when a bot working on its own ends with a "Blocked:" line, it shows under Needs you in the Inbox until you write back. **Search opens the right thread**: a search hit inside a bot's thread is grouped under the thread's title, and clicking it opens that thread with the message highlighted. **Typing stays quick in long chats**: each key no longer redraws every message, so a long chat types as fast as a new one. **See who is working at a glance**: on a computer, the sidebar shows busy bots in a Working block right under Needs you. **Long chats load their earlier messages**: scroll up, or press "Earlier messages", to load older messages 100 at a time without losing your place, until a line marks where the chat starts. **Scripts that send things out ask first**: in Ask only when risky, a bot's script given an email address or a web address that isn't this computer asks before it runs, and the card names the address. **Matching threads come first in search**: a thread whose title matches your search is listed first, and Enter opens it. **The lead reports back once after a Stop**: stopping one teammate while others finish no longer makes the lead post its report twice. **Home shows who needs you and who is working**: on a space's home, those bots move to the front with a small amber or violet dot. **Project scripts that send things out ask first**: in Ask only when risky, a script started with npm, pnpm, yarn, bun, uv, poetry, cargo, go, dotnet, gradle, mvn or make asks before it is given an email or web address that isn't this computer. **Inbox rows lead with what was done**: a Done row that wasn't checked shows just what the bot did, so the Checked rows stand out. **Approval alerts say what the bot wants to do**: a phone alert or Telegram message asking for your OK says "wants to run a command" or "wants to change a file", and the phone alert adds the bot's own line about the step. **Restart notes say the step in plain words**: when kwant restarts mid-reply, the chat note and the Needs you item say what the bot was doing, like "running a command" or "sending a message on Slack", instead of a tool id. **Steps read the same everywhere**: the approval card, the working strip, phone alerts and Telegram use the same words for a step, such as "write a file" or "reading the browser page". **Long chats open faster**: a chat with hundreds of messages draws less when you open it, and a narrow window opens right at the newest message. **See which job stopped**: in the Inbox, a stopped job is titled with the request you gave it, and the reason it stopped is on the line under it.
- **New in 0.9.43.** **Goals that need you show in the Inbox**: a goal waiting on you, such as a fix ready for your OK, shows under Needs you with the reason, and the board's "Waiting on you" counts it. **Fixes are checked the way your goal is**: a goal's Fix task has to pass the goal's own test, so it can't pass on a weaker one. **Goal alerts keep coming after the first incident**: a goal that breaks again the same day still tells you, and its questions for you always go out. **Homebrew builds leave updates to Homebrew**: kwnt built with the `no-update-check` feature never looks for a new release, and the Homebrew formula uses it (with `openssl@4` on Linux). **A goal's heartbeat fills in**: every check of a goal adds a square to its heartbeat ("3/3 ok · Steady"), and the timeline keeps to what happened. **A goal's timeline in plain words**: a goal's history says what happened and names the bot ("First check: OK", "Told you it broke"), and quoted reports drop their markdown. **One tap for Show details on a phone**: tapping Show details on an approval right after typing opens it on the first tap. **Board links open the board on a phone**: a link to the Mission board, Parallel Universe or Dev desk opens that page, not the chat list. **Goals say why no fix is tried**: when no fix can be tested against a goal's checks, the goal moves to Needs you once and says why. **Goal notices open the goal**: a tap on a notice about a goal opens that goal on the Mission board, not the bot's empty chat.
- **New in 0.9.44.** **Failed goal drafts say why**: when a bot can't draft a goal's checks, the Mission board shows the reason and opens the form with your words in it.
- **New in 0.9.45.** **Fixed goals say who fixed them**: the recovered notice names the bot whose fix worked, and a broken goal's notice gives the time it first failed.
- **New in 0.9.46.** **Sign in again and the work carries on**: signing in to a connection closes its "needs you" lines for every bot and each stopped chat picks up where it left off; deleting a connection closes its lines as "Removed."
- **New in 0.9.47.** **Sign in again opens the right row**: "Sign in again" in a chat or the Inbox opens Settings on that connection, lit and focused, with its own Sign in again button. **A suggested connection shows its real state**: its card says ready, needs sign-in or "Removed in Settings", even after a reload or on another device. **The Connect sheet names the app**: connecting an app a bot suggested shows "Connect Google Search Console", not its internal code name. **A failed routine tells you once**: kwant's automatic retries read "Retried automatically" in History, and you get one notice, not one per try. **A routine's unanswered question needs you**: a run whose question nobody answered reads "Needed your answer, didn’t finish" in History instead of OK. **Routine runs name their routine**: Inbox and away items for a routine run carry `routine_id`, so an app can run that routine again. **Failed routines speak plainly**: a failed routine run's Inbox row shows what the bot said about the failure, not the raw tool error. **Routine cards stay current**: the chat card that made a routine shows its current time and next run, Pause or Resume to match, and a note once it's deleted. **Routines on a phone**: a paused routine says Paused, and its on/off switch and Open result are full-size tap targets. **Tidier routine rows**: Inbox summaries drop stray bold marks, and a routine that posts in a thread opens that thread from its row and its failure notice.
- **New in 0.9.48.** **Routine History from the chat**: a routine's row in a chat has a History button that opens Routines on that routine's History. **Answered later**: a routine run that stopped for your answer reads Answered later in Routines once you reply. **Run again**: a failed routine run has a Run again button in the Inbox and on its Needs-you card, and its notice opens the routine in Routines.
- **New in 0.9.49.** **Chat fixes**: no blank strip above the message box after you accept a request, and a long new message lands in view instead of just below it.
- **New in 0.9.50.** **Your message on every screen**: a message you send now shows at once on your other open screens (another device or tab), above the bot's work, instead of after the bot's turn ends. **More room to read**: the message box takes less room. One Answers chip shows who answers and opens the member list, the tip line goes after your first message, and suggestions stay hidden while a bot replies.

0.8 added bots that show their work, approvals off reaching every engine, drag and drop into any chat, import from more places, fresh-eyes reviews with the pre-push card, and Claude kept away from your secrets on a Mac. Every change is in the [CHANGELOG](CHANGELOG.md).

---

## Why kwant

- **A team, not a chat tab.** Each bot has its own role, voice ("soul") and model. Bots @mention each other to hand work on, and a team lead runs short discussions and posts one summary. In a channel, kwant picks the bot that fits each message and shows why. The [Laya](#laya-model-router) router picks the model and effort for bots set to `auto`.
- **"Done" means checked.** The [Mission board](docs/mission-control.md) turns work into cards, one per hand-off. A card turns **Verified** only when kwant runs the check itself, such as a test or a file check, and the card keeps the receipt. Files a bot attaches are evidence to look at, never proof on their own.
- **Dev mode for code.** When bots write code, the [Dev desk](docs/dev-mode.md) shows what landed in each team's repo in plain words, and a bot that didn't write a pull request reviews it, tests and secrets included. Pushes to main, deploys and other production steps always ask first.
- **Facts with sources.** Bots back web facts with a link that opens the page at the exact quote. kwant checks the quote is on the page word for word and refuses one that isn't.
- **Spaces.** Separate spaces for work, a side project and home. Each has its own bots, teams, channels, routines, accounts, approval rule and hiring rule.
- **Every account you have.** Several accounts per engine: Claude, Codex or Grok CLI logins, Anthropic, OpenAI or xAI keys, and your own OpenAI- or Anthropic-compatible server. Set a default and a priority order. When an account runs out of usage, the turn moves to the next one and the chat says so. You can pin a bot to one account.
- **Works while you're away.** Routines on a schedule (bots can set them up when you ask, with an Undo button), follow-ups bots schedule for themselves, and an **Inbox** with what's waiting on you, what finished and what failed. Turn on Automation Discovery and kwant suggests a routine for a task you keep asking for, which you accept or dismiss.
- **Autonomous, with a safety net you control.** Ordinary work runs without stopping. On *Ask only when risky*, the default for new spaces, only risky steps such as `git push`, network calls, unknown commands or changes outside the bot's folder show an approval card with the exact command: *Allow once*, *Always allow* or *Deny*. Per space or per bot, switch to ask every time or don't ask. **Payments always ask**, in every mode: a payment command in the shell, or a checkout in the bot's browser, even behind a plain "Continue". Turn on Flight School and each bot earns more freedom per kind of work, but only you grant it.
- **Rules per bot.** Say what each bot is good for and what it should never get, and which sites its browser may open. Blocked sites always win.
- **Loose ends and letters.** kwant reads your chats and, if you add them, an IMAP inbox and your GitHub issues and pull requests, read-only, and finds what you started and never finished. Photograph a bill or a form and it pulls out the sender, the ask and the deadline and drafts a reply. It never sends anything.
- **Parallel Universe.** Compare up to three ways to deploy an app with local container tests and dated price estimates. Every number is tagged as verified, an estimate or speculation. No cloud account and no spending.
- **No telemetry.** kwant sends nothing to us, and its interface loads nothing from other servers. The one download you don't set up yourself is the [Laya](#laya-model-router) router on first start: its Python packages and its model from PyPI and Hugging Face (`--no-laya` turns it off). Everything else goes only to the AI providers and services you add.
- **And more:** memory and skills per bot (you approve new skills), bots that propose hiring a teammate, import a bot from a Claude Code subagent, a SKILL.md, a character card or a JSON export, search across a space's chats, MCP tool servers, a shared browser for bots, saved passwords with an optional Bitwarden store, and Local or VM computer mode.

See every feature, with screenshots, on [kwant.si/features](https://kwant.si/features).

<table>
  <tr>
    <td width="50%"><img src="docs/media/who-answers.webp" alt="In #general, a customer wore her new boots on one muddy trail and wants to return them. Support answers, with a Matched skills chip next to its name that shows why it was picked, from 7 bots. Wrong bot? is open, listing Lead, Frontend, Backend, QA, Researcher and Marketer."><br><sub><b>Who answers</b>: the bot that fits, and why. <b>Wrong bot?</b> teaches it.</sub></td>
    <td width="50%"><img src="docs/media/mission-board.webp" alt="The Mission board: 8 verified this week, 0 false successes, 2 needing you, a cost of $0.30 per verified result and a chart of verified results per active week. Below, two facts disagree (16 GB and 4 GB of RAM) with a Keep this one button on each, and QA asks for the Act tier on release notes, with Grant and Decline."><br><sub><b>Mission board</b>: verified results, their cost and what needs you (with Truth Ledger and Flight School on).</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/verified.webp" alt="A task, Weekly load-test report, with its Evidence receipt: Verified: file contains passed, checked by kwant. The check: load-test/report.md has p95. Below are the files the team attached, which never mark a task verified on their own, and the activity log."><br><sub><b>Verified</b>: only when kwant runs the check, with a receipt.</sub></td>
    <td><img src="docs/media/payment.webp" alt="TechLead wants to run a curl command to api.stripe.com to pay a domain renewal. The approval card says: This looks like a payment. Payments always ask you, even with approvals off. Deny and Allow Once."><br><sub><b>Payments always ask</b>, in every approval mode.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/bot-rules.webp" alt="A bot's settings. Good for: competitor pricing, gear reviews, market sizing. Not for: customer emails, refunds. Browser sites: only 3 sites, never 2 sites, no local network."><br><sub><b>Rules per bot</b>: Good for, Not for and Browser sites.</sub></td>
    <td><img src="docs/media/inbox.webp" alt="The Inbox, with New and Loose ends tabs. Waiting on you: Designer asks which hero image should ship on the landing page, with two answer buttons. Finished: a handoff and a Monday metrics digest. Failed or stuck: a competitor price scan and a smoke test."><br><sub><b>Inbox</b>: what's waiting on you, what finished and what failed.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/spaces.webp" alt="The space rail with three spaces, Trailhead, Pixel Quest and Home, each with its own icon and colour. Trailhead is open on #general with a weekly digest posted by a routine."><br><sub><b>Spaces</b>: each with its own bots, channels and routines.</sub></td>
    <td><img src="docs/media/accounts.webp" alt="Settings, This space, Accounts: the Order to use list. Work, a Claude login, is out until Thu 03:00; Personal, Codex and a Team gateway are ready. Under Not in the order, This machine's Claude, Codex and Grok and a Grok login are signed out."><br><sub><b>Accounts</b>: several per engine, with a default and fallbacks.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/media/routine.webp" alt="QA set up a routine when asked to check overnight checkout errors every morning: weekdays at 08:30, posting in the direct chat, with Undo, Edit and Pause buttons."><br><sub><b>Routines</b>: ask for "every morning" and get one, with Undo.</sub></td>
    <td><img src="docs/media/loose-ends.webp" alt="Loose ends with Open, Waiting and Snoozed tabs. The top case is a letter from a water company, with the quote it's based on and a draft reply."><br><sub><b>Loose ends</b>, the Inbox's second tab: evidence and a draft. It never sends.</sub></td>
  </tr>
</table>

Screenshots come from a demo space with sample data.

---

## Quickstart

The quickest way is the app for Mac, Linux or Windows (beta): download it, add an AI account you already have and ask your first question. There's nothing to build. On a server, see [Self-hosting on a VPS](#self-hosting-on-a-vps).

### What you need

- **A Mac** (Apple silicon or Intel) **or a Linux x86_64 desktop** for the app.
- **At least one AI account.** Any one of these is enough:
  - a Claude, ChatGPT or Grok account. You don't need to install or sign in to anything first: the **Get started** card installs the AI when it's missing, and you sign in from the app;
  - an Anthropic, OpenAI or xAI API key, which you add in the card or under **Settings → This space → Accounts**.
- To [build from source](#run-from-a-fresh-clone) instead: **Rust** (stable), installed with [rustup](https://rustup.rs), and **Node.js 20 or newer** with npm, to build the web app. The finished binary doesn't need Node.
- Optional: **Docker** for VM mode and the bots' shared browser, and [`uv`](https://docs.astral.sh/uv/) or `python3` for the Laya router.

If you build from source, `kwnt doctor` (`cargo run -- doctor` from a clone) checks all of these and prints the install command for whatever is missing on your machine; `kwnt setup` installs it (see [Without Docker](#without-docker-kwnt-setup)).

### On a Mac: the app

[Download the Mac app](https://github.com/alexsergivan/kwant/releases/latest/download/kwant_universal.dmg) (a universal `.dmg` for Apple silicon and Intel) and drag kwant to Applications. It runs its own kwant on your Mac, with the same `~/.kwnt` data folder as `kwnt serve`, and stays in the menu bar when you close the window. Build it yourself from [`apps/mac`](apps/mac/README.md).

The app is signed ad hoc, not notarized by Apple, so macOS stops it the first time. Click **Done**, open **System Settings → Privacy & Security**, scroll down and click **Open Anyway**. On macOS 14 or earlier, right-click the app and choose **Open** instead.

### On Linux: the app (beta)

The Linux app is for x86_64. Like the Mac app, it runs its own kwant, uses the same `~/.kwnt` data folder and puts an icon in the tray.

- **Ubuntu 22.04 or newer, Debian 12 or newer:** download [`kwant_amd64.deb`](https://github.com/alexsergivan/kwant/releases/latest/download/kwant_amd64.deb), then run `sudo apt install ./kwant_amd64.deb`.
- **Other distributions:** download [`kwant_x86_64.AppImage`](https://github.com/alexsergivan/kwant/releases/latest/download/kwant_x86_64.AppImage), then `chmod +x kwant_x86_64.AppImage` and run it. It needs a normal GTK desktop and FUSE.

Updates arrive in the app, which checks each one's signature before it installs it; with the `.deb`, your system asks for your password. `sudo apt remove kwant` removes the `.deb`, and your data in `~/.kwnt` stays. It's a beta: tested in containers on Ubuntu 22.04, Ubuntu 24.04 and Debian 12, not yet on real desktops, so the tray icon, Wayland and HiDPI may vary. There's no ARM Linux app yet.

### On Windows: the app (beta)

Download `kwant_X.Y.Z_windows_x64_beta_setup.exe` from the [latest release](https://github.com/alexsergivan/kwant/releases/latest) (its SHA-256 is next to it) and run it. It's for 64-bit Windows, installs for your user only with no admin rights, and runs kwant on your PC with the same `.kwnt` data folder in your user folder as `kwnt serve`. The installer isn't code-signed yet, so the first time Windows SmartScreen warns about an unrecognized app: click **More info**, then **Run anyway**. In the beta, the app doesn't update itself (a new version is a new installer), and Grok, the VM and looking inside videos aren't available on Windows yet. In a folder OneDrive syncs with Files On-Demand, Dev mode asks before every push.

### On your phone: iPhone and Android

kwant has a phone app with no app store: you install it from the phone's browser, and it opens full screen from its own Home Screen icon. You need a kwant server your phone can reach over **https** (a [VPS or Docker](#self-hosting-on-a-vps) behind an HTTPS reverse proxy). The desktop apps run kwant at `127.0.0.1`, which a phone can't reach.

1. **Open kwant on the phone.** On a computer signed in to your kwant at its https address, open **Settings → All spaces → Signed-in devices → Add a phone or browser** and scan the QR code with the phone's camera. It opens your kwant and signs the phone in once. Or open your server's address on the phone, tap **Sign in from another device**, and approve the code it shows from a signed-in device.
2. **Add it to the Home Screen.** iPhone and iPad: in Safari, tap **Share → Add to Home Screen**. Android: in Chrome, tap **⋮ → Add to Home screen** (or **Install app**).
3. **Open "kwant bots" from the Home Screen** and turn on **Settings → All spaces → Notifications → Notify me on this device**, so approvals and questions reach you with the app closed.

It loads from your server, so it updates when the server does. Step by step, with fixes for when it won't install: [kwant on your phone](https://kwant.si/docs/phone).

### Run from a fresh clone

```bash
git clone https://github.com/alexsergivan/kwant.git
cd kwant
make run          # builds the web app (frontend/dist), then runs `cargo run -- serve`
```

Open **http://127.0.0.1:8799**. A new person's `#general` opens with the **Get started** card: choose your AI, sign in, pick a team ([guide](https://kwant.si/docs/quickstart#get-started)). A fresh install starts with three bots in `#general`: Architect, TechLead and DevOps. Make more with **New bot**, from a ready-made persona or blank.

The binary embeds `frontend/dist`, so on a fresh clone `cargo build`, `cargo test` and `cargo run -- serve` fail to compile until you've run `make run` or `make frontend` once.

**The first start takes a few minutes:** kwant installs its Laya router and downloads its model (about 420 MB) in the background. Bots answer meanwhile, and the routing rules still pick who answers. To skip Laya, run `cargo run -- serve --no-laya`.

### Build a release binary

```bash
make build                 # web app + cargo build --release
./target/release/kwnt serve
```

### Install the web app

In Chrome or Edge, click **Install app** in **Settings → All spaces** (the sidebar also offers it once, as a tip); in Safari on a Mac, **File → Add to Dock**; on a phone, see [On your phone](#on-your-phone-iphone-and-android). Browsers only install pages served over https or from `127.0.0.1`.

To get notifications on your phone with kwant closed, install it (on an iPhone, add it to the Home Screen first) and turn on **Settings → All spaces → Notifications → Notify me on this device**. kwant then sends a Web Push notification for each kind you've ticked there, such as an approval or a question. It goes through your browser's push service (Google's for Chrome on Android, Apple's on an iPhone), encrypted so that service can't read it. Signing a browser out under **Signed-in devices** stops its notifications.

More in the [quickstart](https://kwant.si/docs/quickstart), [install](https://kwant.si/docs/install) and [phone](https://kwant.si/docs/phone) docs.

---

## Self-hosting on a VPS

### The binary

`kwnt` is self-contained: SQLite and the web app are inside it.

```bash
scp target/release/kwnt user@your-vps:/usr/local/bin/kwnt
ssh user@your-vps "kwnt serve --host 0.0.0.0 --port 8799"
```

On your own computer there is nothing to sign in to. From another machine, open the one-time link `kwnt serve` prints at startup (it works once, for 15 minutes; with `KWNT_ALLOWED_ORIGINS` set it is ready to open and comes with a QR code), or make a new one with `kwnt signin-link`. The browser stays signed in for a year. To add your phone or another browser later, open **Settings → Signed-in devices → Add a phone or browser** in a signed-in browser and scan the one-time QR code (it also works once, for 15 minutes); the same list lets you sign those browsers out. A new browser can also choose **Sign in from another device** on the sign-in screen: it shows a code for 10 minutes, and a signed-in browser shows **New device wants access** with Approve or Deny. Approve only a code that is on a screen you are holding. Prefer the token? On the server, as the user that runs kwnt, `cat ~/.kwnt/api-token` (or `$KWNT_DATA_DIR/api-token`, e.g. `sudo cat /var/lib/kwnt/api-token` with the systemd unit below) prints the token to paste (`kwnt token --raw` does the same; `./target/release/kwnt` if kwnt isn't on your PATH), and `kwnt token` prints a new one-time link like `kwnt signin-link`. Run them as the same user: as another user they fail, because that user can't read the token file (for the systemd unit use `sudo cat` as above). Each install has its own token, so one copied from your laptop won't open kwnt on the server. Or skip sign-in: `ssh -N -L 8799:127.0.0.1:8799 user@your-vps`, then open http://127.0.0.1:8799 on your computer (not with Docker, where requests always count as remote). A one-time link signs in as you, so keep it private until you've used it; the token never appears in the server's output. Put a TLS reverse proxy in front: see [browser service and VPS hosting](docs/browser-service.md).

<details>
<summary>A systemd unit</summary>

```ini
# /etc/systemd/system/kwnt.service
[Unit]
Description=kwant AI bot platform
After=network.target

[Service]
Type=simple
# A dedicated user (useradd --system --create-home --home-dir /var/lib/kwnt kwnt); sign the
# claude/codex/grok CLIs in as this user, or add API keys in the app
User=kwnt
ExecStart=/usr/local/bin/kwnt serve --host 0.0.0.0 --port 8799
Restart=always
Environment=KWNT_DATA_DIR=/var/lib/kwnt

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
systemctl enable --now kwnt
```

</details>

### VPS in 5 minutes (Docker Compose)

One command starts everything kwant needs: kwnt, the **Laya** router (with its model baked in) and a private **Chromium** for the bots (VM mode), with no Docker socket mounted. You need Docker with Compose and about 4 GB of free RAM (Chromium is capped at 1.5 GB).

```bash
git clone https://github.com/alexsergivan/kwant.git && cd kwant
KWNT_CLIS=claude,codex docker compose up -d --build   # any of claude,codex,grok; leave out for none
docker compose exec kwnt kwnt doctor                  # what works, and the fix for what doesn't
docker compose exec kwnt claude auth login            # sign a CLI in (codex login --device-auth, grok login --device-auth)
docker compose exec kwnt kwnt signin-link            # a one-time sign-in link and QR code (15 minutes)
```

Or skip the CLIs and add an Anthropic, OpenAI or xAI API key in the app under **Settings → This space → Accounts**. CLI sign-ins live in the `kwnt-home` volume and data in `kwnt-data`, so both survive rebuilds.

**HTTPS:** the UI is published on `127.0.0.1:8799` only. Put a TLS proxy in front, e.g. [Caddy](https://caddyserver.com) with this `Caddyfile`, and tell kwnt its address:

```
kwant.example.com {
	reverse_proxy 127.0.0.1:8799
}
```

```bash
echo KWNT_ALLOWED_ORIGINS=https://kwant.example.com >> .env && docker compose up -d
```

`docker compose exec kwnt kwnt signin-link` prints a one-time sign-in link for your domain and a QR code to scan with your phone. It works once, for 15 minutes; run it again for the next device. `docker compose exec kwnt kwnt token --raw` still prints the API token, as a fallback.

- **Image sizes** (measured): kwnt alone 0.2 GB; with `KWNT_CLIS=claude,codex,grok` 1.7 GB (the CLIs add about 0.9 GB, so they are off by default). Laya 3.2 GB, the browser 1.2 GB. Idle RAM: Laya about 1.7–2.3 GB, the browser about 0.4 GB, kwnt about 25 MB.
- **kwant's own `execute_shell` tool** runs in per-team Docker containers, so it isn't available inside this stack: that would need the host's Docker socket, which is root on the host. The CLIs' own shell tools work inside the kwnt container.
- **Upgrading from an older image:** kwnt now runs as uid 10001, not root. If it says `/data isn't writable`, run once: `docker compose run --rm -u root kwnt chown -R 10001:10001 /data`.

### Render, Railway, CasaOS, Umbrel and Coolify

[![Deploy to Render](https://render.com/images/deploy-to-render-button.svg)](https://render.com/deploy?repo=https://github.com/alexsergivan/kwant) [![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/kwant)

The button reads [`render.yaml`](render.yaml): one web service from the released image with a 1 GB disk on `/data`. **Render keeps files only on a disk, and disks need a paid instance** (Starter or up), so the free plan won't work. Render makes the sign-in token: copy `KWNT_API_TOKEN` from the service's **Environment** page and paste it on the sign-in screen.

The Railway button builds the same `docker/render.Dockerfile` with a volume on `/data`, plus the bots' private browser from `docker/browser.Dockerfile` on Railway's private network. Railway makes the sign-in token: copy `KWNT_API_TOKEN` from the kwant service's **Variables** tab.

For home servers, [`deploy/`](deploy) has a listing for each store, pinned to the release: [CasaOS](deploy/casaos/Kwnt/docker-compose.yml) (sign in with `kwnt token --raw` from the app's **Terminal**), [Umbrel](deploy/umbrel/kwant) (sign in with the password Umbrel shows for kwant) and [Coolify](deploy/coolify/kwnt.yaml) (sign in with `SERVICE_PASSWORD_64_KWNT` from the service's environment variables). Until a store lists kwant, CasaOS can import the file under **Custom Install** and Coolify can run it as a **Docker Compose** resource; Umbrel needs the listing. These run kwnt alone, without Laya or the bots' browser; see [docs/self-host-stores.md](docs/self-host-stores.md).

### Without Docker: `kwnt setup`

`kwnt setup` installs what kwant needs on this machine: Chromium or Chrome for the bots' browser, Laya (in kwant's data folder, with its own `uv` if you have no Python), and a CLI (`claude` unless you name others with `--clis`). It asks before each step (`--yes` skips the questions), never runs `sudo` itself (it prints the command for you instead), installs into your home folder where it can, skips Docker unless you pass `--docker`, and running it again changes nothing. `--dry-run` shows the plan. The app has the same checklist under **Setup**.

One line per platform, each followed by `kwnt doctor`:

| Platform | Install | Then |
|---|---|---|
| Mac app | Download the `.dmg` from [Releases](https://github.com/alexsergivan/kwant/releases) and open it | Run `kwnt doctor` |
| Linux app (beta, x86_64) | Download the `.deb` or the AppImage from [Releases](https://github.com/alexsergivan/kwant/releases) and [install it](#on-linux-the-app-beta) | Open **Setup** in the app |
| Mac, command line | `git clone https://github.com/alexsergivan/kwant.git && cd kwant && make build && ./target/release/kwnt setup` (uses Homebrew when you have it) | Run `kwnt doctor` |
| Linux desktop or server | The same clone and `make build`, then `kwnt setup`, or `sh scripts/install-linux-deps.sh --clis claude` on Debian/Ubuntu | Run `kwnt doctor` |
| VPS with Docker | [Docker Compose](#vps-in-5-minutes-docker-compose) | Run `docker compose exec kwnt kwnt doctor` |

On Linux (x86_64 or arm64, glibc 2.35+) you can skip the build: `curl -fsSL https://kwant.si/install.sh | sh` installs the prebuilt `kwnt` from the latest release after checking its `.sha256`, and every release also has `kwnt-linux-x86_64.tar.gz` and `kwnt-linux-aarch64.tar.gz`. There's no prebuilt command-line `kwnt` for macOS yet, so on a Mac the command line needs a clone and a build (Rust and Node).

---

## Laya model router

[Laya](https://huggingface.co/convaiinnovations/laya) picks which model and effort a bot uses when its model is `auto`. When kwant picks which bot answers in a channel, Laya adds its own guess from the bots' titles, but it never picks a bot on its own: kwant's [routing rules](docs/channel-routing.md) do. It runs as a small Python service, and **`kwnt serve` manages it for you**: on first run it installs `laya[serve]` into `~/.kwnt/laya/venv` (with `uv` when present, else `python3 -m venv`), starts it, and restarts it if it exits. The header shows its status. While Laya is unavailable, the rules still pick the bot, the model and effort come from a keyword estimate, and the reason line under a reply says it was a keyword match.

| Setting | `config.toml` | Environment | Default |
|---|---|---|---|
| Laya URL | `laya_url` | `LAYA_URL` | `http://127.0.0.1:8000` |
| Manage a local Laya | `laya_managed` | `KWNT_LAYA_MANAGED=0` to disable, or `kwnt serve --no-laya` | `true` |
| Checkpoints to preload | `laya_models` | `KWNT_LAYA_MODELS` | `english` |
| Use an existing `laya-serve` binary | | `KWNT_LAYA_SERVE_BIN` | |

The first start needs `huggingface.co`. Behind a proxy that blocks it, start kwant once on an open network or copy `~/.kwnt/laya/hf` from another machine.

---

## FAQ

**Is kwant free?** Yes. It's open source under the AGPL-3.0: use it, change it and run it for yourself, your team or your company, at any size. You pay only for the AI accounts you already have. If you want to sell or host kwant without publishing your changes, there's a [commercial license](COMMERCIAL.md).

**Can I use my Claude or ChatGPT plan?** Yes. kwant runs the Claude, Codex or Grok CLI you're signed in to, so your plan covers the work. You can also add Anthropic, OpenAI or xAI API keys, or your own OpenAI- or Anthropic-compatible server. When one account runs out of usage, the turn moves to the next.

**How is it different from Grok Bot, Claude Duet or OpenAI dots?** It's the same idea, a team of named bots you message, but open source, running on your own computer or server, with any of your accounts. And a task counts as done only when kwant runs the check itself. Side by side: [kwant vs Grok Bot](https://kwant.si/alternatives/grok-bot).

**Where does my data go?** It stays on your computer or server, in kwant's data folder (`~/.kwnt` by default). kwant has no telemetry and sends nothing to us. Your messages go only to the AI providers and services you add.

**Do I need an account with you?** No. There's no sign-up: you run kwant yourself and sign in only to the AI accounts you add.

**Which computers can run it?** A Mac (Apple silicon or Intel), a Linux desktop (x86_64, beta), a Windows PC (64-bit, beta) or a Linux server, with the binary or Docker Compose.

**Can I run it on a free server?** Yes. [This guide](https://kwant.si/blog/self-host-grok-bot-alternative-free-oracle-cloud) sets kwant up on Oracle Cloud's free tier, step by step.

---

## Documentation

- What changed in each release: [CHANGELOG.md](CHANGELOG.md)
- Website docs: [kwant.si/docs](https://kwant.si/docs/quickstart): quickstart, install, spaces, bots and teams, Mission board, Flight School, Truth Ledger, models and routing, accounts, routines, approvals, the bots' browser, loose ends, Parallel Universe, memory and skills, MCP.
- In this repo:
  - [Channel routing: who answers](docs/channel-routing.md)
  - [Mission Control and verified outcomes](docs/mission-control.md)
  - [Approvals](docs/approvals.md)
  - [Payments always ask](docs/payments.md)
  - [How bots use their browser](docs/bot-browser.md)
  - [Bot actions](docs/bot-actions.md)
  - [Browser service and VPS hosting](docs/browser-service.md)
  - [LLM profiles (accounts) and API access](docs/llm-profiles.md)
  - [Connections API](docs/connections.md)
  - [Live activity and human steps API](docs/live-activity.md)
  - [MCP API](docs/mcp-api.md)
  - [Saved passwords](docs/saved-passwords.md)
  - [Loose ends (zombie hunter)](docs/zombie-hunter.md)
  - [Letters (camera to completed)](docs/camera-to-completed.md)
  - [Parallel Universe](docs/parallel-universe.md)
  - [Dev mode](docs/dev-mode.md)
  - [Idea jar](docs/idea-jar.md)
  - [Dictation](docs/dictation.md): `GET /api/spaces/:id/transcribe` says whether the mic button works in a space; `POST /api/spaces/:id/transcribe` turns a recording into text

---

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and send a change, and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

```bash
make frontend                  # build the web app once (cargo test needs frontend/dist)
cargo test                     # Rust unit and integration tests
cd frontend && npm test        # web app tests
```

## Security

kwant runs commands and holds API keys and passwords, so please report vulnerabilities privately. See [SECURITY.md](SECURITY.md). Don't open a public issue for a security problem.

## Contact

- Email: [contact@kwant.si](mailto:contact@kwant.si), for questions, a commercial license, privacy requests or press.
- Bugs and ideas: [GitHub issues](https://github.com/alexsergivan/kwant/issues). Questions and setups to share: [GitHub Discussions](https://github.com/alexsergivan/kwant/discussions).
- Security problems: report them privately, as [SECURITY.md](SECURITY.md) describes.

## License

kwant is open source under the [GNU Affero General Public License v3.0](LICENSE) (AGPL-3.0-only).

- **Free for everyone:** use it, change it and run it for yourself, your team or your company, at any size.
- **Share your changes if you serve them:** if you change kwant and let others use it over a network, for example as a hosted service, you must publish your changed source code under the AGPL too.
- **Commercial license:** if you want to sell kwant, host it or build it into your product without publishing your changes, you can buy a commercial license instead. See [COMMERCIAL.md](COMMERCIAL.md).

Code published before the switch, including the mac-v0.1.0 release, stays under MIT OR Apache-2.0.

Contributions are accepted under the terms in [CONTRIBUTING.md](CONTRIBUTING.md#license).
