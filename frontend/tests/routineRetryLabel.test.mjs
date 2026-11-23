// routine-retry-once (autonomy/routine-retry-once/brief.md): AutoFix's own run of a failed routine is
// recorded with trigger "retry", and History says "Retried automatically" for it, never "Run now" for a
// press the person didn't make (Mira's routine-for-a-week pass, section 1).
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { RunRow } from './src/components/RoutinesPanel';
  export const run = (props) => renderToStaticMarkup(React.createElement(RunRow, props));
`);

const routine = { id: 'rt1', kwant_id: 'k1', channel_id: null, thread_id: null, name: 'Morning digest' };
const runOf = (trigger) => ({ id: `r-${trigger}`, routine_id: 'rt1', trigger, started_at: '2026-11-23T08:00:00Z', ended_at: '2026-11-23T08:01:00Z', status: 'error', error: 'the model stopped', message_id: null });

test('RunRow: trigger "retry" reads "Retried automatically"', () => {
  const html = ui.run({ routine, run: runOf('retry') });
  assert.match(html, />Retried automatically</);
  assert.doesNotMatch(html, />Run now</, 'AutoFix pressed nothing');
  assert.doesNotMatch(html, />Scheduled</);
});

test('RunRow: a press still reads "Run now" and a scheduled run "Scheduled"', () => {
  assert.match(ui.run({ routine, run: runOf('manual') }), />Run now</);
  assert.match(ui.run({ routine, run: runOf('schedule') }), />Scheduled</);
});
