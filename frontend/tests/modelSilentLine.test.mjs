// model-silent-3min (m7): a bot whose model sent one line and then nothing for 3 min says
// "No word from the model yet" in the sidebar instead of "Working …"; the old line moves to its tooltip.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/h4Status';`);

const st = (status, reason_code, since = '2026-11-28T16:47:00Z') => m.parseH4Status({ kwant_id: 'wri', status, reason_code, since, held: [] });

test('a quiet model_silent status reads "No word from the model yet · since <time>"', () => {
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent')), 'No word from the model yet · since 4:47 PM');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent', null)), 'No word from the model yet');
});

test('other statuses keep their own line', () => {
  for (const s of [st('quiet', 'no_step'), st('quiet', 'no_live_steps'), st('working', 'thinking'), null, undefined]) assert.equal(m.h4QuietLine(s), null);
});

test('the busy line is swapped only for a busy bot with a silent model; the old line becomes the tooltip', () => {
  const lines = { wri: 'Double-checking “Compare three caterers”', res: 'Working on Lunch' };
  const titles = { wri: 'Writer double-checks Researcher’s “Compare three caterers”' };
  const status = { wri: st('quiet', 'model_silent'), res: st('quiet', 'no_step'), gone: st('quiet', 'model_silent') };
  const out = m.withQuietLines(lines, status, titles);
  assert.deepEqual(out, { wri: 'No word from the model yet · since 4:47 PM', res: 'Working on Lunch' });
  assert.equal(titles.wri, 'No word from the model yet · since 4:47 PM. Writer double-checks Researcher’s “Compare three caterers”');
  assert.deepEqual(Object.keys(titles), ['wri']);
  assert.equal(lines.wri, 'Double-checking “Compare three caterers”', 'the input is not changed');
  assert.equal(m.withQuietLines(lines, null), lines, 'an older server (no status) keeps today’s lines');
});
