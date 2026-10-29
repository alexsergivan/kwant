// Token limits are opt-in (docs/mission-control.md §15): budget_tokens 0 means no limit, and it's the
// default. With no limit the board shows plain usage and no bar; a limit is set or removed in place
// on the task panel and the mission header; a task stopped by its limit gets "Remove limit and
// continue", from the card and from the panel, which is one PATCH and one unblock.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'SubmitEvent', 'HTMLFormElement', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`export * from './src/missions';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { TaskCard, TaskDetailBody, MissionSummary, LimitEditor } from './src/components/MissionBoardView';
  export { createRoot } from 'react-dom/client';
  export { React, TaskDetailBody, LimitEditor };
  export const card = (p) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(TaskCard, p)));
  export const detail = (p) => renderToStaticMarkup(React.createElement(TaskDetailBody, p));
  export const mission = (p) => renderToStaticMarkup(React.createElement(MissionSummary, p));
`);

const noop = () => {};
const NOW = Date.parse('2026-10-28T12:00:00Z');
const counts = { backlog: 0, ready: 1, working: 0, needs_you: 0, review: 0, verified: 0, blocked: 0, failed: 0, cancelled: 0 };
const task = (over = {}) => ({
  id: 't1', space_id: 's1', mission_id: null, mission_title: null, title: 'Write the launch notes', acceptance: 'Notes are in docs/launch.md.',
  state: 'working', priority: 1, owner_kwant_id: 'k_qa', owner_name: 'QA', suggested_kwant_id: null, budget_tokens: 0, spent_tokens: 64_900,
  attempts: 0, max_attempts: 3, blocked_reason: null, source_kind: 'delegation', source_id: 'd1', channel_id: 'c1', direct_kwant_id: null,
  thread_id: null, created_by: 'k_lead', created_at: '2026-10-28T09:00:00Z', updated_at: '2026-10-28T11:00:00Z', finished_at: null,
  depends_on: [], waiting_on: [], claim: null, needs_you: [], checks_summary: { status: 'not_checked', passed: 0, failed: 0, total: 0 },
  ...over,
});
const outOfBudget = (over = {}) => task({ state: 'blocked', blocked_reason: 'budget', budget_tokens: 50_000, spent_tokens: 186_000, ...over });
const missionRow = (over = {}) => ({
  id: 'm1', space_id: 's1', title: 'Ship the pricing page', goal: 'A live pricing page', status: 'active', lead_kwant_id: 'k_lead', lead_name: 'Lead',
  channel_id: 'c1', direct_kwant_id: null, thread_id: null, budget_tokens: 0, spent_tokens: 420_000, reserved_tokens: 0, over_budget: false,
  created_by: 'k_lead', approved_at: null, approved_by: null, created_at: '2026-10-28T08:00:00Z', updated_at: '2026-10-28T11:00:00Z', finished_at: null,
  counts, ...over,
});
const detailProps = (t, over = {}) => ({ detail: { task: t, checks: [], evidence: [], events: [] }, names: {}, now: NOW, onAction: noop, onApproveCheck: noop, ...over });

test('no limit reads as plain usage; a limit as "of"', () => {
  assert.equal(m.costText(64_900, 0), '64.9k tokens used');
  assert.equal(m.costText(12_400, 50_000), '12.4k of 50k tokens');
  assert.equal(m.costRatio(64_900, 0), 0);
  assert.equal(m.limitLabel(0), 'No limit');
  assert.equal(m.limitLabel(1_000_000), '1M tokens');
  assert.equal(m.outOfBudget(outOfBudget()), true);
  assert.equal(m.outOfBudget(task({ state: 'blocked', blocked_reason: 'waiting on legal' })), false);
  assert.equal(m.cardStatus(outOfBudget()), 'Reached its token limit');
});

test('typed limits: k and M, separators, ranges, and above what it already used', () => {
  assert.equal(m.parseLimit('300k'), 300_000);
  assert.equal(m.parseLimit(' 1.5M '), 1_500_000);
  assert.equal(m.parseLimit('200,000'), 200_000);
  assert.equal(m.parseLimit('75 000 tokens'), 75_000);
  assert.equal(m.parseLimit('lots'), null);
  assert.equal(m.parseLimit('-5'), null);
  assert.equal(m.limitError('300k', 'task'), null);
  assert.match(m.limitError('abc', 'task'), /Enter a number/);
  assert.match(m.limitError('6M', 'task'), /At most 5M/);
  assert.match(m.limitError('500', 'mission'), /At least 1k/);
  assert.equal(m.limitError('500', 'task'), null);
  assert.match(m.limitError('100k', 'task', 186_000), /already used 186k/);
  assert.equal(m.limitError('300k', 'task', 186_000), null);
});

test('Settings defaults: read from the server, sent back as the two settings', () => {
  assert.deepEqual(m.parseDefaultLimits({ mission_default_task_budget_tokens: 0, mission_default_budget_tokens: 0 }), { task: 0, mission: 0 });
  assert.deepEqual(m.parseDefaultLimits({ mission_default_task_budget_tokens: 50_000, mission_default_budget_tokens: 200_000 }), { task: 50_000, mission: 200_000 });
  assert.equal(m.parseDefaultLimits({ mission_default_budget_tokens: 200_000 }), null, 'an older server without the task default');
  assert.deepEqual(m.defaultLimitsBody({ task: 0, mission: 1_000_000 }), { mission_default_task_budget_tokens: 0, mission_default_budget_tokens: 1_000_000 });
});

test('the API: set or remove a limit is one PATCH; continuing is PATCH then unblock', async () => {
  const seen = [];
  const prev = globalThis.fetch;
  globalThis.fetch = async (url, init = {}) => {
    seen.push({ url: String(url), method: init.method ?? 'GET', body: init.body ? JSON.parse(init.body) : null });
    return new Response(JSON.stringify(String(url).includes('/missions/') ? missionRow() : task({ state: 'ready' })), { status: 200, headers: { 'content-type': 'application/json' } });
  };
  try {
    await m.missionsApi.setTaskBudget('t 1', 200_000);
    await m.missionsApi.setTaskBudget('t1', 0);
    await m.missionsApi.setBudget('m1', 0);
    const t = await m.missionsApi.continueWithLimit('t1', 0);
    assert.equal(t.state, 'ready');
  } finally {
    globalThis.fetch = prev;
  }
  assert.deepEqual(seen, [
    { url: '/api/tasks/t%201', method: 'PATCH', body: { budget_tokens: 200_000 } },
    { url: '/api/tasks/t1', method: 'PATCH', body: { budget_tokens: 0 } },
    { url: '/api/missions/m1', method: 'PATCH', body: { budget_tokens: 0 } },
    { url: '/api/tasks/t1', method: 'PATCH', body: { budget_tokens: 0 } },
    { url: '/api/tasks/t1/unblock', method: 'POST', body: {} },
  ]);
});

test('task panel with no limit: usage, no bar, "Set a limit"', () => {
  const html = ui.detail(detailProps(task(), { onSetBudget: noop }));
  assert.match(html, /64\.9k tokens used/);
  assert.doesNotMatch(html, /role="meter"/);
  assert.match(html, /data-limit-set[^>]*>Set a limit/);
  assert.doesNotMatch(html, /rounded-full bg-rose-500/, "no red bar");
});

test('task panel with a limit: the bar, Change and Remove limit', () => {
  const html = ui.detail(detailProps(task({ budget_tokens: 200_000 }), { onSetBudget: noop }));
  assert.match(html, /64\.9k of 200k tokens/);
  assert.match(html, /role="meter"/);
  assert.match(html, />Change</);
  assert.match(html, />Remove limit</);
});

test('out of budget: the panel offers Remove limit and continue and Raise limit, not a bare Unblock', () => {
  const html = ui.detail(detailProps(outOfBudget(), { onSetBudget: noop, onContinueWithLimit: noop }));
  assert.match(html, /stopped at its 50k token limit after using 186k/);
  assert.match(html, /data-remove-limit-continue[^>]*>Remove limit and continue/);
  assert.match(html, /data-raise-limit[^>]*>Raise limit/);
  assert.doesNotMatch(html, />Unblock</);
  assert.match(html, />Cancel task</);
  // Any other block keeps its Unblock.
  assert.match(ui.detail(detailProps(task({ state: 'blocked', blocked_reason: 'waiting on legal' }))), />Unblock</);
});

test('out of budget on the board: the card has the one-click button', () => {
  const html = ui.card({ task: outOfBudget(), names: {}, now: NOW, onOpen: noop, onRemoveLimit: noop });
  assert.match(html, /186k of 50k tokens/);
  assert.match(html, /data-card-remove-limit[^>]*>Remove limit and continue/);
  assert.doesNotMatch(ui.card({ task: task(), names: {}, now: NOW, onOpen: noop, onRemoveLimit: noop }), /Remove limit and continue/);
});

test('mission header: no limit shows usage and Set a limit; a limit shows the bar', () => {
  const none = ui.mission({ mission: missionRow(), busy: false, onApprove: noop, onCancel: noop, onSetBudget: noop });
  assert.match(none, /420k tokens used/);
  assert.doesNotMatch(none, /role="meter"/);
  assert.match(none, />Set a limit</);
  const some = ui.mission({ mission: missionRow({ budget_tokens: 1_000_000 }), busy: false, onApprove: noop, onCancel: noop, onSetBudget: noop });
  assert.match(some, /420k of 1M tokens/);
  assert.match(some, /role="meter"/);
  assert.match(some, />Remove limit</);
});

// ---- Pressing things, in a DOM ----

const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  root.render(el);
  await settle();
  return { host, done: () => (root.unmount(), host.remove()) };
}
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (input, text) => {
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  set.call(input, text);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};

test('Set a limit: a quick pick sets it in one press; Custom takes "300k"', async () => {
  const set = [];
  const v = await mount(ui.React.createElement(ui.LimitEditor, { budget: 0, spent: 64_900, scope: 'task', onSet: (n) => set.push(n) }));
  try {
    await click(v.host.querySelector('[data-limit-set]'));
    const picks = [...v.host.querySelectorAll('[data-limit-pick]')].map((b) => b.textContent);
    assert.deepEqual(picks, ['No limit', '50k', '200k', '1M'], 'with no limit every pick is offered');
    await click(v.host.querySelector('[data-limit-pick="200000"]'));
    assert.deepEqual(set, [200_000]);
    assert.ok(v.host.querySelector('[data-limit-editor="closed"]'), 'it closes after a pick');
    await click(v.host.querySelector('[data-limit-set]'));
    await click(v.host.querySelector('[data-limit-custom]'));
    const input = v.host.querySelector('input');
    await type(input, '300k');
    v.host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    assert.deepEqual(set, [200_000, 300_000]);
  } finally {
    v.done();
  }
});

test('Out of budget: Remove limit and continue sends 0; Raise limit only offers more than it used', async () => {
  const sent = [];
  const v = await mount(ui.React.createElement(ui.TaskDetailBody, detailProps(outOfBudget(), { onContinueWithLimit: (n) => sent.push(n) })));
  try {
    await click(v.host.querySelector('[data-remove-limit-continue]'));
    assert.deepEqual(sent, [0]);
    await click(v.host.querySelector('[data-raise-limit]'));
    const picks = [...v.host.querySelectorAll('[data-limit-pick]')].map((b) => b.getAttribute('data-limit-pick'));
    assert.deepEqual(picks, ['200000', '1000000'], '50k is under the 186k it used, and "No limit" is the main button');
    await click(v.host.querySelector('[data-limit-pick="1000000"]'));
    assert.deepEqual(sent, [0, 1_000_000]);
  } finally {
    v.done();
  }
});

test('Settings hand-off limit per request: no limit is 0, a pick or "300k" is kept, out of range is refused', () => {
  assert.equal(m.requestLimitFrom('0'), 0);
  assert.equal(m.requestLimitFrom('200000'), 200_000);
  assert.equal(m.requestLimitFrom('300k'), 300_000);
  assert.equal(m.requestLimitFrom('100M'), 100_000_000);
  assert.equal(m.requestLimitFrom('101M'), null);
  assert.equal(m.requestLimitFrom(''), null, 'Custom with nothing typed yet');
  assert.equal(m.requestLimitFrom('lots'), null);
  assert.equal(m.limitLabel(0), 'No limit');
});
