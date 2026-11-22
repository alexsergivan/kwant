// 33p (brief: autonomy/search-thread-title-first/brief.md): in the search box, a thread whose title matches the term
// comes first, so keyboard order (data-index 0, Enter) starts at its first hit. The order follows the term that
// produced the hits, not the live input, so groups don't jump while the person is still typing.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Node', 'Event', 'localStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
dom.window.HTMLElement.prototype.scrollIntoView = function () {};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { SearchModal } from './src/components/SearchModal';
`);

const hit = (id, kwant_id, extra = {}) => ({ message_id: id, channel_id: null, channel_name: null, kwant_id, author: 'a', snippet: `${id} <mark>tax</mark>`, created_at: null, ...extra });
const thread = (id) => hit(id, 'led', { thread_id: 't1', thread_title: '2025 tax return' });
const results = {
  tax: [...Array.from({ length: 7 }, (_, i) => hit(`d${i + 1}`, 'led')), thread('ask'), thread('ans')],
};
const asked = [];
let gate = null;
g.fetch = g.window.fetch = async (url) => {
  const q = new URL(url, 'http://localhost').searchParams.get('q');
  asked.push(q);
  if (gate) await gate;
  return new Response(JSON.stringify(results[q] ?? results.tax), { status: 200, headers: { 'Content-Type': 'application/json' } });
};
const tick = (ms = 10) => new Promise((r) => setTimeout(r, ms));
const type = (input, value) => {
  const setter = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, value);
  input.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
};
const kwants = [{ id: 'led', name: 'Ledger' }];

test('33p: searching "tax" puts "Ledger · 2025 tax return" first and data-index 0 on its first hit', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.SearchModal, { isOpen: true, onClose() {}, kwants, onJump() {} })));
  const input = host.querySelector('input[role="combobox"]');
  type(input, 'tax');
  for (let i = 0; i < 60 && !host.querySelector('[data-index="0"]'); i++) await tick();
  const labels = [...host.querySelectorAll('[role="group"]')].map((el) => el.getAttribute('aria-label'));
  assert.deepEqual(labels, ['Ledger · 2025 tax return', 'Direct chat · Ledger']);
  assert.equal(host.querySelector('[data-index="0"]').id, 'search-hit-ask');
  assert.equal(host.querySelector('[data-index="1"]').id, 'search-hit-ans');
  assert.equal(host.querySelector('[data-index="2"]').id, 'search-hit-d1');
  assert.equal(input.getAttribute('aria-activedescendant'), 'search-hit-ask');

  // Typing on ("taxes") while the old hits are still shown: the groups keep the order the "tax" hits came with.
  let open;
  gate = new Promise((r) => (open = r));
  type(input, 'taxes');
  await tick(300);
  assert.deepEqual([...host.querySelectorAll('[role="group"]')].map((el) => el.getAttribute('aria-label')), labels, 'no jump while typing');
  open();
  gate = null;
  for (let i = 0; i < 60 && host.querySelector('[role="group"]')?.getAttribute('aria-label') !== 'Direct chat · Ledger'; i++) await tick();
  // "taxes" is not a prefix of "tax": once its hits arrive, the server's order stands.
  assert.deepEqual([...host.querySelectorAll('[role="group"]')].map((el) => el.getAttribute('aria-label')), ['Direct chat · Ledger', 'Ledger · 2025 tax return']);
  assert.equal(host.querySelector('[data-index="0"]').id, 'search-hit-d1');
  assert.deepEqual(asked, ['tax', 'taxes']);
  root.unmount();
  host.remove();
});
