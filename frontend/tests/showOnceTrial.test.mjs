// Show it once, the trial in chat (A29, design §S, api.md §10): after a "Try it once" run ends, the
// "`name` still waits for you" card follows the trial's last message, with Approve, "Edit, then
// approve", Reject and Open in Skills. It is rebuilt from history after a reload and goes away once
// the skill is approved or rejected.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';
import { skill } from './fixtures/showOnce.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
if (!g.CSS) g.CSS = { escape: (s) => s };
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.IntersectionObserver = g.window.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
g.window.HTMLElement.prototype.scrollIntoView = function () {};

let pending = [skill()];
const calls = [];
g.fetch = g.window.fetch = async (url, init = {}) => {
  const method = init.method ?? 'GET';
  calls.push(`${method} ${url}`);
  const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  if (method === 'GET' && url === '/api/skills?status=pending') return json(200, pending);
  if (method === 'POST' && url === '/api/skills/sk_1/approve') return json(200, { ...skill(), status: 'enabled' });
  return json(200, []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { ChatView } from './src/components/ChatView';
  export { emitContinuity } from './src/continuity';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setImmediate(r));
};

const tr = { skill_id: 'sk_1', recording_id: 'rec_1' };
const you = { id: 'u1', sender_role: 'user', sender_id: 'PlayerHero', text: 'Hi' };
const sysline = { id: 's1', sender_role: 'system', sender_id: 'system', direct_kwant_id: 'k_rook', text: 'You started a trial of `export-weekly-orders`. It isn’t approved yet', meta: JSON.stringify({ trial: tr }) };
const result = { id: 'k1', sender_role: 'kwant', sender_id: 'k_rook', text: 'Imported 42 orders.', trial: tr };
const later = { id: 'k2', sender_role: 'kwant', sender_id: 'k_rook', text: 'Anything else?' };

function mount(props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const opened = [];
  const base = {
    title: 'Rook', messages: [], streamingText: '', streamingThinking: '', pendingApprovals: [], isStreaming: false,
    onSendMessage: () => true, onResolveApproval: () => {}, contextKey: 'k_rook', onOpenSkills: (k) => opened.push(k),
  };
  const render = (p) => flushSync(() => root.render(h(m.ChatView, { ...base, ...props, ...p })));
  render({});
  return { host, render, opened, unmount: () => (root.unmount(), host.remove()) };
}
const card = (host) => host.querySelector('section[aria-label$="still waits for you"]');

test('while the trial runs there is no card; the SSE end puts it after the trial’s last message', async () => {
  pending = [skill()];
  const v = mount({ messages: [you, sysline] });
  await settle();
  assert.equal(card(v.host), null);
  m.emitContinuity({ type: 'trial_ended', data: { skill_id: 'sk_1', recording_id: 'rec_1', bot: 'k_rook', outcome: 'done' } });
  await settle();
  const c = card(v.host);
  assert.ok(c, 'the card');
  assert.match(c.textContent, /export-weekly-orders still waits for you\./);
  assert.deepEqual([...c.querySelectorAll('button')].map((b) => b.textContent.trim()), ['Approve', 'Edit, then approve', 'Reject', 'Open in Skills']);
  const sys = v.host.querySelector('[data-message-id="s1"]');
  if (sys) assert.ok(sys.compareDocumentPosition(c) & window.Node.DOCUMENT_POSITION_FOLLOWING, 'after the sysline');
  c.querySelector('button:last-child').click();
  assert.deepEqual(v.opened, ['k_rook']);
  v.unmount();
});

test('after a reload the card is rebuilt from history, after the bot’s result, and Approve hides it', async () => {
  pending = [skill()];
  const v = mount({ messages: [you, sysline, result, later] });
  await settle();
  const c = card(v.host);
  assert.ok(c, 'rebuilt');
  const res = v.host.querySelector('[data-message-id="k1"]');
  const next = v.host.querySelector('[data-message-id="k2"]');
  assert.ok(res && next, 'bot messages carry data-message-id');
  assert.ok(res.compareDocumentPosition(c) & window.Node.DOCUMENT_POSITION_FOLLOWING, 'after the result');
  assert.ok(c.compareDocumentPosition(next) & window.Node.DOCUMENT_POSITION_FOLLOWING, 'before the next message');
  [...c.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Approve').click();
  await settle();
  assert.ok(calls.includes('POST /api/skills/sk_1/approve'));
  m.emitContinuity({ type: 'skill_updated', data: { ...skill(), status: 'enabled' } });
  await settle();
  assert.equal(card(v.host), null, 'approved: gone');
  v.unmount();
});

test('after a reload with api.md §6’s outcome marker the card follows the marked message', async () => {
  pending = [skill()];
  const marked = { ...result, trial: undefined, meta: JSON.stringify({ trial: { ...tr, outcome: 'done' } }) };
  let v = mount({ messages: [you, sysline, marked, later] });
  await settle();
  const c = card(v.host);
  assert.ok(c, 'rebuilt from the outcome');
  assert.ok(v.host.querySelector('[data-message-id="k1"]').compareDocumentPosition(c) & window.Node.DOCUMENT_POSITION_FOLLOWING);
  v.unmount();
  const earlier = { ...marked, id: 'k0' };
  const running = { ...result, id: 'k3', trial: undefined, meta: JSON.stringify({ trial: tr }) };
  v = mount({ messages: [you, sysline, earlier, { ...sysline, id: 's2' }, running] });
  await settle();
  assert.equal(card(v.host), null, 'the second run has no outcome yet: still running');
  v.unmount();
});

test('a skill already approved or rejected gets no card; a rejection elsewhere removes it', async () => {
  pending = [];
  let v = mount({ messages: [you, sysline, result] });
  await settle();
  assert.equal(card(v.host), null, 'not pending any more');
  v.unmount();
  pending = [skill()];
  v = mount({ messages: [you, sysline, result] });
  await settle();
  assert.ok(card(v.host));
  m.emitContinuity({ type: 'skill_updated', data: { ...skill(), status: 'rejected' } });
  await settle();
  assert.equal(card(v.host), null);
  v.unmount();
});

test('S-F7: a trial reply that came back empty names the engine in muted text, not an alert', async () => {
  pending = [skill()];
  const empty = { ...result, text: '', trial: undefined, meta: JSON.stringify({ trial: { ...tr, outcome: 'failed' } }) };
  const rook = { id: 'k_rook', name: 'Rook', title: '', avatar_sprite: '', preferred_model: 'claude/claude-opus-5-5' };
  let v = mount({ messages: [you, sysline, empty], kwants: [rook] });
  await settle();
  const bubble = v.host.querySelector('[data-message-id="k1"]');
  const line = bubble.querySelector('[data-empty-trial-reply]');
  assert.ok(line, 'the empty-reply line');
  assert.equal(line.textContent, "Claude sent back an empty reply, so this try didn't finish. Try it again before you approve.");
  assert.match(line.className, /text-zinc-400/);
  assert.doesNotMatch(line.className, /red/);
  assert.equal(line.closest('[role="alert"]'), null, 'not an alert');
  assert.equal(line.querySelector('svg').getAttribute('aria-hidden'), 'true');
  assert.ok(card(v.host), 'the trial card still follows with Approve');
  v.unmount();

  v = mount({ messages: [you, sysline, empty], kwants: [{ ...rook, preferred_model: 'auto' }] });
  await settle();
  assert.equal(v.host.querySelector('[data-empty-trial-reply]').textContent, "The engine sent back an empty reply, so this try didn't finish. Try it again before you approve.");
  v.unmount();

  v = mount({ messages: [you, sysline, { ...result, text: '   ' }, { ...later, text: '' }], kwants: [{ ...rook, preferred_model: 'codex' }] });
  await settle();
  assert.match(v.host.querySelector('[data-message-id="k1"]').textContent, /Codex sent back an empty reply/);
  assert.equal(v.host.querySelector('[data-message-id="k2"] [data-empty-trial-reply]'), null, 'only trial replies');
  v.unmount();

  v = mount({ messages: [you, sysline, result] });
  await settle();
  assert.equal(v.host.querySelector('[data-empty-trial-reply]'), null, 'a reply with text shows the text');
  assert.match(v.host.querySelector('[data-message-id="k1"]').textContent, /Imported 42 orders\./);
  v.unmount();
});
