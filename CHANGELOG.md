# Changelog

Notable changes to kwant, newest first. Version numbers follow the Mac app releases
([GitHub releases](https://github.com/alexsergivan/kwant/releases)). The server (`kwnt`) and the web
app ship from `main`, so changes under **Unreleased** are already on `main` but not yet in a Mac download.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

## [Mac 0.9.37] - 2026-11-21

kwant 0.9.37: a Stop that stops the whole team, add-ons that wait for a busy teammate, team cards that say when a teammate waits for you and plain words when a teammate's model stops.

### Fixed
- Stop on a teammate now stops the team: nothing restarts the stopped work (no keep-going turn, no review turn after you stop the lead's summary), and a teammate that was waiting on the stopped one shows Stopped, not Failed (#970)
- Something you add while the team works now reaches the teammate: when the lead passes on a new line for a teammate that is still on a task from the same chat, it queues after that task ("Waiting for Writer to start") instead of being dropped. The same line twice is still sent once (#968)
- In a team run, a teammate waiting on you now says so on its card in the team chat ("Writer needs your OK to send an email" or "Writer asked you a question") with a "Go to it" button, instead of "Writer is working on it" (#973)
- A teammate whose model stops without answering now says so in plain words, with what to do next ("its model stopped without an answer. Try again, or switch its model."), and other driver errors lose the "Driver execution error:" prefix. The full text stays in kwant's log (#971)
- Line chart dates no longer overlap on tablets and medium-width windows: labels in the middle of the axis keep at least 8 px apart, and the first and last dates stay (#960)

## [Mac 0.9.36] - 2026-11-21

kwant 0.9.36: replies that start with the bot's own words, reply cards that never show up empty, a focus ring on the code block's Copy button, old Telegram cards cleared when you switch bots, line chart dates that don't run into each other and handoffs where the name and the words change together.

### Fixed
- Replies no longer flash an internal routing line ("Laya Router: model=…" or "Laya Team Router: Routed to …") before they start. kwant still picks the model and the bot the same way; that note now goes to kwant's log only (#959)
- A bot's reply card no longer flashes empty when a reply starts, when its first words appear, when the next bot takes over or after Allow once. The card and what is inside it now appear together (#961)
- The Copy button on code blocks now shows a focus ring when you reach it with the keyboard (#939)
- Switching kwant to another Telegram bot now clears the buttons on the cards the old bot sent, so an old tap can't approve or answer anything. kwant edits them through the old bot once, in the background (#941)
- Line chart date labels no longer run into each other at the right edge: the last date is always shown, and the label before it gives way when they would touch (#953)
- Switching kwant to another Telegram bot now clears the old bot's buttons even when Telegram asks kwant to slow down or has a short outage: kwant waits as asked and tries again (#963)
- At a handoff in a team run, the next bot's name now appears together with its own words, never over the previous bot's reply (#965)

## [Mac 0.9.35] - 2026-11-21

kwant 0.9.35: a Copy button that says when it couldn't copy, Telegram cards that lose their buttons when you unpair, phone charts with shorter date labels, no docker.exe left running after kwant is closed by force on Windows, no empty band above the banner of a paused chat and "needs you" cleared the moment you answer an approval.

### Fixed
- Copy now says when it couldn't copy, instead of always saying Copied. The Copy on a code block and under a reply show "Couldn't copy" for 2 seconds when the browser blocks the clipboard (for example on a plain http address), and the code block's Copy is easier to click on a wide screen (#937)
- Unpairing Telegram, in kwant or with /stop, now clears the buttons on cards already in the chat: an approval sent with Allow once keeps only its link to kwant, and a question says it was unpaired. A card still on its way when you unpair is cleared too (#935)
- On a phone, a line chart whose dates are all in one year shows them as month and day (09-07), so the labels have room. The tooltip, the table and screen readers keep the full date; wide charts, bar charts and mixed years are unchanged (#936)
- On Windows, closing kwant by force no longer leaves docker.exe running in the background: the short Docker commands kwant runs for its browser and sandboxes now end when kwant ends, however it ends (#943)
- On a computer, when a chat's budget is paused, the message box no longer leaves an empty band between the chat and the budget banner; the room for the "New" tag comes back once the budget clears (#944)
- Answering an approval in the chat (Allow once, Always allow or Deny) now clears its "needs you" bar and sidebar row right away, instead of a moment later; another ask still waiting in that chat keeps them (#947)

## [Mac 0.9.34] - 2026-11-21

kwant 0.9.34: long code blocks and logs that fold to 16 lines with a "Show all" button, files a bot shares that show a preview again, line charts whose last date label no longer overlaps the one before it, a sidebar that no longer shifts when you open Inbox, wide tables that fade at the edge while there is more to scroll, a composer whose New tag no longer sits on the chat and key numbers that show as tiles instead of one long line.

### Changed
- A code block or log in a reply that runs over 20 lines now shows its first 16 lines with a fade and a "Show all N lines" button; "Show less" folds it again and keeps the button in view. Copy still copies every line (#924)

### Fixed
- A CSV, TSV, Markdown or text file a bot shares in a chat shows its preview again (a table, or the text) above the download button, instead of only the button. Since 0.9.18 the preview was dropped for every real share. Files shared before this update stay as they were (#927)
- On a narrow line chart, the last date on the x axis no longer overlaps the label before it: when the last label is added off the regular spacing, the regular label just before it is dropped. Wide charts and bar charts are unchanged (#925)
- Opening Inbox no longer moves the sidebar's rows up by 2 px. The "New" count on the Inbox page is amber only when a bot is waiting on you, like the sidebar's Inbox count, and screen readers now hear which chat, team or bot is open in the sidebar (#913)
- A wide table in a reply now fades at its right edge while there are more columns to scroll to, and the fade goes once you reach the end. On a phone's comparison card, a Pros or Cons with one entry gets its + or − mark like the others (#929)
- On a computer, the "New" tag above the message box no longer sits over the chat: until you first use @, the box makes room for it, so no message or table passes under it. Phones are unchanged (#930)
- Key numbers in a reply, like revenue and orders, show as tiles again: kwant now asks bots for them as a '- **Label:** value' list, the shape it draws, instead of bare lines that ran together into one paragraph (#932)

## [Mac 0.9.33] - 2026-11-20

kwant 0.9.33: a sidebar that says "Needs you" only when a bot is waiting on you, and an Inbox count that counts a routine run once, goal drafts that reach the Mission board without a hidden card, a phone browser pane that points at Take over, a Settings "needs you" count that ignores unused accounts, a grey Inbox badge until a bot needs you, draft goal cards and board tiles that show their whole text, routine cards that say "Running" once while a run goes, a New space form without a choice you can't pick, cards you can answer the moment they appear, Telegram cards that close cleanly even while they are being sent and a browser that makes room for new pages instead of failing.

### Changed
- The sidebar heading says "Needs you" (amber, with the hand) only for a question, an approval or a failed run; when only finished work is waiting it reads "New" in grey with the Inbox icon. A finished routine run that has a review item now counts once, so the sidebar, the Inbox page and the phone tab bar show the same number (#907)

### Fixed
- Drafting a goal from the Mission board (New goal, Draft checks) no longer stops on an "Allow once" card in the bot's chat that you never saw: the draft now shows on the board, where you approve it. Other changes the bot makes in that turn still ask first (#904)
- On a phone, the idle browser pane no longer says "Enter a URL above to open a page", since phones have no address bar there: it reads "Ask a bot to open a page, or use Take over below" (or "Ask a bot to open a page" until the Take over button shows). Wide screens are unchanged (#906)
- Settings › Apps & accounts no longer shows an amber "1 needs you" for a signed-out account that no bot uses. An account counts only when a bot uses it or is pinned to it (or when no account is in use at all), and that account's row says "Used by <bot>" with an amber Sign in (#908)
- The Inbox count in the sidebar and on a phone's tab bar is amber only when a bot is waiting on you (a question, an approval or a failed run); otherwise it's grey and reads "1 new". With Inbox, the Mission board or another page open, only that page is highlighted in the sidebar, not the last chat as well (#909)
- On the Mission board, a draft goal's card now spans the row, so its title and "proposed this" line show in full (on a phone the title wraps to two lines). The drafting line no longer ends in a double period, and the "Board tasks waiting on you" tile is now "Waiting on you", which fits instead of being cut off (#914)
- While a routine runs, its card says "Running" once, in its header: the run's row in History keeps its spinner without repeating the word (screen readers still hear it). On a phone, the Run now button shows its Play icon like History, Edit and Delete (#916)
- The New space form no longer shows a "Start with" choice whose second option, "Copy bots from another space · Coming soon", could never be picked; on a phone the form now fits without scrolling (#921)
- Answering a bot's card (Allow or Deny) the instant it appears now reaches the bot: before, the answer could come back "This card expired" while the bot kept waiting for it (#923)
- A Telegram approval or question that is answered or closed in kwant while it is still being sent now gets its closing line ("Answered in kwant" or "No longer waiting") and loses its buttons, instead of keeping buttons that no longer did anything (#918)
- With several bots browsing at once, the built-in browser no longer runs out of room and fails with a download that never ends, an empty page or "browser busy": before a bot opens a new page it closes the least recently used idle bot page of any team, and if every page is in use it says plainly that the browser is full. Team pages are never closed (#910)

## [Mac 0.9.32] - 2026-11-20

kwant 0.9.32: a Point bar the hover tag no longer covers, and Point turning off once you send your picks.

### Fixed
- While Point is on, the tag naming the part under the pointer no longer covers the Point bar: when there is no room above the outline it sits below it, or inside it at the bottom left, so "Click the part you mean" stays readable. Point also turns off once you send a message with picks, and the cursor stays in the message box; a click, Esc and removing picks work as before (#891)
- Clicking a reply's quote again while the original message is still highlighted now starts the highlight over, so a second click always answers, as it already did for search hits and Go to the approval (#894)
- Saving App settings › Computer (Run Claude inside the VM, or where bots work) now asks Docker once instead of up to three times, so a save on a slow Docker answers in about 3 s instead of up to 9 s, and Run Claude inside the VM no longer shows as unavailable right after you turn it on (#895)
- Dev mode's production floor card now moves keyboard focus only after the screen shows its target, so after you save a backup command focus always lands on Back up first (and on the field or button you opened or closed), instead of now and then getting lost on a slow machine (#897)
- Keyboard focus now waits until the screen shows its target in 14 more places, so after you open a key field, allow approvals from Telegram, pick a mention, dictate or close a popover, focus lands on the right field or button instead of now and then getting lost on a slow machine (#898)
- After you take back an approval, pause a budget again, cancel a Claude sign-in, remove a coding-agent connection, a file or a hand row, focus now waits for the screen to update before it moves, so it lands on the next button (or Undo) instead of now and then getting lost when the app is busy (#900)

## [Mac 0.9.31] - 2026-11-20

kwant 0.9.31: an approval card that stays put when Go to the approval flashes it and Details' Ran on keeping its effort on one line.

### Fixed
- When Go to the approval flashes the approval card, the card no longer jumps down and back while the amber outline shows: it stays exactly where it was (#878)
- On a narrow window, Details' Ran on row keeps the effort together: "· medium effort" moves to the next line as one piece instead of breaking between its words (#884)
- Pressing Stop on a chat while a teammate's hand-off is reporting back now also stops the lead's follow-up turn; the result card still shows once (#882)
- Pressing Go to the approval again while the approval card is still flashing now starts the flash over, so a second press always answers (#889)

## [Mac 0.9.30] - 2026-11-19

kwant 0.9.30: CLI replies without a run line posing as reasoning, and their effort in Details.

### Fixed
- A reply made through a CLI (for example Claude or Codex on This machine) no longer shows its "Executed via … CLI · … [Effort: …]" line as if it were the bot's reasoning. Details keeps the effort on the Ran on row instead (for example "· medium effort"). Real reasoning still shows, and older messages are covered too (#872)
- After the idea jar hits a usage limit or its cap, it no longer starts one more take a few minutes later: the take is recorded as finished before the idea leaves its peek, so the pause and cap checks see it in time (#870)
- Details' Ran on reads "on this machine" for a reply that ran on the built-in This machine account, instead of "on the This machine account"; named accounts read as before (#873)
- When Go to the approval flashes the approval card, the amber outline now fits the card instead of running past it on a wide window (#875)
- When a teammate finishes a hand-off, the chat now reads busy until its lead picks up the result, so the lead is never started on a second review of the same hand-off in that moment (#876)
- Go to the approval, pressed from the keyboard (the "needs you" bar, the "Approval waiting" pill or Answer), now moves focus to the approval card itself, never to Allow or Deny, so a second Enter can't approve anything; a tap or click never moves focus. The "Approval waiting" pill also steps aside while the "needs you" bar already points at that approval, so there is one pointer (#869)

## [Mac 0.9.29] - 2026-11-19

kwant 0.9.29: an approval that asks once, on its own card.

### Fixed
- When a bot waits for one approval and its card's Allow once and Deny buttons are on screen, the "needs you" bar above the message box now steps aside instead of asking the same thing again. It comes back once you scroll the card away, and it stays when more than one thing is waiting (#861)

## [Mac 0.9.28] - 2026-11-19

kwant 0.9.28: a runaway command that can't take kwant down, your reason for Deny in Details and a Deploy on Railway button.

### Added
- **Your reason for Deny shows in Details.** When you deny with a reason, Details show your words with the step you stopped: on a command, as a second line under "You said no; nothing ran"; on any other card, on its own row ("You said why: ..."). Secrets in them are masked. A plain Deny reads as before (#849, #857)
- A Deploy on Railway button in the README: kwnt from the released image with a volume on /data, and the bots' private browser (#842, thanks to greqone)

### Fixed
- On Linux, when memory runs out, the system now stops the command or AI CLI a bot is running (and everything it started) before kwant itself. kwant says so in plain words: the computer ran out of memory, or something outside kwant stopped it. kwant keeps running (#852, #855)
- On a 320 px phone, Deny, Always allow and Always this session on an approval card keep each label on one line; when two don't fit side by side, the second gets its own line (chat, Away and the Inbox) (#851)
- Get started (the welcome in a new chat) now opens at its top instead of scrolled to the bottom; once the chat has a message it follows the newest one as before (#854)
- Someone new to kwant no longer sees the "Dev mode and Approvals moved to this space's settings" notice about a change they never saw; people who used kwant before still see it once. A space's Approvals setting now marks Ask only when risky as the default, the mode a new space starts on (#859)

## [Mac 0.9.27] - 2026-11-19

kwant 0.9.27: a Deny that can say why.

### Added
- **Deny can now say why.** Tap **Say why** on an approval card, type what the bot should do instead (up to 500 characters), and press Enter or the **Deny and tell** button: the call is denied and the bot that asked hears your reason right away, so it can try another way, and anything new it tries asks you again. The step still reads "You said no; nothing ran". An empty field is a plain Deny, and a plain Deny works exactly as before. Works on cards in chat, Away and the Inbox; Telegram keeps the plain Deny (#839, #840)

## [Mac 0.9.26] - 2026-11-19

kwant 0.9.26: routine runs you said no to shown in calm grey, finished steps you did for the bot that say so, home bots no longer told off for a true claim about another tool, false home claims caught before Home Assistant is tested, "You said no" only after a real Deny, steps nobody finished in time that say so, a bot browser panel that opens on the bot's own page, an address field that follows the tab you pick, one row per command you answered, step counts that match the rows in Details, an empty tab's address field that stays empty, a blocked push that says so on its own step, an address field that keeps what you type, a picked tab that leaves the bot's page alone, step summaries that wrap instead of being cut off on a phone, an address field that a page's title change leaves alone, a held push on Codex that says so on its own step, a picked tab's list entry that follows your own clicks, phone Details steps that open no taller than the rest, typed addresses that stay with their own tab, a held push on grok that says so on its own step, file steps that keep their full path, long grok tool errors shortened live, as on Claude, secrets in long tool errors hidden in full, phone step rows whose focus ring sits inside and whose paths wrap at a folder and no sign-in notes from a login word late in a long tool error.

### Fixed
- In a routine's History and on its card, a run you said no to now shows a grey **You said no** badge with **You said no to Bash; nothing ran** (with the tool's own name) under it, instead of a rose "A step failed". A run where something really broke stays rose, as before. (#794)
- Once a step the bot asked you to do is over, its row in a reply's Details and in the "What <bot> did" list reads **You did it: …**, **Cleared by itself: …**, **You said no: …** or **Asked you: …** (nobody did it in time, or it was stopped), instead of still saying "Waiting for you: …". The bot's own reason is kept exactly as it wrote it. Steps still waiting look as before. (#801)
- A bot that uses Home Assistant and other tools, and really changed something with another tool in the same reply, is no longer sent back to redo a home step when it truthfully says something like "I closed the issue." A sentence that names something in the home ("I turned off the kitchen light.") with no Home Assistant step behind it is still caught, and bots that only use Home Assistant behave exactly as before. (#800)
- On the Claude CLI engine, a Home Assistant server kwant hasn't tested yet now counts as a home server from the first message when its address (or a stdio argument) ends in Home Assistant's own /api/mcp or /mcp_server/sse path, so a bot that says "I turned off the light" without calling it is caught just as it is after a test. What a server says it is still wins over its address. (#805)
- A step now says **You said no** only when you pressed Deny. An approval card that went away with nobody answering is saved as stopped, not as your no. A routine step nobody answered in time reads **Nobody answered in time; <tool> didn't run** and still counts as a failure. A refusal you never saw reads **kwant blocked this; nothing ran**, or **Blocked before push; nothing ran** for kwant's check before a push. What the bot is told is unchanged. (#802)
- A step you didn't get to in time no longer shows as Done (#807)
- The bot browser panel opens on the bot's own newest page, without a jump while it loads (#808)
- Switching tabs in the bot browser shows the new tab's address at once, never the old one (#812)
- In Details, your answer to an approval shows on the command's own row instead of a second row (#811)
- A reply's step count no longer counts a command and your answer to it as two steps (#813)
- An empty bot browser tab no longer shows about:blank in its address field, and its title follows the tab you pick (#814)
- When kwant blocks a push, **Blocked before push; nothing ran** shows on that push's own step, never on another call of the same tool that failed on its own (#809)
- Typing in the bot browser's address field is no longer wiped every 2 seconds, and a tab you go back to shows its newest address (#819)
- A tab you pick in the bot's browser no longer overwrites the bot's own page address (#820)
- On a phone, a reply's step summary above Details wraps to two lines instead of being cut off (#821)
- A page that only changes its title, such as an unread count or a loading message, no longer wipes what you are typing in the bot browser's address field (#823)
- On Codex, a push kwnt's safety check held reads "Blocked before push; nothing ran" on its own step (#822)
- In a tab you picked in the bot's browser, the tab list follows your clicks and Back/Forward again (#824)
- On a phone, a step you can open in a reply's Details is as tall as the others (44 px, not 56) (#827)
- Text typed in the bot browser's address field stays with its own tab, and Enter shows the full https:// address (#826)
- On grok, a push kwnt's safety check held reads "Blocked before push; nothing ran" on its own step (#828)
- File steps in a reply's Details keep their full path, and kwnt run-check flags a file read or written outside the bot's folder without a card (#825)
- On grok, a very long tool error is shortened to 600 characters in the live view, as on Claude (#833)
- On a phone, a step's keyboard focus ring in a reply's Details sits inside its row, and a long file path in a file step wraps at a folder first, whichever engine ran it (Claude, Codex, grok or an MCP file tool); commands wrap as before (#832, #835, #837)

### Security
- A secret in a very long tool error is now masked in full before the error is shortened, so no part of it shows in the live view or the saved step; Codex's live tool error is now capped at 600 characters too (#836)
- A secret in a very long tool result from an API model is now masked in full before the live view shortens it to 4000 characters, and a very long tool error that mentions a login problem only after its first 600 characters no longer says you need to sign in again (#838)

## [Mac 0.9.25] - 2026-11-18

kwant 0.9.25: fold titles centred in their rows on a phone, answered approval steps that say what you chose, a no that is saved as your no, also in routines, the link you send shown as the reply's first step, no blink when the bot browser panel opens after a run, a sign-in notice that follows its own tab's page, a run check that also catches local fetches with no card and failed grok steps that say why.

### Changed
- **kwnt run-check** also lists a web fetch of this computer or your local network that ran with no approval card, since kwant asks before those (#816)

### Fixed
- On a phone, every fold (a reply's Details steps, a check's Details, a goal's Last output and the other fold-out rows) has its title in the middle of its 44 px tap row, instead of stuck to the top with an empty strip under it. The disclosure triangle stays, and desktop, tablets and phones with a mouse look exactly as before. (#791)
- Once you answer an approval card, its step in a reply's Details reads **You said OK: …** or **You said no: …**, and a card nobody answered in time reads **Asked for your OK: …**, instead of still saying "Waiting for your OK: …". The "What the bot did" list for work done while you were away reads the same way. Cards still waiting look as before, and replies saved before the update read the new way too. (#799)
- When you press Deny on an approval card, that step is now saved as declined with **You said no; nothing ran**, instead of a failed step reading "The user denied this tool call." A routine run whose step you said no to reads **You said no to Bash; nothing ran** (with the tool's own name) and counts as not done, never as ok. If something really breaks later in the same run, that failure's own reason shows instead. Refusals you never saw, such as kwant's check before a push, never say "You said no". (#792)
- When your message has a link, kwant's own opening of it before the bot starts is now the first step in the reply's Details: **Opening <host>**, with the page's title, or the reason it didn't open (such as a name that doesn't resolve). Before, a link that didn't open left nothing in Details. Old replies are unchanged. (#797)
- Opening the bot browser panel right after a run stays on one tab from the first frame: the pane stays dark until it knows which pages are open (at most 1.5 s), then shows that tab's page, with no flash of the bot's last page under "Shared" and no jump from one tab to another. It still follows a bot that opens a new page while the panel is open. (#804)
- The Shared browser tab's sign-in notice follows its own page, never the bot's (#817)
- On grok, a failed tool step shows grok's reason instead of just "failed" (#830)

## [Mac 0.9.24] - 2026-11-18

kwant 0.9.24: a Deny that reads as your no, not as a failed step, replies that keep the order you sent your messages in, no grey flash when the bot browser switches pages, a read-only check of what your bots ran without asking and address bars that show their own tab's page.

### Added
- **kwnt run-check** (new): one read-only command that looks through your own history and lists any tool step a bot ran with no approval card, and any card a bot asked again after you said no with nothing from you since. It reads only and changes nothing. It covers the last 7 days by default; `--since`, `--bot`, `--json` and `--strict` (exit 1 when it finds something) narrow it or put it in a script. (#798)

### Changed
- **Your no reads as a no.** When you press **Deny** on an approval card, the step you refused now reads **You said no** in calm grey, with the reason **You said no; nothing ran**, and the reply's Details say **you said no to 1** instead of "1 failed". A plain Deny no longer turns the reply's Details rose with "a step failed" or adds "A step failed, so check this before relying on it." under the reply. Real failures look the same as before, and replies saved before the update read the same way. (#790)

### Fixed
- **Replies keep your order.** Messages you send to a bot while it waits on an approval card are now answered in the order you sent them, in a chat or a thread, and also when the card times out. Before, the replies could come back shuffled, so a short answer like "Teal." didn't say which question it answered. Nothing is lost or merged, as before. (#787)
- Switching the bot browser to another page's tab no longer shows an empty light-grey "Finished" card for a moment first: the pane stays dark until that page's own picture arrives, then shows the page. (#795)
- Each browser tab's address bar shows its own page's address, never the other tab's (#810)

## [Mac 0.9.23] - 2026-11-18

kwant 0.9.23: pictures from your tools saved in the chat, a Deny that holds until you write again, clearer Home Assistant asks and a tidier Home Assistant sheet on a phone, buttons that stay readable on hover and in amber and House no longer claims a home change that didn't happen.

### Added
- **Pictures from tools land in the chat.** When a connected tool (an MCP server) returns a picture, a sound or another file, kwant saves it in the chat with a caption like **Made with FLUX · saved automatically**, and the bot gets the file's name instead of the raw data. Up to 10 files per call (pictures up to 20 MB, other files up to 100 MB); anything past that is named in the chat as not saved. Not yet: bots on Ollama don't save tool files in the chat, and on the Claude engine a tool result over about 25 MB doesn't come through (Claude's own limit). A tool that spends Black Forest Labs or Picsart credits always asks first, even with approvals off: the card says **This uses your Black Forest Labs credits.**, and Allow covers just that one call. Reading your balance or history never asks. **Tools › Find a tool** has a new **FLUX** preset: add it, then sign in. (#780)

### Changed
- **A no stays a no.** When you press **Deny** on an approval card, the bot is told not to try it again or another way unless you ask. Until you write in that chat again, kwant also refuses the same kind of step at once, with no new card: the same program and subcommand, the same website, the same file or the same tool. The step shows as declined: “You said no to a call like this earlier in this chat; kwant didn't ask again”. A message from you clears it; so does a restart. (#781)

### Fixed
- Home Assistant polish: on a phone the sheet's address wraps at a "/" instead of mid-word, the Needs-you strip and the pinned chat's tip say what House wants in words ("House wants to unlock Front door") instead of the raw tool call, House's suggested asks are about your home, and kwant.si and What's new now say bots can reach every device unless you limit them in Home Assistant. (#784)
- Button colours: red delete and revoke buttons darken on hover instead of washing out, the amber "Push anyway" and production-floor buttons use dark text so you can read them, the blue gradient buttons fade a shade deeper on hover, and the Spaces unread badges are easier to read. (#785)
- Home Assistant bots no longer say they unlocked a door or switched something off when no Home Assistant step ran: kwant sends the bot back once to make the call or say plainly that nothing changed, and if it still claims the change, kwant adds its own line under the reply. A filtered home check that fails now makes the bot read the whole home again instead of answering from the error. (#782)

## [Mac 0.9.22] - 2026-11-17

kwant 0.9.22: one green and one purple across the app and Ollama bots that work again on their own when Ollama starts after kwant.

### Changed
- kwant now uses one green and one purple everywhere. About 120 screens still had an older green and an older purple next to kwant's own teal and lavender (approval buttons, Ready chips, Documents, the Start picker, Hiring); they now match. Buttons with white text no longer get lighter and harder to read on hover, the main button's light corner is a little deeper, and "Allow once" is easier to read. Every button now reads at 4.5:1 or better. A build check stops off-palette colours from coming back. (#767)

### Fixed
- If Ollama starts after kwant does, your Ollama bots now work on their own, within about 15 seconds or at your next message, with no Re-check. Before, they kept saying "No account can run bots here yet" until someone pressed Re-check. A bot set to an Ollama model that still can't reach Ollama now says "Ollama isn't answering at <address>. Start it, then try again." with a Re-check button that tells you if it's still down, instead of asking you to add an account. A space that runs only on Ollama now counts it in its subtitle, as the sidebar already did. (#764)

## [Mac 0.9.21] - 2026-11-17

kwant 0.9.21: Home Assistant in two fields with a new Home team, Home Assistant locks, doors and alarms that always ask, a list of what each bot may do without asking, files that reach a bot waiting in the browser at once, a turn end that goes straight to the saved reply, a thread header that stays on Working until the reply appears, closed browser step cards that always say what's next, Needs-attention rows that land on an old ask in a thread, routine runs that say when a step waited for your approval or failed, instead of counting as done and replies whose Details say which step failed or didn't run.

### Added
- **Connect Home Assistant.** A new **Connect Home Assistant** sheet takes your Home Assistant's address and an access token. It builds the `/api/mcp` address and the sign-in header, tests the connection, and says what went wrong in plain words: a wrong token, MCP Server turned off in Home Assistant, an address it can't reach, or a server that isn't Home Assistant. The token field clears after every Connect, and leaving it empty keeps the saved token. Open it from **Tools › Find a tool**, or from the needs of the new **Home** team. That team template sets up House, a bot with its own house avatar, a Check the house skill and a Lights out routine that starts off. Approval cards and chat steps for Home Assistant use house words and name the device and its area, like **House wants to turn off Living room lamp**. (#763)
- **Always allow lasts, and you can take it back.** An **Always allow** answer on an approval card is now saved, so the bot still runs that tool without a card in that conversation after kwant restarts; another conversation, another bot or another tool still asks. A bot's settings list each one under **Allowed without asking**, with where and when, **Take back** on each row and **Take back all**; a take-back applies to the very next call. Payment and production steps are never saved. Behind it are `GET` and `DELETE /api/kwants/:id/always-allowed`, for people only (a bot's token gets 403). Saved answers are left out of exports and imports, and go when their bot, thread or channel is deleted. (#761, #768)

### Fixed
- A file you drop into the chat while a bot waits for you in the browser now reaches the bot at once: the browser step closes as Stopped, its card reads "Closed: <bot> is reading the file you sent.", the browser goes back to the bot and its next turn reads the file, instead of the file waiting up to 5 minutes. Typed messages still wait for Done or Skip, so a sign-in you are in the middle of isn't cut off. A late Done or Skip on any closed browser step now reads as a plain sentence. (#721)
- When a turn ends, the chat goes straight from the bot's working line to its saved reply. It no longer flashes back to an older state for a moment, such as the Approve / Change… row you just answered with a 👍, or a blank working line. A message you already answered with a 👍 never shows the Approve row again, even after the chat reloads. (#737)
- At the end of a reply in a thread, the header keeps saying Working, with its Stop button, until the reply appears, instead of going idle for a moment while the reply is still being written. The header, the "is writing the reply" line and the Stop button next to the message box now go idle together, on the phone's conversation picker too. Pressing Stop in that last moment stays quiet. (#752)
- A closed browser step card always says what's next. A step that timed out now adds "If you finished, say so in the chat.", and a step closed for a reason the card doesn't know says "This step is closed." instead of asking you to press a Done button that isn't there, or saying the bot is still waiting for you. (#756)
- A Needs-attention row for an ask in a thread now opens the thread at that ask and rings it, even when the ask is older than the thread's latest 100 messages; before, it stayed on the newest messages and the ask was out of sight. The thread's messages load around the ask the same way a channel's do. (#741)
- A routine whose step waited for your approval and got none no longer counts as done: the run says "A step is waiting for your approval, so it didn't run", a step that failed with nothing after it says so, and the bot is told not to claim it did what didn't run. Routine runs gain two statuses, needs_you and incomplete, and a step left unanswered shows "Waited 5 minutes for your answer; nothing ran". (#766)
- A reply's Details now say which step failed or didn't run, instead of one "a step failed" for everything. A step nobody approved in time reads "Details · a step didn't run (no answer)" and gets a line under the reply to check it before relying on it, with a Details button that opens that step; a step that failed for good reads "Details · a step failed" with the same line; and a step that failed and then worked on the next try reads "Details · 1 step failed, then worked", with no warning, because the reply is right. A routine run that's waiting for your approval says "Waiting for you" with a link to the card, a run nobody approved says "Needed your approval, nothing ran", and a run with a failed step says "A step failed" with its error. (#776)

### Security
- **Home Assistant locks, doors and alarms always ask.** On a Home Assistant server, a call that acts on a lock, a door, a garage door, a gate or an alarm now always shows an approval card, in every approval mode, Off and Ask only when risky included. The card offers no **Always allow**, and an earlier Always allow for that tool doesn't skip it. Reading the house (GetLiveContext and the tools the server marks read-only) never asks for this reason. A server counts as Home Assistant by the name it reports when it connects, never by its address, and a server not heard from yet counts as Home Assistant for these calls. The chat step names the device, like **Front door**. (#765)

## [Mac 0.9.20] - 2026-11-17

kwant 0.9.20: Needs attention rows that take you to the exact ask, Bots on API engines read files in their own folder, closed browser step cards headed by the step's name in grey, desktop settings that say so when they can't be saved, a Homebrew install for kwnt's server and command line, a bot browser that no longer runs out of connections after a long day, Home Assistant reads that run without asking, ready listings for Render, CasaOS, Umbrel and Coolify and customer and bot messages on the left, in their own bubble.

### Added
- kwnt's server and command line install on Mac and Linux with `brew install alexsergivan/kwant/kwnt`; packaging/ holds the Homebrew, winget and Scoop listings, checked by a new packaging workflow that installs each one on a fresh Mac, Linux and Windows runner. (#748)
- kwant on Render, CasaOS, Umbrel and Coolify: a Deploy to Render button (Render disks need a paid instance) and ready listings for CasaOS, Umbrel and Coolify under deploy/; a store install's sign-in screen says where its password is (KWNT_SIGNIN_HINT). See docs/self-host-stores.md. (#754)

### Fixed
- Clicking a Needs-attention row now opens the right chat at the exact ask and rings it, instead of sometimes an empty chat. A Needs you bar above the message box says the ask in full with one action (Retry, Continue, Ask again, Go to the question or approval) and "Done, clear it", and rows and the bar close by themselves once the ask is answered, replaced or dismissed. A stop only becomes a row once its notice is saved in the chat, and rows whose bot, channel or thread is gone close. (#700, #701)
- Bots on API engines, such as a Custom API or OpenAI account, can now read the files in their own folder with read_file and list_dir, a page at a time with the file's line count, and nothing outside it: paths that leave the folder, including links to a teammate's folder, are refused. The reads need no card, and a customer's turn still asks first. Fixes #715. Thanks to calintro. (#722)
- A browser step card that is closed (done, skipped, solved, timed out or stopped) is now headed by the step's name in grey, "Sign-in", "Human check", "Sign-in code" or "Browser step", instead of still saying "Sign in needed". An open card reads as before. Pressing Done or Skip on a card that had already closed now ends on the card's own footer, which says how it ended. (#734)
- In the desktop app, a setting that can't be saved (Show nudges in the menu, the Perch settings, Remove a server, Run kwant on this Mac) now keeps its old value and says once: "kwant couldn't save the change. Check that ~/.kwnt can be written." Before, the change looked like it stuck and was gone after a restart. Removing a server now saves first and signs it out only after that works, so a failed save no longer signs you out of a server that is still listed. A change that saves looks the same as before. (#744)
- Bots no longer lose their browser after a long day. kwant now closes a browser page's DevTools connections once the page has been idle for 10 minutes, and again when it closes the page, instead of leaving them open until the browser container ran out of room and every browser call failed with "Resource temporarily unavailable". A page closed this way reconnects on its next use and keeps its sign-in; a page someone is watching, a running turn, a download or an open file chooser is never cut. The browser image moves to kwnt-browser:v5, which caps its DevTools proxy at 64 connections. Thanks @calintro (#753, #759)
- Home Assistant's read-only tools (its live state and timers) now run without asking in "Ask only when risky". Home Assistant marks them both read-only and destructive, and kwant now trusts the read-only mark, as the MCP spec says; any tool not marked read-only still asks. Signing in to an MCP server on your home network over plain http now says to add an Authorization = Bearer <token> header instead, rather than a bare https error. (#760)
- A Telegram customer's message and a bot's chat message now sit on the left in an outlined bubble, with their name and letter, so only what you wrote sits on your side of the chat. Your own messages, from Telegram too, and a coding agent's stay on the right as before, and bot replies are unchanged. (#762)

## [Mac 0.9.19] - 2026-11-16

kwant 0.9.19: bot messages that never read as yours, bot messages that name the bot that sent them, a customer's refused call shown in that customer's chat, short Windows file paths in the chat's step titles, a stopped step that gets its tries back once the bot has fixed something, an island Allow once that no longer covers a deploy typed as plain text, a repeat guard that sees through changing timings, ids and ports, a stopped step that a read-only shell command no longer reopens, customer and bot messages that name who wrote them and requests to OpenRouter that name kwant as the app.

### Fixed
- A message a bot sends through the chat API now reads as that bot's in every bot's history (framed as a message from that bot, or from "unnamed" when it gave no name), never as yours. Other bots no longer take it as your instruction, and it no longer counts as your latest words when kwant lists open asks, settles a choice or counts a request's keep-going turns. (#724)
- A message a bot sends through the chat API now carries that bot's name, taken from its own turn, so other bots' history, hand-offs and follow-ups show who sent it instead of "unnamed". A request can't set or fake the name. (#728)
- When kwant refuses a call from a customer's message because nobody allowed it, the owner now sees one quiet line in that customer's chat with its time, such as "A customer's message tried to use memory search. It didn't run, because nobody allowed it. · 1:54 AM". It shows only in that chat, never in the bot's direct chat, for refusals from the last day. (#718)
- A chat step on a Windows file now names it as briefly as on a Mac or Linux: C:\Users\x\clients\acme\contract.md reads …\acme\contract.md, keeping the path's own backslashes, instead of the whole path with every client folder in it. Network paths (\\server\share\…) are shortened the same way, a bare drive like C:\ stays as typed, and Mac and Linux paths read exactly as before. (#735)
- A step that kwant stopped after it failed the same way 3 times (a failing npm test, say) can run again once the bot has changed something: an edit or write that worked, or a different command that worked, gives it 3 more tries. Reads don't count as a change, and the same failure is still stopped for good after 6 times in one turn, so a real loop still ends. (#738)
- The desktop island's Allow once no longer covers a command that ships when a bot passes it as plain text to a tool, like docker push, npm publish, make deploy or fly deploy: it now asks with the full card. Tools that sign someone up to a mailing list or off it (subscribe, unsubscribe, opt in, opt out) ask with the full card too. A note that only mentions a release, or names a tool like Vercel without a push word, keeps Allow once. (#736)
- A step that fails the same way 3 times is now stopped even when only the numbers in its error change, like a duration, a timestamp, a port, a process id, a sha or a uuid. The stop note still quotes the real latest error. An error that differs in a word still counts as a new failure, and a change by the bot still gives the step its tries back. (#743)
- Once a step is stopped for failing the same way 3 times, a shell command that only reads in the project folder (cat, ls, grep, git status, git diff, git log) no longer gives it 3 more tries. An edit, or a command that may write or reads outside the folder, still does. (#747)
- A customer's or a bot's message in the chat now names who wrote it, "Ann (customer)" or "Scout (bot)", with their own letter and a menu that says whose message it is, instead of "You" with your own picture. A customer or bot without a name reads "A customer" or "A bot". Your own messages, from Telegram too, still say "You". (#746)
- Requests to OpenRouter (chat, the model list and the account check of a Custom API account) now name kwant as the app (https://kwant.si), so kwant's usage counts on OpenRouter's app page and rankings. The headers name only kwant, never you or your space; prompts and files go to OpenRouter as before, and no other provider gets them. (#750)

## [Mac 0.9.18] - 2026-11-16

kwant 0.9.18: a team named once, never "… team team", audio files up to 100 MB in the chat, a bot browser preview that never flashes white while a page loads, browser step cards that stop asking for you once they are closed, tidier reply buttons on a phone, a production deploy card that always has a way forward, A refused customer call remembers its chat and A step that keeps failing ends in one Blocked card.

### Fixed
- A team named like "Marketing_Team", "Sales-Team" or "SalesTeam" now reads "@Marketing_Team" in the sidebar, the team home, the message box and the drop-a-file heading, never "… team team". (#694)
- You can drop audio files into a chat: MP3, M4A, AAC, WAV, OGG/Opus and FLAC, up to 100 MB each (other files stay at 20 MB). kwant goes by the file's bytes, not its name, so a program or archive renamed song.mp3 is still refused. A bot gets the audio as a file it can open, or upload on a page for you. AIFF, WMA and MIDI are still refused. (#712)
- The bot browser's live preview no longer flashes white or grey while a page loads: it keeps the last picture until the new page shows (at most 1.5 s, so a page that really is white still shows), and a fresh view opens dark instead of white. This covers the pane, its enlarged view, the phone controls and the takeover screen. (#726)
- A browser step card that is done, skipped, solved, timed out or stopped no longer says the bot "needs you in the browser": it now reads "<name> asked for you in the browser" in grey, with a grey hand, and only an open card keeps the amber hand and the ask. (#729)
- On a phone, a reply's buttons are tidier: Reply, 👍, 👎 and Copy sit on one row as small icons, the result buttons like "Show as text" get their own row below, the empty gap under a reply that browsed no pages is gone, and "From this reply" reads as a quiet caption instead of a chip. The small bot pictures in the Ask: row show an amber dot when a bot needs you, instead of a cut-off "!". (#680)
- A production deploy card that can't tell whether the deploy changes the database is no longer a dead end with only Deny: it offers Set backup command right in the card, then Back up first, and Allow without a backup after you tick "This deploy doesn't change the database". Steps that delete data or run a known migration still need a fresh backup, and a bot can never skip it. The card's Where line now shows the real server, not a variable like $R. (#696, #698)
- A customer's call that kwant refused now remembers the chat it happened in: the approvals list gives each row a link to that chat (a customer's Telegram chat is its thread), so it can show in the right conversation. A retry keeps the first chat, and rows from before this change list without one. (#706)
- A bot whose step keeps failing no longer repeats the same reply while kwant asks Keep going?: it stops once with a Blocked card naming the step and its error, such as "couldn't run journalctl …: Error: no terminal session is open." The same reply after Keep going is blocked too, and the Keep going card now says which step failed. (#705)

## [Mac 0.9.17] - 2026-11-16

kwant 0.9.17: a card that puts kwant bots on your phone, a team chip that names the team once, steps that fold under a reply instead of pushing it down, "Pick a repo" that always lands in Repo URL, Allow once for notes that mention a release or a merge, Blocked when a reply asks you to pick after a closing line, Claude bots that stop repeating a command or file read that keeps failing, The island names a file, never its folders, dates and times that read one way everywhere, a bot that asks for a missing file in the chat, Keep going never passes a customer's words on as yours and Writes named like reads keep the full card.

### Added
- **kwant bots on your phone.** Settings › All spaces now opens with **Use kwant bots on your phone**: show a QR code, scan it with the phone's camera and kwant bots opens there already signed in (the link works once, for 15 minutes), then add it to the Home Screen (Share › Add to Home Screen on iPhone, ⋮ › Add to Home screen on Android) and turn on notifications. No app store is involved. When a phone can't reach the address kwant runs at (127.0.0.1 on This Mac, or plain http), the card says why and what to do instead, and never shows a QR. Opened on a phone, it shows how to install that phone, and it hides once kwant runs from the Home Screen. The sidebar's one-time tip now reads **Tip: use kwant bots on your phone** and opens the card, **Install on this computer** moved into the card, and **Add a phone or browser** says how to add the phone to the Home Screen. The step-by-step guide is kwant.si/docs/phone. (#691)

### Fixed
- The team chip at the end of the Ask: row now reads like "@Booktun.ing Team", with no "(Team)" after it, and a long team name is cut short inside its chip and the Answers pill instead of running off a phone screen; the full name is in the tooltip. After you approve a step with the keyboard, focus goes back to the message box, also when the send fails. (#670)
- Once a bot starts writing its reply after using tools, its steps fold into one "Working · n steps" row under the reply instead of a full card above it, so the words don't jump when the reply lands, on a phone or a wide screen. The row opens to the full list, and a step waiting on you stays above the reply. (#676)
- In Manage team, "Pick a repo" now always puts the cursor in the Repo URL field, even when the app is busy, and switching tabs by hand afterwards no longer pulls the cursor back. (#695)
- The desktop island's Allow once no longer says "This pushes code" for a mail draft, page or note that only mentions a release, a merge or a deploy in a sentence, or for a read such as getting a GitHub release. Commands, branches, tags and actions still need the full card. (#697)
- After a failed step, a reply that asks you to pick, like "Let me know which one you want to keep." or "Let me know what else you need, or which file to use.", now shows Blocked. Polite closing lines such as "Let me know what you'd like to keep an eye on." still don't. (#699)
- On the Claude engine, a bot no longer runs the same built-in tool call (Bash, Read and the like) a fourth time after it failed three times in a row with the same error in one turn: the fourth try is refused with the same stop text kwant's own tools give, and the bot changes course or says what's blocked. Other engines are unchanged. (#693)
- The desktop island names only the file a bot is reading or editing, such as "Reading contract.md", never the folders around it, so a client's folder name doesn't show on a shared screen. Windows paths are cut the same way. The chat keeps its short path as before. (#708)
- Import history, a check result's time and the goal squares' tooltips now show dates the same way as the rest of kwant: always the day, and the year only when it isn't this year ("Nov 15, 3:42 PM"). The bot computer's terminal log keeps its time on one line beside the command, the Jira status dot sits beside the first line, and the branch on a Pushed line is cut short with … instead of breaking mid-name. (#709)
- A browser bot that can't get a file itself, such as a download its browser on another computer couldn't keep, now asks you to drop the file into the chat and uploads it on the page itself, instead of handing you the page. It never says it can't attach files, and for a file the chat can't take it says which file the page needs. (#710)
- When kwant nudges a lead to keep going, it quotes only your own messages: a bot's chat-API message is no longer quoted as yours, and in a chat where only a customer wrote, the hand-off it makes carries the customer's words marked as theirs instead of an empty request. In your own chats nothing changes. (#719)
- The desktop island's Allow once no longer lets through writes whose names sound like reads, such as retrying or changing a payment status, posting a status, listing posts or subscribing someone to a mailing list: they keep the full card. Plain reads such as getting a payment status keep Allow once, and Stripe's retrieve-payment and checking email now get it too. (#716)

## [Mac 0.9.16] - 2026-11-16

kwant 0.9.16: answers that show once, and named phone buttons, approval options in plain words, a reply name that stays the same when it lands, an island that names the search, never the words, follow-ups that never put a customer's words in your mouth, task activity that says when the lead was asked to review, no Blocked label for a polite closing line, Allow once again for ordinary read tools, one refused row per refused call, hand-offs that never pass a customer's words on as yours, times that never wrap away from their AM/PM, Allow once for plain reads of mail, files and payments, downloads that land in the bot's folder from kwant's own browser and a perch that shows on a Mac again.

### Added
- **Browser proxy.** `KWNT_BROWSER_PROXY=socks5://host:port` (or `http://`) sends the bots' browser out through a proxy, so sites see a home connection instead of a datacenter one. It applies to the compose browser, the browser container kwant starts itself and local Chrome; kwant's own services and private ranges always skip it. `kwnt doctor` checks that the proxy answers and that its exit address differs from the direct one. Credentials in the value are refused. See `docs/browser-proxy.md`.

### Changed
- **Your answer shows once, on the question.** When you answer a bot's question card, the card itself now says "You picked …" or "You answered: …" (with "· via Telegram" when you answered there), instead of repeating it in a bubble below. Smaller tidy-ups: the chats that wait on you are called **Needs you** everywhere; the space settings tab for bots is called **Bots**; on phones the chat header's buttons have captions (**Team**, **Pin**, **Edit**, **Browser**) and the row of chips fades on whichever side has more to scroll; **Parallel Universe** is a plain sidebar row instead of a menu with one item. (#640)
- **Approvals in plain words.** The space's approval options say what they do without jargon: **Ask before running commands** asks before a bot runs anything, changes a file or uses one of its apps; **Ask only when risky** asks before a bot goes online, installs something new, publishes or touches anything outside its folder, and says plainly that it isn't a locked box. On a phone the "New" tag no longer covers the @ button, and a team's lead comes first in **Ask:**. (#655)

### Fixed
- The name above a reply a bot is still writing now uses the same type and size as the finished reply, so the name line doesn't change when the reply lands, on a phone or a wide screen. (#664)
- The desktop island now says only what kind of search or helper a bot started, "Searching files", "Searching the web" or "Starting a helper", instead of the pattern, query or brief, so what a bot looks for never shows on a shared screen. The chat's activity list still shows the full words. (#674)
- When a bot follows up in a chat where a customer or a connected agent also wrote, the excerpt it reads labels their lines "(customer)" or "(agent)" and keeps them framed as outside messages, never as yours. Working ahead now starts only from your own message, so a customer's chat never spends your tokens on its own. (#683)
- A task's activity now says in plain words when kwant asked the lead to review a hand-off ("Kwant asked Researcher to review the hand-off") and when a bot wrote down what done means ("Writer wrote down what done means (3 points)"), instead of showing internal names. (#685)
- After a failed step, a polite closing line like "Let me know what else you need." or "Let me know what you find." no longer shows the reply as Blocked. A reply that asks you to pick, like "Let me know which file to use." or "Let me know what you'd like to do next.", still does. (#679)
- Telegram's Allow once and the desktop island's Allow once no longer refuse ordinary read tools with a wrong reason: tools like get_release, list_merge_requests, truncate_text, get_description, url_encode and geocode can be allowed again. Steps that run a command or push code are still sent to kwant for approval. (#682)
- When a bot retries a tool call that a customer's turn refused, kwant now keeps one "refused" entry for that call instead of one per retry. A call with different arguments, or the same call in a later turn, still gets its own entry. (#677)
- When a lead hands work to a teammate from a chat where a customer, a connected agent or another bot also wrote, the hand-off passes on your own newest message as the request. Their words go along only marked as theirs, never as "the person asked". In your own chats the hand-off text is unchanged. (#687)
- A time like "9:55 AM" no longer breaks across two lines on a narrow screen: the AM/PM stays on the same line as the time everywhere kwant shows one. The Jira card's "stopped accepting the token" line and a pushed change's "Pushed" line now say the day when it wasn't today. (#690)
- The desktop island's Allow once no longer refuses plain reads of mail, files, posts and payments with "This acts outside kwant": listing Gmail threads, reading a Dropbox or SharePoint file, getting a WordPress post or a payment status keep Allow once. Sending, replying, posting, paying, sharing, uploading and deleting still need the full card. (#692)
- A bot that uses kwant's own browser can now download files: each one lands in that bot's downloads/ folder with a note saying where, up to 400 MB. A bigger file is stopped with a note to copy its link instead or switch the bot to Local computer, and another bot sharing the same browser never gets it. A browser set to run on another computer still can't keep downloads, and now says so in plain words. (#686)
- The perch shows on a Mac again. Since 0.9.15 it stayed hidden: its window waited to measure itself on a screen frame that a hidden window never gets. It now shows at rest within a couple of seconds, and ⌥⌘K or Window › Open the perch opens it with the keyboard ready for Quick ask. (#720)

## [Mac 0.9.15] - 2026-11-15

kwant 0.9.15: the perch: what needs you and what your bots are doing, at the top of your screen, steadier dictation when several people talk at once, long replies that stream in a block at a time, replies that land without a jump, double-checks you can see wherever they run, customer messages that never count as yours, an island that names the program, never the command, a Pointing line that fits next to Refresh, a Blocked label for a bot that asks you to decide, a Telegram Allow once that never pushes code or runs a command, times that say which day, a hand-back line that fits, Claude bots that stop repeating a kwant tool call that keeps failing and an island Allow once that holds back more risky steps.

### Added
- **The perch (desktop app).** A small island at the top of the screen (around the notch on a Mac, a tab at the top centre elsewhere) shows the faces of the bots that need you or are working, with a count. Click it, or press ⌥⌘K (Ctrl+Alt+K on Windows and Linux), to see what needs you, what each bot is doing now and what just finished, in every place and space. A low-risk approval gets **Allow once** and Deny right there; anything riskier says "Decide in kwant: this one needs the full card." A question's first choices are buttons, and Quick ask sends a bot a message without opening the app. Settings › Perch sets when it shows (when busy, always or off) and its shortcut. (#652)
- **Place chip (desktop app).** The top of the sidebar names the place this window is on (This Mac, or the server's address cut in the middle so two servers stay apart), with an amber badge when something waits for you in another place. Click it to open the place picker. The web app shows no chip. (#649)

### Changed
- **Long replies stream in a block at a time.** While a bot writes, only the block it is still writing redraws; finished paragraphs, lists, tables and code blocks stay as they are, so a long reply keeps scrolling smoothly. The finished message looks exactly as before. (#651)
- **A finished reply doesn't jump.** While a bot writes, its reply already has the avatar, bubble and spacing it keeps once it's done, so nothing moves when it lands, on a phone or a wide screen. The cursor sits at the end of the line being written, and a numbered list that starts at 3 keeps its numbers while it streams. (#656)
- **You see a double-check wherever it runs.** While one bot double-checks another's work, its live reply and the working strip say who is checking which task, with **Open the task**, also on a phone. The task's activity now names checkpoints, checks starting and finishing, and overturned verdicts in plain words instead of internal names. (#659)

### Fixed
- Dictation now keeps at most two recordings in memory: a recording waiting for its turn (up to 30 seconds) no longer holds its audio, one that gets `busy` is never read, and the audio goes to OpenAI without a second copy. Dictation and memory search now pick the OpenAI key in the same order, and a slow transcription says "OpenAI didn't answer in time. Try again in a minute." (#648)
- A customer's message that tries a tool nobody allowed now leaves one closed entry in the bot's approvals list (`GET /api/approvals?status=refused`), with no card and nothing new sent to the customer, and a customer's message, or a connected outside agent's, never counts as your own answer, decision, request or go-ahead. (#660)
- The desktop island now says which program a bot is running, like "Running git", instead of showing the command, so nothing typed into a shell can appear on a shared screen. The chat's activity list still shows the full command. (#662)
- **Pointing fits in the computer pane.** While you point at something on a bot's computer, the footer shows "Pointing · 1 picked" on one line and no longer covers Refresh or Restart, on a wide screen and on a phone. The status line shortens first, and "Esc to stop" moves into the tooltip and screen-reader text. The hand-back line during a takeover gets the same fix. (#663)
- After a failed or declined step, a reply that asks you to decide, like "Let me know whether to retry or skip it." or "Tell me which mirror to use.", now shows as Blocked. A reply that only reports the failure and offers more help, like "The upload failed, and I can retry if you'd like.", no longer does, and "Let me know what you think." stays an offer. (#666)
- On Telegram, Allow once is no longer offered for a step that pushes code, runs a command (a command, script or SQL query) or that kwant flagged as risky: the message says "This pushes code." or "This runs a command." and asks you to approve it in kwant. A tap on an older message for such a step is checked again and goes the same way. (#667)
- A time from another day now always says which day: "You allowed this on Nov 12 at 4:16 AM" instead of "at Nov 12, 4:16 AM", and lines that showed only a time, like a push you let go, a question asked on Jira or a finished routine in the Inbox, add the day when it wasn't today. Times from today read as before, like "at 9:40 AM". (#669)
- While you've taken over a bot's computer, the footer's hand-back line now reads "Hand back to point", so it fits without being cut off, also on a phone, and "Pointing · 1 picked" no longer shifts sideways when you pick a second or third thing. (#673)
- On the Claude engine, a bot no longer runs the same kwant tool call with the same arguments a fourth time after it failed three times in a row with the same error: it gets the same stop text API engines already get, and changes course or ends with a Blocked line. The CLI's own tools (Bash, Read, WebFetch) aren't covered yet. (#671)
- The desktop island's one-tap Allow once now holds back more risky steps and opens the full approval card instead: a command passed under a key like input, line, statement or arguments; a tool that creates a commit or a branch; broadcast, ship and rollout tools; a delete, remove or drop under an action or operation key, a request_verb or a method-override header; and a body sent to a web address with no method. Reading tools such as get_commit or list_branches keep Allow once. On Telegram, a step that creates a commit or branch now also asks you to approve it in kwant. (#672)

## [Mac 0.9.14] - 2026-11-15

kwant 0.9.14: switching between This Mac and your servers in one window, a new chat from any message, smoother long chats while a bot writes, a Blocked label that catches an ask next to an offer and bots that stop repeating a call that keeps failing.

### Added
- **Switch places in one window (desktop app).** Window › Switch place… (⌘⇧O on a Mac, Ctrl+Shift+O on Windows and Linux) opens a small picker with This Mac and each server you added, its status and how many things wait for you there. Picking one switches the current window in place: it keeps its size and spot, and the place you left stays connected in the background, so its nudges keep coming. ⌥↩ (Alt+↩) or ⌥-click still opens it side by side, and ⌘1/⌘2 and the tray rows now switch in place too. (#650)
- **Continue in a new chat from any message.** In a chat with one bot, ⋯ under a message › **Continue in a new chat from here** opens a new chat with the same bot that holds the conversation up to that message, titled "<chat> (continued)", with the files sent up to then. A line at the top, like "Continued from "Price check" · 2 messages" (or "Continued from your main chat with Ben"), opens the original chat at that message; the original stays as it was. Very long chats bring the newest 1,000 messages. Team chats can't be continued. New `POST /api/messages/:id/continue` (the person only; bots get 403). (#618, #617)
- **`GET /api/glance` (server, for the coming desktop island).** One person-only call returns what needs you, what each bot is doing now (its current step, never its thinking), what just finished and which bots you can ask, across every space, with an ETag so a quiet server answers 304. `POST /api/glance/ask` sends a quick message into your chat with a bot. An approval is marked `can_allow_inline` only when it is low-risk: money, the push floor, GitHub writes, shell commands, publishing, deploys, sending email and anything that pushes, merges or releases always need the full card. The app doesn't use it yet. (#646)

### Changed
- **Long chats stay smooth while a bot writes.** A reply that's still coming in now updates the chat at most once per screen frame, and only that message redraws, so a long chat no longer stutters while a bot types. In a 300-message chat, script time fell from 29 s to 12 s per 40 s of streaming, and frames slower than 50 ms from 75% to about 4%. (#642)

### Fixed
- Only the person can answer a bot's question: `POST /api/questions/:id/answer` now refuses a bot's token with 403 `person_only`, so no bot can answer a question meant for you, its own or another bot's. Answers from the app and from Telegram work as before. (#653)
- A finished task's done line no longer reads as fully checked when some of its criteria weren't covered by any part: those criteria get one phrase, failed first, like "Failed: 2 of 2 parts checked; 1 criterion not met." The card's state and its checks are unchanged. (#654)
- After a failed or declined step, a reply that asks you for something and offers more in the same sentence, like "Paste the output, and let me know if it fails.", now shows as Blocked; one that only offers, like "Let me know if you want me to retry, or if you'd rather skip it.", no longer does. (#658)
- On API engines, Codex and Grok, a bot no longer sends the same tool call with the same arguments a fourth time after it failed three times in a row with the same error: it's told to change course or end with a Blocked line, instead of retrying until the call limit. The claude CLI and kwant's own tools aren't covered yet. (#657)

## [Mac 0.9.13] - 2026-11-15

kwant 0.9.13: bots that download and upload files in their browser, a mic to dictate into the message box, done cards that show each part of what you asked, a team home in empty team chats, double-checks where they count, with who and why, fewer false Blocked replies after a failed step, a clear Blocked when a bot stops after a failed step, one clock format across the app, customer chats on Telegram, memory search that leaves out unrelated notes and plain, consistent API errors.

### Added
- **Bots can use hover menus, downloads and file pickers in their browser.** A control that shows only on hover, like a video tile's ⋮ menu, now appears in the bot's list of controls, like **More (on hover) · Self-checkout**; clicking it moves the mouse over the tile first, and a new hover action shows what appears without clicking (it never counts as a payment). A file a page downloads lands in the bot's own folder under `downloads/`, never on your computer and never over a file already there; over 1 GB is cancelled. The new `browser_upload` tool gives a page's file picker a copy of a file from the bot's folder or from the chat's attachments; under Ask, each upload asks you first, and the approval card lists the files. (#580, #593)
- **Dictate into the message box.** A mic between @ and Send records what you say and adds the text to the box, like **Added 23 words. Read them over, then press Send.**, with **Undo**; nothing is sent before you press Send. While it listens it shows the time and a level meter (Esc or the mic stops it), and Send waits. In a space without an OpenAI key the mic is muted and says how to set it up, or you can hide it. On a phone the mic sits in Send's place until you type. This is the app side of 0.9.12's dictation server. (#623)
- **Done cards show each part of your request.** Under the check line, a finished task's done card now lists **Parts of your request**, in your own words, each with **Passed**, **Not met** or **No check**, so a part that nothing verified stands out. **Show what proves it (n)** folds out the criteria behind a part, and a part that was not met opens on its own. The checks list follows under **How it was checked** and starts closed when parts are shown. Cards without parts, or with only one written part, look as before. This is the app side of 0.9.11's request parts. (#626)
- **Team chats open on a team home.** An empty team chat now shows the team's members with the lead marked ("· lead"), its description or a line like "3 bots who work together. Ada leads.", three "For example" asks, and who answers, like "The helpers team answers. Type @ and a name to ask one member.", instead of a bare pane. With no AI account connected, the account card sits inside that home. The home goes once the chat has a message; bot chats and #general look as before, and an empty chat now opens at its top. (#633)
- **Customer chats on Telegram (server, off by default).** A business can give its customers a second Telegram bot, separate from the person's own paired one; a bot the person picks answers. Each customer gets one thread, only the bot's final plain-text reply goes back (links into kwant are dropped), and approvals and questions never reach customers. In a customer turn every tool call waits for the person, and the bot gets kwant's own tools only: claude runs with `--tools ""`, codex and grok answer in text only. A `[[person]]` line or a failed turn hands the chat to the person with a Needs-you item. Limits: 20 messages per chat per hour, 200 chats per space, `/stop` and `/start`. The token lives only in the vault. Set up with `GET/PUT/PATCH/DELETE /api/connections/telegram/customers` and answer with `POST …/customers/chats/:chat_id/reply`; the app has no screen for it yet. (#622)

### Changed
- **Routine runs are double-checked when it counts, and you see who is checking.** A routine run that nothing else checks is double-checked on the routine's first run, after its instructions or acceptance change, and then at most once every 7 days; a run in between notes in its activity that the routine was already double-checked and when the next check is due. High-priority work, hand-offs, goals and checks you ask for are double-checked as before, within the daily cap. While a check runs, the checking bot's line in the sidebar, the working pill and the board's **Live now** say **Double-checking “<task>”**, with who checks whose work and why on hover; the task's activity and risks say the same. (#641)
- **Every time in the app reads the same way.** Clock times and short dates now follow your device's own format everywhere: a 12-hour device sees "8:15 PM" and a 24-hour device sees "20:15", in chat, the Inbox, routines, reminders and the step log alike (before, the Inbox showed "20:15" while chat showed "8:15 PM"). A time from another day carries its date, like "Nov 3, 8:15 PM". A routine's stored time is unchanged, only how it reads, and there's no new setting. (#635)
- **Every `/api` error has one shape:** `{error, code, detail?}`. `error` is a plain sentence, `code` is a short name like `bad_request`, `unauthorized`, `not_found`, `bad_json` or `archived`, and a request the server couldn't read (bad JSON, a missing field) now gets a plain sentence instead of the framework's text. A document name with hidden direction characters is refused with `bad_name`. Clients that read `error` keep working. (#638)

### Fixed
- After a failed step, a bot that only offers choices, like "I can retry, or try another way", "tell me if you want me to…" or "whenever you're ready", no longer shows as Blocked or raises a question card; a reply that really waits on you still does. When a step was blocked, the Approve row stays on the reply, so you can still approve the step from there. (#639)
- A bot that stops after a failed or refused step and then waits on you in its own words now shows as Blocked, and a lead asks you with one question card, instead of looking finished. It counts a failed last tool step, a declined approval and a link that didn't open before the turn, also when decide-first held the first reply; offers like "Let me know if…" don't count. (#634)
- Bots' memory search no longer returns unrelated notes. A match by meaning now needs a minimum similarity for each embedding model (0.25 for OpenAI, 0.5 for local Ollama models), so a search with no real match says "No matching notes." instead of listing 10 loose ones; matches by words are unchanged. A note saved while vectors are being filled now gets its vector in the same fill, a host OpenAI key that fails its check is no longer tried for embeddings, and a cancelled fill no longer blocks the next one. (#630)

## [Mac 0.9.12] - 2026-11-14

kwant 0.9.12: Dev mode that says when it finds your code and lighter long jobs on API engines.

### Added
- Dictate into the message box, server side: new `GET /api/spaces/:id/transcribe` says whether dictation works in a space (and why not, in one sentence), and `POST /api/spaces/:id/transcribe` turns a recording (webm, mp4, mpeg, wav or ogg, up to 25 MB) into text with OpenAI speech-to-text, using the space's own OpenAI API-key account, else the host's OpenAI key unless This machine is off for the space. Only the person can dictate (bots get 403); the audio isn't stored, and the text comes back for editing, so nothing is sent before you press Send. The app has no mic button yet. (#610)

### Changed
- **Dev mode is easier to find.** It's now the first card in Space settings › General, and when kwant finds a team's repo the card says so, like **Found octocat/Hello-World in Web team. Turn it on and your bots review pull requests, show what landed and guard deploys.**, with **Turn on Dev mode** and **Details** (branch to watch and backup command). #general shows a strip, like **Code space has code: octocat/Hello-World**, whose **Turn on Dev mode** opens that card without turning anything on; **Not now** hides the strip for that space. With no repo found, the card asks for one (**Add a repo first.**), and a folder without git doesn't count as found code. (#629)
- **Long jobs on API engines send much less.** On API-key engines (OpenAI-style, OpenRouter, Ollama and the Anthropic API), once a turn's older tool results add up to more than 24 KB, they are cut in one batch to a short note with their first 300 characters, so the prompt cache keeps working; the newest 4 results, errors and results under 1 KB stay whole. The bot reads any trimmed result back in full with the new built-in `kwnt_read_tool_result`. In a 30-step test turn this sent 66% fewer bytes, and the last request was 85% smaller. The claude and codex sign-in engines are unchanged. (#620)

## [Mac 0.9.11] - 2026-11-14

kwant 0.9.11: Fix it and line notes that go to the bot you pick, a Documents tab for the files bots quote and done cards that check each part of what you asked.

### Added
- **Pick which bot fixes it.** Each open finding on a PR card now says who **Fix it** goes to, like **Goes to Ana · lead of Edge**. Press that line to pick another bot of the space from a list (a popover on desktop, a sheet on a phone, with search when the space has more than 8 bots); the default stays first: the PR's author bot, else the lead of the team that owns the repo. Line notes go the same way with **Send N notes…**, a card with no default asks you to pick a bot, and **Open task ›** opens the task once the Undo window has passed. Behind it are the new `GET /api/dev/cards/:id/recipients` and an optional `to` on Fix it and line notes (a bot of another space is a 400 `bad_recipient`); the task, and its redelivery after a restart, goes to the bot you picked. (#609, #615)
- **Add and remove a space's documents in the app.** Settings › This space › **Documents** lists the files bots quote and takes new ones: drop them in or **Choose files** (.md, .markdown, .txt or a PDF with text in it, up to 2 MB each, 200 per space). A document that's already there asks before it's replaced, and the bin removes one. Only you can add or remove documents, not a bot. In chat, a quote from a document shows a chip with the file and its PDF page; it opens the passage on its page, marked. New `POST /api/spaces/:id/docs?name=` (the file as the body, `&replace=1` to replace), `DELETE /api/spaces/:id/docs/:name` and `GET /api/spaces/:id/docs/page?file=&page=&quote=`. (#614, #613)
- **Done cards check each part of what you asked.** A lead now cuts your request into parts, in your own words, and links each part to the criteria that prove it. A finished task's check line on its Inbox done card then counts parts and names one that didn't pass, like `Partly checked: 1 of 2 parts checked; "send it to Ana": no check.` A part passes only when every criterion linked to it passed with evidence, so a bot's claim never counts. Tasks without parts read as before. The task contract (`GET/PUT /api/tasks/:id/contract`) has a new `parts` list; no migration. (#619)
- **Download receipts.** Under This week on the Mission board, **Download receipts** saves a CSV of the space's finished tasks from the last 30 days, 90 days or 12 months, with who checked each one, how, the latest verdict and the evidence (17 columns). Days are UTC, both ends included. Behind it is the new `GET /api/tasks/receipts.csv?from=&to=&state=`: `state` is `closed` (default), `verified`, `failed` or `cancelled`. The file is safe to open in Excel and Sheets (RFC 4180 quoting, cells starting with = + - or @ are guarded) and holds up to 10,000 rows (`X-Kwant-Truncated: true` when cut, and the app says so). (#611, #621)

## [Mac 0.9.10] - 2026-11-14

kwant 0.9.10: usage bars for each AI account, a Done mark and an Approve button on replies and one search for every app.

### Added
- **See how much of each AI account is left.** On Apps & accounts, each Claude or Codex sign-in shows two thin bars, like "5-hour · 93% left · resets 22:29" and "Weekly · 44% left · resets Wed 00:59"; a bar turns amber below 20% left. Claude's numbers come in with every turn. **Check now** reads them when you ask, at most once a minute and never by itself: for Claude one tiny reply on the cheapest model, for Codex its own limits read. A failed check says why, like "Codex is signed out". The model badge on a bot's avatar shows the same line, with an amber dot when its account runs low. API-key accounts show no bars. (#599, #604)
- Bots can read and quote a space's own documents: put Markdown, text or PDF files (up to 2 MB each, 200 files) in `docs/` in the space's data folder (`<data dir>/docs` for Default). A quote is checked word for word, like a web quote, and points to the file and the PDF page. New `GET /api/spaces/:id/docs` and `POST /api/spaces/:id/docs/cite`. (#601)
- Dev mode can give bots Google Chrome's Modern Web Guidance. When Dev mode finds web code in a team's folder, kwant offers a built-in `modern-web` skill once, as Proposed for every bot in the space; nothing changes until you approve it in Skills. The skill looks guidance up with a pinned `npx -y modern-web-guidance@1.0.0 search|retrieve|list`, which runs without an approval card under "Ask only when risky"; any other version, flag or chained command still asks, and turns kwant starts on its own still ask. Needs Node and the network the first time. (#605)

### Changed
- **Replies say when a job is done.** A job's final reply gets a **Done** mark after the bot's name, or what checked it: **Checked**, **Partly checked**, **Check failed** or **Not checked**. When a reply asks for your OK, an **Approve** / **Change…** row sits under it ("Ben is waiting for your OK"); Approve sends "Approved, go ahead.". The name line is now name, mark, time and **Details**, and Details says in words why that bot answered. Copy is a quiet icon, and the engine chip shows only when a bot switched AI mid-reply. (#602)
- **Find any app in one list.** On Apps & accounts, the Another app search shows Composio apps, built-in tools and tool servers from the public directory in one list, one row per app. Each row says where it comes from and how it signs in, like "Sign in with your Notion account" or "Runs on this computer · needs Node.js"; other ways to add the same app fold under the row. One status line says when a source can't be reached. (#606)
- Bots' memory search now finds notes by meaning as well as by words, with the space's own embeddings: its OpenAI API-key account or the host's OpenAI key (text-embedding-3-small), or a local Ollama with an embedding model. Without one of those it is the same keyword search as before, with no extra calls. (#603)

## [Mac 0.9.9] - 2026-11-13

kwant 0.9.9: a browser panel that follows your bot, AI use over 7 or 30 days, by model and routines that are easier to set up.

### Added
- **A browser panel that follows your bot.** When a bot opens a page in its browser, the panel opens by itself in a wide window, with the bot's name ("Ben's browser"), a Browsing now or Finished line and a Live pill, and closes again shortly after the bot is done, unless you used it. **Pages this reply** lists every page the reply opened, by title, newest first, with ● Now on the current one. Each reply that browsed gets a chip ("Ben opened 2 pages") that reopens the panel; on a phone the chip opens it as a sheet. **Point** and **Take over** sit under the preview, and **Enlarge** shows the page bigger, view only. Browser steps now carry the page's title (`page_title`, redacted, up to 160 characters) for Claude, Codex and API bots. (#588, #589)
- **AI use over 7 or 30 days.** **AI use** in Settings › All spaces › Usage switches between 7 days and 30 days. It shows the total ("$4.20 in the last 7 days"), a bar for each day, with a gap on a day without use (arrow keys move between days and read out each one), and **By model**, each model's share; a model that reported no cost says so. When nothing reported a cost, it counts fresh tokens instead. `GET /api/usage/daily` adds `models`. (#592)

### Changed
- **Routines are easier to set up.** The sidebar has a **Routines** row that opens Settings › This space › Routines. With no routines yet, one card says what a routine is, with **New routine**. The form gives examples ("Competitor prices"), shows your time zone in one quiet line ("Kyiv time GMT+2", with **Change**), picks today for a new weekly routine and has a **Weekdays** chip for Monday to Friday. Each card shows the first line of its instructions, when it runs next or that it's paused, and its schedule in words, like "Weekdays at 08:30, Kyiv time". On a phone the card buttons are 44 px. (#591)

### Fixed
- Settings no longer gets stuck on "Update checks are turned off here.": the Mac and Linux apps' update check is ready before the window first asks, and if checks still answer off, the version card asks again on its own and offers **Check again** and **Get the latest version**, a link to the latest release. (#596, #597)
- Adding or renaming a tool server to a name that differs from another one only in case, like Stripe next to stripe, is now refused with the same "already exists" error as an exact match, instead of keeping two. (#590)
- When kwant's Chrome closes right after starting, the message now leads with the line that names the error, like "Missing X server or $DISPLAY", instead of the last line, which was often D-Bus noise. (#595)

## [Mac 0.9.8] - 2026-11-13

kwant 0.9.8: a first screen anyone can start from, done items that say what checked them, a pre-push retry that lets the fixed push go, mission checks that wait for outside services, a chat that says when it only waits for your answer, routine runs that show as routines and @mentioned teammates who speak next.

### Added
- **A first screen for anyone.** A fresh install now starts with a small team, @helpers: Assistant (lead), Researcher and Writer. #general opens with "What do you want done?", your team's faces and three example asks. With no AI account connected yet, the chat says **Connect an AI account to start**, with a **Connect an account** button. Dev mode and Approvals move from the sidebar to Settings › This space › General; the space header shows them in one line ("3 bots · Dev mode · Approvals off · 1 account"), and past users get a one-time pointer to where they went. If your space already has a team, Get started's third step is **Your team**. Existing installs keep their bots and teams as they are. (#583, #585)
- **Done, and here's the check.** A done item in the Inbox says what checked it, in one box above the answer: "Checked", "Partly checked", "Failed" or "Not checked", and **Show what was checked** lists each check with its result and evidence: the task's checks, its criteria and the delivery check. A criterion marked met without evidence counts as a claim, not a check. When nothing checked it, the box says so ("Read the answer before you rely on it."), and a routine's box has **Add a check**. Inbox rows show the same word before the summary. `GET /api/inbox` carries a `checked` object on done items. (#566, #577)
- **Checks that wait for outside services.** A mission check can wait for something that finishes elsewhere, such as a store review or a build farm, instead of failing or spending bot turns on polling. A command check that exits 75, or an http check that gets its `waiting_status` or finds its `waiting_contains`, waits with the task in review and runs again every 15 minutes (`recheck_mins`). The task card says so in one line, "Waiting on ./gate.sh since 09:12 · next look 09:27", with the last line the script printed, and its checks chip reads "1 check waiting". After 72 hours (`wait_hours`) it stops and tells you once in Needs you: the check reads "Stopped waiting", and on the task you can press **Wait 24 h more** (`POST /api/checks/:id/wait-more`) or verify it by hand. Command checks get `KWANT_TASK_ID`, `KWANT_CHECK_ID` and `KWANT_IDEMPOTENCY_KEY`, which stays the same for every run of one review round. (#564, #575)

### Changed
- **Try again on a pre-push card lets that push go.** When a pre-push check stops a bot's push and you press **Ask Ben to try again**, Ben's next push of that branch goes without another approval card, once its checks pass (one push, within 2 hours). **Push anyway** lets its one push go the same way. The card then follows the push in one line, up to **Pushed · 14:32**, with a link to the new check. A production or payment approval still asks, in one amber line with **Open it**. Needs you shows one item per branch, the newest; an older card on a branch that was pushed again says it was replaced. (#571, #576)
- **A chat waiting on your answer says so.** When the only thing left in a chat is a bot's question, one amber line says "Waiting for you: Ben needs one answer to finish", with **Answer** to bring the question into view. The working card, the paused box and the red Stop hide; you can press a choice or just type your answer. The question card says how long ago it was asked, and its choices are easier to press. In the sidebar, teammates who already replied read "Done · replied here". (#567)
- **A routine's run shows as a routine.** A routine's message in a chat used to look like your own "You" message. It now shows as one row with a clock, "Routine · Morning brief", the time, and its instructions on one line; press it to read them in full. Messages you type still show as You. (#570)
- **An @mentioned teammate goes next.** In a team discussion, when the speaker @mentions a teammate ("@Cody please get the table sizes first"), Cody now speaks next, ahead of the planned order, shown as asked by the speaker. A teammate who was already planned moves up instead of speaking twice. The lead may @mention one teammate to hand them a task. (#573)

### Fixed
- On a fresh install with no AI account connected, the first message in #general no longer makes a Concierge ("Concierge changed something… Undo") that then can't answer. The reply is the setup card with **Add an account**, and the Concierge is made with your first message after an account is connected. An account that is only out of usage still counts. (#579)
- With approvals set to Don't ask, work a bot starts on its own (Loose ends, Watch, a task check, an audit, a goal draft) no longer stops for an approval card. Payments, production changes and Flight School limits still ask, and skill drafts always ask. Under Ask or Ask only when risky, these cards now say who started the turn ("Started by Loose ends, not by you, so each change asks first."), drop "Always this session", which never applied there, and Ask only when risky lets read-only shell commands through. (#569)
- When a mission task's browser stays busy through both retries, the task now ends once with a fix line in Needs you, "The browser stayed busy. Let the other job finish or close its page, then press Retry.", and AutoFix no longer runs the whole task again. A busy step that may reach outside, such as a send, gets its own line asking you to check whether it went through. Retry still works. (#563)
- Small fixes to bot budgets and model badges: a message to a paused bot shows only the pause bar, with no start, timer or Stop; the 80% line keeps the counts it was posted with; a finished budget item in the Inbox drops its meter; large counts read "2M" and "2.5M"; and a bot pinned to an AI that can't run here shows a grey dot when failover is off. (#582)
- **Add** on a tool from the public directory in Find a tool now fills in a name that works ("ElevenLabs Voices" becomes `elevenlabs-voices`), so **Add & test** no longer fails until you rename it. You can still change the name. (#584)
- In the live browser view, a page's own file picker, such as TikTok Studio's "Select video", opens a **Choose file…** sheet now instead of doing nothing. Pick a file, watch it upload, and the page gets it as if you had picked it there. It takes the same files as chat attachments: images, videos, PDFs and text files, up to 20 MB (videos 100 MB). (#578)

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
