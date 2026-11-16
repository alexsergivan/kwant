// 23c (autonomy/others-messages-never-you/brief.md): a "user" message someone else wrote never reads "You".
// A Telegram customer reads "Ann (customer)", a bot's chat-API message "Scout (bot)"; the person (Telegram
// too) and a coding agent stay as before. The avatar, the "...'s message" menu and the quote label follow.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const a = await loadBundled(`export * from './src/agentConnections';`);
const rr = await loadBundled(`export * from './src/reactReply';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
`);

const SCOUT = { id: '0e7bf95d-cb8b-4396-a0d7-bcf14a38ab47', name: 'Scout' };
const bots = [SCOUT, { id: 'k-ada', name: 'Ada' }];
const j = (o) => JSON.stringify(o);
const customer = j({ source: 'telegram-customer', from_customer: 'Ann', customer_chat_id: 4242 });
const customerNoName = j({ source: 'telegram-customer', from_customer: '', customer_chat_id: 4242 });
const botWithId = j({ sent_by_bot: true, sent_by_bot_name: 'Old Scout', sent_by_kwant_id: SCOUT.id });
const botNameOnly = j({ sent_by_bot: true, sent_by_bot_name: 'Scout' });
const viaTelegram = j({ source: 'telegram' });
const agent = j({ from_agent: 'Claude Code' });

test('otherSenderLabel: a Telegram customer, with and without a name', () => {
  assert.equal(a.otherSenderLabel(customer, bots, 'telegram-customer:4242'), 'Ann (customer)');
  assert.equal(a.otherSenderLabel(customerNoName, bots, 'telegram-customer:4242'), 'A customer');
  assert.equal(a.otherSenderLabel(j({ from_customer: null }), bots), 'A customer', 'the marker alone is enough');
  assert.equal(a.otherSenderLabel(j({ source: 'telegram-customer' }), bots), 'A customer', 'the source alone is enough');
  assert.equal(a.otherSenderLabel(j({ from_customer: '  Ann  ' }), bots), 'Ann (customer)', 'trimmed');
});

test('otherSenderLabel: a bot by kwant id (its current name), by name only, and with nothing', () => {
  assert.equal(a.otherSenderLabel(botWithId, bots, 'PlayerHero'), 'Scout (bot)', 'kwant id wins: a renamed bot shows its new name');
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: true, sent_by_kwant_id: 'gone', sent_by_bot_name: 'Vega' }), bots), 'Vega (bot)', 'unknown id: the name it sent');
  assert.equal(a.otherSenderLabel(botNameOnly, bots, 'PlayerHero'), 'Scout (bot)');
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: true }), bots, 'k-ada'), 'Ada (bot)', 'no name sent: the sender id, as a known bot');
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: true }), bots, 'Lin'), 'Lin (bot)', 'a short sender id reads as a name');
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: true }), bots, 'PlayerHero'), 'A bot', "the app's own person id is never a bot name");
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: true }), bots, '4c6677dc-533e-4706-8028-4afa492d95a4'), 'A bot', 'an unknown uuid');
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: true }), []), 'A bot');
  assert.equal(a.otherSenderLabel(j({ sent_by_bot: 'yes', sent_by_bot_name: 'X' }), bots), null, 'only sent_by_bot: true counts');
});

test('otherSenderLabel: the person (also via Telegram) and a coding agent are not someone else', () => {
  assert.equal(a.otherSenderLabel(null, bots, 'PlayerHero'), null);
  assert.equal(a.otherSenderLabel(undefined, bots), null);
  assert.equal(a.otherSenderLabel(j({ attachments: [] }), bots, 'PlayerHero'), null);
  assert.equal(a.otherSenderLabel(viaTelegram, bots, 'PlayerHero'), null, 'the person typing in Telegram stays "You"');
  assert.equal(a.otherSenderLabel(agent, bots, 'PlayerHero'), null, 'the coding agent keeps its own label');
  assert.equal(a.agentSenderLabel(agent), 'Claude Code (coding agent)');
  assert.equal(a.otherSenderLabel('not json', bots), null);
  assert.equal(a.otherSenderLabel('"text"', bots), null);
  assert.equal(a.otherSenderLabel('null', bots), null);
});

test('messageAbout and senderLetter', () => {
  assert.equal(a.messageAbout(null), 'your message');
  assert.equal(a.messageAbout('Ann (customer)'), "Ann (customer)'s message");
  assert.equal(a.senderLetter('Ann (customer)'), 'A');
  assert.equal(a.senderLetter('scout (bot)'), 'S');
  assert.equal(a.senderLetter('A customer'), '', 'no name: no letter (a neutral person icon instead)');
  assert.equal(a.senderLetter('A bot'), '');
});

test("reactReply senderLabel: the quote label names a customer or a bot, the person stays You", () => {
  assert.equal(rr.senderLabel('user', 'telegram-customer:4242', 'Chat', customer), 'Ann (customer)');
  assert.equal(rr.senderLabel('user', 'PlayerHero', 'Chat', botWithId, bots), 'Scout (bot)');
  assert.equal(rr.senderLabel('user', 'PlayerHero', 'Chat', j({ sent_by_bot: true })), 'A bot');
  assert.equal(rr.senderLabel('user', 'PlayerHero', 'Chat', viaTelegram), 'You');
  assert.equal(rr.senderLabel('user', 'PlayerHero', 'Chat'), 'You', 'no meta, as before');
  assert.equal(rr.senderLabel('kwant', 'Ada', 'Chat', customer), 'Ada', 'bot replies unchanged');
});

const chatProps = (messages, kwants = []) => ({
  title: 'Ada', subtitle: '', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants, teams: [],
  onContinueFrom: () => {}, home: { kind: 'bot', bot: { id: 'k-ada', name: 'Ada', title: 'Tech lead', avatar_sprite: 'dev' } },
});
const msg = (id, meta, sender_id = 'PlayerHero') => ({ id, sender_role: 'user', sender_id, text: `text ${id}`, meta, created_at: '2026-11-16T09:00:00Z' });
const header = (html, id) => {
  const at = html.indexOf(`data-message-id="${id}"`);
  assert.ok(at >= 0, id);
  const next = html.indexOf('data-message-id="', at + 10);
  return html.slice(at, next < 0 ? undefined : next);
};

test('ChatView: the customer, the bot, the person, Telegram and the coding agent each read as who wrote it', () => {
  const html = ui.chat(chatProps([
    msg('c1', customer, 'telegram-customer:4242'),
    msg('c2', customerNoName, 'telegram-customer:4243'),
    msg('b1', botWithId),
    msg('p1', null),
    msg('t1', viaTelegram),
    msg('g1', agent),
  ], [{ id: SCOUT.id, name: 'Scout', status: 'idle' }]));
  const c1 = header(html, 'c1');
  assert.match(c1, />Ann \(customer\)</);
  assert.match(c1, /data-sender-letter="A"/, "the customer's letter, not the person's avatar");
  assert.doesNotMatch(c1, />You</);
  assert.match(header(html, 'c2'), />A customer</);
  assert.match(header(html, 'c2'), /data-sender-letter=""/);
  const b1 = header(html, 'b1');
  assert.match(b1, />Scout \(bot\)</);
  assert.match(b1, /data-sender-letter="S"/);
  assert.doesNotMatch(b1, />You</);
  for (const id of ['p1', 't1']) {
    assert.match(header(html, id), /<span class="text-zinc-200 shrink-0">You<\/span>/, id);
    assert.doesNotMatch(header(html, id), /data-sender-letter/, id);
  }
  assert.match(header(html, 't1'), /Telegram/i, 'ViaTelegram stays on the person');
  assert.match(header(html, 'g1'), /Claude Code \(coding agent\)/);
  assert.doesNotMatch(header(html, 'g1'), /data-sender-letter/);
  const you = html.match(/>You<\/span>/g) ?? [];
  assert.equal(you.length, 2, 'only the person, typed here and in Telegram');
});

test("ChatView: the message menu is the author's, never \"your message\" for someone else", () => {
  const html = ui.chat(chatProps([msg('c1', customer, 'telegram-customer:4242'), msg('p1', null)]));
  assert.match(html, /aria-label="More for Ann \(customer\)&#x27;s message"/);
  assert.doesNotMatch(header(html, 'c1'), /More for your message/);
  assert.match(header(html, 'p1'), /aria-label="More for your message"/, "the person's own menu as before");
});
