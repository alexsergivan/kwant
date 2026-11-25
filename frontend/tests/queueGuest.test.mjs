// pa11b Queue ticket guest page and big screen (autonomy/pocket-apps/pa11/queue-guest-brief.md): the view mapping from
// Vega's queue API (pa11/queue-impl.md), the copy (ordinal, minutes only with an average, the refusals with the 429
// "limit"), what the buzz can honestly do per browser, the screen's own parser (no names, no ticket), and source rules
// (no hex, no HTML from data, no style attributes, no React in the guest). The browser half is guest/e2e/queue.mjs.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync, readdirSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';
import { QueueBox, queueScenes } from '../guest/fake/queue.mjs';

const m = await loadBundled(`
  export { queueFrom, ticketFrom, phaseOf, live } from './guest/src/queue/view';
  export { ordinal, untilText, minutesFor, eta, wouldBe, refusalCopy, tabTitle, facts } from './guest/src/queue/copy';
  export { detect, Buzz, VIBRATE } from './guest/src/queue/buzz';
  export { queueTvFrom, hm } from './guest/src/queue/tv';
`);

const box = (scene) => new QueueBox(`queue-${scene}--1`);
const viewOf = (scene, seated = true) => m.queueFrom(box(scene).env(seated)).view;

test('copy: ordinals, the box clock, minutes only with a measured average', () => {
  assert.deepEqual([1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111].map(m.ordinal), ['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '101st', '111th']);
  assert.equal(m.untilText('13:00'), '1 PM');
  assert.equal(m.untilText('12:30'), '12:30 PM');
  assert.equal(m.untilText('00:05'), '12:05 AM');
  assert.equal(m.minutesFor(4, 3), 12);
  assert.equal(m.minutesFor(4, null), null);
  assert.equal(m.minutesFor(0, 3), null);
});

test('view: the take screen, before 3 calls there are no minutes anywhere', () => {
  const v = viewOf('take-early', false);
  assert.equal(m.phaseOf(v), 'take');
  assert.equal(v.avg_minutes, null);
  assert.deepEqual(m.wouldBe(v)[1], '.');
  assert.ok(!m.facts(v).some(([, x]) => /min/.test(x)));
  const w = viewOf('take', false);
  assert.match(m.wouldBe(w).join(''), /^6th, about 18 minutes\.$/);
});

test('view: each scene maps to its phase, and a ticket only counts when this phone is seated', () => {
  const want = { take: 'take', ticket: 'waiting', next: 'next', turn: 'turn', skipped: 'skipped', served: 'served', left: 'left', closed: 'closed', ended: 'ended' };
  for (const [scene, phase] of Object.entries(want)) assert.equal(m.phaseOf(viewOf(scene)), phase, scene);
  const notMe = m.queueFrom({ ...box('ticket').env(true), me: undefined }).view;
  assert.equal(notMe.my_ticket, null);
  assert.equal(m.phaseOf(notMe), 'take');
  assert.ok(m.live('turn') && m.live('next') && m.live('waiting') && !m.live('served'));
});

test('view: junk is dropped, never drawn', () => {
  assert.equal(m.queueFrom(null), undefined);
  assert.equal(m.ticketFrom({ number: 3, state: 'gone' }), null);
  assert.equal(m.ticketFrom({ state: 'waiting' }), null);
  const t = m.ticketFrom({ number: 9, state: 'served', place: 2, ahead: 1 });
  assert.equal(t.place, null, 'only a waiting ticket has a place');
  const e = box('ticket').env(true);
  e.view.hours_until = '25:99';
  e.view.avg_minutes = -2;
  e.view.state = 'later';
  const v = m.queueFrom(e).view;
  assert.equal(v.hours_until, undefined);
  assert.equal(v.avg_minutes, null);
  assert.equal(v.state, 'open');
});

test('copy: eta pill, tab titles per phase and the blink', () => {
  const v = viewOf('ticket');
  assert.equal(m.eta(v.my_ticket, v), 'about 12 minutes');
  assert.equal(m.eta(v.my_ticket, { ...v, avg_minutes: null }), null);
  assert.equal(m.tabTitle('waiting', v), "(4th) Tine's Bakery · #27");
  const t = viewOf('turn');
  assert.match(m.tabTitle('turn', t), /^\(!\) Your turn: #27/);
  assert.equal(m.tabTitle('turn', t, true), '#27');
});

test('copy: every refusal, the two 429 limits apart', () => {
  const v = viewOf('take', false);
  assert.match(m.refusalCopy('full', { ...v, max_waiting: 30 })[0], /full right now: 30 waiting/);
  assert.match(m.refusalCopy('paused', { ...v, pause_text: 'Back in 10 minutes' })[0], /Paused: “Back in 10 minutes”/);
  assert.match(m.refusalCopy('closed', v)[0], /closed for today/);
  assert.match(m.refusalCopy('slow', v)[0], /This phone/);
  assert.match(m.refusalCopy('slow_net', v)[0], /this network/);
  assert.equal(m.refusalCopy('name', v, 'Too long')[0], 'Too long');
  for (const s of ['slow', 'slow-net']) assert.equal(queueScenes[s].take.status, 429);
  assert.equal(queueScenes.slow.take.limit, 'phone');
  assert.equal(queueScenes['slow-net'].take.limit, 'network');
});

test('buzz: what this browser can really do; one cue vibrates once, sound only after a tap', () => {
  const calls = [];
  const android = { navigator: { userAgent: 'Mozilla/5.0 (Linux; Android 14)', vibrate: (p) => calls.push(p) } };
  assert.deepEqual(m.detect(android), { vibrate: true, chime: false, wake: false, ios: false });
  const iphone = m.detect({ navigator: { userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)' } });
  assert.equal(iphone.ios, true);
  assert.equal(iphone.vibrate, false);
  assert.equal(m.detect({ navigator: { userAgent: 'Macintosh', maxTouchPoints: 5 } }).ios, true, 'an iPad says Macintosh');
  const b = new m.Buzz(android);
  b.cue('next');
  b.cue('turn');
  assert.deepEqual(calls, [m.VIBRATE.next, m.VIBRATE.turn]);
  assert.equal(b.sound, false);
});

test('screen: its own parser keeps numbers only, never a name or a ticket', () => {
  const b = new QueueBox('screen-queue-tv-open--1');
  const env = b.env(false);
  const x = m.queueTvFrom(env);
  assert.ok(x);
  assert.equal(x.view.now_serving, 24);
  assert.deepEqual(x.view.next.slice(0, 3), [25, 26, 27]);
  const s = JSON.stringify(x.view);
  assert.ok(!/my_ticket|"name"/.test(s), s);
  const big = m.queueTvFrom({ ...env, view: { ...env.view, next: [1, 2, 3, 4, 5, 6, 7, 'x', { name: 'Mia' }] } });
  assert.deepEqual(big.view.next, [1, 2, 3, 4, 5]);
  assert.equal(m.queueTvFrom(null), undefined);
});

test('screen: the clock reads 12-hour, like the phone and the hours', () => {
  assert.match(m.hm('2026-11-24T23:05:00Z'), /^\d{1,2}:05 (AM|PM)$/);
  assert.equal(m.hm('junk'), '');
  assert.equal(m.hm(undefined), '');
});

test('source: tokens only, no HTML from data, no style attributes, no React or app.css in the guest', () => {
  const dir = new URL('../guest/src/queue/', import.meta.url);
  for (const f of readdirSync(dir)) {
    const s = readFileSync(new URL(f, dir), 'utf8');
    if (f.endsWith('.css')) assert.ok(!/#[0-9a-f]{3,8}\b/i.test(s), `${f}: a hex colour`);
    if (f.endsWith('.ts')) {
      assert.ok(!/innerHTML|outerHTML|insertAdjacentHTML|document\.write/.test(s), `${f}: HTML from strings`);
      assert.ok(!/setAttribute\(\s*['"]style|\.style\s*=|\.style\.(?!setProperty)\w+\s*=/.test(s), `${f}: a style attribute`);
      assert.ok(!/from ['"]react|app\.css/.test(s), `${f}: React or app.css`);
    }
  }
});
