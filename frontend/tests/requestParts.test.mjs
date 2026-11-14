// request-parts card (autonomy/request-parts/ui-brief.md): with written parts, the done-check box leads with one
// row per part of the person's request, criteria folded under each, and today's checks under "How it was checked".
// Data: impl.md "JSON shape" sample and four made-up objects in the same shape (src/doneCheckFixture.ts).
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as dc from './src/doneCheck';
  export * as cx from './src/doneCheckFixture';
  export { DoneCheck } from './src/components/DoneCheck';
`);
const { React, createRoot, dc, cx } = m;
const h = React.createElement;
const settle = async (n = 8) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const text = (e) => e.textContent.replace(/\s+/g, ' ').trim();
const clone = (x) => JSON.parse(JSON.stringify(x));

async function box(checked) {
  document.body.innerHTML = '';
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  root.render(h(m.DoneCheck, { checked: dc.knownChecked(checked) }));
  await settle();
  return { el, root, q: (x) => el.querySelector(x), qa: (x) => [...el.querySelectorAll(x)] };
}
const before = (a, b) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);

test('3 parts, one with no check: rows in order in the person\'s words, "No check" plainly, then How it was checked', async () => {
  const r = await box(cx.PARTS_NO_CHECK);
  assert.equal(text(r.q('[data-check-line]')), cx.PARTS_NO_CHECK.line, 'the line is the server\'s, untouched');
  assert.equal(r.q('[data-parts]').getAttribute('data-parts'), 'many');
  assert.equal(text(r.q('[data-parts-heading]')), 'Parts of your request');
  const rows = r.qa('[data-part]');
  assert.deepEqual(rows.map((x) => text(x.querySelector('[data-part-text]'))), ['Make the October sales PDF', 'put it in the shared folder', 'send it to Ana']);
  assert.deepEqual(rows.map((x) => x.getAttribute('data-part')), ['passed', 'passed', 'not_checked']);
  assert.deepEqual(rows.map((x) => text(x.querySelector('[data-part-result]'))), ['Passed', 'Passed', 'No check']);
  assert.match(rows[2].querySelector('[data-part-result]').className, /text-warn/, 'no check stands out in amber');
  assert.equal(rows[2].querySelector('[data-part-toggle]'), null, 'nothing to open on a part with no criteria');
  assert.equal(text(rows[2].querySelector('[data-part-no-criteria]')), '· Nothing checked this part.');
  const how = r.q('[data-how-heading]');
  assert.equal(text(how), 'How it was checked');
  assert.ok(before(r.q('[data-parts]'), how) && before(how, r.q('[data-check-toggle]')), 'parts, then the heading, then the checks');
  assert.equal(r.q('[data-check-list]').getAttribute('aria-labelledby'), how.id);
  assert.equal(r.qa('[data-check-list] [data-check-item]').length, 3, 'today\'s checks are all still listed');
  r.root.unmount();
});

test('criteria fold under each part: closed by default, a keyboard-reachable button opens them with their results', async () => {
  const r = await box(cx.PARTS_NO_CHECK);
  const row = r.qa('[data-part]')[0];
  const btn = row.querySelector('[data-part-toggle]');
  assert.equal(btn.tagName, 'BUTTON');
  assert.equal(btn.getAttribute('type'), 'button');
  assert.equal(btn.getAttribute('aria-expanded'), 'false');
  assert.equal(text(btn), 'Show what proves it (1)');
  const list = document.getElementById(btn.getAttribute('aria-controls'));
  assert.ok(list && list.hidden, 'controls a hidden list');
  // A display utility (flex, grid, block) on the list would beat the hidden attribute and show it while folded.
  for (const ul of r.qa('[data-part-criteria], [data-check-list], [data-parts-list]')) assert.doesNotMatch(ul.className, /(^|\s)(flex|grid|block|inline-flex)(\s|$)/, 'no display class on a foldable list');
  await click(btn);
  assert.equal(btn.getAttribute('aria-expanded'), 'true');
  assert.equal(text(btn), 'Hide what proves it');
  assert.equal(list.hidden, false);
  assert.equal(text(list), 'The PDF has the October totals · Passed');
  assert.ok(r.qa('[data-part]')[1].querySelector('[data-part-criteria]').hidden, 'the other part stays folded');
  r.root.unmount();
});

test('all parts passed: every row Passed, two criteria under the first part', async () => {
  const r = await box(cx.PARTS_ALL_PASSED);
  assert.equal(r.q('section').getAttribute('data-done-check'), 'checked');
  const rows = r.qa('[data-part]');
  assert.equal(rows.length, 2);
  assert.ok(rows.every((x) => text(x.querySelector('[data-part-result]')) === 'Passed'));
  assert.equal(text(rows[0].querySelector('[data-part-toggle]')), 'Show what proves it (2)');
  assert.equal(r.q('[data-part-no-criteria]'), null);
  r.root.unmount();
});

test('one part failed: "Not met" in the bad colour, its fold starts open; the checks list starts closed so the answer stays up', async () => {
  const r = await box(cx.PARTS_ONE_FAILED);
  assert.equal(r.q('section').getAttribute('data-done-check'), 'failed');
  const rows = r.qa('[data-part]');
  const row = rows[0];
  assert.equal(row.getAttribute('data-part'), 'failed');
  assert.equal(text(row.querySelector('[data-part-result]')), 'Not met');
  assert.match(row.querySelector('[data-part-result]').className, /text-bad/);
  const btn = row.querySelector('[data-part-toggle]');
  assert.equal(btn.getAttribute('aria-expanded'), 'true', 'a part that was not met opens its fold');
  assert.equal(text(btn), 'Hide what proves it');
  assert.equal(row.querySelector('[data-part-criteria]').hidden, false);
  assert.equal(row.querySelector('[data-criterion]').getAttribute('data-criterion'), 'failed');
  assert.match(text(row.querySelector('[data-part-criteria]')), /Not met$/);
  for (const other of rows.slice(1)) {
    const t = other.querySelector('[data-part-toggle]');
    if (t) assert.equal(t.getAttribute('aria-expanded'), 'false', 'parts that passed stay folded');
  }
  assert.equal(r.q('[data-check-list]').hidden, true, 'with parts shown, the checks list starts closed');
  assert.equal(r.q('[data-check-toggle]').getAttribute('aria-expanded'), 'false');
  await click(btn);
  assert.equal(row.querySelector('[data-part-criteria]').hidden, true, 'and it still folds');
  r.root.unmount();
});

test('the fold sits on the result line: "Passed · Show what proves it (n)", the dot hidden from screen readers', async () => {
  const r = await box(cx.PARTS_NO_CHECK);
  const row = r.qa('[data-part]')[0];
  const line = row.querySelector('[data-part-result]').parentElement;
  assert.ok(line.contains(row.querySelector('[data-part-toggle]')), 'result and toggle share one line');
  assert.deepEqual([...line.children].map(text), ['Passed', '·', 'Show what proves it (1)'], 'result, dot, toggle; the gap is CSS');
  assert.match(line.className, /\bflex-wrap\b/);
  assert.equal(line.querySelector('[aria-hidden="true"]').textContent, '·');
  assert.doesNotMatch(row.querySelector('[data-part-toggle]').className, /\bmt-1\b/);
  assert.match(r.q('[data-how-heading]').className, /\bmt-3\b/, 'the section gap is bigger than the gap between parts');
  r.root.unmount();
});

test('today\'s card is unchanged without parts: the failed card still opens its checks list', async () => {
  const x = clone(cx.PARTS_ONE_FAILED);
  delete x.parts_written;
  const r = await box(x);
  assert.equal(r.q('[data-parts]'), null);
  assert.equal(r.q('[data-check-list]').hidden, false);
  r.root.unmount();
});

test('one written part: exactly today\'s card, the server line names it and there is no counter', async () => {
  const strip = (html) => html.replace(/id="[^"]*"|aria-controls="[^"]*"|aria-labelledby="[^"]*"/g, '');
  const a = await box(cx.PARTS_ONE_WRITTEN);
  const html = strip(a.el.innerHTML);
  assert.equal(a.q('[data-parts]'), null);
  assert.equal(a.q('[data-how-heading]'), null);
  assert.match(text(a.q('[data-check-line]')), /Add the yearly discount to the FAQ/);
  assert.doesNotMatch(text(a.el), /\b1 of 1\b/);
  assert.doesNotMatch(text(a.el), /parts/i);
  a.root.unmount();
  const plain = clone(cx.PARTS_ONE_WRITTEN);
  for (const k of ['parts', 'parts_total', 'parts_checked', 'parts_written']) delete plain[k];
  const b = await box(plain);
  assert.equal(html, strip(b.el.innerHTML), 'same markup as a card without the parts fields');
  b.root.unmount();
});

test('parts_written false: exactly today\'s card, no parts rows and no "parts" words', async () => {
  const strip = (html) => html.replace(/id="[^"]*"|aria-controls="[^"]*"/g, '');
  const a = await box(cx.PARTS_NOT_WRITTEN);
  const html = strip(a.el.innerHTML);
  assert.equal(a.q('[data-parts]'), null);
  assert.equal(a.q('[data-how-heading]'), null);
  assert.doesNotMatch(text(a.el), /parts/i);
  a.root.unmount();
  const b = await box(cx.CHECKED_PARTLY);
  assert.equal(html, strip(b.el.innerHTML), 'same markup as a card without the parts fields');
  b.root.unmount();
});

test('older server or odd data: missing parts_written, too few parts, malformed parts and unknown results', async () => {
  const noFlag = clone(cx.PARTS_NO_CHECK);
  delete noFlag.parts_written;
  assert.equal(dc.partsLayout(dc.knownChecked(noFlag)), null, 'missing parts_written: today\'s card');
  const empty = { ...clone(cx.PARTS_NO_CHECK), parts: [] };
  assert.equal(dc.partsLayout(dc.knownChecked(empty)), null, 'parts_total 3 but no parts: today\'s card');
  assert.equal(dc.partsLayout(dc.knownChecked({ ...clone(cx.PARTS_NO_CHECK), parts: 'x' })), null);
  const odd = clone(cx.PARTS_NO_CHECK);
  odd.parts = [null, { text: '' }, { text: 'Make it', result: 'maybe', criteria: [{ id: 'c1', text: 'It is made', result: 'weird' }, 7, { id: 'c2' }] }, { text: 'Ship it', result: 'passed' }];
  const k = dc.knownChecked(odd);
  assert.equal(k.parts.length, 2, 'only parts with words are kept');
  assert.equal(k.parts[0].criteria.length, 1, 'only criteria with words are kept');
  assert.deepEqual(k.parts[1].criteria, [], 'missing criteria read as none');
  const r = await box(odd);
  assert.deepEqual(r.qa('[data-part-result]').map(text), ['No check', 'Passed'], 'an unknown result is never Passed');
  await click(r.q('[data-part-toggle]'));
  assert.match(text(r.q('[data-criterion]')), /No check$/);
  r.root.unmount();
});

test('server text is text only: a part with markup is drawn as its characters', async () => {
  const x = clone(cx.PARTS_NO_CHECK);
  x.parts[2].text = '<img src=x onerror=alert(1)> send it to Ana';
  const r = await box(x);
  assert.equal(r.q('[data-part] img'), null);
  assert.equal(text(r.qa('[data-part-text]')[2]), '<img src=x onerror=alert(1)> send it to Ana');
  r.root.unmount();
});
