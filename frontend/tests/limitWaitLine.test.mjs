// limit-wait-status (m3): a bot whose turn is parked on a usage limit isn't "available · idle". The server
// says queued/usage_limit with back_at; the sidebar says "Back 4:42 PM · usage limit" (time first, so a cut in
// the narrow desktop line only eats "usage limit") and the dot is queued, the same reset time the hand-off card shows.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/h4Status';`);

const now = Date.parse('2026-11-28T15:40:00Z');
const st = (status, reason_code, back_at = '2026-11-28T16:42:00Z', kwant_id = 'res') =>
  m.parseH4Status({ kwant_id, status, reason_code, since: '2026-11-28T15:40:00Z', waits_on: { kind: 'usage_limit' }, back_at, held: [] });

test('a usage-limit wait says when the bot is back, on the person’s clock', () => {
  assert.equal(st('queued', 'usage_limit').back_at, '2026-11-28T16:42:00Z', 'back_at is read');
  assert.equal(m.h4LimitLine(st('queued', 'usage_limit'), now), 'Back 4:42 PM · usage limit');
  const tomorrow = m.h4LimitLine(st('queued', 'usage_limit', '2026-11-29T09:10:00Z'), now);
  assert.match(tomorrow, /^Back Sun 9:10 AM · usage limit$/, 'another day gets its weekday');
  const later = m.h4LimitLine(st('queued', 'usage_limit', '2026-12-06T09:10:00Z'), now);
  assert.match(later, /^Back (Dec 6|6 Dec) · usage limit$/, 'further out gets the date only');
  assert.equal(m.h4LimitLine(st('queued', 'usage_limit', null), now), 'Waits for the usage limit', 'no time → no "back"');
  assert.equal(m.h4LimitLine(st('queued', 'usage_limit', '2026-11-28T15:39:00Z'), now), 'Waits for the usage limit', 'a time already past → no "back"');
});

test('other statuses keep their own line', () => {
  for (const s of [st('queued', 'in_line'), st('available', 'idle'), st('quiet', 'model_silent'), null, undefined]) assert.equal(m.h4LimitLine(s, now), null);
});

test('a limit-waiting bot gets a sidebar line and a queued dot; a bot with a turn keeps its own', () => {
  const lines = { wri: 'Working on Menu' };
  const status = { res: st('queued', 'usage_limit'), wri: st('queued', 'usage_limit', null, 'wri'), ed: st('available', 'idle', null, 'ed') };
  assert.deepEqual(m.withLimitLines(lines, status, now), { wri: 'Working on Menu', res: 'Back 4:42 PM · usage limit' });
  assert.deepEqual(lines, { wri: 'Working on Menu' }, 'the input is not changed');
  assert.equal(m.withLimitLines(lines, null, now), lines, 'an older server (no status) keeps today’s lines');
  const dots = { wri: 'working' };
  assert.deepEqual(m.withLimitWaits(dots, status), { wri: 'working', res: 'queued' });
  assert.deepEqual(dots, { wri: 'working' });
  assert.equal(m.withLimitWaits(dots, undefined), dots);
});

test('the sidebar is wired to it', async () => {
  const { readFile } = await import('node:fs/promises');
  const app = await readFile(new URL('../src/App.tsx', import.meta.url), 'utf8');
  assert.match(app, /withLimitLines\(\s*withQuietLines\(/);
  assert.match(app, /withLimitWaits\(botStatuses\(/);
});
