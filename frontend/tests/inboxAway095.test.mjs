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
const notFound = async (url) => (fetched.push(String(url)), { ok: false, status: 404, json: async () => ({}) });
g.fetch = notFound;

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
  assert.deepEqual(r.done.map((i) => i.id), ['r-prices', 'tj-spring'], 'the run row is folded, the team job stays');
  assert.equal(r.runFor['r-prices'].id, 'run-1');
  // A run of another routine, or one far from the review's time, is not folded.
  const far = clone(fx.AWAY_INBOX);
  far.items.find((i) => i.id === 'run-1').timestamp = '2026-11-11T20:00:00Z';
  assert.deepEqual(a.sectionItems(far).done.map((i) => i.id), ['r-prices', 'run-1', 'tj-spring']);
  assert.equal(a.rowKicker(fx.AWAY_ROUTINE_REVIEW), 'Routine');
  assert.equal(a.doneTitle(fx.AWAY_ROUTINE_REVIEW), 'Competitor prices');
});

test('meta line: what ran, when, and why; without the run it leaves the why out', () => {
  const it = fx.AWAY_ROUTINE_REVIEW;
  assert.equal(a.metaLine(it, fx.AWAY_ROUTINE, fx.AWAY_RUN), 'Ben · routine, every Mon 09:00 · ran 22:51 because you pressed Run now');
  assert.equal(a.metaLine(it, fx.AWAY_ROUTINE, { ...fx.AWAY_RUN, trigger: 'schedule' }), 'Ben · routine, every Mon 09:00 · ran 22:51 on schedule');
  assert.equal(a.metaLine({ ...it, source: undefined }, null, null), 'Ben · routine · ran 22:51');
  // Part B's source.trigger, when the run isn't loaded; an app event says so (Needs B 5).
  assert.equal(a.metaLine(it, null, null), 'Ben · routine · ran 22:51 because you pressed Run now');
  assert.equal(a.metaLine({ ...it, source: { ...it.source, trigger: 'event' } }, null, null), 'Ben · routine · ran 22:51 on an app event');
  assert.equal(a.metaLine(it, fx.AWAY_ROUTINE, { ...fx.AWAY_RUN, trigger: 'event' }), 'Ben · routine, every Mon 09:00 · ran 22:51 on an app event');
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

test('queue: Needs you · 1 then Done while you were away · 2, kicker and place, unread dots, the 7-day footer', async () => {
  const { q, qa, root } = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } });
  assert.deepEqual(qa('[data-section]').map((x) => x.getAttribute('data-section')), ['needs', 'done']);
  const heads = qa('[data-section] > h3').map((x) => x.textContent.replace(/\s+/g, ' ').trim());
  assert.match(heads[0], /Needs you · 1/);
  assert.match(heads[1], /Done while you were away · 2/);
  assert.equal(q('[data-row="ap-1"]').textContent.includes('@Spring launch · Ann'), true);
  assert.equal(q('[data-row="r-prices"] [data-kicker]').textContent, 'Routine · ');
  assert.equal(q('[data-row="r-prices"] [data-done-summary]').textContent, 'No change since last week. Beanline still cheapest.');
  assert.equal(q('[data-row="run-1"]'), null, 'the run is folded into its review');
  assert.equal(q('[data-row="tj-spring"] [data-done-summary]').textContent, 'Ben checked 2 shops, Cody drafted the email, you picked 15%');
  // Needs B 2: a team job names the team's size where a row names its bot.
  assert.equal(q('[data-row="tj-spring"]').textContent.includes('@Spring launch · 3 bots'), true);
  // Needs B 1: the header counts what is new, unread rows get a dot with "New" for screen readers.
  assert.equal(q('[data-inbox-new] h2 + p').textContent, '2 new since you last looked, 2 h ago');
  assert.equal(q('[data-row="r-prices"] [data-unread]').textContent, 'New');
  assert.ok(q('[data-row="ap-1"] [data-unread]'));
  assert.equal(q('[data-row="tj-spring"] [data-unread]'), null, 'a read row has no dot');
  assert.equal(q('[data-done-footer]').textContent, 'Done items stay here for 7 days.');
  assert.equal(q('[data-earlier]'), null, 'Earlier this week is gone: the list already holds 7 days');
  root.unmount();
});

test('approval row: its own question when sent, else the card action; title alone is never used', async () => {
  const data = clone(fx.AWAY_INBOX);
  let r = await mountInbox({}, data);
  const title = (q) => q('[data-row="ap-1"]').textContent;
  assert.match(title(r.q), /Approve “Spring sale: 15% off”\?/);
  // F6: the subline still says what the bot wants to use.
  assert.equal(r.q('[data-row="ap-1"] [data-approval-use]').textContent, '· use gmail');
  r.root.unmount();
  const bare = clone(fx.AWAY_INBOX);
  const ap = bare.items.find((x) => x.id === 'ap-1');
  delete ap.question;
  ap.title = 'Use gmail (mail)';
  r = await mountInbox({}, bare);
  assert.equal(title(r.q).includes('Use gmail (mail)'), false, 'no raw server title');
  assert.equal(title(r.q).includes('Approve “Spring sale'), false);
  assert.equal(r.q('[data-row="ap-1"] [data-approval-use]'), null, 'no question: the title already names the tool');
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
  // F2: a neutral chip, not amber; F1: the inline links stay compact and keep the 44 px tap area.
  assert.equal(/amber/.test(q('[data-chip="no-checks"]').className), false);
  for (const x of ['[data-add-check]', '[data-open-in-chat]', '[data-edit-routine]']) {
    assert.ok(q(x).hasAttribute('data-compact'), `${x} is compact`);
    assert.match(q(x).className, /kw-hit/);
  }
  // F5: the answer's scroller is a focusable, named region; on a phone its cells don't wrap.
  const ans = q('[data-answer-text]');
  assert.equal(ans.getAttribute('tabindex'), '0');
  assert.equal(ans.getAttribute('role'), 'region');
  assert.equal(ans.getAttribute('aria-label'), 'Ben’s answer');
  assert.match(ans.className, /max-sm:\[&_td\]:whitespace-nowrap/);
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

test('a team job opens in the pane: led-by meta, the lead’s answer, the team’s trail with who, skipped and dropped steps, Why', async () => {
  const opened = [];
  const { q, qa, root } = await mountInbox({ onOpenMessage: (t) => opened.push(t) });
  await click(q('[data-option="tj-spring"]'));
  assert.equal(q('[data-done-meta]').textContent, 'Ann led Ben and Cody · finished 22:45');
  assert.equal(q('[data-done-answer] h3').textContent, 'Ann’s answer');
  assert.match(q('[data-answer-text]').textContent, /^Done: Ben checked 2 shops/, 'the served excerpt, as text');
  assert.equal(q('[data-done-trail] h3').textContent, 'What the team did');
  assert.equal(q('[data-trail-dropped]').textContent.trim(), '3 more steps before these');
  const rows = qa('[data-trail-row]');
  assert.equal(rows.length, 5, 'four steps, then the answer');
  assert.equal(rows[0].querySelector('[data-trail-by]').textContent, 'Ben');
  assert.match(rows[0].textContent, /Ben · Opened beanline\.example\.com · House Blend/);
  assert.equal(rows[1].getAttribute('data-status'), 'error');
  assert.ok(rows[1].querySelector('[aria-label="Failed"]'));
  assert.equal(rows[3].getAttribute('data-status'), 'skipped');
  assert.match(rows[3].textContent, /Cody · Sent the draft to the list · skipped/);
  assert.equal(rows[3].querySelector('[aria-label="Failed"]'), null, 'skipped: no icon');
  assert.match(rows[3].lastElementChild.className, /text-zinc-400/);
  assert.match(rows[4].textContent, /Wrote the answer in @Spring launch/);
  assert.equal(q('[data-done-why]').textContent.replace(/\s+/g, ' ').trim(), 'Why You asked on Nov 11: Plan a spring sale for the house blend and draft the email.');
  assert.equal(q('[data-redo]').textContent.trim(), 'Ask Ann to redo in chat');
  await click(q('[data-redo]'));
  assert.deepEqual(opened, [{ message_id: 'm-spring', channel_id: 'c-spring', kwant_id: 'k-ann', thread_id: null }], 'opens the lead’s answer');
  root.unmount();
});

test('F7: a finished hand-off with no review and no part B fields: summary answer, Ask Cody to redo in chat opens its chat', async () => {
  const data = clone(fx.AWAY_INBOX);
  data.items.push(clone(fx.AWAY_HANDOFF));
  const chats = [];
  const { q, root } = await mountInbox({ onOpen: (i) => chats.push(i.id) }, data);
  await click(q('[data-option="d-spring"]'));
  assert.equal(q('[data-done-meta]').textContent, 'Cody · finished 22:45');
  assert.match(q('[data-answer-text]').textContent, /Drafted the sale email/);
  assert.equal(q('[data-done-trail]'), null, 'no trail for a job the API has no steps for');
  assert.equal(q('[data-redo]').textContent.trim(), 'Ask Cody to redo in chat');
  await click(q('[data-redo]'));
  assert.deepEqual(chats, ['d-spring']);
  root.unmount();
});

test('Why for a task and a hand-off (Needs B 3)', () => {
  const base = { ...fx.AWAY_HANDOFF, why: 'Check the RoastCo price' };
  assert.equal(a.whyLead({ ...base, why_kind: 'task' }), 'The task said:');
  assert.equal(a.whyLead({ ...base, why_kind: 'ask' }), 'You asked Cody:');
  assert.equal(a.whyLead({ ...base, why_kind: 'request' }), 'You asked on Nov 11:');
  assert.equal(a.whyLead({ ...base, why_kind: 'routine' }), null, 'routines keep their own copy');
  assert.equal(a.whyLead({ ...base, why: '  ', why_kind: 'task' }), null);
  assert.equal(a.ledLine({ ...fx.AWAY_TEAM_JOB, members: [1, 2, 3, 4].map((n) => ({ kwant_id: `k${n}`, kwant_name: `B${n}` })) }), 'Ann led B1, B2 and 2 more');
  assert.equal(a.teamSize({ ...fx.AWAY_TEAM_JOB, members: [] }), '1 bot');
  assert.equal(a.teamSize({ ...fx.AWAY_TEAM_JOB, members: undefined }), null, 'no members field: no count');
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

test('header: n new, nothing new, and today’s window line on a server older than part B; Done’s empty line', async () => {
  const now = Date.parse(fx.AWAY_INBOX.now);
  assert.equal(a.unreadLine(fx.AWAY_INBOX, now), '2 new since you last looked, 2 h ago');
  const none = clone(fx.AWAY_INBOX);
  none.items.forEach((i) => (i.unread = false));
  assert.equal(a.unreadLine(none, now), 'Nothing new since you last looked, 2 h ago');
  const old = clone(fx.AWAY_INBOX);
  old.items.forEach((i) => delete i.unread);
  old.since = old.last_seen;
  assert.equal(a.unreadLine(old, now), null);
  const empty = { ...clone(fx.AWAY_INBOX), items: [] };
  assert.equal(a.unreadLine(empty, now), 'Nothing new since you last looked, 2 h ago', 'a 7-day window with nothing in it');
  const doneless = clone(fx.AWAY_INBOX);
  doneless.items = doneless.items.filter((i) => i.group === 'waiting_on_you');
  const { q, root } = await mountInbox({}, doneless);
  assert.equal(q('[data-section="done"] p').textContent, 'Nothing finished in the last 7 days.');
  root.unmount();
});

test('F3: after Looks good the notice uses the button’s words and the routine’s name; a failure says nothing was saved', async () => {
  g.fetch = async (url) => (fetched.push(String(url)), { ok: true, status: 200, json: async () => ({}) });
  let r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } });
  await click(r.q('[data-option="r-prices"]'));
  await click(r.q('[data-approve]'));
  await settle(20);
  assert.equal(r.q('[data-inbox-status]').textContent, 'Looks good: “Competitor prices”.');
  r.root.unmount();
  g.fetch = async (url) => (fetched.push(String(url)), { ok: false, status: 500, json: async () => ({ error: 'Disk full.' }) });
  r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } });
  await click(r.q('[data-option="r-prices"]'));
  await click(r.q('[data-approve]'));
  await settle(20);
  assert.equal(r.q('[data-review-status]').textContent, 'Couldn’t save “Competitor prices”. Disk full. Nothing was saved.');
  r.root.unmount();
  g.fetch = notFound;
});

test('F4: on a phone, Ask Ben to redo… spans the row when there is no Roll back', async () => {
  const mm = window.matchMedia;
  window.matchMedia = (query) => ({ matches: /max-width/.test(query), media: query, addEventListener() {}, removeEventListener() {} });
  try {
    let r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } });
    await click(r.q('[data-option="r-prices"]'));
    assert.match(r.q('[data-send-back]').className, /col-span-2/);
    r.root.unmount();
    const data = clone(fx.AWAY_INBOX);
    data.items.find((i) => i.id === 'r-prices').changed = true;
    r = await mountInbox({ doneDetails: { 'r-prices': fx.AWAY_DETAIL } }, data);
    await click(r.q('[data-option="r-prices"]'));
    assert.ok(r.q('[data-roll-back]'));
    assert.equal(/col-span-2/.test(r.q('[data-send-back]').className), false, 'with Roll back, the two share the row');
    r.root.unmount();
  } finally {
    window.matchMedia = mm;
  }
});

test('routine History: a run with a result message links to it', () => {
  assert.deepEqual(m.runResultTarget(fx.AWAY_ROUTINE, fx.AWAY_RUN), fx.AWAY_TARGET);
  assert.equal(m.runResultTarget(fx.AWAY_ROUTINE, { ...fx.AWAY_RUN, message_id: null }), null);
});
