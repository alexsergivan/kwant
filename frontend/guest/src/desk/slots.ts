// pa13b Booking Desk: the box's times as the guest sees them (pa13b-design.md §2 "Times").
// A slot's `start` is the box's own wall-clock time ("2026-12-02T10:00", no zone). It is read as if it were UTC and
// formatted with timeZone UTC, so the page shows exactly the studio's time and never converts it to the phone's zone.
// Nothing here works out a new number for the guest except how many times a day has open (the design's "{n} open").
import type { Slot } from './view';

const START = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/;
/** The widest zone offset east of UTC is +14 and west is -12: a wall-clock time 12 h older than now is past everywhere. */
const PAST_EVERYWHERE_MS = 12 * 3600_000;

/** The wall-clock start as a UTC instant (only for formatting and ordering), or undefined when it isn't a time. */
export function wall(start: string | undefined): number | undefined {
  const m = start ? START.exec(start) : null;
  if (!m) return undefined;
  const [y, mo, d, hh, mm] = m.slice(1).map(Number);
  const t = Date.UTC(y, mo - 1, d, hh, mm);
  const back = new Date(t);
  // 2026-02-31 or 25:00 is not a time the box would send: drop it rather than show another day.
  if (back.getUTCMonth() !== mo - 1 || back.getUTCDate() !== d || hh > 23 || mm > 59) return undefined;
  return t;
}

type Parts = { weekday: string; month: string; day: string; time: string };

function parts(t: number, locale?: string): Parts {
  const f = new Intl.DateTimeFormat(locale, { weekday: 'short', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit', timeZone: 'UTC' });
  const p: Record<string, string> = {};
  for (const x of f.formatToParts(new Date(t))) p[x.type] = (p[x.type] ?? '') + x.value;
  // 12-hour clocks drop ":00" ("10 AM", "10:30 AM"); 24-hour ones keep it ("10:00", "14:30").
  const ampm = p.dayPeriod ? ` ${p.dayPeriod}` : '';
  const min = p.dayPeriod && p.minute === '00' ? '' : `:${p.minute}`;
  return { weekday: p.weekday ?? '', month: p.month ?? '', day: p.day ?? '', time: `${p.hour}${min}${ampm}` };
}

/** "10 AM" (the time button). The slot's label when `start` isn't a time. */
export function timeLabel(s: Pick<Slot, 'start' | 'label' | 'id'>, locale?: string): string {
  const t = wall(s.start);
  return t === undefined ? fallback(s) : parts(t, locale).time;
}

/** "Wed Dec 2, 10 AM": the strip, the notes and the ask button. The slot's label when `start` isn't a time. */
export function whenLabel(s: Pick<Slot, 'start' | 'label' | 'id'>, locale?: string): string {
  const t = wall(s.start);
  if (t === undefined) return fallback(s);
  const p = parts(t, locale);
  return `${p.weekday} ${p.month} ${p.day}, ${p.time}`;
}

/** "Thu 10 AM": an offer chip in Bell's reply (copy offer_chip "{day} {time}"). */
export function chipLabel(s: Pick<Slot, 'start' | 'label' | 'id'>, locale?: string): string {
  const t = wall(s.start);
  if (t === undefined) return fallback(s);
  const p = parts(t, locale);
  return `${p.weekday} ${p.time}`;
}

const fallback = (s: Pick<Slot, 'label' | 'id'>): string => (s.label ?? '').trim() || s.id;

/** "1 hour", "30 min", "1 h 30 min": the box's own `minutes`, as words. */
export function lengthLabel(minutes: number): string {
  if (minutes % 60 === 0) return `${minutes / 60} hour${minutes === 60 ? '' : 's'}`;
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

export interface Day {
  key: string;
  /** "Wed" */
  weekday: string;
  /** "2" */
  date: string;
  /** "Wed Dec 2" */
  long: string;
  /** Every slot of the day still to come, open or not, in time order. */
  slots: Slot[];
  open: Slot[];
}

/** Not past anywhere on earth: the box filters past times itself; this only drops ones that a stale view still holds. */
export const notPast = (s: Slot, now: number): boolean => {
  const t = wall(s.start);
  return t !== undefined && t + PAST_EVERYWHERE_MS > now;
};

/** The days the picker shows, in order, each with its slots. Times that aren't times, or are past, are dropped. */
export function groupDays(slots: Slot[], now: number, locale?: string): Day[] {
  const byDay = new Map<string, Day>();
  const list = slots.filter((s) => notPast(s, now)).sort((a, b) => wall(a.start)! - wall(b.start)! || (a.id < b.id ? -1 : 1));
  for (const s of list) {
    const key = s.start.slice(0, 10);
    let d = byDay.get(key);
    if (!d) {
      const p = parts(wall(s.start)!, locale);
      d = { key, weekday: p.weekday, date: p.day, long: `${p.weekday} ${p.month} ${p.day}`, slots: [], open: [] };
      byDay.set(key, d);
    }
    d.slots.push(s);
    if (s.open) d.open.push(s);
  }
  return [...byDay.values()];
}

/** Open and still to come: what the starter, the offer chips and the picker may offer. */
export const openSlots = (slots: Slot[], now: number): Slot[] => slots.filter((s) => s.open && notPast(s, now));

/** Each day's length line uses the day's slot length when they all agree; otherwise the line leaves it out. */
export function dayMinutes(d: Day): number | undefined {
  const m = d.open[0]?.minutes ?? d.slots[0]?.minutes;
  return m !== undefined && d.slots.every((s) => s.minutes === m) && m > 0 ? m : undefined;
}
