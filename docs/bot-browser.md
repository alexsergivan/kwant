# How bots use their browser

Bots drive the browser in their side panel through `browser_open`, `browser_page` and `browser_act`
(kwnt's own tools) or the shell API (`/api/sandbox/:id/{navigate,page,act}`). Both return the same
compact page state: `url`, `title`, `text`, `clickable`. Payments are covered in
[payments.md](payments.md); the person's live view uses `/interact`, which bots can't call.

## Clicking by ref

`BrowserController::list_clickables` (every page state the bot gets: `/page`, `/act`, `browser_*`)
numbers the controls it lists, topmost first: `[ref] role "name"`, plus ` · context` when several
controls share a name (the first line of the largest block around it holding no other control of
that name, e.g. the product card). Listing keeps the elements in the page
(`window[Symbol.for('kwnt.refs')]`), so the refs a bot was last shown are the ones that resolve.

`browser_act` / `/act` take `{"action": "click", "ref": 9}` (also `type` with a ref, which clicks that
field first). A whole copied line as `text` (`[9] button "Add to cart" · …`) counts as that ref; a bare
`[1]` stays a label (footnote links). Before clicking, `REF_JS` checks the element is still in the
page, shown, still has the same label, and is the topmost element at its center after scrolling it
into view (and the frame it sits in is topmost too). Then kwant clicks there with real mouse events.
Otherwise nothing is clicked and the error says why (`gone`, `hidden`, `now reads "…"`, `covered by div
"Cookie banner"`, `no list of refs yet` after a reload); the page state returned with it is a fresh
list. The payment floor judges the element the ref points to.

### New controls

A control that wasn't in the page's previous list starts with `*`: `*[12] option "Large"`. Identity
is the element itself (the kept refs), not its label, so a renamed button isn't new and a second
"Add to cart" that just appeared is. The first list of a page marks nothing, nor does the first list
after a reload, a navigation, or an address change other than its `#fragment` (the registry keeps
the address it was made for). Every list compares against the one before it, whoever asked for it,
so a read right after an action's answer marks nothing. With `find`, new controls come first among
equally matching ones; without it, page order stays. The mark doesn't count towards `hash`, so a
re-read of an unchanged page is still `unchanged`. A copied `*[12] …` line works as a ref.
(Idea from browser-use's `_assign_interactive_indices_and_mark_new_nodes`.)

Limits: the page can read or change the list like anything else on it; another page read between
the payment check and the click renumbers it (one bot drives one browser, so rare).

## What changed, and the loop stop

Each `/act` (and `browser_act`) reads the page before and after the action
(`BrowserController::snapshot`: url, title, the rendered text in lines, scroll position, field fill
lengths and focus; field values never leave the page) and returns `changed`, one short line from
`progress::what_changed`: `Went to <url> ("<title>").`, or `Title is now "…". New: "…", "…" (+n
more). Gone: "…".`, or `Nothing on the page changed.` A page that hasn't changed yet is read once
more after 400 ms (a slow submit). A click always focuses what it hits, so for clicks a moved focus
alone is no change; for keys (Tab) it is.

`progress::Tracker` (one per browser, reset when an address is opened):
- 3 actions in a row that change nothing: the third returns `stalled: true` and `error` set to
  `STALL_STOP` (try something else, or tell the person). The count then starts over.
- The same click (same ref or label) or the same typing into the same place a 3rd time in a row on
  the same address, when the two before it changed nothing, is refused before it runs
  (`REPEAT_STOP`), until the bot does something else. A repeat that changes the page (a counter,
  "Load more") resets the count, so it can run any number of times. Scrolls and keys don't count
  and don't break a run.

The page text in page states is now `document.body.innerText` (laid out in lines, hidden parts left
out) instead of the text of a detached copy, which ran lines together.

## Cheaper reads

Every page state carries `hash` (`reads::page_hash`, FNV-1a over url, title, the text shown, every
control line and the `find` words). A read of a page that still reads the same as the reader's last
look returns only `{url, title, hash, blocked, unchanged: true, note}`:
- HTTP: `GET /page?since=<hash>`, with the hash of the bot's last page state (from `/page`,
  `/navigate` or `/act`). Without `since` the page is always sent whole.
- `browser_page`: the turn is the reader (`SelfScope::turn_id`). Every browser tool's answer counts
  as a look (`BrowserController::reads`, the last 32 readers), so a read right after an action that
  changed nothing is "unchanged". `full: true` sends it whole anyway; a new turn, or one without an
  id, always gets it whole.

Pages list at most 40 controls (`reads::pick_controls`). Without `find` they are the first 40 in
page order (topmost first). With `find` (`browser_page`, or `/page?find=`), the controls naming the
most of its words (3+ letters, a few filler words left out; plain word matching, no model call)
come first, ties in page order. Ref numbers are the page's own, so a control past the 40th still
clicks by its ref. `controls_note` says what was left out. Step answers (`/act`, `browser_act`)
list the first 40 in page order.

## Citing what a bot read

A bot backs facts it read on the web with links that open the page at the words it quotes
(`#:~:text=` text fragments). It never writes the link itself: it passes the words, and kwant checks
them against the page open now (`sandbox::cite`, plain text matching, no model call).
- MCP: `browser_cite` with `text` (the exact words) or `block` (an id from a read with blocks). It
  only reads, so it is free in always-ask turns (`READ_ONLY_SELF_TOOLS`) and is a core tool.
- HTTP: `POST /api/sandbox/:id/cite` with `{"text": …}` or `{"block": …}`.

The answer is `{quote, url, link}`; `quote` is the page's own text and `url` the address it is on.
The words must be on the page word for word: runs of spaces and line breaks count as one space,
capitals count. They must be there exactly once, in whole words, in the page's rendered text
(`innerText`, up to 200,000 characters) or in a same-origin frame's, which is then the `url`.
Otherwise the call is refused with the reason (HTTP 422, a tool error): not on the page (with the
page's spelling when only the capitals differ), on it N times (quote more), starts or ends inside a
word. Nothing is quoted from the bot's own text.

The link: `text=start` for a quote of up to 300 characters on one line, else `text=start,end` with
its first and last five words. Every part is percent-encoded except letters, digits and `._~`, so
`-`, `,`, `&`, `%` and spaces in the words can't break the fragment. Browsers highlight the first
match without case or accents (`résumé` is `resume`, `ß` is `ss`, `ς` is `σ`); `cite::resolve`
mirrors that on the page folded the same way (`cite::fold`: lowercase, compatibility decomposition
with the accents dropped, plus `ß`, `ς`, `æ`, `œ`, `ø`, `ł`, `đ` and a few more), more leniently: no
word or block boundaries, so it sees every lookalike Chrome would and maybe more. When its first match isn't the quote, up to three words before and after it on the same line are
added as `prefix-,` and `,-suffix`. If that still doesn't single it out, the call is refused rather
than returning a link that highlights something else. Two identical-looking passages with the
same words around them can't be linked apart, so the later one is refused. A call with both `text`
and `block` is refused too. An address with its own fragment
(`#/route`) keeps it: `#/route:~:text=…`.

Blocks: `browser_page` with `blocks: true` (HTTP `/page?blocks=1`) sends the page's lines as
`[b3-9f2a41c0] text` (first 150, each cut at 300 characters) instead of `text`. The id is the line's
number and a 32-bit hash of its text, so an id from an older read never cites a different block: if lines
moved it finds the block by its hash, and if it's gone the call is refused ("read it again"). A
read with blocks has its own `hash`, so "unchanged" (`since`, per-turn reads) only answers for the
same kind of read.

Limits: frames from other sites can't be read from the page, so their text can't be cited; a
browser's own matching follows the page's blocks, which `innerText` lines only approximate, so a
link can occasionally highlight nothing (it then opens at the top); pages that change text on load
or per visit can make a link miss later.

## Sites a bot may open

Each bot has three optional settings, set by the person on `PUT /api/kwants/:id` and shown on the
bot's read routes:

- `browser_allowed_sites`: when not empty, the bot's browser opens only these sites.
- `browser_blocked_sites`: never these. The block-list wins over the allow-list.
- `browser_block_private_network` (off by default): no loopback, private, link-local or other
  internal addresses, also by a hostname that resolves to one. It uses the rules in
  `src/mcp/fetch_risk.rs`.

An entry is a host: `example.com` covers the site and all its subdomains, `*.example.com` only the
subdomains, and an IP matches only itself. Entries are lowercased, a pasted URL keeps only its host,
and international names are stored as punycode. A bad entry (empty, `*`, a wildcard inside a name
or on an IP) refuses the whole update with a 400. There are at most 200 entries per list. A bot's
own token can't change these settings (403); only the person can.

With both lists empty and the setting off, nothing is checked and nothing extra runs, exactly as
before. Otherwise every navigation the bot causes is checked: the address it opens (refused before
loading, 403 with `site_rule`), and through Chrome request interception (`Fetch`, top-level
documents) the link it clicks, any redirect and any new tab the page opens. A refused page is
stopped in the browser (`ERR_BLOCKED_BY_CLIENT`). The bot's action reply carries `error` with a
message that names the rule, and a `site_rule` with `rule`, `host`, `url`, `entry` and `message`.
Whatever the person does in live control (`/interact`, the stream, take control) is never checked.

Limits: only top-level pages are checked. Frames, images, scripts and `fetch`/XHR from an allowed
page are not, so this sets which sites a bot visits; it is not a network firewall. The
private-network check resolves the name once, so DNS rebinding between that check and Chrome's own
lookup isn't caught. In VM mode the name is resolved on the kwnt host, not inside the VM. A
navigation started by the page on its own (a timer) after the person's last input counts as theirs
until the bot acts again. In a browser several bots share, the rules of the bot that drove it last
apply.

## Tests

`tests/browser_refs_test.rs`, `tests/browser_progress_test.rs` and `tests/browser_reads_test.rs` `tests/browser_cite_test.rs` and `tests/browser_site_rules_test.rs` (real Chrome, skipped without one;
the `saucedemo_*` and `a_public_page_*` tests are ignored and need the network), `ref_tests` in `src/sandbox/browser.rs`
and the tests in `src/sandbox/progress.rs`, `src/sandbox/reads.rs`, `src/sandbox/cite.rs` and `src/sandbox/site_guard.rs`.
