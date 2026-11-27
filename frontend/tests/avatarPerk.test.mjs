// avatar-perk-on-send (Nov 27): you send a bot a message, its avatar perks up 160 ms later (wide eyes, "o", one hop).
// The turn_started mood change lands about 30 ms after send; it used to cancel the queued perk, so the perk never
// showed. Now a mood change keeps a pending or showing perk and plays its own blink-through once the perk ends.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const s = await loadBundled(`
  export { createLife, BLINK_THROUGH, HOP, NOTICE_BLINK } from './src/avatarLife';
`);

// The same fake clock livingAvatars.test.mjs uses.
function fakeEnv({ reduced = false, seed = 1 } = {}) {
  let t = 0;
  let n = 0;
  const timers = new Map();
  let r = seed;
  const env = {
    now: () => t,
    setTimeout: (fn, ms) => (timers.set(++n, { fn, at: t + ms }), n),
    clearTimeout: (id) => timers.delete(id),
    random: () => ((r = (r * 16807) % 2147483647) / 2147483647),
    reduced: () => reduced,
  };
  const advance = (ms, each) => {
    const end = t + ms;
    for (;;) {
      const next = [...timers.entries()].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > end) break;
      timers.delete(next[0]);
      t = next[1].at;
      next[1].fn();
      each?.(t);
    }
    t = end;
  };
  return { env, advance, timerCount: () => timers.size, now: () => t };
}
// Records every frame change with its time, so a test can read the timeline back.
function el(f) {
  const attrs = {};
  const log = [];
  const note = () => log.push([f.now(), attrs['data-life'] ?? null, 'data-hop' in attrs]);
  return {
    attrs,
    log,
    setAttribute(k, v) { attrs[k] = v; note(); },
    removeAttribute(k) { delete attrs[k]; note(); },
  };
}
// One registered header avatar with idle life pushed out of the way (no blink inside the window under test).
function setup() {
  const f = fakeEnv();
  const life = s.createLife(f.env);
  const a = el(f);
  life.register(a, { zone: 'header', kwantId: 'a' });
  return { f, life, a };
}
/** The frames shown (data-life set), in order, from time `from` on; one entry per frame change. */
const lifes = (a, from = 0) =>
  a.log.filter(([t]) => t >= from).map(([, l]) => l).filter((l, i, xs) => l && l !== xs[i - 1]);
const frame = (a) => [a.attrs['data-life'] ?? null, 'data-hop' in a.attrs];

test('send, then turn_started 30 ms later: the perk still shows at 160 ms, then thinking blinks through', () => {
  const { f, life, a } = setup();
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'thinking');
  assert.deepEqual(frame(a), [null, false], 'nothing yet: the perk is still pending');
  f.advance(s.NOTICE_BLINK - 30);
  assert.deepEqual(frame(a), ['perk', true], 'wide eyes and one hop at 160 ms');
  f.advance(s.HOP);
  assert.deepEqual(frame(a), ['closed', false], 'the perk ended: thinking blinks through');
  f.advance(s.BLINK_THROUGH);
  assert.deepEqual(frame(a), [null, false], 'rest face (the thinking face ModernAvatar already draws)');
  assert.equal(life.stats().active, 0, 'nothing left counted');
  assert.equal(life.stats().queued, 0);
});

test('thinking then working inside the perk window: one blink-through after the perk, not two', () => {
  const { f, life, a } = setup();
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'thinking');
  f.advance(100);
  life.setMood(a, 'working');
  f.advance(30);
  assert.deepEqual(frame(a), ['perk', true]);
  f.advance(s.HOP + s.BLINK_THROUGH + 500);
  assert.deepEqual(lifes(a, s.NOTICE_BLINK + s.HOP), ['closed'], 'one blink-through for the newest mood');
  assert.equal(life.stats().active, 0);
});

test('a mood change while the perk shows keeps it to its full 120 ms', () => {
  const { f, life, a } = setup();
  life.perk('a');
  f.advance(s.NOTICE_BLINK + 40);
  life.setMood(a, 'working');
  assert.deepEqual(frame(a), ['perk', true], 'still perking');
  f.advance(s.HOP - 40);
  assert.deepEqual(frame(a), ['closed', false]);
});

test('needs_you after a perk still hops; failed after a perk swaps at once when the perk ends', () => {
  let { f, life, a } = setup();
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'needs_you');
  f.advance(s.NOTICE_BLINK - 30 + s.HOP + s.BLINK_THROUGH);
  assert.deepEqual(frame(a), [null, true], 'needs you hops after the blink-through');
  ({ f, life, a } = setup());
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'failed');
  f.advance(s.NOTICE_BLINK - 30);
  assert.deepEqual(frame(a), ['perk', true]);
  f.advance(s.HOP);
  assert.deepEqual(frame(a), [null, false], 'failed: no motion after the perk');
  assert.equal(life.stats().active, 0);
});

test('a mood that comes back to where it was inside the perk window plays nothing after it', () => {
  const { f, life, a } = setup();
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'thinking');
  life.setMood(a, 'idle');
  f.advance(1000);
  assert.deepEqual(lifes(a), ['perk']);
});

test('the perk still obeys hidden and off screen: both drop it and the waiting mood change', () => {
  let { f, life, a } = setup();
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'thinking');
  life.setHidden(true);
  f.advance(2000);
  assert.deepEqual(lifes(a), [], 'nothing plays while hidden');
  assert.equal(life.stats().active, 0);
  assert.equal(life.stats().queued, 0);
  ({ f, life, a } = setup());
  life.perk('a');
  f.advance(30);
  life.setMood(a, 'thinking');
  life.setVisible(a, false);
  f.advance(2000);
  assert.deepEqual(lifes(a), [], 'nothing plays off screen');
  assert.equal(life.stats().active, 0);
  // Paused while the perk is pending: setMood cancels as before.
  ({ f, life, a } = setup());
  life.perk('a');
  life.setVisible(a, false);
  life.setVisible(a, true);
  life.setMood(a, 'thinking');
  assert.deepEqual(frame(a), ['closed', false], 'no perk left: the mood blinks through at once');
});

test('control: setMood with no perk pending behaves exactly as before (blink-through now, rest 100 ms later)', () => {
  const { f, life, a } = setup();
  life.setMood(a, 'thinking');
  assert.deepEqual(frame(a), ['closed', false]);
  assert.equal(life.stats().active, 1);
  f.advance(s.BLINK_THROUGH);
  assert.deepEqual(frame(a), [null, false]);
  assert.equal(life.stats().active, 0);
  // After a perk has fully ended, a mood change is immediate again.
  life.perk('a');
  f.advance(s.NOTICE_BLINK + s.HOP + 1);
  life.setMood(a, 'working');
  assert.deepEqual(frame(a), ['closed', false], 'immediate, nothing deferred');
  f.advance(s.BLINK_THROUGH);
  life.setMood(a, 'finished');
  assert.deepEqual(frame(a), ['closed', false]);
  f.advance(s.BLINK_THROUGH);
  assert.deepEqual(frame(a), [null, true], 'finished hops');
});

test('other moments still win over a pending perk as before (hello cancels it)', () => {
  const { f, life, a } = setup();
  life.perk('a');
  f.advance(30);
  life.hello('a');
  assert.equal(a.attrs['data-life'], 'happy');
  life.setMood(a, 'thinking');
  assert.equal(a.attrs['data-life'], 'closed', 'no perk pending any more: the mood plays at once');
});
