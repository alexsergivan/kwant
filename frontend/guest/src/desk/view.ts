// The front desk's guest view (pa13/impl.md "Pocket API v1: front desk", which wins over design.md), read from one
// Pocket API v1 state: pocket and me from pa1, the kit's view (title, place, host_name, bot, me, starters, messages,
// waiting_for_host, bot_typing, limits) from desk.rs. Values are copied or left out, never worked out; anything with
// the wrong shape is dropped, so junk from the box can't draw a wrong screen. Only this guest's own conversation is
// ever in the view: the box sends nothing else, and nothing else is read here.

export type From = 'guest' | 'bot' | 'host' | 'note';
export type NoteKind = 'handoff' | 'daily_cap' | 'host_away' | BookingNote;
/** pa13b: the booking changes the thread says in the page's own words. */
export type BookingNote = 'booking_requested' | 'booking_confirmed' | 'booked_by_host' | 'booking_declined' | 'booking_cancelled';

export interface Msg {
  id: string;
  from: From;
  text: string;
  at: string;
  note?: NoteKind;
  /** A booking note's time (design "For Vega": `slot` and `start` on the note). */
  slot?: string;
  start?: string;
  /** pa13b K9: open slot ids Bell offers as chips in its reply (at most 3). */
  offers?: string[];
  client_id?: string;
}

/** pa13b: one time the box offers. `start` is the box's wall-clock time, "YYYY-MM-DDTHH:MM". */
export interface Slot {
  id: string;
  start: string;
  minutes: number;
  label?: string;
  open: boolean;
}

/** pa13b: this guest's own booking: never anyone else's. Declined or cancelled reads as none. */
export interface Booking {
  slot: string;
  start?: string;
  label?: string;
  minutes?: number;
  state: 'requested' | 'confirmed';
  by?: string;
}

export interface Limits {
  per_hour?: number;
  left_this_hour?: number;
  /** When sending opens again (only while left_this_hour is 0). */
  next_at?: string;
  max_chars: number;
  /** pa13b: a booking request's note. */
  note_max_chars: number;
}

export interface DeskMe {
  id: string;
  name: string;
  /** YYYY-MM-DD: this guest's messages go then (last message + 7 days). None before the first message. */
  delete_on?: string;
}

export interface DeskView {
  title: string;
  place: string;
  /** "" when the host left it out: the page then says "the host" and draws the kit bell icon. */
  host_name: string;
  bot: string;
  keep_days?: number;
  ended: boolean;
  /** pocket.expires_at: when the whole desk goes (End + 7 days). */
  expires_at?: string;
  /** pocket.needs_pin: false means name only, on join and rejoin. */
  needs_pin: boolean;
  me?: DeskMe;
  starters: string[];
  messages: Msg[];
  waiting: boolean;
  typing: boolean;
  limits: Limits;
  /** pa13b: `mode: "booking"`, or a view that carries slots. */
  booking: boolean;
  /** pa13b spec `what` ("your cut") for the join line, when the box sends it. */
  what?: string;
  slots: Slot[];
  my_booking: Booking | null;
}

export type Screen = 'join' | 'rejoin' | 'chat' | 'ended';

/** impl.md: a message is at most 1,000 characters. Used only until the box says its own `limits.max_chars`. */
export const MAX_CHARS = 1000;
/** pa13b-design.md: a request's note is at most 200 characters, until the box says `limits.note_max_chars`. */
export const NOTE_MAX_CHARS = 200;
const OFFERS = 3;
const STARTERS = 3;

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);
const FROM: readonly string[] = ['guest', 'bot', 'host', 'note'];
const NOTES: readonly string[] = ['handoff', 'daily_cap', 'host_away', 'booking_requested', 'booking_confirmed', 'booked_by_host', 'booking_declined', 'booking_cancelled'];
const START = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/;

function slotFrom(x: unknown): Slot | undefined {
  const s = obj(x);
  const id = str(s?.id);
  const start = str(s?.start);
  const minutes = num(s?.minutes);
  if (!s || !id || !start || !START.test(start)) return undefined;
  const label = str(s.label)?.trim();
  return { id, start: start.slice(0, 16), minutes: minutes !== undefined && minutes > 0 ? minutes : 0, label: label || undefined, open: s.open === true };
}

/** my_booking, or null. Vega's draft also sends `state: "declined"`, which the strip treats as no booking. */
export function bookingFrom(x: unknown): Booking | null {
  const b = obj(x);
  const slot = str(b?.slot);
  const state = str(b?.state);
  if (!b || !slot || (state !== 'requested' && state !== 'confirmed')) return null;
  const start = str(b.start);
  const out: Booking = { slot, state };
  if (start && START.test(start)) out.start = start.slice(0, 16);
  const label = str(b.label)?.trim();
  if (label) out.label = label;
  const m = num(b.minutes);
  if (m !== undefined && m > 0) out.minutes = m;
  const by = str(b.by);
  if (by) out.by = by;
  return out;
}

export const slotsFrom = (x: unknown): Slot[] => (Array.isArray(x) ? x : []).map(slotFrom).filter((s): s is Slot => !!s);

function msgFrom(x: unknown): Msg | undefined {
  const m = obj(x);
  const id = str(m?.id);
  const from = str(m?.from);
  const at = str(m?.at);
  if (!m || !id || !from || !FROM.includes(from) || !at) return undefined;
  const text = str(m.text) ?? '';
  const out: Msg = { id, from: from as From, text, at };
  if (from === 'note') {
    // A note's words are the page's own, so a note this page has no words for is not drawn.
    const n = str(m.note);
    if (!n || !NOTES.includes(n)) return undefined;
    out.note = n as NoteKind;
    if (n.startsWith('book')) {
      const slot = str(m.slot);
      const start = str(m.start);
      if (slot) out.slot = slot;
      if (start && START.test(start)) out.start = start.slice(0, 16);
    }
  } else if (!text.trim()) return undefined;
  if (from === 'bot' && Array.isArray(m.offers)) {
    const ids = m.offers.filter((o): o is string => typeof o === 'string' && !!o).slice(0, OFFERS);
    if (ids.length) out.offers = ids;
  }
  const c = str(m.client_id);
  if (c && from === 'guest') out.client_id = c;
  return out;
}

/** One v1 state (state, join, rejoin, action, info, a ws frame or the boot JSON) as the desk's view. */
export function deskFrom(json: unknown): { view: DeskView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  if (!j || !pocket) return undefined;
  const kv = obj(j.view) ?? {};
  const bot = obj(kv.bot);
  const pme = obj(j.me);
  const kme = obj(kv.me);
  const id = str(kme?.id) ?? str(pme?.id);
  const name = str(kme?.name) ?? str(pme?.name) ?? '';
  const del = str(kme?.delete_on);
  const lim = obj(kv.limits);
  const max = num(lim?.max_chars);
  const noteMax = num(lim?.note_max_chars);
  const slots = slotsFrom(kv.slots);
  const what = str(kv.what)?.trim();
  const view: DeskView = {
    title: str(kv.title) ?? str(pocket.title) ?? '',
    place: str(kv.place) ?? '',
    host_name: (str(kv.host_name) ?? '').trim(),
    bot: (str(bot?.name) ?? '').trim(),
    keep_days: num(kv.keep_days),
    ended: str(pocket.state) === 'ended',
    expires_at: str(pocket.expires_at),
    needs_pin: pocket.needs_pin !== false,
    starters: (Array.isArray(kv.starters) ? kv.starters : [])
      .map((s) => (typeof s === 'string' ? s.trim() : ''))
      .filter(Boolean)
      .slice(0, STARTERS),
    messages: (Array.isArray(kv.messages) ? kv.messages : []).map(msgFrom).filter((m): m is Msg => !!m),
    waiting: kv.waiting_for_host === true,
    typing: kv.bot_typing === true,
    limits: {
      per_hour: num(lim?.per_hour),
      left_this_hour: num(lim?.left_this_hour),
      next_at: str(lim?.next_at),
      max_chars: max !== undefined && max > 0 ? max : MAX_CHARS,
      note_max_chars: noteMax !== undefined && noteMax > 0 ? noteMax : NOTE_MAX_CHARS,
    },
    booking: str(kv.mode) === 'booking' || Array.isArray(kv.slots),
    what: what || undefined,
    slots,
    my_booking: bookingFrom(kv.my_booking),
  };
  // A seat needs pa1's `me` (the cookie's player); the kit's own `me` only adds the delete date.
  if (pme && id) view.me = { id, name, delete_on: del && /^\d{4}-\d{2}-\d{2}/.test(del) ? del.slice(0, 10) : undefined };
  return { view, version: num(j.version) };
}

/** Which screen the view draws. `rejoin`: the guest tapped "I've joined before", or had a seat on this page before. */
export function screenOf(v: DeskView, o: { rejoin: boolean }): Screen {
  if (v.ended) return 'ended';
  if (v.me) return 'chat';
  return o.rejoin ? 'rejoin' : 'join';
}
