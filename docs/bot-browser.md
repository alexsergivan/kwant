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

## Tests

`tests/browser_refs_test.rs` (real Chrome, skipped without one; `saucedemo_by_refs` is ignored and
needs the network) and `ref_tests` in `src/sandbox/browser.rs`.
