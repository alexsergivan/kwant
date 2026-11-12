// 0.9.5 F (release-095/inbox-away/brief.md): Inbox "Done while you were away". Sections, the done pane's
// order and copy (mockup 2), Looks good / Ask to redo, Roll back only when something changed, the 7-day
// footer, and the routine History link to a run's result.
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
const fetched = [];
g.fetch = async (url) => (fetched.push(String(url)), { ok: false, status: 404, json: async () => ({}) });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as away from './src/doneAway';
  export * as fx from './src/doneAwayFixture';
  export * as store from './src/inboxStore';
  export { InboxNew } from './src/components/InboxNew';
  export { DoneItemView, useDoneDetail } from './src/components/DoneDetail';
  export { RoutinesPanel, runResultTarget } from './src/components/RoutinesPanel';
`);
const { React, createRoot, away: a, fx, store: s } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const clone = (x) => JSON.parse(JSON.stringify(x));
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

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

test('sections: Needs you holds waiting and failed, Done holds to review and finished; a routine run folds into its review', () => {
  const r = a.sectionItems(fx.AWAY_INBOX);
  assert.deepEqual(r.needs.map((i) => i.id), ['ap-1']);
  assert.deepEqual(r.done.map((i) => i.id), ['r-prices', 'd-spring'], 'the run row is folded, the team job stays');
  assert.equal(r.runFor['r-prices'].id, 'run-1');
  // A run of another routine, or one far from the review's time, is not folded.
  const far = clone(fx.AWAY_INBOX);
  far.items.find((i) => i.id === 'run-1').timestamp = '2026-11-11T20:00:00Z';
  assert.deepEqual(a.sectionItems(far).done.map((i) => i.id), ['r-prices', 'run-1', 'd-spring']);
  assert.equal(a.rowKicker(fx.AWAY_ROUTINE_REVIEW), 'Routine');
  assert.equal(a.doneTitle(fx.AWAY_ROUTINE_REVIEW), 'Competitor prices');
});

test('meta line: what ran, when, and why; without the run it leaves the why out', () => {
  const it = fx.AWAY_ROUTINE_REVIEW;
  assert.equal(a.metaLine(it, fx.AWAY_ROUTINE, fx.AWAY_RUN), 'Ben · routine, every Mon 09:00 · ran 22:51 because you pressed Run now');
  assert.equal(a.metaLine(it, fx.AWAY_ROUTINE, { ...fx.AWAY_RUN, trigger: 'schedule' }), 'Ben · routine, every Mon 09:00 · ran 22:51 on schedule');
  assert.equal(a.metaLine(it, null, null), 'Ben · routine · ran 22:51');
  assert.equal(a.scheduleShort({ type: 'daily', time: '08:30', tz: 'UTC' }), 'every day 08:30');
  assert.equal(a.scheduleShort({ type: 'interval', every_minutes: 120 }), 'every 2 h');
  assert.equal(a.scheduleShort(null), '');
});

test('trail: tool steps at their own time, thinking left out, then where the answer was written', () => {
  const rows = a.trailFrom(fx.AWAY_MESSAGE, '@Spring launch');
  assert.deepEqual(rows.map((r) => `${r.at} ${r.what}${r.detail ? ' · ' + r.detail : ''}`), [
    '22:51:02 Opened beanline.example.com · found House Blend 1 kg $24.00, 15% off until Mar 31',
    '22:51:05 Opened roastco.example.com · found 1 kg $27.00, free shipping over $40',
    '22:51:09 Compared with last Monday’s run · nothing changed',
    '22:51:10 Wrote the answer in @Spring launch',
  ]);
  assert.deepEqual(a.trailFrom({ ...fx.AWAY_MESSAGE, meta: null }, 'x'), [], 'no saved steps: no made-up trail');
  assert.deepEqual(a.trailFrom(null, 'x'), []);
});

test('changed: the item field wins, then meta.changed, then rollback_ok; Roll back hides when nothing changed', () => {
  const it = fx.AWAY_ROUTINE_REVIEW;
  assert.equal(a.changedState(it, null), false);
  const noField = { ...it, changed: undefined };
  assert.equal(a.changedState(noField, { ...fx.AWAY_MESSAGE, meta: { changed: false } }), false);
  assert.equal(a.changedState(noField, null), true, 'rollback_ok means there is something to roll back');
  assert.equal(a.changedState({ ...noField, rollback_ok: false }, null), null);
  assert.equal(a.rollbackShown(it, false), false);
  assert.equal(a.rollbackShown(it, null), true);
  assert.equal(a.rollbackShown({ ...it, rollback_ok: false }, true), false);
  assert.equal(a.noChecksSet(it), true);
});

test('queue: Needs you · 1 then Done while you were away · 2, kicker and place, the 7-day footer', async () => {
  const { q, qa, root } = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } });
  assert.deepEqual(qa('[data-section]').map((x) => x.getAttribute('data-section')), ['needs', 'done']);
  const heads = qa('[data-section] > h3').map((x) => x.textContent.replace(/\s+/g, ' ').trim());
  assert.match(heads[0], /Needs you · 1/);
  assert.match(heads[1], /Done while you were away · 2/);
  assert.equal(q('[data-row="ap-1"]').textContent.includes('@Spring launch · Ann'), true);
  assert.equal(q('[data-row="r-prices"] [data-kicker]').textContent, 'Routine · ');
  assert.equal(q('[data-row="r-prices"] [data-done-summary]').textContent, 'No change since last week. Beanline still cheapest.');
  assert.equal(q('[data-row="run-1"]'), null, 'the run is folded into its review');
  assert.equal(q('[data-row="d-spring"] [data-done-summary]').textContent, 'Ben checked 2 shops, Cody drafted the email, you picked 15%');
  assert.match(q('[data-done-footer]').textContent, /Done items stay here for 7 days\./);
  assert.match(q('[data-earlier]').textContent, /Earlier this week/);
  root.unmount();
});

test('approval row: its own question when sent, else the card action; title alone is never used', async () => {
  const data = clone(fx.AWAY_INBOX);
  let r = await mountInbox({}, data);
  const title = (q) => q('[data-row="ap-1"]').textContent;
  assert.match(title(r.q), /Approve “Spring sale: 15% off”\?/);
  r.root.unmount();
  const bare = clone(fx.AWAY_INBOX);
  const ap = bare.items.find((x) => x.id === 'ap-1');
  delete ap.question;
  ap.title = 'Use gmail (mail)';
  r = await mountInbox({}, bare);
  assert.equal(title(r.q).includes('Use gmail (mail)'), false, 'no raw server title');
  assert.equal(title(r.q).includes('Approve “Spring sale'), false);
  r.root.unmount();
});

test('pane order: meta, chips, answer with Open in chat, What Ben did, Why, then Looks good and Ask Ben to redo…', async () => {
  const opened = [];
  const edits = [];
  const { q, qa, root } = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL }, onOpenMessage: (t) => opened.push(t), onEditRoutine: (id) => edits.push(id) });
  await click(q('[data-option="r-prices"]'));
  const pane = q('[data-done-answer]').closest('section[role="region"], [data-review-item], div');
  assert.ok(pane);
  const order = ['[data-done-meta]', '[data-done-chips]', '[data-done-answer]', '[data-done-trail]', '[data-done-why]', '[data-approve]'];
  const els = order.map((x) => q(x));
  els.forEach((e, i) => assert.ok(e, `${order[i]} is drawn`));
  for (let i = 1; i < els.length; i++) {
    assert.ok(els[i - 1].compareDocumentPosition(els[i]) & Node.DOCUMENT_POSITION_FOLLOWING, `${order[i - 1]} comes before ${order[i]}`);
  }
  assert.equal(q('[data-done-meta]').textContent, 'Ben · routine, every Mon 09:00 · ran 22:51 because you pressed Run now');
  assert.match(q('[data-chip="read-only"]').textContent, /Read only · changed nothing/);
  assert.match(q('[data-chip="no-checks"]').textContent, /No checks set · Add one/);
  assert.match(q('[data-done-answer] h3').textContent, /Ben’s answer/);
  assert.ok(q('[data-done-answer] table'), 'the answer is the message, table and all');
  assert.equal(q('[data-done-answer]').innerHTML.includes('<script'), false);
  assert.match(q('[data-done-trail] h3').textContent, /What Ben did/);
  assert.equal(qa('[data-trail-row]').length, 4);
  assert.match(q('[data-why-quote]').textContent, /^Check Beanline and RoastCo prices/);
  assert.match(q('[data-done-why]').textContent, /You set up this routine on Nov 11:/);
  assert.equal(q('[data-approve]').textContent.trim(), 'Looks good');
  assert.equal(q('[data-send-back]').textContent.trim(), 'Ask Ben to redo…');
  assert.equal(qa('kbd').length, 0, 'no letter hints on the buttons');
  assert.equal(q('[data-approve]').getAttribute('aria-keyshortcuts'), 'A', 'the shortcut stays');
  assert.equal(q('[data-roll-back]'), null, 'nothing changed: no Roll back');
  assert.equal(q('[data-verdict]'), null, 'no checks: no verdict box');
  await click(q('[data-open-in-chat]'));
  assert.deepEqual(opened, [fx.AWAY_TARGET]);
  await click(q('[data-edit-routine]'));
  await click(q('[data-add-check]'));
  assert.deepEqual(edits, ['rt-prices', 'rt-prices']);
  root.unmount();
});

test('a review that changed something keeps Roll back; a non-routine review keeps its old layout', async () => {
  const data = clone(fx.AWAY_INBOX);
  data.items.find((i) => i.id === 'r-prices').changed = true;
  const { q, root } = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } }, data);
  await click(q('[data-option="r-prices"]'));
  assert.ok(q('[data-roll-back]'), 'Roll back shows when something changed');
  assert.equal(q('[data-chip="read-only"]'), null);
  root.unmount();
});

test('a finished team job opens in the pane: meta, the answer from its summary, Ask Cody to redo… opens its chat', async () => {
  const chats = [];
  const { q, root } = await mountInbox({ onOpen: (i) => chats.push(i.id) });
  await click(q('[data-option="d-spring"]'));
  assert.equal(q('[data-done-meta]').textContent, 'Cody · finished 22:45');
  assert.match(q('[data-answer-text]').textContent, /Ben checked 2 shops/);
  assert.equal(q('[data-done-trail]'), null, 'no trail for a job the API has no steps for');
  assert.match(q('[data-redo]').textContent, /Ask Cody to redo…/);
  root.unmount();
});

test('the pane loads routine, run and message itself through the done api', async () => {
  const calls = [];
  const { q, root } = await mountInbox({ doneApi: fx.awayDoneApi(calls) });
  await click(q('[data-option="r-prices"]'));
  await settle(20);
  assert.deepEqual(calls, ['routines', 'runs:rt-prices', 'message:m-prices']);
  assert.equal(q('[data-done-meta]').textContent, 'Ben · routine, every Mon 09:00 · ran 22:51 because you pressed Run now');
  assert.equal(q('[data-chip="read-only"]') !== null, true);
  root.unmount();
});

test('Earlier this week loads the last 7 days', async () => {
  s.inboxStore.reset();
  const seen = [];
  s.inboxSource.get = async (since) => (seen.push(since), clone(fx.AWAY_INBOX));
  const now = Date.parse('2026-11-12T00:50:00Z');
  await s.showEarlier(now);
  assert.equal(s.inboxStore.get().since, '2026-11-05T00:50:00.000Z');
  assert.equal(a.doneSince(now), '2026-11-05T00:50:00.000Z');
  assert.equal(a.DONE_FOOTER, 'Done items stay here for 7 days.');
});

test('routine History: a run with a result message links to it', () => {
  assert.deepEqual(m.runResultTarget(fx.AWAY_ROUTINE, fx.AWAY_RUN), fx.AWAY_TARGET);
  assert.equal(m.runResultTarget(fx.AWAY_ROUTINE, { ...fx.AWAY_RUN, message_id: null }), null);
});
