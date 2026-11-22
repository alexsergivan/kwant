// 33x goal-needs-you-in-inbox (Mira's UX pass at 0.9.40, §3): while a goal sits at Needs you, the Inbox's top
// shows a "Needs you" row naming it (so "You're all caught up" hides), and the board's "Waiting on you" counts it.
// The rule is the strip's own: a goal counts when the strip draws it as Needs you (Not now and a released fix don't).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const b = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { GoalsNeedYou, goalNeedsYouReason } from './src/components/GoalsNeedYou';
  export { GoalsStrip } from './src/components/GoalsStrip';
  export * from './src/goals';
  export { emitMission } from './src/missions';
`);

const src = (f) => readFileSync(new URL(`../src/${f}`, import.meta.url), 'utf8');
const settle = async (ms = 0) => {
  if (ms) await new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};
const NOW = Date.parse('2026-11-22T12:00:00Z');
const minsAgo = (m) => new Date(NOW - m * 60_000).toISOString();
const check = { id: 'c1', label: 'Site answers', kind: 'http', spec: { url: 'https://kwant.si' }, required: true, latest: { status: 'failed', verdict: 'not_met', excerpt: 'HTTP 502', observed_at: minsAgo(10) } };
const goal = (over = {}) =>
  b.parseGoal({
    id: 'g1', space_id: 's1', title: 'The site stays up', kwant_id: 'k_cody', status: 'met', mode: 'suggest', interval_minutes: 60,
    created_by: 'person', created_at: minsAgo(600), updated_at: minsAgo(10), checks: [check], open_mission: null, ...over,
  });
const held = { id: 'm1', title: 'Restore: The site stays up', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: 't_run' };
const fixReady = (over = {}) => goal({ status: 'needs_you', status_reason: 'fix_ready', open_mission: held, ...over });

async function mount(Comp, props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = b.createRoot(host);
  root.render(b.React.createElement(Comp, props));
  await settle();
  return { host, rerender: async (p) => (root.render(b.React.createElement(Comp, p)), await settle()), done: () => (root.unmount(), host.remove()) };
}

test('goalsNeedingYou: the strip’s Needs you, nothing else', () => {
  localStorage.removeItem(b.GOAL_NOT_NOW_KEY);
  const list = [
    fixReady({ id: 'a', title: 'A fix waits' }),
    goal({ id: 'b', title: 'Budget spent', status: 'needs_you', status_reason: 'budget_spent' }),
    goal({ id: 'c', title: 'All good' }),
    goal({ id: 'd', title: 'Failing, bot still looking', status: 'not_met' }),
    goal({ id: 'e', title: 'Chose Not now', status: 'needs_you', status_reason: 'declined' }),
    // Fix it pressed: the run task is released and waits to be claimed. Nothing is asked of the person.
    fixReady({ id: 'f', title: 'Released', open_mission: { ...held, run_task_id: null } }),
    goal({ id: 'g', title: 'A draft', status: 'draft' }),
    goal({ id: 'h', title: 'Paused', status: 'paused' }),
  ];
  assert.deepEqual(b.goalsNeedingYou(list, NOW).map((g) => g.id), ['a', 'b']);
  // Not now on this device (the strip keeps it per goal): no longer a request.
  assert.deepEqual(b.goalsNeedingYou(list, NOW, { a: 't_run' }).map((g) => g.id), ['b']);
  assert.deepEqual(b.goalsNeedingYou([], NOW), []);
});

test('goalNeedsYouReason: why, without repeating Needs you', () => {
  assert.equal(b.goalNeedsYouReason(fixReady()), 'A fix is ready for your OK');
  assert.equal(b.goalNeedsYouReason(goal({ status: 'needs_you', status_reason: 'budget_spent' })), b.needsYouText('budget_spent').replace('Needs you: ', '').replace(/^./, (c) => c.toUpperCase()));
  // A failing goal in suggest mode whose fix is held reads as the same OK.
  assert.equal(b.goalNeedsYouReason(goal({ status: 'not_met', open_mission: held })), 'A fix is ready for your OK');
  assert.equal(b.goalNeedsYouReason(goal({ status: 'needs_you', status_reason: null })), null);
  assert.equal(b.goalNeedsYouReason(goal({ status: 'needs_you', status_reason: 'The key expired' })), 'The key expired');
});

test('GoalsNeedYou: one plain Needs-you row per goal, marked data-needs-you; tap opens the goal on the board', async () => {
  localStorage.removeItem(b.GOAL_NOT_NOW_KEY);
  const opened = [];
  let reads = 0;
  const api = { list: async () => (reads++, [fixReady(), goal({ id: 'g2', title: 'Backups run nightly', status: 'needs_you', status_reason: 'budget_spent' }), goal({ id: 'g3', title: 'Fine' })]) };
  const m = await mount(b.GoalsNeedYou, { api, now: NOW, onOpen: (id) => opened.push(id) });
  const box = m.host.querySelector('section[data-needs-goals]');
  assert.ok(box, 'the block shows');
  assert.ok(box.hasAttribute('data-needs-you'), 'marked so "You’re all caught up" hides');
  assert.equal(box.getAttribute('aria-labelledby'), 'needs-goals-h');
  assert.match(m.host.querySelector('#needs-goals-h').textContent, /Needs you/);
  const rows = [...box.querySelectorAll('button[data-needs-goal]')];
  assert.deepEqual(rows.map((r) => r.dataset.needsGoal), ['g1', 'g2']);
  assert.match(rows[0].textContent, /The site stays up/);
  // The chip says Needs you; the line under the title says why, once.
  assert.equal(rows[0].querySelector('[data-needs-goal-why]').textContent, 'A fix is ready for your OK');
  assert.match(rows[1].textContent, /Backups run nightly/);
  assert.equal(rows[1].textContent.match(/Needs you/g).length, 1);
  // Plain amber chip, no holo sticker (kwant look: holo only where a check earned it).
  assert.ok(rows[0].querySelector('.kw-chip.kw-chip-warn'));
  assert.equal(box.querySelector('[data-sticker], .kw-holo, .holo'), null);
  // 44 px target and a visible focus ring.
  assert.match(rows[0].className, /min-h-11/);
  assert.match(rows[0].className, /focus-visible:ring-2/);
  rows[1].click();
  assert.deepEqual(opened, ['g2']);
  assert.equal(reads, 1);
  m.done();
});

test('GoalsNeedYou: renders nothing while loading, on an error, on a server without Goals, or when none needs you', async () => {
  let release;
  const slow = { list: () => new Promise((r) => (release = r)) };
  const m = await mount(b.GoalsNeedYou, { api: slow, now: NOW, onOpen() {} });
  assert.equal(m.host.innerHTML, '', 'no loading flash');
  release([goal()]);
  await settle();
  assert.equal(m.host.innerHTML, '', 'none needs you: nothing');
  m.done();
  const broken = await mount(b.GoalsNeedYou, { api: { list: async () => { throw new b.GoalError('nope', 500); } }, now: NOW, onOpen() {} });
  assert.equal(broken.host.innerHTML, '', 'error: nothing');
  broken.done();
  const old = await mount(b.GoalsNeedYou, { api: { list: async () => { throw new b.GoalError('no goals', 404); } }, now: NOW, onOpen() {} });
  assert.equal(old.host.innerHTML, '');
  old.done();
});

test('GoalsNeedYou: refreshKey and goal_updated read again; after Fix it the row leaves', async () => {
  localStorage.removeItem(b.GOAL_NOT_NOW_KEY);
  let list = [fixReady()];
  let reads = 0;
  const api = { list: async () => (reads++, list.map((x) => ({ ...x }))) };
  const m = await mount(b.GoalsNeedYou, { api, now: NOW, onOpen() {}, refreshKey: 0 });
  assert.ok(m.host.querySelector('[data-needs-goal="g1"]'));
  list = [goal({ status: 'fixing', open_mission: { ...held, run_task_id: null } })];
  b.emitMission({ type: 'goal_updated', data: { id: 'g1' } });
  b.emitMission({ type: 'goal_updated', data: { id: 'g1' } });
  await settle(300);
  assert.equal(reads, 2, 'a burst of goal_updated is one read');
  assert.equal(m.host.querySelector('[data-needs-you]'), null, 'the row left: the Inbox is back to all caught up');
  list = [fixReady()];
  await m.rerender({ api, now: NOW, onOpen() {}, refreshKey: 1 });
  assert.equal(reads, 3);
  assert.ok(m.host.querySelector('[data-needs-goal="g1"]'));
  m.done();
});

test('opening a goal from the Inbox: the strip opens it on mount when the board wasn’t there yet', async () => {
  // openGoal dispatches on the (jsdom) window; the mission bus stays on Node's own CustomEvent.
  const nodeCustomEvent = globalThis.CustomEvent;
  globalThis.CustomEvent = window.CustomEvent;
  b.openGoalOnBoard('g1');
  globalThis.CustomEvent = nodeCustomEvent;
  const api = { list: async () => [fixReady()], get: async () => b.parseGoalDetail({ goal: fixReady(), checks: [check], events: [], fix_tasks: [], missions: [] }) };
  const m = await mount(b.GoalsStrip, { names: { k_cody: 'Cody' }, now: NOW, api });
  await settle();
  assert.ok(document.querySelector('[role="dialog"]'), 'the goal’s drawer is open');
  assert.match(document.querySelector('[role="dialog"]').textContent, /The site stays up/);
  assert.equal(b.takePendingGoal(), null, 'used once');
  m.done();
});

test('GoalsStrip reports its Needs-you count; the board adds it to Waiting on you', async () => {
  localStorage.removeItem(b.GOAL_NOT_NOW_KEY);
  const seen = [];
  const api = { list: async () => [fixReady(), goal({ id: 'g2' })], get: async (id) => b.parseGoalDetail({ goal: fixReady({ id }), checks: [check], events: [], fix_tasks: [], missions: [] }) };
  const m = await mount(b.GoalsStrip, { names: {}, now: NOW, api, onNeedsYou: (n) => seen.push(n) });
  await settle();
  assert.equal(seen.at(-1), 1);
  m.done();
  const board = src('components/MissionBoardView.tsx');
  assert.match(board, /needsYou=\{needsYouCount\(tasks\) \+ goalsNeedYou\}/);
  assert.match(board, /onNeedsYou=\{setGoalsNeedYou\}/);
  assert.match(board, /Tasks and goals waiting on a question, approval or step from you\./);
  const app = src('App.tsx');
  assert.match(app, /<GoalsNeedYou[^>]*onOpen=/, 'mounted in the Inbox top slot');
  assert.ok(app.indexOf('<GoalsNeedYou') > app.indexOf('<SpendStopNeedsYou') && app.indexOf('<GoalsNeedYou') < app.indexOf('<BudgetNeedsYou'), 'next to the other Needs-you blocks');
});
