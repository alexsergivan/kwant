// 0.9.8 done-check (autonomy/done-check/design.md section 5): the "Done, and here's the check" box in both done
// layouts and the one-word marker on done rows. Data: Vega's #566 `checked` object (impl.md, real sample).
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
g.fetch = async () => ({ ok: false, status: 404, json: async () => ({}) });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as dc from './src/doneCheck';
  export * as cx from './src/doneCheckFixture';
  export * as fx from './src/doneAwayFixture';
  export * as store from './src/inboxStore';
  export { InboxNew } from './src/components/InboxNew';
  export { DoneItemView } from './src/components/DoneDetail';
  export { DoneCheck, CheckMarker } from './src/components/DoneCheck';
`);
const { React, createRoot, dc, cx, fx, store: s } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const clone = (x) => JSON.parse(JSON.stringify(x));
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const text = (e) => e.textContent.replace(/\s+/g, ' ').trim();

async function mount(node) {
  document.body.innerHTML = '';
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  root.render(node);
  await settle();
  return { el, root, q: (x) => el.querySelector(x), qa: (x) => [...el.querySelectorAll(x)] };
}

const box = (checked, props = {}) => mount(h(m.DoneCheck, { checked: dc.knownChecked(checked), ...props }));

async function mountInbox(props = {}, data = clone(fx.AWAY_INBOX)) {
  document.body.innerHTML = '';
  sessionStorage.clear();
  window.history.replaceState(null, '', '/inbox');
  s.inboxStore.reset();
  s.inboxSource.get = async () => data;
  s.inboxStore.patch({ data });
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: true, now: Date.parse(data.now), placeName: fx.awayPlaceName, ...props }));
  await settle();
  return { el, root, q: (x) => el.querySelector(x), qa: (x) => [...el.querySelectorAll(x)] };
}

const withChecked = (id, checked) => {
  const data = clone(fx.AWAY_INBOX);
  data.items.find((i) => i.id === id).checked = clone(checked);
  return data;
};

test('each state draws its box classes, its icon and its prefix colour; failed starts open, the rest folded', async () => {
  const want = {
    checked: { box: 'border-teal-300/25', icon: 'lucide-shield-check', prefix: 'text-ok', open: false, sample: cx.CHECKED_OK, word: 'Checked:' },
    partly: { box: 'border-amber-300/30', icon: 'lucide-shield-half', prefix: 'text-warn', open: false, sample: cx.CHECKED_PARTLY, word: 'Partly checked:' },
    failed: { box: 'border-rose-300/35', icon: 'lucide-shield-x', prefix: 'text-bad', open: true, sample: cx.CHECKED_FAILED, word: 'Failed:' },
    not_checked: { box: 'border-dashed', icon: 'lucide-shield', prefix: 'text-zinc-200', open: false, sample: cx.CHECKED_NONE, word: 'Not checked:' },
  };
  for (const [state, w] of Object.entries(want)) {
    const { q, root } = await box(w.sample);
    const sec = q('section');
    assert.equal(sec.getAttribute('aria-label'), 'How it was checked');
    assert.equal(sec.getAttribute('data-done-check'), state);
    assert.match(sec.className, new RegExp(w.box.replace(/[/[\].]/g, '\\$&')), `${state} box`);
    const icon = q('[data-check-line] svg');
    assert.ok(icon.classList.contains(w.icon), `${state} icon ${icon.getAttribute('class')}`);
    assert.equal(icon.getAttribute('aria-hidden'), 'true');
    assert.equal(q('[data-check-prefix]').textContent, w.word);
    assert.ok(q('[data-check-prefix]').classList.contains(w.prefix), `${state} prefix colour`);
    assert.equal(text(q('[data-check-line]')), w.sample.line, 'the server line as sent');
    const t = q('[data-check-toggle]');
    if (w.sample.items.length) {
      assert.equal(t.getAttribute('aria-expanded'), String(w.open));
      assert.equal(t.getAttribute('aria-controls'), q('[data-check-list]').id);
      assert.equal(q('[data-check-list]').hidden, !w.open);
    } else assert.equal(t, null);
    root.unmount();
  }
});

test('the line: unknown prefix drawn whole, blank line uses the fallback words', async () => {
  assert.deepEqual(dc.splitLine('partly', 'Partly checked: 1 of 2 checks passed.'), { prefix: 'Partly checked:', rest: '1 of 2 checks passed.' });
  assert.deepEqual(dc.splitLine('checked', 'All good here.'), { prefix: null, rest: 'All good here.' });
  assert.equal(dc.splitLine('checked', '').prefix + ' ' + dc.splitLine('checked', '').rest, 'Checked: every check passed.');
  assert.equal(dc.FALLBACK_LINE.partly, 'Partly checked: some checks passed.');
  assert.equal(dc.FALLBACK_LINE.failed, 'Failed: a check failed.');
  assert.equal(dc.FALLBACK_LINE.not_checked, 'Not checked: nothing verified this result.');
  const { q, root } = await box({ ...cx.CHECKED_FAILED, line: '  ' });
  assert.equal(text(q('[data-check-line]')), 'Failed: a check failed.');
  root.unmount();
});

test('toggle: Show what was checked (n) / Hide, chevron turns, the open list has each check in server order', async () => {
  const { q, qa, root } = await box(cx.CHECKED_PARTLY);
  const t = q('[data-check-toggle]');
  assert.equal(text(t), 'Show what was checked (4)');
  assert.ok(t.hasAttribute('data-compact'));
  assert.match(t.className, /kw-focus/);
  assert.match(t.className, /kw-hit/);
  assert.match(t.querySelector('svg').getAttribute('class'), /motion-reduce:transition-none/);
  await click(t);
  assert.equal(text(t), 'Hide what was checked');
  assert.equal(t.getAttribute('aria-expanded'), 'true');
  assert.match(t.querySelector('svg').getAttribute('class'), /rotate-180/);
  assert.match(q('[data-check-list]').className, /kw-fade-in/);
  assert.deepEqual(qa('[data-check-what]').map((e) => e.textContent), cx.CHECKED_PARTLY.items.map((i) => i.what));
  const rows = qa('[data-check-item]').map((li) => [...li.querySelectorAll('p')].map(text).join(' | '));
  assert.equal(rows[0], 'https://kwant.si/pricing | Passed · Opened the address | HTTP 200 in 212 ms');
  assert.equal(rows[1], 'npm run check-prices | Not checked · Ran the command');
  assert.equal(q('[data-check-list] a'), null, 'server text is text, never a link');
  await click(t);
  assert.equal(q('[data-check-list]').hidden, true);
  root.unmount();
});

test('a claim gets the amber triangle and "Not checked"; an unknown result is "Not checked", never "Passed"', async () => {
  const odd = { ...cx.CHECKED_PARTLY, items: [...cx.CHECKED_PARTLY.items, { what: 'Mystery', how: 'ran the command', result: 'exploded', evidence: null }] };
  const { q, qa, root } = await box(odd);
  await click(q('[data-check-toggle]'));
  const claim = qa('[data-check-item]')[3];
  assert.equal(claim.getAttribute('data-check-item'), 'claim');
  assert.ok(claim.querySelector('svg').classList.contains('lucide-triangle-alert') || claim.querySelector('svg').classList.contains('lucide-alert-triangle'));
  assert.ok(claim.querySelector('svg').classList.contains('text-warn'));
  assert.equal(claim.querySelector('[data-check-result]').textContent, 'Not checked');
  assert.ok(claim.querySelector('[data-check-result]').classList.contains('text-warn'));
  assert.match(text(claim), /Task criterion, marked met with no evidence \(a claim\)/);
  assert.equal(claim.querySelector('[data-sticker]'), null, 'a claim is never holographic');
  const unknown = qa('[data-check-item]')[4];
  assert.equal(unknown.getAttribute('data-check-item'), 'not_checked');
  assert.equal(unknown.querySelector('[data-check-result]').textContent, 'Not checked');
  assert.equal(dc.itemKind({ result: undefined, how: '' }), 'not_checked');
  assert.equal(dc.ITEM_WORD.waiting, 'Still waiting');
  assert.equal(dc.howText('kwant compared the reply with what you asked for'), 'kwant compared the reply with what you asked for');
  assert.equal(dc.howText('opened the address'), 'Opened the address');
  root.unmount();
});

test('items_dropped: 2 gives "2 more not shown." and the toggle count includes it; failed evidence keeps its line breaks', async () => {
  const { q, qa, root } = await box(cx.CHECKED_FAILED);
  assert.equal(text(q('[data-check-toggle]')), 'Hide what was checked', 'failed starts open');
  assert.equal(q('[data-check-dropped]').textContent, '2 more not shown.');
  assert.equal(dc.droppedLine(1), '1 more not shown.');
  const ev = q('[data-check-evidence]');
  assert.ok(ev.textContent.includes('\n'));
  assert.match(ev.className, /whitespace-pre-wrap/);
  assert.equal(qa('[data-check-item="waiting"] [data-check-result]')[0].textContent, 'Still waiting');
  await click(q('[data-check-toggle]'));
  assert.equal(text(q('[data-check-toggle]')), 'Show what was checked (6)');
  root.unmount();
});

test('no items: the help line and no toggle; "Add a check" only for a routine with onEditRoutine', async () => {
  let r = await box(cx.CHECKED_NONE);
  assert.equal(r.q('[data-check-toggle]'), null);
  assert.equal(r.q('[data-check-help]').textContent, 'Read the answer before you rely on it.');
  assert.equal(r.q('[data-add-check]'), null);
  r.root.unmount();
  const edits = [];
  r = await box(cx.CHECKED_NONE, { onAddCheck: () => edits.push('x') });
  assert.equal(text(r.q('[data-check-help]')), 'Read the answer before you rely on it. Add a check');
  await click(r.q('[data-add-check]'));
  assert.deepEqual(edits, ['x']);
  r.root.unmount();
  // In the pane: the routine review with onEditRoutine gets Add a check; a hand-off with no routine does not.
  const data = withChecked('r-prices', cx.CHECKED_NONE);
  const got = [];
  r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL }, onEditRoutine: (id) => got.push(id) }, data);
  await click(r.q('[data-option="r-prices"]'));
  assert.ok(r.q('[data-done-check="not_checked"]'));
  assert.equal(r.q('[data-chip="no-checks"]'), null, 'the box replaces the No checks set chip');
  await click(r.q('[data-done-check] [data-add-check]'));
  assert.deepEqual(got, ['rt-prices']);
  r.root.unmount();
  r = await mount(h(m.DoneItemView, { item: { ...fx.AWAY_HANDOFF, checked: cx.CHECKED_NONE }, d: { loading: false, error: null, routine: null, run: null, message: null, target: null }, place: null, onEditRoutine: () => {} }));
  assert.ok(r.q('[data-done-check="not_checked"]'));
  assert.equal(r.q('[data-add-check]'), null, 'no routine: no Add a check');
  r.root.unmount();
});

test('pane order: chips, then the check box, then the answer; criteria fold hidden when the box lists task criteria', async () => {
  const data = withChecked('r-prices', cx.CHECKED_PARTLY);
  const it = data.items.find((i) => i.id === 'r-prices');
  it.criteria = [{ index: 0, text: 'The three plan prices match the plan sheet', status: 'met', evidence: [], note: null }];
  it.criteria_summary = { met: 1, total: 1, not_met: 0, not_checked: 0 };
  const r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } }, data);
  await click(r.q('[data-option="r-prices"]'));
  const chips = r.q('[data-done-chips]');
  const boxEl = r.q('[data-done-check]');
  const ans = r.q('[data-done-answer]');
  assert.ok(chips && boxEl && ans);
  assert.ok(chips.compareDocumentPosition(boxEl) & Node.DOCUMENT_POSITION_FOLLOWING);
  assert.ok(boxEl.compareDocumentPosition(ans) & Node.DOCUMENT_POSITION_FOLLOWING);
  assert.equal(r.el.textContent.includes('What done looks like'), false, 'criteria are in the box');
  r.root.unmount();
  // Without task criteria in the box, the fold stays.
  const keep = withChecked('r-prices', cx.CHECKED_OK);
  Object.assign(keep.items.find((i) => i.id === 'r-prices'), { criteria: it.criteria, criteria_summary: it.criteria_summary });
  const r2 = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } }, keep);
  await click(r2.q('[data-option="r-prices"]'));
  assert.equal(r2.el.textContent.includes('What done looks like'), true);
  r2.root.unmount();
});

test('no checked object (older server) or an unknown state: no box, the No checks set chip as before', async () => {
  assert.equal(dc.knownChecked(undefined), null);
  assert.equal(dc.knownChecked({ state: 'mostly', items: [] }), null);
  let r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL }, onEditRoutine: () => {} });
  await click(r.q('[data-option="r-prices"]'));
  assert.equal(r.q('[data-done-check]'), null);
  assert.match(r.q('[data-chip="no-checks"]').textContent, /No checks set · Add one/);
  assert.equal(r.q('[data-row="r-prices"] [data-check-marker]'), null);
  assert.equal(r.q('[data-row="r-prices"] [data-sticker]'), null);
  r.root.unmount();
  r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } }, withChecked('r-prices', { state: 'mostly', items: [], items_dropped: 0, line: 'x' }));
  await click(r.q('[data-option="r-prices"]'));
  assert.equal(r.q('[data-done-check]'), null);
  r.root.unmount();
});

test('list row: the marker comes before the summary; the holo sticker only for checked', async () => {
  const cases = [
    ['checked', cx.CHECKED_OK, 'Checked No change since last week. Beanline still cheapest.'],
    ['partly', cx.CHECKED_PARTLY, 'Partly checked · No change since last week. Beanline still cheapest.'],
    ['failed', cx.CHECKED_FAILED, 'Check failed · No change since last week. Beanline still cheapest.'],
    ['not_checked', cx.CHECKED_NONE, 'Not checked · No change since last week. Beanline still cheapest.'],
  ];
  for (const [state, c, want] of cases) {
    const r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } }, withChecked('r-prices', c));
    const sum = r.q('[data-row="r-prices"] [data-done-summary]');
    assert.equal(text(sum), want, state);
    const holo = sum.querySelector('[data-sticker="holo"]');
    if (state === 'checked') {
      assert.ok(holo, 'checked is the holo sticker');
      assert.equal(sum.firstElementChild, holo, 'the marker comes first');
    } else {
      assert.equal(holo, null, `${state} is never holographic`);
      assert.equal(sum.firstElementChild.getAttribute('data-check-marker'), state);
      assert.match(sum.firstElementChild.className, { partly: /text-warn/, failed: /text-bad/, not_checked: /text-zinc-300/ }[state]);
    }
    assert.equal(r.qa('[data-sticker]').length, state === 'checked' ? 1 : 0, 'the only sticker on the list');
    r.root.unmount();
  }
  // A done (non-review) row and a team job get it too; a row with no summary shows the marker alone.
  const data = withChecked('tj-spring', cx.CHECKED_FAILED);
  data.items.find((i) => i.id === 'tj-spring').summary = '';
  const r = await mountInbox({}, data);
  assert.equal(text(r.q('[data-row="tj-spring"] [data-done-summary]')), 'Check failed');
  r.root.unmount();
});

test('390 px: a 120-character URL as what wraps anywhere instead of pushing the box sideways', async () => {
  assert.equal(cx.LONG_URL.length, 120);
  const { q, root } = await box({ ...cx.CHECKED_FAILED, items: [{ what: cx.LONG_URL, how: 'opened the address', result: 'failed', evidence: `HTTP 404 at ${cx.LONG_URL}` }] });
  for (const sel of ['[data-check-what]', '[data-check-evidence]']) assert.match(q(sel).className, /\[overflow-wrap:anywhere\]/, sel);
  assert.match(q('section').className, /min-w-0/);
  assert.match(q('[data-check-item] > div').className, /min-w-0/);
  assert.match(q('[data-check-line]').className, /minmax\(0,1fr\)/);
  root.unmount();
});

test('a plain to-review task in Done keeps the full review layout and gets the box above the verdict', async () => {
  const data = withChecked('r-prices', cx.CHECKED_PARTLY);
  const it = data.items.find((i) => i.id === 'r-prices');
  it.title = 'Refresh the pricing page';
  it.verdict = { ...it.verdict, result: 'pass' };
  const r = await mountInbox({}, data);
  await click(r.q('[data-option="r-prices"]'));
  const boxEl = r.q('[data-done-check="partly"]');
  assert.ok(boxEl, 'the box is drawn');
  assert.equal(r.q('[data-done-answer]'), null, 'not the done layout');
  assert.equal(text(r.q('[data-row="r-prices"] [data-done-summary]')), `Partly checked · ${it.summary}`, 'a plain review row: marker then its summary');
  const verdict = r.q('[data-verdict]');
  if (verdict) assert.ok(boxEl.compareDocumentPosition(verdict) & Node.DOCUMENT_POSITION_FOLLOWING, 'box before the verdict');
  r.root.unmount();
});
