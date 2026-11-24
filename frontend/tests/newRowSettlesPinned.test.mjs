// chat-gap-scroll (Oleks, Nov 24: "chat windows scrolls up for some reason sometimes"): with no reply streaming, a new
// message lands, the chat pins to the bottom while that row still has its kw-row-lazy estimate (160 px, it's below the
// screen at that moment), and a frame later it draws at its real height. The row observer is new for that render, so
// its first report is already the real height, and a row's first report never re-pinned: the newest message stayed
// the difference (153 px in the Chrome repro) below the view, which reads as the chat having scrolled up.
// JSDOM has no layout, so the test gives the rows and the scroller their sizes and plays the observer's reports.
// Red at main d35c8a52.
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
// Every observer, with what it watches, so the test can deliver the browser's reports itself.
const observers = [];
class FakeRO {
  constructor(cb) { this.cb = cb; this.targets = new Set(); observers.push(this); }
  observe(t) { this.targets.add(t); }
  unobserve(t) { this.targets.delete(t); }
  disconnect() { this.targets.clear(); }
}
dom.window.ResizeObserver = FakeRO;
g.ResizeObserver = FakeRO;
if (!g.CSS) g.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };
// Rows are drawn at the lazy estimate until the test says otherwise.
const rowHeight = new WeakMap();
const realRect = dom.window.HTMLElement.prototype.getBoundingClientRect;
dom.window.HTMLElement.prototype.getBoundingClientRect = function () {
  if (!this.hasAttribute('data-message-id')) return realRect.call(this);
  const h = rowHeight.get(this) ?? 160;
  return { top: 0, left: 0, right: 600, bottom: h, width: 600, height: h, x: 0, y: 0, toJSON() {} };
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { ChatView } from './src/components/ChatView';
`);

const at = (i) => new Date(Date.parse('2026-11-24T16:00:00Z') + i * 60_000).toISOString();
const msg = (id, i) => ({ id, sender_role: i % 2 ? 'kwant' : 'user', sender_id: i % 2 ? 'Quant' : 'me', text: `${id} text`, created_at: at(i), tokens_used: 0 });
const first = Array.from({ length: 12 }, (_, i) => msg(`m${i}`, i));
const base = { title: 'Binance Trading Lab', subtitle: '', streamingText: '', streamingThinking: '', isStreaming: false, pendingApprovals: [], onSendMessage() {}, onResolveApproval() {}, kwants: [], teams: [], onHighlightDone() {} };
const tick = (ms = 10) => new Promise((r) => setTimeout(r, ms));

/** Mounts the chat with a scroller that has sizes: content height `size.h`, 700 px of view, a real scrollTop. */
async function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const size = { h: 4000, top: 0 };
  let scroller = null;
  const render = async (messages) => {
    m.flushSync(() => root.render(m.React.createElement(m.ChatView, { ...base, messages })));
    if (!scroller) {
      scroller = host.querySelector('[data-message-id]').parentElement;
      while (scroller && !/overflow-y-auto|overflow-auto/.test(scroller.className)) scroller = scroller.parentElement;
      assert.ok(scroller, 'the chat scroller is found');
      Object.defineProperty(scroller, 'scrollHeight', { configurable: true, get: () => size.h });
      Object.defineProperty(scroller, 'clientHeight', { configurable: true, get: () => 700 });
      Object.defineProperty(scroller, 'scrollTop', { configurable: true, get: () => size.top, set: (v) => (size.top = Math.max(0, Math.min(v, size.h - 700))) });
    }
    await tick();
  };
  await render(first);
  // The person follows the bottom.
  size.top = size.h - 700;
  scroller.dispatchEvent(new dom.window.Event('scroll'));
  return { host, root, render, size, get scroller() { return scroller; } };
}
/** The browser's first report for `row` from the observer that watches the chat's rows. */
const report = (row, h) => {
  const ro = observers.filter((o) => o.targets.has(row)).at(-1);
  assert.ok(ro, 'a row observer watches the new row');
  rowHeight.set(row, h);
  ro.cb([{ target: row, borderBoxSize: [{ blockSize: h }], contentRect: { height: h, width: 600 } }], ro);
};

test('chat-gap-scroll: a new message that draws taller than its estimate after the pin stays in view (no reply streaming)', async () => {
  const s = await mount();
  s.size.h += 160; // the new row, at its estimate
  await s.render([...first, msg('probe', 12)]);
  assert.equal(s.size.top, s.size.h - 700, 'the chat pinned to the bottom as the message landed');
  const row = s.host.querySelector('[data-message-id="probe"]');
  s.size.h += 153; // a frame later it draws at 313 px
  report(row, 313);
  assert.equal(s.size.h - s.size.top - 700, 0, 'the newest message is fully in view, not 153 px below it');
  s.root.unmount();
});

test('chat-gap-scroll: a reader scrolled up is not pulled down when a new row settles', async () => {
  const s = await mount();
  s.size.top -= 1500;
  s.scroller.dispatchEvent(new dom.window.Event('scroll'));
  const readingAt = s.size.top;
  s.size.h += 160;
  await s.render([...first, msg('probe', 12)]);
  const row = s.host.querySelector('[data-message-id="probe"]');
  s.size.h += 153;
  report(row, 313);
  assert.equal(s.size.top, readingAt, 'the view stays where they read');
  s.root.unmount();
});

test('chat-gap-scroll: a row that reports the height it was measured at does not re-pin', async () => {
  const s = await mount();
  s.size.h += 160;
  await s.render([...first, msg('probe', 12)]);
  const row = s.host.querySelector('[data-message-id="probe"]');
  let sets = 0;
  const desc = Object.getOwnPropertyDescriptor(s.scroller, 'scrollTop');
  Object.defineProperty(s.scroller, 'scrollTop', { configurable: true, get: desc.get, set: (v) => { sets++; desc.set(v); } });
  report(row, 160);
  assert.equal(sets, 0, 'no change of height, no scroll');
  s.root.unmount();
});
