// Bot status dots: one status per bot (Needs you > Working > Queued > Can't run > Available), and
// a bot with a running turn never shows the green Available dot, in the open chat included.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const s = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ModernAvatar } from './src/components/ModernAvatar';
  export * from './src/botStatus';
  export { turnStarted, turnEnded } from './src/kwantActivity';
  export const avatar = (p) => renderToStaticMarkup(React.createElement(ModernAvatar, p));
  // Mounted for real, so the avatar follows the live turn store.
  export const mount = (p) => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(ModernAvatar, q)));
    render(p);
    return { host, render, flush: (fn) => flushSync(fn), unmount: () => root.unmount() };
  };
`);
const dotOf = (host) => host.querySelector('[data-status]')?.getAttribute('data-status');

const run = (turn_id, kwant_id, state = 'running') => ({ turn_id, kwant_id, state });

test('an idle bot that can run is Available, and Can\'t run when no account works', () => {
  const st = s.botStatuses({ turns: [] });
  assert.equal(s.statusOf('a', st, true), 'available');
  assert.equal(s.statusOf('a', st, null), 'available');
  assert.equal(s.statusOf('a', st, false), 'cant_run');
});

test('a running turn is Working, wherever it runs (the open chat included)', () => {
  // The helper has no notion of the open chat: every running turn counts.
  const st = s.botStatuses({ turns: [run('t1', 'editor')] });
  assert.equal(s.statusOf('editor', st, true), 'working');
});

test('a running turn beats Can\'t run', () => {
  const st = s.botStatuses({ turns: [run('t1', 'editor')] });
  assert.equal(s.statusOf('editor', st, false), 'working');
});

test('queued only is Queued; queued plus running is Working', () => {
  assert.equal(s.statusOf('a', s.botStatuses({ turns: [run('q', 'a', 'queued')] }), false), 'queued');
  const st = s.botStatuses({ turns: [run('q', 'a', 'queued'), run('r', 'a')] });
  assert.equal(st.a, 'working');
  // Order doesn't matter.
  assert.equal(s.botStatuses({ turns: [run('r', 'a'), run('q', 'a', 'queued')] }).a, 'working');
});

test('a turn waiting on the person is Needs you, and beats a running turn', () => {
  const awaiting = new Set(['ask']);
  assert.equal(s.botStatuses({ turns: [run('ask', 'a')], awaiting }).a, 'needs_you');
  assert.equal(s.botStatuses({ turns: [run('r', 'a'), run('ask', 'a')], awaiting }).a, 'needs_you');
  assert.equal(s.botStatuses({ turns: [run('ask', 'a'), run('r', 'a')], awaiting }).a, 'needs_you');
});

test('an open browser step is Needs you even without a tracked turn', () => {
  const st = s.botStatuses({ turns: [run('r', 'a')], needsYou: { a: 1, b: 2, c: 0 } });
  assert.equal(st.a, 'needs_you');
  assert.equal(st.b, 'needs_you');
  assert.equal(st.c, undefined);
});

test('priority order is Needs you > Working > Queued > Can\'t run > Available', () => {
  assert.deepEqual([...s.STATUS_PRIORITY], ['needs_you', 'working', 'queued', 'cant_run', 'available']);
});

test('only Available is green; Working pulses but respects reduced motion', () => {
  for (const k of s.STATUS_PRIORITY) assert.equal(s.STATUS_DOT[k].includes('emerald'), k === 'available', k);
  assert.match(s.STATUS_DOT.working, /animate-pulse/);
  assert.match(s.STATUS_DOT.working, /motion-reduce:animate-none/);
  assert.equal(s.statusText('Editor', 'working'), 'Editor · Working');
});

test('the avatar dot shows the status in words for tooltips and screen readers', () => {
  const html = s.avatar({ name: 'Editor', kwantId: 'ed', status: 'needs_you' });
  assert.match(html, /data-status="needs_you"/);
  assert.match(html, /title="Editor · Needs you"/);
  assert.match(html, /<span class="sr-only">Editor · Needs you<\/span>/);
  assert.match(html, /bg-amber-400/);
  assert.doesNotMatch(html, /emerald/);
});

test('the avatar never shows green while its bot has a running turn, even if the status lags', () => {
  for (const lagging of ['available', 'cant_run', 'queued']) {
    const m = s.mount({ name: 'Editor', kwantId: 'ed', status: lagging });
    assert.equal(dotOf(m.host), lagging, 'idle: shows what it was given');
    m.flush(() => s.turnStarted('ed', 'chat-1'));
    assert.equal(dotOf(m.host), 'working', `${lagging} + running turn`);
    assert.doesNotMatch(m.host.innerHTML, /emerald/);
    assert.equal(m.host.querySelector('.sr-only').textContent, 'Editor · Working');
    m.flush(() => s.turnEnded('ed', 'chat-1'));
    assert.equal(dotOf(m.host), lagging, 'turn over: back to the given status');
    m.unmount();
  }
});

test('Needs you still wins over a running turn on the avatar', () => {
  const m = s.mount({ name: 'Editor', kwantId: 'ed', status: 'needs_you' });
  m.flush(() => s.turnStarted('ed', 'chat-1'));
  assert.equal(dotOf(m.host), 'needs_you');
  m.flush(() => s.turnEnded('ed', 'chat-1'));
  m.unmount();
});

test('without a status the avatar draws no dot', () => {
  const html = s.avatar({ name: 'Editor', kwantId: 'ed' });
  assert.doesNotMatch(html, /data-status/);
  assert.doesNotMatch(html, /sr-only/);
});

const describe = (ctx) => ({ 'room-1': 'Engineering Room', 'th-1': 'Release notes' })[ctx] ?? 'another conversation';
const turn = (turn_id, kwant_id, context_id, state = 'running') => ({ turn_id, kwant_id, context_id, state });

test('the line under the name: other chats are named, the open chat says "here"', () => {
  const lines = s.busyLines({
    turns: [turn('a', 'ed', 'open'), turn('b', 'lin', 'room-1'), turn('c', 'rig', 'th-1', 'queued'), turn('d', 'ada', 'open', 'queued')],
    openCtx: 'open',
    describe,
  });
  assert.deepEqual(lines, { ed: 'Working here', lin: 'Working on Engineering Room', rig: 'Waiting to work on Release notes', ada: 'Waiting to work here' });
});

test('the line prefers running over queued, then other chats over the open one', () => {
  const t = [turn('q', 'ed', 'room-1', 'queued'), turn('r', 'ed', 'open')];
  assert.equal(s.busyLines({ turns: t, openCtx: 'open', describe }).ed, 'Working here');
  const u = [turn('r1', 'ed', 'open'), turn('r2', 'ed', 'room-1')];
  assert.equal(s.busyLines({ turns: u, openCtx: 'open', describe }).ed, 'Working on Engineering Room');
});

test('the line says when a bot waits on the person, here or elsewhere', () => {
  const lines = s.busyLines({
    turns: [turn('q1', 'ed', 'open'), turn('b1', 'lin', 'room-1'), turn('q2', 'rig', 'th-1')],
    openCtx: 'open',
    awaiting: new Set(['q1', 'b1', 'q2']),
    browser: new Set(['b1']),
    describe,
  });
  assert.deepEqual(lines, { ed: 'Asked you a question here', lin: 'Needs you in the browser in Engineering Room', rig: 'Asked you a question in Release notes' });
});
