// 33j long-chat-typing-stays-quick (autonomy/long-chat-typing-stays-quick/brief.md): typing in the composer of a
// long chat redraws no message row. The rows still redraw when something they show changes (a new message, Reply,
// a reaction's answer). Real ChatView in jsdom; every 10th message is a delegation card, drawn through
// renderDelegation, so its calls count row draws. Made-up words only; no network.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'KeyboardEvent', 'MouseEvent', 'FocusEvent', 'MutationObserver', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
const ls = { getItem: () => null, setItem: () => {}, removeItem: () => {} };
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true, writable: true });
Object.defineProperty(dom.window, 'localStorage', { value: ls, configurable: true });
dom.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.ResizeObserver ??= globalThis.ResizeObserver;
globalThis.IntersectionObserver ??= class { observe() {} unobserve() {} disconnect() {} };
dom.window.HTMLElement.prototype.scrollIntoView ??= function () {};
dom.window.HTMLElement.prototype.scrollTo ??= function () {};
globalThis.IS_REACT_ACT_ENVIRONMENT = false;
globalThis.fetch = async () => ({ ok: true, status: 200, json: async () => [] });

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { ChatView } from './src/components/ChatView';
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ChatView, q)));
    render(p);
    return { host, render, unmount: () => { root.unmount(); host.remove(); } };
  };
  export const act = (fn) => flushSync(fn);
  export const h = React.createElement;
`);

const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

const at = (i) => new Date(Date.UTC(2026, 10, 1, 9, 0, i)).toISOString();
const delegation = (i) =>
  JSON.stringify({ kind: 'delegation_result', delegation_id: `d${i}`, thread_id: `t${i}`, specialist_id: 'k-wren', specialist_name: 'Wren', status: 'done', title: `Task ${i}` });
/** 100 messages: the person and Ada take turns; every 10th is a delegation card. */
const longChat = (n = 100) =>
  Array.from({ length: n }, (_, i) =>
    i % 10 === 9
      ? { id: `m${i}`, sender_role: 'system', sender_id: 'system', text: `Wren finished task ${i}`, meta: delegation(i), created_at: at(i) }
      : i % 2 === 0
        ? { id: `m${i}`, sender_role: 'user', sender_id: 'PlayerHero', text: `Question number ${i} about the spring plan`, created_at: at(i) }
        : { id: `m${i}`, sender_role: 'kwant', sender_id: 'Ada', text: `Answer ${i}:\n\n- one point\n- **another** point\n\n| a | b |\n|---|---|\n| 1 | 2 |`, created_at: at(i) },
  );

const drawn = { n: 0 };
const renderDelegation = (msg) => {
  drawn.n++;
  return ui.h('div', { 'data-message-id': msg.id, 'data-delegation-card': '' }, msg.text);
};
const props = (extra = {}) => ({
  title: 'Ada',
  subtitle: '',
  messages: longChat(),
  streamingText: '',
  streamingThinking: '',
  isStreaming: false,
  pendingApprovals: [],
  onSendMessage: () => {},
  onResolveApproval: () => {},
  onReact: async () => ({ ok: true }),
  kwants: [{ id: 'k-ada', name: 'Ada' }],
  teams: [],
  contextKey: 'k:ada',
  renderDelegation,
  ...extra,
});
const typeInto = (ta, value) =>
  ui.act(() => {
    Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set.call(ta, value);
    ta.dispatchEvent(new window.Event('input', { bubbles: true }));
  });
const click = (el) => ui.act(() => el.dispatchEvent(new window.MouseEvent('click', { bubbles: true })));

test('typing 57 characters in a 100-message chat draws no message row', async () => {
  const v = ui.mount(props());
  await settle();
  assert.equal(v.host.querySelectorAll('[data-delegation-card]').length, 10, 'the 10 delegation cards are on screen');
  assert.equal(v.host.querySelectorAll('[data-message-id]').length, 100, 'all 100 rows are on screen');
  const ta = v.host.querySelector('textarea[data-composer]');
  ta.focus();
  await settle();
  const words = 'Ada, plan the spring newsletter with the three quotes ok.';
  assert.equal(words.length, 57);
  drawn.n = 0;
  let typed = '';
  for (const ch of words) {
    typed += ch;
    typeInto(ta, typed);
  }
  await settle();
  assert.equal(ta.value, words, 'the composer shows every keystroke');
  assert.equal(drawn.n, 0, `typing redrew message rows ${drawn.n} times (${drawn.n / 10} redraws of the list)`);
  // The @mention list still opens while the rows stay put
  typeInto(ta, `${words} @A`);
  await settle();
  assert.ok(v.host.querySelector('#kw-mention-list'), 'the @mention list opens');
  assert.equal(drawn.n, 0);
  v.unmount();
});

test('the rows still redraw when what they show changes: a new message and Reply', async () => {
  const p = props();
  const v = ui.mount(p);
  await settle();
  const ta = v.host.querySelector('textarea[data-composer]');
  typeInto(ta, 'draft');
  drawn.n = 0;
  // A new message lands
  const more = [...p.messages, { id: 'm100', sender_role: 'kwant', sender_id: 'Ada', text: 'A new answer', created_at: at(100) }];
  v.render({ ...p, messages: more });
  await settle();
  assert.ok(v.host.querySelector('[data-message-id="m100"]'), 'the new message shows');
  assert.ok(drawn.n >= 10, 'the list redrew for the new message');
  assert.equal(ta.value, 'draft', 'the draft is kept');
  // Reply on a reply: its button turns on and the composer shows the reply chip
  const row = v.host.querySelector('[data-message-id="m99"]') ? v.host.querySelector('[data-message-id="m97"]') : null;
  assert.ok(row);
  const replyBtn = [...row.querySelectorAll('button')].find((b) => /^Reply/.test(b.getAttribute('aria-label') ?? b.textContent ?? ''));
  assert.ok(replyBtn, 'the reply has a Reply button');
  const before = replyBtn.outerHTML;
  click(replyBtn);
  await settle();
  const after = [...v.host.querySelector('[data-message-id="m97"]').querySelectorAll('button')].find((b) => /^Reply/.test(b.getAttribute('aria-label') ?? b.textContent ?? ''));
  assert.notEqual(after.outerHTML, before, 'the Reply button shows it is on (the row redrew)');
  v.unmount();
});
