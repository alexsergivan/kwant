// 29p deny-with-reason-card (Iris): "Say why" on the approval card. Deny stays one tap; a Deny with words
// carries them as `reason` in the resolve body, from the chat card and from the Inbox/Away card alike.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export * as away from './src/away';
  export * as fx from './src/dev/fixtures';
  export { ModernApprovalCard } from './src/components/ModernApprovalCard';
  export { AwayView } from './src/components/AwayView';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const h = m.createElement;

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(C, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const key = async (el, k) => {
  el.dispatchEvent(new window.KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
  await settle();
};
// A controlled React input: set the DOM value past React's tracker, then fire `input`.
const type = async (input, text) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, text);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const submit = async (form) => {
  form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};

// A GitHub MCP call from a named bot, like mobileCards' pr().
const pr = (over = {}) => ({
  id: 'ap1',
  tool_name: 'mcp__github__create_pull_request',
  details: { source: 'mcp', server: 'github', tool: 'create_pull_request', kwant_name: 'TechLead', arguments: { title: 'Spring sale', repo: 'acme/site' } },
  ...over,
});
const sayWhy = (host) => host.querySelector('[data-approval-say-why]');
const denyBtn = (host) => [...host.querySelectorAll('[data-approval-actions] button')].find((b) => b.textContent.trim() === 'Deny');

test('29p: resolveBody sends reason only for a deny with text; every other body is unchanged', () => {
  const b = m.away.resolveBody;
  assert.deepEqual(b('deny'), { decision: 'deny' });
  assert.deepEqual(b('deny', {}), { decision: 'deny' });
  assert.deepEqual(b('deny', { reason: '   ' }), { decision: 'deny' }, 'blank is a plain Deny');
  assert.deepEqual(b('deny', { reason: '  use the staging repo  ' }), { decision: 'deny', reason: 'use the staging repo' });
  assert.deepEqual(b('allow_once', { reason: 'x' }), { decision: 'allow_once' }, 'never on Allow');
  assert.deepEqual(b('always_allow', { reason: 'x' }), { decision: 'always_allow' });
  assert.deepEqual(b('allow_once', { withoutBackup: true }), { decision: 'allow_once', without_backup: true });
  assert.deepEqual(b('deny', { withoutBackup: true }), { decision: 'deny' });
  assert.equal(b('deny', { reason: 'y'.repeat(501) }).reason.length, 500, 'cut to 500');
  assert.equal(m.away.DENY_REASON_MAX, 500);
});

test('29p: the Inbox/Away request carries the reason, and only on a deny', async () => {
  const realFetch = globalThis.fetch;
  const sent = [];
  globalThis.fetch = async (url, init) => {
    sent.push([url, JSON.parse(init.body)]);
    return new Response('{}', { status: 200 });
  };
  try {
    await m.away.awayApi.resolve('a1', 'deny', false, 'try the docs folder');
    await m.away.awayApi.resolve('a2', 'deny');
    await m.away.awayApi.resolve('a3', 'allow_once', true);
    await m.away.awayApi.resolve('a4', 'allow_once');
  } finally {
    globalThis.fetch = realFetch;
  }
  assert.deepEqual(sent, [
    ['/api/approvals/a1/resolve', { decision: 'deny', reason: 'try the docs folder' }],
    ['/api/approvals/a2/resolve', { decision: 'deny' }],
    ['/api/approvals/a3/resolve', { decision: 'allow_once', without_backup: true }],
    ['/api/approvals/a4/resolve', { decision: 'allow_once' }],
  ]);
});

test('29p: the chat path builds its body with resolveBody (App.tsx handleResolveApproval)', () => {
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const fn = app.slice(app.indexOf('const handleResolveApproval'), app.indexOf('const handleResolveApproval') + 1200);
  assert.match(fn, /body: JSON\.stringify\(resolveBody\(decision, opts\)\)/);
});

test('29p: Deny stays one tap with no reason; Say why is closed at first', async () => {
  const calls = [];
  const host = await mount(m.ModernApprovalCard, { approval: pr(), onResolve: (...a) => calls.push(a) });
  assert.ok(sayWhy(host), 'Say why is offered');
  assert.equal(sayWhy(host).getAttribute('aria-expanded'), 'false');
  assert.equal(host.querySelector('[data-approval-why]'), null, 'the field is closed');
  await click(denyBtn(host));
  assert.deepEqual(calls, [['ap1', 'deny']], 'plain Deny: no opts, no reason');
});

test('29p: Say why opens a focused, labelled field; its button denies and tells the bot', async () => {
  const calls = [];
  const host = await mount(m.ModernApprovalCard, { approval: pr(), onResolve: (...a) => calls.push(a) });
  await click(sayWhy(host));
  assert.equal(sayWhy(host).getAttribute('aria-expanded'), 'true');
  const form = host.querySelector('[data-approval-why]');
  assert.equal(sayWhy(host).getAttribute('aria-controls'), form.id);
  const input = form.querySelector('input');
  assert.equal(document.activeElement, input, 'opening it focuses the field');
  assert.equal(input.getAttribute('aria-label'), 'Tell TechLead why you’re denying this');
  assert.equal(input.getAttribute('placeholder'), 'What should it do instead?');
  assert.equal(input.getAttribute('maxlength'), '500');
  assert.match(form.querySelector('[data-approval-deny-why]').textContent, /^Deny and tell TechLead$/);
  await type(input, '  Open it on the staging repo instead  ');
  await submit(form); // Enter in a one-field form submits it
  assert.deepEqual(calls, [['ap1', 'deny', { reason: 'Open it on the staging repo instead' }]]);
});

test('29p: an empty field sends a plain deny; 501 chars are cut to 500', async () => {
  const calls = [];
  const host = await mount(m.ModernApprovalCard, { approval: pr(), onResolve: (...a) => calls.push(a) });
  await click(sayWhy(host));
  await click(host.querySelector('[data-approval-deny-why]'));
  assert.deepEqual(calls.pop(), ['ap1', 'deny', undefined]);
  const input = host.querySelector('[data-approval-why] input');
  await type(input, 'z'.repeat(501));
  assert.equal(input.value.length, 500, 'the field holds at most 500');
  await click(host.querySelector('[data-approval-deny-why]'));
  assert.equal(calls.pop()[2].reason.length, 500);
});

test('29p: Escape closes the field and gives focus back to Say why', async () => {
  const host = await mount(m.ModernApprovalCard, { approval: pr(), onResolve() {} });
  await click(sayWhy(host));
  await key(host.querySelector('[data-approval-why] input'), 'Escape');
  assert.equal(host.querySelector('[data-approval-why]'), null);
  assert.equal(document.activeElement, sayWhy(host));
  assert.equal(sayWhy(host).getAttribute('aria-expanded'), 'false');
});

test('29p: no bot name says "the bot"', async () => {
  const host = await mount(m.ModernApprovalCard, { approval: { id: 'n1', tool_name: 'write_file', details: { path: '/tmp/a.txt' } }, onResolve() {} });
  await click(sayWhy(host));
  assert.match(host.querySelector('[data-approval-deny-why]').textContent, /^Deny and tell the bot$/);
  assert.equal(host.querySelector('[data-approval-why] input').getAttribute('aria-label'), 'Tell the bot why you’re denying this');
});

test('29p: not on a waiting card, not on the production-floor card', () => {
  const waiting = m.renderToStaticMarkup(h(m.ModernApprovalCard, { approval: pr({ waiting: true }), onResolve() {} }));
  assert.match(waiting, />Deny</);
  assert.doesNotMatch(waiting, /Say why|data-approval-say-why/);
  const floor = m.renderToStaticMarkup(h(m.ModernApprovalCard, { approval: m.fx.FLOOR_GUESS_OLD, onResolve() {}, onLoadBackup: async () => null }));
  assert.doesNotMatch(floor, /Say why|data-approval-say-why/);
});

test('29p: the Inbox/Away card passes the reason through to its resolve', async () => {
  const item = {
    kind: 'approval', id: 'ap1', kwant_id: 'k1', kwant_name: 'TechLead', title: 'Use create_pull_request (github)',
    summary: 'acme/site', status: 'pending', timestamp: '2026-11-05T08:00:00Z',
    link: { thread_id: 't1', channel_id: null, direct_kwant_id: null },
    tool_name: 'mcp__github__create_pull_request', details: pr().details,
  };
  const data = { since: '2026-11-05T07:00:00Z', last_seen: null, now: '2026-11-05T08:05:00Z', stuck_after_minutes: 30, waiting: [item], finished: [], failed: [], unseen: { waiting: 1, finished: 0, failed: 0, total: 1 } };
  const calls = [];
  const noop = () => {};
  const host = await mount(m.AwayView, { data, loading: false, error: null, now: Date.parse('2026-11-05T08:05:00Z'), onRefresh: noop, onOpen: noop, onAnswer: noop, onResolve: (it, d, wb, reason) => calls.push([it.id, d, wb, reason]) });
  await click(sayWhy(host));
  await type(host.querySelector('[data-approval-why] input'), 'Ask me after lunch');
  await submit(host.querySelector('[data-approval-why]'));
  assert.deepEqual(calls, [['ap1', 'deny', undefined, 'Ask me after lunch']]);
});
