// 25a: the thread header says "Working" (with its Stop) until the reply lands. Mira's design check of #737 saw the
// header drop "Working" and Stop ~350 ms before the body's live card and the composer Stop went (turn_ended removes the
// turn a message fetch before clearLive). The open chat's badge now follows `live` too, so all three go idle together.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ThreadHeader, ThreadBar, ThreadPicker } from './src/components/ThreadBar';
  export * from './src/threads';
  const noop = async () => {};
  export const header = (thread, badge) =>
    renderToStaticMarkup(React.createElement(ThreadHeader, { thread, badge, onBack: noop, onStop: noop, onArchive: noop }));
  const props = (extra) => ({ kwantName: 'TechLead', error: null, showArchived: false, onToggleArchived: noop, onOpenDirect: noop,
    onOpenThread: noop, onNewThread: noop, onRetry: noop, ...extra });
  export const bar = (extra) => renderToStaticMarkup(React.createElement(ThreadBar, props(extra)));
  export const picker = (extra) => renderToStaticMarkup(React.createElement(ThreadPicker, props(extra)));
`);

const WORKING = { kind: 'working', label: 'Working' };
const thread = (id, extra = {}) => ({ id, kwant_id: 'k', kind: 'user', title: `Thread ${id}`, archived: false, delegation: null, created_at: '2026-11-16T12:00:00Z', updated_at: '2026-11-16T12:00:00Z', ...extra });
const delegation = (status) => ({ id: 'd1', status, lead_kwant_id: 'lead', specialist_kwant_id: 'k', title: 'Task' });
const buttons = (html) => [...html.matchAll(/<button[^>]*>(.*?)<\/button>/g)].map((b) => b[1].replace(/<[^>]+>/g, '').trim());

test('writingBadge: writing with no badge, or a final one, reads Working', () => {
  assert.deepEqual(m.writingBadge(null, true), WORKING);
  for (const kind of ['done', 'failed', 'cancelled', 'interrupted']) {
    assert.deepEqual(m.writingBadge({ kind, label: kind }, true), WORKING, kind);
  }
});

test('writingBadge: Waiting for you, Queued and Working stay as they are', () => {
  const waiting = { kind: 'waiting', label: 'Waiting for you' };
  const queued = { kind: 'queued', label: 'Queued · #2' };
  assert.equal(m.writingBadge(waiting, true), waiting);
  assert.equal(m.writingBadge(queued, true), queued);
  assert.equal(m.writingBadge(WORKING, true), WORKING);
});

test('writingBadge: not writing leaves every badge unchanged', () => {
  assert.equal(m.writingBadge(null, false), null);
  for (const b of [WORKING, { kind: 'done', label: 'Done' }, { kind: 'waiting', label: 'Waiting for you' }, { kind: 'queued', label: 'Queued' }]) {
    assert.equal(m.writingBadge(b, false), b);
  }
});

test('the header keeps Working and Stop while the open thread is live and its turn is already gone', () => {
  const th = thread('th');
  const noTurns = {}; // turn_ended removed the turn; the saved reply isn't on screen yet
  const plain = m.threadBadge(th, noTurns);
  assert.equal(plain, null, 'threadBadge alone: idle (the old header)');
  assert.ok(!buttons(m.header(th, plain)).includes('Stop'), 'before: no Stop in the window');
  const html = m.header(th, m.writingBadge(plain, true));
  assert.match(html, />Working</);
  assert.ok(buttons(html).includes('Stop'), 'after: Stop stays with the live card');
  // a delegation already marked Done in the same window still reads Working until the body goes idle
  const done = thread('th', { kind: 'delegation', delegation: delegation('completed') });
  assert.ok(buttons(m.header(done, m.writingBadge(m.threadBadge(done, noTurns), true))).includes('Stop'));
  // once clearLive drops the live card the header goes idle with it
  assert.ok(!buttons(m.header(th, m.writingBadge(plain, false))).includes('Stop'));
  assert.match(m.header(done, m.writingBadge(m.threadBadge(done, noTurns), false)), />Done</);
});

test('ThreadBar: only the open conversation follows openWriting; other rows keep threadBadge', () => {
  const threads = [thread('a'), thread('b')];
  const html = m.bar({ threads, openThreadId: 'a', turns: {}, directBadge: null, openWriting: true });
  assert.equal((html.match(/>Working</g) ?? []).length, 1, 'just the open row');
  assert.ok(html.indexOf('Working') > html.indexOf('Thread a') && html.indexOf('Working') < html.indexOf('Thread b'));
  assert.doesNotMatch(m.bar({ threads, openThreadId: 'a', turns: {}, directBadge: null, openWriting: false }), />Working</);
});

test('ThreadPicker (phone): the open conversation\'s dot reads Working while its reply is written', () => {
  const threads = [thread('a')];
  const idle = m.picker({ threads, openThreadId: 'a', turns: {}, directBadge: null, openWriting: false });
  const writing = m.picker({ threads, openThreadId: 'a', turns: {}, directBadge: null, openWriting: true });
  assert.notEqual(idle, writing);
  assert.match(writing, /Working/);
  assert.doesNotMatch(idle, /Working/);
});

test('App feeds the open chat\'s live state to the header, the direct chat badge and the bar', () => {
  const src = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const head = src.slice(src.indexOf('<ThreadHeader'), src.indexOf('delegatedBy=', src.indexOf('<ThreadHeader')));
  assert.match(head, /badge=\{writingBadge\(/);
  assert.match(head, /shownThread\.id in live,?\s*\)\}/);
  assert.match(src, /directBadge: writingBadge\(threadBadge\(\{ id: currentKwant\.id, delegation: null \}, turns, awaitingTurns\), !openThreadId && currentKwant\.id in live\)/);
  assert.match(src, /openWriting: isCurrentContextStreaming,/);
  assert.match(src, /const isCurrentContextStreaming = openCtx in live;/);
});
