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
  export { clock as inboxClock } from './src/components/InboxNew';
  export { hm as awayHm, scheduleShort } from './src/doneAway';
  export { checkedLabel } from './src/devMode';
  export { dayLabel as worktreeDay } from './src/dev/worktrees';
`);

// 19d: timeText writes U+00A0 before AM/PM on every runtime, so strings compare exactly as they are.
const n = (s) => s;
const AT = '2026-11-16T20:15:00Z'; // a Monday
const MORNING = '2026-11-16T08:30:00Z';

const TABLE = [
  ['timeText', () => [m.timeText(AT, { locale: 'en-US' }), m.timeText(AT, { locale: 'en-GB' })], ['8:15\u00a0PM', '20:15']],
  ['timeText morning', () => [m.timeText(MORNING, { locale: 'en-US' }), m.timeText(MORNING, { locale: 'en-GB' })], ['8:30\u00a0AM', '08:30']],
  ['timeText with a zone', () => [m.timeText(AT, { locale: 'en-US', timeZone: 'Asia/Kolkata' }), m.timeText(AT, { locale: 'en-GB', timeZone: 'Asia/Kolkata' })], ['1:45\u00a0AM', '01:45']],
  ['dateText', () => [m.dateText(AT, { locale: 'en-US' }), m.dateText(AT, { locale: 'en-GB' })], ['Nov 16', '16 Nov']],
  ['dateText weekday + year', () => [m.dateText(AT, { locale: 'en-US', weekday: true, year: true }), m.dateText(AT, { locale: 'en-GB', weekday: true, year: true })], ['Mon, Nov 16, 2026', 'Mon, 16 Nov 2026']],
  ['weekdayClock (a Next line)', () => [m.weekdayClock(MORNING, { locale: 'en-US' }), m.weekdayClock(MORNING, { locale: 'en-GB' })], ['Mon, Nov 16, 8:30\u00a0AM', 'Mon 16 Nov, 08:30']],
  ['dateClock', () => [m.dateClock(AT, { locale: 'en-US' }), m.dateClock(AT, { locale: 'en-GB' })], ['Nov 16, 8:15\u00a0PM', '16 Nov, 20:15']],
  ['wallClock 08:30', () => [m.wallClock('08:30', 'en-US'), m.wallClock('08:30', 'en-GB')], ['8:30\u00a0AM', '08:30']],
  ['wallClock 20:05', () => [m.wallClock('20:05', 'en-US'), m.wallClock('20:05', 'en-GB')], ['8:05\u00a0PM', '20:05']],
  ['secondsText (step logs)', () => [m.secondsText(AT, { locale: 'en-US' }), m.secondsText(AT, { locale: 'en-GB' })], ['8:15:00\u00a0PM', '20:15:00']],
  ['wallClock midnight', () => [m.wallClock('00:00', 'en-US'), m.wallClock('00:00', 'en-GB')], ['12:00\u00a0AM', '00:00']],
];

for (const [name, run, want] of TABLE) {
  test(`clock: ${name} in en-US and en-GB`, () => assert.deepEqual(run().map(n), want));
}

// 18d (clock says day): a time inside an "at" sentence. Today: "at 8:15 PM"; another day: "on Nov 10 at 8:15 PM";
// another year: "on Nov 10, 2025 at 8:15 PM". No "yesterday". Read at a fixed now on the Monday of AT.
test('clock: atClock says the day when it is not today, in en-US and en-GB', () => {
  const NOW = Date.parse('2026-11-16T22:00:00Z');
  const at = (t, l) => n(m.atClock(t, NOW, l));
  assert.deepEqual([at(AT, 'en-US'), at(AT, 'en-GB')], ['at 8:15\u00a0PM', 'at 20:15']);
  assert.deepEqual([at(MORNING, 'en-US'), at(MORNING, 'en-GB')], ['at 8:30\u00a0AM', 'at 08:30']);
  const OTHER = '2026-11-10T09:40:00Z';
  assert.deepEqual([at(OTHER, 'en-US'), at(OTHER, 'en-GB')], ['on Nov 10 at 9:40\u00a0AM', 'on 10 Nov at 09:40']);
  const YESTERDAY = '2026-11-15T23:59:00Z';
  assert.deepEqual([at(YESTERDAY, 'en-US'), at(YESTERDAY, 'en-GB')], ['on Nov 15 at 11:59\u00a0PM', 'on 15 Nov at 23:59']);
  const LAST_YEAR = '2025-11-10T09:40:00Z';
  assert.deepEqual([at(LAST_YEAR, 'en-US'), at(LAST_YEAR, 'en-GB')], ['on Nov 10, 2025 at 9:40\u00a0AM', 'on 10 Nov 2025 at 09:40']);
  for (const bad of [null, undefined, '', 'not a time']) assert.equal(m.atClock(bad, NOW, 'en-US'), null);
  // The same day rule as localClock: the two agree on which times are today.
  for (const t of [AT, MORNING, OTHER, YESTERDAY, LAST_YEAR]) assert.equal(m.atClock(t, NOW, 'en-US').startsWith('at '), m.localClock(t, NOW, 'en-US') === m.timeText(t, { locale: 'en-US' }));
});

// 19d (time never splits): AM/PM is held to its time by a no-break space, so a narrow row can't wrap "10:51 / PM".
test('clock: en-US puts U+00A0 before AM/PM (never U+202F or a plain space); en-GB has none', () => {
  for (const t of [m.timeText(AT, { locale: 'en-US' }), m.timeText(MORNING, { locale: 'en-US' }), m.secondsText(AT, { locale: 'en-US' }), m.wallClock('20:05', 'en-US')]) {
    assert.match(t, /\d\u00a0[AP]M$/, JSON.stringify(t));
    assert.doesNotMatch(t, /[\u202f ][AP]M/, JSON.stringify(t));
  }
  assert.equal(m.timeText(AT, { locale: 'en-US' }), '8:15\u00a0PM');
  assert.equal(m.localClock('2026-11-11T22:51:00Z', Date.parse('2026-11-12T00:50:00Z'), 'en-US'), 'Nov 11, 10:51\u00a0PM', 'only the AM/PM space is held; the date keeps its plain spaces');
  for (const t of [m.timeText(AT, { locale: 'en-GB' }), m.secondsText(AT, { locale: 'en-GB' }), m.dateClock(AT, { locale: 'en-GB' })]) assert.doesNotMatch(t, /\u00a0|\u202f/, JSON.stringify(t));
});

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
  const want = new Date(AT).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' }).replace(/[\u202f ](AM|PM)/, '\u00a0$1');
  assert.equal(m.timeText(AT), want);
  assert.equal(m.wallClock('20:15'), want);
});

test('routines: the schedule and its Next line use the same clock', () => {
  const s = { type: 'weekly', weekdays: ['monday'], time: '08:30', tz: 'UTC' };
  const line = n(m.describeSchedule(s));
  const want = n(m.wallClock('08:30'));
  assert.ok(line.includes(`at ${want},`), line);
  assert.ok(n(m.formatWhen(MORNING)).endsWith(want), m.formatWhen(MORNING));
  assert.equal(n(m.formatWhen(MORNING, 'en-US')), 'Mon, Nov 16, 8:30\u00a0AM');
  assert.equal(n(m.describeSchedule({ type: 'daily', time: '17:00', tz: 'UTC' })).includes(n(m.wallClock('17:00'))), true);
  assert.equal(m.formatWhen(null), '—');
});

test('call sites that forced 24 h or en-GB now follow the locale', () => {
  assert.equal(n(m.reviewClock(AT, 'en-US', Date.parse(AT))), '8:15\u00a0PM', 'the Inbox');
  assert.equal(m.reviewClock(AT, 'en-GB', Date.parse(AT)), '20:15');
  assert.equal(n(m.callTime(AT, new Date(AT), 'en-US')), '8:15\u00a0PM');
  assert.equal(n(m.callTime(AT, new Date('2026-11-17T09:00:00Z'), 'en-US')), 'Nov 16, 8:15\u00a0PM');
  assert.equal(m.callTime(AT, new Date('2026-11-17T09:00:00Z'), 'en-GB'), '16 Nov, 20:15');
  assert.equal(m.shortDate(AT, 'en-US'), 'Nov 16, 2026');
  assert.equal(m.shortDate(AT, 'en-GB'), '16 Nov 2026');
  assert.equal(n(m.hm(AT, 'UTC', 'en-US')), '8:15\u00a0PM', 'missionWatch keeps its zone');
  assert.equal(m.hm(AT, 'UTC', 'en-GB'), '20:15');
  assert.equal(m.usageDay('2026-10-20', new Date(AT), 'en-US'), 'Tue, Oct 20', 'DailyUsage stays on the UTC day');
  assert.equal(m.usageDay('2026-10-20', new Date(AT), 'en-GB'), 'Tue 20 Oct');
  assert.equal(m.devDay('2026-11-03T10:00:00Z', new Date(AT), 'en-GB'), 'Tue 3 Nov');
  assert.equal(n(m.onceWords('2026-11-06T15:00:00Z', 'en-US')), 'once on Fri, Nov 6 at 3:00\u00a0PM');
});

test('one clock: hand-built HH:MM spots now read the device clock (Inbox queue, Done-away, Dev desk, worktrees)', () => {
  const want = n(m.timeText(AT));
  assert.equal(n(m.inboxClock(AT, Date.parse(AT))), want);
  assert.equal(n(m.awayHm(AT)), want);
  assert.equal(n(m.checkedLabel(Date.parse(AT))), want);
  assert.equal(n(m.worktreeDay(AT, Date.parse(AT)).time), want);
  assert.equal(n(m.worktreeDay(AT, Date.parse('2026-12-01T10:00:00Z')).day), n(m.dateText(AT)));
  assert.equal(n(m.scheduleShort({ type: 'weekly', time: '20:15', weekdays: ['mon'], tz: 'UTC' })), `every Mon ${n(m.wallClock('20:15'))}`);
  assert.equal(m.inboxClock('nope'), '');
});

test('one clock: the Inbox row and review item carry the date on another day, like the card and chat (Round 2)', () => {
  const LATER = Date.parse('2026-11-19T09:00:00Z');
  assert.equal(n(m.inboxClock(AT, LATER)), n(m.localClock(AT, LATER)), 'the row follows localClock');
  assert.equal(n(m.inboxClock(AT, LATER)), `${n(m.dateText(AT))}, ${want0()}`);
  assert.equal(n(m.reviewClock(AT, 'en-US', LATER)), 'Nov 16, 8:15\u00a0PM', 'in Review since … yesterday or earlier');
  assert.equal(m.reviewClock(AT, 'en-GB', LATER), '16 Nov, 20:15');
  assert.equal(m.reviewClock(null), null);
  assert.equal(m.reviewClock('nope'), null);
});
const want0 = () => n(m.timeText(AT));
