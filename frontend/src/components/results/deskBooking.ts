// Pocket apps (pa13b): the Booking Desk's pure parts, with no React: the slot validator, the studio's wall-clock time
// formatter, the week maths, QuickFill's slot maker and the optimistic view of the host's answers.
// Field names are the design's "For Vega" (pa13/pa13b-design.md); Vega's pa13b-impl.md wins once it exists.
// Times are the studio's wall-clock times ("2026-12-02T10:00"), read as UTC and never converted to another zone.

import type { DeskBooking, DeskSlot, DeskSlotPerson } from '../../smartResults';
import { isObj } from '../../smartResults';

export const MAX_SLOTS = 200;
/** The stepper's most (design); the server takes up to 60 (pa13b-impl.md), so the validator does too. */
export const MAX_SEATS = 20;
const MAX_SEATS_WIRE = 60;
export const MAX_REQUESTS_SHOWN = 3;
const SLOT = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/;
const ID = /^[A-Za-z0-9_-]{1,64}$/;

// ---------------------------------------------------------------------------------------------------
// The clock: the host's Mac is at the studio, so its local wall clock is the studio's. Tests and shots set it.

let clock: () => Date = () => new Date();
export const bookingClock = {
  now: () => clock(),
  /** Tests only: a fixed "now". Null: the real clock. */
  set(d: Date | null) {
    clock = d ? () => new Date(d.getTime()) : () => new Date();
  },
};
const pad = (n: number) => String(n).padStart(2, '0');
/** "2026-12-01T11:40", the local wall clock. */
export const nowKey = (d = bookingClock.now()) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
export const todayKey = () => nowKey().slice(0, 10);

// ---------------------------------------------------------------------------------------------------
// Formatting (design §2: Intl on `start` read as UTC, then drop ":00")

export const isSlotStart = (v: unknown): v is string => {
  if (typeof v !== 'string') return false;
  const m = SLOT.exec(v);
  if (!m) return false;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]));
  // Rejects "2026-02-30T10:00" and "25:00": the parts must survive the round trip.
  return d.getUTCMonth() === +m[2] - 1 && d.getUTCDate() === +m[3] && d.getUTCHours() === +m[4] && d.getUTCMinutes() === +m[5];
};
const asDate = (start: string) => {
  const m = SLOT.exec(start.length === 10 ? `${start}T00:00` : start);
  return m ? new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5])) : new Date(NaN);
};
const fmt = (start: string, o: Intl.DateTimeFormatOptions) => {
  const d = asDate(start);
  return Number.isNaN(d.getTime()) ? start : new Intl.DateTimeFormat('en-US', { ...o, timeZone: 'UTC' }).format(d);
};
/** "10 AM", "10:30 AM". */
export const slotTime = (start: string) => fmt(start, { hour: 'numeric', minute: '2-digit' }).replace(':00', '');
/** "Wed Dec 2". */
export const slotDay = (start: string) => fmt(start, { weekday: 'short', month: 'short', day: 'numeric' }).replace(',', '');
/** "Wed Dec 2, 10 AM". */
export const slotWhen = (start: string) => `${slotDay(start)}, ${slotTime(start)}`;
/** "Wed" and "Dec 2" for a week column head. */
export const weekday = (day: string) => fmt(day, { weekday: 'short' });
export const monthDay = (day: string) => fmt(day, { month: 'short', day: 'numeric' });
/** "1 hour", "30 min", "1 h 30 min". */
export const lengthLabel = (min: number) => {
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (!h) return `${m} min`;
  if (!m) return h === 1 ? '1 hour' : `${h} hours`;
  return `${h} h ${m} min`;
};

// ---------------------------------------------------------------------------------------------------
// Days and weeks ("2026-12-01")

export const addDays = (day: string, n: number) => {
  const d = asDate(day);
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
};
/** 0 Monday … 6 Sunday. */
export const dow = (day: string) => (asDate(day).getUTCDay() + 6) % 7;
export const weekOf = (day: string) => addDays(day, -dow(day));
export const weekDays = (monday: string) => Array.from({ length: 7 }, (_, i) => addDays(monday, i));
export const DOW_LABEL = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
/** "Dec 1 – 5" or "Nov 30 – Dec 4". */
export function weekRange(days: string[]): string {
  if (days.length === 0) return '';
  const a = days[0];
  const b = days[days.length - 1];
  if (a === b) return monthDay(a);
  return a.slice(5, 7) === b.slice(5, 7) ? `${monthDay(a)} – ${Number(b.slice(8))}` : `${monthDay(a)} – ${monthDay(b)}`;
}
const toMin = (hhmm: string) => Number(hhmm.slice(0, 2)) * 60 + Number(hhmm.slice(3, 5));
const fromMin = (m: number) => `${pad(Math.floor(m / 60))}:${pad(m % 60)}`;
export const slotEnd = (s: Pick<DeskSlot, 'start' | 'minutes'>) => toMin(s.start.slice(11)) + s.minutes;

// ---------------------------------------------------------------------------------------------------
// Validation: a junk slot rejects the block, as a junk guest does in pa13a; missing counts are worked out.

function validPerson(v: unknown, booked: boolean): DeskSlotPerson | undefined {
  if (!isObj(v)) return undefined;
  // A booking whose guest's data went stays as {guest: null, name: null} (Vega's retention); it keeps the seat.
  const gone = booked && (v.guest === null || v.guest === undefined) && (v.name === null || v.name === undefined);
  if (!gone && (typeof v.guest !== 'string' || !ID.test(v.guest))) return undefined;
  const name = v.name === null || v.name === undefined ? null : typeof v.name === 'string' && v.name.length <= 40 && v.name.trim() ? v.name : undefined;
  const note = v.note === null || v.note === undefined || v.note === '' ? null : typeof v.note === 'string' && v.note.length <= 200 ? v.note : undefined;
  const at = v.at === null || v.at === undefined ? null : typeof v.at === 'string' && v.at.length <= 60 ? v.at : undefined;
  const by = v.by === null || v.by === undefined ? (booked ? 'guest' : null) : v.by === 'guest' || v.by === 'host' ? v.by : undefined;
  if (name === undefined || note === undefined || at === undefined || by === undefined) return undefined;
  // A deleted chat keeps the booking with no name (retention); a request needs its guest.
  if (name === null && !booked) return undefined;
  return { guest: gone ? '' : (v.guest as string), name, note, at, by };
}

export function validSlot(v: unknown): DeskSlot | undefined {
  if (!isObj(v) || !isSlotStart(v.start)) return undefined;
  const id = v.id === null || v.id === undefined ? null : typeof v.id === 'string' && ID.test(v.id) ? v.id : undefined;
  // A time without a length is an hour (the design's default).
  const minutes = v.minutes === undefined || v.minutes === null ? 60 : v.minutes;
  if (typeof minutes !== 'number' || !Number.isInteger(minutes) || minutes < 5 || minutes > 1440) return undefined;
  const seats = v.seats === undefined || v.seats === null ? 1 : typeof v.seats === 'number' && Number.isInteger(v.seats) && v.seats >= 1 && v.seats <= MAX_SEATS_WIRE ? v.seats : undefined;
  const label = v.label === undefined || v.label === null || v.label === '' ? null : typeof v.label === 'string' && v.label.length <= 40 ? v.label : undefined;
  const req = v.requested === undefined || v.requested === null ? [] : Array.isArray(v.requested) ? v.requested.map((p) => validPerson(p, false)) : undefined;
  const conf = v.confirmed === undefined || v.confirmed === null ? [] : Array.isArray(v.confirmed) ? v.confirmed.map((p) => validPerson(p, true)) : undefined;
  if (id === undefined || seats === undefined || label === undefined || !req || !conf || req.includes(undefined) || conf.includes(undefined)) return undefined;
  if (req.length + conf.length > seats) return undefined;
  return { id, start: v.start, minutes, label, seats, requested: req as DeskSlotPerson[], confirmed: conf as DeskSlotPerson[] };
}

const count = (v: unknown) => (v === undefined || v === null ? null : typeof v === 'number' && Number.isInteger(v) && v >= 0 && v <= 1_000_000 ? v : undefined);

/** The desk's booking part from the host view (`slots` + `summary.*`). Junk → undefined; missing counts are worked out. */
export function validBooking(v: Record<string, unknown>): DeskBooking | undefined {
  const raw = v.slots === undefined || v.slots === null ? [] : v.slots;
  if (!Array.isArray(raw) || raw.length > MAX_SLOTS) return undefined;
  const slots = raw.map(validSlot);
  if (slots.includes(undefined)) return undefined;
  const list = (slots as DeskSlot[]).slice().sort((a, b) => (a.start < b.start ? -1 : a.start > b.start ? 1 : 0));
  const ids = list.map((s) => s.id).filter(Boolean);
  if (new Set(ids).size !== ids.length) return undefined;
  const c = { open_left: count(v.open_left), requests: count(v.requests), booked_upcoming: count(v.booked_upcoming), booked_total: count(v.booked_total) };
  if (Object.values(c).includes(undefined)) return undefined;
  const t = tally(list);
  return {
    slots: list,
    open_left: c.open_left ?? t.open_left,
    requests: c.requests ?? t.requests,
    booked_upcoming: c.booked_upcoming ?? t.booked_upcoming,
    booked_total: c.booked_total ?? t.booked_total,
  };
}

/** Counts worked out from the slots: open seats in future times, requests, booked seats still to come and in all. */
export function tally(slots: DeskSlot[], now = nowKey()) {
  let open = 0;
  let req = 0;
  let up = 0;
  let all = 0;
  for (const s of slots) {
    const future = s.start > now;
    req += s.requested.length;
    all += s.confirmed.length;
    if (future) {
      up += s.confirmed.length;
      open += Math.max(0, s.seats - s.requested.length - s.confirmed.length);
    }
  }
  return { open_left: open, requests: req, booked_upcoming: up, booked_total: all };
}

/** "Live · 2 requests", "Live · 3 for you" (requests and hand-offs), "Open · no requests". */
export function bookingChip(requests: number, handoffs: number): string {
  if (requests + handoffs === 0) return 'Open · no requests';
  if (handoffs === 0) return `Live · ${requests} ${requests === 1 ? 'request' : 'requests'}`;
  return `Live · ${requests + handoffs} for you`;
}

// ---------------------------------------------------------------------------------------------------
// The requests, oldest first: what the Allow cards above the week show.

export interface BookingRequest {
  slot: DeskSlot;
  person: DeskSlotPerson;
}
export function requestsOf(slots: DeskSlot[]): BookingRequest[] {
  const out: BookingRequest[] = [];
  for (const slot of slots) for (const person of slot.requested) out.push({ slot, person });
  const t = (r: BookingRequest) => (r.person.at ? Date.parse(r.person.at) : NaN);
  return out.sort((a, b) => {
    const x = t(a);
    const y = t(b);
    if (Number.isNaN(x) || Number.isNaN(y)) return a.slot.start < b.slot.start ? -1 : 1;
    return x - y;
  });
}
/** A guest's own request or booking (one per guest), for "Book a time" moving it. */
export function holdingOf(slots: DeskSlot[], guest: string): { slot: DeskSlot; state: 'requested' | 'confirmed' } | null {
  for (const slot of slots) {
    if (slot.requested.some((p) => p.guest === guest)) return { slot, state: 'requested' };
    if (slot.confirmed.some((p) => p.guest === guest) && slot.start > nowKey()) return { slot, state: 'confirmed' };
  }
  return null;
}
export const inUse = (s: DeskSlot) => s.requested.length + s.confirmed.length > 0;
export const isOpen = (s: DeskSlot, now = nowKey()) => s.start > now && s.seats > s.requested.length + s.confirmed.length;

// ---------------------------------------------------------------------------------------------------
// QuickFill: days, from, to, length and seats make the shown week's times on the client; set_slots saves them.

export interface FillRule {
  /** 0 Monday … 6 Sunday. */
  days: number[];
  from: string; // "09:00"
  to: string; // "17:00"
  minutes: number;
  seats: number;
}
export const FILL_DEFAULT: FillRule = { days: [0, 1, 2, 3, 4], from: '09:00', to: '17:00', minutes: 60, seats: 1 };

/** The times a rule makes in one week: no past times, none past `to`. */
export function fillWeek(rule: FillRule, monday: string, now = nowKey()): DeskSlot[] {
  const out: DeskSlot[] = [];
  const a = toMin(rule.from);
  const b = toMin(rule.to);
  if (!(rule.minutes >= 5) || b <= a) return out;
  for (const i of [...new Set(rule.days)].sort()) {
    const day = addDays(monday, i);
    for (let m = a; m + rule.minutes <= b; m += rule.minutes) {
      const start = `${day}T${fromMin(m)}`;
      if (start <= now) continue;
      out.push({ id: null, start, minutes: rule.minutes, label: null, seats: Math.min(MAX_SEATS, Math.max(1, rule.seats)), requested: [], confirmed: [] });
    }
  }
  return out;
}

const overlaps = (x: DeskSlot, y: DeskSlot) => x.start.slice(0, 10) === y.start.slice(0, 10) && toMin(x.start.slice(11)) < slotEnd(y) && toMin(y.start.slice(11)) < slotEnd(x);

/**
 * "Fill these days": the filled days of the shown week get the rule's times. Past times and times in use stay as
 * they are; a new time that would overlap one of them is left out. Other days and weeks are untouched. At most 200.
 */
export function applyFill(slots: DeskSlot[], rule: FillRule, monday: string, now = nowKey()): { slots: DeskSlot[]; made: number; capped: boolean } {
  const days = new Set(rule.days.map((i) => addDays(monday, i)));
  const keep = slots.filter((s) => !days.has(s.start.slice(0, 10)) || s.start <= now || inUse(s));
  const fixed = keep.filter((s) => days.has(s.start.slice(0, 10)));
  const fresh = fillWeek(rule, monday, now).filter((n) => !fixed.some((f) => overlaps(n, f)));
  const room = Math.max(0, MAX_SLOTS - keep.length);
  const made = fresh.slice(0, room);
  return { slots: sortSlots([...keep, ...made]), made: made.length, capped: fresh.length > room };
}

/** "+ Add" on a day: the hour after that day's last time (or 9 AM), same length; null when the day is full or past. */
export function nextSlotFor(slots: DeskSlot[], day: string, minutes = 60, now = nowKey()): DeskSlot | null {
  if (slots.length >= MAX_SLOTS) return null;
  const same = slots.filter((s) => s.start.slice(0, 10) === day);
  const len = same.length ? same[same.length - 1].minutes : minutes;
  let m = same.length ? Math.max(...same.map(slotEnd)) : 9 * 60;
  // The first future time on today.
  if (day === now.slice(0, 10)) m = Math.max(m, Math.ceil((toMin(now.slice(11)) + 1) / 30) * 30);
  if (m + len > 24 * 60) return null;
  const start = `${day}T${fromMin(m)}`;
  if (start <= now) return null;
  return { id: null, start, minutes: len, label: null, seats: same[same.length - 1]?.seats ?? 1, requested: [], confirmed: [] };
}

export const sortSlots = (s: DeskSlot[]) => s.slice().sort((a, b) => (a.start < b.start ? -1 : a.start > b.start ? 1 : 0));

/** What set_slots sends: ids for the times that exist, none for new ones. */
export const slotsWire = (s: DeskSlot[]) =>
  sortSlots(s).map((x) => ({ ...(x.id ? { id: x.id } : {}), start: x.start, minutes: x.minutes, seats: x.seats, ...(x.label ? { label: x.label } : {}) }));

/** The header line: "32 times · 1 hour each · 1 guest per time". */
export function timesLine(slots: DeskSlot[]): string {
  const n = slots.length;
  if (!n) return 'No times yet';
  const lens = [...new Set(slots.map((s) => s.minutes))];
  const seats = [...new Set(slots.map((s) => s.seats))];
  const parts = [`${n} ${n === 1 ? 'time' : 'times'}`];
  parts.push(lens.length === 1 ? `${lengthLabel(lens[0])} each` : 'mixed lengths');
  parts.push(seats.length === 1 ? `${seats[0]} ${seats[0] === 1 ? 'guest' : 'guests'} per time` : 'mixed seats');
  return parts.join(' · ');
}

// ---------------------------------------------------------------------------------------------------
// The host's answers, laid over the slots so the card and the Inbox agree at once (pa13a's optimistic rule).

export type BookingAnswerLike = { phase: 'holding' | 'sending' | 'sent' | 'failed'; kind: string; slot?: string | null };
export const bookingKey = (code: string, guest: string, slot: string) => `${code}:${guest}@${slot}`;

/** A Yes moves the request to booked; a Sorry frees the seat. A failed answer changes nothing. */
export function withBookingAnswers(code: string, b: DeskBooking, get: (k: string) => BookingAnswerLike | undefined): DeskBooking {
  let moved = 0;
  let freed = 0;
  const slots = b.slots.map((s) => {
    if (s.requested.length === 0) return s;
    const requested: DeskSlotPerson[] = [];
    const confirmed = s.confirmed.slice();
    for (const p of s.requested) {
      const a = get(bookingKey(code, p.guest, s.id ?? s.start));
      if (a && a.phase !== 'failed' && a.kind === 'confirm') {
        confirmed.push({ ...p, by: 'guest' });
        moved++;
      } else if (a && a.phase !== 'failed' && a.kind === 'decline') freed++;
      else requested.push(p);
    }
    return requested.length === s.requested.length ? s : { ...s, requested, confirmed };
  });
  if (!moved && !freed) return b;
  return {
    slots,
    requests: Math.max(0, b.requests - moved - freed),
    booked_upcoming: b.booked_upcoming + moved,
    booked_total: b.booked_total + moved,
    open_left: b.open_left + freed,
  };
}
