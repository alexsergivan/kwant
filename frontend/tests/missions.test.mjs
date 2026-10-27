// Mission board (Mission Control, Phase A): one board per space with Ready / Working / Needs you /
// Review / Verified. Blocked and Failed are badges, never columns. A task's receipt lists every check
// with its result, and says "Not checked" when nothing has run: Verified is never a bot's word.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { MissionBoardView, TaskCard, TaskDetailBody, EvidenceReceipt, InsightsStrip, MissionSummary } from './src/components/MissionBoardView';
  import { buildReceipt } from './src/missions';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const view = (p) => r(MissionBoardView, p);
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
  export const detail = (p) => r(TaskDetailBody, p);
  export const receipt = (d, names = {}) => r(EvidenceReceipt, { receipt: buildReceipt(d, names), names });
  export const insights = (p) => r(InsightsStrip, p);
  export const mission = (p) => r(MissionSummary, p);
`);

const NOW = Date.parse('2026-10-27T12:00:00Z');
const noop = () => {};
const names = { k_ada: 'Ada', k_lin: 'Lin', k_rigel: 'Rigel' };
const counts = (over = {}) => ({ backlog: 0, ready: 0, working: 0, needs_you: 0, review: 0, verified: 0, blocked: 0, failed: 0, cancelled: 0, ...over });

// Shaped like Task in mission-control/contract.md §4.
const task = (over = {}) => ({
  id: 't1', space_id: 's1', mission_id: 'm1', mission_title: 'Launch the pricing page', title: 'Write the pricing copy',
  acceptance: 'Copy for three plans is in docs/pricing.md and mentions the yearly discount.', state: 'ready', priority: 1,
  owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 50000, spent_tokens: 12400,
  attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1',
  channel_id: 'c1', direct_kwant_id: null, thread_id: null, created_by: 'k_ada',
  created_at: '2026-10-27T09:00:00Z', updated_at: '2026-10-27T11:00:00Z', finished_at: null,
  depends_on: [], waiting_on: [], claim: null, needs_you: [],
  checks_summary: { status: 'not_checked', passed: 0, failed: 0, total: 0 },
  ...over,
});
const check = (over = {}) => ({
  id: 'ch1', task_id: 't1', kind: 'file_contains', spec: { path: 'docs/pricing.md', text: 'yearly' }, required: true, approved: true,
  status: 'passed', output_excerpt: null, checked_by: 'kwant', checked_by_name: null, requested_by: 'k_ada', evidence_id: null,
  created_at: '2026-10-27T10:00:00Z', started_at: '2026-10-27T11:00:00Z', finished_at: '2026-10-27T11:00:05Z', ...over,
});
const evidence = (over = {}) => ({
  id: 'e1', task_id: 't1', kind: 'url', label: 'Preview', uri: 'https://preview.example/pricing', path: null, sha256: null, bytes: null,
  excerpt: null, captured_by: 'k_lin', run_id: null, captured_at: '2026-10-27T11:00:00Z', ...over,
});
const missionRow = (over = {}) => ({
  id: 'm1', space_id: 's1', title: 'Launch the pricing page', goal: 'A live pricing page with three plans', status: 'active',
  lead_kwant_id: 'k_ada', lead_name: 'Ada', channel_id: 'c1', direct_kwant_id: null, thread_id: null,
  budget_tokens: 200000, spent_tokens: 50000, reserved_tokens: 0, over_budget: false, created_by: 'k_ada',
  approved_at: '2026-10-27T09:00:00Z', approved_by: 'person', created_at: '2026-10-27T08:00:00Z', updated_at: '2026-10-27T11:00:00Z',
  finished_at: null, counts: counts({ ready: 1, verified: 1 }), ...over,
});
const insightsRow = { verified_7d: 4, false_success_7d: 1, needs_you_now: 2, blocked_now: 1, interventions_7d: 3, tokens_7d: 480000, tokens_per_verified_7d: 120000 };
const viewProps = (over = {}) => ({
  tasks: [task()], missions: [missionRow()], insights: insightsRow, names, missionId: null, column: 'ready', loading: false, error: null,
  now: NOW, onMission: noop, onColumn: noop, onOpenTask: noop, onApproveMission: noop, onCancelMission: noop, onRefresh: noop, ...over,
});

test('five columns; blocked sits in Ready and failed in Review; cancelled is off the board', () => {
  assert.deepEqual(m.COLUMNS.map((c) => c.label), ['Ready', 'Working', 'Needs you', 'Review', 'Verified']);
  const want = { backlog: 'ready', ready: 'ready', blocked: 'ready', working: 'working', needs_you: 'needs_you', review: 'review', failed: 'review', verified: 'verified', cancelled: null };
  for (const [s, c] of Object.entries(want)) assert.equal(m.columnOf(s), c, s);
  const g = m.groupByColumn([
    task({ id: 'a', priority: 1, updated_at: '2026-10-27T10:00:00Z' }),
    task({ id: 'b', priority: 3 }),
    task({ id: 'c', priority: 1, updated_at: '2026-10-27T11:30:00Z' }),
    task({ id: 'x', state: 'cancelled' }),
  ]);
  assert.deepEqual(g.ready.map((t) => t.id), ['b', 'c', 'a'], 'urgent first, then newest');
  assert.equal(Object.values(g).flat().length, 3);
});

test('the board renders every column, with Blocked and Failed as badges', () => {
  const html = ui.view(viewProps({
    tasks: [task({ id: 'r', state: 'blocked', blocked_reason: 'budget' }), task({ id: 'f', state: 'failed', attempts: 3 }), task({ id: 'v', state: 'verified', finished_at: '2026-10-27T11:00:00Z', checks_summary: { status: 'passed', passed: 1, failed: 0, total: 1 } })],
  }));
  for (const c of ['ready', 'working', 'needs_you', 'review', 'verified']) assert.match(html, new RegExp(`data-column="${c}"`));
  assert.doesNotMatch(html, /data-column="blocked"|data-column="failed"/);
  assert.match(html, />Blocked</);
  assert.match(html, />Failed</);
  assert.match(html, /Out of budget/);
  assert.match(html, /Checks failed 3 of 3 times/);
  // Phone: tabs with counts drive one column at a time.
  assert.match(html, /role="tablist"/);
  assert.match(html, /aria-selected="true"[^>]*>Ready/);
  assert.match(html, /data-empty-column="working"/);
});

test('a card shows owner, cost and an honest check chip', () => {
  const html = ui.card({ task: task({ state: 'review' }), names, now: NOW, onOpen: noop });
  assert.match(html, /Lin/);
  assert.match(html, /12\.4k of 50k tokens/);
  assert.match(html, /Not checked/);
  assert.match(html, /Bot says done\. Not checked yet/);
  const working = ui.card({ task: task({ state: 'working', claim: { kwant_id: 'k_lin', claimed_at: '2026-10-27T11:50:00Z', expires_at: '2026-10-27T12:20:00Z' } }), names, now: NOW, onOpen: noop });
  assert.match(working, /Lin started 10 min ago/);
  const asks = ui.card({ task: task({ state: 'needs_you', needs_you: [{ kind: 'approval', id: 'a1', text: 'OK to publish the page?' }] }), names, now: NOW, onOpen: noop });
  assert.match(asks, /OK to publish the page\?/);
});

test('checksText and costText read plainly', () => {
  assert.deepEqual(m.checksText({ status: 'not_checked', passed: 0, failed: 0, total: 0 }), { text: 'Not checked', tone: 'zinc' });
  assert.equal(m.checksText({ status: 'failed', passed: 1, failed: 2, total: 3 }).text, '2 checks failed');
  assert.equal(m.checksText({ status: 'pending', passed: 1, failed: 0, total: 2 }).text, '1 of 2 checks passed');
  assert.equal(m.checksText({ status: 'passed', passed: 1, failed: 0, total: 1 }).text, 'Check passed');
  assert.equal(m.costText(12400, 50000), '12.4k of 50k tokens');
  assert.equal(m.costText(2_000_000, 0), '2M tokens');
  assert.equal(m.costRatio(80, 50), 1);
});

test('the receipt says "Not checked" when no check has run, and never hides it', () => {
  const d = { task: task({ state: 'review' }), checks: [check({ status: 'pending', finished_at: null, started_at: null, checked_by: null })], evidence: [evidence()], events: [] };
  const r = m.buildReceipt(d, names);
  assert.equal(r.notChecked, true);
  assert.equal(r.verifiedBy, null);
  const html = ui.receipt(d, names);
  assert.match(html, /data-not-checked/);
  assert.match(html, /Not run yet/);
  assert.match(html, /Preview/);
  assert.match(html, /never mark a task verified on their own/);
  const none = ui.receipt({ task: task({ state: 'review' }), checks: [], evidence: [], events: [] });
  assert.match(none, /Not checked/);
  assert.match(none, /Nothing attached/);
});

test('each check shows its result, target, who ran it and output; Verified names the check', () => {
  const out = evidence({ id: 'e2', kind: 'check_output', label: 'output', uri: null, excerpt: 'ok 12 tests' });
  const d = {
    task: task({ state: 'verified', finished_at: '2026-10-27T11:00:05Z' }),
    checks: [
      check({ id: 'c-pass' }),
      check({ id: 'c-cmd', kind: 'command', spec: { command: 'npm test' }, status: 'failed', output_excerpt: '1 failing', finished_at: '2026-10-27T10:00:00Z', evidence_id: 'e2' }),
      check({ id: 'c-own', kind: 'peer_review', spec: { note: 'looks fine' }, checked_by: 'k_lin', checked_by_name: 'Lin', finished_at: '2026-10-27T09:00:00Z' }),
    ],
    evidence: [out, evidence()],
    events: [],
  };
  const r = m.buildReceipt(d, names);
  assert.equal(r.notChecked, false);
  assert.equal(r.verifiedBy.check.id, 'c-pass');
  assert.deepEqual(r.lines.map((l) => l.check.id), ['c-pass', 'c-cmd', 'c-own'], 'newest result first');
  assert.equal(r.lines[2].ownClaim, true, "owner's own review doesn't count");
  assert.deepEqual(r.attachments.map((e) => e.id), ['e1'], 'check output sits on its line, not in attachments');
  const html = ui.receipt(d, names);
  assert.match(html, /data-verified-by/);
  assert.match(html, /docs\/pricing\.md/);
  assert.match(html, /npm test/);
  assert.match(html, /1 failing/);
  assert.match(html, /Checked by kwant/);
  assert.match(html, /doesn’t count toward Verified/);
  assert.doesNotMatch(html, /data-not-checked/);
});

test('a check a bot added waits for the person, with an Allow button', () => {
  const html = ui.detail({
    detail: { task: task({ state: 'review' }), checks: [check({ status: 'needs_approval', approved: false, finished_at: null, checked_by: null, kind: 'command', spec: { command: 'curl -sf https://x.example' } })], evidence: [], events: [] },
    names, now: NOW, onAction: noop, onApproveCheck: noop,
  });
  assert.match(html, /Needs your OK/);
  assert.match(html, /Allow this check/);
});

test('only web links open; file paths and other schemes are shown as text', () => {
  assert.equal(m.safeHttps('https://a.example/x'), 'https://a.example/x');
  assert.equal(m.safeHttps('javascript:alert(1)'), null);
  assert.equal(m.safeHttps('file:///etc/passwd'), null);
  assert.equal(m.safeHttps('/home/maus/out.png'), null);
  const html = ui.receipt({ task: task({ state: 'review' }), checks: [], evidence: [evidence({ id: 'bad', label: 'Sneaky', uri: 'javascript:alert(1)' })], events: [] });
  assert.match(html, /Sneaky/);
  assert.doesNotMatch(html, /href="javascript/);
});

test('detail shows acceptance, owner, cost, tries, live run, source chat and the right actions', () => {
  const t = task({ state: 'working', claim: { kwant_id: 'k_lin', claimed_at: '2026-10-27T11:50:00Z', expires_at: '2026-10-27T12:20:00Z' } });
  const html = ui.detail({ detail: { task: t, checks: [], evidence: [], events: [] }, names, now: NOW, onAction: noop, onApproveCheck: noop, onOpenChat: noop });
  assert.match(html, /What done looks like/);
  assert.match(html, /mentions the yearly discount/);
  assert.match(html, /12\.4k of 50k tokens/);
  assert.match(html, /0 of 3/);
  assert.match(html, /data-live/);
  assert.match(html, /goes back to Ready in 20 min/);
  assert.match(html, /Open the chat it came from/);
  assert.match(html, />Block</);
  assert.match(html, />Cancel task</);
  assert.doesNotMatch(html, /Mark verified/);
  const review = ui.detail({ detail: { task: task({ state: 'review' }), checks: [], evidence: [], events: [] }, names, now: NOW, onAction: noop, onApproveCheck: noop });
  assert.match(review, /Mark verified/);
  assert.match(review, /Send back/);
  assert.match(review, /Run checks/);
  assert.doesNotMatch(review, /Open the chat/, 'no chat link without a handler');
  assert.deepEqual(m.actionsFor({ state: 'failed' }), ['retry', 'cancel']);
  assert.deepEqual(m.actionsFor({ state: 'verified' }), []);
  assert.ok(m.NEEDS_REASON.reject && m.NEEDS_REASON.block, 'send back and block ask why');
});

test('sourceChat points at the channel, thread or direct chat', () => {
  assert.deepEqual(m.sourceChat(task()), { channelId: 'c1', kwantId: null, threadId: null });
  assert.deepEqual(m.sourceChat(task({ channel_id: null, direct_kwant_id: 'k_lin', thread_id: 'th1' })), { channelId: null, kwantId: 'k_lin', threadId: 'th1' });
  assert.equal(m.sourceChat(task({ channel_id: null })), null);
});

test('insights strip: verified this week, false successes, blocked', () => {
  const html = ui.insights({ insights: insightsRow });
  assert.match(html, /Verified this week/);
  assert.match(html, /False successes/);
  assert.match(html, /Blocked now/);
  assert.match(html, /120k/);
  assert.equal(ui.insights({ insights: null }), '');
  assert.match(ui.insights({ insights: { ...insightsRow, tokens_per_verified_7d: null } }), /—/);
});

test('a draft mission waits for the person; over budget says what it means', () => {
  const html = ui.mission({ mission: missionRow({ status: 'draft', approved_at: null }), busy: false, onApprove: noop, onCancel: noop });
  assert.match(html, /Waiting for your OK/);
  assert.match(html, /Nothing runs until you approve it/);
  assert.match(html, /Approve mission/);
  const over = ui.mission({ mission: missionRow({ over_budget: true, spent_tokens: 210000 }), busy: false, onApprove: noop, onCancel: noop });
  assert.match(over, /Over budget/);
  assert.match(over, /raise the budget/);
  // The board lists drafts on "All work".
  assert.match(ui.view(viewProps({ missions: [missionRow({ status: 'draft' })] })), /Approve mission/);
});

test('board states: loading, empty, error, sign-in', () => {
  assert.match(ui.view(viewProps({ tasks: null, loading: true })), /Loading the board/);
  assert.match(ui.view(viewProps({ tasks: [], missions: [] })), /No tasks yet/);
  const err = ui.view(viewProps({ error: 'Could not reach the server.' }));
  assert.match(err, /role="alert"/);
  assert.match(err, /Retry/);
  assert.match(ui.view(viewProps({ needsToken: true })), /Confirm it’s you/);
  assert.match(ui.view(viewProps({ onBack: noop })), /aria-label="Back to chats"/);
});

test('mission filter narrows the board to one mission', () => {
  const html = ui.view(viewProps({ tasks: [task(), task({ id: 't2', mission_id: 'm2', title: 'Other mission task' })], missions: [missionRow(), missionRow({ id: 'm2', title: 'Other' })], missionId: 'm1' }));
  assert.doesNotMatch(html, /Other mission task/);
  assert.match(html, /data-mission="m1"/);
});

test('live events upsert tasks and missions; cancelled tasks drop off', () => {
  const list = [task({ id: 'a' }), task({ id: 'b' })];
  const moved = m.upsertTask(list, task({ id: 'b', state: 'working' }));
  assert.equal(moved.length, 2);
  assert.equal(moved.find((t) => t.id === 'b').state, 'working');
  assert.deepEqual(m.upsertTask(list, task({ id: 'a', state: 'cancelled' })).map((t) => t.id), ['b']);
  assert.equal(m.upsertTask(list, task({ id: 'n' })).length, 3);
  const ms = m.upsertMission([missionRow()], missionRow({ status: 'done' }));
  assert.equal(ms.length, 1);
  assert.equal(ms[0].status, 'done');
  let got = null;
  const off = m.onMission('task_updated', (d) => (got = d.task.id));
  m.emitMission({ type: 'task_updated', data: { task: task({ id: 'live' }) } });
  off();
  m.emitMission({ type: 'task_updated', data: { task: task({ id: 'after' }) } });
  assert.equal(got, 'live');
});

test('the setting is read only when the server has it', () => {
  assert.equal(m.parseMissionBoardSetting({ mission_board: true }), true);
  assert.equal(m.parseMissionBoardSetting({ mission_board: false }), false);
  assert.equal(m.parseMissionBoardSetting({ decide_first: true }), null);
  assert.equal(m.parseMissionBoardSetting(null), null);
});

test('errors read plainly', () => {
  assert.match(m.errorFrom(403, null).message, /Only you can change tasks/);
  assert.match(m.errorFrom(404, null).message, /doesn’t have the Mission board/);
  assert.equal(m.errorFrom(400, { error: 'reason is required' }).message, 'Reason is required.');
});
