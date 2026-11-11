// 0.9.0 P1 #511 QA fixes (qa-p1-ui.md, Nov 11): the live MissionWatchControls in jsdom.
// F-A report opens itself when the same check id goes checking -> done; F-B no endless
// "Loading the report…" while a check runs; F-C setup focus; F-D failed stop on desktop;
// F-E failed Undo on the phone; N-1 no colour transition in reduced motion.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1/board', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'localStorage', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const ui = await loadBundled(`
  import React from 'react';
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  import { MissionWatchControls, WatchBoardContext } from './src/components/MissionWatch';
  export { parseWatch, parseReport, parseCandidates } from './src/missionWatch';
  /** A board with one mission card; window.__setWatch plays the SSE update. */
  export const Harness = ({ mission, api, phone, openReportFor }) => {
    const [m, setM] = React.useState(mission);
    window.__setWatch = (w) => setM((x) => ({ ...x, watch: w }));
    const board = { api, phone, timeZone: 'UTC', openReportFor, onChanged: (_id, w) => setM((x) => ({ ...x, watch: w })) };
    return React.createElement(WatchBoardContext.Provider, { value: board },
      React.createElement(MissionWatchControls, { mission: m }, ({ chip, report }) => React.createElement('div', null, chip, report)));
  };
`);
const h = ui.React.createElement;
const settle = async (ms = 40) => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, ms));
};

const counts = (dont_hold, needs_you = 0) => ({ claims: 2, dont_hold, needs_you, cant_tell: 0, holds: 2 - dont_hold });
const claimRows = (dont) => [
  { task_id: 't-1', task_title: 'Write the tests', owner_kwant_id: 'k-lin', owner_name: 'Lin', state: 'done', claimed: 'Done', checked: 'Ran `npm test`', verdict: dont ? 'doesnt_hold' : 'holds', needs_you: false },
  { task_id: 't-2', task_title: 'Ship the docs', owner_kwant_id: 'k-lin', owner_name: 'Lin', state: 'done', claimed: 'Done', checked: 'The page is live', verdict: 'holds', needs_you: false },
];
const rawReport = (id, dont = 1) => ({
  id, mission_id: 'm-1', kwant_id: 'k-vega', kwant_name: 'Vega', trigger: 'stop', status: 'done',
  stopped_at: '2026-11-11T03:00:00Z', started_at: '2026-11-11T03:01:00Z', finished_at: '2026-11-11T03:04:00Z',
  error: null, reason: null, counts: counts(dont), claims: claimRows(dont), actions: [], inbox: null, message_id: 'msg-' + id,
});
const rawWatch = (state, last_check = null) =>
  ui.parseWatch({ state, kwant_id: 'k-vega', kwant_name: 'Vega', set_at: '2026-11-11T02:00:00Z', stopped: false, stopped_at: null, last_check, reason: null, can_check_now: false });
const check = (id, status, c = null) => ({ id, status, trigger: 'stop', started_at: '2026-11-11T03:01:00Z', finished_at: status === 'checking' ? null : '2026-11-11T03:04:00Z', stopped_at: null, counts: c, message_id: null });
const cands = () =>
  ui.parseCandidates([
    { kwant_id: 'k-old', name: 'Old', relation: 'no_task', task_ids: [], can_run: false, reason: { code: 'x', text: 'paused' }, default: false },
    { kwant_id: 'k-vega', name: 'Vega', relation: 'no_task', task_ids: [], can_run: true, reason: null, default: true },
    { kwant_id: 'k-ada', name: 'Ada', relation: 'lead', task_ids: [], can_run: true, reason: null, default: false },
  ]);

function fakeApi(over = {}) {
  const calls = [];
  const api = {
    get: async (id) => (calls.push(`get ${id}`), { watch: null, report: ui.parseReport(rawReport('wc-2')), candidates: cands() }),
    set: async (id, k) => (calls.push(`set ${id} ${k}`), rawWatch('watching')),
    stop: async (id) => (calls.push(`stop ${id}`), rawWatch('off')),
    checkNow: async () => rawWatch('checking', check('wc-3', 'checking')),
    check: async () => null,
    checks: async (id) => (calls.push(`checks ${id}`), []),
    ...over,
  };
  return { api, calls };
}
const mount = (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  root.render(h(ui.Harness, { phone: false, ...props }));
  return { host, root, done: () => (root.unmount(), host.remove()) };
};
const mission = (w) => ({ id: 'm-1', title: 'Launch', status: 'active', watch: w });
const chip = (host) => host.querySelector('[data-watch-chip]');
const click = (el) => el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));

test('F-A: the same check id going checking -> done with a doesn’t-hold opens the report by itself', async () => {
  const { api, calls } = fakeApi();
  const t = mount({ mission: mission(rawWatch('checking', check('wc-2', 'checking'))), api });
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report]').length, 0, 'closed while checking');
  window.__setWatch(rawWatch('dont_hold', check('wc-2', 'done', counts(1))));
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report="wc-2"]').length, 1, 'report wc-2 opened itself');
  assert.ok(calls.includes('get m-1'));
  t.done();
});

test('F-A: a needs-you result opens it too; an all-hold result does not; a later SSE echo does not reopen', async () => {
  const { api } = fakeApi();
  let t = mount({ mission: mission(rawWatch('checking', check('wc-2', 'checking'))), api });
  await settle();
  window.__setWatch(rawWatch('all_hold', check('wc-2', 'done', counts(0, 1))));
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report]').length, 1, 'needs-you opens');
  t.done();

  t = mount({ mission: mission(rawWatch('checking', check('wc-2', 'checking'))), api });
  await settle();
  window.__setWatch(rawWatch('all_hold', check('wc-2', 'done', counts(0, 0))));
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report]').length, 0, 'all-hold stays closed');
  t.done();

  // Opened, hidden by the person, then the same finished check arrives again: stays hidden.
  t = mount({ mission: mission(rawWatch('checking', check('wc-2', 'checking'))), api });
  await settle();
  window.__setWatch(rawWatch('dont_hold', check('wc-2', 'done', counts(1))));
  await settle();
  click(chip(t.host));
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report]').length, 0, 'hidden from the chip');
  window.__setWatch(rawWatch('dont_hold', check('wc-2', 'done', counts(1))));
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report]').length, 0, 'an echo of the same check does not reopen');
  t.done();
});

test('F-B: report opened while the first check runs says there is no report yet (not "Loading" forever)', async () => {
  const { api, calls } = fakeApi();
  const t = mount({ mission: mission(rawWatch('checking', check('wc-2', 'checking'))), api, openReportFor: 'm-1' });
  await settle();
  assert.ok(calls.includes('checks m-1'), 'asks for earlier checks');
  const none = t.host.querySelector('[data-watch-report-none]');
  assert.ok(none, 'plain "no report yet" line');
  assert.match(none.textContent, /^No report yet\. Vega is checking the claims/);
  assert.equal(t.host.querySelector('[data-watch-report-loading]'), null);
  // When the check finishes, its report replaces the line.
  window.__setWatch(rawWatch('dont_hold', check('wc-2', 'done', counts(1))));
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report="wc-2"]').length, 1);
  t.done();
});

test('F-B: report opened while a re-check runs shows the last finished report', async () => {
  const { api } = fakeApi({ checks: async () => [ui.parseReport(rawReport('wc-1'))] });
  const t = mount({ mission: mission(rawWatch('checking', check('wc-2', 'checking'))), api, openReportFor: 'm-1' });
  await settle();
  assert.equal(t.host.querySelectorAll('[data-watch-report="wc-1"]').length, 1, 'last finished report wc-1');
  assert.equal(t.host.querySelector('[data-watch-report-loading]'), null);
  t.done();
});

test('F-C: first open focuses the selected radio once candidates load; save and Esc return focus to the chip', async () => {
  let release;
  const gate = new Promise((r) => (release = r));
  const { api } = fakeApi({ get: async () => (await gate, { watch: null, report: null, candidates: cands() }) });
  const t = mount({ mission: mission(rawWatch('off')), api });
  await settle();
  click(chip(t.host));
  await settle();
  release();
  await settle();
  const a = document.activeElement;
  assert.equal(a.tagName, 'INPUT', `focus is on ${a.tagName} "${a.textContent}"`);
  assert.equal(a.type, 'radio');
  assert.equal(a.checked, true);
  assert.equal(a.value, 'k-vega', 'the default pick');
  // Save: the submit goes through the form.
  t.host.querySelector('[data-watch-setup] form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.equal(t.host.querySelector('[role=dialog]'), null, 'closed after save');
  assert.equal(document.activeElement, chip(t.host), 'focus back on the chip after save');
  // Esc from a reopened panel.
  click(chip(t.host));
  await settle();
  assert.ok(t.host.querySelector('[role=dialog]'));
  t.host.querySelector('[role=dialog]').dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(t.host.querySelector('[role=dialog]'), null);
  assert.equal(document.activeElement, chip(t.host), 'focus back on the chip after Esc');
  t.done();
});

test('F-D: a failed stop on desktop shows the error in the confirm and keeps the watcher', async () => {
  const { api } = fakeApi({ stop: async () => { throw new Error('nope'); } });
  const t = mount({ mission: mission(rawWatch('watching')), api });
  await settle();
  click(chip(t.host));
  await settle();
  click(t.host.querySelector('[data-watch-stop]'));
  await settle();
  click(t.host.querySelector('[data-watch-stop-yes]'));
  await settle();
  const al = [...t.host.querySelectorAll('[role=dialog] [role=alert]')];
  assert.equal(al.length, 1);
  assert.equal(al[0].textContent, 'nope');
  assert.ok(t.host.querySelector('[data-watch-stop-confirm]'), 'confirm stays open');
  assert.match(chip(t.host).textContent, /Watching/);
  t.done();
});

test('F-E: a failed Undo on the phone shows an alert and the chip stays at the real state (off)', async () => {
  const { api, calls } = fakeApi({ set: async (id, k) => { calls.push(`set ${id} ${k}`); throw new Error('Server down.'); } });
  const t = mount({ mission: mission(rawWatch('watching')), api, phone: true });
  await settle();
  click(chip(t.host));
  await settle();
  click(t.host.querySelector('[data-watch-stop]'));
  await settle();
  assert.equal(chip(t.host).textContent, 'Watch');
  click(t.host.querySelector('[data-undo]'));
  await settle();
  assert.ok(calls.includes('set m-1 k-vega'), 'Undo is PUT with the old watcher');
  const al = t.host.querySelector('[data-watch-chip-error][role=alert]');
  assert.ok(al, 'alert under the chip');
  assert.equal(al.textContent, 'Couldn’t turn the watch back on: Server down. The mission isn’t watched now.');
  assert.equal(chip(t.host).textContent, 'Watch');
  // Opening the setup clears the stale alert.
  click(chip(t.host));
  await settle();
  assert.equal(t.host.querySelector('[data-watch-chip-error]'), null);
  t.done();
});

test('N-1: the chip has no colour transition in reduced motion', async () => {
  const { api } = fakeApi();
  const t = mount({ mission: mission(rawWatch('watching')), api });
  await settle();
  const cls = chip(t.host).className.split(/\s+/);
  assert.ok(cls.includes('motion-reduce:transition-none'));
  assert.ok(cls.includes('motion-reduce:duration-0'));
  t.done();
});
