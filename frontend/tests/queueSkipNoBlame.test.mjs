// queue-skip-no-blame (autonomy/queue-skip-no-blame/brief.md, Mira's ux-pass-queue-ticket m1): a ticket the host
// skipped by hand before the call time ran out says it was skipped and, for the server's 10 recall minutes, that the
// host can bring it back. The auto-skip (call time ran out) keeps "nobody came in 3 minutes".
import './pinLocale.mjs';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`
  export { skippedLines, skippedByHand, RECALL_MINUTES } from './guest/src/queue/copy';
  export { clock } from './guest/src/app';
`);

const T0 = Date.parse('2026-11-28T16:42:00Z');
const iso = (min) => new Date(T0 + min * 60000).toISOString();
const view = (extra = {}) => ({ state: 'open', title: "Tine's Bakery", line: '', now_serving: 28, waiting: 6, avg_minutes: 3, call_minutes: 3, ask_name: false, host_name: 'Tine', pause_text: '', close_text: '', my_ticket: null, joined: true, ...extra });
const ticket = (called, done) => ({ number: 3, place: null, ahead: null, state: 'skipped', called_at: iso(called), done_at: iso(done), closed: false });

test('hand skip within 10 minutes: skipped at, and the host can bring it back until done + 10', () => {
  const t = ticket(0, 0.1);
  assert.equal(m.skippedByHand(t, view()), true);
  const sk = m.skippedLines(t, view(), T0 + 2 * 60000);
  assert.equal(sk.word, 'Skipped');
  const at = m.clock(iso(0.1));
  const until = m.clock(iso(10.1));
  assert.equal(sk.lead, `Number #3 was skipped at ${at}. If you're here, show this at the counter: Tine can bring it back until ${until}.`);
  assert.doesNotMatch(sk.lead, /nobody came|missed/);
  assert.equal(sk.after, 'Now 6 waiting, about 18 minutes. Or ask at the counter.');
  assert.match(m.skippedLines(t, view({ host_name: '' }), T0).lead, /the host can bring it back/);
});

test('hand skip after 10 minutes: the plain "was skipped at" lead, no bring-back, no blame', () => {
  const t = ticket(0, 0.1);
  const sk = m.skippedLines(t, view(), T0 + 10.1 * 60000);
  assert.equal(sk.word, 'Skipped');
  assert.equal(sk.lead, `Number #3 was skipped at ${m.clock(iso(0.1))}.`);
  assert.equal(m.RECALL_MINUTES, 10);
  const rs = readFileSync(new URL('../../src/pocket/queue.rs', import.meta.url), 'utf8');
  assert.match(rs, /pub const RECALL_MINUTES: i64 = 10;/, 'the guest constant follows the server');
});

test('auto-skip (call time ran out) is unchanged', () => {
  const t = ticket(0, 3); // queue.rs sweep: done_at = called_at + call_minutes
  assert.equal(m.skippedByHand(t, view()), false);
  const sk = m.skippedLines(t, view(), T0 + 4 * 60000);
  assert.equal(sk.word, 'You missed your turn');
  assert.equal(sk.lead, `Number #3 was called at ${m.clock(iso(0))} and nobody came in 3 minutes.`);
  assert.equal(sk.after, 'Now 6 waiting, about 18 minutes. Or ask at the counter.');
  // No times at all: today's line without the clock.
  const bare = { ...t, called_at: undefined, done_at: undefined };
  assert.equal(m.skippedLines(bare, view(), T0).lead, 'Number #3 was called and nobody came in 3 minutes.');
});
