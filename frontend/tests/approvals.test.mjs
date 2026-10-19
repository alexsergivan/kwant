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
  assert.match(html, /Dev<\/span> wants to run a command/);
  assert.match(html, /<pre[^>]*>touch x<\/pre>/);
  assert.match(html, /In folder:.*\/tmp\/bots\/dev/);
  assert.match(html, /Allow Once/);
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
  assert.match(html, /Dev<\/span> wants to use <span[^>]*>Bash<\/span>/);
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
