// 0.9.0 V1 review inbox (release-090/designs/v1-review-inbox.md, verification-plan.md D1–D5): the Approve all
// runner, the badge, H5 rank 3 "To review", the two-pane queue, and the Mission board / Dev desk entry points.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as store from './src/inboxStore';
  export { inboxApi, InboxError, inboxBadge, verdictShort, cmpSortKey, INBOX_FIXTURE, INBOX_FIXTURE_EMPTY, FIXTURE_BATCH, FIXTURE_REVIEW_ITEMS } from './src/api/inbox';
  export { attentionRows, attentionRank, PINNED_RANK, rollupLabel } from './src/chatsTab';
  export { InboxNew } from './src/components/InboxNew';
  export { TaskCard } from './src/components/MissionBoardView';
`);
const { React, store: s, inboxApi, InboxError } = m;
const html = (el) => m.renderToStaticMarkup(el);
const doc = (h) => new DOMParser().parseFromString(`<body>${h}</body>`, 'text/html');
const clone = (x) => JSON.parse(JSON.stringify(x));

const seed = (data = clone(m.INBOX_FIXTURE)) => {
  s.inboxStore.reset();
  s.inboxSource.get = async () => data;
  s.inboxStore.patch({ data });
  return data;
};

test('badge: counts.badge when the server sends it, else unseen + pre-push', () => {
  assert.equal(m.inboxBadge({ counts: { badge: 13 } }, { unseen: { total: 2 } }, 1), 13);
  assert.equal(m.inboxBadge({ counts: {} }, { unseen: { total: 2 } }, 1), 3);
  assert.equal(m.inboxBadge(null, null, 0), 0);
});

test('Approve all: one at a time with verdict_id; a stale item comes back on its own and the rest carry on', async () => {
  seed();
  const calls = [];
  inboxApi.verify = async (taskId, verdictId) => {
    calls.push([taskId, verdictId]);
    if (taskId === m.FIXTURE_BATCH[1].task_id) throw new InboxError(409, 'stale', 'stale', ['src/a.ts']);
  };
  await s.approveAll();
  const b = s.inboxStore.get().batch;
  assert.deepEqual(calls, m.FIXTURE_BATCH.map((i) => [i.task_id, i.verdict.id]), 'every item, in order, each with its verdict id');
  assert.deepEqual(b.rows.map((r) => r.status), ['approved', 'came_back', 'approved', 'approved', 'approved']);
  assert.equal(b.running, false);
  const back = s.inboxStore.get().data.items.find((i) => i.id === m.FIXTURE_BATCH[1].id);
  assert.equal(back?.group, 'failed_or_stuck', 'its own row in Failed or stuck');
  assert.match(s.inboxStore.get().notice.text, /Approved 4 of 5 · 1 came back/);
});

test('Approve all: "Stop after this one" finishes the current item and leaves the rest', async () => {
  seed();
  let n = 0;
  inboxApi.verify = async () => {
    n++;
    if (n === 2) s.stopAfterThisOne();
  };
  await s.approveAll();
  assert.deepEqual(s.inboxStore.get().batch.rows.map((r) => r.status), ['approved', 'approved', 'stopped', 'stopped', 'stopped']);
  assert.equal(n, 2);
});

test('Approve all: signed out halts the run; nothing after it is tried', async () => {
  seed();
  let n = 0;
  inboxApi.verify = async () => {
    if (++n === 2) throw new InboxError(401, 'Sign in again');
  };
  await s.approveAll();
  const b = s.inboxStore.get().batch;
  assert.deepEqual(b.rows.map((r) => r.status), ['approved', 'error', 'stopped', 'stopped', 'stopped']);
  assert.match(b.halted, /sign in again/);
  assert.equal(n, 2);
});

test('deep link: ?item= takes an item id or a task id; the crumb remembers where it came from', () => {
  const data = seed();
  const it = data.items.find((i) => i.kind === 'review');
  s.openItem(it.task_id, 'board');
  assert.equal(s.inboxStore.get().selectedId, it.id);
  assert.equal(s.inboxStore.get().crumb, 'board');
  s.inboxStore.patch({ data: null });
  s.openItem('r-later', null);
  assert.equal(s.inboxStore.get().pendingKey, 'r-later', 'resolved once the list loads');
});

test('H5 rank 3 is To review; the pinned block covers 1–4 and the rollup counts it', () => {
  const data = clone(m.INBOX_FIXTURE);
  const review = data.items.find((i) => i.kind === 'review' && i.group === 'to_review');
  assert.equal(m.attentionRank({ review }), 3);
  assert.equal(m.PINNED_RANK, 4);
  const kw = { id: review.kwant_id, name: review.kwant_name };
  const rows = m.attentionRows({ kwants: [kw, { id: 'zz', name: 'Zed' }], needs: [], failed: [], finished: [], busy: new Set(), status: {}, openedAt: {}, reviews: [review] });
  assert.equal(rows[0].rank, 3);
  assert.match(rows[0].line ?? rows[0].text ?? JSON.stringify(rows[0]), /To review/);
  assert.equal(m.rollupLabel({ needs: 0, stuck: 0, finished: 0, review: 2 }), '2 to review');
});

test('Inbox › New, wide: 392px queue, item pane with the first review selected, Low-risk card with Approve all N', () => {
  seed();
  s.select(null);
  const d = doc(html(React.createElement(m.InboxNew, { noLoad: true, wide: true, now: Date.parse(m.INBOX_FIXTURE.now) })));
  assert.ok(d.querySelector('[data-inbox-new="wide"] .w-\\[392px\\]'));
  assert.deepEqual([...d.querySelectorAll('[data-section]')].map((n) => n.getAttribute('data-section')), ['waiting_on_you', 'failed_or_stuck', 'to_review', 'finished']);
  const card = d.querySelector('[data-batch-card]');
  assert.ok(card);
  assert.match(d.querySelector('[data-approve-all]').textContent, /Approve all 5/);
  assert.ok(d.querySelector('[data-inline-approve]'), 'a one-tap row has inline Approve');
  assert.ok(d.querySelector('[data-item-pane]'));
});

test('Inbox › New, phone: a list; empty shows all clear', () => {
  seed(clone(m.INBOX_FIXTURE_EMPTY));
  const d = doc(html(React.createElement(m.InboxNew, { noLoad: true, wide: false })));
  assert.ok(d.querySelector('[data-inbox-new="list"]'));
  assert.ok(d.querySelector('[data-all-clear]'));
});

const task = (over = {}) => ({
  id: 't1', space_id: 's1', mission_id: 'm1', mission_title: 'M', title: 'Write the pricing copy', acceptance: '', state: 'review', priority: 1,
  owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 50000, spent_tokens: 12400,
  attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1',
  channel_id: 'c1', direct_kwant_id: null, thread_id: null, created_by: 'k_ada',
  created_at: '2026-10-27T09:00:00Z', updated_at: '2026-10-27T11:00:00Z', finished_at: null,
  depends_on: [], waiting_on: [], claim: null, needs_you: [], checks_summary: { status: 'passed', passed: 2, failed: 0, total: 2 }, ...over,
});

test('Mission board card in Review: one "Review" button (verdict under the title), Dismiss stays, no Mark verified or Send back', () => {
  const data = seed();
  const it = data.items.find((i) => i.kind === 'review' && i.verdict.result === 'pass' && i.verdict.confidence);
  const card = (p) => doc(html(React.createElement('ul', null, React.createElement(m.TaskCard, { task: task({ id: it.task_id }), names: {}, onOpen: () => {}, onQuick: () => {}, ...p }))));
  s.inboxNav.go = () => {};
  const d = card();
  assert.ok(d.querySelector('[data-card-review]'));
  assert.equal(d.querySelector('[data-card-action="verify"]'), null);
  assert.equal(d.querySelector('[data-card-action="reject"]'), null);
  assert.ok(d.querySelector('[data-card-action="cancel"]'), 'Dismiss stays');
  assert.equal(d.querySelector('[data-card-verdict]').textContent, m.verdictShort(it.verdict));
  s.inboxNav.go = null;
  const old = card();
  assert.equal(old.querySelector('[data-card-review]'), null, 'without the Inbox the old buttons stay');
  assert.ok(old.querySelector('[data-card-action="verify"]'));
});

// ---- D7 Being checked, V1-3 held count, V1-4 stale notice once ----

const withItems = (...extra) => {
  const data = clone(m.INBOX_FIXTURE);
  data.items = [...data.items.filter((i) => !extra.some((e) => e.id === i.id)), ...clone(extra)].sort(m.cmpSortKey);
  return seed(data);
};
const rowOf = (d, id) => d.querySelector(`[data-row="${id}"]`);
const wide = () => doc(html(React.createElement(m.InboxNew, { noLoad: true, wide: true, now: Date.parse(m.INBOX_FIXTURE.now) })));

test('D7: the list row says "Being checked" while the verifier holds it, "Not checked yet" without a hold', () => {
  const FX = m.FIXTURE_REVIEW_ITEMS;
  withItems(FX.beingChecked, FX.notChecked);
  const d = wide();
  const being = rowOf(d, FX.beingChecked.id);
  assert.equal(being.querySelector('[data-verdict-word]').getAttribute('data-verdict-word'), 'being_checked');
  assert.match(being.textContent, /Being checked/);
  assert.doesNotMatch(being.textContent, /Not checked yet/);
  assert.match(rowOf(d, FX.notChecked.id).textContent, /Not checked yet/);
  assert.equal(being.querySelector('[data-inline-approve]'), null);
});

test('D7: inline Approve and Approve all leave a Being checked item out, even if the server offers it', async () => {
  const FX = m.FIXTURE_REVIEW_ITEMS;
  const offered = { ...FX.beingChecked, one_tap: true, batchable: true };
  const data = withItems(offered);
  data.batch = { ...data.batch, ids: [...data.batch.ids.slice(0, 2), offered.id, ...data.batch.ids.slice(2)] };
  s.inboxStore.patch({ data });
  assert.equal(rowOf(wide(), offered.id).querySelector('[data-inline-approve]'), null, 'no inline Approve');
  assert.equal(s.batchItems(data).some((i) => i.id === offered.id), false);
  const calls = [];
  inboxApi.verify = async (taskId) => calls.push(taskId);
  await s.approveAll();
  assert.deepEqual(calls, m.FIXTURE_BATCH.map((i) => i.task_id), 'only the five checked items');
  assert.equal(calls.includes(offered.task_id), false);
});

test('V1-3: denying every held action drops "N held" from the row at once, without a refresh', () => {
  const FX = m.FIXTURE_REVIEW_ITEMS;
  const two = {
    ...FX.passHeld,
    held_actions: [FX.passHeld.held_actions[0], { ...FX.passHeld.held_actions[0], approval_id: 'a-refund-2', summary: 'create second refund' }],
  };
  withItems(two);
  assert.equal(rowOf(wide(), two.id).querySelector('[data-held-count]').textContent, '2 held');
  s.markHeld(two.id, 'a-refund', 'denied');
  assert.equal(rowOf(wide(), two.id).querySelector('[data-held-count]').textContent, '1 held');
  s.markHeld(two.id, 'a-refund-2', 'denied');
  assert.equal(rowOf(wide(), two.id).querySelector('[data-held-count]'), null);
  assert.doesNotMatch(rowOf(wide(), two.id).textContent, /held/);
  // Resolved ones the server still lists (heldDenied: denied + expired) are not counted either.
  withItems(FX.heldDenied);
  assert.equal(rowOf(wide(), FX.heldDenied.id).querySelector('[data-held-count]'), null);
});

test('V1-4: a stale inline Approve says "changed after its check" once, on the row, not again in the status line', async () => {
  const data = seed();
  const one = data.items.find((i) => i.kind === 'review' && i.one_tap);
  inboxApi.verify = async () => {
    throw new InboxError(409, 'stale', 'stale', ['README.md']);
  };
  s.select(null);
  await s.approveOne(one);
  const st = s.inboxStore.get();
  assert.doesNotMatch(st.notice.text, /changed after its check/);
  assert.match(st.notice.text, /back in Failed or stuck, and nothing was approved/);
  s.select(null);
  const d = wide();
  const row = rowOf(d, one.id);
  assert.equal(row.getAttribute('data-group'), 'failed_or_stuck');
  assert.match(row.textContent, /README\.md changed after its check/);
  assert.doesNotMatch(d.querySelector('[data-inbox-status]').textContent, /changed after its check/);
});

test('V1-4: the batch row of an item that came back shows "Came back"', async () => {
  seed();
  inboxApi.verify = async (taskId) => {
    if (taskId === m.FIXTURE_BATCH[1].task_id) throw new InboxError(409, 'stale', 'stale', ['src/a.ts']);
  };
  await s.approveAll();
  s.select('batch');
  const d = wide();
  const row = d.querySelector(`[data-batch-row="${m.FIXTURE_BATCH[1].id}"]`);
  assert.equal(row.getAttribute('data-status'), 'came_back');
  assert.equal(row.querySelector('[data-batch-word]').textContent, 'Came back');
  assert.match(row.textContent, /src\/a\.ts changed after its check/);
  assert.doesNotMatch(s.inboxStore.get().notice.text, /changed after its check/);
  assert.equal(d.querySelector(`[data-batch-row="${m.FIXTURE_BATCH[0].id}"] [data-batch-word]`).textContent, 'Approved');
});
