// 0.8.1 F3 (Iris): the pre-push pointer lines in team chats and channels (Mira,
// release-090/designs/f3-prepush-in-team.md): which lines show, their order and cap, each phase's
// copy, the screen-reader name, the 10 min leave, and that a line only opens the bot's 1:1.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'CustomEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export * from './src/dev/prepush';
  export * as fx from './src/dev/prepushFixtures';
  export * from './src/components/dev/PrepushPointers';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const { fx } = m;
const h = m.createElement;
const MIN = 60_000;
const card = (raw) => {
  const c = m.parsePrepush(raw);
  assert.ok(c, 'fixture parses');
  return c;
};
/** A card for bot `k`, created `ago` min before T0+60 and updated `upd` min before it. */
const NOW = fx.PP_T0 + 60 * MIN;
const at = (raw, k, id, createdAgo, updatedAgo) => card({ ...raw, id, kwant_id: k, created_at: NOW - createdAgo * MIN, updated_at: NOW - updatedAgo * MIN });

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(m.PrepushPointers, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return { host, rerender: async (p) => { root.render(h(m.PrepushPointers, p)); await settle(); } };
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const names = { k_ann: 'Ann', k_bo: 'Bo', k_cy: 'Cy', k_di: 'Di', k_out: 'Out' };

test('one line per member bot: its newest card by updated_at, created in the last 24 h', () => {
  const cards = [
    at(fx.PP_CHECKING, 'k_ann', 'a_old', 50, 40),
    at(fx.PP_BLOCKED, 'k_ann', 'a_new', 30, 5),
    at(fx.PP_CHECKING, 'k_bo', 'b_stale', 25 * 60, 1), // created 25 h ago
    at(fx.PP_CHECKING, 'k_out', 'o_1', 5, 1), // not a member
  ];
  const pick = m.pointerLines(cards, ['k_ann', 'k_bo'], NOW);
  assert.deepEqual(pick.lines.map((c) => c.id), ['a_new']);
  assert.equal(pick.more, 0);
});

test('Needs-you lines first, then newest first; at most 3 with the rest counted', () => {
  const cards = [
    at(fx.PP_CHECKING, 'k_ann', 'a', 10, 1),
    at(fx.PP_BLOCKED, 'k_bo', 'b', 10, 2),
    at(fx.PP_NEEDS_YOU, 'k_cy', 'c', 30, 20),
    at(fx.PP_RETRIED, 'k_di', 'd', 10, 3),
    at(fx.PP_FAILED_NEEDS_YOU, 'k_out', 'e', 30, 25),
  ];
  const all = m.pointerLines(cards, ['k_ann', 'k_bo', 'k_cy', 'k_di', 'k_out'], NOW);
  assert.deepEqual(all.lines.map((c) => c.id), ['c', 'e', 'a']);
  assert.equal(all.more, 2);
  const two = m.pointerLines(cards, ['k_ann', 'k_cy'], NOW);
  assert.deepEqual(two.lines.map((c) => c.id), ['c', 'a'], 'two bots in one team: two lines, P4 first');
});

test('P2, P6 and P0 leave 10 min after their update; other phases stay', () => {
  const settled = [at(fx.PP_CLEAR, 'k_ann', 'clear', 30, 11), at(fx.PP_OVERRIDDEN, 'k_bo', 'over', 30, 10), at(fx.PP_NO_CHANGE, 'k_cy', 'none', 30, 12)];
  assert.deepEqual(m.pointerLines(settled, ['k_ann', 'k_bo', 'k_cy'], NOW).lines, []);
  const fresh = [at(fx.PP_CLEAR, 'k_ann', 'clear', 30, 9), at(fx.PP_BLOCKED, 'k_bo', 'blk', 300, 200)];
  const pick = m.pointerLines(fresh, ['k_ann', 'k_bo'], NOW);
  assert.deepEqual(pick.lines.map((c) => c.id).sort(), ['blk', 'clear']);
  assert.equal(pick.nextExpiry, NOW + MIN, 'the clear line leaves in 1 min');
  // A settled newest card doesn't bring back the bot's older card.
  const older = [at(fx.PP_BLOCKED, 'k_ann', 'older', 60, 50), at(fx.PP_CLEAR, 'k_ann', 'newer', 30, 20)];
  assert.deepEqual(m.pointerLines(older, ['k_ann'], NOW).lines, []);
});

test("each phase's copy matches Mira's table", () => {
  const T = (raw, extra = {}) => m.pointerText(card({ ...raw, ...extra, branch: 'qa-skip' }), 'Ann');
  assert.equal(T(fx.PP_CHECKING), 'Ann is checking before pushing qa-skip');
  assert.equal(T(fx.PP_RETRIED), "Ann's push of qa-skip is being checked again");
  assert.equal(T(fx.PP_BLOCKED), 'Ann is fixing 1 finding before pushing qa-skip · Try 1 of 2');
  assert.equal(T(fx.PP_BLOCKED, { blockers: [] }), 'Ann is fixing what blocked pushing qa-skip · Try 1 of 2');
  assert.equal(T(fx.PP_FAILED), "A check couldn't run on Ann's push of qa-skip");
  assert.equal(T(fx.PP_NEEDS_YOU), "Ann's push of qa-skip is waiting for you: 1 finding open");
  assert.equal(T(fx.PP_FAILED_NEEDS_YOU), "Ann's push of qa-skip is waiting for you: the checks couldn't run");
  assert.equal(T(fx.PP_CLEAR), 'Cleared: Ann can push qa-skip · 5 checks passed');
  assert.equal(T(fx.PP_OVERRIDDEN), "You let Ann's push of qa-skip go anyway");
  assert.equal(T(fx.PP_NO_CHANGE), 'Nothing to push: qa-skip has no new commits');
  assert.equal(T(fx.PP_OVERRIDDEN, { status: 'pushed' }), "You let Ann's push of qa-skip go anyway", '0.9.7: pushed is known now');
  assert.equal(T(fx.PP_CHECKING, { status: 'merged_later' }), "Ann has a push check this version can't show · update the app to see where it is");
  const two = card({ ...fx.PP_BLOCKED, branch: 'qa-skip', blockers: [fx.PP_BLOCKED.blockers[0], { ...fx.PP_BLOCKED.blockers[0], id: 'f2' }] });
  assert.equal(m.openCount(two), 2);
  assert.equal(m.pointerText(two, 'Ann'), 'Ann is fixing 2 findings before pushing qa-skip · Try 1 of 2');
});

test('screen-reader name: the line, then where it goes', () => {
  const c = card({ ...fx.PP_NEEDS_YOU, branch: 'qa-skip' });
  assert.equal(m.pointerLabel(c, 'Ann'), "Ann's push of qa-skip is waiting for you: 1 finding open. Open Ann's chat");
});

test('the stack: labelled, P4 amber with the chip, branch as code, no card buttons, never HTML from data', async () => {
  const evil = { ...fx.PP_NEEDS_YOU, kwant_id: 'k_ann', branch: '<img src=x onerror=alert(1)>', created_at: NOW - 5 * MIN, updated_at: NOW - MIN };
  const { host } = await mount({ cards: [card(evil), at(fx.PP_CHECKING, 'k_bo', 'b', 5, 1)], memberIds: ['k_ann', 'k_bo'], names, onOpen() {}, onOpenInbox() {}, now: () => NOW });
  const stack = host.querySelector('[data-prepush-pointers]');
  assert.equal(stack.getAttribute('aria-label'), 'Checks before pushing');
  const rows = [...host.querySelectorAll('[data-prepush-pointer]')];
  assert.equal(rows.length, 2);
  assert.equal(rows[0].dataset.prepushPhase, 'P4');
  assert.match(rows[0].className, /amber/);
  assert.equal(rows[0].querySelector('.kw-chip-warn').textContent, 'Needs you');
  assert.equal(rows[0].querySelector('code.dv-sha').textContent, evil.branch);
  assert.equal(host.querySelector('img'), null);
  assert.doesNotMatch(rows[1].className, /amber/);
  assert.equal(rows[1].querySelector('.dv-led').dataset.s, 'run');
  for (const r of rows) {
    assert.equal(r.tagName, 'BUTTON');
    assert.equal(r.getAttribute('type'), 'button');
    assert.match(r.className, /min-h-11/, '44 px on a phone');
    assert.match(text(r), /Open$/);
  }
  assert.equal([...host.querySelectorAll('button')].filter((b) => /try again|anyway/i.test(text(b))).length, 0, 'the card buttons stay in the 1:1');
  assert.equal(host.querySelector('[data-prepush-pointers-more]'), null);
});

test('Open: a click on the line opens the bot chat at that card; "+n more" opens the Inbox', async () => {
  const opened = [];
  let inbox = 0;
  const cards = ['k_ann', 'k_bo', 'k_cy', 'k_di'].map((k, i) => at(fx.PP_CHECKING, k, `c_${k}`, 10, i + 1));
  const { host } = await mount({ cards, memberIds: Object.keys(names), names, onOpen: (c) => opened.push(c.id), onOpenInbox: () => inbox++, now: () => NOW });
  const rows = [...host.querySelectorAll('[data-prepush-pointer]')];
  assert.equal(rows.length, 3);
  rows[1].dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  assert.deepEqual(opened, ['c_k_bo']);
  const more = host.querySelector('[data-prepush-pointers-more]');
  assert.equal(text(more), '+1 more in the Inbox');
  more.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  assert.equal(inbox, 1);
});

test('nothing renders when no member bot has a line', async () => {
  const { host } = await mount({ cards: [at(fx.PP_CHECKING, 'k_out', 'o', 5, 1)], memberIds: ['k_ann'], names, onOpen() {}, onOpenInbox() {}, now: () => NOW });
  assert.equal(host.innerHTML, '');
});

test('the live region speaks phase changes only, with the line text', async () => {
  const props = (raw) => ({ cards: [card({ ...raw, id: 'x', kwant_id: 'k_ann', branch: 'qa-skip', created_at: NOW - 5 * MIN, updated_at: NOW - MIN })], memberIds: ['k_ann'], names, onOpen() {}, onOpenInbox() {}, now: () => NOW });
  const { host, rerender } = await mount(props(fx.PP_CHECKING));
  const live = host.querySelector('[data-prepush-pointers-live]');
  assert.equal(live.getAttribute('aria-live'), 'polite');
  assert.equal(live.textContent, '', 'the first render says nothing');
  await rerender(props(fx.PP_CHECKING));
  assert.equal(live.textContent, '');
  await rerender(props(fx.PP_NEEDS_YOU));
  assert.equal(live.textContent, "Ann's push of qa-skip is waiting for you: 1 finding open");
});

test('a settled line leaves on its own when its 10 min are up', async () => {
  let now = NOW;
  const c = at(fx.PP_CLEAR, 'k_ann', 'cl', 30, 10 - 0.001); // leaves 60 ms after NOW
  const { host } = await mount({ cards: [c], memberIds: ['k_ann'], names, onOpen() {}, onOpenInbox() {}, now: () => now });
  assert.ok(host.querySelector('[data-prepush-pointer]'));
  now = NOW + MIN;
  await new Promise((r) => setTimeout(r, 200));
  await settle();
  assert.equal(host.querySelector('[data-prepush-pointer]'), null);
});
