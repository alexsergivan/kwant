// 30q (brief: autonomy/quote-jump-restarts/brief.md; Mira's 30n design check, "Small things" 3): a second click on
// a reply's quote while the original is still ringing must ring it again. The quote jump used a single-frame restart
// (clear, then set the id in one requestAnimationFrame), which in Chromium sometimes ran before React committed the
// clear, so nothing restarted. It now uses restartFlash (wait for the clear to commit, then two frames), like 30n.
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

const messages = [
  { id: 'a1', sender_role: 'kwant', sender_id: 'Mia', text: 'The report is in the Files tab.', created_at: '2026-11-19T14:00:00Z', tokens_used: 0 },
  { id: 'u1', sender_role: 'user', sender_id: 'me', text: 'Thanks, opening it.', created_at: '2026-11-19T14:00:05Z', tokens_used: 0,
    reply_to: { id: 'a1', sender_role: 'kwant', sender_id: 'Mia', excerpt: 'The report is in the Files tab.' } },
];
const tick = (ms = 10) => new Promise((r) => setTimeout(r, ms));
const until = async (f) => { for (let i = 0; i < 40 && !f(); i++) await tick(); return f(); };

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.ChatView, {
    title: 'Mia', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
    pendingApprovals: [], onSendMessage() {}, onResolveApproval() {}, kwants: [], teams: [],
  })));
  return { host, done: () => { root.unmount(); host.remove(); } };
}

// 'jsdom': JSDOM's own frames (~16 ms timers), which happen to land after React commits. 'early': the frame runs
// before React commits the cleared flash (a microtask), which is the Chromium race 30n measured (3 of 6 misses).
const realRaf = g.requestAnimationFrame;
for (const frames of ['jsdom', 'early']) test(`30q (${frames} frames): a second click on the same quote during its 1.6 s ring rings the original again and starts a fresh 1.6 s`, async () => {
  g.requestAnimationFrame = frames === 'early' ? (cb) => { queueMicrotask(() => cb(performance.now())); return 0; } : realRaf;
  const { host, done } = mount();
  const quote = host.querySelector('[data-reply-quote="a1"]');
  assert.ok(quote, 'the reply shows its quote');
  const orig = () => host.querySelector('[data-message-id="a1"]');
  const el = orig();
  const ringed = () => orig().classList.contains('kw-flash-quote');

  quote.click();
  assert.ok(await until(ringed), 'the first click rings the original');
  assert.equal(orig().getAttribute('data-flash-kind'), 'quote');
  await tick(1000);
  assert.ok(ringed(), 'still ringing 1.0 s in');

  const seen = [];
  const mo = new MutationObserver(() => seen.push(ringed()));
  mo.observe(el, { attributes: true, attributeFilter: ['class'] });
  host.querySelector('[data-reply-quote="a1"]').click();
  await until(() => seen.length >= 2 && ringed());
  mo.disconnect();
  assert.deepEqual(seen, [false, true], `second click: kw-flash-quote went ${JSON.stringify(seen)} (want off, then on again)`);
  assert.equal(orig(), el, 'same message element');
  assert.equal(host.querySelectorAll('.kw-flash-quote, .kw-flash').length, 1, 'one ring');
  assert.equal(orig().getAttribute('data-flash-kind'), 'quote', 'still the violet quote ring, not amber');

  // A fresh 1.6 s from the second click: still on at 2.0 s after the first click, off by 1.6 s after the second.
  await tick(1000);
  assert.ok(ringed(), 'still ringing 2.0 s after the first click');
  await tick(800);
  assert.ok(!ringed(), 'off 1.6 s after the second click');
  done();
  g.requestAnimationFrame = realRaf;
});
