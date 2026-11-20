// 30n (brief: autonomy/approval-ring-again/brief.md; Mira's 30j design check, "Small things"): a second
// "Go to the approval" while the card is still flashing must ring it again. Before, the second press set the same
// flash id, React changed nothing, and the first 2.4 s ring just ran out, so the press looked like it did nothing.
// Now the flash is cleared and set again (the quote jump's pattern): kw-flash goes off and back on, the 2.4 s starts
// over, and #878's rule still holds: one .kw-flash element, the same wrapper and card, only kw-flash toggles.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame', 'MutationObserver']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
dom.window.HTMLElement.prototype.scrollIntoView = function () {};
dom.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
if (!('ResizeObserver' in dom.window)) dom.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.ResizeObserver = dom.window.ResizeObserver;
if (!g.CSS) g.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { ChatView } from './src/components/ChatView';
`);

const approval = { id: 'ap1', tool_name: 'execute_shell', details: { source: 'local_shell', tool: 'execute_shell', arguments: { command: 'curl -sI https://example.com', cwd: '/tmp' }, kwant_name: 'Mia' } };
const messages = [
  { id: 'u1', sender_role: 'user', sender_id: 'me', text: 'Check example.com', created_at: '2026-11-19T14:00:00Z', tokens_used: 0 },
  { id: 'a1', sender_role: 'kwant', sender_id: 'Mia', text: 'Checking it now.', created_at: '2026-11-19T14:00:05Z', tokens_used: 0 },
];
const tick = (ms = 10) => new Promise((r) => setTimeout(r, ms));
const until = async (f) => { for (let i = 0; i < 40 && !f(); i++) await tick(); return f(); };

// ChatView as App drives it: a press sets highlightMessageId, and onHighlightDone clears it again.
function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  let hl = null;
  const render = () => root.render(m.React.createElement(m.ChatView, {
    title: 'Mia', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
    pendingApprovals: [approval], onSendMessage() {}, onResolveApproval() {}, kwants: [], teams: [],
    highlightMessageId: hl, highlightRing: hl != null, onHighlightDone: () => { hl = null; render(); },
  }));
  m.flushSync(render);
  const press = () => { hl = 'ap1'; m.flushSync(render); };
  return { host, press, done: () => { root.unmount(); host.remove(); } };
}

test('30n: a second "Go to the approval" during the 2.4 s flash rings the card again and starts a fresh 2.4 s', async () => {
  const { host, press, done } = mount();
  const card = host.querySelector('[data-approval-card="ap1"]');
  const wrap = card.parentElement;
  const idle = wrap.className;

  press();
  assert.ok(await until(() => wrap.classList.contains('kw-flash')), 'the first press flashes the card');
  await tick(1200);
  assert.ok(wrap.classList.contains('kw-flash'), 'still flashing 1.2 s in');

  // Watch the wrapper's class: a fresh ring means kw-flash comes off and goes back on (a CSS animation only
  // restarts that way); the same id set again leaves it untouched.
  const seen = [];
  const mo = new MutationObserver(() => seen.push(wrap.classList.contains('kw-flash')));
  mo.observe(wrap, { attributes: true, attributeFilter: ['class'] });
  press();
  await until(() => seen.length >= 2 && wrap.classList.contains('kw-flash'));
  mo.disconnect();
  assert.deepEqual(seen, [false, true], `second press: kw-flash went ${JSON.stringify(seen)} (want off, then on again)`);

  // #878's no-jump rule still holds after the restart.
  assert.equal(host.querySelector('[data-approval-card="ap1"]'), card, 'same card element (no remount, 30a)');
  assert.equal(card.parentElement, wrap, 'same wrapper element');
  assert.equal(host.querySelectorAll('.kw-flash').length, 1, 'one amber ring');
  assert.equal(wrap.className.replace(/\s*kw-flash\b/, ''), idle, 'only kw-flash toggles');
  assert.equal(wrap.getAttribute('data-flash-kind'), 'ask');

  // A fresh 2.4 s: the first ring would have ended at 2.4 s after the first press (1.2 s from here); it's still on.
  await tick(1500);
  assert.ok(wrap.classList.contains('kw-flash'), 'still flashing 2.7 s after the first press: the second press restarted the 2.4 s');
  await tick(1100);
  assert.equal(host.querySelectorAll('.kw-flash').length, 0, 'and it ends 2.4 s after the second press');
  assert.equal(wrap.className, idle);
  done();
});
