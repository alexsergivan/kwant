// Pocket apps (pa10b): the gather Pocket card in chat, one engine with three faces (Sign-up, RSVP, Vote) and the
// Club Sub Chaser (pa10b/design.md §1, §2, §4, §5, "For Pixel" 1-7). States: 1 RSVP open, 2 People and paid,
// 3 Nudges, 4 no stable address, 5 Sign-up open, 6 someone moved up, 7 Vote open, 8 Vote closed, 9 Vote tie, plus the
// closed RSVP and Sign-up result cards and the Done rows. The host card shows counts, first names and the host's own
// paid ticks; the validator rejects any block where a person carries a ballot. Holo once, only when earned: Picked
// on a closed vote's winner. A tie, not-paid and every nudge stay plain; moving up is ink New. Nothing sends:
// a nudge is Copy or the phone's Share sheet, and a row says "Copied <time>", never "Sent".

import React, { useEffect, useRef, useState } from 'react';
import {
  AlertTriangle,
  ArrowUp,
  BellRing,
  CalendarCheck,
  Check,
  CheckCircle2,
  Clock,
  Copy,
  EyeOff,
  Globe,
  Info,
  ListOrdered,
  Lock,
  Pencil,
  RotateCcw,
  Share2,
  Square,
  Timer,
  Trash2,
  Trophy,
  Users,
  Utensils,
  Wallet,
  Wifi,
  Zap,
} from 'lucide-react';
import type { GatherAnswer, GatherCount, GatherFace, GatherInfo, GatherKind, GatherNudge, GatherPerson, GatherPhase, GatherRound, GatherSlot, PocketBlock } from '../../smartResults';
import { isIndex, isObj, isText, optText } from '../../smartResults';
import { Sticker } from '../Sticker';
import { ModernAvatar } from '../ModernAvatar';
import type { Mood } from '../../kwantMood';
import { pocketApi, PocketApiError } from '../../pocketApi';
import { useFocusAfterRender } from '../../focusAfterRender';
import { Bar, Callout, GHOST, HALF, ICON, LATER_CLS, LaterButton, LinkField, PRIMARY, Qr, Row, STOP } from './pocketParts';
import { GuestCreature, HowTo, NoQuickLink, portOf, validSantaJoin } from './PocketSanta';

// ---------------------------------------------------------------------------------------------------
// Times: the server sends RFC 3339; the card shows them in the person's own time zone. Text that isn't a
// timestamp (a bot that already wrote "Thu 8 PM") shows as it is.

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/;
const parse = (s: string): Date | null => {
  if (!ISO.test(s)) return null;
  const d = new Date(s);
  return Number.isNaN(d.getTime()) ? null : d;
};
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
/** "Fri 9:12 PM" in the person's own time zone. */
const dayTime = (d: Date) => d.toLocaleString('en-US', { weekday: 'short', hour: 'numeric', minute: '2-digit' }).replace(',', '');
const clock = (d: Date) => d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
const dateOnly = (d: Date) => d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });
/** "Thu, Dec 10 at 8:00 PM" */
export const showDateTime = (s: string) => {
  const d = parse(s);
  return d ? `${dateOnly(d)} at ${clock(d)}` : s;
};
/** "Sat, Dec 19" */
export const showDate = (s: string) => {
  const d = parse(s);
  return d ? dateOnly(d) : s;
};
/** "9:12 PM" today, else "Mon 9:12 PM". */
export const showWhen = (s: string, now = new Date()) => {
  const d = parse(s);
  return d ? (sameDay(d, now) ? clock(d) : dayTime(d)) : s;
};

// ---------------------------------------------------------------------------------------------------
// Validator: block.gather is Lin's `host_view` (pa10b/server-shape.md) plus `title` and `expires_at` from the
// pocket, copied by the bot as they are. It comes out as the card's view (smartResults GatherInfo).

export const GATHER_FACES: GatherFace[] = ['signup', 'rsvp', 'vote'];
export const GATHER_PHASES: GatherPhase[] = ['open', 'closed', 'tie'];
const ANSWERS: GatherAnswer[] = ['yes', 'maybe', 'no'];
const KINDS: GatherKind[] = ['maybe', 'not_paid', 'moved_up', 'not_answered'];
const OUTCOMES = ['winner', 'tie', 'picked', 'no_votes'] as const;
const MAX_PEOPLE = 200;
const MAX_SLOTS = 20;
const MAX_OPTIONS = 12;
const isCount = (v: unknown): v is number => isIndex(v) && v <= 100_000;
const firstName = (v: unknown): v is string => isText(v, 40);
const optTime = (v: unknown) => optText(v, 60);
const creature = (v: unknown) => (v === undefined || v === null || v === '' ? null : typeof v === 'string' && /^[a-z0-9_-]{1,32}$/.test(v) ? v : undefined);
const arr = (v: unknown, max: number): unknown[] | undefined => {
  const list = v ?? [];
  return Array.isArray(list) && list.length <= max ? list : undefined;
};

/** The food keys the RSVP offers (Lin's FOOD), in words. Unknown keys read with spaces. */
const FOOD_WORDS: Record<string, string> = { gluten_free: 'gluten-free', dairy_free: 'dairy-free', nut_allergy: 'nut allergy' };
const foodWord = (k: string) => FOOD_WORDS[k] ?? k.replace(/_/g, ' ');
const isFoodKey = (v: unknown): v is string => typeof v === 'string' && /^[a-z_]{1,24}$/.test(v);

/**
 * Keys that would put what someone picked on the host's card. A ballot is secret: the host sees who voted and the
 * totals, never a person's ballot (design "For Juno" #4). Any of these keys on a person-shaped object (one with a
 * name or an id), or anywhere under a list of people, rejects the whole block. `ballots` and `rankings` are rejected
 * anywhere: the host card never carries ballots at all (the count is `result.ballots_counted`). The vote's own
 * `ballot` (kind, options, in_play) sits on the host view's root, which is not a person.
 */
export const BALLOT_KEYS = ['ballot', 'ballots', 'ranking', 'rankings', 'ranked', 'picks', 'pick', 'picked', 'choice', 'choices', 'my_vote', 'voted_for', 'preference', 'preferences'] as const;
const BALLOT_SET = new Set<string>(BALLOT_KEYS);
const ANYWHERE = new Set(['ballots', 'rankings']);
const PERSON_LISTS = new Set(['people', 'person', 'voters', 'voter', 'members', 'member', 'nudges', 'drafts', 'who', 'waiting_people', 'guests', 'guest', 'not_answered', 'moved_up', 'me']);

/** The first path where a person and a ballot meet, or null. Walks arrays and objects; depth-capped. */
export function findBallotPair(v: unknown, path = '', depth = 0, person = false): string | null {
  if (depth > 10 || v === null || typeof v !== 'object') return null;
  if (Array.isArray(v)) {
    for (let i = 0; i < v.length; i++) {
      const hit = findBallotPair(v[i], `${path}[${i}]`, depth + 1, person);
      if (hit) return hit;
    }
    return null;
  }
  const o = v as Record<string, unknown>;
  const keys = Object.keys(o).map((k) => k.toLowerCase());
  const personShaped = person || keys.includes('name') || keys.includes('to') || keys.includes('person_id') || keys.includes('player_id');
  for (const [k, x] of Object.entries(o)) {
    const lk = k.toLowerCase();
    if (ANYWHERE.has(lk) || (personShaped && BALLOT_SET.has(lk))) return `${path}.${k}`;
    const hit = findBallotPair(x, `${path}.${k}`, depth + 1, personShaped || PERSON_LISTS.has(lk));
    if (hit) return hit;
  }
  return null;
}

function validPerson(v: unknown): GatherPerson | undefined {
  if (!isObj(v) || !isText(v.id, 64) || !firstName(v.name)) return undefined;
  const avatar = creature(v.avatar);
  const answer = v.answer === undefined || v.answer === null ? null : ANSWERS.includes(v.answer as GatherAnswer) ? (v.answer as GatherAnswer) : undefined;
  const plus = v.plus_ones === undefined || v.plus_ones === null ? 0 : isIndex(v.plus_ones) && v.plus_ones <= 5 ? v.plus_ones : undefined;
  const food = arr(v.food, 10);
  const other = optText(v.food_other, 80);
  const paidAt = optTime(v.paid_at);
  if (avatar === undefined || answer === undefined || plus === undefined || !food || !food.every(isFoodKey) || other === undefined || paidAt === undefined) return undefined;
  const words = [...(food as string[]).map(foodWord), ...(other ? [other] : [])];
  // A paid tick only means something on a Yes row.
  return { id: v.id, name: v.name, avatar, answer, plus_ones: plus, food: words.length ? words.join(', ') : null, paid: answer === 'yes' ? paidAt !== null : null };
}

function validSlot(v: unknown, i: number): GatherSlot | undefined {
  if (!isObj(v) || v.slot !== i || !isText(v.name, 60) || !isIndex(v.cap) || v.cap < 1 || v.cap > 99) return undefined;
  const people = arr(v.people, v.cap);
  const waitingPeople = arr(v.waiting_people, 50);
  const waiting = v.waiting === undefined || v.waiting === null ? (waitingPeople?.length ?? 0) : isCount(v.waiting) ? v.waiting : undefined;
  if (!people || !waitingPeople || waiting === undefined) return undefined;
  const who: GatherSlot['who'] = [];
  for (const w of people) {
    if (!isObj(w) || !firstName(w.name)) return undefined;
    const note = optText(w.note, 40);
    const avatar = creature(w.avatar);
    if (note === undefined || avatar === undefined) return undefined;
    who.push({ name: w.name, avatar, note: note ?? null });
  }
  const names: string[] = [];
  for (const w of waitingPeople) {
    if (!isObj(w) || !firstName(w.name)) return undefined;
    names.push(w.name);
  }
  return { id: String(i), name: v.name, cap: v.cap, who, waiting: names, waiting_count: Math.max(waiting, names.length) };
}

/** `[{option, name?, votes}]`, named from the ballot's options. */
function validCounts(v: unknown, names: string[]): Array<GatherCount & { option: number }> | undefined {
  const list = arr(v, MAX_OPTIONS);
  if (!list) return undefined;
  const out: Array<GatherCount & { option: number }> = [];
  for (const c of list) {
    if (!isObj(c) || !isIndex(c.option) || c.option >= names.length || !isCount(c.votes)) return undefined;
    out.push({ option: c.option, name: names[c.option], votes: c.votes });
  }
  return out;
}

function validRound(v: unknown, names: string[]): GatherRound | undefined {
  if (!isObj(v)) return undefined;
  const counts = validCounts(v.counts, names);
  const dropped = arr(v.dropped, MAX_OPTIONS);
  const exhausted = v.exhausted === undefined || v.exhausted === null ? 0 : isCount(v.exhausted) ? v.exhausted : undefined;
  if (!counts || counts.length === 0 || !dropped || exhausted === undefined) return undefined;
  const out: string[] = [];
  for (const d of dropped) {
    if (!isObj(d) || !isIndex(d.option) || d.option >= names.length) return undefined;
    out.push(names[d.option]);
  }
  return { counts: counts.map(({ name, votes }) => ({ name, votes })), out, exhausted };
}

function validRsvp(v: Record<string, unknown>, people: GatherPerson[], notAnswered: string[], members: number | null): GatherInfo['rsvp'] | undefined {
  const c = v.counts;
  if (!isObj(c) || !isCount(c.yes) || !isCount(c.maybe) || !isCount(c.no)) return undefined;
  const plus = c.plus_ones === undefined || c.plus_ones === null ? 0 : isCount(c.plus_ones) ? c.plus_ones : undefined;
  if (plus === undefined) return undefined;
  const coming = c.coming === undefined || c.coming === null ? c.yes + plus : isCount(c.coming) ? c.coming : undefined;
  let food: string | null | undefined = null;
  if (v.food !== undefined && v.food !== null) {
    const f = v.food;
    const other = isObj(f) ? arr(f.other, 50) : undefined;
    const counts = isObj(f) ? f.counts ?? {} : undefined;
    if (!isObj(counts) || !other || !other.every((o) => isText(o, 80)) || !Object.entries(counts).every(([k, n]) => isFoodKey(k) && isCount(n))) food = undefined;
    else {
      const parts = Object.entries(counts as Record<string, number>)
        .filter(([, n]) => n > 0)
        .sort((a, b) => b[1] - a[1])
        .map(([k, n]) => `${n} ${foodWord(k)}`);
      const all = [...parts, ...(other as string[])];
      food = all.length ? all.join(', ') : null;
    }
  }
  let paid: NonNullable<GatherInfo['rsvp']>['paid'] | undefined = null;
  if (v.paid !== undefined && v.paid !== null) {
    const p = v.paid;
    const notYet = isObj(p) ? arr(p.not_yet, MAX_PEOPLE) : undefined;
    paid = isObj(p) && isCount(p.ticked) && isCount(p.of) && p.ticked <= p.of && notYet && notYet.every(firstName) ? { ticked: p.ticked, of: p.of, not_yet: notYet as string[] } : undefined;
  }
  if (coming === undefined || food === undefined || paid === undefined) return undefined;
  const by = (a: GatherAnswer) => people.filter((p) => p.answer === a).map((p) => p.name);
  return { yes: c.yes, maybe: c.maybe, no: c.no, plus_ones: plus, coming, maybe_names: by('maybe'), no_names: by('no'), never: members ? notAnswered.length : null, food, paid };
}

function validVote(v: Record<string, unknown>, phase: GatherPhase, members: number | null): GatherInfo['vote'] | undefined {
  const b = v.ballot;
  if (!isObj(b) || (b.kind !== 'one_pick' && b.kind !== 'ranked') || !isCount(v.voted)) return undefined;
  const opts = arr(b.options, MAX_OPTIONS);
  if (!opts || opts.length < 2) return undefined;
  const names: string[] = [];
  for (const [i, o] of opts.entries()) {
    if (!isObj(o) || o.option !== i || !isText(o.name, 60)) return undefined;
    names.push(o.name);
  }
  const inPlayRaw = arr(b.in_play, MAX_OPTIONS);
  if (!inPlayRaw || !inPlayRaw.every((i) => isIndex(i) && i < names.length)) return undefined;
  const inPlay = inPlayRaw.length ? (inPlayRaw as number[]) : names.map((_, i) => i);
  if (members !== null && v.voted > members) return undefined;
  const votersRaw = arr(v.voters, MAX_PEOPLE);
  if (!votersRaw) return undefined;
  const voters: Array<{ name: string; avatar: string | null }> = [];
  for (const p of votersRaw) {
    // Who voted is a name, a creature and a time: nothing else rides along (the ballot scan rejects the rest).
    if (!isObj(p) || !firstName(p.name)) return undefined;
    const avatar = creature(p.avatar);
    if (avatar === undefined) return undefined;
    voters.push({ name: p.name, avatar });
  }
  const options = inPlay.map((i) => ({ option: i, name: names[i] }));
  if (phase === 'open') {
    const live = v.live === undefined || v.live === null ? [] : validCounts(v.live, names);
    if (!live) return undefined;
    // Every place in play shows, 0 if nobody put it first yet.
    const counts = options.map((o) => ({ name: o.name, votes: live.find((c) => c.option === o.option)?.votes ?? 0 }));
    return { kind: b.kind, voted: v.voted, members, voters, options, counts, rounds: [], winner: null, outcome: null, tied: [] };
  }
  const r = v.result;
  if (!isObj(r) || !OUTCOMES.includes(r.outcome as (typeof OUTCOMES)[number])) return undefined;
  const roundsRaw = arr(r.rounds, MAX_OPTIONS + 1);
  if (!roundsRaw) return undefined;
  const rounds = roundsRaw.map((x) => validRound(x, names));
  if (rounds.some((x) => !x)) return undefined;
  const winner = r.winner === undefined || r.winner === null ? null : isObj(r.winner) && isIndex(r.winner.option) && r.winner.option < names.length ? names[r.winner.option] : undefined;
  const tied = validCounts(r.tied, names);
  if (winner === undefined || !tied) return undefined;
  const outcome = r.outcome as (typeof OUTCOMES)[number];
  // A winner only exists once it's closed; a tie names at least two places and no winner.
  if (phase === 'tie' && (outcome !== 'tie' || winner || tied.length < 2)) return undefined;
  if (phase === 'closed' && (outcome === 'tie' || ((outcome === 'winner' || outcome === 'picked') && !winner))) return undefined;
  const last = (rounds[rounds.length - 1] as GatherRound | undefined)?.counts ?? [];
  // The final round's count, with every place in play (0 once it dropped out).
  const counts = options.map((o) => ({ name: o.name, votes: last.find((c) => c.name === o.name)?.votes ?? 0 }));
  return { kind: b.kind, voted: v.voted, members, voters, options, counts, rounds: rounds as GatherRound[], winner, outcome, tied };
}

/** The join link the card shows, for a draft's `{link}`; with none, the draft reads without it. */
const fillLink = (text: string, url: string | null) => (url ? text.split('{link}').join(url) : text.replace(/[\s:]*\{link\}/g, ''));

function validNudge(v: unknown, people: Array<{ id: string; avatar: string | null }>, url: string | null, now: Date): GatherNudge | undefined {
  if (!isObj(v) || !firstName(v.to) || !KINDS.includes(v.kind as GatherKind) || !isText(v.text, 400)) return undefined;
  const pid = v.player_id === undefined || v.player_id === null ? null : isText(v.player_id, 64) ? v.player_id : undefined;
  const copied = optTime(v.copied_at);
  const next = optTime(v.next_copy_at);
  if (pid === undefined || copied === undefined || next === undefined || (v.can_copy !== undefined && typeof v.can_copy !== 'boolean')) return undefined;
  const capped = v.can_copy === false || !!next;
  return {
    key: v.to.toLowerCase(),
    name: v.to,
    player_id: pid,
    avatar: (pid && people.find((p) => p.id === pid)?.avatar) || null,
    why: v.kind as GatherKind,
    text: fillLink(v.text, url),
    copied_at: copied ? showWhen(copied, now) : null,
    until: capped ? (next ? showWhen(next, now) : 'later') : null,
  };
}

/**
 * The `gather` part of a Pocket block: Lin's host view plus the pocket's `title` and `expires_at`
 * (pa10b/server-shape.md). Missing optional fields fall back; a wrong type, a face without its part, a winner
 * before the close, or a person carrying a ballot rejects the whole block (the reply shows as markdown).
 */
export function validGather(v: unknown, fallbackTitle?: string, now = new Date()): GatherInfo | undefined {
  if (!isObj(v) || !GATHER_FACES.includes(v.face as GatherFace) || !GATHER_PHASES.includes(v.phase as GatherPhase)) return undefined;
  if (findBallotPair(v)) return undefined;
  const face = v.face as GatherFace;
  const phase = v.phase as GatherPhase;
  if (phase === 'tie' && face !== 'vote') return undefined;
  const title = v.title === undefined || v.title === null ? fallbackTitle : v.title;
  if (!isText(title, 80) || !isText(v.expires_at, 60)) return undefined;
  const when = optText(v.when, 160);
  const place = optText(v.place, 60);
  const price = optText(v.price, 60);
  const closes = optTime(v.closes_at);
  const event = optTime(v.event_at);
  const host_name = v.host_name === undefined || v.host_name === null ? null : firstName(v.host_name) ? v.host_name : undefined;
  if ([when, place, price, closes, event, host_name].includes(undefined)) return undefined;
  if (v.show_names !== undefined && v.show_names !== null && typeof v.show_names !== 'boolean') return undefined;
  const membersN = v.members === undefined || v.members === null ? 0 : isCount(v.members) ? v.members : undefined;
  const notAnswered = arr(v.not_answered, MAX_PEOPLE);
  if (membersN === undefined || !notAnswered || !notAnswered.every(firstName)) return undefined;
  const members = membersN > 0 ? membersN : null;
  const join = v.join === undefined || v.join === null ? null : validSantaJoin(v.join);
  if (join === undefined) return undefined;
  const url = join?.anywhere_url ?? join?.wifi_url ?? null;

  const peopleRaw = arr(v.people, MAX_PEOPLE);
  if (!peopleRaw) return undefined;
  const people = peopleRaw.map(validPerson);
  if (people.some((p) => !p)) return undefined;

  let rsvp: GatherInfo['rsvp'] | undefined = null;
  let slots: GatherSlot[] = [];
  let moved_up: GatherInfo['moved_up'] = null;
  let vote: GatherInfo['vote'] | undefined = null;
  const avatars: Array<{ id: string; avatar: string | null }> = [...(people as GatherPerson[])];
  if (face === 'rsvp') rsvp = validRsvp(v, people as GatherPerson[], notAnswered as string[], members);
  if (face === 'signup') {
    const raw = arr(v.slots, MAX_SLOTS);
    if (!raw || raw.length === 0) return undefined;
    const list = raw.map(validSlot);
    if (list.some((s) => !s)) return undefined;
    slots = list as GatherSlot[];
    for (const s of raw as Array<Record<string, unknown>>) for (const p of [...((s.people as unknown[]) ?? []), ...((s.waiting_people as unknown[]) ?? [])]) if (isObj(p) && isText(p.id, 64)) avatars.push({ id: p.id, avatar: creature(p.avatar) ?? null });
    const moves = arr(v.moved_up, MAX_PEOPLE);
    if (!moves) return undefined;
    const fresh: Array<{ name: string; slot: string; at: string }> = [];
    for (const m of moves) {
      if (!isObj(m) || !firstName(m.name) || !isIndex(m.slot) || m.slot >= slots.length || (m.seen !== undefined && typeof m.seen !== 'boolean')) return undefined;
      const at = optTime(m.at);
      if (at === undefined) return undefined;
      if (m.seen !== true) fresh.push({ name: m.name, slot: slots[m.slot].name, at: at ? showWhen(at, now) : '' });
    }
    if (fresh.length) {
      const last = fresh[fresh.length - 1];
      moved_up = { ...last, note: null, more: fresh.length - 1 };
    }
  }
  if (face === 'vote') {
    vote = validVote(v, phase, members);
    for (const p of (v.voters as unknown[]) ?? []) if (isObj(p) && isText(p.id, 64)) avatars.push({ id: p.id, avatar: creature(p.avatar) ?? null });
  }
  if (rsvp === undefined || vote === undefined) return undefined;
  if (face === 'rsvp' && !rsvp) return undefined;
  if (face === 'vote' && !vote) return undefined;

  const draftsRaw = arr(v.drafts, MAX_PEOPLE);
  if (!draftsRaw) return undefined;
  const nudges = draftsRaw.map((d) => validNudge(d, avatars, url, now));
  if (nudges.some((n) => !n)) return undefined;
  // State 6's "A note for Hana is ready" is Hana's moved-up draft.
  if (moved_up) moved_up.note = (nudges as GatherNudge[]).find((n) => n.why === 'moved_up' && n.key === moved_up!.name.toLowerCase())?.text ?? null;

  return {
    face,
    phase,
    title,
    when: when ? showDateTime(when) : null,
    place: place ?? null,
    price: price ?? null,
    closes_at: closes ? showDateTime(closes) : null,
    event_at: event ? showDate(event) : null,
    deletes_on: showDate(v.expires_at),
    host_name: host_name ?? null,
    show_names: v.show_names !== false,
    members,
    not_answered: notAnswered as string[],
    rsvp,
    slots,
    moved_up,
    vote,
    people: people as GatherPerson[],
    nudges: nudges as GatherNudge[],
    join,
  };
}

// ---------------------------------------------------------------------------------------------------
// Words and rules

/** The holo "Picked": only on a closed vote with a winner (design "Holo only when earned"). */
export const earnedPicked = (g: Pick<GatherInfo, 'face' | 'phase' | 'vote'>): boolean => g.face === 'vote' && g.phase === 'closed' && !!g.vote?.winner;

/** Tally's mood for this pocket (design §5): idle while open, working on the Chaser or a live vote, finished when closed or someone moved up, needs_you on a tie. */
export function gatherMood(g: Pick<GatherInfo, 'face' | 'phase' | 'moved_up'>, nudgesOpen = false): Mood {
  if (g.phase === 'tie') return 'needs_you';
  if (g.phase === 'closed' || g.moved_up) return 'finished';
  if (nudgesOpen || g.face === 'vote') return 'working';
  return 'idle';
}

/** Hover reasons for buttons whose host action isn't in Lin's API yet (card-impl.md "Disabled until"). */
export const GATHER_LATER = {
  edit: 'Ask Tally in chat to change the details',
  editSlots: 'Ask Tally in chat to change the slots',
  signupPeople: 'Each slot above lists who is in; a list by person comes later',
  deleteBallots: 'Comes with a later update; ballots delete by themselves on the date above',
  deleteAnswers: 'Comes with a later update; answers delete by themselves on the date above',
  noLink: 'The link shows here once your box has one',
} as const;

const FACE_LABEL: Record<GatherFace, string> = { rsvp: 'RSVP', signup: 'Sign-up', vote: 'Vote' };
const FACE_NOUN: Record<GatherFace, string> = { rsvp: 'the RSVP', signup: 'sign-ups', vote: 'the vote' };
const NUM = new Intl.NumberFormat('en');
const num = (n: number) => NUM.format(n);
const usd = (n: number) => `$${n < 0.01 && n > 0 ? '0.01' : n.toFixed(2)}`;
const list = (xs: string[]) => xs.join(', ');
const people = (n: number) => `${num(n)} ${n === 1 ? 'person' : 'people'}`;

export function gatherBudgetLine(b: Pick<PocketBlock, 'budget'>): string {
  const tail = ' Answers, counting and nudge drafts use none.';
  if (!b.budget) return `Bot calls so far aren't counted yet.${tail}`;
  const { calls, usd: spent } = b.budget;
  return `About ${num(calls)} bot ${calls === 1 ? 'call' : 'calls'} to set this up, about ${usd(spent)}.${tail}`;
}

/** The server keeps it 7 days after the event, or after it closes when there's no event. */
export const keptLine = (g: Pick<GatherInfo, 'deletes_on' | 'event_at'>) => `Kept until ${g.deletes_on} (7 days after ${g.event_at ? 'the event' : 'it closes'}), then deleted.`;

/** The line under the title: when · place · price. */
export const whereLine = (g: Pick<GatherInfo, 'when' | 'place' | 'price'>) => [g.when, g.place, g.price].filter(Boolean).join(' · ');

/** The list the host sends the restaurant: names, plus-ones and food needs. Never the price or paid (design §4). */
export function restaurantList(g: GatherInfo): string {
  const yes = g.people.filter((p) => p.answer === 'yes');
  const guests = yes.reduce((n, p) => n + p.plus_ones, 0);
  const when = [g.when, g.place].filter(Boolean).join(' · ');
  const head = `${g.title}${when ? ` · ${when}` : ''}\n${num(yes.length + guests)} coming${guests ? ` (${num(yes.length)} + ${num(guests)} ${guests === 1 ? 'guest' : 'guests'})` : ''}`;
  const rows = yes.map((p) => `- ${p.name}${p.plus_ones ? ` +${p.plus_ones}` : ''}${p.food ? `: ${p.food}` : ''}`);
  const food = g.rsvp?.food ? `\nFood needs: ${g.rsvp.food}` : '';
  return `${head}${food}\n\n${rows.join('\n')}`;
}

/** Who brings what, slot by slot. */
export function whoBringsWhat(g: GatherInfo): string {
  return [g.title, ...g.slots.map((s) => `${s.name} (${s.who.length} of ${s.cap}): ${s.who.length ? s.who.map((w) => (w.note ? `${w.name} (${w.note})` : w.name)).join(', ') : 'nobody'}`)].join('\n');
}

/** The vote's result in one shareable line. */
export function resultLine(g: GatherInfo): string {
  const v = g.vote;
  if (!v?.winner) return g.title;
  const final = v.counts.find((c) => c.name === v.winner)?.votes ?? 0;
  const total = v.counts.reduce((n, c) => n + c.votes, 0);
  return `${g.title}: ${v.winner} ${v.outcome === 'picked' ? `was picked after a tie at ${final} of ${total}` : `wins with ${final} of ${total}`}${v.outcome !== 'picked' && v.kind === 'ranked' && v.rounds.length > 1 ? ' in the final round' : ''}.`;
}

/** The Done row's outcome line: counts and the winner, no names or food needs (design "For Juno" #8). */
export function gatherDoneLine(g: GatherInfo): string {
  if (g.face === 'vote') {
    const v = g.vote!;
    const top = [...v.counts].sort((a, b) => b.votes - a.votes);
    if (v.winner && v.outcome === 'picked') return `${v.winner}, picked after a tie`;
    if (v.winner) return `${v.winner} won${top.length > 1 ? `, ${top[0].votes} to ${top[1].votes}${v.kind === 'ranked' && v.rounds.length > 1 ? ' in the final round' : ''}` : ''}`;
    return `${num(v.voted)} voted`;
  }
  if (g.face === 'rsvp') {
    const r = g.rsvp!;
    return `${num(r.coming)} coming${r.paid ? ` · ${num(r.paid.ticked)} of ${num(r.paid.of)} paid` : ''}`;
  }
  const n = g.slots.reduce((a, s) => a + s.who.length, 0);
  return `${people(n)} in ${g.slots.length} ${g.slots.length === 1 ? 'slot' : 'slots'}`;
}

// ---------------------------------------------------------------------------------------------------
// Pixel glyphs for the kit tiles (design PXI: clip, calcheck, ballot)

const GLYPHS: Record<GatherFace, string[]> = {
  signup: ['...###...', '.###.###.', '.#######.', '.#.....#.', '.#.###.#.', '.#.....#.', '.#.###.#.', '.#.....#.', '.#######.'],
  rsvp: ['.#.....#.', '#########', '#########', '#.......#', '#.....#.#', '#....#..#', '#.#.#...#', '#..#....#', '#########'],
  vote: ['...###...', '...#.#...', '...###...', '.#######.', '##.....##', '#########', '#.......#', '#.......#', '#########'],
};

export const GatherGlyph: React.FC<{ face: GatherFace; size?: number }> = ({ face, size = 22 }) => (
  <svg viewBox="0 0 9 9" width={size} height={size} aria-hidden="true" data-glyph={face} style={{ shapeRendering: 'crispEdges' }}>
    {GLYPHS[face].flatMap((row, y) => [...row].map((c, x) => (c === '#' ? <rect key={`${x}-${y}`} x={x} y={y} width={1} height={1} fill="currentColor" /> : null)))}
  </svg>
);

// ---------------------------------------------------------------------------------------------------
// Small parts

const GatherChip: React.FC<{ g: GatherInfo }> = ({ g }) => {
  if (g.phase === 'tie') return <span className="kw-chip kw-chip-warn">Tie: you pick</span>;
  if (g.phase === 'closed')
    return (
      <span className="kw-chip kw-chip-ok">
        <Check size={12} aria-hidden="true" />
        Closed
      </span>
    );
  if (g.face === 'vote') {
    const v = g.vote!;
    // The live state the host glances for, like the quiz's Live chip. White on sky-600 is 5.1:1.
    return (
      <span className="kw-chip border-sky-400 bg-sky-600 font-semibold text-white" data-chip="voting">
        <span className="kw-status-dot bg-white" aria-hidden="true" />
        Voting · {num(v.voted)}
        {v.members !== null ? ` of ${num(v.members)}` : ''}
      </span>
    );
  }
  const n = g.face === 'rsvp' ? g.rsvp!.yes + g.rsvp!.maybe + g.rsvp!.no : g.slots.reduce((a, s) => a + s.who.length, 0);
  return (
    <span className="kw-chip kw-chip-info">
      <Users size={12} aria-hidden="true" />
      Open · {num(n)} {g.face === 'rsvp' ? 'answered' : 'signed up'}
    </span>
  );
};

const OnlyYou: React.FC = () => (
  <span className="kw-chip" data-chip="only-you">
    <EyeOff size={12} aria-hidden="true" />
    Only you
  </span>
);

/** RSVP: Yes / Maybe / No tiles. On a phone the tiles drop the names ("4 people"). */
export const CountTiles: React.FC<{ r: NonNullable<GatherInfo['rsvp']> }> = ({ r }) => {
  const tile = (label: string, n: number, wide: string, narrow: string) => (
    <div className="min-w-0 rounded-xl border border-hairline-strong bg-surface-main px-3 py-2" data-count={label.toLowerCase()}>
      <div className="text-2xs font-semibold text-zinc-400">{label}</div>
      <div className="text-[22px] font-extrabold leading-7 text-zinc-50 tabular-nums">{num(n)}</div>
      <div className="truncate text-2xs font-medium text-zinc-300">
        <span className="hidden sm:inline">{wide}</span>
        <span className="sm:hidden">{narrow}</span>
      </div>
    </div>
  );
  const coming = r.coming;
  return (
    <div className="grid grid-cols-3 gap-2" role="group" aria-label="Answers">
      {tile('Yes', r.yes, r.plus_ones ? `+${num(r.plus_ones)} plus-ones · ${num(coming)} coming` : `${num(coming)} coming`, `${num(coming)} coming`)}
      {tile('Maybe', r.maybe, r.maybe_names.length ? list(r.maybe_names) : people(r.maybe), people(r.maybe))}
      {tile('No', r.no, r.no_names.length ? list(r.no_names) : people(r.no), people(r.no))}
    </div>
  );
};

/** RSVP: paid so far, with the not-yet names. Host-only: the "Only you" chip says so. */
export const PaidSummary: React.FC<{ paid: NonNullable<NonNullable<GatherInfo['rsvp']>['paid']>; bar?: boolean }> = ({ paid, bar = true }) => (
  <>
    <Row icon={<Wallet size={15} aria-hidden="true" />} aside={<OnlyYou />}>
      <strong className="font-semibold text-zinc-50 tabular-nums">
        Paid {num(paid.ticked)} of {num(paid.of)}
      </strong>
      {paid.not_yet.length > 0 ? ` · not yet: ${list(paid.not_yet)}` : ' · everyone paid'}
    </Row>
    {bar && paid.of > 0 && (
      <li className="list-none">
        <Bar value={paid.ticked / paid.of} label={`${num(paid.ticked)} of ${num(paid.of)} paid`} />
      </li>
    )}
  </>
);

const Pips: React.FC<{ n: number; cap: number }> = ({ n, cap }) =>
  cap <= 10 ? (
    <span className="inline-flex gap-[3px]" aria-hidden="true" data-pips>
      {Array.from({ length: cap }, (_, i) => (
        <i key={i} className={`h-2 w-2 rounded-[2px] ${i < n ? 'bg-sky-300' : 'bg-zinc-700'}`} />
      ))}
    </span>
  ) : null;

/** Sign-up: slot rows with cap pips, "n of cap", Full, who's in, and the waiting list. */
export const SlotList: React.FC<{ slots: GatherSlot[] }> = ({ slots }) => (
  <ul className="flex flex-col gap-1.5" aria-label="Slots" data-slots>
    {slots.map((s) => {
      const full = s.who.length >= s.cap;
      return (
        <li
          key={s.id}
          className={`grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-1 rounded-xl border bg-surface-main px-3 py-2 text-[15px] leading-[22px] sm:grid-cols-[128px_150px_minmax(0,1fr)] sm:text-[13px] sm:leading-5 ${
            full ? 'border-sky-800' : 'border-hairline'
          }`}
          data-slot={s.name}
        >
          <span className="min-w-0 font-semibold text-zinc-50">{s.name}</span>
          <span className="flex items-center gap-[7px] whitespace-nowrap text-zinc-300 tabular-nums" aria-label={`${s.who.length} of ${s.cap}${full ? ', full' : ''}`}>
            <Pips n={s.who.length} cap={s.cap} />
            <span aria-hidden="true">
              {s.who.length} of {s.cap}
            </span>
            {full && (
              <span className="kw-chip" aria-hidden="true">
                Full
              </span>
            )}
          </span>
          <span className="col-span-2 min-w-0 text-zinc-300 sm:col-span-1">{s.who.length ? s.who.map((w) => (w.note ? `${w.name} (${w.note})` : w.name)).join(', ') : 'Nobody yet'}</span>
          {s.waiting_count > 0 && (
            <span className="col-span-2 text-[13px] text-zinc-400 sm:col-start-3 sm:text-[12.5px]" data-waiting>
              Waiting list: {s.waiting.length ? s.waiting.map((n, i) => `${i + 1} ${n}`).join(', ') : people(s.waiting_count)} · moves up on its own
            </span>
          )}
        </li>
      );
    })}
  </ul>
);

/** Vote: one bar per place. The leader (or every tied place) is bold with a `--sky-400` bar. */
export const VoteBars: React.FC<{ counts: GatherCount[]; total: number; lead: string[] }> = ({ counts, total, lead }) => (
  <ul className="flex flex-col gap-[7px]" aria-label="Count" data-vbars>
    {counts.map((c) => {
      const on = lead.includes(c.name);
      return (
        <li
          key={c.name}
          className={`grid grid-cols-[116px_minmax(0,1fr)_36px] items-center gap-2.5 text-[15px] sm:grid-cols-[150px_minmax(0,1fr)_44px] sm:text-[13px] ${on ? 'font-semibold text-zinc-50' : 'text-zinc-200'}`}
          data-lead={on || undefined}
        >
          <span className="truncate">{c.name}</span>
          <span className="h-2.5 overflow-hidden rounded-[3px] border border-hairline bg-surface-main" role="img" aria-label={`${c.name}: ${c.votes} of ${total}`}>
            <i
              className={`block h-full w-full origin-left ${on ? 'bg-sky-400' : 'bg-zinc-600'}`}
              style={{ transform: `scaleX(${total ? Math.min(1, c.votes / total) : 0})`, transition: 'transform 200ms var(--kw-ease)' }}
            />
          </span>
          <span className="text-right text-zinc-300 tabular-nums">{c.votes}</span>
        </li>
      );
    })}
  </ul>
);

/** "How it was counted": each round in plain words. */
const Rounds: React.FC<{ rounds: GatherRound[] }> = ({ rounds }) =>
  rounds.length > 1 ? (
    <ol className="list-decimal pl-5 text-[13px] leading-[19px] text-zinc-400 marker:text-zinc-500 sm:text-[12.5px]" aria-label="How it was counted" data-rounds>
      {rounds.map((r, i) => (
        <li key={i}>
          Round {i + 1}: {r.counts.map((c) => `${c.name} ${c.votes}`).join(' · ')}.{r.out.length ? ` ${r.out.join(' and ')} ${r.out.length === 1 ? 'drops' : 'drop'} out; ${r.out.length === 1 ? 'its votes move' : 'their votes move'} to each voter's next choice.` : ''}
          {r.exhausted ? ` ${num(r.exhausted)} ${r.exhausted === 1 ? 'ballot had' : 'ballots had'} no choice left.` : ''}
        </li>
      ))}
    </ol>
  ) : null;

const VoterStack: React.FC<{ voters: Array<{ name: string; avatar: string | null }> }> = ({ voters }) =>
  voters.length > 0 ? (
    <span className="flex -space-x-1.5" aria-hidden="true" data-stack>
      {voters.slice(0, 5).map((p, i) => (
        <span key={`${p.name}-${i}`} className="rounded-md bg-surface-raised">
          <GuestCreature avatar={p.avatar} size={22} />
        </span>
      ))}
    </span>
  ) : null;

const AnswerChip: React.FC<{ p: GatherPerson }> = ({ p }) => {
  if (!p.answer) return <span className="kw-chip">No answer</span>;
  const text = p.answer === 'yes' ? `Yes${p.plus_ones ? ` +${p.plus_ones}` : ''}` : p.answer === 'maybe' ? 'Maybe' : 'No';
  return <span className={`kw-chip ${p.answer === 'yes' ? 'kw-chip-ok' : p.answer === 'maybe' ? 'kw-chip-warn' : ''}`}>{text}</span>;
};

// ---------------------------------------------------------------------------------------------------
// How members join (pa10a's box with club words)

type JoinMode = 'anywhere' | 'wifi';

const GatherJoinBox: React.FC<{ g: GatherInfo; name: string }> = ({ g, name }) => {
  const join = g.join;
  const anywhere = join?.mode === 'anywhere' && !!join.anywhere_url;
  const [mode, setMode] = useState<JoinMode>(anywhere ? 'anywhere' : 'wifi');
  const radios = useRef<Array<HTMLButtonElement | null>>([]);
  const url = !join ? null : mode === 'anywhere' && anywhere ? join.anywhere_url : join.wifi_url;
  const modes: JoinMode[] = ['anywhere', 'wifi'];
  const onKey = (e: React.KeyboardEvent) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const next = modes[(modes.indexOf(mode) + 1) % 2];
    setMode(next);
    radios.current[modes.indexOf(next)]?.focus();
  };
  const P = 'text-[15px] leading-[22px] text-zinc-100 sm:text-body';
  const SMALL = 'text-[13px] leading-5 text-zinc-400 sm:text-meta';
  return (
    <div className="flex flex-col gap-4 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5 sm:flex-row" data-join="full" data-join-mode={anywhere ? 'anywhere' : join?.wifi_url ? 'wifi' : 'none'}>
      {url ? (
        <div key={url} className="kw-fade-in">
          <Qr url={url} size="full" name={name} />
        </div>
      ) : null}
      <div className="flex min-w-0 flex-1 flex-col gap-2.5">
        <div className="text-[13px] font-semibold text-zinc-200 sm:text-meta" id={`join-${g.face}`}>
          How members join
        </div>
        {anywhere ? (
          <>
            <div role="radiogroup" aria-labelledby={`join-${g.face}`} onKeyDown={onKey} className="inline-flex self-start rounded-xl border border-hairline-strong bg-surface-rail p-1">
              {modes.map((m, i) => {
                const on = m === mode;
                const Icon = m === 'wifi' ? Wifi : Globe;
                return (
                  <button
                    key={m}
                    ref={(el) => (radios.current[i] = el)}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    tabIndex={on ? 0 : -1}
                    onClick={() => setMode(m)}
                    className={`flex min-h-[44px] flex-none items-center justify-center gap-1.5 whitespace-nowrap rounded-lg px-3 text-meta font-semibold kw-focus sm:min-h-[32px] ${on ? 'bg-sky-600 text-white' : 'text-zinc-300 hover:text-zinc-100'}`}
                  >
                    <Icon size={14} aria-hidden="true" />
                    {m === 'wifi' ? 'Same Wi-Fi' : 'From anywhere'}
                  </button>
                );
              })}
            </div>
            {mode === 'anywhere' ? (
              <p className={P}>
                {join?.anywhere_kind === 'https' ? 'Uses your own https address.' : "Uses your Mac's Tailscale Funnel address."} It stays the same, so the link can sit in the club chat until {FACE_NOUN[g.face]}{' '}
                {g.face === 'signup' ? 'close' : 'closes'}.
              </p>
            ) : (
              <p className={P}>Members on your Wi-Fi open the page; anyone else can't reach it. Pick From anywhere for the club chat.</p>
            )}
            <p className={SMALL}>Works while this Mac is on. When it sleeps, the page says so and tries again by itself; no answer is lost.</p>
            {url ? <LinkField url={url} /> : <p className={SMALL}>Your box hasn't said its Wi-Fi address yet.</p>}
          </>
        ) : (
          <>
            <p className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5" data-no-link={join?.wifi_url ? undefined : true}>
              <Wifi size={15} className={`${ICON} ${join?.wifi_url ? 'text-sky-300' : 'text-zinc-500'}`} aria-hidden="true" />
              {join?.wifi_url ? (
                <span>
                  <strong className="font-semibold text-zinc-50">Same Wi-Fi only, for now.</strong> Members on the same Wi-Fi as this Mac can answer; anyone else can't reach the page.
                </span>
              ) : (
                <span>
                  <strong className="font-semibold text-zinc-50">Only this Mac can open the page right now.</strong> Phones need an address they can reach.
                </span>
              )}
            </p>
            <div className="flex flex-col gap-2.5 rounded-xl border border-hairline bg-surface-rail/60 p-3" data-callout="no-stable-address">
              <p className="flex items-start gap-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-5">
                <Info size={15} className={`${ICON} text-zinc-400`} aria-hidden="true" />
                <span>No address that stays the same is set up on this Mac. To let members answer from home until it closes, turn on Tailscale Funnel: free, about 5 minutes.</span>
              </p>
              <HowTo port={portOf(join?.wifi_url ?? null)} bot="Tally" chat="the club chat" />
              <NoQuickLink />
            </div>
            {url && <LinkField url={url} />}
          </>
        )}
      </div>
    </div>
  );
};

/** Once shared: a smaller code and the link that stays until it closes. */
const GatherCompactJoin: React.FC<{ g: GatherInfo; name: string }> = ({ g, name }) => {
  const url = g.join?.anywhere_url ?? g.join?.wifi_url ?? null;
  if (!url) return null;
  const Icon = g.join?.anywhere_url ? Globe : Wifi;
  return (
    <div className="flex flex-col items-center gap-3.5 rounded-2xl border border-sky-800/70 bg-surface-rail/40 p-3.5 sm:flex-row" data-join="compact">
      <Qr url={url} size="compact" name={name} />
      <div className="flex w-full min-w-0 flex-1 flex-col gap-2">
        <p className="flex items-start gap-1.5 text-[13px] leading-5 text-zinc-300 sm:text-meta">
          <Icon size={14} className="mt-0.5 shrink-0 text-sky-300" aria-hidden="true" />
          Same link until it closes. Members open it on their phone: no app, no account.
        </p>
        <LinkField url={url} />
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// People and paid (state 2): host view only

const SHOW_FIRST = 9;

export const PeoplePaidPanel: React.FC<{ block: PocketBlock; g: GatherInfo }> = ({ block, g }) => {
  const [paid, setPaid] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [all, setAll] = useState(false);
  const total = g.people.length;
  const shown = all ? g.people : g.people.slice(0, SHOW_FIRST);
  const tick = async (p: GatherPerson) => {
    const was = paid[p.id] ?? p.paid === true;
    setPaid((m) => ({ ...m, [p.id]: !was }));
    setBusy(p.id);
    setError(null);
    try {
      await pocketApi.setPaid(block.code, p.id, !was);
    } catch (e) {
      setPaid((m) => ({ ...m, [p.id]: was }));
      setError(`Couldn't save ${p.name}'s tick: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`);
    } finally {
      setBusy(null);
    }
  };
  return (
    <section className="rounded-2xl border border-hairline-strong bg-surface-rail/40" aria-label="People and paid" data-people>
      <header className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 border-b border-hairline px-3.5 py-2.5">
        <h4 className="text-[15px] font-semibold text-zinc-100 sm:text-meta">People and paid · {num(total)}</h4>
        <span className="flex items-center gap-1 text-[13px] font-medium text-zinc-400 sm:text-2xs">
          <EyeOff size={12} aria-hidden="true" />
          Paid ticks: only you see them
        </span>
      </header>
      {error && (
        <p role="alert" className="flex items-start gap-2 border-b border-hairline px-3.5 py-2 text-[15px] text-bad sm:text-[13px]">
          <AlertTriangle size={15} className={ICON} aria-hidden="true" />
          {error}
        </p>
      )}
      {g.people.length === 0 ? (
        <p className="px-3.5 py-3 text-[15px] text-zinc-400 sm:text-[13px]">Nobody has answered yet. Share the link in the club chat.</p>
      ) : (
        <ul>
          {shown.map((p) => {
            const on = paid[p.id] ?? p.paid === true;
            return (
              <li key={p.id} className="flex min-h-[56px] items-center gap-3 border-b border-hairline px-3.5 last:border-b-0 sm:min-h-[48px]" data-person={p.name}>
                <GuestCreature avatar={p.avatar} size={24} />
                <span className="flex min-w-0 flex-1 flex-col py-1.5">
                  <span className="truncate text-[15px] font-semibold text-zinc-100 sm:text-body">
                    {p.name}
                  </span>
                  <span className="truncate text-[13px] text-zinc-400 sm:text-2xs">{p.food ?? 'no food needs'}</span>
                </span>
                <AnswerChip p={p} />
                {p.answer === 'yes' && p.paid !== null ? (
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={on}
                    aria-label={`${p.name} paid`}
                    aria-busy={busy === p.id || undefined}
                    onClick={() => tick(p)}
                    className={`inline-flex min-h-[44px] min-w-[96px] shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 text-[13px] font-semibold kw-focus sm:min-h-[32px] sm:text-[12.5px] ${
                      on ? 'border-emerald-300/35 text-ok' : 'border-hairline-strong text-zinc-300 hover:bg-white/[0.04]'
                    }`}
                    data-paid={on}
                  >
                    <span className={`grid h-4 w-4 place-items-center rounded border-2 ${on ? 'border-transparent bg-[color:var(--kw-ok)] text-surface-rail' : 'border-zinc-500'}`} aria-hidden="true">
                      {on && <Check size={11} strokeWidth={3} />}
                    </span>
                    {on ? 'Paid' : 'Not yet'}
                  </button>
                ) : (
                  <span className="hidden min-w-[96px] sm:block" aria-hidden="true" />
                )}
              </li>
            );
          })}
        </ul>
      )}
      {!all && g.people.length > SHOW_FIRST && (
        <button type="button" onClick={() => setAll(true)} className="flex min-h-[44px] w-full items-center rounded-b-2xl border-t border-hairline px-3.5 text-[13px] font-semibold text-sky-300 hover:bg-white/[0.03] kw-focus sm:min-h-[36px] sm:text-[12.5px]" data-action="show-all">
          Show all {num(total)}
        </button>
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// Nudges (state 3): the Chaser. Nothing sends by itself.

const GROUP: Record<GatherKind, { title: string; aside: string; why: string }> = {
  maybe: { title: 'Said maybe', aside: 'to each person', why: 'maybe' },
  not_paid: { title: 'Not paid yet', aside: 'only to the person, never the group', why: 'not paid' },
  moved_up: { title: 'Moved up', aside: 'to each person', why: 'moved up' },
  not_answered: { title: "Haven't answered", aside: 'to each person', why: 'no answer' },
};
const NOT_VOTED = { title: "Haven't voted", why: 'not voted' };

/** A draft's next copy time from a host view (the action's answer or a fresh GET), shown in the person's zone. */
function nextCopyFrom(view: unknown, to: string): string | null {
  const v = isObj(view) && isObj(view.host_view) ? view.host_view : view;
  const drafts = isObj(v) && Array.isArray(v.drafts) ? v.drafts : [];
  const d = drafts.find((x) => isObj(x) && typeof x.to === 'string' && x.to.toLowerCase() === to.toLowerCase());
  return isObj(d) && typeof d.next_copy_at === 'string' && d.next_copy_at.length <= 60 ? showWhen(d.next_copy_at) : null;
}

type Local = { at: string; until: string; capped: boolean } | 'skipped';
const canShare = () => typeof navigator !== 'undefined' && typeof navigator.share === 'function';

export const NudgesPanel: React.FC<{ block: PocketBlock; g: GatherInfo }> = ({ block, g }) => {
  const [local, setLocal] = useState<Record<string, Local>>({});
  const [texts, setTexts] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState<string | null>(null);
  const [note, setNote] = useState<{ kind: 'ok' | 'error'; text: string } | null>(null);
  const focusAfter = useFocusAfterRender();
  const panelRef = useRef<HTMLElement | null>(null);
  const share = canShare();
  const textOf = (n: GatherNudge) => texts[n.key] ?? n.text;
  const record = async (n: GatherNudge) => {
    const now = new Date();
    const in48 = dayTime(new Date(now.getTime() + 48 * 3600_000));
    try {
      const r = await pocketApi.copied(block.code, n.name);
      setLocal((m) => ({ ...m, [n.key]: { at: clock(now), until: nextCopyFrom(r, n.name) ?? in48, capped: false } }));
    } catch (e) {
      if (e instanceof PocketApiError && e.status === 409) {
        // Copied inside the 48 h window: the box's view says until when.
        const until = await pocketApi.read(block.code).then((v) => nextCopyFrom(v, n.name), () => null);
        setLocal((m) => ({ ...m, [n.key]: { at: n.copied_at ?? clock(now), until: until ?? 'later', capped: true } }));
      } else {
        setLocal((m) => ({ ...m, [n.key]: { at: clock(now), until: in48, capped: false } }));
        setNote({ kind: 'error', text: `Copied, but your box didn't note it: ${e instanceof PocketApiError ? e.message : 'something went wrong.'}` });
      }
    }
  };
  const copy = async (n: GatherNudge) => {
    setNote(null);
    try {
      await navigator.clipboard.writeText(textOf(n));
    } catch {
      setNote({ kind: 'error', text: `Couldn't copy ${n.name}'s note. Select the text and copy it yourself.` });
      return;
    }
    await record(n);
  };
  const shareIt = async (n: GatherNudge) => {
    setNote(null);
    try {
      await navigator.share({ text: textOf(n) });
    } catch (e) {
      if ((e as Error)?.name !== 'AbortError') setNote({ kind: 'error', text: "Couldn't open the share sheet. Copy works the same." });
      return;
    }
    await record(n);
  };
  const skip = (n: GatherNudge) => {
    setLocal((m) => ({ ...m, [n.key]: 'skipped' }));
    setNote({ kind: 'ok', text: `Skipped ${n.name}. Nothing goes to them.` });
    focusAfter(() => panelRef.current?.querySelector<HTMLElement>('[data-nudge-open] [data-action="copy-nudge"], [data-action="copy-nudge"]'));
  };
  const groups = (Object.keys(GROUP) as GatherKind[]).map((why) => ({ why, rows: g.nudges.filter((n) => n.why === why) })).filter((x) => x.rows.length > 0);
  const live = g.nudges.filter((n) => local[n.key] !== 'skipped').length;
  // A vote's missing members read "Haven't voted".
  const words = (why: GatherKind) => (why === 'not_answered' && g.face === 'vote' ? { ...GROUP[why], ...NOT_VOTED } : GROUP[why]);
  const btn = 'kw-btn-ghost min-h-[44px] sm:min-h-[32px] whitespace-nowrap text-[14px] px-2.5 sm:px-3 sm:text-meta';
  const primary = 'kw-btn-primary kw-hover-deepen min-h-[44px] sm:min-h-[32px] whitespace-nowrap text-[14px] px-2.5 sm:px-3 sm:text-meta';
  return (
    <section ref={panelRef} className="rounded-2xl border border-hairline-strong bg-surface-rail/40" aria-label="Nudges" data-nudges>
      <header className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 border-b border-hairline px-3.5 py-2.5">
        <h4 className="text-[15px] font-semibold text-zinc-100 sm:text-meta">Nudges · {num(live)} drafted</h4>
        <span className="text-[13px] font-medium text-zinc-400 sm:text-2xs">Nothing sends by itself</span>
      </header>
      {note && (
        <p role="status" className={`flex items-start gap-2 border-b border-hairline px-3.5 py-2 text-[15px] sm:text-[13px] ${note.kind === 'ok' ? 'text-zinc-300' : 'text-bad'}`}>
          {note.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
          {note.text}
        </p>
      )}
      {groups.length === 0 ? (
        <p className="px-3.5 py-3 text-[15px] text-zinc-400 sm:text-[13px]">No nudges needed right now. Everyone has answered{g.face === 'rsvp' ? ' and paid' : ''}.</p>
      ) : (
        groups.map(({ why, rows }) => {
          const shownRows = rows.filter((n) => local[n.key] !== 'skipped');
          // The first row in each group that can be copied now is open with its draft.
          const openId = shownRows.find((n) => !n.until && !local[n.key])?.key ?? null;
          const w = words(why);
          return (
            <div key={why} role="group" aria-label={`${w.title} · ${shownRows.length}`}>
              <div className="flex justify-between gap-2 border-b border-hairline bg-sky-500/[0.06] px-3.5 py-2 text-[13px] font-bold text-zinc-300 sm:text-2xs">
                <span>
                  {w.title} · {num(shownRows.length)}
                </span>
                <span className="font-medium text-zinc-400">{w.aside}</span>
              </div>
              <ul>
                {shownRows.map((n) => {
                  const l = local[n.key];
                  const done = l && l !== 'skipped' ? l : null;
                  const capped = !!n.until && !done;
                  const open = n.key === openId;
                  return (
                    <li key={n.key} className="grid grid-cols-[28px_minmax(0,1fr)] gap-x-2.5 gap-y-1 border-b border-hairline px-3.5 py-2.5 last:border-b-0" data-nudge={n.name} data-nudge-open={open || undefined}>
                      <GuestCreature avatar={n.avatar} size={26} />
                      <div className="flex min-h-[28px] flex-wrap items-center gap-2 text-[15px] font-semibold text-zinc-100 sm:text-[13.5px]">
                        {n.name}
                        <span className="text-[13px] font-medium text-zinc-400 sm:text-2xs">{w.why}</span>
                        {done ? (
                          <span className="kw-chip" data-chip="copied">
                            <Check size={12} aria-hidden="true" />
                            Copied {done.at}
                          </span>
                        ) : capped && n.copied_at ? (
                          <span className="kw-chip" data-chip="copied">
                            <Check size={12} aria-hidden="true" />
                            Copied {n.copied_at}
                          </span>
                        ) : null}
                        {!open && !done && !capped && (
                          <button type="button" onClick={() => copy(n)} className={`${btn} ml-auto hidden sm:inline-flex`} data-action="copy-nudge" aria-label={`Copy ${n.name}'s note`}>
                            <Copy size={14} aria-hidden="true" />
                            Copy
                          </button>
                        )}
                      </div>
                      {open &&
                        (editing === n.key ? (
                          <textarea
                            autoFocus
                            aria-label={`${n.name}'s note`}
                            maxLength={400}
                            value={textOf(n)}
                            onChange={(e) => setTexts((m) => ({ ...m, [n.key]: e.target.value }))}
                            className="col-start-2 min-h-[88px] rounded-lg border border-hairline-strong bg-surface-raised px-2.5 py-2 text-[15px] leading-[22px] text-zinc-100 kw-focus sm:text-[13px] sm:leading-[19px]"
                          />
                        ) : (
                          <p className="col-start-2 rounded-[4px_10px_10px_10px] border border-hairline bg-surface-raised px-2.5 py-2 text-[15px] leading-[22px] text-zinc-200 sm:text-[13px] sm:leading-[19px]" data-draft>
                            {textOf(n)}
                          </p>
                        ))}
                      {done ? (
                        <p className="col-start-2 text-[13px] leading-5 text-zinc-400 sm:text-[12.5px]" role="status">
                          {done.capped ? `One nudge per person every 48 hours. Next one from ${done.until}.` : `Copied, not sent: you send it from your own chat. Copy again from ${done.until}.`}
                        </p>
                      ) : capped ? (
                        <p className="col-start-2 text-[13px] leading-5 text-zinc-400 sm:text-[12.5px]" data-capped>
                          One nudge per person every 48 hours. Next one from {n.until}.
                        </p>
                      ) : open ? (
                        <div className="col-start-2 mt-1 flex flex-wrap items-center gap-2">
                          <button type="button" onClick={() => copy(n)} className={primary} data-action="copy-nudge" aria-label={`Copy ${n.name}'s note`}>
                            <Copy size={14} aria-hidden="true" />
                            Copy
                          </button>
                          {share && (
                            <button type="button" onClick={() => shareIt(n)} className={btn} data-action="share-nudge">
                              <Share2 size={14} aria-hidden="true" />
                              Share…
                            </button>
                          )}
                          <button type="button" onClick={() => setEditing(editing === n.key ? null : n.key)} aria-pressed={editing === n.key} className={btn} data-action="edit-nudge">
                            <Pencil size={14} aria-hidden="true" />
                            {editing === n.key ? 'Done' : 'Edit'}
                          </button>
                          <button type="button" onClick={() => skip(n)} className={btn} data-action="skip-nudge">
                            Skip
                          </button>
                        </div>
                      ) : (
                        // Phone: Copy and Share… on their own row.
                        <div className="col-start-2 flex flex-wrap gap-2 sm:hidden">
                          <button type="button" onClick={() => copy(n)} className={btn} data-action="copy-nudge" aria-label={`Copy ${n.name}'s note`}>
                            <Copy size={14} aria-hidden="true" />
                            Copy
                          </button>
                          {share && (
                            <button type="button" onClick={() => shareIt(n)} className={btn} data-action="share-nudge">
                              <Share2 size={14} aria-hidden="true" />
                              Share…
                            </button>
                          )}
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          );
        })
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card's rows per face

const KeptRow: React.FC<{ g: GatherInfo }> = ({ g }) => <Row icon={<Lock size={15} aria-hidden="true" />}>{keptLine(g)}</Row>;

const CopyNote: React.FC<{ text: string; label: string }> = ({ text, label }) => {
  const [done, setDone] = useState<'yes' | 'failed' | null>(null);
  useEffect(() => {
    if (!done) return;
    const t = setTimeout(() => setDone(null), 2000);
    return () => clearTimeout(t);
  }, [done]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setDone('yes');
    } catch {
      setDone('failed');
    }
  };
  return (
    <button type="button" onClick={copy} className="kw-btn-ghost min-h-[44px] shrink-0 whitespace-nowrap px-2.5 text-[14px] sm:min-h-[32px] sm:px-3 sm:text-meta" data-action="copy-note">
      {done === 'yes' ? <Check size={14} aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
      <span aria-live="polite">{done === 'yes' ? 'Copied' : done === 'failed' ? "Couldn't copy" : label}</span>
    </button>
  );
};

const RsvpRows: React.FC<{ g: GatherInfo; nudges: number }> = ({ g, nudges }) => {
  const r = g.rsvp!;
  const maybe = g.nudges.filter((n) => n.why === 'maybe').length;
  const unpaid = g.nudges.filter((n) => n.why === 'not_paid').length;
  const parts = [maybe ? `${num(maybe)} said maybe` : null, unpaid ? `${num(unpaid)} not paid yet` : null, nudges - maybe - unpaid > 0 ? `${num(nudges - maybe - unpaid)} more` : null].filter(Boolean);
  return (
    <>
      <CountTiles r={r} />
      <ul className="flex flex-col gap-2">
        {r.food && (
          <Row icon={<Utensils size={15} aria-hidden="true" />}>
            Food needs: <strong className="font-semibold text-zinc-50">{r.food}</strong>
          </Row>
        )}
        {r.paid && <PaidSummary paid={r.paid} />}
        {g.closes_at && <Row icon={<Clock size={15} aria-hidden="true" />}>Closes {g.closes_at}. Members can change their answer until then.</Row>}
        {nudges > 0 && (
          <Row icon={<BellRing size={15} aria-hidden="true" />}>
            <strong className="font-semibold text-zinc-50 tabular-nums">
              {num(nudges)} {nudges === 1 ? 'nudge' : 'nudges'} drafted
            </strong>
            : {parts.join(', ')}. Nothing sends by itself.
          </Row>
        )}
        <KeptRow g={g} />
      </ul>
    </>
  );
};

const SignupRows: React.FC<{ g: GatherInfo }> = ({ g }) => {
  const m = g.moved_up;
  return (
    <>
      {m && (
        <ul className="flex flex-col gap-2" data-moved-up>
          <li className="flex items-start gap-2 text-[15px] leading-[22px] text-ok sm:text-[13px] sm:leading-5">
            <ArrowUp size={15} className={ICON} aria-hidden="true" />
            <span className="min-w-0 flex-1">
              A spot opened in {m.slot}{m.at ? ` at ${m.at}` : ''}. <strong className="font-semibold">{m.name} moved up</strong> from the waiting list on their own; {m.name}'s page already says so.
              {m.more > 0 ? ` ${num(m.more)} more moved up too; the slots below show who.` : ''}
            </span>
            <Sticker tone="ink" size="sm" ariaLabel={`New: ${m.name} moved up`}>
              New
            </Sticker>
          </li>
          {m.note && (
            <Row icon={<BellRing size={15} aria-hidden="true" />} aside={<CopyNote text={m.note} label="Copy note" />}>
              A note for {m.name} is ready, if you want to tell them yourself.
            </Row>
          )}
        </ul>
      )}
      <SlotList slots={g.slots} />
      <ul className="flex flex-col gap-2">
        {g.closes_at && <Row icon={<Clock size={15} aria-hidden="true" />}>Sign-ups close {g.closes_at}.</Row>}
        <KeptRow g={g} />
      </ul>
    </>
  );
};

const leaders = (counts: GatherCount[]) => {
  const top = Math.max(0, ...counts.map((c) => c.votes));
  return top > 0 ? counts.filter((c) => c.votes === top).map((c) => c.name) : [];
};

const VoteRows: React.FC<{ g: GatherInfo }> = ({ g }) => {
  const v = g.vote!;
  const total = v.counts.reduce((n, c) => n + c.votes, 0);
  if (g.phase === 'open')
    return (
      <>
        <ul className="flex flex-col gap-2">
          <Row icon={<Users size={15} aria-hidden="true" />} aside={<VoterStack voters={v.voters} />}>
            <strong className="font-semibold text-zinc-50 tabular-nums">
              {num(v.voted)}
              {v.members !== null ? ` of ${num(v.members)}` : ''}
            </strong>{' '}
            {v.members !== null ? 'members voted' : 'voted so far'}
          </Row>
          {v.members !== null && v.members > 0 && (
            <li className="list-none">
              <Bar value={v.voted / v.members} label={`${num(v.voted)} of ${num(v.members)} voted`} />
            </li>
          )}
        </ul>
        <div className="flex flex-col gap-2">
          <div className="text-[13px] font-semibold text-zinc-200 sm:text-2xs">Live count{v.kind === 'ranked' ? ' · 1st choices' : ''}</div>
          <VoteBars counts={v.counts} total={total} lead={leaders(v.counts)} />
        </div>
        <ul className="flex flex-col gap-2">
          {v.kind === 'ranked' && (
            <Row icon={<ListOrdered size={15} aria-hidden="true" />}>
              Ranked: if no place has more than half, the last one drops out and its votes move to each voter's next choice. Final count at close.
            </Row>
          )}
          <Row icon={<EyeOff size={15} aria-hidden="true" />}>You see who voted, never what they picked.{g.show_names ? ' Members see the live count after they vote.' : ''}</Row>
          {g.closes_at && <Row icon={<Clock size={15} aria-hidden="true" />}>Closes {g.closes_at}.</Row>}
          <KeptRow g={g} />
        </ul>
      </>
    );
  if (g.phase === 'tie') {
    const tied = v.tied.map(({ name, votes }) => ({ name, votes }));
    return (
      <>
        <ul className="flex flex-col gap-2">
          <Row tone="warn" icon={<AlertTriangle size={15} aria-hidden="true" />}>
            {list(tied.map((t) => t.name)).replace(/, ([^,]*)$/, ' and $1')} have <strong className="font-semibold text-zinc-50 tabular-nums">{tied[0]?.votes ?? 0} each</strong>
            {v.kind === 'ranked' && v.rounds.length > 1 ? ' in the final round' : ''}.
          </Row>
        </ul>
        <VoteBars counts={tied} total={tied.reduce((n, c) => n + c.votes, 0)} lead={tied.map((t) => t.name)} />
        <ul className="flex flex-col gap-2">
          <Row icon={<Info size={15} aria-hidden="true" />}>Members see "Tie: {g.host_name ?? 'the host'} picks" until you choose. No winner is shown yet.</Row>
        </ul>
      </>
    );
  }
  const winner = v.winner;
  const final = v.counts.filter((c) => c.votes > 0);
  const won = v.counts.find((c) => c.name === winner)?.votes ?? 0;
  return (
    <>
      <ul className="flex flex-col gap-2">
        {winner ? (
          <li className="flex flex-wrap items-center gap-2 text-[15px] leading-[22px] text-ok sm:text-[13px] sm:leading-5" data-winner>
            <Trophy size={15} className="shrink-0" aria-hidden="true" />
            <span className="min-w-0">
              <strong className="font-semibold">{winner}</strong> {v.outcome === 'picked' ? 'is your pick after a tie at' : 'wins with'}{' '}
              <strong className="font-semibold tabular-nums">
                {won} of {total}
              </strong>
              {v.outcome !== 'picked' && v.kind === 'ranked' && v.rounds.length > 1 ? ' in the final round' : ''}
            </span>
            {earnedPicked(g) && (
              <Sticker tone="holo" size="sm" ariaLabel={`Picked: ${winner} won the count`}>
                Picked
              </Sticker>
            )}
          </li>
        ) : (
          <Row icon={<Info size={15} aria-hidden="true" />}>The vote is closed. {num(v.voted)} voted; no winner was counted.</Row>
        )}
      </ul>
      {final.length > 0 && <VoteBars counts={final} total={total} lead={winner ? [winner] : []} />}
      <Rounds rounds={v.rounds} />
      <ul className="flex flex-col gap-2">
        <Row icon={<Lock size={15} aria-hidden="true" />}>Ballots are deleted on {g.deletes_on}. The result stays in Done.</Row>
      </ul>
    </>
  );
};

// ---------------------------------------------------------------------------------------------------
// Results (closed RSVP and Sign-up, design §4)

const TABLE = 'w-full border-collapse text-left text-[15px] sm:text-[13px]';
const TH = 'border-b border-hairline bg-surface-rail/60 px-3 py-2 text-2xs font-semibold text-zinc-400';
const TD = 'border-b border-hairline px-3 py-2 align-top text-zinc-200';

const RsvpResult: React.FC<{ g: GatherInfo }> = ({ g }) => {
  const r = g.rsvp!;
  const yes = g.people.filter((p) => p.answer === 'yes');
  const guests = r.plus_ones;
  return (
    <>
      <ul className="flex flex-col gap-2">
        <li className="flex items-start gap-2 text-[15px] leading-[22px] text-ok sm:text-[13px] sm:leading-5">
          <CheckCircle2 size={15} className={ICON} aria-hidden="true" />
          <span>
            <strong className="font-semibold tabular-nums">{num(r.coming)} coming</strong>
            {guests ? ` (${num(r.yes)} members + ${num(guests)} plus-ones)` : ''} · {num(r.no)} no{r.never !== null ? ` · ${num(r.never)} never answered` : ''}
          </span>
        </li>
        {r.food && (
          <Row icon={<Utensils size={15} aria-hidden="true" />}>
            Tell {g.place ?? 'the place'}: <strong className="font-semibold text-zinc-50">{r.food}</strong>
          </Row>
        )}
        {r.paid && <PaidSummary paid={r.paid} bar={false} />}
      </ul>
      {yes.length > 0 && (
        <div className="overflow-hidden rounded-xl border border-hairline-strong">
          <table className={TABLE} data-result="rsvp">
            <thead>
              <tr>
                <th className={TH}>Coming</th>
                <th className={TH}>Food</th>
                <th className={TH}>
                  Paid <span className="sr-only">(only you see this)</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {yes.map((p) => (
                <tr key={p.id} className="last:[&>td]:border-b-0">
                  <td className={TD}>
                    <span className="flex items-center gap-2">
                      <GuestCreature avatar={p.avatar} size={20} />
                      {p.name}
                      {p.plus_ones ? <span className="text-zinc-400">+{p.plus_ones}</span> : null}
                    </span>
                  </td>
                  <td className={TD}>{p.food ?? <span className="text-zinc-500">none</span>}</td>
                  <td className={TD}>{p.paid ? <span className="kw-chip kw-chip-ok">Paid</span> : <span className="kw-chip">Not yet</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
};

/** "Every slot is full except Washing-up crew (2 of 3). Leo stayed on the Desserts waiting list." */
export function signupSummary(g: GatherInfo): string {
  const open = g.slots.filter((s) => s.who.length < s.cap);
  const head =
    open.length === 0
      ? 'Every slot is full.'
      : open.length === g.slots.length
        ? `No slot is full yet.`
        : `Every slot is full except ${open.map((s) => `${s.name} (${s.who.length} of ${s.cap})`).join(', ')}.`;
  const waiting = g.slots.filter((s) => s.waiting_count).map((s) => (s.waiting.length ? `${list(s.waiting)} stayed on the ${s.name} waiting list.` : `${people(s.waiting_count)} stayed on the ${s.name} waiting list.`));
  return [head, ...waiting].join(' ');
}

const SignupResult: React.FC<{ g: GatherInfo }> = ({ g }) => (
  <>
    <ul className="flex flex-col gap-2">
      <li className="flex items-start gap-2 text-[15px] leading-[22px] text-ok sm:text-[13px] sm:leading-5">
        <CheckCircle2 size={15} className={ICON} aria-hidden="true" />
        <span>{signupSummary(g)}</span>
      </li>
    </ul>
    <div className="overflow-hidden rounded-xl border border-hairline-strong">
      <table className={TABLE} data-result="signup">
        <thead>
          <tr>
            <th className={TH}>Slot</th>
            <th className={TH}>Who brings what</th>
            <th className={TH}>
              <span className="sr-only">Filled</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {g.slots.map((s) => (
            <tr key={s.id} className="last:[&>td]:border-b-0">
              <td className={`${TD} font-semibold text-zinc-100`}>{s.name}</td>
              <td className={TD}>{s.who.length ? s.who.map((w) => (w.note ? `${w.name} (${w.note})` : w.name)).join(', ') : <span className="text-zinc-500">nobody</span>}</td>
              <td className={`${TD} whitespace-nowrap tabular-nums`}>
                {s.who.length} of {s.cap}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  </>
);

// ---------------------------------------------------------------------------------------------------
// Confirm, actions

type ConfirmKind = 'stop' | 'close';
const CONFIRM: Record<ConfirmKind, { q: (g: GatherInfo) => string; text: (g: GatherInfo) => string; yes: string; no: string }> = {
  stop: {
    q: (g) => `Stop ${g.title}?`,
    text: (g) => `Members can't answer any more and the link stops working. What's in stays until ${g.deletes_on}, then it's deleted.`,
    yes: 'Stop it',
    no: 'Keep it open',
  },
  close: {
    q: () => 'Close the vote now?',
    text: (g) => `Voting ends now and the final count runs. It would close by itself ${g.closes_at}.`,
    yes: 'Close and count',
    no: 'Keep voting open',
  },
};

const Confirm: React.FC<{ g: GatherInfo; kind: ConfirmKind; busy: boolean; onYes: () => void; onNo: () => void }> = ({ g, kind, busy, onYes, onNo }) => {
  const c = CONFIRM[kind];
  const no = useRef<HTMLButtonElement | null>(null);
  useEffect(() => no.current?.focus(), []);
  return (
    <div role="alertdialog" aria-label={c.q(g)} aria-describedby={`gather-confirm-${kind}`} onKeyDown={(e) => e.key === 'Escape' && onNo()} className="flex flex-col gap-3 rounded-xl border border-hairline-strong bg-surface-rail/60 p-3.5" data-confirm={kind}>
      <p className="text-[15px] font-semibold text-zinc-50 sm:text-body">{c.q(g)}</p>
      <p id={`gather-confirm-${kind}`} className="text-[15px] leading-[22px] text-zinc-300 sm:text-[13px] sm:leading-5">
        {c.text(g)}
      </p>
      <div className="grid grid-cols-2 gap-2 sm:flex">
        <button type="button" disabled={busy} onClick={onYes} className={kind === 'stop' ? `${STOP} border border-rose-300/30` : PRIMARY} data-action={`confirm-${kind}`}>
          {busy ? 'Working…' : c.yes}
        </button>
        <button type="button" ref={no} onClick={onNo} className={GHOST}>
          {c.no}
        </button>
      </div>
    </div>
  );
};

type Panel = 'people' | 'nudges' | null;
type Done = { kind: 'ok' | 'error'; text: string; final: boolean } | null;

const GRID = 'grid grid-cols-2 gap-2 sm:flex sm:flex-wrap sm:items-center';
const FULL = 'col-span-2 sm:col-span-1';
const HALF_BTN = `kw-btn-ghost ${HALF} aria-expanded:border-sky-600 aria-expanded:bg-sky-500/10 aria-expanded:text-zinc-50`;

const ShareLink: React.FC<{ g: GatherInfo; name: string }> = ({ g, name }) => {
  const url = g.join?.anywhere_url ?? g.join?.wifi_url ?? null;
  const [state, setState] = useState<'copied' | 'failed' | null>(null);
  useEffect(() => {
    if (!state) return;
    const t = setTimeout(() => setState(null), 2000);
    return () => clearTimeout(t);
  }, [state]);
  if (!url)
    return (
      <button type="button" aria-disabled="true" title={GATHER_LATER.noLink} aria-description={GATHER_LATER.noLink} onClick={(e) => e.preventDefault()} className={`${PRIMARY} ${FULL} ${LATER_CLS}`}>
        <Share2 size={14} aria-hidden="true" />
        Share link
      </button>
    );
  const go = async () => {
    if (canShare()) {
      try {
        await navigator.share({ title: name, url });
        return;
      } catch (e) {
        if ((e as Error)?.name === 'AbortError') return;
      }
    }
    try {
      await navigator.clipboard.writeText(url);
      setState('copied');
    } catch {
      setState('failed');
    }
  };
  return (
    <button type="button" onClick={go} className={`${PRIMARY} ${FULL}`} data-action="share-link">
      {state === 'copied' ? <Check size={14} aria-hidden="true" /> : <Share2 size={14} aria-hidden="true" />}
      <span aria-live="polite">{state === 'copied' ? 'Link copied' : state === 'failed' ? "Couldn't copy" : 'Share link'}</span>
    </button>
  );
};

const CopyButton: React.FC<{ text: string; label: string; done: string; primary?: boolean; icon?: React.ReactNode; action: string; onCopied?: () => void }> = ({ text, label, done, primary, icon, action, onCopied }) => {
  const [state, setState] = useState<'yes' | 'failed' | null>(null);
  useEffect(() => {
    if (!state) return;
    const t = setTimeout(() => setState(null), 2000);
    return () => clearTimeout(t);
  }, [state]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState('yes');
      onCopied?.();
    } catch {
      setState('failed');
    }
  };
  return (
    <button type="button" onClick={copy} className={primary ? `${PRIMARY} ${FULL}` : HALF_BTN} data-action={action}>
      {state === 'yes' ? <Check size={14} aria-hidden="true" /> : (icon ?? <Copy size={14} aria-hidden="true" />)}
      <span aria-live="polite">{state === 'yes' ? done : state === 'failed' ? "Couldn't copy" : label}</span>
    </button>
  );
};

const GatherActions: React.FC<{
  block: PocketBlock;
  g: GatherInfo;
  panel: Panel;
  setPanel: (p: Panel) => void;
  ask: (t: string) => void;
  onConfirm: (k: ConfirmKind) => void;
  onPick: (t: { option: number; name: string }) => void;
  onRevote: () => void;
  busy: boolean;
}> = ({ block, g, panel, setPanel, ask, onConfirm, onPick, onRevote, busy }) => {
  const stopCls = `${STOP} col-span-2 justify-self-center sm:ml-auto`;
  const toggle = (p: Exclude<Panel, null>) => setPanel(panel === p ? null : p);
  const stop = (
    <button type="button" onClick={() => onConfirm('stop')} className={stopCls} data-action="stop">
      <Square size={13} aria-hidden="true" />
      Stop
    </button>
  );
  const later = (icon: React.ReactNode, why: string, label: string) => (
    <LaterButton icon={icon} why={why} half>
      {label}
    </LaterButton>
  );
  if (g.face === 'vote' && g.phase === 'tie')
    return (
      <div className={GRID}>
        {g.vote!.tied.map((t) => (
          <button key={t.option} type="button" disabled={busy} onClick={() => onPick(t)} className={`${PRIMARY} ${FULL}`} data-action="pick">
            Pick {t.name}
          </button>
        ))}
        <button type="button" disabled={busy} onClick={onRevote} className={`${GHOST} ${FULL} sm:ml-auto`} data-action="revote">
          <RotateCcw size={14} aria-hidden="true" />
          Vote again for 24 h
        </button>
      </div>
    );
  if (g.phase === 'closed') {
    if (g.face === 'vote')
      return (
        <div className={GRID}>
          {g.vote!.winner ? (
            <button type="button" onClick={() => ask(`Make the dinner RSVP for ${g.vote!.winner}.`)} className={`${PRIMARY} ${FULL}`} data-action="make-rsvp">
              <CalendarCheck size={14} aria-hidden="true" />
              Make the dinner RSVP
            </button>
          ) : null}
          <CopyButton text={resultLine(g)} label="Share the result" done="Result copied" icon={<Share2 size={14} aria-hidden="true" />} action="share-result" />
          {later(<Trash2 size={14} aria-hidden="true" />, GATHER_LATER.deleteBallots, 'Delete ballots now')}
        </div>
      );
    if (g.face === 'rsvp') {
      const unpaid = g.nudges.find((n) => n.why === 'not_paid' && !n.until);
      return (
        <div className={GRID}>
          <CopyButton primary text={restaurantList(g)} label="Copy list for the restaurant" done="List copied" action="copy-list" />
          {unpaid && <CopyButton text={unpaid.text} label={`Nudge ${unpaid.name}`} done="Copied, not sent" icon={<BellRing size={14} aria-hidden="true" />} action="nudge-one" onCopied={() => void pocketApi.copied(block.code, unpaid.name).catch(() => undefined)} />}
          {later(<Trash2 size={14} aria-hidden="true" />, GATHER_LATER.deleteAnswers, 'Delete answers now')}
        </div>
      );
    }
    return (
      <div className={GRID}>
        <CopyButton primary text={whoBringsWhat(g)} label="Copy who brings what" done="Copied" action="copy-who" />
        <button type="button" onClick={() => ask(`Run ${g.title} again.`)} className={HALF_BTN} data-action="run-again">
          <RotateCcw size={14} aria-hidden="true" />
          Run it again
        </button>
      </div>
    );
  }
  if (g.face === 'rsvp')
    return (
      <div className={GRID}>
        <ShareLink g={g} name={block.name} />
        <button type="button" onClick={() => toggle('nudges')} aria-expanded={panel === 'nudges'} className={HALF_BTN} data-action="nudges">
          <BellRing size={14} aria-hidden="true" />
          Nudges · {num(g.nudges.length)}
        </button>
        <button type="button" onClick={() => toggle('people')} aria-expanded={panel === 'people'} className={HALF_BTN} data-action="people">
          <Users size={14} aria-hidden="true" />
          People and paid
        </button>
        {later(<Pencil size={14} aria-hidden="true" />, GATHER_LATER.edit, 'Edit')}
        {stop}
      </div>
    );
  if (g.face === 'signup')
    return (
      <div className={GRID}>
        <ShareLink g={g} name={block.name} />
        {later(<Pencil size={14} aria-hidden="true" />, GATHER_LATER.editSlots, 'Edit slots')}
        {g.nudges.length > 0 ? (
          <button type="button" onClick={() => toggle('nudges')} aria-expanded={panel === 'nudges'} className={HALF_BTN} data-action="nudges">
            <BellRing size={14} aria-hidden="true" />
            Nudges · {num(g.nudges.length)}
          </button>
        ) : (
          later(<Users size={14} aria-hidden="true" />, GATHER_LATER.signupPeople, 'People')
        )}
        {stop}
      </div>
    );
  return (
    <div className={GRID}>
      <ShareLink g={g} name={block.name} />
      {later(<Pencil size={14} aria-hidden="true" />, GATHER_LATER.edit, 'Edit')}
      <button type="button" onClick={() => onConfirm('close')} className={HALF_BTN} data-action="close">
        <Timer size={14} aria-hidden="true" />
        Close now
      </button>
      {g.nudges.length > 0 && (
        <button type="button" onClick={() => toggle('nudges')} aria-expanded={panel === 'nudges'} className={HALF_BTN} data-action="nudges">
          <BellRing size={14} aria-hidden="true" />
          Nudges · {num(g.nudges.length)}
        </button>
      )}
      {stop}
    </div>
  );
};

// ---------------------------------------------------------------------------------------------------
// The card

export const GatherCard: React.FC<{ block: PocketBlock; ask: (t: string) => void }> = ({ block, ask }) => {
  const g = block.gather!;
  const [panel, setPanel] = useState<Panel>(null);
  const [confirm, setConfirm] = useState<ConfirmKind | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<Done>(null);
  const opener = useRef<HTMLDivElement | null>(null);
  const focusAfter = useFocusAfterRender();
  const run = async (what: () => Promise<unknown>, ok: string, final: boolean) => {
    setBusy(true);
    try {
      await what();
      setDone({ kind: 'ok', text: ok, final });
      setConfirm(null);
    } catch (e) {
      setDone({ kind: 'error', text: `Couldn't do that: ${e instanceof PocketApiError ? e.message : 'something went wrong.'} Try again.`, final: false });
    } finally {
      setBusy(false);
    }
  };
  const act = (k: ConfirmKind) =>
    k === 'close'
      ? run(() => pocketApi.closeNow(block.code), 'Voting is closed. The final count shows here in a moment.', true)
      : run(() => pocketApi.end(block.code), `${g.title} is stopped. Members see that it's closed.`, true);
  const pick = (t: { option: number; name: string }) => run(() => pocketApi.pick(block.code, t.option), `You picked ${t.name}. Members see it now.`, true);
  const revote = () => run(() => pocketApi.revote24h(block.code), 'Voting is open again for 24 hours. Members can change their ranking.', true);
  const close = () => {
    const k = confirm;
    setConfirm(null);
    focusAfter(() => opener.current?.querySelector<HTMLButtonElement>(`[data-action="${k}"]`));
  };
  const open = g.phase === 'open';
  const stopped = done?.kind === 'ok' && done.final;
  const showsPanel = open && !stopped ? panel : null;
  // State 2 and 3 swap the join box for their panel; state 6 has no join box (the link is already out).
  const joinFull = g.face === 'rsvp' || g.join?.mode !== 'anywhere';
  const join = !open || stopped || showsPanel || g.moved_up ? null : joinFull ? <GatherJoinBox g={g} name={block.name} /> : <GatherCompactJoin g={g} name={block.name} />;
  const results = g.phase === 'closed' && g.face !== 'vote';
  return (
    <section
      className="pocket-card relative flex max-w-[680px] flex-col gap-4 overflow-hidden rounded-2xl border border-sky-800 bg-surface-raised p-4 shadow-card sm:p-[18px]"
      data-pocket={block.state}
      data-gather={`${g.face}-${g.phase}`}
      data-mood={gatherMood(g, panel === 'nudges')}
      aria-label={`${block.name}, pocket app ${FACE_LABEL[g.face]}`}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-sky-500/[0.07] to-transparent" aria-hidden="true" />
      <header className="relative flex items-start gap-3">
        <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-sky-700 bg-sky-950 text-sky-200" aria-hidden="true">
          <GatherGlyph face={g.face} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-2xs font-semibold text-sky-300">
            Pocket app · {FACE_LABEL[g.face]}
          </div>
          <h3 className="text-[20px] font-bold leading-[26px] text-zinc-50 sm:text-[19px]">{g.title}</h3>
          {(whereLine(g) || block.when) && <p className="text-[15px] text-zinc-300 sm:text-[13px]">{whereLine(g) || block.when}</p>}
          <span className="mt-1.5 block sm:hidden">
            <GatherChip g={g} />
          </span>
        </div>
        <span className="hidden shrink-0 sm:block">
          <GatherChip g={g} />
        </span>
      </header>
      <div className="relative flex flex-col gap-4">
        {g.face === 'rsvp' && (results ? <RsvpResult g={g} /> : <RsvpRows g={g} nudges={g.nudges.length} />)}
        {g.face === 'signup' && (results ? <SignupResult g={g} /> : <SignupRows g={g} />)}
        {g.face === 'vote' && <VoteRows g={g} />}
        {showsPanel === 'people' && <PeoplePaidPanel block={block} g={g} />}
        {showsPanel === 'nudges' && <NudgesPanel block={block} g={g} />}
        {join}
        {done ? (
          <p role="status" className={`flex items-start gap-2 text-[15px] sm:text-[13px] ${done.kind === 'ok' ? 'text-ok' : 'text-bad'}`} data-done={done.kind}>
            {done.kind === 'ok' ? <Check size={15} className={ICON} aria-hidden="true" /> : <AlertTriangle size={15} className={ICON} aria-hidden="true" />}
            {done.text}
          </p>
        ) : null}
        {confirm ? (
          <Confirm g={g} kind={confirm} busy={busy} onYes={() => act(confirm)} onNo={close} />
        ) : stopped ? null : (
          <div ref={opener}>
            <GatherActions block={block} g={g} panel={panel} setPanel={setPanel} ask={ask} onConfirm={setConfirm} onPick={pick} onRevote={revote} busy={busy} />
          </div>
        )}
      </div>
      {!results && (
        <footer className="relative -mx-4 -mb-4 flex items-start gap-2 border-t border-hairline px-4 py-3 text-meta text-zinc-400 sm:-mx-[18px] sm:-mb-[18px] sm:px-[18px]" data-budget>
          <Zap size={13} className="mt-0.5 shrink-0" aria-hidden="true" />
          <span>{gatherBudgetLine(block)}</span>
        </footer>
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------------------------------
// Inbox · Done rows (design §4): one per pocket, at most one sticker, holo only on a vote with a winner

const DONE_TITLE: Record<GatherFace, string> = { rsvp: 'RSVP closed', signup: 'Sign-up closed', vote: 'Vote closed' };

export const GatherDoneRow: React.FC<{ g: GatherInfo; onOpen?: () => void }> = ({ g, onOpen }) => (
  <li className="grid grid-cols-[28px_minmax(0,1fr)_auto] items-center gap-2.5 border-b border-hairline px-3.5 py-2.5 last:border-b-0" data-done-row={g.face}>
    <ModernAvatar role="tally" name="Tally" size={28} modelBadge={false} mood={gatherMood(g)} moodText={`Tally: ${DONE_TITLE[g.face].toLowerCase()}`} />
    <div className="min-w-0">
      <div className="truncate text-[15px] font-semibold text-zinc-100 sm:text-[13.5px]">
        {g.title} · {DONE_TITLE[g.face]}
      </div>
      <div className="truncate text-[13px] text-zinc-400 sm:text-2xs">{gatherDoneLine(g)}</div>
    </div>
    {earnedPicked(g) ? (
      <Sticker tone="holo" size="sm" ariaLabel={`Picked: ${g.vote!.winner}`}>
        Picked
      </Sticker>
    ) : onOpen ? (
      <button type="button" onClick={onOpen} className="min-h-[44px] rounded-lg px-2 text-[13px] font-medium text-zinc-300 hover:bg-white/[0.04] kw-focus sm:min-h-[32px] sm:text-2xs">
        Open
      </button>
    ) : (
      <span />
    )}
  </li>
);

export const GatherDoneList: React.FC<{ items: GatherInfo[] }> = ({ items }) => (
  <section className="overflow-hidden rounded-2xl border border-hairline-strong bg-surface-raised" aria-label="Done" data-done-list>
    <h4 className="border-b border-hairline px-3.5 py-2.5 text-2xs font-bold text-zinc-300">Inbox · Done</h4>
    <ul>
      {items.map((g) => (
        <GatherDoneRow key={`${g.face}-${g.title}`} g={g} onOpen={() => undefined} />
      ))}
    </ul>
  </section>
);

export { Callout };
