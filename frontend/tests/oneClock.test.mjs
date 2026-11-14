// One clock (roadmap 13c, autonomy/one-clock/brief.md): every time in the app goes through src/clock.ts on the
// device's own locale. The table pins en-US (12 h) and en-GB (24 h) by passing the locale; the app passes none.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

process.env.TZ = 'UTC';
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`
  export * from './src/clock';
  export { describeSchedule, formatWhen } from './src/continuity';
  export { clock as reviewClock } from './src/reviewItem';
  export { callTime, shortDate } from './src/agentConnections';
  export { hm } from './src/missionWatch';
  export { dayLabel as usageDay } from './src/components/DailyUsage';
  export { dayLabel as devDay } from './src/devMode';
  export { onceWords } from './src/botActions';
`);

// Some ICU builds put a narrow no-break space before AM/PM; compare on plain spaces.
const n = (s) => (typeof s === 'string' ? s.replace(/[  ]/g, ' ') : s);
const AT = '2026-11-16T20:15:00Z'; // a Monday
const MORNING = '2026-11-16T08:30:00Z';

const TABLE = [
  ['timeText', () => [m.timeText(AT, { locale: 'en-US' }), m.timeText(AT, { locale: 'en-GB' })], ['8:15 PM', '20:15']],
  ['timeText morning', () => [m.timeText(MORNING, { locale: 'en-US' }), m.timeText(MORNING, { locale: 'en-GB' })], ['8:30 AM', '08:30']],
  ['timeText with a zone', () => [m.timeText(AT, { locale: 'en-US', timeZone: 'Asia/Kolkata' }), m.timeText(AT, { locale: 'en-GB', timeZone: 'Asia/Kolkata' })], ['1:45 AM', '01:45']],
  ['dateText', () => [m.dateText(AT, { locale: 'en-US' }), m.dateText(AT, { locale: 'en-GB' })], ['Nov 16', '16 Nov']],
  ['dateText weekday + year', () => [m.dateText(AT, { locale: 'en-US', weekday: true, year: true }), m.dateText(AT, { locale: 'en-GB', weekday: true, year: true })], ['Mon, Nov 16, 2026', 'Mon, 16 Nov 2026']],
  ['weekdayClock (a Next line)', () => [m.weekdayClock(MORNING, { locale: 'en-US' }), m.weekdayClock(MORNING, { locale: 'en-GB' })], ['Mon, Nov 16, 8:30 AM', 'Mon 16 Nov, 08:30']],
  ['dateClock', () => [m.dateClock(AT, { locale: 'en-US' }), m.dateClock(AT, { locale: 'en-GB' })], ['Nov 16, 8:15 PM', '16 Nov, 20:15']],
  ['wallClock 08:30', () => [m.wallClock('08:30', 'en-US'), m.wallClock('08:30', 'en-GB')], ['8:30 AM', '08:30']],
  ['wallClock 20:05', () => [m.wallClock('20:05', 'en-US'), m.wallClock('20:05', 'en-GB')], ['8:05 PM', '20:05']],
  ['wallClock midnight', () => [m.wallClock('00:00', 'en-US'), m.wallClock('00:00', 'en-GB')], ['12:00 AM', '00:00']],
];

for (const [name, run, want] of TABLE) {
  test(`clock: ${name} in en-US and en-GB`, () => assert.deepEqual(run().map(n), want));
}

test('clock: missing or unreadable times give null; wallClock keeps what it cannot read', () => {
  for (const f of [m.timeText, m.dateText, m.weekdayClock, m.dateClock]) {
    assert.equal(f(null), null);
    assert.equal(f(''), null);
    assert.equal(f('not a time'), null);
  }
  assert.equal(m.wallClock('8.30'), '8.30');
  assert.equal(m.wallClock('25:00'), '25:00');
  assert.equal(m.wallClock(''), '');
});

test('clock: no locale means the device default, the same as the platform writes it', () => {
  const want = new Date(AT).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  assert.equal(m.timeText(AT), want);
  assert.equal(m.wallClock('20:15'), want);
});

test('routines: the schedule and its Next line use the same clock', () => {
  const s = { type: 'weekly', weekdays: ['monday'], time: '08:30', tz: 'UTC' };
  const line = n(m.describeSchedule(s));
  const want = n(m.wallClock('08:30'));
  assert.ok(line.includes(`at ${want},`), line);
  assert.ok(n(m.formatWhen(MORNING)).endsWith(want), m.formatWhen(MORNING));
  assert.equal(n(m.formatWhen(MORNING, 'en-US')), 'Mon, Nov 16, 8:30 AM');
  assert.equal(n(m.describeSchedule({ type: 'daily', time: '17:00', tz: 'UTC' })).includes(n(m.wallClock('17:00'))), true);
  assert.equal(m.formatWhen(null), '—');
});

test('call sites that forced 24 h or en-GB now follow the locale', () => {
  assert.equal(n(m.reviewClock(AT, 'en-US')), '8:15 PM', 'the Inbox');
  assert.equal(m.reviewClock(AT, 'en-GB'), '20:15');
  assert.equal(n(m.callTime(AT, new Date(AT), 'en-US')), '8:15 PM');
  assert.equal(n(m.callTime(AT, new Date('2026-11-17T09:00:00Z'), 'en-US')), 'Nov 16, 8:15 PM');
  assert.equal(m.callTime(AT, new Date('2026-11-17T09:00:00Z'), 'en-GB'), '16 Nov, 20:15');
  assert.equal(m.shortDate(AT, 'en-US'), 'Nov 16, 2026');
  assert.equal(m.shortDate(AT, 'en-GB'), '16 Nov 2026');
  assert.equal(n(m.hm(AT, 'UTC', 'en-US')), '8:15 PM', 'missionWatch keeps its zone');
  assert.equal(m.hm(AT, 'UTC', 'en-GB'), '20:15');
  assert.equal(m.usageDay('2026-10-20', new Date(AT), 'en-US'), 'Tue, Oct 20', 'DailyUsage stays on the UTC day');
  assert.equal(m.usageDay('2026-10-20', new Date(AT), 'en-GB'), 'Tue 20 Oct');
  assert.equal(m.devDay('2026-11-03T10:00:00Z', new Date(AT), 'en-GB'), 'Tue 3 Nov');
  assert.equal(n(m.onceWords('2026-11-06T15:00:00Z', 'en-US')), 'once on Fri, Nov 6 at 3:00 PM');
});
