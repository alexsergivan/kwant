// pa12 P2: one viewer's view of a `made` pocket (pa12/plan.md section 1), read defensively into the page's own shape.
// The box filters every collection by its read rule before it sends anything (made::view), so this file never decides
// who may see what: a collection arrives either with `rows` (the ones this viewer may see) or as a `count` only, and a
// count is drawn as a count, never as guessed rows. Junk is dropped, never drawn. The contract is
// autonomy/pocket-apps/pa12/P2/blocks.md.

export type Surface = 'phone' | 'screen' | 'remote';
export type Value = string | number | boolean;
export interface Row {
  id: string;
  owner: string | null;
  mine: boolean;
  values: Record<string, Value>;
}
export interface Collection {
  rows?: Row[];
  count: number;
  locked: boolean;
  /** Only for a collection a `vote` action counts: value -> ballots, worked out on the box. */
  tally?: Record<string, number>;
}
export interface Player {
  id: string;
  name: string;
  avatar: string;
}
export interface Field {
  name: string;
  type: 'text' | 'int' | 'choice' | 'bool';
  label: string;
  max?: number;
  min?: number;
  of?: string[];
}
export type Block =
  | { type: 'say'; text: string | null; big: boolean }
  | { type: 'form'; into: string; action: 'put' | 'answer'; fields: Field[]; submit: string; done: string }
  | { type: 'list'; from: string; show: string[]; empty: string; noun: [string, string]; take_back: boolean }
  | { type: 'board'; from: string; show: string; vote: string; empty: string }
  | { type: 'timer'; label: string }
  | { type: 'scores'; top: number }
  | { type: 'turn' }
  | { type: 'reveal'; secret: string; show: string; label: string; hidden: string };
export interface MadeView {
  title: string;
  words: Record<string, string>;
  blocks: Block[];
  collections: Record<string, Collection>;
  revealed: Record<string, Record<string, Value>[]>;
  step: { index: number; of: number; say: string | null } | null;
  timer: { ends_at: number | null; seconds: number; running: boolean } | null;
  turn: { player: string; next: string | null } | null;
  scores: { player: string; points: number }[] | null;
  players: Player[];
  /** The host's buttons the box allows now (remote only), like the quiz remote's `host.can`. */
  can: string[];
  me: string | null;
  /** The box's clock when it sent this view (ms), so a countdown runs on the box's time, not the phone's. */
  now: number | null;
  version: string;
  ended: boolean;
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown, max = 400): string | undefined => (typeof x === 'string' && x.trim() ? [...x].slice(0, max).join('') : undefined);
const int = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? Math.trunc(x) : undefined);
const arr = (x: unknown): unknown[] => (Array.isArray(x) ? x : []);
const NAME = /^[a-z][a-z0-9_]{0,31}$/;
const name = (x: unknown): string | undefined => (typeof x === 'string' && NAME.test(x) ? x : undefined);
const time = (x: unknown): number | undefined => {
  const t = typeof x === 'string' ? Date.parse(x) : NaN;
  return Number.isFinite(t) ? t : undefined;
};
const value = (x: unknown): Value | undefined => (typeof x === 'string' ? [...x].slice(0, 400).join('') : typeof x === 'number' && Number.isFinite(x) ? x : typeof x === 'boolean' ? x : undefined);
const values = (x: unknown): Record<string, Value> => {
  const out: Record<string, Value> = {};
  for (const [k, v] of Object.entries(obj(x) ?? {})) {
    const val = value(v);
    if (name(k) && val !== undefined) out[k] = val;
  }
  return out;
};

export const MAX_BLOCKS = 12;
const FIELD_TYPES = ['text', 'int', 'choice', 'bool'];

function field(x: unknown): Field | undefined {
  const f = obj(x);
  const n = name(f?.name);
  const t = f?.type;
  if (!f || !n || typeof t !== 'string' || !FIELD_TYPES.includes(t)) return undefined;
  const of = arr(f.of).map((o) => str(o, 60)).filter((o): o is string => !!o).slice(0, 12);
  if (t === 'choice' && of.length < 2) return undefined;
  return { name: n, type: t as Field['type'], label: str(f.label, 60) ?? n, max: int(f.max), min: int(f.min), ...(t === 'choice' ? { of } : {}) };
}

/** One block from the spec's `screens.<surface>.blocks`; an unknown or broken block gives undefined and is skipped. */
export function blockFrom(x: unknown): Block | undefined {
  const b = obj(x);
  switch (b?.type) {
    case 'say': {
      // `from: "step"` shows the current step's line (steps[].say) instead of fixed text.
      const text = str(b.text, 280);
      return text || b.from === 'step' ? { type: 'say', text: text ?? null, big: b.size === 'big' } : undefined;
    }
    case 'form': {
      const into = name(b.into);
      const fields = arr(b.fields).map(field).filter((f): f is Field => !!f).slice(0, 6);
      if (!into || !fields.length) return undefined;
      return { type: 'form', into, action: b.action === 'answer' ? 'answer' : 'put', fields, submit: str(b.submit, 30) ?? 'Send', done: str(b.done, 120) ?? 'Sent.' };
    }
    case 'list': {
      const from = name(b.from);
      const show = arr(b.show).map(name).filter((s): s is string => !!s).slice(0, 3);
      if (!from || !show.length) return undefined;
      const noun = arr(b.noun).map((s) => str(s, 30));
      return { type: 'list', from, show, empty: str(b.empty, 120) ?? 'Nothing here yet.', noun: [noun[0] ?? 'entry', noun[1] ?? noun[0] ?? 'entries'], take_back: b.take_back === true };
    }
    case 'board': {
      const from = name(b.from);
      const show = name(b.show);
      const vote = name(b.vote);
      return from && show && vote ? { type: 'board', from, show, vote, empty: str(b.empty, 120) ?? 'Nothing to vote on yet.' } : undefined;
    }
    case 'timer':
      return { type: 'timer', label: str(b.label, 60) ?? 'Time left' };
    case 'scores':
      return { type: 'scores', top: Math.min(Math.max(int(b.top) ?? 10, 1), 60) };
    case 'turn':
      return { type: 'turn' };
    case 'reveal': {
      const secret = name(b.secret);
      const show = name(b.show);
      if (!secret || !show) return undefined;
      return { type: 'reveal', secret, show, label: str(b.label, 60) ?? 'The answers', hidden: str(b.hidden, 120) ?? 'Revealed at the end.' };
    }
    default:
      return undefined;
  }
}

function rowFrom(x: unknown): Row | undefined {
  const r = obj(x);
  const id = str(r?.id, 64);
  if (!r || !id) return undefined;
  return { id, owner: str(r.owner, 64) ?? null, mine: r.mine === true, values: values(r.values) };
}

function collectionFrom(x: unknown): Collection | undefined {
  const c = obj(x);
  if (!c) return undefined;
  const locked = c.locked === true;
  const t = obj(c.tally);
  const tally = t ? Object.fromEntries(Object.entries(t).filter(([k, v]) => str(k, 64) && int(v) !== undefined && (v as number) >= 0).map(([k, v]) => [k, v as number])) : undefined;
  if (Array.isArray(c.rows)) {
    const rows = c.rows.map(rowFrom).filter((r): r is Row => !!r);
    // A long `everyone` list comes as the newest rows plus the full count (plan section 5): the count is the box's.
    const count = int(c.count);
    return { rows, count: count !== undefined && count >= rows.length ? count : rows.length, locked, ...(tally ? { tally } : {}) };
  }
  const count = int(c.count);
  return count !== undefined && count >= 0 ? { count, locked, ...(tally ? { tally } : {}) } : undefined;
}

/** A pocket/1 state for a made pocket into a MadeView; anything that isn't one gives undefined. */
export function madeFrom(json: unknown): { view: MadeView; version?: number } | undefined {
  const s = obj(json);
  const v = obj(s?.view);
  const p = obj(s?.pocket);
  if (!s || !v || (p && p.kit !== 'made')) return undefined;
  const app = obj(v.app);
  const collections: Record<string, Collection> = {};
  for (const [k, c] of Object.entries(obj(v.collections) ?? {})) {
    const col = name(k) && collectionFrom(c);
    if (col) collections[k] = col;
  }
  const revealed: MadeView['revealed'] = {};
  for (const [k, items] of Object.entries(obj(v.revealed) ?? {})) if (name(k)) revealed[k] = arr(items).map(values).filter((x) => Object.keys(x).length);
  const st = obj(v.step);
  const tm = obj(v.timer);
  const tu = obj(v.turn);
  const words: Record<string, string> = {};
  for (const [k, w] of Object.entries(obj(v.words) ?? {})) {
    const t = name(k) && str(w, 120);
    if (t) words[k] = t;
  }
  const players = arr(v.players)
    .map((x) => {
      const q = obj(x);
      const id = str(q?.id, 64);
      const n = str(q?.name, 40);
      return id && n ? { id, name: n, avatar: str(q?.avatar, 20) ?? 'antenna' } : undefined;
    })
    .filter((x): x is Player => !!x);
  const scores = Array.isArray(v.scores)
    ? v.scores
        .map((x) => {
          const q = obj(x);
          const player = str(q?.player, 64);
          const points = int(q?.points);
          return player && points !== undefined ? { player, points } : undefined;
        })
        .filter((x): x is { player: string; points: number } => !!x)
    : null;
  const me = str(obj(s.me)?.id, 64) ?? null;
  const view: MadeView = {
    title: str(v.title, 80) ?? str(p?.title, 80) ?? 'Pocket app',
    words,
    blocks: arr(app?.blocks).map(blockFrom).filter((b): b is Block => !!b).slice(0, MAX_BLOCKS),
    collections,
    revealed,
    step: st && int(st.index) !== undefined && int(st.of) !== undefined ? { index: int(st.index)!, of: int(st.of)!, say: str(st.say, 200) ?? null } : null,
    timer: tm && int(tm.seconds) !== undefined ? { ends_at: time(tm.ends_at) ?? null, seconds: Math.max(0, int(tm.seconds)!), running: tm.running === true && time(tm.ends_at) !== undefined } : null,
    turn: tu && str(tu.player, 64) ? { player: str(tu.player, 64)!, next: str(tu.next, 64) ?? null } : null,
    scores,
    players,
    can: arr(v.can).filter((c): c is string => typeof c === 'string'),
    me,
    now: time(v.now) ?? null,
    version: str(app?.version, 40) ?? '',
    ended: p?.state === 'ended',
  };
  return { view, version: int(s.version) };
}

export const who = (v: MadeView, id: string | null | undefined): Player | undefined => (id ? v.players.find((p) => p.id === id) : undefined);

/** Seconds left on the box's timer, on the box's clock: `skew` is the box's now minus this device's when the view came. */
export function secondsLeft(t: MadeView['timer'], nowMs: number, skew = 0): number | null {
  if (!t) return null;
  if (!t.running || t.ends_at === null) return t.seconds;
  return Math.max(0, Math.ceil((t.ends_at - (nowMs + skew)) / 1000));
}

export const clock = (s: number): string => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
