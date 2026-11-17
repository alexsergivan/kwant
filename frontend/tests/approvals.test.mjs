import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// Sound settings and the chat read localStorage on import.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const a = await loadBundled(`export * from './src/approvals';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ChatView } from './src/components/ChatView';
  import { AwayView } from './src/components/AwayView';
  export const chat = (props) => renderToStaticMarkup(React.createElement(ChatView, props));
  export const away = (props) => renderToStaticMarkup(React.createElement(AwayView, props));
`);

// Shaped like a pending claude_builtin ticket in GET /api/away (BrokerApprover in mcp_runtime.rs).
const bashItem = (over = {}) => ({
  kind: 'approval', id: 'ap1', kwant_id: 'k-dev', kwant_name: 'Dev', title: 'Use Bash', status: 'pending',
  summary: 'command: touch x, description: Create x', timestamp: '2026-10-19T21:00:00Z',
  link: { thread_id: null, channel_id: null, direct_kwant_id: null },
  tool_name: 'Bash',
  details: {
    source: 'claude_builtin', tool: 'Bash', arguments: { command: 'touch x', description: 'Create x' },
    cwd: '/tmp/bots/dev', outside_folder: false, kwant_id: 'k-dev', kwant_name: 'Dev',
  },
  ...over,
});
const awayResp = (waiting) => ({
  since: '2026-10-12T21:00:00Z', last_seen: null, now: '2026-10-19T21:05:00Z', stuck_after_minutes: 30,
  waiting, finished: [], failed: [], unseen: { waiting: waiting.length, finished: 0, failed: 0, total: waiting.length },
});

const chatProps = (pendingApprovals) => ({
  title: 'Dev', subtitle: 'Backend', messages: [], streamingText: '', streamingThinking: '', isStreaming: true,
  pendingApprovals, onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
});

test('a Bash approval loaded after a reload renders as the built-in card inline in the bot chat', () => {
  // The page opens after the ApprovalRequired event already fired: only the server's list has it.
  const loaded = a.syncApprovals([], a.approvalsFromAway(awayResp([bashItem()]).waiting));
  const here = a.approvalsForChat(loaded, { kind: 'kwant', kwantId: 'k-dev', threadId: null });
  assert.equal(here.length, 1);
  const html = ui.chat(chatProps(here));
  assert.match(html, /Dev<\/strong> wants to run a command/);
  assert.match(html, /<pre[^>]*>touch x<\/pre>/);
  assert.match(html, /In folder:.*\/tmp\/bots\/dev/);
  assert.match(html, /Allow once/);
  assert.match(html, /Always this session/);
  assert.match(html, /Deny/);
  assert.doesNotMatch(html, /command: touch x/, 'not the old raw summary');
});

test('the card stays out of other chats, but shows in a channel the bot is in', () => {
  const loaded = a.approvalsFromAway([bashItem()]);
  assert.equal(a.approvalsForChat(loaded, { kind: 'kwant', kwantId: 'k-other' }).length, 0);
  assert.equal(a.approvalsForChat(loaded, { kind: 'channel', channelId: 'c1', memberIds: ['k-x'] }).length, 0);
  assert.equal(a.approvalsForChat(loaded, { kind: 'channel', channelId: 'c1', memberIds: ['k-dev'] }).length, 1);
  assert.equal(a.approvalsForChat(loaded, null).length, 0);
  // With the conversation known, only there.
  const linked = a.approvalsFromAway([bashItem({ link: { thread_id: null, channel_id: 'c2', direct_kwant_id: null } })]);
  assert.equal(a.approvalsForChat(linked, { kind: 'kwant', kwantId: 'k-dev' }).length, 0);
  assert.equal(a.approvalsForChat(linked, { kind: 'channel', channelId: 'c2', memberIds: [] }).length, 1);
});

test('While you were away shows the same card and labels, not raw text', () => {
  const html = ui.away({
    data: awayResp([bashItem()]), loading: false, error: null, busy: null, notice: null,
    onRefresh: () => {}, onOpen: () => {}, onAnswer: () => {}, onResolve: () => {},
  });
  assert.match(html, /<pre[^>]*>touch x<\/pre>/);
  assert.match(html, /In folder:.*\/tmp\/bots\/dev/);
  assert.match(html, /Always this session/);
  assert.doesNotMatch(html, /Always allow</, 'no old lowercase raw-button row');
  assert.doesNotMatch(html, /command: touch x/);
});

test('an older server without details still gets a card with the summary and session label', () => {
  const old = bashItem({ tool_name: undefined, details: undefined });
  const [card] = a.approvalsFromAway([old]);
  assert.equal(card.tool_name, 'Bash');
  assert.equal(card.kwant_id, 'k-dev');
  const html = ui.chat(chatProps([card]));
  assert.match(html, /Dev<\/strong> wants to run a command/);
  assert.match(html, /command: touch x, description: Create x/);
  assert.match(html, /Always this session/);
});

test('live and loaded lists merge: answered ones drop, live details beat a summary, a fresh live one stays', () => {
  const live = a.approvalFromEvent({ id: 'ap1', tool_name: 'Bash', details: bashItem().details });
  const fresh = a.approvalFromEvent({ id: 'ap2', tool_name: 'Bash', details: { ...bashItem().details, arguments: { command: 'rm y' } } });
  const gone = a.approvalFromEvent({ id: 'ap0', tool_name: 'Bash', details: bashItem().details });
  const summaryOnly = a.approvalsFromAway([bashItem({ tool_name: undefined, details: undefined })]);
  const next = a.syncApprovals([gone, live, fresh], summaryOnly, new Set(['ap2']));
  assert.deepEqual(next.map((x) => x.id), ['ap1', 'ap2']);
  assert.equal(next[0].details.source, 'claude_builtin', 'kept the live details');
  assert.deepEqual(a.withoutApproval(next, 'ap1').map((x) => x.id), ['ap2']);
  // Two asks at once both show.
  assert.equal(a.upsertApproval([live], fresh).length, 2);
});

test('a reloaded payment keeps its reason in every form the server sends, so no Always button', () => {
  const reason = 'POST to a payment host (api.stripe.com)';
  const forms = [
    bashItem({ details: { ...bashItem().details, payment_reason: reason } }),
    bashItem({ tool_name: undefined, details: undefined, payment_reason: reason }),
  ];
  for (const item of forms) {
    const card = a.approvalFromAway(item);
    assert.equal(card.details.payment_reason, reason);
    const html = ui.away({
      data: awayResp([item]), loading: false, error: null, busy: null, notice: null,
      onRefresh: () => {}, onOpen: () => {}, onAnswer: () => {}, onResolve: () => {},
    });
    assert.match(html, /This looks like a payment:<\/span> POST to a payment host \(api.stripe.com\)\./);
    assert.doesNotMatch(html, /Always this session|Always allow/);
  }
  // An old summary item without a reason is unchanged.
  const plain = a.approvalFromAway(bashItem({ tool_name: undefined, details: undefined }));
  assert.equal(plain.details.source, 'summary');
  assert.equal('payment_reason' in plain.details, false);
});

// ha-sheet-polish (b): the Needs-you strip and the sidebar pin tip say the card's sentence, never the
// server's raw "Use intent__HassTurnOff (home-assistant)" (Mira's 0.9.21 follow-up 2).
// Shaped like the 0.9.21 shot rig's lock ask (homeAssistant.test.mjs's haTicket, as an Inbox item).
const haItem = (args, over = {}) => ({
  kind: 'approval', id: 'h1', kwant_id: 'k-house', kwant_name: 'House', status: 'waiting',
  title: 'Use intent__HassTurnOff (home-assistant)', summary: '',
  tool_name: 'mcp__home-assistant__intent__HassTurnOff',
  details: { source: 'mcp', server: 'home-assistant', tool: 'intent__HassTurnOff', arguments: args },
  ...over,
});

test('approvalLine: a Home Assistant lock ask reads like its card, even when the server set a question', () => {
  const lock = { name: 'Front door', area: 'Hallway', domain: ['lock'] };
  assert.equal(a.approvalLine(haItem(lock)), 'House wants to unlock Front door');
  assert.equal(a.approvalLine(haItem(lock, { question: 'Turn off “Front door”?' })), 'House wants to unlock Front door');
  assert.equal(a.approvalLine(haItem({ name: 'Living room lamp' })), 'House wants to turn off Living room lamp');
  // The always-ask floor with no domain never reads "turn off"
  const floor = haItem({ name: 'Front door' });
  floor.details.home_always_ask = true;
  assert.equal(a.approvalLine(floor), 'House wants to unlock or open Front door');
  // No bot name: the action alone, capitalised, as awayTitle does
  assert.equal(a.approvalLine(haItem(lock, { kwant_name: '' })), 'Unlock Front door');
});

test('approvalLine: a raw "Use <tool> (<server>)" title with no question gives the card sentence; anything else keeps its line', () => {
  const gh = { kind: 'approval', id: 'g1', kwant_id: 'k-ann', kwant_name: 'Ann', title: 'Use create_pull_request (github)', summary: '', status: 'waiting' };
  assert.equal(a.approvalLine(gh), 'Ann wants to open a pull request');
  assert.equal(a.approvalLine(bashItem()), 'Dev wants to run a command');
  // A question the server wrote wins for a non-home tool; so does a title already in words
  assert.equal(a.approvalLine({ ...gh, question: 'Open “Fix checkout” on demo-app?' }), null);
  assert.equal(a.approvalLine({ ...gh, title: 'Wants to run a command' }), null);
  // Not an approval: never touched
  assert.equal(a.approvalLine({ kind: 'stopped', id: 's1', kwant_id: 'k', kwant_name: 'Q', title: 'Use the retry', summary: '', status: 'open' }), null);
});
