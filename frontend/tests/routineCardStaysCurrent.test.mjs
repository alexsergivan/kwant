// routine-card-stays-current: the chat card that made a routine shows the routine as it is now (Mira's
// ux-pass-0945-routines report, section 4). Stored details say 8:00 AM and on; Routines says 4:20 PM and paused.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

const b = await loadBundled(`export * from './src/botActions'; export * from './src/routineNow';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { BotActionCard } from './src/components/BotActionCard';
  export const action = (props) => renderToStaticMarkup(React.createElement(BotActionCard, props));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/[^\S ]+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((m) => text(m[1]));

const weekdays = ['mon', 'tue', 'wed', 'thu', 'fri'];
const meta = (over = {}) =>
  b.parseBotActionMeta({
    kind: 'bot_action', action_id: 'a1', action: 'routine_create', kwant_id: 'k1', target_kind: 'routine', target_id: 'r1',
    summary: "created routine 'Morning brief', weekly mon,tue,wed,thu,fri 08:00 UTC, delivering to #general",
    details: {
      routine_id: 'r1', name: 'Morning brief', kwant_id: 'k1', bot_name: 'Coach',
      schedule: { type: 'weekly', time: '08:00', weekdays, tz: 'UTC' }, schedule_text: 'weekly mon,tue,wed,thu,fri 08:00 UTC', enabled: true,
      next_run_at: '2026-11-24T08:00:00Z', channel_id: 'c1', delivers_to: '#general',
    },
    status: 'applied', can_undo: true, needs_confirmation: false, created_at: '2026-11-23T10:00:00Z', resolved_at: null, ...over,
  });
const routine = (over = {}) => ({
  id: 'r1', kwant_id: 'k1', channel_id: 'c1', name: 'Morning brief', instructions: 'x',
  schedule: { type: 'weekly', time: '16:20', weekdays, tz: 'UTC' }, schedule_text: 'weekly mon,tue,wed,thu,fri 16:20 UTC',
  enabled: false, next_run_at: null, last_run_at: null, last_status: null, created_at: '2026-11-23T10:00:00Z', ...over,
});
const props = (m, over = {}) => ({ meta: m, messageId: 'm1', kwantName: 'Coach', kwantId: 'k1', onDecide: async () => {}, onOpenRoutine: () => {}, ...over });

test('a card made at 8:00 AM shows the routine as it is now: 4:20 PM and Paused, with Resume', () => {
  const html = ui.action(props(meta(), { routine: routine() }));
  const t = text(html);
  assert.match(t, /When Weekdays at 4:20 PM/, t);
  assert.doesNotMatch(t, /8:00/, t);
  assert.match(t, /Next run Paused/, t);
  assert.deepEqual(buttons(html), ['Undo', 'Edit', 'Resume']);
  // The pure parts agree.
  assert.deepEqual(b.routineButtons(meta(), routine()), { edit: true, pause: false, resume: true });
  assert.equal(b.routineFacts(meta(), routine()).find((f) => f.label === 'When').value.includes('4:20'), true);
});

test('turned back on, it shows the current next run and offers Pause', () => {
  const html = ui.action(props(meta(), { routine: routine({ enabled: true, next_run_at: '2026-11-24T16:20:00Z' }) }));
  const t = text(html);
  assert.doesNotMatch(t, /Paused/);
  assert.match(t, /Next run \S+/);
  assert.deepEqual(buttons(html), ['Undo', 'Edit', 'Pause']);
});

test('the headline does not repeat the schedule the table shows', () => {
  const t = text(ui.action(props(meta(), { routine: routine() })));
  assert.doesNotMatch(t, /08:00 UTC/, t);
  assert.match(t, /Created routine 'Morning brief', delivering to #general/, t);
});

test('a routine that is gone has no When row or raw schedule, says it won\'t run again and offers no buttons', () => {
  const html = ui.action(props(meta(), { routine: null }));
  const t = text(html);
  assert.match(t, /It's since been deleted, so it won't run again\./, t);
  assert.doesNotMatch(t, /When|8:00|08:00|weekly/, t);
  assert.doesNotMatch(t, /Next run/, t);
  assert.deepEqual(buttons(html), []);
});

test('before the list loads, the stored facts show as before', () => {
  const html = ui.action(props(meta()));
  assert.match(text(html), /When Weekdays at 8:00 AM/);
  assert.deepEqual(buttons(html), ['Undo', 'Edit', 'Pause']);
});

test('run now keeps its own This run row; delete cards are unchanged', () => {
  const run = meta({ action: 'routine_run_now', can_undo: false, details: { ...meta().details, run_id: 'x', run_status: 'needs_you' } });
  const t = text(ui.action(props(run, { routine: routine() })));
  assert.match(t, /This run Waiting on you/, t);
  assert.doesNotMatch(t, /Next run/);
  const inc = meta({ action: 'routine_run_now', can_undo: false, details: { ...meta().details, run_id: 'x', run_status: 'incomplete' } });
  assert.match(text(ui.action(props(inc))), /This run A step failed/);
  const del = meta({ action: 'routine_delete', target_id: 'r1' });
  const dt = text(ui.action(props(del, { routine: null })));
  assert.doesNotMatch(dt, /Deleted|since been/, dt);
});

test('Pause and Resume PATCH { enabled } in place and the store shows the new state', async () => {
  const calls = [];
  let list = [routine({ enabled: true, next_run_at: '2026-11-24T16:20:00Z' })];
  const api = {
    routines: async () => list,
    updateRoutine: async (id, body) => {
      calls.push([id, body]);
      list = list.map((r) => (r.id === id ? { ...r, ...body, next_run_at: body.enabled ? '2026-11-24T16:20:00Z' : null } : r));
      return list.find((r) => r.id === id);
    },
  };
  const s = b.createRoutineStore(api);
  assert.equal(s.get('r1'), undefined);
  const off = s.subscribe(() => {});
  await s.load();
  assert.equal(s.get('r1').enabled, true);
  assert.equal(s.get('nope'), null);
  assert.equal(await s.setEnabled('r1', false), undefined);
  assert.deepEqual(calls, [['r1', { enabled: false }]]);
  assert.equal(s.get('r1').enabled, false);
  await s.setEnabled('r1', true);
  assert.deepEqual(calls[1], ['r1', { enabled: true }]);
  // A failure comes back as words and leaves the state as it was.
  api.updateRoutine = async () => {
    throw new Error('It no longer exists.');
  };
  assert.equal(await s.setEnabled('r1', false), 'It no longer exists.');
  assert.equal(s.get('r1').enabled, true);
  off();
});

test('one shared request for every card; it refetches when told a routine changed', async () => {
  let n = 0;
  const s = b.createRoutineStore({ routines: async () => (n++, [routine()]), updateRoutine: async () => routine() });
  const a = s.subscribe(() => {});
  const c = s.subscribe(() => {});
  await s.load();
  assert.equal(n, 1);
  await s.refresh();
  assert.equal(n, 2);
  a();
  c();
});
