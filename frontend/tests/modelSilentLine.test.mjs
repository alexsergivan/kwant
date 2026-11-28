// model-silent-3min (m7): a bot whose model sent one line and then nothing for 3 min says
// "No word from Claude · 4:47 PM" in the sidebar instead of "Working …"; the tooltip says
// "No word from Claude since 4:47\u00a0PM." and then the old line.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const m = await loadBundled(`export * from './src/h4Status';`);

const st = (status, reason_code, since = '2026-11-28T16:47:00Z', label = 'Claude') =>
  m.parseH4Status({ kwant_id: 'wri', status, reason_code, since, waits_on: label === null ? undefined : { kind: 'model', label }, held: [] });

test('a quiet model_silent status names the engine: "No word from Claude · <time>"', () => {
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent')), 'No word from Claude · 4:47\u00a0PM');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent'), true), 'No word from Claude since 4:47\u00a0PM');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent', null)), 'No word from Claude yet');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent', null), true), 'No word from Claude yet');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent', undefined, 'Codex')), 'No word from Codex · 4:47\u00a0PM');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent', undefined, null)), 'No word from the AI · 4:47\u00a0PM', 'no engine label → "the AI"');
  assert.equal(m.h4QuietLine(st('quiet', 'model_silent', null, null)), 'No word from the AI yet');
});

test('other statuses keep their own line', () => {
  for (const s of [st('quiet', 'no_step'), st('quiet', 'no_live_steps'), st('working', 'thinking'), null, undefined]) assert.equal(m.h4QuietLine(s), null);
});

test('the busy line is swapped only for a busy bot with a silent model; the old line goes in the tooltip after the full form', () => {
  const lines = { wri: 'Double-checking “Compare three caterers”', res: 'Working on Lunch', ed: 'Working on Menu' };
  const titles = { wri: 'Writer double-checks Researcher’s “Compare three caterers”' };
  const status = { wri: st('quiet', 'model_silent'), res: st('quiet', 'no_step'), ed: st('quiet', 'model_silent', null), gone: st('quiet', 'model_silent') };
  const out = m.withQuietLines(lines, status, titles);
  assert.deepEqual(out, { wri: 'No word from Claude · 4:47\u00a0PM', res: 'Working on Lunch', ed: 'No word from Claude yet' });
  assert.equal(titles.wri, 'No word from Claude since 4:47\u00a0PM. Writer double-checks Researcher’s “Compare three caterers”');
  assert.equal(titles.ed, 'No word from Claude yet. Working on Menu', 'no title yet → the old line');
  assert.deepEqual(Object.keys(titles).sort(), ['ed', 'wri']);
  assert.equal(lines.wri, 'Double-checking “Compare three caterers”', 'the input is not changed');
  assert.equal(m.withQuietLines(lines, null), lines, 'an older server (no status) keeps today’s lines');
});
