// #1278 chat-time-every-row: your messages, other people's user-side messages and a working bot's row show their
// time like a bot reply does, from the one formatter (localClock), in a <time dateTime>; the live row's time is the
// turn's start and is taken once.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { MessageTime } from './src/components/ReplyMeta';
  export { localClock } from './src/clock';
  export { liveTurnStart } from './src/workingStrip';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  export const time = (props) => renderToStaticMarkup(React.createElement(MessageTime, props));
`);

const SCOUT = { id: '0e7bf95d-cb8b-4396-a0d7-bcf14a38ab47', name: 'Scout', status: 'idle' };
const j = (o) => JSON.stringify(o);
// Today, so the row shows only the time, as a reply does today.
const AT = new Date(new Date().setHours(8, 6, 0, 0)).toISOString();
const msg = (id, meta, sender_id = 'PlayerHero', role = 'user') => ({ id, sender_role: role, sender_id, text: `text of ${id}`, meta, created_at: AT });
const props = (messages, extra = {}) => ({
  title: 'Support', messages, streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], onSendMessage: async () => true, onResolveApproval: () => {},
  kwants: [{ id: 'k-support', name: 'Support', status: 'idle' }, SCOUT], teams: [], onContinueFrom: () => {},
  home: { kind: 'bot', bot: { id: 'k-support', name: 'Support' } }, ...extra,
});
const row = (html, id) => {
  const at = html.indexOf(`data-message-id="${id}"`);
  assert.ok(at > 0, `row ${id}`);
  const start = html.lastIndexOf('<div', at);
  const next = html.indexOf('data-message-id="', at + 10);
  return html.slice(start, next < 0 ? undefined : html.lastIndexOf('<div', next));
};
const timeOf = (r, attr) => r.match(new RegExp(`<time [^>]*${attr}[^>]*>(?:<span[^>]*>· </span>)?([^<]+)</time>`))?.[1];

const html = ui.chat(props([
  msg('p1', null),
  msg('c1', j({ source: 'telegram-customer', from_customer: 'Ann', customer_chat_id: 4242 }), 'telegram-customer:4242'),
  msg('b1', j({ sent_by_bot: true, sent_by_bot_name: 'Scout', sent_by_kwant_id: SCOUT.id })),
  msg('g1', j({ from_agent: 'Claude Code' })),
  msg('t1', j({ source: 'telegram' })),
  msg('k1', null, 'Support', 'kwant'),
]));

test('your message shows the same time as a bot reply, from the same formatter', () => {
  const reply = timeOf(row(html, 'k1'), 'data-reply-time');
  const mine = timeOf(row(html, 'p1'), 'data-message-time="user"');
  assert.ok(reply, 'the reply has its time');
  assert.equal(mine, reply);
  assert.equal(mine, ui.localClock(AT));
  assert.doesNotMatch(mine, /,/, 'today: the time only, no date');
});

test("other people's user-side rows (customer, bot, coding agent, you via Telegram) get the time too", () => {
  for (const id of ['c1', 'b1', 'g1', 't1']) assert.equal(timeOf(row(html, id), 'data-message-time="user"'), ui.localClock(AT), id);
});

test('a <time dateTime> for screen readers; on your side the header mirrors so the name stays by your avatar', () => {
  const mine = row(html, 'p1');
  assert.match(mine, new RegExp(`<time class="shrink-0 whitespace-nowrap text-xs text-zinc-400" data-message-time="user" dateTime="${AT}">`));
  const header = mine.match(/<div class="([^"]*)"[^>]*>(?:<span[^>]*>You<\/span>)/)?.[1] ?? '';
  assert.match(header, /whitespace-nowrap/, 'one line, no new wrapping');
  assert.match(header, /flex-row-reverse/, 'your side mirrors');
  // DOM order is name then time, so a screen reader says "You, 8:06 AM".
  assert.ok(mine.indexOf('>You</span>') < mine.indexOf('data-message-time="user"'));
  assert.doesNotMatch(row(html, 'c1').match(/data-sender="other"[^]*?<\/div>/)?.[0] ?? '', /flex-row-reverse/);
  const header2 = row(html, 'c1').match(/<div class="([^"]*)" data-sender="other"/)?.[1] ?? '';
  assert.doesNotMatch(header2, /flex-row-reverse/, "someone else's row stays left to right");
});

test('a row on another day shows the date like a reply does; no time without a readable created_at', () => {
  const old = '2026-01-05T08:06:00Z';
  const h = ui.time({ at: old, kind: 'user' });
  assert.match(h, /data-message-time="user" dateTime="2026-01-05T08:06:00.000Z">/);
  assert.ok(h.includes(`>${ui.localClock(old)}<`));
  assert.match(ui.localClock(old), /,/);
  assert.equal(ui.time({ at: null, kind: 'user' }), '');
  assert.equal(ui.time({ at: 'nonsense', kind: 'user' }), '');
});

test('a working bot shows when the turn started, in the place and style the reply time takes; not while in line', () => {
  const before = Date.now();
  const live = ui.chat(props([msg('p1', null)], { isStreaming: true, streamingSpeakerName: 'Support' }));
  const line = live.slice(live.indexOf('data-live-name-line=""'));
  const t = line.match(/<time class="shrink-0 whitespace-nowrap text-zinc-400" data-message-time="live" dateTime="([^"]+)"><span aria-hidden="true" class="sm:hidden">· <\/span>([^<]+)<\/time>/);
  assert.ok(t, 'live time');
  assert.ok(Date.parse(t[1]) >= before - 1000 && Date.parse(t[1]) <= Date.now() + 1000, 'started now');
  assert.equal(t[2], ui.localClock(t[1]));
  // Same element classes and phone dot as the finished reply's time, so nothing moves when it lands.
  const reply = ui.time({ at: t[1] });
  assert.equal(reply.replace(' data-reply-time=""', ''), t[0].replace(' data-message-time="live"', ''));
  const queued = ui.chat(props([msg('p1', null)], { isStreaming: true, queued: { position: 2, reason: 'busy' } }));
  assert.doesNotMatch(queued, /data-message-time="live"/);
});

test('the start is taken once; only a step from well before (chat reopened mid-turn) moves it back', () => {
  const shown = Date.parse('2026-11-26T08:06:01Z');
  const step = (at) => ({ turnId: 't', steps: [{ started_at: at }] });
  assert.equal(ui.liveTurnStart(null, shown), shown);
  assert.equal(ui.liveTurnStart(step('2026-11-26T08:05:58Z'), shown), shown, 'a few seconds of clock drift: no flip');
  assert.equal(ui.liveTurnStart(step('2026-11-26T08:06:20Z'), shown), shown, 'later steps never move it');
  assert.equal(ui.liveTurnStart(step('2026-11-26T07:58:00Z'), shown), Date.parse('2026-11-26T07:58:00Z'));
});

test('one formatter: the rows use MessageTime, nobody formats a time of their own', () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  assert.match(chat, /<MessageTime at=\{msg\.created_at\} kind="user" \/>/);
  assert.match(chat, /<MessageTime at=\{liveStartedAt\} kind="live" \/>/);
  const meta = readFileSync(new URL('../src/components/ReplyMeta.tsx', import.meta.url), 'utf8');
  assert.match(meta, /<MessageTime at=\{at\} now=\{now\} \/>/);
  assert.equal(meta.match(/localClock\(/g).length, 1);
});
