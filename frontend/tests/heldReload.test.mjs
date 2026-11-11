// 0.9.0 #510 QA should-fix 1+2 (qa-510.md): the held line comes back from the server's `held[]` after a
// reload, on another device, and for every earlier held message; nit 2: Try again clears only released lines.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/h4Status';`);

const names = { jun: 'Juno', lin: 'Lin' };
const nameOf = (id) => names[id];
const msg = (id, extra = {}) => ({ kind: 'message', id, since: '2026-11-11T04:10:00Z', reason_code: 'stuck', ...extra });
const gaveUp = (held) => m.parseH4Status({ kwant_id: 'jun', status: 'stuck', reason_code: 'stalled_gave_up', stuck: { gave_up_at: '2026-11-11T04:05:00Z', tries: 2 }, held });

test('reload with 1 held message: its line comes back from the status, with Try again', () => {
  const status = { jun: gaveUp([msg('m-1')]) };
  const rows = m.heldMessages([], status, nameOf);
  assert.equal(rows.length, 1);
  assert.deepEqual([rows[0].messageId, rows[0].kwantId, rows[0].name, rows[0].reason], ['m-1', 'jun', 'Juno', 'stuck']);
  const line = m.heldLine(rows[0], status.jun);
  assert.equal(line.tryAgain, true);
  assert.match(line.text, /^Waiting for Juno: Juno stopped after 2 tries at 04:05\. Your message is kept/);
});

test('reload with 3 held messages to one bot: every one gets its own line; hand-offs get none', () => {
  const status = { jun: gaveUp([msg('m-1'), msg('m-2'), { kind: 'handoff', id: 'd-1' }, msg('m-3')]) };
  const rows = m.heldMessages([], status, nameOf);
  assert.deepEqual(rows.map((r) => r.messageId), ['m-1', 'm-2', 'm-3']);
  assert.ok(rows.every((r) => m.heldLine(r, status.jun).tryAgain));
});

test('the instant send reply and the status entry for the same message make one line, not two', () => {
  const sent = m.heldFromReply({ status: 'held', user_message_id: 'm-3', held: { kwant_id: 'jun', kwant_name: 'Juno', reason_code: 'stuck' } }, 'tmp-3');
  // Before the status catches up: the reply's line alone.
  assert.deepEqual(m.heldMessages([sent], { jun: gaveUp([msg('m-1')]) }, nameOf).map((r) => r.messageId), ['m-3', 'm-1']);
  // After: still one line for m-3 (the reply's, keyed by its temp id too).
  const rows = m.heldMessages([sent], { jun: gaveUp([msg('m-1'), msg('m-3')]) }, nameOf);
  assert.deepEqual(rows.map((r) => r.messageId), ['m-3', 'm-1']);
  assert.equal(rows[0].tempId, 'tmp-3');
});

test('a waits_on_you hold rebuilt from the status keeps the question time; unknown bot falls back to "The bot"', () => {
  const s = m.parseH4Status({ kwant_id: 'lin', status: 'needs_you', reason_code: 'asked_question', question: { id: 'q', asked_at: '2026-11-11T03:58:00Z' }, held: [msg('m-7', { reason_code: 'waits_on_you' })] });
  const [row] = m.heldMessages([], { lin: s }, nameOf);
  assert.equal(row.reason, 'waits_on_you');
  assert.match(m.heldLine(row, s).text, /^Queued for Lin: Lin is waiting for your answer to its question \(03:58\)\./);
  const [anon] = m.heldMessages([], { zed: { ...s, kwant_id: 'zed' } }, nameOf);
  assert.equal(anon.name, 'The bot');
});

test('release clears the lines: no held[] means no line; Try again keeps only what the server still holds', () => {
  assert.deepEqual(m.heldMessages([], { jun: gaveUp([]) }, nameOf), []);
  assert.deepEqual(m.heldMessages([], null, nameOf), []);
  const a = m.heldFromReply({ status: 'held', user_message_id: 'm-1', held: { kwant_id: 'jun', reason_code: 'stuck' } }, 't1');
  const b = m.heldFromReply({ status: 'held', user_message_id: 'm-2', held: { kwant_id: 'jun', reason_code: 'stuck' } }, 't2');
  const c = m.heldFromReply({ status: 'held', user_message_id: 'm-5', held: { kwant_id: 'lin', reason_code: 'stuck' } }, 't5');
  // Server released m-1 but still holds m-2: only m-1's line goes; Lin's is untouched.
  assert.deepEqual(m.afterRelease([a, b, c], 'jun', gaveUp([msg('m-2')])).map((h) => h.messageId), ['m-2', 'm-5']);
  // Released everything (status back to available).
  const avail = m.parseH4Status({ kwant_id: 'jun', status: 'available', reason_code: '', held: [] });
  assert.deepEqual(m.afterRelease([a, b, c], 'jun', avail).map((h) => h.messageId), ['m-5']);
  // No status in the reply: today's behaviour, all of that bot's lines go.
  assert.deepEqual(m.afterRelease([a, b, c], 'jun', null).map((h) => h.messageId), ['m-5']);
});
