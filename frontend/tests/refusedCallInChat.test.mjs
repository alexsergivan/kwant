// 22d: a customer's call nobody allowed (a closed `refused` row with 22a's `link`) is one quiet line in
// that customer's chat, and nowhere else. The Telegram "Allowed once" line is unchanged.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const a = await loadBundled(`export * from './src/approvals';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  export { refusedCallText } from './src/components/RefusedCustomerCall';
  export { localClock } from './src/clock';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
`);

const now = Date.parse('2026-11-15T22:00:00Z');
const thread = { thread_id: 'th-ann', channel_id: null, direct_kwant_id: null };
// Shaped like GET /api/approvals?status=refused (src/server/approval_modes.rs list, 16d details).
const refusedRow = (over = {}) => ({
  id: 'ap_r1', kwant_id: 'k-shop', tool_name: 'mcp__kwnt_tools__memory_search', status: 'refused', answered_via: null,
  created_at: '2026-11-15T21:00:00Z', link: thread,
  details: { source: 'mcp', server: 'kwnt_tools', tool: 'memory_search', arguments: { query: 'x' }, kwant_id: 'k-shop', kwant_name: 'Shop', customer_chat: true, refused: 'nobody_allowed' },
  ...over,
});
const telegramRow = { id: 'ap_t1', kwant_id: 'k-shop', tool_name: 'Bash', status: 'allowed_once', answered_via: 'telegram', created_at: '2026-11-15T21:30:00Z', details: { kwant_name: 'Shop' } };

const customerThread = { kind: 'kwant', kwantId: 'k-shop', threadId: 'th-ann' };
const directChat = { kind: 'kwant', kwantId: 'k-shop', threadId: null };
const chatProps = (settledApprovals) => ({
  title: 'Shop', subtitle: '', messages: [], streamingText: '', streamingThinking: '', isStreaming: false,
  pendingApprovals: [], settledApprovals, onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
});
const LINE = "A customer&#x27;s message tried to use memory search. It didn&#x27;t run, because nobody allowed it.";

test('a refused row with a thread link is drawn only in that thread, as one quiet line with no buttons', () => {
  const settled = a.refusedFromRecords([refusedRow()], now);
  assert.equal(settled.length, 1);
  assert.equal(settled[0].refused, true);
  assert.deepEqual(a.approvalsForChat(settled, directChat), [], "not in the bot's direct chat");
  assert.deepEqual(a.approvalsForChat(settled, { kind: 'kwant', kwantId: 'k-shop', threadId: 'th-bob' }), [], "not in another customer's chat");
  assert.deepEqual(a.approvalsForChat(settled, { kind: 'channel', channelId: 'ch1', memberIds: ['k-shop'] }), []);
  const here = a.approvalsForChat(settled, customerThread);
  assert.equal(here.length, 1);
  const html = ui.chat(chatProps(here));
  assert.ok(html.includes(LINE), html);
  assert.match(html, /data-refused-call="ap_r1"/);
  // Round 2 (Mira): the line ends with " · <time>" from the row's asked_at, like the Telegram line.
  const p = /data-refused-call="ap_r1"[^>]*>[^]*?<p class="[^"]*">([^]*?)<\/p>/.exec(html)[1].replace(/<!-- -->/g, '').replace(/&#x27;/g, "'");
  assert.ok(p.endsWith(` · ${ui.localClock(here[0].asked_at)}`), p);
  assert.ok(p.startsWith(ui.refusedCallText(here[0])), p);
  const noTime = ui.chat(chatProps([{ ...here[0], asked_at: undefined }]));
  assert.ok(noTime.includes(`${LINE}</p>`), 'no asked_at: no time and no dot');
  assert.doesNotMatch(html, /kwnt_tools/, 'no internal server name');
  assert.doesNotMatch(html.slice(html.indexOf('data-refused-call')), /^[^]*?<button[^>]*>[^<]*(Allow|Deny)/, 'nothing to answer');
  assert.doesNotMatch(html, /Allowed once from Telegram/);
});

test('a refused row without a chat (stored before 22a), or with a malformed one, is drawn nowhere', () => {
  for (const link of [null, undefined, {}, { thread_id: '', channel_id: null, direct_kwant_id: null }, { thread_id: 'a', channel_id: 'b', direct_kwant_id: null }, 'th-ann']) {
    const settled = a.refusedFromRecords([refusedRow({ link })], now);
    assert.deepEqual(settled, [], JSON.stringify(link));
  }
});

test('only refused rows from the last day; other statuses and junk are ignored; oldest first', () => {
  const rows = [
    refusedRow({ id: 'ap_r3', created_at: '2026-11-15T21:50:00Z' }),
    refusedRow({ id: 'ap_r2', created_at: '2026-11-15T20:00:00Z' }),
    refusedRow({ id: 'ap_old', created_at: '2026-11-13T20:00:00Z' }),
    refusedRow({ id: 'ap_w', status: 'waiting' }),
    refusedRow({ id: 'ap_nt', created_at: null }),
    null,
  ];
  assert.deepEqual(a.refusedFromRecords(rows, now).map((r) => r.id), ['ap_r2', 'ap_r3']);
  assert.deepEqual(a.refusedFromRecords({ error: 'x' }, now), []);
});

test('a channel or direct-chat link places it there, and a non-kwant tool reads like its card', () => {
  const ch = a.refusedFromRecords([refusedRow({ link: { thread_id: null, channel_id: 'ch1', direct_kwant_id: null } })], now);
  assert.equal(a.approvalsForChat(ch, { kind: 'channel', channelId: 'ch1', memberIds: [] }).length, 1);
  assert.equal(a.approvalsForChat(ch, customerThread).length, 0);
  const gh = a.refusedFromRecords([refusedRow({ tool_name: 'mcp__github__create_pull_request', details: { source: 'mcp', server: 'github', tool: 'create_pull_request', arguments: {} } })], now);
  assert.equal(ui.refusedCallText(gh[0]), "A customer's message tried to open a pull request on GitHub. It didn't run, because nobody allowed it.");
});

test('an allowed_once Telegram row still reads "Allowed once from Telegram", and lines sort by time', () => {
  const settled = a.addSettled(a.settledFromRecords([telegramRow], now), a.refusedFromRecords([refusedRow()], now));
  // The Telegram row has no link: it stays in every chat with its bot, exactly as before.
  assert.deepEqual(a.approvalsForChat(settled, directChat).map((r) => r.id), ['ap_t1']);
  const here = a.settledByTime(a.approvalsForChat(settled, customerThread));
  assert.deepEqual(here.map((r) => r.id), ['ap_r1', 'ap_t1'], 'the 21:00 refusal before the 21:30 Telegram allow');
  const html = ui.chat(chatProps(here));
  assert.match(html, /Shop wanted to run a command · <b[^>]*>✓ Allowed once from Telegram<\/b>/); // 33a: the card's own words
  assert.ok(html.indexOf('data-refused-call') < html.indexOf('data-resolved-via="telegram"'));
  // Lines without a time keep their order after the timed ones.
  assert.deepEqual(a.settledByTime([{ id: 'x' }, { id: 'y', asked_at: '2026-11-15T21:00:00Z' }, { id: 'z' }]).map((r) => r.id), ['y', 'x', 'z']);
});
