// 31f routine-card-says-it-once (autonomy/routine-card-says-it-once/brief.md, Mira's UX pass 0.9.31 rows c and g):
// while one run is going the card says "Running" once, in its header badge; that run's History row keeps the badge
// box and spinner with "Running" for screen readers only. The phone Run now button shows its Play icon too.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { RoutineRow, RunRow } from './src/components/RoutinesPanel';
  export const card = (props) => renderToStaticMarkup(React.createElement(RoutineRow, props));
  export const run = (props) => renderToStaticMarkup(React.createElement(RunRow, props));
`);

const routine = {
  id: 'rt1', kwant_id: 'k1', channel_id: null, thread_id: null, name: 'Morning note', instructions: 'Write the morning note.',
  enabled: true, schedule: { kind: 'daily', time: '08:00' }, schedule_text: 'Every day at 8:00', next_run_at: '2026-11-21T08:00:00Z',
  last_run_at: '2026-11-20T08:00:00Z', last_status: 'running', last_error: null, origin: 'user',
};
const runOf = (id, status, error = null) => ({ id, routine_id: 'rt1', trigger: 'manual', started_at: '2026-11-20T08:00:00Z', ended_at: null, status, error, message_id: null });
const cardProps = { routine, placeLabel: 'Chat', tick: 0, onEdit: () => {}, onChanged: () => {} };

/** Text a sighted person sees: drop sr-only spans and tags, then count the word. */
const visible = (html) => html.replace(/<span class="sr-only">[^<]*<\/span>/g, '').replace(/<[^>]+>/g, ' ');
const count = (text, word) => (text.match(new RegExp(`\\b${word}\\b`, 'g')) ?? []).length;

test('one run going, History open: the card shows exactly one visible "Running"', () => {
  const header = ui.card(cardProps);
  const row = ui.run({ routine, run: runOf('r-live', 'running') });
  // The card with History open is the header plus its running run's row.
  assert.equal(count(visible(header + row), 'Running'), 1, 'only the header badge says it');
  assert.match(header, /data-run-badge="running"[^>]*>.*Running<\/span>/, 'the header badge keeps its word');
});

test('the running History row keeps its badge box, spinner and an sr-only "Running"', () => {
  const row = ui.run({ routine, run: runOf('r-live', 'running') });
  assert.match(row, /data-run-state="running"/);
  assert.match(row, /data-run-badge="running"/, 'the badge box stays');
  assert.match(row, /data-run-badge="running"[^>]*><svg[^>]*animate-spin/, 'the spinner stays');
  assert.match(row, /<span class="sr-only">Running<\/span>/, 'a screen reader still hears it');
  assert.equal(count(visible(row), 'Running'), 0);
});

test('a finished and a failed run row keep their words', () => {
  const ok = ui.run({ routine, run: runOf('r-ok', 'ok') });
  assert.match(ok, /data-run-state="ok"/);
  assert.doesNotMatch(ok, /sr-only/);
  const okLabel = ok.match(/data-run-badge="ok"[^>]*>([^<]+)</);
  assert.ok(okLabel && okLabel[1].trim().length > 0, 'the ok badge shows its word');

  const fail = ui.run({ routine, run: runOf('r-fail', 'incomplete', 'Bash failed') });
  assert.match(fail, /data-run-badge="error"[^>]*>A step failed</);
  assert.doesNotMatch(fail, /sr-only/);
});

test('the Run now button shows its Play icon on a phone too', () => {
  const html = ui.card(cardProps);
  const btn = html.match(/<button[^>]*>(?:(?!<\/button>).)*Run now<\/button>/s);
  assert.ok(btn, 'Run now button rendered');
  assert.match(btn[0], /<svg[^>]*lucide-play/, 'Play icon present');
  assert.doesNotMatch(btn[0], /<svg[^>]*class="[^"]*\bhidden\b/, 'the icon is not hidden on small screens');
});
