// 34i (Mira's UX pass on connections, section 3): the "suggests a tool" card reads ready / needs_sign_in /
// removed from /api/connections, so a reload, another device or a delete no longer shows a false "Ready".
// The install's answer is only the optimistic value until a list fetched after it answers; a list error
// keeps today's behaviour and never shows "Removed".
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/suggestionServerState';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { McpSuggestionCard } from './src/components/McpSuggestionCard';
  export const render = (props) => renderToStaticMarkup(React.createElement(McpSuggestionCard, props));
`);

const rows = [
  { id: 'mcp:tasks', kind: 'mcp', status: 'needs_sign_in' },
  { id: 'mcp:echo', kind: 'mcp', status: 'connected' },
  { id: 'mcp:off1', kind: 'mcp', status: 'off' },
  { id: 'mcp:new', kind: 'mcp', status: 'unknown' },
  { id: 'account:claude', kind: 'account', status: 'needs_sign_in' },
  { id: 'mcp:', status: 'connected' },
  null,
];

test('MCP rows map to ready or needs_sign_in by server id; other kinds are ignored', () => {
  assert.deepEqual(m.mcpServerStates(rows), { tasks: 'needs_sign_in', echo: 'ready', off1: 'ready', new: 'ready' });
  assert.deepEqual(m.mcpServerStates(null), {});
});

test('the list decides: ready, needs_sign_in, removed', () => {
  const listed = { states: m.mcpServerStates(rows), seq: 5 };
  assert.equal(m.suggestionServerState('echo', listed, undefined, 1), 'ready');
  assert.equal(m.suggestionServerState('tasks', listed, undefined, 1), 'needs_sign_in');
  assert.equal(m.suggestionServerState('gone', listed, undefined, 1), 'removed');
  // A reload starts with no local flag: the list still says "Sign in", not "Ready".
  assert.equal(m.suggestionServerState('tasks', listed, false, 3), 'needs_sign_in');
  // Deleted after a sign-in earlier this session: the newer list says removed.
  assert.equal(m.suggestionServerState('gone', listed, false, 4), 'removed');
});

test('the install answer is optimistic until a list fetched after it answers', () => {
  const older = { states: {}, seq: 2 };
  assert.equal(m.suggestionServerState('tasks', older, true, 3), 'needs_sign_in');
  assert.equal(m.suggestionServerState('echo', older, false, 3), 'ready');
  // A card sign-in (local false) beats an older list that still says needs_sign_in.
  assert.equal(m.suggestionServerState('tasks', { states: { tasks: 'needs_sign_in' }, seq: 2 }, false, 3), 'ready');
});

test('no list answer, or a failed one, keeps the optimistic value and never says removed', () => {
  assert.equal(m.suggestionServerState('tasks', null, true, 1), 'needs_sign_in');
  assert.equal(m.suggestionServerState('tasks', null, undefined, 1), 'ready');
  assert.equal(m.suggestionServerState('tasks', null, false, 1), 'ready');
});

test('fetchServerStates reads /api/connections quietly and throws on errors', async () => {
  let asked;
  const ok = async (url, init) => {
    asked = { url, init };
    return new Response(JSON.stringify({ connections: rows }), { status: 200 });
  };
  const got = await m.fetchServerStates(7, ok);
  assert.equal(asked.url, '/api/connections');
  assert.equal(asked.init.cache, 'no-store');
  assert.equal(got.seq, 7);
  assert.equal(got.states.tasks, 'needs_sign_in');
  await assert.rejects(m.fetchServerStates(8, async () => new Response('{"error":"person only"}', { status: 401 })));
  await assert.rejects(m.fetchServerStates(9, async () => new Response('{}', { status: 200 })));
  await assert.rejects(m.fetchServerStates(10, async () => { throw new TypeError('offline'); }));
});

const meta = {
  kind: 'mcp_suggestion', suggestion_id: 's1', kwant_id: 'k1', registry_name: 'io.example/tasklane', version: '1.0.0',
  title: 'Tasklane', description: 'Your tasks.', reason: 'You asked about your tasks.', repository_url: null,
  transport: 'http', url: 'https://tasklane.example/mcp', command: null, args: [], package: null,
  runs_on_your_computer: false, fixed_env: [], fixed_headers: [], inputs: [],
  status: 'installed', server_id: 'tasks', last_error: null,
};
const card = (serverState, over = {}) =>
  ui.render({ meta: { ...meta, ...over }, messageId: 'm1', kwantName: 'Ben', onInstall: async () => ({}), onDismiss: async () => {}, onSignIn: async () => {}, serverState });

test('the card says each state in words, with the Sign in button only when it needs one', () => {
  const ready = card('ready');
  assert.match(ready, /Ready\. Ben can use it from its next reply\./);
  assert.doesNotMatch(ready, />Sign in</);
  assert.match(card(undefined), /Ready\. Ben can use it/);

  const signIn = card('needs_sign_in');
  assert.match(signIn, /Installed\. Sign in to Tasklane so Ben can use it\./);
  assert.match(signIn, />Sign in</);
  assert.doesNotMatch(signIn, /Ready\./);

  const removed = card('removed');
  assert.match(removed, /Removed in Settings\. Ben carries on without it\./);
  assert.doesNotMatch(removed, /Ready\.|>Sign in<|<button/);
  assert.match(removed, /role="status"/);
  assert.match(removed, /border-zinc-800/);
  assert.doesNotMatch(removed, /border-teal-400/);

  // Only installed cards read the server state.
  assert.match(card('removed', { status: 'dismissed' }), /Not installed\. Ben carries on without it\./);
  assert.doesNotMatch(card('needs_sign_in', { status: 'dismissed' }), />Sign in</);
});

test('a chat needs the list only for an installed suggestion with a server, meta as object or JSON text', () => {
  const installed = { ...meta };
  const pending = { ...meta, status: 'pending', server_id: null };
  const msgs = (x) => [{ meta: null }, { meta: { kind: 'question' } }, { meta: '{"kind":"note"}' }, ...x];
  assert.equal(m.chatHasInstalledServer(msgs([]), {}), false);
  assert.equal(m.chatHasInstalledServer(msgs([{ meta: installed }]), {}), true);
  assert.equal(m.chatHasInstalledServer(msgs([{ meta: JSON.stringify(installed) }]), {}), true);
  assert.equal(m.chatHasInstalledServer(msgs([{ meta: JSON.stringify(pending) }]), {}), false);
  // The live record wins over the message's own meta (installed after the card was posted).
  const live = { s1: { id: 's1', status: 'installed', server_id: 'tasks', last_error: null, card: null } };
  assert.equal(m.chatHasInstalledServer(msgs([{ meta: JSON.stringify(pending) }]), live), true);
});
