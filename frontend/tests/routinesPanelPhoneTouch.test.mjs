// routines-panel-phone-touch (autonomy/routines-panel-phone-touch/brief.md, Mira's ux-pass-0945-routines Minor d, e, k):
// a paused routine's pill says plain "Paused" (not the OK green) and its last result moves into the "last ran" line;
// on a phone the on/off switch and History's "Open result" are 44 px hit areas. Desktop keeps its sizes.
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

const base = {
  id: 'rt1', kwant_id: 'k1', channel_id: null, thread_id: null, name: 'Morning note', instructions: 'Write the morning note.',
  enabled: true, schedule: { kind: 'daily', time: '08:00' }, schedule_text: 'Every day at 8:00', next_run_at: '2026-11-24T08:00:00Z',
  last_run_at: '2026-11-23T08:00:00Z', last_status: 'ok', last_error: null, origin: 'user',
};
const card = (over) => ui.card({ routine: { ...base, ...over }, placeLabel: 'Chat', tick: 0, onEdit: () => {}, onChanged: () => {} });
const text = (html) => html.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ');

test('k: a paused routine shows a plain "Paused" pill, not the OK green', () => {
  const html = card({ enabled: false });
  assert.match(html, /data-routine-paused[^>]*>Paused<\/span>/, 'the pill says Paused');
  assert.doesNotMatch(html, /data-run-badge="ok"/, 'no OK pill while paused');
  assert.doesNotMatch(html, /data-routine-paused[^>]*class="[^"]*text-ok|class="[^"]*text-ok[^"]*"[^>]*data-routine-paused/, 'not the OK green');
});

test('k: the last result moves into the "last ran" line, with the existing status words', () => {
  const ok = text(card({ enabled: false }));
  assert.match(ok, /last ran [^·]+, OK/, 'OK result in the line');
  const failed = text(card({ enabled: false, last_status: 'incomplete', last_error: 'Bash failed' }));
  assert.match(failed, /last ran [^·]+, a step failed/, 'failed result in the line');
});

test('k: an enabled routine is unchanged (OK pill, no result in the line)', () => {
  const html = card({});
  assert.match(html, /data-run-badge="ok"[^>]*>OK<\/span>/);
  assert.doesNotMatch(html, /data-routine-paused/);
  assert.doesNotMatch(text(html), /last ran [^·]+, OK/);
});

test('k: a run going on a paused routine keeps its live badge', () => {
  const html = card({ enabled: false, last_status: 'running' });
  assert.match(html, /data-run-badge="running"/);
});

test('d: on a phone the switch is a 44x44 hit area; desktop keeps 36x24', () => {
  const label = card({}).match(/<label[^>]*title="Turn off"[^>]*>/);
  assert.ok(label, 'switch label rendered');
  for (const c of ['w-11', 'h-11', 'sm:w-auto', 'sm:h-auto', 'sm:p-1', 'sm:-m-1', 'sm:mt-0']) assert.match(label[0], new RegExp(`class="[^"]*(?<![\\w:-])${c.replace(/[-:]/g, '\\$&')}(?![\\w-])`), c);
});

test('e: "Open result" keeps main\'s classes; its 44 px phone tap area is kw-hit, not a taller row (Round 2)', () => {
  const row = ui.run({ routine: base, run: { id: 'r1', routine_id: 'rt1', trigger: 'schedule', started_at: '2026-11-23T08:00:00Z', ended_at: null, status: 'ok', error: null, message_id: 'm1' }, onOpenRunResult: () => {} });
  const btn = row.match(/<button[^>]*data-run-result[^>]*>/);
  assert.ok(btn, 'Open result rendered');
  assert.match(btn[0], /\bkw-hit\b/);
  assert.doesNotMatch(btn[0], /min-h-11/);
});

test('k: a paused routine that never ran shows "Paused" once (no pill)', () => {
  const html = card({ enabled: false, last_status: null, last_run_at: null });
  assert.doesNotMatch(html, /data-routine-paused/);
  assert.doesNotMatch(html, /data-run-badge/);
  assert.equal((html.replace(/<[^>]+>/g, ' ').match(/\bPaused\b/g) ?? []).length, 1);
});
