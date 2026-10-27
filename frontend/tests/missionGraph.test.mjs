// Mission board, Phase B1 (docs/mission-control.md §11): a verified task whose inputs may have changed
// gets a "May be invalid" flag. It stays Verified; it waits in Needs you until the person says Still
// valid, Redo or Cancel. Redo on a verified task makes a copy (a fork) that runs on its own.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { MissionBoardView, TaskCard, TaskDetailBody, WorkGraphView, FlagBox } from './src/components/MissionBoardView';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const view = (p) => r(MissionBoardView, p);
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
  export const detail = (p) => r(TaskDetailBody, p);
  export const graph = (p) => r(WorkGraphView, p);
  export const flagBox = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(FlagBox, p)));
`);

const NOW = Date.parse('2026-10-27T12:00:00Z');
const noop = () => {};
const names = { k_lin: 'Lin' };
const passed = { status: 'passed', passed: 1, failed: 0, needs_approval: 0, required_needs_approval: 0, total: 1 };
const task = (over = {}) => ({
  id: 'a', space_id: 's1', mission_id: 'm1', mission_title: 'Pricing page', title: 'Write the copy', acceptance: 'docs/pricing.md exists',
  state: 'verified', priority: 1, owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 0, spent_tokens: 18000,
  attempts: 1, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1', channel_id: 'c1', direct_kwant_id: null,
  thread_id: null, created_by: 'k_lin', created_at: '2026-10-27T09:00:00Z', updated_at: '2026-10-27T11:00:00Z', finished_at: '2026-10-27T11:00:00Z',
  depends_on: [], waiting_on: [], claim: null, needs_you: [], checks_summary: passed, forked_from: null, flags: [], ...over,
});
const flag = (over = {}) => ({
  id: 'f1', kind: 'may_be_invalid', cause: 'evidence_changed', cause_task_id: 'a', cause_title: 'Write the copy',
  detail: { path: 'docs/pricing.md', old_sha256: 'aa', new_sha256: 'bb' }, created_at: '2026-10-27T11:50:00Z', resolved_at: null, resolution: null, ...over,
});
const flagged = (f = flag(), over = {}) => task({ flags: [f], needs_you: [{ kind: 'may_be_invalid', id: f.id, text: 'Check this still holds: …' }], ...over });
const detailProps = (t, over = {}) => ({ detail: { task: t, checks: [], evidence: [], events: [] }, names, now: NOW, onAction: noop, onApproveCheck: noop, onOpenTask: noop, onResolveFlag: noop, onFork: noop, ...over });

test('flag wording covers each cause, a gone file, and a missing title', () => {
  assert.equal(m.flagText(flag()), 'Its file docs/pricing.md changed since it was verified');
  assert.equal(m.flagText(flag({ detail: { path: 'docs/pricing.md', old_sha256: 'aa', new_sha256: null } })), 'Its file docs/pricing.md is gone since it was verified');
  assert.equal(m.flagText(flag({ detail: {} })), 'A file it was verified with changed');
  assert.equal(m.flagText(flag({ cause: 'upstream_evidence_changed' })), 'A file from “Write the copy”, which this builds on, changed');
  assert.equal(m.flagText(flag({ cause: 'upstream_forked' })), '“Write the copy”, which this builds on, is being redone');
  assert.equal(m.flagText(flag({ cause: 'upstream_forked', cause_title: null })), 'a task it builds on, which this builds on, is being redone');
  assert.equal(m.flagText(flag({ cause: 'something_new' })), 'Something it depends on changed');
});

test('open flags come from `flags`, fall back to the needs_you item, and missing fields mean none', () => {
  assert.deepEqual(m.openFlags(task({ flags: undefined, needs_you: [] })), []);
  assert.deepEqual(m.openFlags({ needs_you: [] }), [], 'an older server sends no flags field');
  assert.equal(m.openFlags(task({ flags: [flag({ resolved_at: '2026-10-27T11:55:00Z', resolution: 'still_valid' })] })).length, 0, 'a resolved flag is not open');
  const both = m.openFlags(flagged());
  assert.equal(both.length, 1, 'the needs_you item for a listed flag is not counted twice');
  assert.equal(both[0].cause, 'evidence_changed');
  const only = m.openFlags(task({ flags: undefined, needs_you: [{ kind: 'may_be_invalid', id: 'f9', text: 'Check this still holds: the file changed' }] }));
  assert.deepEqual(only.map((f) => [f.id, f.cause, f.text]), [['f9', null, 'the file changed']]);
});

test('a flagged task sits in Needs you, stays Verified, and the card says why', () => {
  const t = flagged();
  assert.equal(m.columnOf(t.state), 'verified');
  assert.equal(m.columnOfTask(t), 'needs_you');
  assert.equal(m.columnOfTask(task()), 'verified');
  assert.equal(m.columnOfTask(task({ state: 'cancelled', flags: [flag()] })), null, 'cancelled stays off the board');
  const g = m.groupByColumn([t, task({ id: 'b' })]);
  assert.deepEqual(g.needs_you.map((x) => x.id), ['a']);
  assert.deepEqual(g.verified.map((x) => x.id), ['b']);
  const c = ui.card({ task: t, names, now: NOW, onOpen: noop });
  assert.match(c, /data-flag-badge[^>]*><span[^>]*>May be invalid</);
  assert.match(c, />Verified</, 'the card still says Verified');
  assert.match(c, /data-flag-line[\s\S]*Its file docs\/pricing\.md changed since it was verified/);
  const plain = ui.card({ task: task({ flags: undefined }), names, now: NOW, onOpen: noop });
  assert.doesNotMatch(plain, /May be invalid|data-flag-line/);
  const html = ui.view({ tasks: [t], missions: [], insights: null, names, missionId: null, column: 'needs_you', loading: false, error: null, now: NOW, onMission: noop, onColumn: noop, onOpenTask: noop, onApproveMission: noop, onCancelMission: noop, onRefresh: noop });
  assert.doesNotMatch(html.slice(html.indexOf('data-column="needs_you"')).split('data-column="review"')[0], /Nothing needs you/);
});

test('resolve choices follow the server rules: Cancel is off on Verified, Redo off while running', () => {
  const by = (s) => Object.fromEntries(m.resolveOptions(s).map((o) => [o.choice, o]));
  const v = by('verified');
  assert.deepEqual(m.resolveOptions('verified').map((o) => o.label), ['Still valid', 'Redo', 'Cancel']);
  assert.equal(v.still_valid.disabled, null);
  assert.equal(v.redo.disabled, null);
  assert.match(v.redo.hint, /copy of this task that starts running on its own/);
  assert.match(v.cancelled.disabled, /verified task can’t be cancelled/);
  for (const s of ['working', 'needs_you']) {
    assert.match(by(s).redo.disabled, /working on it/, s);
    assert.equal(by(s).cancelled.disabled, null, s);
  }
  assert.match(by('cancelled').redo.disabled, /cancelled/);
  assert.match(by('review').redo.hint, /back to Ready/);
  assert.match(by('blocked').redo.hint, /back to Waiting/);
  assert.match(by('ready').redo.hint, /just closes the flag/);
});

test('the detail shows the flag with three choices; Cancel is disabled on a verified task', () => {
  const html = ui.detail(detailProps(flagged()));
  const box = html.slice(html.indexOf('data-flags'));
  assert.match(box, /May be invalid: <\/span>Its file docs\/pricing\.md changed since it was verified\. It stays Verified until you decide\./);
  assert.match(box, /<button[^>]*data-resolve="still_valid"[^>]*>Still valid</);
  assert.doesNotMatch(box.match(/<button[^>]*data-resolve="still_valid"[^>]*>/)[0], /\sdisabled=""/);
  assert.doesNotMatch(box.match(/<button[^>]*data-resolve="redo"[^>]*>/)[0], /\sdisabled=""/);
  assert.match(box.match(/<button[^>]*data-resolve="cancelled"[^>]*>/)[0], /\sdisabled=""/);
  assert.match(box, /data-resolve-off="cancelled"[^>]*>A verified task can’t be cancelled/);
  assert.match(box, /starts running on its own/);
  // The flag isn't also listed as a question with "Answer in chat".
  assert.doesNotMatch(html, /data-needs-you/);
  assert.doesNotMatch(html, /Question: Check this still holds/);
  // Running: Redo off. Upstream cause: a link to the task it builds on.
  const running = ui.detail(detailProps(flagged(flag({ cause: 'upstream_evidence_changed', cause_task_id: 'up' }), { state: 'working' })));
  assert.match(running.match(/<button[^>]*data-resolve="redo"[^>]*>/)[0], /\sdisabled=""/);
  assert.match(running, /data-flag-cause-link[^>]*>Open “Write the copy”</);
  // Without a resolve handler (e.g. read-only), no buttons.
  const ro = ui.flagBox({ flag: m.openFlags(flagged())[0], state: 'verified' });
  assert.doesNotMatch(ro, /data-resolve=/);
});

test('after resolving: a fork says plainly it runs on its own; 409s read as what changed', () => {
  const fork = task({ id: 'a2', title: 'Write the copy', state: 'backlog', forked_from: 'a' });
  assert.match(m.resolvedText('redo', { task: task(), fork }), /Made a copy to redo it: “Write the copy”\. It starts running on its own; this task stays as it was\./);
  assert.equal(m.resolvedText('redo', { task: task({ state: 'ready' }), fork: null }), 'Flag closed. It will run again.');
  assert.equal(m.resolvedText('still_valid', { task: task(), fork: null }), 'Marked still valid.');
  assert.equal(m.resolvedText('cancelled', { task: task({ state: 'cancelled' }), fork: null }), 'Cancelled.');
  const e409 = (msg) => m.errorFrom(409, { error: msg });
  assert.match(m.flagErrorText(e409('this flag is already resolved')), /already settled, maybe in another tab/);
  assert.match(m.flagErrorText(e409("it's running; resolve when it stops")), /working on it now/);
  assert.match(m.flagErrorText(e409("a verified task can't be cancelled; choose still_valid or redo")), /A verified task can’t be cancelled\. Choose Still valid or Redo\./);
  assert.match(m.flagErrorText(e409('the mission is cancelled')), /mission is cancelled/);
  assert.match(m.flagErrorText(e409('the task is cancelled; fork it to redo it')), /The task is cancelled/);
  assert.match(m.flagErrorText(e409('')), /changed while you were deciding/);
  assert.match(m.flagErrorText(m.errorFrom(400, { error: 'reason is required' })), /Reason is required/);
});

test('fork and resolve send an Idempotency-Key; the same key is reused on a repeat', async () => {
  const calls = [];
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    const body = String(url).includes('/fork') ? task({ id: 'a2', forked_from: 'a', state: 'backlog' }) : { flag: flag({ resolved_at: 'x', resolution: 'redo' }), task: task(), fork: task({ id: 'a2', forked_from: 'a' }) };
    return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
  };
  try {
    const k = m.newIdempotencyKey();
    assert.ok(k.length >= 8 && k.length <= 200);
    assert.notEqual(k, m.newIdempotencyKey());
    const f = await m.missionsApi.fork('a', 'the file changed', k);
    assert.equal(f.forked_from, 'a');
    await m.missionsApi.fork('a', 'the file changed', k);
    await m.missionsApi.resolveFlag('a', 'f1', 'redo', 'key-2');
    const hdr = (c) => new Headers(c.init.headers).get('Idempotency-Key');
    assert.equal(calls[0].url, '/api/tasks/a/fork');
    assert.equal(calls[0].init.method, 'POST');
    assert.deepEqual(JSON.parse(calls[0].init.body), { reason: 'the file changed' });
    assert.equal(hdr(calls[0]), k);
    assert.equal(hdr(calls[1]), k, 'a repeat carries the same key');
    assert.equal(calls[2].url, '/api/tasks/a/flags/f1/resolve');
    assert.deepEqual(JSON.parse(calls[2].init.body), { resolution: 'redo' });
    assert.equal(hdr(calls[2]), 'key-2');
    assert.equal(new Headers(calls[2].init.headers).get('Content-Type'), 'application/json');
  } finally {
    globalThis.fetch = realFetch;
  }
});

test('"Copied from" links to the original; the original lists its copy; Redo as a copy is on Verified only', () => {
  const orig = task();
  const fork = task({ id: 'a2', state: 'ready', forked_from: 'a', finished_at: null });
  const byId = { a: orig, a2: fork };
  const fh = ui.detail(detailProps(fork, { tasksById: byId }));
  assert.match(fh, /data-copied-from[\s\S]*Copied from[\s\S]*<button[^>]*>“Write the copy”<\/button>/);
  const oh = ui.detail(detailProps(orig, { tasksById: byId }));
  assert.match(oh, /data-redone-as="a2"[\s\S]*Redone as[\s\S]*“Write the copy”/);
  assert.match(oh, /data-fork[^>]*>[\s\S]*Redo as a copy</);
  // Original not loaded (another space, or cancelled): still a link, without a title.
  const lone = ui.detail(detailProps(fork, { tasksById: {} }));
  assert.match(lone, /Copied from[\s\S]*>an earlier task</);
  // Missing field: nothing about copies.
  const old = ui.detail(detailProps(task({ forked_from: undefined, state: 'review' })));
  assert.doesNotMatch(old, /data-copies|Redo as a copy/);
  const c = ui.card({ task: fork, names, now: NOW, onOpen: noop });
  assert.match(c, /data-forked-from[^>]*>A copy, made to redo an earlier task</);
});

const node = (over = {}) => ({ id: 'a', title: 'A', state: 'verified', owner_kwant_id: 'k_lin', owner_name: 'Lin', attempts: 1, max_attempts: 3, forked_from: null, downstream: 0, flags: [], checks_summary: passed, ...over });

test('graph steps follow the edges; the cause shows how many further down were not flagged', () => {
  // A → B → C, A → D (not started); A's file changed: A and B flagged, C and D not.
  const g = {
    mission_id: 'm1', scanned_at: '2026-10-27T11:50:00Z', scan_skipped: [],
    nodes: [
      node({ id: 'c', title: 'C' }),
      node({ id: 'b', title: 'B', downstream: 1, flags: [flag({ id: 'fb', cause: 'upstream_evidence_changed', cause_task_id: 'a', cause_title: 'A' })] }),
      node({ id: 'a', title: 'A', downstream: 3, flags: [flag({ cause_title: 'A' })] }),
      node({ id: 'd', title: 'D', state: 'backlog' }),
      node({ id: 'a2', title: 'A', state: 'ready', forked_from: 'a' }),
    ],
    edges: [{ from: 'a', to: 'b' }, { from: 'b', to: 'c' }, { from: 'a', to: 'd' }],
  };
  const lv = m.graphLevels(g);
  assert.deepEqual(lv.map((l) => [l.step, l.rows.map((r) => r.node.id)]), [[1, ['a', 'a2']], [2, ['b', 'd']], [3, ['c']]]);
  const a = lv[0].rows[0];
  assert.equal(a.flaggedBecause, 1);
  assert.equal(a.notFlaggedText, '2 more tasks further down weren’t flagged');
  assert.equal(lv[1].rows[0].notFlaggedText, null, 'B caused no flags');
  assert.deepEqual(lv[2].rows[0].after.map((n) => n.id), ['b']);
  assert.equal(m.graphLevels({ nodes: [node({ downstream: 2, flags: [flag()] }), node({ id: 'b', flags: [flag({ id: 'x', cause: 'upstream_evidence_changed', cause_task_id: 'a' })] })], edges: [{ from: 'a', to: 'b' }] })[0].rows[0].notFlaggedText, '1 more task further down wasn’t flagged');
  // A loop (shouldn't happen) still lists every task.
  const loop = m.graphLevels({ nodes: [node({ id: 'x' }), node({ id: 'y' })], edges: [{ from: 'x', to: 'y' }, { from: 'y', to: 'x' }] });
  assert.equal(loop.flatMap((l) => l.rows).length, 2);

  const html = ui.graph({ graph: g, names, now: NOW, onOpenTask: noop, onScan: noop });
  assert.match(html, /How the tasks connect/);
  assert.match(html, /Files last checked for changes 10 min ago · 2 may be invalid/);
  assert.match(html, /data-graph-step="1"[\s\S]*Step 1 · builds on nothing/);
  assert.match(html, /data-graph-node="c"[\s\S]*?data-graph-after[^>]*>After “B”</);
  assert.match(html, /data-graph-node="a2"[\s\S]*?data-graph-forked[^>]*>Copied from “A”</);
  assert.match(html, /data-graph-node="b"[\s\S]*?data-graph-flag="upstream_evidence_changed"/);
  assert.match(html, /data-graph-not-flagged[^>]*>2 more tasks further down weren’t flagged</);
  assert.match(html, /data-scan[^>]*>[\s\S]*Check files now/);
});

test('graph states: loading, error, empty, never scanned, skipped files', () => {
  assert.match(ui.graph({ graph: null, names, loading: true, onOpenTask: noop }), /Loading…/);
  const err = ui.graph({ graph: null, names, error: 'Couldn’t reach the server.', onOpenTask: noop, onRetry: noop });
  assert.match(err, /role="alert"[\s\S]*Couldn’t load how the tasks connect\. Couldn’t reach the server\.[\s\S]*Retry/);
  const empty = ui.graph({ graph: { mission_id: 'm1', nodes: [], edges: [], scanned_at: null, scan_skipped: [] }, names, onOpenTask: noop });
  assert.match(empty, /data-graph-empty[^>]*>This mission has no tasks yet\./);
  assert.match(empty, /Files haven’t been checked for changes yet/);
  const skipped = ui.graph({ graph: { mission_id: 'm1', nodes: [node()], edges: [], scanned_at: null, scan_skipped: [{ task_id: 'a', path: 'big.bin', why: 'over 50 MB' }] }, names, onOpenTask: noop });
  assert.match(skipped, /1 file wasn’t checked/);
});

test('the board offers "How tasks connect" only with a mission picked', () => {
  const mission = { id: 'm1', space_id: 's1', title: 'Pricing page', goal: '', status: 'active', lead_kwant_id: 'k_lin', lead_name: 'Lin', channel_id: null, direct_kwant_id: null, thread_id: null, budget_tokens: 0, spent_tokens: 0, reserved_tokens: 0, over_budget: false, created_by: 'k_lin', approved_at: null, approved_by: null, created_at: '', updated_at: '', finished_at: null, counts: { backlog: 0, ready: 0, working: 0, needs_you: 0, review: 0, verified: 1, blocked: 0, failed: 0, cancelled: 0 } };
  const base = { tasks: [task()], missions: [mission], insights: null, names, column: 'ready', loading: false, error: null, now: NOW, onMission: noop, onColumn: noop, onOpenTask: noop, onApproveMission: noop, onCancelMission: noop, onRefresh: noop, onView: noop };
  assert.doesNotMatch(ui.view({ ...base, missionId: null }), /data-view-toggle/);
  const board = ui.view({ ...base, missionId: 'm1', view: 'board' });
  assert.match(board, /data-view-toggle[\s\S]*aria-pressed="true"[^>]*>Board</);
  assert.match(board, /data-board="tabs"/);
  const g = ui.view({ ...base, missionId: 'm1', view: 'graph', graph: { mission_id: 'm1', nodes: [node()], edges: [], scanned_at: null, scan_skipped: [] } });
  assert.match(g, /data-graph/);
  assert.doesNotMatch(g, /data-board="tabs"/);
});

test('activity reads flag and fork events plainly', () => {
  const ev = (kind, detail = {}, actor = 'person') => ({ id: 1, task_id: 'a', mission_id: 'm1', kind, actor, actor_name: null, from_state: null, to_state: null, detail, created_at: '' });
  assert.equal(m.eventText(ev('flagged', {}, 'kwant')), 'Flagged: it may no longer hold');
  assert.equal(m.eventText(ev('flag_resolved', { resolution: 'still_valid' })), 'You settled a flag: still valid');
  assert.equal(m.eventText(ev('forked', { reason: 'the file changed' })), 'You made a copy to redo it: the file changed');
});
