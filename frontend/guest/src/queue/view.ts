// The Queue ticket's guest view (pa11/queue-impl.md "Views", which wins over queue-design.md), read from one Pocket API
// v1 state: `pocket` and `me` from pa1, the kit's guest view from queue.rs. Values are copied or left out, never worked
// out; anything with the wrong shape is dropped. Only this phone's own ticket is ever in it: the box sends no one else's
// ticket or name, and nothing else is read here.

export type QueueState = 'open' | 'paused' | 'closed' | 'ended';
export type TicketState = 'waiting' | 'called' | 'served' | 'skipped' | 'left';

export interface Ticket {
  number: number;
  /** Only while waiting: the waiting tickets numbered before this one, plus 1. */
  place: number | null;
  ahead: number | null;
  state: TicketState;
  taken_at?: string;
  called_at?: string;
  /** Only while called, from the server clock. */
  call_ends_at?: string;
  done_at?: string;
  /** The queue closed while this ticket was waiting or called. */
  closed: boolean;
}

export interface QueueView {
  state: QueueState;
  title: string;
  line: string;
  /** "HH:MM" on the box's clock, or none ("until I close it"). */
  hours_until?: string;
  now_serving: number | null;
  waiting: number;
  /** null until 3 calls today: every minute figure stays off the page until then. */
  avg_minutes: number | null;
  call_minutes: number;
  max_waiting?: number;
  ask_name: boolean;
  /** "" when the host left it out: the page says "the host". */
  host_name: string;
  pause_text: string;
  close_text: string;
  paused_at?: string;
  /** The box's clock when it wrote this view: the countdown's offset. */
  now?: string;
  my_ticket: Ticket | null;
  /** pa1's `me`: this phone has joined (the guest key cookie). */
  joined: boolean;
}

/** What the page draws for the view. `next` is a waiting ticket at place 1 while the queue is open. */
export type Phase = 'take' | 'waiting' | 'next' | 'turn' | 'skipped' | 'served' | 'left' | 'closed' | 'ended';

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);
const whole = (x: unknown): number | null => {
  const n = num(x);
  return n !== undefined && n >= 0 && Number.isInteger(n) ? n : null;
};
const QSTATES: readonly string[] = ['open', 'paused', 'closed', 'ended'];
const TSTATES: readonly string[] = ['waiting', 'called', 'served', 'skipped', 'left'];
const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;

/** One `my_ticket` (or a 409 has_ticket's `ticket`). A ticket without a number or a known state is not drawn. */
export function ticketFrom(x: unknown): Ticket | null {
  const t = obj(x);
  const number = whole(t?.number);
  const state = str(t?.state);
  if (!t || !number || !state || !TSTATES.includes(state)) return null;
  const waiting = state === 'waiting';
  const place = waiting ? whole(t.place) : null;
  return {
    number,
    place: place && place > 0 ? place : null,
    ahead: waiting ? whole(t.ahead) : null,
    state: state as TicketState,
    taken_at: str(t.taken_at),
    called_at: str(t.called_at),
    call_ends_at: state === 'called' ? str(t.call_ends_at) : undefined,
    done_at: str(t.done_at),
    closed: t.closed === true,
  };
}

/** One v1 state (state, join, action, info, a ws frame or the boot JSON) as the queue's guest view. */
export function queueFrom(json: unknown): { view: QueueView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  if (!j || !pocket) return undefined;
  const kv = obj(j.view) ?? {};
  const ks = str(kv.state);
  const ended = str(pocket.state) === 'ended' || ks === 'ended';
  const until = str(kv.hours_until);
  const avg = whole(kv.avg_minutes);
  const call = whole(kv.call_minutes);
  const max = whole(kv.max_waiting);
  const view: QueueView = {
    state: ended ? 'ended' : ks && QSTATES.includes(ks) ? (ks as QueueState) : 'open',
    title: (str(kv.title) ?? str(pocket.title) ?? '').trim(),
    line: (str(kv.line) ?? '').trim(),
    hours_until: until && HHMM.test(until) ? until : undefined,
    now_serving: whole(kv.now_serving) || null,
    waiting: whole(kv.waiting) ?? 0,
    avg_minutes: avg && avg > 0 ? avg : null,
    call_minutes: call && call > 0 ? call : 3,
    max_waiting: max || undefined,
    ask_name: kv.ask_name === true,
    host_name: (str(kv.host_name) ?? '').trim(),
    pause_text: (str(kv.pause_text) ?? '').trim(),
    close_text: (str(kv.close_text) ?? '').trim(),
    paused_at: str(kv.paused_at),
    now: str(kv.now),
    my_ticket: obj(j.me) ? ticketFrom(kv.my_ticket) : null,
    joined: !!obj(j.me),
  };
  return { view, version: num(j.version) };
}

export function phaseOf(v: QueueView): Phase {
  if (v.state === 'ended') return 'ended';
  const t = v.my_ticket;
  if (!t) return 'take';
  if (t.closed) return 'closed';
  if (t.state === 'called') return 'turn';
  if (t.state === 'waiting') return t.place === 1 && v.state === 'open' ? 'next' : 'waiting';
  return t.state;
}

/** A ticket still in the line (the page keeps the buzz, the tab title and the wake lock only for these). */
export const live = (p: Phase): boolean => p === 'waiting' || p === 'next' || p === 'turn';
