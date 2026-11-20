// 30j (brief: autonomy/approval-flash-no-jump/brief.md; Rigel's 30h QA observations 1 and 2): when kwant points
// at a waiting approval, the card must not move as the amber flash starts and ends. Before, the wrapper was
// display: contents idle and a block while flashing, so the chat list's space-y gap (20 px, 24 px from sm) applied
// to it only while flashing and the card hopped 8 / 12 px down and back. And the card root got its own kw-flash from
// ring.ts on top of the wrapper's, so two amber rings were stacked.
// JSDOM has no layout, so this pins what decides the box: the wrapper's classes idle and flashing.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame']) {
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
const base = {
  title: 'Mia', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [approval], onSendMessage() {}, onResolveApproval() {}, kwants: [], teams: [], onHighlightDone() {},
};
const tick = (ms = 10) => new Promise((r) => setTimeout(r, ms));
const classes = (el) => new Set(el.className.split(/\s+/).filter(Boolean));

test('30j: the approval card keeps its box as the flash starts and ends; only kw-flash toggles, and on one element', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.ChatView, base)));
  const card = host.querySelector('[data-approval-card="ap1"]');
  assert.ok(card, 'the approval card is rendered');
  const wrap = card.parentElement;
  const list = wrap.parentElement;
  assert.match(list.className, /\bspace-y-/, 'the wrapper sits in the chat list, whose space-y gap is what moved the card');
  const idle = classes(wrap);
  const cardIdle = classes(card);
  assert.ok(!idle.has('kw-flash'));

  m.flushSync(() => root.render(m.React.createElement(m.ChatView, { ...base, highlightMessageId: 'ap1', highlightRing: true })));
  for (let i = 0; i < 20 && !wrap.classList.contains('kw-flash'); i++) await tick();
  assert.ok(wrap.classList.contains('kw-flash'), 'the wrapper flashes');
  assert.equal(host.querySelector('[data-approval-card="ap1"]'), card, 'same card element: the flash never remounts it (30a)');
  assert.equal(card.parentElement, wrap, 'same wrapper element');
  const on = classes(wrap);

  // Same box idle and flashing: the classes differ by kw-flash and nothing else (no contents <-> block switch).
  const diff = [...new Set([...idle, ...on])].filter((c) => idle.has(c) !== on.has(c));
  assert.deepEqual(diff, ['kw-flash'], `idle "${[...idle].join(' ')}" vs flashing "${[...on].join(' ')}"`);
  // A wrapper with a box in a space-y list would take the 20 / 24 px gap and move the idle card down, so it cancels
  // it: the card's own my-3 stays the gap above it, exactly where it sat idle before.
  assert.ok(!idle.has('contents') && on.has('!mt-0'), 'the wrapper keeps a box and cancels the list gap');
  // One amber ring while flashing (Rigel's observation 2): the wrapper, sized to the card in 30h, not the card too.
  const rings = host.querySelectorAll('.kw-flash');
  assert.equal(rings.length, 1, `${rings.length} .kw-flash elements while flashing`);
  assert.equal(rings[0], wrap);
  assert.deepEqual(classes(card), cardIdle, 'the card root itself is untouched by the flash');
  // Still tagged as an ask, the way message rows are, so tests can tell it from a search landing.
  assert.equal(wrap.getAttribute('data-flash-kind'), 'ask');

  // The flash ends after 2.4 s and the wrapper is back to its idle classes.
  await tick(2500);
  assert.deepEqual(classes(wrap), idle);
  assert.equal(host.querySelectorAll('.kw-flash').length, 0);
  assert.equal(wrap.hasAttribute('data-flash-kind'), false);
  root.unmount();
  host.remove();
});
