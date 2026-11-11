// 0.9.0 S-A: a small (low-tier) task that changed a must_not_change path is still approved, and the
// verifier records it as a medium `scope` finding. The task view shows one amber line per such path,
// next to the receipt's "Checked by" lines, and nothing otherwise.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { TaskDetailBody } from './src/components/MissionBoardView';
  export const detail = (p) => renderToStaticMarkup(React.createElement(TaskDetailBody, p));
`);

const NOW = Date.parse('2026-11-11T12:00:00Z');
const noop = () => {};
const names = { k_lin: 'Lin' };
const task = (over = {}) => ({
  id: 't1', space_id: 's1', mission_id: 'm1', mission_title: 'Docs', title: 'Fix the README typo',
  acceptance: 'README says kwant.', state: 'verified', priority: 1,
  owner_kwant_id: 'k_lin', owner_name: 'Lin', suggested_kwant_id: null, budget_tokens: 50000, spent_tokens: 1200,
  attempts: 1, max_attempts: 3, blocked_reason: null, source_kind: 'mission', source_id: 'm1',
  channel_id: 'c1', direct_kwant_id: null, thread_id: null, created_by: 'k_lin',
  created_at: '2026-11-11T09:00:00Z', updated_at: '2026-11-11T11:00:00Z', finished_at: '2026-11-11T11:00:00Z',
  depends_on: [], waiting_on: [], claim: null, needs_you: [],
  checks_summary: { status: 'passed', passed: 1, failed: 0, total: 1 },
  ...over,
});
const finding = (path, over = {}) => ({ id: 'f1', step: 'scope', severity: 'medium', text: `Changed a path it must not change: ${path}`, repro: `diff of ${path}`, ...over });
const verdict = (over = {}) => ({ id: 'v1', task_id: 't1', verdict: 'pass', risk_tier: 'low', findings: [], overturned_at: null, created_at: '2026-11-11T11:00:00Z', ...over });
const render = (warnings) =>
  ui.detail({ detail: { task: task(), checks: [], evidence: [], events: [] }, names, now: NOW, onAction: noop, onApproveCheck: noop, scopeWarnings: warnings });

test('scopeWarnings: one per changed must_not_change path on a low-tier pass', () => {
  const v = verdict({ findings: [finding('docs/api.md'), { id: 'f2', step: 'reproduce', severity: 'medium', text: 'flaky' }, finding('src/db/missions.rs', { id: 'f3' }), finding('docs/api.md', { id: 'f4' })] });
  assert.deepEqual(m.scopeWarnings([v]), [{ id: 'f1', path: 'docs/api.md' }, { id: 'f3', path: 'src/db/missions.rs' }]);
});

test('scopeWarnings: only the newest verdict, and nothing for other tiers, verdicts or shapes', () => {
  const hit = verdict({ findings: [finding('docs/api.md')] });
  assert.deepEqual(m.scopeWarnings([verdict(), hit]), [], 'newest verdict comes first and has none');
  assert.deepEqual(m.scopeWarnings([verdict({ risk_tier: 'medium', findings: [finding('a')] })]), []);
  assert.deepEqual(m.scopeWarnings([verdict({ verdict: 'needs_you', findings: [finding('a')] })]), []);
  assert.deepEqual(m.scopeWarnings([verdict({ overturned_at: '2026-11-11T11:30:00Z', findings: [finding('a')] })]), []);
  assert.deepEqual(m.scopeWarnings([verdict({ findings: [finding('a', { severity: 'high' })] })]), []);
  for (const bad of [undefined, null, {}, [], [null], [verdict({ findings: 'x' })], [verdict({ findings: [null, { step: 'scope', severity: 'medium', text: 3 }] })]]) {
    assert.deepEqual(m.scopeWarnings(bad), []);
  }
});

test('task view: one amber line per scope finding, naming the bot and the path', () => {
  const html = render([{ id: 'f1', path: 'docs/api.md' }, { id: 'f3', path: 'src/db/missions.rs' }]);
  const lines = html.match(/data-scope-warning(?!s)/g) ?? [];
  assert.equal(lines.length, 2);
  assert.match(html, /Lin changed <code[^>]*>docs\/api\.md<\/code>, which this task said must not change\. It was still approved because the task is small\./);
  assert.match(html, /Lin changed <code[^>]*>src\/db\/missions\.rs<\/code>/);
  assert.match(html, /aria-label="Changed paths it must not change"/);
  assert.ok(html.indexOf('data-scope-warnings') > html.indexOf('data-receipt'), 'sits in the receipt');
});

test('task view: nothing when there is no scope finding; paths are text, never HTML', () => {
  assert.doesNotMatch(render([]), /data-scope-warning/);
  assert.doesNotMatch(render(undefined), /data-scope-warning/);
  const html = render([{ id: 'f1', path: '<img src=x>' }]);
  assert.doesNotMatch(html, /<img/);
  assert.match(html, /&lt;img src=x&gt;/);
  assert.doesNotMatch(html, /\p{Extended_Pictographic}/u);
});
