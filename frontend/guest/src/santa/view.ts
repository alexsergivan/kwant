// Secret Santa's guest view (pa10a server-impl.md, "The santa API"), read from one Pocket API v1 state: pocket and me
// from pa1, the kit's view (facts, people, match, asks, reveal) from santa.rs. Values are copied or left out, never
// worked out; anything with the wrong shape is dropped, so junk from the box can't draw a wrong screen.

export type SantaPhase = 'collecting' | 'drawn' | 'revealed';
export type RevealKind = 'scheduled' | 'skipped' | 'never' | 'shown';

export interface Person {
  id: string;
  name: string;
  avatar: string;
}

export interface Facts {
  phase: SantaPhase;
  gift_on?: string;
  budget?: string;
  host_name?: string;
  draw_at?: string;
  reveal?: RevealKind;
  reveal_at?: string;
  joined?: number;
  expected?: number;
}

export interface Ask {
  id: string;
  q: string;
  a?: string;
  at?: string;
  answered_at?: string;
  /** received: not seen yet (ink New). sent: an answer not seen yet. */
  new: boolean;
}

export interface Asks {
  sent: Ask[];
  received: Ask[];
  left_today?: number;
  max_chars: number;
  open: boolean;
}

export interface Me {
  id: string;
  name: string;
  avatar: string;
  /** pa1's own notices: a wrong PIN try on this seat, or the host reset the PIN. */
  notices: { kind: 'wrong_pin' | 'pin_reset'; at: string }[];
}

export interface SantaView {
  title: string;
  ended: boolean;
  /** pocket.expires_at: the day everything is deleted (YYYY-MM-DD or RFC 3339). */
  expires_at?: string;
  /** pocket.retention days after the reveal (or the gift day). */
  keep_days?: number;
  players?: number;
  facts: Facts;
  me?: Me;
  people: Person[];
  my_wish?: string;
  in_draw: boolean;
  can_open: boolean;
  opened: boolean;
  match: (Person & { wish?: string }) | null;
  gift_ready: boolean;
  /** A late joiner took this guest's old match: the match is closed until "Open it" again. */
  match_changed: boolean;
  asks: Asks;
  my_santa: Person | null;
  reveal_loop: { giver: Person; receiver: Person }[] | null;
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const text = (x: unknown): string | undefined => (typeof x === 'string' && x.trim() ? x : undefined);
const count = (x: unknown): number | undefined => (typeof x === 'number' && Number.isInteger(x) && x >= 0 ? x : undefined);
const list = (x: unknown): unknown[] => (Array.isArray(x) ? x : []);
const PHASES: SantaPhase[] = ['collecting', 'drawn', 'revealed'];
const REVEALS: RevealKind[] = ['scheduled', 'skipped', 'never', 'shown'];

export function personFrom(x: unknown): Person | undefined {
  const p = obj(x);
  const id = str(p?.id);
  const name = text(p?.name);
  if (!id || !name) return undefined;
  return { id, name, avatar: str(p?.avatar) ?? '' };
}

function askFrom(x: unknown, flag: 'new' | 'new_answer'): Ask | undefined {
  const a = obj(x);
  const id = str(a?.id);
  const q = text(a?.q);
  if (!id || !q) return undefined;
  return { id, q, a: text(a?.a), at: str(a?.at), answered_at: str(a?.answered_at), new: a?.[flag] === true };
}

export function factsFrom(x: unknown): Facts | undefined {
  const f = obj(x);
  const phase = str(f?.phase) as SantaPhase;
  if (!PHASES.includes(phase)) return undefined;
  const reveal = str(f?.reveal) as RevealKind;
  return {
    phase,
    gift_on: str(f?.gift_on),
    budget: text(f?.budget),
    host_name: text(f?.host_name),
    draw_at: str(f?.draw_at),
    reveal: REVEALS.includes(reveal) ? reveal : undefined,
    reveal_at: str(f?.reveal_at),
    joined: count(f?.joined),
    expected: count(f?.expected),
  };
}

/** One v1 state (boot JSON, info, state, join, rejoin, action, a ws frame) as Santa's guest view. */
export function santaFrom(json: unknown): { view: SantaView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  if (!j || !pocket) return undefined;
  if (str(pocket.kit) !== 'santa') return undefined;
  const kv = obj(j.view) ?? {};
  const facts = factsFrom(kv);
  if (!facts) return undefined;
  const m = obj(j.me);
  const meId = str(m?.id);
  const me: Me | undefined =
    m && text(m.name)
      ? {
          id: meId ?? '',
          name: str(m.name)!,
          avatar: str(m.avatar) ?? '',
          notices: list(m.notices)
            .map(obj)
            .filter((n): n is Obj => !!n && (n.kind === 'wrong_pin' || n.kind === 'pin_reset'))
            .map((n) => ({ kind: n.kind as 'wrong_pin' | 'pin_reset', at: str(n.at) ?? '' })),
        }
      : undefined;
  const seated = !!me;
  const asks = obj(kv.asks);
  const matchObj = obj(kv.match);
  const match = seated ? personFrom(matchObj) : undefined;
  const loop = Array.isArray(kv.reveal_loop)
    ? kv.reveal_loop
        .map(obj)
        .map((l) => ({ giver: personFrom(l?.giver), receiver: personFrom(l?.receiver) }))
        .filter((l): l is { giver: Person; receiver: Person } => !!l.giver && !!l.receiver)
    : null;
  const retention = obj(pocket.retention);
  const view: SantaView = {
    title: str(pocket.title) ?? '',
    ended: str(pocket.state) === 'ended',
    expires_at: str(pocket.expires_at),
    keep_days: count(retention?.days),
    players: count(pocket.players),
    facts,
    me,
    people: list(kv.people).map(personFrom).filter((p): p is Person => !!p),
    my_wish: seated ? text(kv.my_wish) : undefined,
    in_draw: seated && kv.in_draw === true,
    can_open: seated && kv.can_open === true,
    opened: seated && kv.opened === true,
    match: match ? { ...match, wish: text(matchObj?.wish) } : null,
    gift_ready: seated && kv.gift_ready === true,
    match_changed: seated && list(kv.notices).some((n) => obj(n)?.kind === 'match_changed'),
    asks: {
      sent: seated ? list(asks?.sent).map((a) => askFrom(a, 'new_answer')).filter((a): a is Ask => !!a) : [],
      received: seated ? list(asks?.received).map((a) => askFrom(a, 'new')).filter((a): a is Ask => !!a) : [],
      left_today: count(asks?.left_today),
      max_chars: count(asks?.max_chars) || 200,
      open: seated && asks?.open === true,
    },
    // The reveal only when the server says so, and only with its own loop (G8).
    my_santa: seated && facts.phase === 'revealed' ? (personFrom(kv.my_santa) ?? null) : null,
    reveal_loop: seated && facts.phase === 'revealed' && loop && loop.length ? loop : null,
  };
  const version = typeof j.version === 'number' && Number.isFinite(j.version) ? j.version : undefined;
  return { view, version };
}

/** The kit of a v1 state (boot JSON or info), or '' when it can't be read. */
export const kitOf = (json: unknown): string => str(obj(obj(json)?.pocket)?.kit) ?? '';

export type Screen = 'join' | 'rejoin' | 'late' | 'waiting' | 'unwrap' | 'match' | 'reveal' | 'ended';

/**
 * Which screen a view draws. `rejoin`: the guest tapped "I've joined before". `joinLate`: "The host added me".
 * `unwrapped`: this device has opened the gift box before.
 */
export function screenOf(v: SantaView, o: { rejoin?: boolean; joinLate?: boolean; unwrapped?: boolean } = {}): Screen {
  const f = v.facts;
  if (!v.me) {
    if (v.ended || f.phase === 'revealed') return o.rejoin && !v.ended ? 'rejoin' : 'ended';
    if (o.rejoin) return 'rejoin';
    if (f.phase === 'drawn' && !o.joinLate) return 'late';
    return 'join';
  }
  if (v.ended) return 'ended';
  if (f.phase === 'revealed' && v.reveal_loop) return 'reveal';
  if (f.phase === 'collecting' || !v.in_draw) return 'waiting';
  // The match only once this device has unwrapped it; a closed match (match_changed) is opened again from G5.
  if (v.match && !v.match_changed) return o.unwrapped ? 'match' : 'unwrap';
  return v.can_open ? 'unwrap' : 'waiting';
}
