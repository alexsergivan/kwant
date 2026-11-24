// The gather guest view (pa10b: Sign-up, RSVP and Vote on one engine), read from one Pocket API v1 state: pocket and
// me from pa1, the kit's view from Lin's gather.rs (`facts` + `guest_view`; server-impl.md "Pocket API v1: gather").
// Values are copied or left out, never worked out; anything with the wrong shape is dropped, so junk from the box
// can't draw a wrong screen. Fields the host didn't choose stay absent.

export type Face = 'rsvp' | 'signup' | 'vote';
export type Phase = 'open' | 'tie' | 'closed';
export type Answer = 'yes' | 'maybe' | 'no';

export interface Person {
  id: string;
  name: string;
  avatar: string;
}

export interface Slot {
  slot: number;
  name: string;
  cap: number;
  filled: number;
  full: boolean;
  waiting: number;
  /** Names only when the host turned show_names on; null otherwise. */
  people: (Person & { note?: string })[] | null;
  waiting_people: Person[] | null;
}

export interface Option {
  option: number;
  name: string;
}

export interface Votes extends Option {
  votes: number;
}

export interface Round {
  round: number;
  counts: Votes[];
  exhausted: number;
  dropped: Option[];
}

export interface Result {
  outcome: 'winner' | 'tie' | 'picked' | 'no_votes';
  winner?: Votes;
  tied: Votes[];
  ballots?: number;
  final_round_active?: number;
  rounds: Round[];
}

export interface Facts {
  face: Face;
  phase: Phase;
  when?: string;
  place?: string;
  host_name?: string;
  price?: string;
  closes_at?: string;
  event_at?: string;
  /** The fields the host chose: name, answer | slot | ballot, plus_ones, food_needs, slot_note. */
  fields: string[];
  show_names: boolean;
  joined?: number;
  // rsvp
  food_choices: string[];
  max_plus_ones: number;
  note_max: number;
  // signup
  waiting_list: boolean;
  slots: Slot[];
  // vote
  show_live_count: boolean;
  ballot?: { kind: 'one_pick' | 'ranked'; options: Option[]; in_play: number[] };
  voted?: number;
}

export interface Mine {
  // rsvp
  answer?: Answer;
  plus_ones?: number;
  food: string[];
  food_other?: string;
  /** Only the guest's own tick, and only once the host ticked it. */
  paid?: { at: string };
  // signup
  slot?: number;
  note?: string;
  waiting?: { slot: number; position: number };
  moved_up?: { slot: number; at?: string; new: boolean };
  // vote
  voted: boolean;
  ballot?: number[];
}

export interface Seat {
  id: string;
  name: string;
  avatar: string;
  /** pa1's own notices: a wrong PIN try on this seat, or the host reset the PIN. */
  notices: { kind: 'wrong_pin' | 'pin_reset'; at: string }[];
}

export interface GatherView {
  title: string;
  ended: boolean;
  /** pocket.expires_at: the day everything is deleted. */
  expires_at?: string;
  keep_days?: number;
  facts: Facts;
  seat?: Seat;
  mine?: Mine;
  counts?: { yes: number; maybe: number; no: number; coming: number };
  /** RSVP names + Yes / Maybe / No, only with show_names. */
  people: (Person & { answer: Answer })[] | null;
  live: Votes[] | null;
  result: Result | null;
}

type Obj = Record<string, unknown>;
const obj = (x: unknown): Obj | undefined => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Obj) : undefined);
const str = (x: unknown): string | undefined => (typeof x === 'string' ? x : undefined);
const text = (x: unknown): string | undefined => (typeof x === 'string' && x.trim() ? x : undefined);
const count = (x: unknown): number | undefined => (typeof x === 'number' && Number.isInteger(x) && x >= 0 ? x : undefined);
const list = (x: unknown): unknown[] => (Array.isArray(x) ? x : []);
const some = <T>(xs: (T | undefined)[]): T[] => xs.filter((x): x is T => x !== undefined);
const FACES: Face[] = ['rsvp', 'signup', 'vote'];
const PHASES: Phase[] = ['open', 'tie', 'closed'];
const ANSWERS: Answer[] = ['yes', 'maybe', 'no'];
const FIELDS = ['name', 'answer', 'slot', 'ballot', 'plus_ones', 'food_needs', 'slot_note'];
/** Food needs the guest page knows how to say (Lin's FOOD); anything else from the box is dropped. */
export const FOOD_LABELS: Record<string, string> = {
  vegetarian: 'Vegetarian',
  vegan: 'Vegan',
  gluten_free: 'Gluten-free',
  dairy_free: 'Dairy-free',
  nut_allergy: 'Nut allergy',
  halal: 'Halal',
  kosher: 'Kosher',
};
const food = (x: unknown): string | undefined => (typeof x === 'string' && x in FOOD_LABELS ? x : undefined);

export function personFrom(x: unknown): Person | undefined {
  const p = obj(x);
  const id = str(p?.id);
  const name = text(p?.name);
  if (!id || !name) return undefined;
  return { id, name, avatar: str(p?.avatar) ?? '' };
}

const optionFrom = (x: unknown): Option | undefined => {
  const o = obj(x);
  const option = count(o?.option);
  const name = text(o?.name);
  return option !== undefined && name ? { option, name } : undefined;
};

const votesFrom = (x: unknown, names?: Map<number, string>): Votes | undefined => {
  const o = obj(x);
  const option = count(o?.option);
  const votes = count(o?.votes);
  const name = text(o?.name) ?? (option !== undefined ? names?.get(option) : undefined);
  return option !== undefined && votes !== undefined && name ? { option, name, votes } : undefined;
};

function slotFrom(x: unknown): Slot | undefined {
  const s = obj(x);
  const slot = count(s?.slot);
  const name = text(s?.name);
  const cap = count(s?.cap);
  const filled = count(s?.filled);
  if (slot === undefined || !name || !cap || filled === undefined) return undefined;
  const people = Array.isArray(s?.people)
    ? some(
        (s!.people as unknown[]).map((p) => {
          const q = personFrom(p);
          return q ? { ...q, note: text(obj(p)?.note) } : undefined;
        }),
      )
    : null;
  return {
    slot,
    name,
    cap,
    filled,
    full: s?.full === true,
    waiting: count(s?.waiting) ?? 0,
    people,
    waiting_people: Array.isArray(s?.waiting_people) ? some((s!.waiting_people as unknown[]).map(personFrom)) : null,
  };
}

function resultFrom(x: unknown, names: Map<number, string>): Result | null {
  const r = obj(x);
  const outcome = str(r?.outcome) as Result['outcome'];
  if (!r || !['winner', 'tie', 'picked', 'no_votes'].includes(outcome)) return null;
  const winner = votesFrom(r.winner, names);
  if (outcome === 'winner' && !winner) return null;
  return {
    outcome,
    winner,
    tied: some(list(r.tied).map((t) => votesFrom(t, names))),
    ballots: count(r.ballots),
    final_round_active: count(r.final_round_active),
    rounds: some(
      list(r.rounds).map((x) => {
        const o = obj(x);
        const round = count(o?.round);
        if (!round) return undefined;
        return {
          round,
          counts: some(list(o?.counts).map((c) => votesFrom(c, names))),
          exhausted: count(o?.exhausted) ?? 0,
          dropped: some(list(o?.dropped).map((d) => optionFrom(d) ?? (count(d) !== undefined && names.has(d as number) ? { option: d as number, name: names.get(d as number)! } : undefined))),
        };
      }),
    ),
  };
}

export function factsFrom(x: unknown): Facts | undefined {
  const f = obj(x);
  const face = str(f?.face) as Face;
  const phase = str(f?.phase) as Phase;
  if (!FACES.includes(face) || !PHASES.includes(phase)) return undefined;
  const b = obj(f?.ballot);
  const kind = str(b?.kind);
  const options = some(list(b?.options).map(optionFrom));
  const fields = list(f?.fields).filter((x): x is string => typeof x === 'string' && FIELDS.includes(x));
  return {
    face,
    phase,
    when: text(f?.when),
    place: text(f?.place),
    host_name: text(f?.host_name),
    price: text(f?.price),
    closes_at: text(f?.closes_at),
    event_at: text(f?.event_at),
    fields,
    // Names are shown only when the server says so in as many words.
    show_names: f?.show_names === true,
    joined: count(f?.joined),
    food_choices: face === 'rsvp' && fields.includes('food_needs') ? some(list(f?.food_choices).map(food)) : [],
    max_plus_ones: face === 'rsvp' && fields.includes('plus_ones') ? (count(f?.max_plus_ones) ?? 0) : 0,
    note_max: count(f?.note_max) || 40,
    waiting_list: f?.waiting_list === true,
    slots: face === 'signup' ? some(list(f?.slots).map(slotFrom)) : [],
    show_live_count: f?.show_live_count === true,
    ballot:
      face === 'vote' && (kind === 'one_pick' || kind === 'ranked') && options.length
        ? { kind, options, in_play: list(b?.in_play).filter((o): o is number => count(o) !== undefined && options.some((x) => x.option === o)) }
        : undefined,
    voted: count(f?.voted),
  };
}

function mineFrom(x: unknown, f: Facts): Mine {
  const m = obj(x) ?? {};
  const answer = str(m.answer) as Answer;
  const paidAt = str(obj(m.paid)?.at);
  const w = obj(m.waiting);
  const mv = obj(m.moved_up);
  const slots = new Set(f.slots.map((s) => s.slot));
  const opts = new Set(f.ballot?.options.map((o) => o.option) ?? []);
  const ballot = Array.isArray(m.ballot) ? m.ballot.filter((o): o is number => count(o) !== undefined && opts.has(o)) : undefined;
  const slot = count(m.slot);
  const wslot = count(w?.slot);
  const position = count(w?.position);
  const mslot = count(mv?.slot);
  return {
    answer: f.face === 'rsvp' && ANSWERS.includes(answer) ? answer : undefined,
    plus_ones: f.max_plus_ones ? count(m.plus_ones) : undefined,
    food: f.food_choices.length ? some(list(m.food).map(food)) : [],
    food_other: f.food_choices.length ? text(m.food_other) : undefined,
    paid: f.face === 'rsvp' && paidAt ? { at: paidAt } : undefined,
    slot: slot !== undefined && slots.has(slot) ? slot : undefined,
    note: text(m.note),
    waiting: wslot !== undefined && slots.has(wslot) && position ? { slot: wslot, position } : undefined,
    moved_up: mslot !== undefined && slots.has(mslot) ? { slot: mslot, at: str(mv?.at), new: mv?.new === true } : undefined,
    voted: f.face === 'vote' && m.voted === true,
    ballot: ballot && ballot.length ? ballot : undefined,
  };
}

/** One v1 state (boot JSON, info, state, join, rejoin, action, a ws frame) as the gather guest view. */
export function gatherFrom(json: unknown): { view: GatherView; version?: number } | undefined {
  const j = obj(json);
  const pocket = obj(j?.pocket);
  if (!j || !pocket || str(pocket.kit) !== 'gather') return undefined;
  const kv = obj(j.view) ?? {};
  const facts = factsFrom(kv);
  if (!facts) return undefined;
  const m = obj(j.me);
  const seat: Seat | undefined =
    m && text(m.name)
      ? {
          id: str(m.id) ?? '',
          name: str(m.name)!,
          avatar: str(m.avatar) ?? '',
          notices: list(m.notices)
            .map(obj)
            .filter((n): n is Obj => !!n && (n.kind === 'wrong_pin' || n.kind === 'pin_reset'))
            .map((n) => ({ kind: n.kind as 'wrong_pin' | 'pin_reset', at: str(n.at) ?? '' })),
        }
      : undefined;
  const c = obj(kv.counts);
  const yes = count(c?.yes);
  const maybe = count(c?.maybe);
  const no = count(c?.no);
  const coming = count(c?.coming);
  const names = new Map((facts.ballot?.options ?? []).map((o) => [o.option, o.name]));
  const retention = obj(pocket.retention);
  const view: GatherView = {
    title: str(pocket.title) ?? '',
    ended: str(pocket.state) === 'ended',
    expires_at: str(pocket.expires_at),
    keep_days: count(retention?.days),
    facts,
    seat,
    mine: seat ? mineFrom(kv.me, facts) : undefined,
    counts: facts.face === 'rsvp' && yes !== undefined && maybe !== undefined && no !== undefined && coming !== undefined ? { yes, maybe, no, coming } : undefined,
    people:
      facts.face === 'rsvp' && facts.show_names && Array.isArray(kv.people)
        ? some(
            kv.people.map((p) => {
              const q = personFrom(p);
              const a = str(obj(p)?.answer) as Answer;
              return q && ANSWERS.includes(a) ? { ...q, answer: a } : undefined;
            }),
          )
        : null,
    // Live 1st choices only for a guest who voted, while voting is open (the server decides; this only reads).
    live: facts.face === 'vote' && seat && facts.phase === 'open' && Array.isArray(kv.live) ? some(kv.live.map((x) => votesFrom(x, names))) : null,
    result: facts.face === 'vote' ? resultFrom(kv.result, names) : null,
  };
  // Without show_names, no slot carries people, whatever the box sent.
  if (!facts.show_names) for (const s of facts.slots) (s.people = null), (s.waiting_people = null);
  const version = typeof j.version === 'number' && Number.isFinite(j.version) ? j.version : undefined;
  return { view, version };
}

export type Screen =
  | 'join' // G1 RSVP, G3 sign-up, G6 one pick, G7 ranked (also "Change my answer" for a seated guest)
  | 'rejoin' // G10
  | 'in' // G2 RSVP you're in; sign-up you're in
  | 'waiting' // G4
  | 'moved' // G5
  | 'voted' // G8
  | 'result' // G9 (and a tie: "Oleks picks")
  | 'closed'; // G11

/** Which screen a view draws. `rejoin`: the guest tapped "I've answered before". `edit`: "Change my answer". */
export function screenOf(v: GatherView, o: { rejoin?: boolean; edit?: boolean } = {}): Screen {
  const f = v.facts;
  const open = f.phase === 'open' && !v.ended;
  if (f.face === 'vote' && (f.phase !== 'open' || v.ended) && v.result) return 'result';
  if (!open) return o.rejoin && !v.ended ? 'rejoin' : 'closed';
  if (!v.seat) return o.rejoin ? 'rejoin' : 'join';
  const m = v.mine!;
  if (o.edit) return 'join';
  if (f.face === 'rsvp') return m.answer ? 'in' : 'join';
  if (f.face === 'signup') {
    if (m.moved_up && m.slot === m.moved_up.slot) return 'moved';
    if (m.waiting) return 'waiting';
    return m.slot !== undefined ? 'in' : 'join';
  }
  return m.voted ? 'voted' : 'join';
}

const host = (f: Facts): string => f.host_name ?? 'The host';

/**
 * The who-sees line (design "For Iris" 3), generated from the fields the host turned on: [who, what] pairs, the who
 * part in bold. Never hard-coded per pocket.
 */
export function whoSees(f: Facts): [string, string][] {
  const h = host(f);
  const has = (x: string) => f.fields.includes(x);
  if (f.face === 'rsvp') {
    const extra = [has('plus_ones') ? 'plus-ones' : '', has('food_needs') ? 'food needs' : ''].filter(Boolean);
    const mine = ['your answer', ...extra];
    const what = mine.length === 1 ? mine[0] : `${mine.slice(0, -1).join(', ')} and ${mine[mine.length - 1]}`;
    return [
      [`${h} sees`, ` ${what}.`],
      ['Other members see', f.show_names ? ' only your name and Yes / Maybe / No.' : ' only the totals, never your name.'],
    ];
  }
  if (f.face === 'signup') {
    const what = has('slot_note') ? 'your name, your slot and what you bring.' : 'your name and your slot.';
    if (f.show_names) return [['Everyone sees', ` ${what}`]];
    return [
      [`${h} sees`, ` ${what}`],
      ['Other members see', ' only how full each slot is.'],
    ];
  }
  const ranked = f.ballot?.kind === 'ranked';
  const out: [string, string][] = [[`${h} sees`, ` that you voted, not ${ranked ? 'your order' : 'what'}.`]];
  if (f.show_live_count) out.push(['Everyone sees', ' the totals after voting.']);
  return out;
}

/** "1st", "2nd", "3rd", "11th": an ordinal for a number the server sent. */
export function nth(n: number): string {
  const t = n % 100;
  if (t >= 11 && t <= 13) return `${n}th`;
  return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`;
}

/**
 * The ranked list's move rule: moving `opt` up or down swaps it with its neighbour; moving the last ranked option down
 * leaves it out. Returns the new order (the input is left as it is).
 */
export function move(order: number[], opt: number, dir: 'up' | 'down'): number[] {
  const i = order.indexOf(opt);
  if (i < 0) return order;
  const next = order.slice();
  if (dir === 'up') {
    if (i === 0) return order;
    [next[i - 1], next[i]] = [next[i], next[i - 1]];
  } else if (i === order.length - 1) next.splice(i, 1);
  else [next[i], next[i + 1]] = [next[i + 1], next[i]];
  return next;
}
