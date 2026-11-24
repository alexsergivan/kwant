// chat-gap-scroll (Oleks, Nov 24: "sometimes there is a free space after accepting message"): when the last ask
// leaves, the Needs-you strip fades for 120 ms and then goes. App rebuilds `asks` as a new array on every render,
// so a re-render inside those 120 ms (the stream starting, the inbox refetch) cancelled the fade timer and the
// re-run saw no asks before and none now, so the ghost was never cleared. The strip stayed mounted, invisible
// (kw-needs-leave, opacity 0) and full height, until a new ask came: a blank gap above the composer.
// Red at main d35c8a52.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body><textarea data-composer></textarea></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
let reduced = false;
dom.window.matchMedia = (q) => ({ matches: q.includes('reduce') ? reduced : false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
if (!('ResizeObserver' in dom.window)) dom.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.ResizeObserver = dom.window.ResizeObserver;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { NeedsYouStrip } from './src/components/NeedsYouStrip';
`);

const NOW = Date.parse('2026-11-24T16:50:00Z');
const ask = { kind: 'approval', id: 'ap1', kwant_id: 'q', kwant_name: 'Quant', title: 'Run a command', summary: 'git push', status: 'pending',
  timestamp: new Date(NOW - 60_000).toISOString(), link: { thread_id: null, channel_id: 'c1', direct_kwant_id: null }, action: 'decide', dismissable: false };
const tick = (ms) => new Promise((r) => setTimeout(r, ms));

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  // Like App: every render passes a fresh array, even when nothing changed.
  const render = (asks) => m.flushSync(() => root.render(m.React.createElement(m.NeedsYouStrip, { asks: [...asks], onAction() {}, onDismiss: async () => {}, now: NOW })));
  const strip = () => host.querySelector('[data-needs-you-strip]');
  // What's left of the strip, as text (a failed assert must not print a whole DOM node).
  const left = () => { const el = strip(); return el ? `strip still mounted: class="${el.className}"` : 'gone'; };
  return { host, root, render, strip, left };
}

test('chat-gap-scroll: the last ask leaving fades out and is gone, even when the parent re-renders mid-fade', async () => {
  reduced = false;
  const s = mount();
  s.render([ask]);
  assert.ok(s.strip(), 'the strip shows the ask');
  s.render([]);
  assert.ok(s.strip()?.className.includes('kw-needs-leave'), 'it fades for 120 ms instead of vanishing mid-read');
  await tick(50);
  s.render([]); // the stream starts or the inbox refreshes: App renders again with a new empty array
  await tick(30);
  s.render([]);
  await tick(120);
  assert.equal(s.left(), 'gone', 'the strip is gone 200 ms later, not left invisible at full height');
  s.root.unmount();
});

test('chat-gap-scroll: without re-renders the fade still ends at 120 ms', async () => {
  reduced = false;
  const s = mount();
  s.render([ask]);
  s.render([]);
  await tick(160);
  assert.equal(s.left(), 'gone');
  s.root.unmount();
});

test('chat-gap-scroll: a new ask during the fade shows at once, with no ghost class', async () => {
  reduced = false;
  const s = mount();
  s.render([ask]);
  s.render([]);
  await tick(40);
  s.render([{ ...ask, id: 'ap2', title: 'Publish the package' }]);
  assert.ok(s.strip() && !s.strip().className.includes('kw-needs-leave'));
  assert.match(s.strip().textContent, /Publish the package/);
  await tick(150);
  assert.notEqual(s.left(), 'gone', 'the new ask stays');
  s.root.unmount();
});

test('chat-gap-scroll: reduced motion has no ghost, and focus in the strip goes to the composer', async () => {
  reduced = true;
  const s = mount();
  s.render([ask]);
  const btn = s.strip().querySelector('button');
  btn.focus();
  s.strip().dispatchEvent(new dom.window.FocusEvent('focusin', { bubbles: true }));
  s.render([]);
  assert.equal(s.left(), 'gone', 'no fade under reduced motion');
  assert.equal(document.activeElement?.tagName, 'TEXTAREA', 'focus lands on the composer');
  s.root.unmount();
});
