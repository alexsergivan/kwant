// Mission board, Phase B3 (docs/mission-control.md §12–§13): AutoFix explains a failure and retries it,
// proposes workflow changes the person approves, and a restore holds the task until the person presses Run.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { TaskCard, TaskDetailBody, InsightsStrip, RestorePreviewForm, RestoreSection, WorkflowChangeBox, FailureBox, HeldBox } from './src/components/MissionBoardView';
  const r = (c, p) => renderToStaticMarkup(React.createElement(c, p));
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
  export const detail = (p) => r(TaskDetailBody, p);
  export const insights = (p) => r(InsightsStrip, p);
  export const preview = (p) => r(RestorePreviewForm, p);
  export const restore = (p) => r(RestoreSection, p);
  export const change = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(WorkflowChangeBox, p)));
  export const failure = (p) => r(FailureBox, p);
  export const held = (p) => r(HeldBox, p);
`);

const NOW = Date.parse('2026-10-27T12:00:00Z');
const noop = () => {};
const names = { k_lin: 'Lin' };
const none = { status: 'not_checked', passed: 0, failed: 0, needs_approval: 0, required_needs_approval: 0, total: 0 };
const task = (over = {}) => ({
  id: 'a', space_id: 's1', mission_id: 'm1', mission_title: 'Pricing page', title: 'Write the copy', acceptance: 'docs/pricing.md exists',
  state: 'failed', priority: 1, owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 40000, spent_tokens: 18000,
  attempts: 1, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1', channel_id: 'c1', direct_kwant_id: null,
  thread_id: null, created_by: 'k_lin', created_at: '2026-10-27T09:00:00Z', updated_at: '2026-10-27T11:00:00Z', finished_at: null,
  depends_on: [], waiting_on: [], claim: null, needs_you: [], checks_summary: none, forked_from: null, flags: [], ...over,
});
const attempt = (over = {}) => ({ id: 'af1', strategy: 'retry_with_diagnostics', status: 'suggested', not_before: null, reason: 'a check failed', tokens_spent: 0, retry_task_id: null, ...over });
const lastFailure = { class: 'check_failed', summary: 'npm test: 2 tests failed', created_at: '2026-10-27T11:50:00Z' };
const failed = (over = {}) => task({ last_failure: lastFailure, autofix: attempt(), recovery_spent_tokens: 3200, recovery_budget_tokens: 10000, ...over });
const detailProps = (t, over = {}) => ({ detail: { task: t, checks: [], evidence: [], events: [] }, names, now: NOW, onAction: noop, onApproveCheck: noop, ...over });
const ev = (kind, detail = {}, actor = 'kwant') => ({ id: kind, task_id: 'a', kind, actor, actor_name: null, from_state: null, to_state: null, detail, created_at: '2026-10-27T11:00:00Z' });

test('failure classes and AutoFix attempts read as plain sentences', () => {
  assert.equal(m.failureClassLabel('auth'), 'Signed out, or the key was rejected');
  assert.equal(m.failureClassLabel('something_new'), 'Something new', 'an unknown class is humanized, not dropped');
  assert.equal(m.autofixStatusText(attempt(), NOW), 'AutoFix suggests: try again with the check’s output. It waits for your OK.');
  assert.match(m.autofixStatusText(attempt({ status: 'scheduled', strategy: 'wait_and_retry', not_before: '2026-10-27T12:05:00Z' }), NOW), /^AutoFix will wait, then try again in /);
  assert.equal(m.autofixStatusText(attempt({ status: 'skipped', reason: 'over_recovery_budget: 9.8k of 10k' }), NOW), 'AutoFix didn’t retry: another try would go over its retry budget.');
  assert.equal(m.autofixStatusText(attempt({ status: 'skipped', reason: 'no_attempts_left' }), NOW), 'AutoFix didn’t retry: no tries left.');
  assert.equal(m.recoveryText(failed()), '3.2k of 10k tokens');
  assert.equal(m.recoveryText(task()), null, 'an older server sends no recovery fields');
});

test('a failed task AutoFix is handling says so on its card and sits in Needs you when it waits for the person', () => {
  const t = failed();
  assert.equal(m.cardStatus(t, names, NOW), 'AutoFix suggests a retry');
  assert.equal(m.columnOfTask(t), m.columnOf('failed'), 'without a needs_you item it stays where failed tasks go');
  assert.equal(m.columnOfTask(failed({ needs_you: [{ kind: 'autofix_suggested', id: 'af1', text: 'Retry?' }] })), 'needs_you');
  assert.equal(m.cardStatus(failed({ autofix: attempt({ status: 'running' }) }), names, NOW), 'AutoFix is retrying');
  assert.equal(m.cardStatus(failed({ autofix: null }), names, NOW), 'A check failed (try 1 of 3)');
  const c = ui.card({ task: failed({ autofix: attempt({ status: 'scheduled' }) }), names, now: NOW, onOpen: noop });
  assert.match(c, /data-autofix-badge[^>]*><span[^>]*>AutoFix</);
  assert.match(c, /text-violet-200[^>]*>AutoFix retries shortly/);
  assert.doesNotMatch(ui.card({ task: failed({ autofix: null }), names, now: NOW, onOpen: noop }), /data-autofix-badge/);
});

test('the task panel shows the failure, the AutoFix plan, its tries and its spend', () => {
  const failures = [
    { id: 'fl0', attempt: 1, source: 'turn', class: 'transient', summary: 'the model timed out', excerpt: null, checkpoint_id: null, created_at: '2026-10-27T11:00:00Z', autofix: attempt({ id: 'af0', status: 'succeeded', strategy: 'wait_and_retry' }) },
    { id: 'fl1', attempt: 2, source: 'check', class: 'check_failed', summary: 'npm test: 2 tests failed', excerpt: 'FAIL pricing.test.js', checkpoint_id: null, created_at: '2026-10-27T11:50:00Z', autofix: attempt() },
  ];
  const html = ui.detail(detailProps(failed({ needs_you: [{ kind: 'autofix_suggested', id: 'af1', text: 'Retry with the check output? npm test: 2 tests failed' }] }), { failures, onRetryAutofix: noop, onSkipAutofix: noop }));
  assert.match(html, /data-failure-class="check_failed"[\s\S]*A check failed[\s\S]*npm test: 2 tests failed/);
  assert.match(html, /data-autofix-status="suggested"/);
  assert.match(html, /data-autofix-retry[^>]*>[\s\S]*Retry now/);
  assert.match(html, /data-autofix-skip[^>]*>Don’t retry/);
  assert.match(html, />AutoFix tries<\/dt><dd[^>]*>1</);
  assert.match(html, />Spent on retries<\/dt><dd[^>]*>3\.2k of 10k tokens</);
  assert.match(html, /Earlier failures \(1\)[\s\S]*Try 1:[\s\S]*the model timed out/);
  assert.match(html, /FAIL pricing\.test\.js/, 'the redacted output is behind a disclosure');
  assert.doesNotMatch(html, /data-needs-you/, 'the AutoFix item is not repeated under Waiting for you');
  const fix = ui.detail(detailProps(task({ last_failure: { ...lastFailure, class: 'auth' }, autofix: null, needs_you: [{ kind: 'fix_needed', id: 'fl2', text: 'Sign in to GitHub again in Connections.' }] })));
  assert.match(fix, /data-fix-needed[^>]*><span[^>]*>To fix: <\/span>Sign in to GitHub again in Connections\./);
  assert.doesNotMatch(fix, /data-autofix-retry/, 'nothing to retry');
  assert.doesNotMatch(ui.detail(detailProps(task())), /data-failure/, 'nothing without a failure');
});

test('AutoFix retries show as AutoFix in the activity, not as the person', () => {
  assert.equal(m.eventText(ev('autofix', { strategy: 'retry_with_diagnostics' })), 'AutoFix sent it back to Ready: try again with the check’s output');
  assert.match(m.eventText(ev('failure', { class: 'check_failed', summary: 'npm test failed' })), /^It failed: a check failed\. npm test failed/);
  assert.equal(m.eventText(ev('released', {}, 'person')), 'You let it run again');
  for (const k of ['autofix', 'autofix_scheduled', 'autofix_started', 'autofix_finished']) assert.doesNotMatch(m.eventText(ev(k, { strategy: 'retry_same', status: 'succeeded' })), /^You\b/, k);
});

test('a proposed workflow change shows before and after, and only Approve applies it', () => {
  const c = {
    id: 'wc1', task_id: 'a', routine_id: null, target: 'check_spec', reason: 'The check runs a script that does not exist',
    before: { check_id: 'c1', kind: 'command', spec: { cmd: 'npm run tset' }, required: true },
    after: { check_id: 'c1', kind: 'command', spec: { cmd: 'npm run tset' }, required: false },
    proposed_by: 'kwant', proposed_by_name: null, status: 'proposed', decided_at: null, created_at: '2026-10-27T11:55:00Z',
  };
  const rows = m.changeRows(c.before, c.after);
  assert.deepEqual(rows.map((r) => [r.label, r.before, r.after, r.changed]).slice(0, 1), [['Required', 'Yes', 'No', true]]);
  assert.ok(!rows.some((r) => r.key === 'check_id'), 'ids are not shown');
  assert.deepEqual(m.changeRows(3, 5).map((r) => [r.before, r.after]), [['3', '5']]);
  const item = { kind: 'workflow_change', id: 'wc1', text: 'Allow this change to a check: …' };
  const html = ui.change({ item, change: c, now: NOW, onDecide: noop });
  assert.match(html, /Proposed change to a check: <\/span>The check runs a script that does not exist/);
  assert.match(html, /AutoFix proposed it/);
  assert.match(html, /data-change-row="required"[\s\S]*Now[\s\S]*Yes[\s\S]*After[\s\S]*No/);
  assert.match(html, /Unchanged \(2\)/);
  assert.match(html, /data-change-approve[^>]*>[\s\S]*Approve and apply/);
  assert.match(html, /data-change-reject[^>]*>Reject/);
  assert.match(html, /Nothing changes until you approve/);
  const loading = ui.change({ item, change: null, onDecide: noop });
  assert.match(loading, /<button[^>]*disabled=""[^>]*data-change-approve/, 'no approving before the details are in');
  const panel = ui.detail(detailProps(task({ state: 'blocked', needs_you: [item] }), { changes: [c], onDecideChange: noop }));
  assert.match(panel, /data-workflow-changes[\s\S]*Change to approve[\s\S]*data-workflow-change="wc1"/);
  assert.equal(m.columnOfTask(task({ state: 'blocked', needs_you: [item] })), 'needs_you');
});

test('a held task says it waits for Run, on the card and in the panel (D10)', () => {
  const t = task({ state: 'ready', held_at: '2026-10-27T11:58:00Z' });
  assert.equal(m.cardStatus(t, names, NOW), 'On hold after restore. Press Run to start it');
  assert.equal(m.cardStatus(task({ state: 'backlog', held_at: '2026-10-27T11:58:00Z' }), names, NOW), 'On hold after restore. Press Run to start it');
  const c = ui.card({ task: t, names, now: NOW, onOpen: noop });
  assert.match(c, /data-held-badge[^>]*><span[^>]*>On hold</);
  assert.match(c, /text-amber-200[^>]*>On hold after restore/);
  const html = ui.detail(detailProps(t, { onRunHeld: noop }));
  assert.match(html, /data-held[\s\S]*On hold after restore[\s\S]*No bot picks it up until you press Run/);
  assert.match(html, /data-run-held[^>]*>[\s\S]*Run</);
  assert.doesNotMatch(ui.detail(detailProps(task({ state: 'ready' }))), /data-held/);
  assert.equal(m.eventText(ev('held', { checkpoint_id: 'cp1' }, 'person')).length > 0, true);
});

test('restore errors and results read plainly', () => {
  const err = (code, status = 409) => new m.MissionError('x', status, code);
  assert.match(m.restoreErrorText(err('stale_preview')), /Files changed since this preview/);
  assert.match(m.restoreErrorText(err('not_held')), /isn’t on hold any more/);
  assert.match(m.restoreErrorText(err('unknown_force_path', 422)), /isn’t in the preview any more/);
  assert.match(m.autofixErrorText(err('over_recovery_budget')), /retry budget/);
  const r = { restored: ['a.md', 'b.md'], deleted: ['c.md'], failed: [], task: task({ state: 'ready', held_at: '2026-10-27T12:00:00Z' }) };
  assert.equal(m.restoredText(r, false), 'Restored: 2 files put back, 1 file removed. It’s on hold until you press Run.');
  assert.equal(m.restoredText({ ...r, task: task({ state: 'ready' }) }, true), 'Restored: 2 files put back, 1 file removed. It will run again.');
});

test('the restore preview offers Restore (hold) by default and Restore and run again; conflicts name no task without proof (D11)', () => {
  const p = {
    checkpoint_id: 'cp1', preview_sha256: 'abc', changes: [{ path: 'docs/pricing.md', action: 'overwrite' }, { path: 'docs/new.md', action: 'delete' }],
    conflicts: [{ path: 'README.md', changed_by_task: null, why: 'changed outside this task' }, { path: 'docs/faq.md', changed_by_task: 'b' }],
    skipped: [{ path: 'big.bin', why: 'too large' }], external_not_undone: [{ tool_name: 'http', summary: 'POST https://api.example.com/publish', at: '2026-10-27T11:30:00Z' }],
    not_undone_note: 'kwant can’t undo these. Check them yourself.', effects_known: false, task_state_after: 'ready', refused: null,
  };
  assert.equal(m.conflictText(p.conflicts[0]), 'Changed outside this task');
  assert.equal(m.conflictText(p.conflicts[1], (id) => (id === 'b' ? 'Write the FAQ' : undefined)), 'Changed by “Write the FAQ”');
  assert.equal(m.conflictText(p.conflicts[1]), 'Changed by another task');
  const html = ui.preview({ preview: p, titleOf: () => 'Write the FAQ', onRestore: noop, onCancel: noop });
  assert.match(html, /Put back[\s\S]*docs\/pricing\.md/);
  assert.match(html, /data-force="README\.md"[\s\S]*Changed outside this task/);
  assert.match(html, /Changed by “Write the FAQ”/);
  assert.match(html, /data-not-undone[\s\S]*kwant can’t undo these[\s\S]*POST https:\/\/api\.example\.com\/publish/);
  assert.match(html, /data-effects-unknown[^>]*>Side effects unknown/);
  assert.match(html, /data-restore[^-][^>]*>[\s\S]*Restore<\/button>/);
  assert.match(html, /data-restore-rerun[^>]*>Restore and run again/);
  assert.match(html, /stays on hold until you press Run/);
  const refused = ui.preview({ preview: { ...p, refused: 'running' }, onRestore: noop });
  assert.match(refused, /data-restore-refused="running"/);
  assert.doesNotMatch(refused, /data-restore-rerun/);
});

test('the restore section shows only with checkpoints, and not on a verified task', () => {
  assert.equal(ui.restore({ task: task({ checkpoint_count: 0 }) }), '');
  assert.match(ui.restore({ task: task({ checkpoint_count: 3 }) }), /data-restore-open[^>]*>[\s\S]*Restore files from a checkpoint \(3\)/);
  assert.match(ui.restore({ task: task({ state: 'verified', checkpoint_count: 3 }) }), /Use “Redo as a copy”/);
  assert.match(ui.restore({ task: task({ state: 'working', checkpoint_count: 3 }) }), /restore once it stops/);
});

test('Insights count AutoFix fixes when the server sends them', () => {
  const base = { verified_7d: 4, verified_by_check_7d: 3, verified_by_person_7d: 1, needs_you: 2, rework_rate_7d: 0.1, tokens_7d: 50000, tokens_per_verified_7d: 12500, open: 5 };
  const old = ui.insights({ insights: base });
  assert.doesNotMatch(old, /Fixed by AutoFix/);
  const html = ui.insights({ insights: { ...base, autofix_7d: { attempts: 5, succeeded: 3, tokens: 8000 }, failures_by_class_7d: { transient: 3, check_failed: 2 } } });
  assert.match(html, /Fixed by AutoFix[\s\S]*3 of 5[\s\S]*8k tokens on retries/);
  assert.match(html, /A temporary error: 3, A check failed: 2/);
  assert.match(html, /lg:grid-cols-6/);
});

test('AutoFix settings parse safely from GET /api/settings', () => {
  assert.equal(m.parseAutofixSettings({}), null, 'an older server: no AutoFix setting shown');
  assert.equal(m.parseAutofixSettings({ autofix: 'maybe' }), null);
  assert.deepEqual(m.parseAutofixSettings({ autofix: 'off', recovery_budget_pct: 25 }), { autofix: 'off', recovery_budget_pct: 25 });
  assert.deepEqual(m.parseAutofixSettings({ autofix: 'on', recovery_budget_pct: 140 }), { autofix: 'on', recovery_budget_pct: 100 });
  assert.deepEqual(m.parseAutofixSettings({ autofix: 'suggest' }), { autofix: 'suggest', recovery_budget_pct: 25 });
});
