// 25e: a bot's settings list every "Always allow" it was given (kept across restarts), and the
// person takes one or all of them back. Lin's routes: GET/DELETE /api/kwants/:id/always-allowed.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { AlwaysAllowedList } from './src/components/ApprovalsSetting';
  export * from './src/alwaysAllowed';
`);
const { React } = m;

const NOW = Date.parse('2026-11-16T18:00:00Z');
const grant = (tool_name, conversation, conversation_kind, conversation_label, minsAgo) => ({
  tool_name, conversation, conversation_kind, conversation_label, created_at: new Date(NOW - minsAgo * 60_000).toISOString(),
});
const three = () => [
  grant('mcp__github__create_pull_request', 'th1', 'thread', 'Weekly report', 5),
  grant('mcp__notes__write_note', 'ch1', 'channel', '#general', 90),
  grant('Bash', 'k1', 'direct', 'your chat', 60 * 30),
];

const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
let calls = [];
let server = () => json(200, []);
g.fetch = g.window.fetch = async (url, init) => {
  calls.push({ url: String(url), method: init?.method ?? 'GET', body: init?.body ? JSON.parse(init.body) : undefined });
  return server(String(url), init);
};

const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
  calls = [];
});
const mount = async (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(m.AlwaysAllowedList, { kwantId: 'k1', kwantName: 'Dev', now: NOW, ...props }));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const rows = (host) => [...host.querySelectorAll('[data-always-allowed-row]')].map((r) => r.textContent.replace(/\s+/g, ' ').trim());
const btn = (host, text) => [...host.querySelectorAll('button')].find((b) => b.textContent.trim() === text);
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('words: the card\'s name for the tool, and where it applies', () => {
  assert.equal(m.grantTool('mcp__github__create_pull_request'), 'Open a pull request on GitHub');
  assert.equal(m.grantTool('Bash'), 'Run a command');
  assert.equal(m.grantTool('Write'), 'Write a file');
  assert.equal(m.grantTool('execute_shell'), 'Run a command on your computer');
  assert.equal(m.grantTool('mcp__notes__write_note'), 'Use write note (notes)');
  assert.equal(m.grantWhere(grant('x', 'th1', 'thread', 'Weekly report', 0)), 'in Weekly report');
  assert.equal(m.grantWhere(grant('x', 'ch1', 'channel', '#general', 0)), 'in #general');
  assert.equal(m.grantWhere(grant('x', 'ch1', 'channel', 'general', 0)), 'in #general');
  assert.equal(m.grantWhere(grant('x', 'k1', 'direct', 'your chat', 0)), 'in your chat');
  assert.equal(m.grantWhere(grant('x', 'gone', 'unknown', 'gone', 0)), 'in a conversation that’s gone');
  assert.equal(m.parseAlwaysAllowed({}), null);
  assert.deepEqual(m.parseAlwaysAllowed([{ tool_name: 'Bash' }, { tool_name: 'Bash', conversation: 'k1', conversation_kind: 'odd' }]).map((r) => r.conversation_kind), ['unknown']);
});

test('no grants: nothing at all, not even a heading', async () => {
  server = () => json(200, []);
  const host = await mount();
  assert.equal(calls[0].url, '/api/kwants/k1/always-allowed');
  assert.equal(host.innerHTML, '');
  const given = await mount({ initial: [] });
  assert.equal(given.innerHTML, '');
});

test('an older server without the list shows nothing', async () => {
  server = () => json(404, { error: 'not found' });
  const host = await mount();
  assert.equal(host.innerHTML, '');
});

test('three grants: one row each, newest first, with Take back each and Take back all', async () => {
  server = () => json(200, three());
  const host = await mount();
  assert.match(host.textContent, /Allowed without asking\s*\(3\)/);
  assert.deepEqual(rows(host), [
    'Open a pull request on GitHubin Weekly report · allowed 5 min agoTake back',
    'Use write note (notes)in #general · allowed 1 h agoTake back',
    'Run a commandin your chat · allowed yesterdayTake back',
  ]);
  assert.equal(host.querySelectorAll('[data-take-back]').length, 3);
  assert.ok(btn(host, 'Take back all'));
  assert.equal(host.querySelector('[data-take-back]').getAttribute('aria-label'), 'Take back: Open a pull request on GitHub in Weekly report');
});

test('one grant: no Take back all', async () => {
  const host = await mount({ initial: three().slice(0, 1) });
  assert.equal(host.querySelectorAll('[data-take-back]').length, 1);
  assert.equal(btn(host, 'Take back all'), undefined);
});

test('Take back sends that one grant and shows what is left; focus stays in the list', async () => {
  const list = three();
  server = (_url, init) => (init?.method === 'DELETE' ? json(200, { revoked: 1, always_allowed: [list[0], list[2]] }) : json(200, list));
  const host = await mount();
  calls = [];
  await click(host.querySelectorAll('[data-take-back]')[1]);
  assert.deepEqual(calls, [{ url: '/api/kwants/k1/always-allowed', method: 'DELETE', body: { conversation: 'ch1', tool_name: 'mcp__notes__write_note' } }]);
  assert.equal(rows(host).length, 2);
  assert.ok(!rows(host).some((r) => r.startsWith('Use write note')), 'the taken-back row is gone');
  assert.match(host.querySelector('[data-always-allowed-done]').textContent, /Taken back\. Dev asks again before it can use write note \(notes\) in #general\./);
  assert.equal(document.activeElement, host.querySelectorAll('[data-take-back]')[1], 'focus moves to the next row');
});

test('Take back all sends {} and leaves only the confirmation, no heading', async () => {
  server = (_url, init) => (init?.method === 'DELETE' ? json(200, { revoked: 3, always_allowed: [] }) : json(200, three()));
  const host = await mount();
  calls = [];
  await click(btn(host, 'Take back all'));
  assert.deepEqual(calls.map((c) => [c.method, c.body]), [['DELETE', {}]]);
  assert.equal(rows(host).length, 0);
  assert.ok(!host.textContent.includes('Allowed without asking'));
  assert.match(host.textContent, /Taken back\. Dev asks again before using any of these\./);
});

test('a failed take-back keeps the row and says why', async () => {
  server = (_url, init) =>
    init?.method === 'DELETE' ? json(500, { error: "it asks again now, but couldn't be removed for good: try again" }) : json(200, three());
  const host = await mount();
  await click(host.querySelector('[data-take-back]'));
  assert.equal(rows(host).length, 3);
  assert.equal(host.querySelector('[role="alert"]').textContent.trim(), "It asks again now, but couldn't be removed for good: try again.");
});

test('a list that fails to load says so, and Try again loads it', async () => {
  let fail = true;
  server = () => (fail ? json(500, {}) : json(200, three()));
  const host = await mount();
  assert.match(host.querySelector('[role="alert"]').textContent, /Couldn’t load what this bot does without asking\./);
  fail = false;
  await click(btn(host, 'Try again'));
  assert.equal(rows(host).length, 3);
});
