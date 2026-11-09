// 0.8.0 P6 (#467): a second message to a bot still answering in the same chat waits (`turn_queued`,
// reason agent_busy) and starts when the first reply ends. The app used to hide that wait whenever a
// reply was running here, so "Screenshot give me!" looked ignored. Now the waiting turn shows under
// the running reply, with its place in line, until it starts or is cancelled.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { QueuedBeside } from './src/components/QueuedBeside';
  export * from './src/threads';
  export const draw = (items) => renderToStaticMarkup(React.createElement(QueuedBeside, { items }));
`);

const ev = (type, data) => ({ type, data });
const WAIT = 'Waiting for this bot’s other replies to finish.';
// What App.tsx hands ChatView for one queued turn.
const asQueued = (turns, x) => ({ position: m.queuePosition(turns, x.turn_id), reason: x.reason ?? 'agent_busy', slots: x.slots, speaker: 'Pixel', turnId: x.turn_id });
const besideHtml = (turns, ctx) => m.draw(m.queuedInChat(turns, ctx).beside.map((x) => asQueued(turns, x)));

// Oleks asks Pixel something in the direct chat, then writes again while it is still answering.
const runningPlusQueued = () => {
  let s = m.applyTurnEvent({}, ev('turn_started', { turn_id: 'r1', kwant_id: 'pixel', context_id: 'pixel' }));
  s = m.applyTurnEvent(s, ev('turn_queued', { turn_id: 'q1', kwant_id: 'pixel', context_id: 'pixel', position: 1, reason: 'agent_busy' }));
  return s;
};

test('one running and one queued turn in the same chat: both show, the queued one under the reply', () => {
  const s = runningPlusQueued();
  const q = m.queuedInChat(s, 'pixel');
  assert.equal(q.alone, null, 'the running reply keeps the live bubble');
  assert.deepEqual(q.beside.map((x) => x.turn_id), ['q1'], 'the waiting turn is not hidden');
  const html = besideHtml(s, 'pixel');
  assert.match(html, /role="status"/);
  assert.ok(html.includes('#1 in line.'), html);
  assert.ok(html.includes(WAIT), html);

  // A third message keeps its own place in line.
  const s2 = m.applyTurnEvent(s, ev('turn_queued', { turn_id: 'q2', kwant_id: 'pixel', context_id: 'pixel', position: 2, reason: 'agent_busy' }));
  const html2 = besideHtml(s2, 'pixel');
  assert.ok(html2.indexOf('#1 in line.') >= 0 && html2.indexOf('#2 in line.') > html2.indexOf('#1 in line.'), html2);
});

test('the queued chip clears when that turn starts, and when it is cancelled', () => {
  let s = runningPlusQueued();
  s = m.applyTurnEvent(s, ev('turn_ended', { turn_id: 'r1', kwant_id: 'pixel', context_id: 'pixel', outcome: 'completed' }));
  // Between the two events the old "In line" bubble stands in, as before.
  assert.equal(m.queuedInChat(s, 'pixel').alone?.turn_id, 'q1');
  s = m.applyTurnEvent(s, ev('turn_started', { turn_id: 'q1', kwant_id: 'pixel', context_id: 'pixel' }));
  assert.deepEqual(m.queuedInChat(s, 'pixel'), { alone: null, beside: [] });
  assert.equal(besideHtml(s, 'pixel'), '', 'nothing drawn once it runs');

  // Cancelled while waiting: the server ends the queued turn.
  let c = runningPlusQueued();
  c = m.applyTurnEvent(c, ev('turn_ended', { turn_id: 'q1', kwant_id: 'pixel', context_id: 'pixel', outcome: 'cancelled' }));
  assert.deepEqual(m.queuedInChat(c, 'pixel').beside, []);
  assert.equal(besideHtml(c, 'pixel'), '');
});

test("another chat's queued turn doesn't show here", () => {
  let s = runningPlusQueued();
  s = m.applyTurnEvent(s, ev('turn_queued', { turn_id: 'q-th', kwant_id: 'pixel', context_id: 'thread-9', thread_id: 'thread-9', reason: 'agent_busy' }));
  assert.deepEqual(m.queuedInChat(s, 'pixel').beside.map((x) => x.turn_id), ['q1']);
  // A chat with only someone else's waiting turn, and nothing running, is untouched.
  assert.deepEqual(m.queuedInChat(s, 'thread-9'), { alone: s['q-th'], beside: [] });
  assert.deepEqual(m.queuedInChat(s, 'elsewhere'), { alone: null, beside: [] });
});

test('the old path is unchanged: queued with nothing running takes the bubble (agent, server and VM waits)', () => {
  for (const reason of ['agent_busy', 'server_busy', 'vm_busy']) {
    const s = m.applyTurnEvent({}, ev('turn_queued', { turn_id: 'q', kwant_id: 'pixel', context_id: 'pixel', reason, slots: 2 }));
    const q = m.queuedInChat(s, 'pixel');
    assert.equal(q.alone?.turn_id, 'q', reason);
    assert.equal(q.alone?.reason, reason);
    assert.deepEqual(q.beside, [], `${reason}: no extra chip`);
  }
  assert.equal(m.draw([]), '');
  assert.equal(m.draw(undefined), '');
});

test('ChatView draws the chips under the running reply, never next to the "In line" bubble', () => {
  const chat = readFileSync(new URL('../src/components/ChatView.tsx', import.meta.url), 'utf8');
  const live = chat.indexOf('Live Streaming Assistant Message');
  const chip = chat.indexOf('{!queued && <QueuedBeside items={queuedBeside} />}');
  assert.ok(live > 0 && chip > live, 'inside the live reply block, after the bubble');
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /queuedBeside=\{queuedBeside\}/);
  assert.doesNotMatch(app, /openTurns\.some\(\(t\) => t\.state === 'running'\) \? undefined : openTurns\.find/, 'the hiding rule is gone');
});
