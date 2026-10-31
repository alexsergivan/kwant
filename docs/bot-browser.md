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

## Tests

`tests/browser_refs_test.rs` and `tests/browser_progress_test.rs` (real Chrome, skipped without one;
the `saucedemo_*` tests are ignored and need the network), `ref_tests` in `src/sandbox/browser.rs`
and the tests in `src/sandbox/progress.rs`.
