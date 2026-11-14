// Routines setup (0.9.9, Mira's audit #9): the sidebar row, the card's first line and next run,
// the time zone as a city with Change, and the Weekdays shortcut chip.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { RoutineRow, RoutineForm } from './src/components/RoutinesPanel';
  import { Sidebar } from './src/components/Sidebar';
  const r = (C) => (props) => renderToStaticMarkup(React.createElement(C, props));
  export const row = r(RoutineRow);
  export const form = r(RoutineForm);
  export const sidebar = r(Sidebar);
`);

const noop = () => {};
const text = (html) => html.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ');
const routine = (over = {}) => ({
  id: 'r1', kwant_id: 'k1', channel_id: null, name: 'Competitor prices',
  instructions: 'Check Beanline and RoastCo prices and tell me what changed since last week.',
  schedule: { type: 'weekly', time: '09:00', weekdays: ['mon'], tz: 'Europe/Kyiv' }, enabled: true,
  next_run_at: '2026-11-16T07:00:00Z', last_run_at: '2026-11-09T07:00:00Z', last_status: 'ok', created_at: '2026-10-20T08:00:00Z', ...over,
});
const rowProps = (over = {}) => ({ routine: routine(over), kwantName: 'Ben', placeLabel: '#spring-launch', tick: 0, onEdit: noop, onChanged: noop });

test('the card says what it does, then when it runs next, then schedule · bot · place', () => {
  const h = ui.row(rowProps());
  const t = text(h);
  assert.match(h, /data-routine-what[^>]*>Check Beanline and RoastCo prices and tell me what changed since last week\.</);
  assert.match(h, /title="Check Beanline/);
  assert.match(t, /Next [A-Z][a-z]{2}, Nov 1[56]/);
  assert.match(t, /last ran Mon, Nov 9/);
  assert.match(t, /Every Monday at 09:00, Kyiv time · Ben · #spring-launch/);
  assert.doesNotMatch(t, /Next run:|\(Europe\/Kyiv\)/);
  // Order: name, first line, next, meta.
  assert.ok(t.indexOf('Competitor prices') < t.indexOf('Check Beanline'));
  assert.ok(t.indexOf('Check Beanline') < t.indexOf('Next '));
  assert.ok(t.indexOf('Next ') < t.indexOf('Every Monday'));
});

test('a multi-line routine shows its first line with an ellipsis; the whole text is in the title', () => {
  const h = ui.row(rowProps({ instructions: '\n  Summarise new web orders.\nThen list anyone waiting on a reply.' }));
  assert.match(h, /data-routine-what[^>]*>Summarise new web orders\. …</);
});

test('a paused routine says Paused and mutes its name and first line', () => {
  const h = ui.row(rowProps({ enabled: false }));
  assert.match(h, /text-zinc-300 font-medium">Paused</);
  assert.doesNotMatch(text(h), /Next /);
  assert.match(h, /font-medium break-words text-zinc-400">Competitor prices</);
});

test('a Mon–Fri routine reads "Weekdays at …"', () => {
  const t = text(ui.row(rowProps({ schedule: { type: 'weekly', time: '08:30', weekdays: ['mon', 'tue', 'wed', 'thu', 'fri'], tz: 'Europe/Kyiv' } })));
  assert.match(t, /Weekdays at 08:30, Kyiv time/);
});

test('phone: the four actions sit in a 4-column grid of 44 px buttons; desktop keeps the indented row', () => {
  const h = ui.row(rowProps());
  assert.match(h, /grid grid-cols-4 gap-1\.5 sm:flex sm:flex-wrap sm:items-center sm:pl-9/);
  assert.equal((h.match(/min-h-11 sm:min-h-0 justify-center whitespace-nowrap/g) || []).length, 4, 'each label stays on one line');
  // Like History's chevron, Run now's Play icon is desktop only, so "Run now" fits a phone column.
  assert.match(h, /hidden sm:block[^"]*lucide-play|lucide-play[^"]*hidden sm:block/);
});

test('Settings scrolls the open tab into view (phone tab strip), without motion', async () => {
  const { readFileSync } = await import('node:fs');
  const s = readFileSync(new URL('../src/components/SpaceSettings.tsx', import.meta.url), 'utf8');
  assert.match(s, /ref=\{tab === t\.id \? currentTabRef : undefined\}\s*aria-current=\{tab === t\.id \? 'page'/);
  assert.match(s, /currentTabRef\.current\?\.scrollIntoView\?\.\(\{ block: 'nearest', inline: 'nearest' \}\);\s*\}, \[tab, scope\]\)/);
});

const kwants = [{ id: 'k1', name: 'Ben' }];
const weekly = { kwant_id: 'k1', channel_id: null, name: 'x', instructions: 'y', schedule: { type: 'weekly', time: '09:00', weekdays: ['fri'], tz: 'Europe/Kyiv' } };

test('the new form has the paired example placeholders', () => {
  const h = ui.form({ kwants, channels: [], onCancel: noop, onSaved: noop });
  assert.match(h, /placeholder="Competitor prices"/);
  assert.match(h, /placeholder="Check our 3 competitors&#x27; prices and tell me what changed since last time\."/);
});

test('the time zone shows as a city with its offset and Change; the full id is in the title', () => {
  const h = ui.form({ kwants, channels: [], seed: weekly, onCancel: noop, onSaved: noop });
  assert.match(h, /title="Europe\/Kyiv"[^>]*data-tz-row/);
  assert.match(text(h), /Kyiv time\s*GMT\+[23]\s*Change/);
  assert.match(h, /data-tz-change/);
  assert.doesNotMatch(h, /<datalist/);
});

test('a zone Intl does not know starts open, so the saved value shows', () => {
  const h = ui.form({ kwants, channels: [], seed: { ...weekly, schedule: { ...weekly.schedule, tz: 'Mars/Olympus' } }, onCancel: noop, onSaved: noop });
  assert.match(h, /value="Mars\/Olympus"/);
  assert.doesNotMatch(h, /data-tz-change/);
});

test('Weekly has a Weekdays chip, pressed only for exactly Mon–Fri, and every day chip has a focus ring', () => {
  const one = ui.form({ kwants, channels: [], seed: weekly, onCancel: noop, onSaved: noop });
  assert.match(one, /aria-pressed="false" aria-label="Weekdays, Monday to Friday"/);
  assert.match(one, /Weekdays<span class="sm:hidden"> \(Mon–Fri\)<\/span>/);
  const days = one.match(/<button[^>]*aria-pressed="(true|false)"[^>]*>(Mon|Tue|Wed|Thu|Fri|Sat|Sun)</g);
  assert.equal(days.length, 7);
  for (const d of days) assert.match(d, /focus-visible:outline-sky-500/);
  const five = ui.form({ kwants, channels: [], seed: { ...weekly, schedule: { ...weekly.schedule, weekdays: ['mon', 'tue', 'wed', 'thu', 'fri'] } }, onCancel: noop, onSaved: noop });
  assert.match(five, /aria-pressed="true" aria-label="Weekdays, Monday to Friday"/);
});

test('the sidebar has a Routines row after Mission board only when it can open them', () => {
  const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
    created_at: '', general_channel_id: null, counts: { bots: 0, ready_accounts: 0, unread: 0 } };
  const base = { space, spaces: [space], onSwitchSpace: noop, onCreateSpace: noop, onOpenSpaceSettings: noop, onOpenArchived: noop,
    onOpenSearch: noop, kwants: [], channels: [], teams: [], selectedId: '', isChannel: true, onSelectKwant: noop, onSelectChannel: noop,
    onSelectTeam: noop, onSummonKwant: noop, onCreateTeam: noop, onOpenInbox: noop, missionBoard: true, onOpenMissions: noop };
  assert.doesNotMatch(ui.sidebar(base), /data-routines-row/);
  const h = ui.sidebar({ ...base, onOpenRoutines: noop });
  assert.match(h, /data-routines-row/);
  const row = h.slice(h.indexOf('data-routines-row') - 300, h.indexOf('data-routines-row') + 600);
  assert.doesNotMatch(row.slice(row.lastIndexOf('<button')), /aria-current/);
  assert.ok(h.indexOf('Mission board') < h.indexOf('>Routines<'), 'after Mission board');
});
