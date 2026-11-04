// Thumbs and Reply on bot messages (react-reply contract §3): the toggle, the excerpt, the save with
// rollback, and ChatView's chip, Esc, send and quote, rendered in JSDOM.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
if (!g.CSS) g.CSS = { escape: (s) => s };
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
g.ResizeObserver = g.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.IntersectionObserver = g.window.IntersectionObserver = class { observe() {} unobserve() {} disconnect() {} };
g.window.HTMLElement.prototype.scrollIntoView = function () { g.__scrolled = this.getAttribute('data-message-id'); };
g.fetch = g.window.fetch = async () => new Response('[]', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/reactReply';
  export { ChatView } from './src/components/ChatView';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = () => new Promise((r) => setImmediate(r));

test('a pressed thumb clears; the other one switches', () => {
  assert.equal(m.nextReaction(null, 'up'), 'up');
  assert.equal(m.nextReaction('up', 'up'), null);
  assert.equal(m.nextReaction('up', 'down'), 'down');
  assert.equal(m.nextReaction('down', 'down'), null);
});

test('missing or odd reaction and reply_to read as none', () => {
  assert.equal(m.reactionOf(undefined), null);
  assert.equal(m.reactionOf('love'), null);
  assert.equal(m.reactionOf('down'), 'down');
  assert.equal(m.replyRefOf(undefined), null);
  assert.equal(m.replyRefOf({}), null);
  assert.deepEqual(m.replyRefOf({ id: 'a', missing: true }), { id: 'a', missing: true });
  assert.deepEqual(m.replyRefOf({ id: 'a', sender_role: 'kwant', sender_id: 'Ada', excerpt: 'Hi' }), { id: 'a', sender_role: 'kwant', sender_id: 'Ada', excerpt: 'Hi' });
});

test('the excerpt is one plain line, cut with an ellipsis', () => {
  assert.equal(m.excerptOf('# Plan\n\n- **Ship** the [fix](http://x) `now`'), 'Plan Ship the fix now');
  assert.equal(m.excerptOf('```js\ncode()\n```\nAfter'), 'After');
  const cut = m.excerptOf('word '.repeat(80), 40);
  assert.equal(cut.length <= 40, true);
  assert.equal(cut.endsWith('…'), true);
  assert.equal(m.excerptOf('<b>tag</b>'), '<b>tag</b>', 'kept as text, never parsed');
});

test('names: uuids fall back to the chat title, the person is You', () => {
  assert.equal(m.senderLabel('kwant', 'Ada', 'Chat'), 'Ada');
  assert.equal(m.senderLabel('kwant', '4c6677dc-533e-4706-8028-4afa492d95a4', 'Chat'), 'Chat');
  assert.equal(m.senderLabel('user', 'PlayerHero', 'Chat'), 'You');
});

test('saveReaction PUTs the contract body and reports failures', async () => {
  const calls = [];
  const ok = async (url, init) => (calls.push([url, init]), { ok: true, status: 200, json: async () => ({ message_id: 'm 1', reaction: 'up' }) });
  assert.deepEqual(await m.saveReaction(ok, 'm 1', 'up'), { reaction: 'up', answered: false }, 'no `answered` (before the backend PR) reads as false');
  const yes = async () => ({ ok: true, status: 200, json: async () => ({ message_id: 'm', reaction: 'up', answered: true }) });
  assert.deepEqual(await m.saveReaction(yes, 'm', 'up'), { reaction: 'up', answered: true });
  assert.equal(calls[0][0], '/api/messages/m%201/reaction');
  assert.equal(calls[0][1].method, 'PUT');
  assert.deepEqual(JSON.parse(calls[0][1].body), { reaction: 'up' });
  await m.saveReaction(ok, 'x', null);
  assert.deepEqual(JSON.parse(calls[1][1].body), { reaction: null });
  const gone = async () => ({ ok: false, status: 404, json: async () => { throw new Error('no body'); } });
  await assert.rejects(m.saveReaction(gone, 'x', 'up'), /gone/);
  const bad = async () => ({ ok: false, status: 400, json: async () => ({ error: 'reactions are for bot messages' }) });
  await assert.rejects(m.saveReaction(bad, 'x', 'up'), /bot messages/);
});

test('a thumb is a yes or no only on the newest bot message that asks something', () => {
  assert.equal(m.endsWithQuestion('Done.\n\nShall I deploy it?'), true);
  assert.equal(m.endsWithQuestion('Did it work? Yes.\n\nAll done.'), false, 'only the last paragraph counts');
  const q = { id: 'q', sender_role: 'kwant', text: 'Ready. Go ahead?' };
  const done = { id: 'd', sender_role: 'kwant', text: 'Done.' };
  const you = { id: 'u', sender_role: 'user', text: 'ok?' };
  const note = { id: 'n', sender_role: 'system', text: 'note' };
  assert.equal(m.questionMessageId([done, q], false), 'q');
  assert.equal(m.questionMessageId([q, note], false), 'q', 'system notes after it do not count');
  assert.equal(m.questionMessageId([q, done], false), null, 'not the newest bot message');
  assert.equal(m.questionMessageId([q, you], false), null, 'the person already answered');
  assert.equal(m.questionMessageId([q], true), null, 'a reply is running');
  assert.deepEqual(m.reactionLabels(true), { up: 'Yes, go ahead', down: 'No' });
  assert.deepEqual(m.reactionLabels(false), { up: 'Good reply', down: 'Not helpful' });
});

test('bot names for quotes: from the list, else the short id, else "a bot"', () => {
  const bots = [{ id: 'k1', name: 'Ada' }];
  assert.equal(m.botNameOf('k1', bots), 'Ada');
  assert.equal(m.botNameOf('Ada', bots), 'Ada');
  assert.equal(m.botNameOf('Lin', bots), 'Lin');
  assert.equal(m.botNameOf('4c6677dc-533e-4706-8028-4afa492d95a4', bots), 'a bot');
  assert.equal(m.botNameOf(undefined, bots), 'a bot');
});

const bot = { id: 'b1', sender_role: 'kwant', sender_id: 'Ada', text: '**Plan**: ship the fix today' };
const mine = { id: 'u1', sender_role: 'user', sender_id: 'PlayerHero', text: 'Do it', reply_to: { id: 'b1', sender_role: 'kwant', sender_id: 'Ada', excerpt: 'Plan: ship the fix today' } };
const older = { id: 'u0', sender_role: 'user', sender_id: 'PlayerHero', text: 'Old', reply_to: { id: 'gone', missing: true } };
const sys = { id: 's1', sender_role: 'system', sender_id: 'system', text: 'Note' };

function mount(props) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  const sent = [];
  const reacts = [];
  const base = {
    title: 'Ada',
    messages: [older, bot, mine, sys],
    streamingText: '',
    streamingThinking: '',
    pendingApprovals: [],
    isStreaming: false,
    onSendMessage: (text, att, opts) => { sent.push({ text, opts }); return true; },
    onReact: async (msg, r) => (reacts.push([msg.id, r]), g.__reactResult ?? { ok: true, answered: false }),
    onResolveApproval: () => {},
    contextKey: 'k1',
  };
  const render = (p) => flushSync(() => root.render(h(m.ChatView, { ...base, ...props, ...p })));
  render({});
  return { host, root, sent, reacts, render };
}
const type = (ta, value) => {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value').set;
  setter.call(ta, value);
  ta.dispatchEvent(new window.Event('input', { bubbles: true }));
};

test('ChatView: thumbs only on bot messages, toggles with aria-pressed', async () => {
  const v = mount({});
  await settle();
  const thumbs = v.host.querySelectorAll('[data-reaction]');
  assert.equal(thumbs.length, 2, 'one bot message, two thumbs; none on the person or system notes');
  const up = v.host.querySelector('[data-reaction="up"]');
  assert.equal(up.getAttribute('aria-pressed'), 'false');
  assert.equal(up.getAttribute('aria-label'), 'Good reply');
  assert.equal(v.host.querySelector('[data-reaction="down"]').getAttribute('aria-label'), 'Not helpful');
  assert.equal(up.closest('[role="group"]').getAttribute('aria-label'), 'React to this reply');
  flushSync(() => up.click());
  assert.deepEqual(v.reacts.at(-1), ['b1', 'up']);
  v.render({ messages: [older, { ...bot, reaction: 'up' }, mine, sys] });
  const up2 = v.host.querySelector('[data-reaction="up"]');
  assert.equal(up2.getAttribute('aria-pressed'), 'true');
  assert.equal(up2.getAttribute('title'), 'Good reply · click to undo');
  flushSync(() => up2.click());
  assert.deepEqual(v.reacts.at(-1), ['b1', null], 'pressing the selected thumb clears it');
  flushSync(() => v.host.querySelector('[data-reaction="down"]').click());
  assert.deepEqual(v.reacts.at(-1), ['b1', 'down']);
  v.root.unmount();
});

test('ChatView: Reply opens the chip, Esc cancels, send carries the reply and clears it', async () => {
  const v = mount({});
  await settle();
  const reply = [...v.host.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === 'Reply to Ada');
  assert.ok(reply);
  flushSync(() => reply.click());
  const chip = v.host.querySelector('[data-reply-chip]');
  assert.ok(chip);
  assert.match(chip.textContent, /Replying toAda:Plan: ship the fix today/);
  assert.equal(chip.getAttribute('aria-live'), 'polite');
  assert.equal(v.host.querySelector('form').querySelector('[data-reply-chip]'), chip, 'the chip is inside the composer');
  assert.ok(reply.className.includes('text-sky-200'), 'the Reply being written is marked');
  const ta = v.host.querySelector('textarea[data-composer]');
  assert.equal(document.activeElement, ta, 'focus goes to the composer');
  flushSync(() => ta.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  assert.equal(v.host.querySelector('[data-reply-chip]'), null, 'Esc cancels');

  flushSync(() => reply.click());
  flushSync(() => v.host.querySelector('[aria-label="Cancel reply"]').click());
  assert.equal(v.host.querySelector('[data-reply-chip]'), null, 'the x cancels');

  flushSync(() => reply.click());
  flushSync(() => type(ta, 'Yes, go'));
  flushSync(() => ta.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
  assert.equal(v.sent.length, 1);
  assert.equal(v.sent[0].text, 'Yes, go');
  assert.equal(v.sent[0].opts.replyTo.id, 'b1');
  assert.equal(v.host.querySelector('[data-reply-chip]'), null, 'cleared after send');

  flushSync(() => type(ta, 'Plain'));
  flushSync(() => ta.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
  assert.equal(v.sent[1].opts, undefined, 'the next message is not a reply');
  v.root.unmount();
});

test('ChatView: switching chats drops the reply', async () => {
  const v = mount({});
  await settle();
  flushSync(() => [...v.host.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === 'Reply to Ada').click());
  assert.ok(v.host.querySelector('[data-reply-chip]'));
  v.render({ contextKey: 'k2' });
  await settle();
  assert.equal(v.host.querySelector('[data-reply-chip]'), null);
  v.root.unmount();
});

test('ChatView: the quote in the bubble opens the original; a deleted one says so', async () => {
  const v = mount({});
  await settle();
  const quote = v.host.querySelector('button[data-reply-quote="b1"]');
  assert.ok(quote, 'a loaded original makes the quote a button');
  assert.match(quote.textContent, /AdaPlan: ship the fix today/);
  assert.match(quote.getAttribute('aria-label'), /Show the original$/);
  g.__scrolled = null;
  flushSync(() => quote.click());
  await new Promise((r) => setTimeout(r, 30));
  assert.equal(g.__scrolled, 'b1');
  const row = v.host.querySelector('[data-message-id="b1"]');
  assert.ok(row.className.includes('kw-flash-quote'), 'violet ring, not the amber search flash');
  assert.equal(document.activeElement, row, 'focus lands on the original');
  const gone = v.host.querySelector('[data-reply-quote="gone"]');
  assert.equal(gone.tagName, 'DIV');
  assert.match(gone.textContent, /deleted/);
  v.root.unmount();
});

test('ChatView: a quote whose original is not loaded says so instead of scrolling', async () => {
  const v = mount({ messages: [mine] });
  await settle();
  g.__scrolled = null;
  flushSync(() => v.host.querySelector('[data-reply-quote="b1"]').click());
  await settle();
  assert.equal(g.__scrolled, null);
  assert.match(v.host.querySelector('[role="status"]').textContent, /isn't loaded here/);
  v.root.unmount();
});

test('ChatView: thumbs on the newest question read yes/no, and go back to feedback once answered', async () => {
  const q = { id: 'q1', sender_role: 'kwant', sender_id: 'Ada', text: 'Tests pass.\n\nShall I open the PR?' };
  const v = mount({ messages: [bot, q] });
  await settle();
  const upQ = v.host.querySelector('[data-message-reactions="q1"] [data-reaction="up"]');
  assert.equal(upQ.getAttribute('aria-label'), 'Yes, go ahead');
  assert.equal(v.host.querySelector('[data-message-reactions="q1"] [data-reaction="down"]').getAttribute('aria-label'), 'No');
  assert.equal(v.host.querySelector('[data-message-reactions="b1"] [data-reaction="up"]').getAttribute('aria-label'), 'Good reply', 'older messages are feedback');
  g.__reactResult = { ok: true, answered: true };
  flushSync(() => upQ.click());
  await settle();
  await settle();
  g.__reactResult = undefined;
  assert.equal(v.host.querySelector('[data-message-reactions="q1"] [data-reaction="up"]').getAttribute('aria-label'), 'Good reply');
  v.render({ messages: [bot, q], isStreaming: true });
  v.root.unmount();
});

test('ChatView: a failed save says "Not saved" next to the thumbs', async () => {
  const v = mount({});
  await settle();
  g.__reactResult = { ok: false, answered: false, error: 'HTTP 500' };
  flushSync(() => v.host.querySelector('[data-reaction="down"]').click());
  await settle();
  await settle();
  g.__reactResult = undefined;
  assert.match(v.host.querySelector('[data-message-reactions="b1"]').parentElement.textContent, /Not saved\. Try again\./);
  v.root.unmount();
});

test('ChatView: a send that fails keeps the reply chip', async () => {
  const v = mount({ onSendMessage: async () => false });
  await settle();
  flushSync(() => v.host.querySelector('[data-reply-button]').click());
  const ta = v.host.querySelector('textarea[data-composer]');
  flushSync(() => type(ta, 'Go'));
  flushSync(() => ta.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true })));
  await settle();
  await settle();
  assert.ok(v.host.querySelector('[data-reply-chip]'));
  v.root.unmount();
});

test('a message_added for a known bot message updates its thumbs (the backend sends reactions this way)', async () => {
  const { readFileSync } = await import('node:fs');
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  // No invented reaction events: the full updated message arrives as message_added and replaces by id.
  assert.doesNotMatch(app, /case '(reaction_updated|ReactionUpdated|message_reaction|MessageReaction|message_reaction_updated|MessageReactionUpdated)'/);
  assert.match(app, /case 'message_added':[\s\S]*?if \(list\.some\(\(x\) => x\.id === m\.id\)\) \{\s*const replaced = list\.map\(\(x\) => \(x\.id === m\.id \? m : x\)\)/);
  const v = mount({});
  await settle();
  assert.equal(v.host.querySelector('[data-reaction="down"]').getAttribute('aria-pressed'), 'false');
  // What App does with the event: the same id is replaced by the server's copy.
  const event = { type: 'message_added', data: { message: { ...bot, reaction: 'down' } } };
  const list = [older, bot, mine, sys].map((x) => (x.id === event.data.message.id ? event.data.message : x));
  v.render({ messages: list });
  assert.equal(v.host.querySelector('[data-reaction="down"]').getAttribute('aria-pressed'), 'true');
  assert.equal(v.host.querySelector('[data-reaction="up"]').getAttribute('aria-pressed'), 'false');
  v.render({ messages: list.map((x) => (x.id === 'b1' ? { ...x, reaction: null } : x)) });
  assert.equal(v.host.querySelector('[data-reaction="down"]').getAttribute('aria-pressed'), 'false', 'a cleared reaction from another window clears here');
  v.root.unmount();
});
