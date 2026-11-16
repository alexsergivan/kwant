// 25b (autonomy/others-messages-own-side/brief.md): only what you wrote sits on your side of the chat.
// A Telegram customer's message and a bot's chat-API message sit on the left in a neutral bubble with their letter;
// yours, yours from Telegram and a coding agent's stay on the right in your bubble. Content rules are unchanged.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
`);

const SCOUT = { id: '0e7bf95d-cb8b-4396-a0d7-bcf14a38ab47', name: 'Scout', status: 'idle' };
const j = (o) => JSON.stringify(o);
const customer = j({ source: 'telegram-customer', from_customer: 'Ann', customer_chat_id: 4242 });
const bot = j({ sent_by_bot: true, sent_by_bot_name: 'Scout', sent_by_kwant_id: SCOUT.id });
const viaTelegram = j({ source: 'telegram' });
const agent = j({ from_agent: 'Claude Code' });
const file = { id: '3f2b8c1e-7d4a-4b9e-9c1a-2e5f6a7b8c9d', name: 'receipt-10482.pdf', mime: 'application/pdf', size: 48213 };
const msg = (id, meta, sender_id = 'PlayerHero', role = 'user') => ({
  id, sender_role: role, sender_id, text: `text of ${id}`, meta, created_at: '2026-11-16T09:00:00Z',
});
const props = (messages) => ({
  title: 'Support', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: async () => true, onResolveApproval: () => {},
  kwants: [{ id: 'k-support', name: 'Support', status: 'idle' }, SCOUT], teams: [], onContinueFrom: () => {},
  home: { kind: 'bot', bot: { id: 'k-support', name: 'Support' } },
});
// One message's markup: from its row to the next row.
const row = (html, id) => {
  const at = html.indexOf(`data-message-id="${id}"`);
  assert.ok(at > 0, `row ${id}`);
  const start = html.lastIndexOf('<div', at);
  const next = html.indexOf('data-message-id="', at + 10);
  return html.slice(start, next < 0 ? undefined : html.lastIndexOf('<div', next));
};
const rowClass = (r) => r.match(/^<div[^>]*class="([^"]*)"/)[1];
const YOUR_BUBBLE = /bg-zinc-800 text-zinc-100 rounded-tr-sm/;

const html = ui.chat(props([
  msg('c1', customer, 'telegram-customer:4242'),
  msg('b1', bot),
  msg('p1', null),
  msg('t1', viaTelegram),
  msg('g1', agent),
  msg('k1', null, 'Support', 'kwant'),
]));

test("a customer's and a bot's chat-API message sit on the left in the other bubble", () => {
  for (const id of ['c1', 'b1']) {
    const r = row(html, id);
    assert.doesNotMatch(rowClass(r), /flex-row-reverse|ml-auto/, `${id}: not on your side`);
    assert.match(r, /items-start/, `${id}: column starts on the left`);
    assert.doesNotMatch(r, /items-end/, id);
    assert.match(r, /data-other-bubble=""/, `${id}: the other bubble`);
    assert.match(r, /bg-zinc-900\/40 text-zinc-200 rounded-tl-sm/, `${id}: neutral, left corner`);
    assert.doesNotMatch(r, YOUR_BUBBLE, `${id}: never your bubble`);
    assert.doesNotMatch(r, /violet/, `${id}: no violet`);
    assert.match(r, /data-sender-letter=/, `${id}: still the letter avatar`);
  }
});

test('yours, yours from Telegram and a coding agent stay on the right in your bubble', () => {
  for (const id of ['p1', 't1', 'g1']) {
    const r = row(html, id);
    assert.match(rowClass(r), /ml-auto flex-row-reverse/, `${id}: your side`);
    assert.match(r, /items-end/, id);
    assert.match(r, YOUR_BUBBLE, id);
    assert.doesNotMatch(r, /data-other-bubble/, id);
  }
});

test("a bot's normal reply is untouched", () => {
  const r = row(html, 'k1');
  assert.doesNotMatch(rowClass(r), /flex-row-reverse/);
  assert.match(r, /flex-1 max-w-4xl items-start/);
  assert.doesNotMatch(r, /data-other-bubble/);
});

test("an other's menu opens from the right gutter and still names them; yours from the left", () => {
  const c1 = row(html, 'c1');
  assert.match(c1, /absolute left-full bottom-0 ml-1" data-message-more-gutter/);
  assert.match(c1, /aria-label="More for Ann \(customer\)&#x27;s message"/);
  assert.match(row(html, 'b1'), /aria-label="More for Scout \(bot\)&#x27;s message"/);
  const p1 = row(html, 'p1');
  assert.match(p1, /absolute right-full bottom-0 mr-1" data-message-more-gutter/);
  assert.match(p1, /aria-label="More for your message"/);
});

test("an other's attached file sits on their side; yours on yours", () => {
  const f = ui.chat(props([
    msg('c2', j({ ...JSON.parse(customer), attachments: [file] }), 'telegram-customer:4242'),
    msg('p2', j({ attachments: [file] })),
  ]));
  const c2 = row(f, 'c2');
  const p2 = row(f, 'p2');
  assert.match(c2, /receipt-10482\.pdf/);
  assert.match(p2, /receipt-10482\.pdf/);
  // FileTiles: align 'start' for others, 'end' for yours
  const tiles = (r) => r.slice(r.indexOf('receipt-10482.pdf') - 1500, r.indexOf('receipt-10482.pdf'));
  assert.doesNotMatch(tiles(c2), /justify-end|items-end|ml-auto/, 'customer file on the left');
  assert.match(tiles(p2), /justify-end|items-end|ml-auto/, 'your file on the right');
});
