// The hand-off cap notice in a DOM: pressing Continue against the real POST /api/chains/:id/continue
// client, for each answer the route gives (200 continued, 409 already continued, 410 lead gone).
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

let calls = [];
let answer = { status: 200, body: { status: 'continued' } };
g.fetch = async (url, init = {}) => {
  calls.push([init.method ?? 'GET', String(url)]);
  return new Response(JSON.stringify(answer.body), { status: answer.status, headers: { 'Content-Type': 'application/json' } });
};
g.window.fetch = g.fetch;

const { React, createRoot, HandoffCappedNotice, parseHandoffCapped, chainBudgetApi } = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { HandoffCappedNotice } from './src/components/SystemNotice';
  export { parseHandoffCapped, chainBudgetApi } from './src/chainBudget';
`);

const settle = async () => {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
};

const meta = (resumed = false) =>
  parseHandoffCapped(JSON.stringify({ kind: 'handoff_capped', lead: 'k0', waiting: ['Parser', 'Backend'], waiting_ids: ['k1', 'k2'], sent: 16, text: 'r', user_prompt: 'p', resumed }));

const mount = async (m) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(React.createElement(HandoffCappedNotice, { messageId: 'n1', meta: m, onContinue: chainBudgetApi.continue }));
  await settle();
  return { host, root, section: () => host.querySelector('[data-handoff-capped]'), button: () => host.querySelector('[data-handoff-continue]') };
};

const press = async (v) => {
  v.button().dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('200: Continue posts once, then says who it went to and the button goes', async () => {
  calls = [];
  answer = { status: 200, body: { status: 'continued' } };
  const v = await mount(meta());
  assert.match(v.section().textContent, /Paused after 16 hand-offs so it doesn't loop\. Continue\?/);
  assert.match(v.section().textContent, /Waiting: Parser, Backend/);
  await press(v);
  assert.deepEqual(calls, [['POST', '/api/chains/n1/continue']]);
  assert.equal(v.button(), null);
  assert.equal(v.section().dataset.state, 'sent');
  assert.match(v.section().textContent, /Continued: handed on to Parser, Backend\./);
  assert.doesNotMatch(v.section().textContent, /Continue\?/);
  v.root.unmount();
});

test('409: someone already continued it — say so, no button, no error', async () => {
  answer = { status: 409, body: { error: 'this chain was already continued' } };
  const v = await mount(meta());
  await press(v);
  assert.equal(v.button(), null);
  assert.equal(v.section().dataset.state, 'already');
  assert.match(v.section().textContent, /Already continued\./);
  assert.equal(v.section().querySelector('[role="alert"]'), null);
  v.root.unmount();
});

test('410: the lead is gone — say so plainly and drop the button', async () => {
  answer = { status: 410, body: { error: 'the lead of this chat is gone' } };
  const v = await mount(meta());
  await press(v);
  assert.equal(v.button(), null);
  assert.equal(v.section().dataset.state, 'gone');
  assert.match(v.section().querySelector('[role="alert"]').textContent, /lead of this chat was removed/);
  assert.doesNotMatch(v.section().textContent, /Continue\?/);
  v.root.unmount();
});

test('other failures keep the button so the person can try again', async () => {
  answer = { status: 500, body: { error: 'could not set up the chat' } };
  const v = await mount(meta());
  await press(v);
  assert.equal(v.section().dataset.state, 'error');
  assert.match(v.section().querySelector('[role="alert"]').textContent, /Could not set up the chat/);
  assert.ok(v.button() && !v.button().disabled);
  v.root.unmount();
});

test('resumed:true on load shows it already continued, with no button', async () => {
  calls = [];
  const v = await mount(meta(true));
  assert.equal(v.button(), null);
  assert.equal(v.section().dataset.state, 'sent');
  assert.match(v.section().textContent, /Continued: handed on to Parser, Backend\./);
  assert.deepEqual(calls, []);
  v.root.unmount();
});

test('a typed "continue" flips a live notice to continued when the message refreshes', async () => {
  answer = { status: 200, body: { status: 'continued' } };
  const v = await mount(meta());
  assert.ok(v.button());
  v.root.render(React.createElement(HandoffCappedNotice, { messageId: 'n1', meta: meta(true), onContinue: chainBudgetApi.continue }));
  await settle();
  assert.equal(v.button(), null);
  assert.equal(v.section().dataset.state, 'sent');
  v.root.unmount();
});
