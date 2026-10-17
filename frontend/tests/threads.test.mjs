import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import ts from 'typescript';
import { build } from 'esbuild';

const load = async (path) => {
  const source = await readFile(new URL(path, import.meta.url), 'utf8');
  const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
  return import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
};

const t = await load('../src/threads.ts');

// ---------------------------------------------------------------------------
// Deltas only reach the chat they belong to

test('a tagged delta renders only in its own chat', () => {
  const delta = { type: 'TextDelta', data: 'hi', turn_id: 'u1', kwant_id: 'cto', context_id: 'thread-1', thread_id: 'thread-1' };
  assert.equal(t.shouldRenderDelta(delta, 'thread-1', null), true);
  // The specialist's direct chat is open while it works on the delegated thread: nothing leaks in.
  assert.equal(t.shouldRenderDelta(delta, 'cto', 'cto'), false);
  assert.equal(t.shouldRenderDelta(delta, 'general', 'thread-1'), false, 'the tag wins over the fallback');
  assert.equal(t.streamTarget(delta, 'cto'), 'thread-1');
});

test('untagged deltas (older servers, team discussions) go to the chat that started the stream', () => {
  const delta = { type: 'TextDelta', data: 'hi' };
  assert.equal(t.eventContextId(delta), null);
  assert.equal(t.shouldRenderDelta(delta, 'general', 'general'), true);
  assert.equal(t.shouldRenderDelta(delta, 'cto', 'general'), false);
  assert.equal(t.shouldRenderDelta(delta, 'cto', null), false);
  assert.equal(t.shouldRenderDelta(delta, '', ''), false, 'nothing open, nothing drawn');
});

test('lifecycle events carry their chat in data', () => {
  assert.equal(t.eventContextId({ type: 'turn_started', data: { context_id: 'c1' } }), 'c1');
  assert.equal(t.eventContextId({ type: 'TurnComplete', context_id: 'top', data: { context_id: 'inner' } }), 'top');
  assert.equal(t.eventContextId({ type: 'TextDelta', data: 'context_id' }), null, 'string data is not a tag');
  assert.equal(t.contextOf({ selectedId: 'cto', threadId: 'th' }), 'th');
  assert.equal(t.contextOf({ selectedId: 'cto', threadId: null }), 'cto');
});

// ---------------------------------------------------------------------------
// Turns and the queue

const ev = (type, data) => ({ type, data });

test('turns move queued -> running -> gone, and the queue counts down', () => {
  let m = t.hydrateTurns([{ turn_id: 'a', kwant_id: 'cto', context_id: 'th', thread_id: 'th', state: 'running', since: '1' }]);
  m = t.applyTurnEvent(m, ev('turn_queued', { turn_id: 'b', kwant_id: 'cto', context_id: 'cto', position: 1, reason: 'agent_busy' }));
  m = t.applyTurnEvent(m, ev('turn_queued', { turn_id: 'c', kwant_id: 'cto', context_id: 'cto', position: 2, reason: 'agent_busy' }));
  m = t.applyTurnEvent(m, ev('turn_queued', { turn_id: 'd', kwant_id: 'ellis', context_id: 'ellis', position: 1, reason: 'agent_busy' }));
  assert.equal(t.queuePosition(m, 'b'), 1);
  assert.equal(t.queuePosition(m, 'c'), 2);
  assert.equal(t.queuePosition(m, 'd'), 1, 'another agent has its own line');
  assert.equal(t.queuePosition(m, 'a'), null, 'running turns have no position');

  m = t.applyTurnEvent(m, ev('turn_started', { turn_id: 'b', kwant_id: 'cto', context_id: 'cto' }));
  assert.equal(m.b.state, 'running');
  assert.equal(t.queuePosition(m, 'c'), 1, 'moves up when the one ahead starts');
  m = t.applyTurnEvent(m, ev('turn_ended', { turn_id: 'a', kwant_id: 'cto', context_id: 'th', outcome: 'completed' }));
  assert.equal(m.a, undefined);
  const same = t.applyTurnEvent(m, ev('turn_ended', { turn_id: 'zzz' }));
  assert.equal(same, m, 'unknown turns leave the map alone');
  assert.equal(t.applyTurnEvent(m, ev('TextDelta', 'x')), m);
});

test('server-wide queue counts every agent', () => {
  let m = t.hydrateTurns([]);
  m = t.applyTurnEvent(m, ev('turn_queued', { turn_id: 'x', kwant_id: 'a', context_id: 'a', position: 1, reason: 'server_busy' }));
  m = t.applyTurnEvent(m, ev('turn_queued', { turn_id: 'y', kwant_id: 'b', context_id: 'b', position: 2, reason: 'server_busy' }));
  assert.equal(t.queuePosition(m, 'y'), 2);
});

test('background turns are the agent’s work outside its direct chat', () => {
  const m = t.hydrateTurns([
    { turn_id: '1', kwant_id: 'cto', context_id: 'cto', thread_id: null, state: 'running', since: '1' },
    { turn_id: '2', kwant_id: 'cto', context_id: 'th', thread_id: 'th', state: 'running', since: '2' },
    { turn_id: '3', kwant_id: 'ellis', context_id: 'th2', thread_id: 'th2', state: 'running', since: '3' },
  ]);
  assert.deepEqual(t.backgroundTurns(m, 'cto').map((x) => x.turn_id), ['2']);
  assert.deepEqual(t.turnsIn(m, 'cto').map((x) => x.turn_id), ['1']);
});

// ---------------------------------------------------------------------------
// Thread list

const thread = (id, extra = {}) => ({
  id,
  kwant_id: 'cto',
  title: id,
  kind: 'chat',
  team_id: null,
  created_by: 'user',
  archived: false,
  created_at: '2026-10-01',
  updated_at: '2026-10-01',
  ...extra,
});
const delegation = (status) => ({ id: 'd1', thread_id: 'th', status, error: null });

test('thread badges: live turns first, then the delegation record', () => {
  const none = t.hydrateTurns([]);
  assert.equal(t.threadBadge(thread('th'), none), null, 'idle user threads have no badge');
  assert.deepEqual(t.threadBadge(thread('th', { delegation: delegation('completed') }), none), { kind: 'done', label: 'Done' });
  assert.equal(t.threadBadge(thread('th', { delegation: delegation('interrupted') }), none).kind, 'interrupted');
  assert.equal(t.threadBadge(thread('th', { delegation: delegation('cancelled') }), none).kind, 'cancelled');
  let m = t.applyTurnEvent(none, ev('turn_queued', { turn_id: 'q', kwant_id: 'cto', context_id: 'th', position: 3, reason: 'agent_busy' }));
  assert.deepEqual(t.threadBadge(thread('th', { delegation: delegation('queued') }), m), { kind: 'queued', label: 'Queued · #1' });
  m = t.applyTurnEvent(m, ev('turn_started', { turn_id: 'q', kwant_id: 'cto', context_id: 'th' }));
  assert.equal(t.threadBadge(thread('th', { delegation: delegation('completed') }), m).kind, 'working');
});

test('threads sort delegated first, newest first, archived hidden unless asked', () => {
  const list = [
    thread('old-chat', { updated_at: '2026-10-01' }),
    thread('new-chat', { updated_at: '2026-10-05' }),
    thread('task', { kind: 'delegation', updated_at: '2026-09-01' }),
    thread('gone', { archived: true, updated_at: '2026-10-09' }),
  ];
  assert.deepEqual(t.sortThreads(list).map((x) => x.id), ['task', 'new-chat', 'old-chat']);
  assert.deepEqual(t.sortThreads(list, true).map((x) => x.id), ['task', 'new-chat', 'old-chat', 'gone']);
});

test('thread updates keep the delegation the list route sent', () => {
  const d = delegation('running');
  const list = [thread('th', { delegation: d })];
  const next = t.upsertThread(list, thread('th', { title: 'Renamed' }));
  assert.equal(next[0].title, 'Renamed');
  assert.equal(next[0].delegation, d);
  assert.equal(t.upsertThread(list, thread('other')).length, 2);
  assert.equal(t.applyDelegation(list, { ...d, status: 'completed' })[0].delegation.status, 'completed');
  assert.equal(t.applyDelegation(list, { ...d, thread_id: 'unknown' }), list);
});

// ---------------------------------------------------------------------------
// Delegation notes

const metaJson = (extra = {}) =>
  JSON.stringify({
    kind: 'delegation_result',
    delegation_id: 'd1',
    thread_id: 'th',
    specialist_id: 'cto',
    specialist_name: 'CTO',
    lead_id: 'ellis',
    status: 'completed',
    title: 'Research vector DBs',
    ...extra,
  });

test('meta arrives as a JSON string and anything else is ignored safely', () => {
  const m = t.parseDelegationMeta(metaJson());
  assert.equal(m.kind, 'delegation_result');
  assert.equal(m.specialist_name, 'CTO');
  assert.equal(m.thread_id, 'th');
  assert.equal(t.parseDelegationMeta(JSON.parse(metaJson())).delegation_id, 'd1', 'objects work too');
  for (const bad of [null, undefined, '', '   ', '{not json', '"str"', '42', metaJson({ kind: 'routine' }), metaJson({ delegation_id: 5 })]) {
    assert.equal(t.parseDelegationMeta(bad), null, String(bad));
  }
  const sparse = t.parseDelegationMeta(JSON.stringify({ kind: 'delegation_started', delegation_id: 'd', thread_id: 't' }));
  assert.equal(sparse.specialist_name, 'The specialist');
  assert.equal(sparse.title, 'Delegated task');
});

test('status lines read as sentences', () => {
  assert.equal(t.delegationStatusLine('queued', 'CTO', { queuePos: 2 }), 'Waiting for CTO · #2 in line');
  assert.equal(t.delegationStatusLine('queued', 'CTO'), 'Waiting for CTO to start');
  assert.equal(t.delegationStatusLine('running', 'CTO'), 'CTO is working on it');
  assert.equal(t.delegationStatusLine('failed', 'CTO', { error: 'quota' }), "CTO couldn't finish: quota");
  assert.equal(t.delegationStatusLine('interrupted', 'CTO'), 'Interrupted by a restart');
});

test('messages are placed by thread, channel, then direct chat', () => {
  assert.equal(t.messageContextId({ thread_id: 'th', channel_id: 'c', direct_kwant_id: 'k' }), 'th');
  assert.equal(t.messageContextId({ channel_id: 'c', direct_kwant_id: 'k' }), 'c');
  assert.equal(t.messageContextId({ direct_kwant_id: 'k' }), 'k');
  assert.equal(t.messageContextId({}), null);
});

test('limit fields accept whole numbers in range only', () => {
  assert.equal(t.limitError('max_turns_per_agent', '4'), null);
  assert.equal(t.limitError('max_turns_per_agent', '0'), 'Use 1–16.');
  assert.equal(t.limitError('max_turns_global', '65'), 'Use 1–64.');
  assert.equal(t.limitError('delegation_follow_up_minutes', '0'), null);
  assert.equal(t.limitError('delegation_follow_up_minutes', '1441'), 'Use 0–1440.');
  for (const bad of ['', ' ', '2.5', '-1', 'abc', '1e2']) {
    assert.match(t.limitError('max_turns_per_agent', bad), /whole number/, bad);
  }
});

// ---------------------------------------------------------------------------
// The delegation card, rendered

// The card's imports touch browser storage (sound settings) when they load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const card = await (async () => {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const out = await build({
    stdin: {
      contents: `
        import React from 'react';
        import { renderToStaticMarkup } from 'react-dom/server.browser';
        import { DelegationCard } from './src/components/DelegationCard';
        export const render = (props) => renderToStaticMarkup(React.createElement(DelegationCard, props));
      `,
      resolveDir: root,
      loader: 'tsx',
    },
    bundle: true,
    write: false,
    format: 'esm',
    platform: 'node',
    jsx: 'automatic',
    logLevel: 'silent',
    define: { 'process.env.NODE_ENV': '"production"' },
  });
  return import(`data:text/javascript;base64,${Buffer.from(out.outputFiles[0].text).toString('base64')}`);
})();

const props = (extra = {}) => ({
  meta: t.parseDelegationMeta(metaJson({ kind: 'delegation_started', status: 'queued' })),
  text: 'CTO is on it.',
  messageId: 'm1',
  onOpenThread: () => {},
  onCancel: async () => {},
  ...extra,
});

test('a running delegation card shows its status, the thread link and Stop', () => {
  const html = card.render(props({ live: { ...delegation('running'), id: 'd1' } }));
  assert.match(html, /Delegated to/);
  assert.match(html, /Research vector DBs/);
  assert.match(html, /role="status"[^>]*>.*CTO is working on it/s);
  assert.match(html, /Open CTO&#x27;s thread|Open CTO's thread/);
  assert.match(html, /aria-label="Stop “Research vector DBs”"/);
  assert.match(html, /aria-labelledby="dlg-m1"/);
});

test('a queued card shows its place in line, not an error', () => {
  const html = card.render(props({ queuePos: 2 }));
  assert.match(html, /Waiting for CTO · #2 in line/);
  assert.doesNotMatch(html, /role="alert"/);
});

test('a result card shows the answer and no Stop', () => {
  const html = card.render(props({ meta: t.parseDelegationMeta(metaJson()), text: 'Use **pgvector**.' }));
  assert.match(html, /Result from/);
  assert.match(html, /CTO finished/);
  assert.match(html, /<strong[^>]*>pgvector<\/strong>/);
  assert.doesNotMatch(html, /Stop “/);
  assert.match(html, /Open CTO/);
});

test('the live record wins over the status frozen in meta', () => {
  const html = card.render(props({ live: { ...delegation('cancelled'), id: 'd1' } }));
  assert.match(html, />Stopped</);
  assert.doesNotMatch(html, /Stop “/);
});

test('long answers collapse behind a toggle', () => {
  const html = card.render(props({ meta: t.parseDelegationMeta(metaJson()), text: 'x '.repeat(1000) }));
  assert.match(html, /aria-expanded="false"[^>]*>Show the whole answer/);
});
