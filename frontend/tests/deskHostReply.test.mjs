// desk-host-reply-composer (front desk fix #3, ux-pass-front-desk report): in a front desk guest's thread the host
// answered in Bell's normal composer, Bell said "Done." and Tom never saw it. The composer is now "Reply to Tom": it
// sends with pocketApi.deskReply and never starts a bot turn; the thread shows the sent line; the raw [[person]]
// mark is the hand-off chip; "Pick up where you left off" leaves these threads out.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLTextAreaElement', 'Element', 'Node', 'Event', 'FocusEvent', 'MouseEvent', 'KeyboardEvent', 'InputEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/deskThread';
  export { recentWork } from './src/botHome';
  export { DeskReplyComposer, DeskSentRow, DeskHandoffChip, useDeskThread } from './src/components/DeskReplyComposer';
  import React from 'react';
  import { DeskReplyComposer, useDeskThread } from './src/components/DeskReplyComposer';
  // The way ChatView wires them: the hook keeps the echo and the gone state, the composer sends through it.
  export function Harness({ guest, count = 1 }) {
    const desk = useDeskThread(guest, count);
    return React.createElement('div', null,
      React.createElement('ul', { 'data-lines': '' }, desk.lines.map((l) => React.createElement('li', { key: l.id, 'data-line': l.sending ? 'sending' : 'sent' }, l.text))),
      React.createElement(DeskReplyComposer, { guest, bot: 'Bell', desk }));
  }
`);

const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0)); };
const meta = (o) => JSON.stringify(o);
const TOM = { source: 'pocket-guest', from_customer: 'Tom', pocket_code: 'seaview', guest_id: 'pl_tom' };

// ---- recentWork
test('recentWork leaves front desk guest threads out of "Pick up where you left off"', () => {
  const t = (id, at, extra = {}) => ({ id, title: id, updated_at: at, kind: 'chat', archived: false, created_by: 'user', ...extra });
  const out = m.recentWork([t('Tom', '2026-11-28T10:00:00Z', { created_by: 'pocket-guest' }), t('Emma', '2026-11-28T09:00:00Z', { created_by: 'pocket-guest' }), t('Plan the week', '2026-11-27T10:00:00Z'), t('Wi-Fi card', '2026-11-26T10:00:00Z')]);
  assert.deepEqual(out.map((r) => r.id), ['Plan the week', 'Wi-Fi card']);
  // A list with only guest threads has nothing to pick up
  assert.deepEqual(m.recentWork([t('Tom', '2026-11-28T10:00:00Z', { created_by: 'pocket-guest' })]), []);
});

// ---- which thread is a desk guest's
test('deskGuestOf: created_by "pocket-guest" or the guest messages\' meta; Telegram customers and own chats are not', () => {
  const msgs = [{ meta: meta(TOM) }, { meta: null }, { meta: meta({ kind: 'reply' }) }];
  assert.deepEqual(m.deskGuestOf('pocket-guest', msgs, 'Tom'), { code: 'seaview', guestId: 'pl_tom', guestName: 'Tom' });
  // An older thread list without created_by: the messages say so
  assert.deepEqual(m.deskGuestOf(undefined, msgs), { code: 'seaview', guestId: 'pl_tom', guestName: 'Tom' });
  // No guest message yet: still the desk's thread, nothing to send to
  assert.deepEqual(m.deskGuestOf('pocket-guest', [], 'Tom'), { code: null, guestId: null, guestName: 'Tom' });
  assert.equal(m.deskGuestOf('user', [{ meta: meta({ source: 'telegram', from_customer: 'Ana' }) }]), null);
  assert.equal(m.deskGuestOf('user', [{ meta: 'not json' }]), null);
  assert.equal(m.deskGuestOf(null, []), null);
});

// ---- the hand-off chip
test('deskHandoff hides the [[person]] line and keeps the desk card\'s reason ("Request:" dropped)', () => {
  assert.deepEqual(m.deskHandoff('[[person]] Late check-out: 1 PM, Sun Nov 29', 'Bell'), { text: '', reason: 'Late check-out: 1 PM, Sun Nov 29' });
  assert.deepEqual(m.deskHandoff('I\'ll ask Marta.\n[[person]] Request: Late check-out: 1 PM', 'Bell'), { text: 'I\'ll ask Marta.', reason: 'Late check-out: 1 PM' });
  assert.deepEqual(m.deskHandoff('[[person]]: a complaint', 'Bell'), { text: '', reason: 'a complaint' });
  assert.deepEqual(m.deskHandoff('[[person]]', 'Bell'), { text: '', reason: 'Bell asked you to reply' });
  assert.equal(m.deskHandoff('The Wi-Fi is SeaView-Guest.', 'Bell'), null);
  assert.equal(m.handoffChipText('Late check-out: 1 PM, Sun Nov 29'), 'Needs you · Late check-out: 1 PM, Sun Nov 29');
});

test('deskSentTo: the host\'s reply the server saved in the thread (meta to_guest); the words', () => {
  // desk_turns.rs after_act saves each reply/decline line as a user message with { to_guest, pocket_code }
  assert.equal(m.deskSentTo({ sender_role: 'user', meta: meta({ to_guest: 'pl_tom', pocket_code: 'seaview' }) }), 'pl_tom');
  assert.equal(m.deskSentTo({ sender_role: 'user', meta: null }), null, 'a person\'s own chat line');
  assert.equal(m.deskSentTo({ sender_role: 'kwant', meta: meta({ to_guest: 'pl_tom' }) }), null);
  assert.equal(m.deskSentTo({ sender_role: 'user', meta: meta(TOM) }), null, 'the guest\'s own message');
  assert.equal(m.pocketEnded({ state: 'live', ended_at: null }), false);
  assert.equal(m.pocketEnded({ state: 'ended', ended_at: null }), true);
  assert.equal(m.pocketEnded({ state: 'closed' }), true);
  assert.equal(m.goneError({ status: 410, code: 'ended' }), true);
  assert.equal(m.goneError({ status: 404, code: 'no_such_player' }), true);
  assert.equal(m.goneError({ status: 0, code: null }), false);
  assert.equal(m.deskReplyNote('Tom', 'Bell'), 'Tom reads what you send here. Bell doesn’t answer it.');
  // The chat's clock: 12-hour with AM/PM where the viewer uses it, 24-hour where they don't
  assert.match(m.sentLabel('Tom', '2026-11-28T16:31:00Z', 'en-US'), /^You → Tom · \d{1,2}:31\s?(AM|PM)$/);
  assert.match(m.sentLabel('Tom', '2026-11-28T16:31:00Z', 'en-GB'), /^You → Tom · \d{2}:31$/);
  assert.equal(m.sentLabel('Tom', ''), 'You → Tom');
});

// ---- the composer, mounted, against a fake box
function fakeBox({ getStatus = 200, replyStatus = 200, ended = false } = {}) {
  const calls = [];
  let hostSaid = [];
  g.fetch = g.window.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push({ url: String(url), method, body });
    const pocket = { code: 'seaview', state: ended ? 'ended' : 'live', ended_at: null };
    if (String(url).startsWith('/api/pockets/seaview/action')) {
      if (replyStatus !== 200) return { ok: false, status: replyStatus, json: async () => ({ error: replyStatus === 410 ? 'ended' : 'oops', message: replyStatus === 410 ? 'This pocket app has ended.' : 'The box had a problem.' }) };
      hostSaid = [...hostSaid, { id: `h${hostSaid.length + 1}`, from: 'host', text: body.body.text, at: '2026-11-28T16:31:00Z' }];
      return { ok: true, status: 200, json: async () => ({ pocket, summary: null, removed: 0 }) };
    }
    if (String(url).startsWith('/api/pockets/seaview')) {
      if (getStatus !== 200) return { ok: false, status: getStatus, json: async () => ({ error: getStatus === 410 ? 'ended' : 'no_such_player' }) };
      return { ok: true, status: 200, json: async () => ({ pocket, summary: null, removed: 0, conversation: { guest: { id: 'pl_tom', name: 'Tom' }, messages: hostSaid } }) };
    }
    return { ok: false, status: 404, json: async () => ({}) };
  };
  return calls;
}

function mount(guest) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const draw = (count = 1) => m.flushSync(() => root.render(m.React.createElement(m.Harness, { guest, count })));
  draw();
  return { host, draw, done: () => { m.flushSync(() => root.unmount()); host.remove(); } };
}

function type(el, value) {
  const set = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
}

const GUEST = { code: 'seaview', guestId: 'pl_tom', guestName: 'Tom' };

test('"Reply to Tom" sends with deskReply to the guest, never a bot turn, and the sent line shows', async () => {
  const calls = fakeBox();
  const { host, draw, done } = mount(GUEST);
  await settle();
  const box = host.querySelector('textarea[data-desk-reply]');
  assert.ok(box, 'the reply box');
  assert.equal(box.getAttribute('placeholder'), 'Reply to Tom');
  assert.equal(host.querySelector('[data-desk-note]').textContent, 'Tom reads what you send here. Bell doesn’t answer it.');
  assert.equal(host.querySelector('form').getAttribute('aria-label'), 'Reply to Tom');
  type(box, 'Tom, the ferry to Trieste leaves at 4:30 PM from the main pier.');
  await settle();
  box.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  await settle();
  const posts = calls.filter((c) => c.method !== 'GET');
  assert.deepEqual(posts, [{ url: '/api/pockets/seaview/action', method: 'POST', body: { name: 'reply', body: { guest: 'pl_tom', text: 'Tom, the ferry to Trieste leaves at 4:30 PM from the main pier.' } } }]);
  assert.ok(calls.every((c) => c.url.startsWith('/api/pockets/')), 'nothing but the desk is called (no chat, no turn)');
  assert.deepEqual([...host.querySelectorAll('[data-line]')].map((li) => [li.dataset.line, li.textContent]), [['sent', 'Tom, the ferry to Trieste leaves at 4:30 PM from the main pier.']]);
  assert.equal(box.value, '', 'the box clears after a send');
  // The server's saved copy lands in the thread (one more message): the echo steps aside, so it shows once
  draw(2);
  await settle();
  assert.equal(host.querySelectorAll('[data-line]').length, 0);
  done();
});

test('an empty box sends nothing; Shift+Enter is a new line', async () => {
  const calls = fakeBox();
  const { host, done } = mount(GUEST);
  await settle();
  const box = host.querySelector('textarea[data-desk-reply]');
  box.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  type(box, 'Hi');
  await settle();
  box.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, cancelable: true }));
  await settle();
  assert.equal(calls.filter((c) => c.method !== 'GET').length, 0);
  assert.equal(host.querySelector('button[type=submit]').disabled, false);
  done();
});

test('the desk has ended (or the guest left): one muted line in place of the box', async () => {
  fakeBox({ getStatus: 410 });
  let v = mount(GUEST);
  await settle();
  assert.equal(v.host.querySelector('textarea'), null);
  assert.equal(v.host.querySelector('[data-desk-composer="closed"]').textContent, 'This desk has ended, so a reply can’t reach the guest.');
  v.done();

  fakeBox({ getStatus: 404 });
  v = mount(GUEST);
  await settle();
  assert.equal(v.host.querySelector('[data-desk-composer="closed"]').textContent, 'Tom has left this desk, so a reply can’t reach them.');
  v.done();

  fakeBox({ ended: true });
  v = mount(GUEST);
  await settle();
  assert.ok(v.host.querySelector('[data-desk-composer="closed"]'));
  v.done();
});

test('a send that fails keeps the words and says why; a pocket that ended on send closes the box', async () => {
  fakeBox({ replyStatus: 500 });
  let v = mount(GUEST);
  await settle();
  let box = v.host.querySelector('textarea[data-desk-reply]');
  type(box, 'On my way');
  await settle();
  box.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  await settle();
  assert.equal(v.host.querySelector('[data-desk-error]').textContent, 'The box had a problem.');
  assert.equal(box.value, 'On my way');
  assert.equal(v.host.querySelectorAll('[data-line]').length, 0, 'no echo is left behind');
  v.done();

  fakeBox({ replyStatus: 410 });
  v = mount(GUEST);
  await settle();
  box = v.host.querySelector('textarea[data-desk-reply]');
  type(box, 'On my way');
  await settle();
  box.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  await settle();
  assert.ok(v.host.querySelector('[data-desk-composer="closed"]'));
  v.done();
});

test('no guest message yet: nothing to send to, so no box', async () => {
  const calls = fakeBox();
  const { host, done } = mount({ code: null, guestId: null, guestName: 'Tom' });
  await settle();
  assert.equal(host.querySelector('textarea'), null);
  assert.equal(calls.length, 0);
  done();
});

// ---- ChatView wiring (source): the desk composer replaces the bot's, and nothing in it starts a turn
test('ChatView swaps in the desk composer and the chip; the desk composer never calls onSendMessage', () => {
  const view = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const comp = readFileSync(new URL('../src/components/DeskReplyComposer.tsx', import.meta.url), 'utf8');
  assert.match(view, /deskGuest \? \(\s*<DeskReplyComposer\s/);
  assert.match(view, /deskHandoff\(/);
  assert.match(view, /if \(deskGuest && isUser && deskSentTo\(msg\)\)\s*return <DeskSentRow /);
  assert.match(view, /placeSettled\(chatItems\.map\(itemTime\), deskGuest \? desk\.lines : \[\]\)/);
  // F1 (Mira): Reply under Bell's messages is not drawn when deskGuest is set; every ReplyButton is gated on !deskGuest
  assert.match(view, /\{!deskGuest && <ReplyButton /);
  assert.doesNotMatch(view.replace(/\{!deskGuest && <ReplyButton /g, ''), /<ReplyButton /);
  assert.doesNotMatch(comp, /onSendMessage|sendMessage|\/api\/chat/);
  assert.match(comp, /pocketApi\.deskReply\(code, guestId, body\)/);
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /threadCreatedBy=\{shownThread\?\.created_by \?\? null\}/);
});
