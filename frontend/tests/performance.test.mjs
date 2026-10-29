// Insights → Performance (Phase D1, docs/mission-control.md §17): a read-only table of how each bot
// does per kind of work, from GET /api/performance. The verified rate shows its 95% range, groups
// under min_n say "few tasks", an empty report says numbers appear as tasks finish, it says it
// doesn't change who gets work, and a 403 hides the whole section.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const p = await loadBundled(`export * from './src/performance';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { PerformanceBody, PerformancePanel, EMPTY_TEXT, FILTERED_EMPTY_TEXT, REPORT_ONLY_TEXT } from './src/components/PerformancePanel';
  import { PerfError } from './src/performance';
  export { createRoot } from 'react-dom/client';
  export { React, PerformancePanel, PerfError, EMPTY_TEXT, FILTERED_EMPTY_TEXT, REPORT_ONLY_TEXT };
  export const body = (props) => renderToStaticMarkup(React.createElement(PerformanceBody, props));
`);

const group = (over = {}) => ({
  task_class: 'general', kwant_id: 'k_shelly', kwant_name: 'Shelly', engine: 'cli',
  n: 9, verified: 5, failed: 3, unverified: 1, verified_rate: 0.5556, verified_ci95: [0.2666, 0.8112],
  median_latency_ms: 81_234, median_fresh_tokens: 23_110, cost_usd: 1.38, cost_per_task_usd: 0.153,
  interventions_per_task: 0.4444, low_confidence: false, ...over,
});
const report = (groups) => ({ since: '2026-09-29T00:00:00Z', report_only: true, min_n: 5, observations: groups.reduce((n, x) => n + x.n, 0), groups });
const noop = () => {};

test('verified rate reads as "56% (27–81%)" and clamps', () => {
  assert.equal(p.rateText(0.5556, [0.2666, 0.8112]), '56% (27–81%)');
  assert.equal(p.rateText(1, [0.5655, 1]), '100% (57–100%)');
  assert.equal(p.rateText(0, null), '0%');
  assert.equal(p.rateText(1.2, [-0.1, 1.3]), '100% (0–100%)');
  assert.equal(p.rateLabel(0.75, [0.4677, 0.9111]), '75% verified, likely between 47% and 91%');
});

test('time, tokens, cost and per-task numbers', () => {
  assert.equal(p.latencyText(81_234), '1.4 min');
  assert.equal(p.latencyText(400), '1 s');
  assert.equal(p.latencyText(45_000), '45 s');
  assert.equal(p.latencyText(30 * 60_000), '30 min');
  assert.equal(p.latencyText(90 * 60_000), '1.5 h');
  assert.equal(p.latencyText(null), '—');
  assert.equal(p.tokensText(23_110), '23.1k');
  assert.equal(p.tokensText(null), '—');
  assert.equal(p.costText(0.153), '$0.15');
  assert.equal(p.costText(0.004), '<$0.01');
  assert.equal(p.costText(0), '$0');
  assert.equal(p.costText(12.6), '$13');
  assert.equal(p.costText(null), '—');
  assert.equal(p.perTaskText(0.4444), '0.4');
  assert.equal(p.perTaskText(0), '0');
  assert.equal(p.perTaskText(2), '2');
  assert.equal(p.modelText('auto'), 'Not reported');
  assert.equal(p.modelText('claude-opus-5-5'), 'claude-opus-5-5');
  assert.equal(p.botName({ kwant_name: null, kwant_id: 'k_x' }, { k_x: 'Rex' }), 'Rex');
  assert.equal(p.botName({ kwant_name: null, kwant_id: 'k_gone' }), 'Removed bot');
});

test('the URL carries by, since and only the filters that are set; All time is a date', () => {
  assert.equal(p.perfUrl({ by: 'bot', since: '30d' }), '/api/performance?by=bot&since=30d');
  assert.equal(p.perfUrl({ by: 'model', since: '90d', class: 'research', bot: 'k_1' }), '/api/performance?by=model&since=90d&class=research&bot=k_1');
  assert.equal(p.perfUrl({ by: 'bot', since: 'all' }), '/api/performance?by=bot&since=2000-01-01');
});

test('which state shows: 403 hides, other errors show, no groups is empty', () => {
  const E = (s) => new p.PerfError('x', s);
  assert.equal(p.perfView({ report: null, error: E(403), loading: false }), 'hidden');
  assert.equal(p.perfView({ report: null, error: E(500), loading: false }), 'error');
  assert.equal(p.perfView({ report: null, error: null, loading: true }), 'loading');
  assert.equal(p.perfView({ report: report([]), error: null, loading: false }), 'empty');
  assert.equal(p.perfView({ report: report([group()]), error: null, loading: true }), 'rows');
});

test('rows: the range, the numbers, and "few tasks" only on small groups', () => {
  const html = ui.body({ report: report([group(), group({ kwant_id: 'k_rex', kwant_name: 'Rex', n: 3, verified: 2, verified_rate: 0.6667, verified_ci95: [0.2077, 0.9385], low_confidence: true })]), error: null, loading: false, by: 'bot', filtered: false, onRetry: noop });
  assert.match(html, /56% \(27–81%\)/);
  assert.match(html, /67% \(21–94%\)/);
  assert.match(html, /1\.4 min/);
  assert.match(html, /23\.1k/);
  assert.match(html, /\$0\.15/);
  assert.equal((html.match(/data-few-tasks/g) ?? []).length, 2, 'table and phone card for Rex only');
  assert.match(html, /fewer than 5 finished tasks/);
  assert.doesNotMatch(html, /Model</, 'no model column when grouped by bot');
  const byModel = ui.body({ report: report([group({ model: 'auto' })]), error: null, loading: false, by: 'model', filtered: false, onRetry: noop });
  assert.match(byModel, /Model</);
  assert.match(byModel, /Not reported/);
  assert.doesNotMatch(byModel, /data-few-tasks/);
});

test('empty, filtered empty, error and 403 bodies', () => {
  assert.match(ui.body({ report: report([]), error: null, loading: false, by: 'bot', filtered: false, onRetry: noop }), new RegExp(ui.EMPTY_TEXT));
  assert.equal(ui.EMPTY_TEXT, 'No finished tasks yet. Numbers appear as tasks finish.');
  assert.match(ui.body({ report: report([]), error: null, loading: false, by: 'bot', filtered: true, onRetry: noop }), new RegExp(ui.FILTERED_EMPTY_TEXT));
  const err = ui.body({ report: null, error: new ui.PerfError('Server is busy.', 500), loading: false, by: 'bot', filtered: false, onRetry: noop });
  assert.match(err, /role="alert"/);
  assert.match(err, /Couldn’t load performance\. Server is busy\./);
  assert.match(err, />Retry</);
  assert.match(ui.body({ report: null, error: new ui.PerfError('x', 401), loading: false, by: 'bot', filtered: false, onRetry: noop }), /Only you can see these numbers/);
  assert.equal(ui.body({ report: null, error: new ui.PerfError('x', 403), loading: false, by: 'bot', filtered: false, onRetry: noop }), '');
});

const flush = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (load) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  root.render(ui.React.createElement(ui.PerformancePanel, { load }));
  await flush();
  return { host, root };
};

test('the panel says it is report-only, and a 403 hides the whole section', async () => {
  const ok = await mount(async () => report([]));
  assert.ok(ok.host.querySelector('[data-performance]'));
  assert.match(ok.host.textContent, new RegExp(ui.REPORT_ONLY_TEXT));
  assert.match(ok.host.textContent, /No finished tasks yet/);
  ok.root.unmount();
  const denied = await mount(async () => {
    throw new ui.PerfError('Only the person can see this.', 403);
  });
  assert.equal(denied.host.innerHTML, '');
  denied.root.unmount();
});

test('filters reload with by, class, bot and since; choices stay listed after filtering', async () => {
  const calls = [];
  const load = async (q) => {
    calls.push(q);
    return report(q.class ? [group({ task_class: q.class })] : [group(), group({ task_class: 'research', kwant_id: 'k_rex', kwant_name: 'Rex' })]);
  };
  const { host, root } = await mount(load);
  assert.deepEqual(calls[0], { by: 'bot', since: '30d', class: undefined, bot: undefined });
  const [cls, , since] = host.querySelectorAll('select');
  assert.deepEqual([...cls.options].map((o) => o.textContent), ['All kinds of work', 'General', 'research']);
  const change = async (el, v) => {
    el.value = v;
    el.dispatchEvent(new window.Event('change', { bubbles: true }));
    await flush();
  };
  await change(cls, 'research');
  assert.equal(calls.at(-1).class, 'research');
  assert.equal(host.querySelectorAll('select')[0].options.length, 3, 'filtering by a class keeps the others listed');
  await change(since, 'all');
  assert.equal(calls.at(-1).since, 'all');
  const byModel = [...host.querySelectorAll('button[aria-pressed]')].find((b) => b.textContent === 'By model');
  byModel.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await flush();
  assert.equal(calls.at(-1).by, 'model');
  assert.equal(byModel.getAttribute('aria-pressed'), 'true');
  root.unmount();
});

test('the board shows Performance on All work only, and Refresh reloads it', () => {
  const src = readFileSync(new URL('../src/components/MissionBoardView.tsx', import.meta.url), 'utf8');
  assert.match(src, /showPerformance && !mission && !needsToken && <PerformancePanel names=\{names\} refreshKey=\{refreshCount\} \/>/);
  assert.match(src, /setRefreshCount\(\(n\) => n \+ 1\)/);
});
