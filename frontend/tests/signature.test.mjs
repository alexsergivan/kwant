import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const s = await loadBundled(`export * from './src/signature';`);
const w = await loadBundled(`export * from './src/workingStrip';`);

const ev = (kind, detail = {}) => ({ kind, detail });
const fakeStore = () => {
  const m = new Map();
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => void m.set(k, v), m };
};

test('heartbeat dots: met is green, not_met red, a fix amber, anything else grey; the last 7 stay', () => {
  const dots = s.heartbeatDots([
    ev('checked', { verdict: 'met' }),
    ev('checked', { verdict: 'not_met' }),
    ev('fix_opened'),
    ev('replanned'),
    ev('checked', { verdict: 'unknown' }),
    ev('checked', {}),
    ev('note'),
    ev('checked', { verdict: 'met' }),
  ]);
  assert.deepEqual(dots.map((d) => d.tone), ['ok', 'failing', 'fixing', 'fixing', 'unknown', 'unknown', 'ok']);
  const many = Array.from({ length: 12 }, (_, i) => ev('checked', { verdict: i % 2 ? 'met' : 'not_met' }));
  assert.equal(s.heartbeatDots(many).length, 7);
  assert.equal(s.heartbeatDots(many, 5).length, 5);
  assert.deepEqual(s.heartbeatDots([]), []);
});

test('heartbeat text counts each tone and never claims more than the dots show', () => {
  assert.equal(s.heartbeatText([]), 'No checks yet');
  const dots = s.heartbeatDots([ev('checked', { verdict: 'met' }), ev('checked', { verdict: 'met' }), ev('checked', { verdict: 'not_met' }), ev('fix_opened')]);
  assert.equal(s.heartbeatText(dots), 'Last 4 checks, oldest first: 2 OK, 1 failing, 1 fix underway');
  assert.match(s.heartbeatText(s.heartbeatDots([ev('checked', { verdict: 'met' })])), /^Last 1 check,/);
});

test('lessons chip: the count comes from the lessons_shown keys; none, no chip', () => {
  const none = s.lessonsChip([ev('created')]);
  assert.equal(none, null);
  assert.equal(s.lessonsChip([ev('lessons_shown', { keys: [] })]), null);
  assert.equal(s.lessonsChip([ev('lessons_shown', {})]), null);
  const one = s.lessonsChip([ev('lessons_shown', { keys: ['a'] })]);
  assert.equal(one.text, 'Learned from 1 earlier task');
  assert.equal(one.title, 'It started with 1 lesson from earlier work');
  const three = s.lessonsChip([ev('lessons_shown', { keys: ['a', 'b'] }), ev('lessons_shown', { keys: ['a', 'b', 'c'] })]);
  assert.equal(three.count, 3, 'the latest event wins');
  assert.equal(three.text, 'Learned from 3 earlier tasks');
});

test('verified stamp: a count only when every check passed, else just "Verified"', () => {
  assert.equal(s.stampText({ passed: 3, total: 3 }), 'Verified · 3/3');
  assert.equal(s.stampText({ passed: 1, total: 2 }), 'Verified', 'an optional check waiting is not a count');
  assert.equal(s.stampText({ passed: 0, total: 0 }), 'Verified');
  assert.equal(s.stampText(null), 'Verified');
  assert.equal(s.stampLabel({ passed: 3, total: 3 }), 'Verified, 3 of 3 checks passed. Open the evidence receipt');
  assert.equal(s.stampLabel(undefined), 'Verified. Open the evidence receipt');
});

test('first moments happen once per space and kind, and never again', () => {
  const st = fakeStore();
  assert.equal(s.firstHolder('verified', 'sp1', st), null);
  assert.equal(s.claimFirst('verified', 'sp1', 't1', st), true);
  assert.equal(s.claimFirst('verified', 'sp1', 't2', st), false, 'not twice');
  assert.equal(s.firstHolder('verified', 'sp1', st), 't1', 'the sticker stays on the first task');
  assert.equal(s.claimFirst('verified', 'sp2', 't9', st), true, 'another space has its own');
  assert.equal(s.claimFirst('goal_met', 'sp1', 'g1', st), true, 'another kind has its own');
  assert.equal(s.claimFirst('verified', '', 'x', st), true);
  assert.equal(s.claimFirst('verified', '', 'y', st), false, 'no space shares one default key');
});

test('a space that already had one is claimed quietly: no celebration, no sticker', () => {
  const st = fakeStore();
  s.claimQuietly('goal_met', 'sp1', st);
  assert.equal(s.claimFirst('goal_met', 'sp1', 'g1', st), false);
  assert.equal(s.firstHolder('goal_met', 'sp1', st), '', 'held by no one');
});

test('without storage nothing celebrates, so nothing can repeat', () => {
  assert.equal(s.claimFirst('verified', 'sp1', 't1', null), false);
  const broken = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
  assert.equal(s.claimFirst('verified', 'sp1', 't1', broken), false);
  assert.equal(s.firstHolder('verified', 'sp1', broken), null);
});

test('first-moment copy: one line and one sticker label each, no emoji', () => {
  for (const k of ['verified', 'goal_met']) {
    assert.ok(s.FIRST_LINE[k].length > 10 && s.FIRST_STICKER[k].length > 3);
    assert.doesNotMatch(s.FIRST_LINE[k] + s.FIRST_STICKER[k], /\p{Extended_Pictographic}/u);
  }
  assert.equal(s.FIRST_STICKER.verified, 'First verified');
  assert.equal(s.FIRST_STICKER.goal_met, 'First goal kept');
});

test('working strip: the label follows the open step, then falls back', () => {
  assert.equal(w.workingLabel({}), 'getting started');
  assert.equal(w.workingLabel({ writing: true }), 'writing the reply');
  assert.equal(w.workingLabel({ queued: true, writing: true }), 'waiting in line');
  assert.equal(w.workingLabel({ awaitingAnswer: true }), 'waiting for your answer');
  assert.equal(w.workingLabel({ awaitingAnswer: true, awaitingBrowser: true }), 'waiting for you in the browser');
});

test('elapsed time reads m:ss and h:mm:ss, never negative', () => {
  assert.equal(w.formatElapsed(0), '0:00');
  assert.equal(w.formatElapsed(42_900), '0:42');
  assert.equal(w.formatElapsed(725_000), '12:05');
  assert.equal(w.formatElapsed(3_729_000), '1:02:09');
  assert.equal(w.formatElapsed(-5), '0:00');
  assert.equal(w.formatElapsed(NaN), '0:00');
});

test('turn start is the earliest step, else when the strip appeared', () => {
  assert.equal(w.turnStart(null, 5000), 5000);
  const live = { steps: [{ started_at: new Date(2000).toISOString() }, { started_at: 'bad' }, { started_at: new Date(9000).toISOString() }] };
  assert.equal(w.turnStart(live, 5000), 2000);
});
