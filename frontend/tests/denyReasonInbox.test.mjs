// 29q deny-reason-inbox-guard (Iris): the Inbox's own approval card carries a Say why reason to the server.
// Mounts the real InboxNew (no stubbed onResolve) and reads the POST body it sends. Guards InboxNew's QueueRow
// passing `opts?.reason` on: without it, an Inbox Deny with words would reach the bot as a plain no.
process.env.TZ = 'UTC';
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/inbox', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'CustomEvent', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

// Every request the Inbox makes: [method, url, parsed body or null].
let sent = [];
g.fetch = async (url, init = {}) => {
  sent.push([init.method ?? 'GET', String(url), init.body ? JSON.parse(init.body) : null]);
  return { ok: true, status: 200, json: async () => ({}) };
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as fx from './src/doneAwayFixture';
  export * as store from './src/inboxStore';
  export { InboxNew } from './src/components/InboxNew';
`);
const { React, createRoot, fx, store: s } = m;
const h = React.createElement;
const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const clone = (x) => JSON.parse(JSON.stringify(x));
const click = async (b) => {
  b.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};
// A controlled React input: set the DOM value past React's tracker, then fire `input`.
const type = async (input, text) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(input, text);
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
// Enter in the field: the browser submits its form.
const enter = async (input) => {
  input.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  input.form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};

let root = null;
afterEach(() => {
  root?.unmount();
  root = null;
  document.body.innerHTML = '';
});

async function mountInbox(wide) {
  const data = clone(fx.AWAY_INBOX);
  document.body.innerHTML = '';
  sessionStorage.clear();
  window.history.replaceState(null, '', '/inbox');
  s.inboxStore.reset();
  s.inboxSource.get = async () => data;
  s.inboxStore.patch({ data });
  const el = document.createElement('div');
  document.body.appendChild(el);
  root = createRoot(el);
  root.render(h(m.InboxNew, { onOpen: () => {}, noLoad: true, wide, now: Date.parse(data.now), placeName: fx.awayPlaceName }));
  await settle();
  sent = [];
  const row = el.querySelector('[data-row="ap-1"]');
  assert.ok(row, 'the Inbox shows the approval row');
  return row;
}
const resolves = () => sent.filter(([method, url]) => method === 'POST' && url === '/api/approvals/ap-1/resolve');
const denyBtn = (row) => [...row.querySelectorAll('[data-approval-actions] button')].find((b) => b.textContent.trim() === 'Deny');

for (const wide of [true, false]) {
  const where = wide ? 'wide' : 'phone';

  test(`29q (${where}): an Inbox Deny with words sends them as reason`, async () => {
    const row = await mountInbox(wide);
    await click(row.querySelector('[data-approval-say-why]'));
    const input = row.querySelector('[data-approval-why] input');
    assert.ok(input, 'Say why opens the field in the Inbox card');
    await type(input, '  Use the staging list  ');
    await enter(input);
    assert.deepEqual(resolves().map(([, , body]) => body), [{ decision: 'deny', reason: 'Use the staging list' }]);
  });

  test(`29q (${where}): a plain one-tap Inbox Deny sends no reason`, async () => {
    const row = await mountInbox(wide);
    await click(denyBtn(row));
    const bodies = resolves().map(([, , body]) => body);
    assert.deepEqual(bodies, [{ decision: 'deny' }]);
    assert.equal('reason' in bodies[0], false);
  });
}

test('29q: Say why looks open while its field is open, and quiet again once closed', async () => {
  const row = await mountInbox(true);
  const btn = row.querySelector('[data-approval-say-why]');
  const look = () => ({ open: btn.classList.contains('text-zinc-100'), quiet: btn.classList.contains('text-[#a39dcc]'), expanded: btn.getAttribute('aria-expanded') });
  assert.deepEqual(look(), { open: false, quiet: true, expanded: 'false' });
  await click(btn);
  assert.deepEqual(look(), { open: true, quiet: false, expanded: 'true' });
  await click(btn);
  assert.deepEqual(look(), { open: false, quiet: true, expanded: 'false' });
});

test('29q: the Inbox approval card keeps the 54 px indent from sm up only, so a phone field has room', async () => {
  const row = await mountInbox(false);
  const wrap = row.querySelector('[data-approval-actions]').closest('li > div');
  const cls = wrap.className.split(/\s+/);
  assert.ok(cls.includes('sm:pl-[54px]'));
  assert.equal(cls.includes('pl-[54px]'), false, 'no 54 px indent below sm');
});
