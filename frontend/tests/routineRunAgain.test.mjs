// routine-run-again (autonomy/routine-run-again/brief.md; Mira's routine-for-a-week pass, fix 2): a failed routine run
// that names its routine (Vega's #1114 `routine_id`) gets one Run again button in the Needs-you strip and in its Inbox
// row, which POSTs /api/routines/:id/run; without routine_id nothing changes. The routine_failed toast gets
// "Open routine", which opens Routines on that routine's row with its History. Red at #1114's head 8a3eaf34.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
const calls = [];
g.fetch = async (url, init = {}) => {
  calls.push({ url: String(url), method: init.method ?? 'GET' });
  if (/\/api\/routines\/[^/]+\/run$/.test(String(url))) return { ok: true, status: 200, json: async () => ({ id: 'run-new', status: 'running' }), text: async () => '' };
  return { ok: false, status: 404, json: async () => ({}), text: async () => '' };
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * as ask from './src/askHere';
  export * as rra from './src/routineRunAgain';
  export * as store from './src/inboxStore';
  export { InboxNew } from './src/components/InboxNew';
  export { NeedsYouStrip } from './src/components/NeedsYouStrip';
  export { RoutineRow } from './src/components/RoutinesPanel';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 12) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
const NOW = '2026-11-23T19:00:00Z';

// The failed run from impl.md's "The field (for Iris)" example, as /api/inbox sends it.
const failedRun = (extra = {}) => ({
  kind: 'routine_run',
  id: 'run-5b0c',
  kwant_id: 'k-lead',
  kwant_name: 'Ops',
  title: 'Weekly report',
  summary: 'the provider timed out',
  status: 'error',
  timestamp: '2026-11-23T18:40:00Z',
  link: { thread_id: null, channel_id: null, direct_kwant_id: 'k-lead' },
  routine_id: 'rt-9e7d',
  action: 'retry',
  dismissable: true,
  group: 'failed_or_stuck',
  sort_key: [2, 0, 0, 'Ops', 'run-5b0c'],
  unread: true,
  ...extra,
});
const OLD_SENTENCE = 'There’s no button for this one. Press Run now on it in Routines, or clear it once it’s handled.';
const noRoutine = () => {
  const it = failedRun();
  delete it.routine_id;
  return it;
};

test('askHere: a failed routine run with routine_id gets Run again; without it, no button and today’s sentence', () => {
  for (const status of ['error', 'incomplete', 'needs_you']) assert.equal(m.ask.askAction(failedRun({ status })), 'run_again', status);
  assert.equal(m.ask.ACTION_LABEL.run_again, 'Run again');
  assert.equal(m.ask.askHint(failedRun(), 'run_again', 'Ops'), 'Run it again here, or clear it once it’s handled.');
  assert.doesNotMatch(m.ask.askHint(failedRun(), 'run_again', 'Ops'), /board|no button/i);
  // An older server (no field), a null, an empty id, a finished run or another kind: as today.
  for (const it of [noRoutine(), failedRun({ routine_id: null }), failedRun({ routine_id: '' })]) {
    assert.equal(m.ask.askAction(it), null);
    assert.equal(m.ask.askHint(it, null, 'Ops'), OLD_SENTENCE);
  }
  assert.equal(m.rra.runAgainId(failedRun({ status: 'ok' })), null);
  assert.equal(m.rra.runAgainId(failedRun({ kind: 'delegation' })), null);
  assert.equal(m.rra.runAgainId(failedRun()), 'rt-9e7d');
});

function mountStrip(props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  m.flushSync(() => root.render(h(m.NeedsYouStrip, { onAction: () => {}, onDismiss: async () => {}, now: Date.parse(NOW), ...props })));
  return { host, unmount: () => (root.unmount(), host.remove()) };
}

test('strip: Run again shows, runs the action, then says Started. and holds the button; no routine_id, no button', async () => {
  const seen = [];
  const s = mountStrip({ asks: [failedRun()], onAction: async (item, action) => void seen.push([item.routine_id, action]), sentWaitMs: 60_000 });
  const btn = s.host.querySelector('[data-strip-action="run_again"]');
  assert.ok(btn, 'one Run again button');
  assert.equal(btn.textContent, 'Run again');
  assert.equal(btn.getAttribute('aria-label'), 'Run Ops’s routine again');
  assert.equal(s.host.querySelector('[data-strip-hint]').textContent, 'Run it again here, or clear it once it’s handled.');
  await click(btn);
  assert.deepEqual(seen, [['rt-9e7d', 'run_again']]);
  assert.equal(s.host.querySelector('[data-strip-hint]').textContent, 'Started.');
  assert.equal(s.host.querySelector('[data-needs-you-live]').textContent, 'Started.', 'said to a screen reader');
  assert.equal(s.host.querySelector('[data-needs-you-strip]').getAttribute('data-strip-state'), 'started');
  assert.ok(s.host.querySelector('[data-strip-action="run_again"]').disabled, 'no second run from a double tap');
  assert.ok(s.host.querySelector('[data-strip-dismiss]'), 'the item stays until the run takes it away');
  s.unmount();

  const fail = mountStrip({ asks: [failedRun()], onAction: async () => { throw new Error('the routine was deleted'); } });
  await click(fail.host.querySelector('[data-strip-action="run_again"]'));
  assert.equal(fail.host.querySelector('[data-strip-error]').textContent, 'Couldn’t run it again: the routine was deleted. Try again, or write to Ops.');
  assert.equal(fail.host.querySelector('[data-strip-action="run_again"]').disabled, false);
  fail.unmount();

  const old = mountStrip({ asks: [noRoutine()] });
  assert.equal(old.host.querySelector('[data-strip-action]'), null);
  assert.equal(old.host.querySelector('[data-strip-hint]').textContent, OLD_SENTENCE);
  old.unmount();
});

test('runAgain posts to /api/routines/{routine_id}/run, the Run now call', async () => {
  calls.length = 0;
  assert.equal(await m.rra.runAgain(failedRun()), 'started');
  assert.deepEqual(calls, [{ url: '/api/routines/rt-9e7d/run', method: 'POST' }]);
  assert.equal(await m.rra.runAgain(failedRun(), async () => ({ status: 'skipped' })), 'skipped');
  await assert.rejects(m.rra.runAgain(noRoutine()));
});

async function mountInbox(items) {
  document.body.innerHTML = '';
  sessionStorage.clear();
  m.store.inboxStore.reset();
  const data = { now: NOW, since: '2026-11-22T19:00:00Z', last_seen: NOW, counts: { waiting_on_you: 0, failed_or_stuck: items.length, to_review: 0, finished: 0, badge: items.length }, items, batch: null };
  m.store.inboxSource.get = async () => data;
  m.store.inboxStore.patch({ data });
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide: true, now: Date.parse(NOW) }));
  await settle();
  return { el, q: (x) => el.querySelector(x), unmount: () => root.unmount() };
}

test('Inbox row: a failed routine run gets Run again, which posts the run and says Started; none without routine_id', async () => {
  const v = await mountInbox([failedRun()]);
  const btn = v.q('[data-run-again="run-5b0c"]');
  assert.ok(btn, 'Run again on the row');
  assert.equal(btn.textContent.trim(), 'Run again');
  assert.equal(btn.getAttribute('aria-label'), 'Run “Weekly report” again');
  calls.length = 0;
  await click(btn);
  assert.deepEqual(calls.filter((c) => c.method === 'POST'), [{ url: '/api/routines/rt-9e7d/run', method: 'POST' }]);
  assert.match(v.el.textContent, /Started: “Weekly report”/);
  v.unmount();

  const old = await mountInbox([noRoutine()]);
  assert.equal(old.q('[data-run-again]'), null);
  old.unmount();
});

const routine = (id, name, extra = {}) => ({
  id, kwant_id: 'k-lead', channel_id: null, name, instructions: 'Write the weekly report.', schedule: { type: 'weekly', weekdays: ['mon'], time: '09:00', tz: 'UTC' },
  enabled: true, next_run_at: null, last_run_at: null, last_status: null, created_at: NOW, ...extra,
});

test('toast: routine_failed has Open routine, which opens Routines and focuses that routine', async () => {
  const a = m.rra.failedRoutineToastAction(() => {});
  assert.equal(a.label, 'Open routine');
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /n\.kind === 'routine_failed'\s*\?\s*failedRoutineToastAction\(/, 'the toast uses it');
  assert.doesNotMatch(app, /n\.kind === 'routine_failed'\s*\?\s*undefined/);

  const n = { kind: 'routine_failed', kwant_id: 'k-lead', title: 'Routine "Weekly "big" report" failed' };
  const list = [routine('rt-other', 'Weekly "big" report', { kwant_id: 'k-else' }), routine('rt-1', 'Weekly "big" report'), routine('rt-2', 'Daily')];
  const did = [];
  const id = await m.rra.openFailedRoutine(n, { routines: async () => list, openRoutines: () => did.push('open'), focusRoutine: (x) => did.push(x) });
  assert.equal(id, 'rt-1');
  assert.deepEqual(did, ['open', 'rt-1'], 'Routines opens at once, then the row is focused');
  // Same name twice: the one whose last run failed.
  const twice = [routine('rt-a', 'Daily', { last_status: 'ok' }), routine('rt-b', 'Daily', { last_status: 'error', last_run_at: NOW })];
  assert.equal(m.rra.routineOfNotice({ kwant_id: 'k-lead', title: 'Routine "Daily" failed' }, twice), 'rt-b');
  // No match or no list: Routines still opens, unfocused.
  did.length = 0;
  assert.equal(await m.rra.openFailedRoutine({ kwant_id: 'k-lead', title: 'Routine "Gone" failed' }, { routines: async () => list, openRoutines: () => did.push('open'), focusRoutine: (x) => did.push(x) }), null);
  assert.equal(await m.rra.openFailedRoutine(n, { routines: async () => { throw new Error('offline'); }, openRoutines: () => did.push('open'), focusRoutine: (x) => did.push(x) }), null);
  assert.deepEqual(did, ['open', 'open']);
});

test('Routines: the run focus opens the row’s History, highlights it and focuses Run now (no pause hint)', async () => {
  document.body.innerHTML = '';
  const el = document.createElement('div');
  document.body.appendChild(el);
  const root = createRoot(el);
  const r = routine('rt-1', 'Weekly report', { last_status: 'error' });
  const base = { routine: r, placeLabel: 'direct chat', tick: 0, onEdit: () => {}, onChanged: () => {} };
  root.render(h('ul', null, h(m.RoutineRow, base)));
  await settle();
  const hist = () => [...el.querySelectorAll('button')].find((b) => /History/.test(b.textContent));
  assert.equal(hist().getAttribute('aria-expanded'), 'false');
  root.render(h('ul', null, h(m.RoutineRow, { ...base, focusRun: 1 })));
  await settle();
  assert.equal(hist().getAttribute('aria-expanded'), 'true', 'History open');
  assert.match(el.querySelector('li').className, /border-sky-500\/60/, 'highlighted');
  assert.equal(document.activeElement, el.querySelector('[data-routine-run="rt-1"]'), 'Run now has focus');
  assert.doesNotMatch(el.textContent, /Turn the switch off/);
  root.unmount();
});
