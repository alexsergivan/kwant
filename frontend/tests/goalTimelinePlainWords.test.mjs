// 34a goal-timeline-plain-words (Mira's goals UX pass, Minor a, b, c, g): a goal's timeline and its current-work
// line say what happened in plain words. A failed look names the bot and never quotes the raw reason; "fix ready,
// waiting for your OK" only once the server says so (needs_you / fix_ready), not while the bot still looks; the
// first passing check, a break, a notice and a capped notice read as sentences; a quoted report has no markdown.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'localStorage', 'sessionStorage']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const b = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { GoalRow, GoalDetailBody } from './src/components/GoalsStrip';
  export * from './src/goals';
  export { stateLabel, missionStatusLabel } from './src/missions';
  export const rowHtml = (p) => renderToStaticMarkup(React.createElement(GoalRow, { onOpen() {}, ...p }));
  export const detailHtml = (p) => renderToStaticMarkup(React.createElement(GoalDetailBody, { busy: false, error: null, onAction() {}, ...p }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&').replace(/[^\S ]+/g, ' ').trim();
const NOW = Date.parse('2026-11-02T12:00:00Z');
const minsAgo = (m) => new Date(NOW - m * 60_000).toISOString();
const minsAhead = (m) => new Date(NOW + m * 60_000).toISOString();
const names = { k_patch: 'Patch' };
const READY = 'fix ready, waiting for your OK';

const failedCheck = () => ({
  id: 'c1', label: 'Shop home page answers', kind: 'http', spec: { url: 'https://shop.example', expect_status: 200 }, required: true, freshness_minutes: null,
  metric: 'status', operator: '=', target: '200',
  latest: { status: 'failed', verdict: 'not_met', excerpt: 'HTTP 502', observed_value: '502', observed_at: minsAgo(5) },
});
const goal = (over = {}) =>
  b.parseGoal({
    id: 'g1', space_id: 's1', title: 'The shop stays up', description: '', kwant_id: 'k_patch', status: 'not_met', mode: 'suggest',
    interval_minutes: 60, created_by: 'person', last_checked_at: minsAgo(5), next_check_at: minsAhead(55), created_at: minsAgo(600), updated_at: minsAgo(5),
    checks: [failedCheck()], open_mission: null, ...over,
  });
const mission = (over = {}) => ({ id: 'm1', title: 'Restore: The shop stays up', status: 'active', attempt: 1, opened_at: minsAgo(20), run_task_id: 't_run', ...over });
const ev = (id, kind, detail = {}, mins = 0, actor = 'kwant') => ({ id, kind, actor, detail, created_at: minsAgo(mins) });

test('a: a failed look names the goal’s bot and says it stopped without a report', () => {
  const failed = { kind: 'investigated', actor: 'kwant', detail: { failed: true, summary: 'no review after its last attempt' } };
  assert.equal(b.goalEventText(failed, names, { bot: 'Patch' }), 'Patch couldn’t finish looking into it');
  assert.equal(b.goalEventText(failed, names), 'The bot couldn’t finish looking into it', 'no bot: The bot');
  assert.equal(b.goalEventQuote(failed), 'It stopped without a report.', 'never the raw reason');
  const done = { kind: 'investigated', actor: 'kwant', detail: { summary: 'The disk is full.' } };
  assert.equal(b.goalEventText(done, names, { bot: 'Patch' }), 'Patch looked into it');
  assert.equal(b.goalEventQuote(done), 'The disk is full.');
});

test('a: the row’s decision never quotes a failed look’s raw reason as its report', () => {
  const g = goal({ status: 'needs_you', status_reason: 'fix_ready', open_mission: mission({ opened_at: minsAgo(40) }) });
  const events = [ev('e1', 'investigated', { failed: true, summary: 'no review after its last attempt' }, 30)];
  assert.equal(b.goalDiagnosis(g, events, NOW).summary, null);
  assert.equal(b.diagnosisPhase(g, events, NOW), 'failed');
  const shown = { ...g, raw_status: g.status, status: b.displayStatus(g, events, NOW) };
  const row = text(b.rowHtml({ goal: shown, names, now: NOW, onAction() {}, decision: b.goalDecision(shown, events, NOW, null) }));
  assert.doesNotMatch(row, /no review after/, row);
  assert.match(row, /Patch couldn’t finish looking into it/);
  // A later good look is the report again.
  const later = [...events, ev('e2', 'investigated', { summary: 'The disk is full.' }, 10)];
  assert.equal(b.goalDiagnosis(g, later, NOW).summary, 'The disk is full.');
});

test('b: "fix ready" only when the goal needs you with a fix ready, at every place it can show', () => {
  // Still looking: the mission is open and its held Fix task exists from the start.
  const looking = goal({ open_mission: mission() });
  const work = b.currentWorkText(looking);
  assert.ok(work && !work.includes(READY), work);
  assert.equal(work, `Restore: The shop stays up · ${b.missionStatusLabel('active').label}`);
  const ready = goal({ status: 'needs_you', status_reason: 'fix_ready', open_mission: mission() });
  assert.equal(b.currentWorkText(ready), `Restore: The shop stays up · ${READY}`);
  // The display status can differ from the server's; the server's own status decides.
  assert.ok(!b.currentWorkText({ ...looking, raw_status: 'not_met', status: 'needs_you' }).includes(READY));
  assert.ok(b.currentWorkText({ ...ready, raw_status: 'needs_you', status: 'not_met' }).includes(READY));

  // A held fix task with no mission.
  const task = { id: 't_fix', title: 'Fix: shop', state: 'ready', held: true, created_at: minsAgo(10) };
  assert.equal(b.currentWorkText(goal({ fix_tasks: [task] })), `Fix: shop · ${b.stateLabel('ready').label}`);
  assert.equal(b.currentWorkText(goal({ status: 'needs_you', status_reason: 'fix_ready', fix_tasks: [task] })), `Fix: shop · ${READY}`);

  // Row: the fix-task link line.
  const row = text(b.rowHtml({ goal: goal({ status: 'fixing', fix_tasks: [task] }), names, now: NOW, onOpenTask() {} }));
  assert.doesNotMatch(row, /fix ready/, row);
  assert.match(row, new RegExp(`Fix: shop · ${b.stateLabel('ready').label}`));

  // Drawer: the mission list and the fix-task list.
  const detail = (g, fix_tasks = []) => text(b.detailHtml({ detail: b.parseGoalDetail({ goal: g, checks: g.checks, events: [], fix_tasks, missions: g.open_mission ? [g.open_mission] : [] }), names, now: NOW }));
  assert.doesNotMatch(detail(looking), /fix ready/);
  assert.match(detail(ready), /fix ready, waiting for your OK/);
  const taskOnly = goal({ fix_tasks: [task] });
  assert.doesNotMatch(detail(taskOnly, [{ task_id: 't_fix', title: 'Fix: shop', state: 'ready', held: true }]), /fix ready/);
  const taskReady = goal({ status: 'needs_you', status_reason: 'fix_ready', fix_tasks: [task] });
  assert.match(detail(taskReady, [{ task_id: 't_fix', title: 'Fix: shop', state: 'ready', held: true }]), /Fix: shop · fix ready, waiting for your OK/);
});

test('c: first check, break, notices and capped notices read as sentences', () => {
  const e = (kind, detail = {}, opts) => b.goalEventText({ kind, actor: 'kwant', detail }, names, opts);
  assert.equal(e('met', {}, { first: true }), 'First check: OK');
  assert.equal(e('met'), 'OK again');
  assert.equal(e('not_met', { failing: ['HTTP status = 200 (it saw 502)'], confirmed_after: 2 }), 'Broke: HTTP status = 200 (it saw 502)');
  assert.equal(e('not_met', { failing: ['A (it saw 1)', 'B (it saw 2)', 'C (it saw 3)'] }), 'Broke: A (it saw 1) and 2 more');
  assert.equal(e('not_met', { check_label: 'Site answers', fail_streak: 2 }), 'Broke: “Site answers” failed 2 times in a row', 'old events keep their line');
  assert.equal(e('not_met'), 'Broke');
  assert.equal(e('notified', { kind: 'broken' }), 'Told you it broke');
  assert.equal(e('notified', { kind: 'recovered' }), 'Told you it works again');
  assert.equal(e('notified', { kind: 'cant_check' }), 'Told you it can’t be checked');
  assert.equal(e('notified', { kind: 'needs_you' }), 'Told you it needs you');
  assert.equal(e('notified', { kind: 'decision' }), 'Told you a decision is waiting');
  assert.equal(e('notified', { kind: 'something_new' }), 'Told you');
  assert.equal(e('notified'), 'Told you');
  assert.equal(e('notified', { kind: 'constructor' }), 'Told you', 'an odd kind is never a lookup into the object');
  assert.equal(e('notice_capped', { kind: 'broken' }), 'Didn’t ping you (already alerted today)');

  // "First" is the caller's call: no earlier met or not_met, and the start of the story is in view.
  const evs = [ev('e1', 'created', {}, 300, 'person'), ev('e2', 'approved', {}, 290, 'person'), ev('e3', 'met', {}, 280), ev('e4', 'not_met', { failing: ['X (it saw 502)'] }, 60), ev('e5', 'met', {}, 10)];
  assert.deepEqual(b.firstMetIds(evs), new Set(['e3']));
  assert.deepEqual(b.firstMetIds(evs.slice(2)), new Set(), 'a page that starts mid-story claims no first check');
  const failingCheck = [evs[0], ev('c1', 'checked', { verdict: 'not_met' }, 200), ev('f1', 'fix_opened', {}, 100), ev('e9', 'met', {}, 10)];
  assert.deepEqual(b.firstMetIds(failingCheck), new Set(), 'OK after a failing check or a fix is OK again');
  const quiet = [evs[0], evs[1], ev('c2', 'checked', { verdict: 'met' }, 285), ev('p1', 'paused', {}, 284, 'person'), ev('r1', 'resumed', {}, 283, 'person'), ev('e3', 'met', {}, 280)];
  assert.deepEqual(b.firstMetIds(quiet), new Set(['e3']), 'passing checks, pauses and edits before it keep it first');
});

test('g: a quoted report has no markdown and only its first paragraph', () => {
  const summary = '## What is failing\n**HTTP 502** on `/`, the shop is down.\n\n## Likely cause\n- The disk is full.\n- Old logs.';
  const q = b.goalEventQuote({ kind: 'investigated', detail: { summary } });
  assert.equal(q, 'What is failing\nHTTP 502 on `/`, the shop is down.');
  assert.doesNotMatch(q, /#|\*\*/);
  assert.equal(b.goalEventQuote({ kind: 'investigated', detail: { summary: '- one\n* two\n1. three' } }), 'one\ntwo\nthree');
  assert.equal(b.goalEventQuote({ kind: 'fix_failed', detail: { excerpt: '### Tests\n__2 failed__' } }), 'Tests\n2 failed');
  assert.equal(b.goalEventQuote({ kind: 'investigated', detail: { summary: '## \n\n' } }), null, 'nothing left: no quote');
  // A blank line after the heading (Mira, Rigel): the quote keeps the finding, not just the title.
  assert.equal(b.goalEventQuote({ kind: 'investigated', detail: { summary: '## What is failing\n\nThe shop answers 502.\n\n## Likely cause\nDisk' } }), 'What is failing\nThe shop answers 502.');
  assert.equal(b.goalEventQuote({ kind: 'investigated', detail: { summary: '# Investigation report\n\n## What is failing\n\nThe shop answers 502.\nSince 7:38.\n\nMore.' } }), 'Investigation report\nWhat is failing\nThe shop answers 502.\nSince 7:38.');
});

test('the drawer timeline tells the week in plain words, with the bot’s name', () => {
  const g = goal({ open_mission: mission() });
  const events = [
    ev('e1', 'created', {}, 600, 'person'),
    ev('e2', 'approved', {}, 590, 'person'),
    ev('e3', 'met', {}, 580),
    ev('e4', 'not_met', { failing: ['HTTP status = 200 (it saw 502)'], confirmed_after: 2 }, 60),
    ev('e5', 'notified', { kind: 'broken' }, 59),
    ev('e6', 'notice_capped', { kind: 'broken' }, 40),
    ev('e7', 'investigated', { failed: true, summary: 'no review after its last attempt' }, 30),
    ev('e8', 'investigated', { summary: '## What is failing\n**HTTP 502**\n\n## Likely cause\nDisk' }, 20),
  ];
  const html = b.detailHtml({ detail: b.parseGoalDetail({ goal: g, checks: g.checks, events, fix_tasks: [], missions: [g.open_mission] }), names, now: NOW });
  // The timeline only: the decision card above it quotes the whole report, which is not this item.
  const t = text(html.slice(html.indexOf('data-goal-timeline'), html.indexOf('</ol>', html.indexOf('data-goal-timeline'))));
  for (const line of ['First check: OK', 'Broke: HTTP status = 200 (it saw 502)', 'Told you it broke', 'Didn’t ping you (already alerted today)', 'Patch couldn’t finish looking into it', 'It stopped without a report.', 'Patch looked into it', 'What is failing HTTP 502']) {
    assert.ok(t.includes(line), `${line} in: ${t}`);
  }
  for (const raw of ['OK again', 'Notified', 'Notice capped', 'no review after', 'The bot looked', '##', '**', 'Likely cause Disk', READY]) assert.ok(!t.includes(raw), `no ${raw} in: ${t}`);
});
