// Continue in a new chat from here (autonomy/continue-chat/brief.md, "Frontend (Iris)"): the API helper, where the
// item shows, the Continued line, and the overflow's keyboard.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`export * from './src/continueChat';`);
const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { MessageMore, MessageMoreError, ContinuedLine } from './src/components/MessageMore';
  export { createRoot } from 'react-dom/client';
  export { React, flushSync, MessageMore, MessageMoreError, ContinuedLine };
`);

const json = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

// ---- the API helper ----

test('fromMessage posts to the continue route and reads the 201', async () => {
  const calls = [];
  const send = async (url, init) => {
    calls.push([url, init?.method]);
    return json(201, { id: 't2', kwant_id: 'k1', title: 'Prices (continued)', kind: 'chat', forked_from_context: 't1', forked_from_message_id: 'm 3', copied: 12, truncated: false });
  };
  const t = await m.continueChatApi.fromMessage('m 3', send);
  assert.deepEqual(calls, [['/api/messages/m%203/continue', 'POST']]);
  assert.equal(t.id, 't2');
  assert.equal(t.forked_copied, 12, 'the top-level copied fills in when the thread view has no count');
  assert.equal(t.forked_truncated, false);
});

test("fromMessage prefers the thread view's own counts (Lin's forked_copied / forked_truncated)", async () => {
  const send = async () => json(201, { id: 't2', kwant_id: 'k1', forked_from_context: 'k1', forked_from_message_id: 'm9', forked_copied: 1000, forked_truncated: true, copied: 1000, truncated: true });
  const t = await m.continueChatApi.fromMessage('m9', send);
  assert.equal(t.forked_copied, 1000);
  assert.equal(t.forked_truncated, true);
});

test("fromMessage errors: our own words for Lin's 404/403/5xx, the server's only for the 400", async () => {
  // Lin's exact bodies, from src/server/continue_chat.rs on lin-continue-chat (#618)
  const cases = [
    [400, { error: 'Only a chat with one bot can be continued.' }, "Couldn't start a new chat: only a chat with one bot can be continued."],
    [400, null, "Couldn't start a new chat: only a chat with one bot can be continued."],
    [404, { error: 'message not found' }, "Couldn't start a new chat: that message is no longer in this chat."],
    [404, { error: "that isn't in this space" }, "Couldn't start a new chat: that message is no longer in this chat."],
    [403, { error: 'only the person continues a chat in a new one' }, "Couldn't start a new chat: only you can do this, not a bot."],
    [500, { error: 'database is locked' }, "Couldn't start a new chat: something went wrong (error 500). Try again."],
    [502, null, "Couldn't start a new chat: something went wrong (error 502). Try again."],
  ];
  for (const [status, body, line] of cases) {
    const send = async () => (body ? json(status, body) : new Response('nope', { status }));
    await assert.rejects(m.continueChatApi.fromMessage('m1', send), (e) => e instanceof m.ContinueError && e.status === status && m.continueErrorLine(e) === line);
  }
  await assert.rejects(m.continueChatApi.fromMessage('m1', async () => { throw new TypeError('Failed to fetch'); }), (e) => e.status === 0 && /didn't answer/.test(e.message));
  await assert.rejects(m.continueChatApi.fromMessage('m1', async () => json(201, { nope: 1 })), /no chat/);
});

test('the error line is one plain sentence', () => {
  assert.equal(m.continueErrorLine(new Error('Only a chat with one bot can be continued.')), "Couldn't start a new chat: only a chat with one bot can be continued.");
  assert.equal(m.continueErrorLine(new m.ContinueError("the server didn't answer", 0)), "Couldn't start a new chat: the server didn't answer.");
  assert.equal(m.continueErrorLine(new Error('')), "Couldn't start a new chat: something went wrong.");
  assert.equal(m.continueErrorLine(null), "Couldn't start a new chat: something went wrong.");
});

// ---- where the item shows ----

const base = { chatKind: 'bot', threadKind: null, message: { id: 'm1', sender_role: 'kwant' }, newest: false, streaming: false, botSession: false };
const shows = (o = {}) => m.canContinueFrom({ ...base, ...o });

test('it shows on the person’s and the bot’s messages in a chat with one bot', () => {
  assert.equal(shows(), true, 'main chat, a reply');
  assert.equal(shows({ message: { id: 'm2', sender_role: 'user' } }), true, 'the person’s message');
  assert.equal(shows({ threadKind: 'chat' }), true, 'a chat thread');
  assert.equal(shows({ streaming: true }), true, 'an older message while a reply streams');
});

test('it hides in team chats, channels, delegations, bot sessions and on a message still being written', () => {
  for (const chatKind of ['team', 'channel', 'general', undefined]) assert.equal(shows({ chatKind }), false, String(chatKind));
  assert.equal(shows({ inTeam: true }), false);
  assert.equal(shows({ threadKind: 'delegation' }), false);
  assert.equal(shows({ botSession: true }), false);
  assert.equal(shows({ message: { id: 'temp-1', sender_role: 'user' } }), false, 'not sent yet');
  assert.equal(shows({ newest: true, streaming: true }), false, 'the newest while a reply streams');
  assert.equal(shows({ message: { id: 'm3', sender_role: 'system' } }), false);
});

// ---- the Continued line ----

test('continuedLine reads like the brief', () => {
  assert.equal(m.continuedLine('Price check', 12, false), 'Continued from "Price check" · 12 messages');
  assert.equal(m.continuedLine('Cody', 1), 'Continued from "Cody" · 1 message');
  assert.equal(m.continuedLine('Price check', 1000, true), 'Continued from "Price check" · the newest 1,000 messages');
  assert.equal(m.continuedLine('Ben', 4, false, true), 'Continued from your main chat with Ben · 4 messages', 'the main chat: no quotes');
  assert.equal(m.continuedLine('Ben', 1000, true, true), 'Continued from your main chat with Ben · the newest 1,000 messages');
  assert.equal(m.continuedLine('Old', null), 'Continued from "Old"', 'no count after a reload on an older server');
  assert.equal(m.continuedLine('  ', 3), 'Continued from "the earlier chat" · 3 messages');
});

test('forkOf reads the thread fields, null for a thread that was not continued', () => {
  assert.equal(m.forkOf({ forked_from_context: null, forked_from_message_id: null }), null);
  assert.equal(m.forkOf(null), null);
  assert.deepEqual(m.forkOf({ forked_from_context: 'k1', forked_from_message_id: 'm1', forked_copied: 4, forked_truncated: null }), { context: 'k1', messageId: 'm1', copied: 4, truncated: false });
});

// ---- the overflow ----

const mount = (Comp, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  const render = (p) => ui.flushSync(() => root.render(ui.React.createElement(Comp, p)));
  render(props);
  return { host, render, unmount: () => (root.unmount(), host.remove()) };
};
const tick = () => new Promise((r) => setTimeout(r, 0));
const key = (el, k) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));

test('the overflow: one button with a name, the menu opens with the item focused, Esc closes and returns focus', async () => {
  let ran = 0;
  const v = mount(ui.MessageMore, { messageId: 'm1', about: "Cody's reply", items: [{ label: m.CONTINUE_LABEL, run: () => ran++ }] });
  const btn = v.host.querySelector('[data-message-more-button]');
  assert.equal(btn.getAttribute('aria-label'), "More for Cody's reply");
  assert.equal(btn.getAttribute('aria-haspopup'), 'menu');
  assert.equal(btn.getAttribute('aria-expanded'), 'false');
  assert.equal(v.host.querySelector('[role="menu"]'), null, 'closed at first: not a fourth visible button');
  btn.focus();
  btn.click();
  await tick();
  assert.equal(btn.getAttribute('aria-expanded'), 'true');
  const item = v.host.querySelector('[role="menuitem"]');
  assert.equal(item.textContent, m.CONTINUE_LABEL);
  assert.equal(document.activeElement, item, 'the item takes focus');
  key(item, 'ArrowDown');
  assert.equal(document.activeElement, item, 'arrows wrap within one item');
  key(item, 'Escape');
  await tick();
  await tick();
  assert.equal(v.host.querySelector('[role="menu"]'), null);
  assert.equal(document.activeElement, btn, 'Esc gives focus back to ⋯');
  key(btn, 'ArrowDown');
  await tick();
  v.host.querySelector('[role="menuitem"]').click();
  await tick();
  assert.equal(ran, 1);
  assert.equal(v.host.querySelector('[role="menu"]'), null, 'it closes when the item runs');
  assert.equal(document.activeElement, v.host.querySelector('[data-message-more-button]'), 'focus stays at the message, by the error line if it fails');
  v.unmount();
});

test('while the new chat is made, ⋯ spins, cannot be pressed and keeps focus', async () => {
  const props = { messageId: 'm1', about: 'your message', align: 'end', reveal: 'hover', busyLabel: 'Starting a new chat…', items: [{ label: m.CONTINUE_LABEL, run: () => {} }] };
  const v = mount(ui.MessageMore, props);
  const btn = v.host.querySelector('[data-message-more-button]');
  btn.click();
  await tick();
  assert.match(v.host.querySelector('[role="menu"]').className, /right-0/, 'the person’s message opens to the left');
  v.host.querySelector('[role="menuitem"]').click();
  await tick();
  v.render({ ...props, busy: true });
  assert.equal(document.activeElement, btn, 'focus stays on ⋯ (aria-disabled, not disabled)');
  assert.equal(btn.getAttribute('aria-busy'), 'true');
  assert.equal(btn.getAttribute('aria-disabled'), 'true');
  assert.equal(btn.getAttribute('aria-label'), 'Starting a new chat…');
  assert.ok(btn.querySelector('svg.animate-spin.motion-reduce\\:animate-none'), 'the spinner, still under reduced motion');
  btn.click();
  key(btn, 'ArrowDown');
  await tick();
  assert.equal(v.host.querySelector('[role="menu"]'), null, 'a second press does nothing');
  assert.match(v.host.querySelector('[data-message-more]').className, /opacity-100/, 'visible while busy, even off hover');
  v.render({ ...props, busy: false });
  assert.equal(btn.hasAttribute('aria-busy'), false);
  assert.match(v.host.querySelector('[data-message-more]').className, /\[@media\(hover:hover\)\]:opacity-0/, 'hidden until hover on desktop');
  v.unmount();
});

test('the error line and the Continued link', async () => {
  const e = mount(ui.MessageMoreError, { text: "Couldn't start a new chat: Message not found." });
  assert.equal(e.host.querySelector('[data-continue-error]').getAttribute('role'), 'status');
  e.unmount();
  let opened = 0;
  const c = mount(ui.ContinuedLine, { text: 'Continued from "Prices" · 4 messages', onOpen: () => opened++ });
  const b = c.host.querySelector('[data-continued-from] button');
  assert.equal(b.textContent, 'Continued from "Prices" · 4 messages');
  b.click();
  assert.equal(opened, 1);
  c.unmount();
});

// ---- wiring (source checks) ----

test('ChatView puts ⋯ right after Copy, the line before the messages, and keeps it out of message text', () => {
  const src = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const copy = src.indexOf('<CopyGhost copied=');
  const more = src.indexOf("continueMenu(msg, `${displayName}'s reply`, 'start')");
  assert.ok(copy > 0 && more > copy && more - copy < 200, '⋯ sits next to Copy');
  assert.match(src, /continueMenu\(msg, 'your message', 'end', 'hover'\)/);
  assert.match(src, /absolute right-full bottom-0 mr-1" data-message-more-gutter/, 'the person’s ⋯ sits in the gutter, no row of its own');
  assert.equal(/mt-1 flex justify-end">\{more\}/.test(src), false, 'no ⋯ row under the person’s bubble');
  assert.ok(src.indexOf('<ContinuedLine') > 0);
  assert.equal(/handleCopy\([^)]*continued/i.test(src), false);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /continueChatApi\.fromMessage<Thread>\(msg\.id, personFetch\)/, 'person-only, so "Confirm it’s you" can answer a 401');
  assert.match(app, /fork\.truncated,\s*fork\.context === currentKwant\.id,/, 'the main chat reads "your main chat with <bot>"');
  assert.match(app, /canContinueHere = !!currentKwant && !currentTeam && !botSession && \(!openThreadId \|\| shownThread\?\.kind === 'chat'\)/);
});
