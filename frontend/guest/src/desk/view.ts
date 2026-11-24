// The front desk's guest view (pa13/impl.md "Pocket API v1: front desk", which wins over design.md), read from one
// Pocket API v1 state: pocket and me from pa1, the kit's view (title, place, host_name, bot, me, starters, messages,
// waiting_for_host, bot_typing, limits) from desk.rs. Values are copied or left out, never worked out; anything with
// the wrong shape is dropped, so junk from the box can't draw a wrong screen. Only this guest's own conversation is
// ever in the view: the box sends nothing else, and nothing else is read here.

export type From = 'guest' | 'bot' | 'host' | 'note';
export type NoteKind = 'handoff' | 'daily_cap' | 'host_away';

export interface Msg {
  id: string;
  from: From;
  text: string;
  at: string;
  note?: NoteKind;
  client_id?: string;
}

export interface Limits {
  per_hour?: number;
  left_this_hour?: number;
  /** When sending opens again (only while left_this_hour is 0). */
  next_at?: string;
  max_chars: number;
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
}

export type Screen = 'join' | 'rejoin' | 'chat' | 'ended';

/** impl.md: a message is at most 1,000 characters. Used only until the box says its own `limits.max_chars`. */
export const MAX_CHARS = 1000;
const STARTERS = 3;

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);
const FROM: readonly string[] = ['guest', 'bot', 'host', 'note'];
const NOTES: readonly string[] = ['handoff', 'daily_cap', 'host_away'];

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
  } else if (!text.trim()) return undefined;
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
    },
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
