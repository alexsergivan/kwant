// Idea jar (0.7, Iris): the person drops ideas in; a free bot peeks in with no tools and writes a take
// (verdict, why, effort, risks, plan, questions, who should do it). Do it makes a DRAFT mission.
// Spec: /home/maus/shared/ship-software-safely/idea-jar/spec.md. API: idea-jar/impl/juno.md (PR #320).
// Idea text and takes are untrusted data: shown as plain text only, never as Markdown or HTML.

import { useEffect, useState } from 'react';
import { PersonCancelledError, personFetch } from './auth';
import { relativeTime } from './away';

export type IdeaStatus = 'new' | 'peeking' | 'ready' | 'picked' | 'later' | 'failed';
export type IdeaVerdict = 'worth_it' | 'maybe' | 'not_now';
export type IdeaEffort = 'S' | 'M' | 'L';
export const IDEA_STATUSES: readonly IdeaStatus[] = ['new', 'peeking', 'ready', 'picked', 'later', 'failed'];
export const IDEA_MAX = 2000;

export interface IdeaPlanStep {
  title: string;
  done_when: string;
}

export interface IdeaTake {
  title: string;
  verdict: IdeaVerdict;
  why: string;
  effort: IdeaEffort;
  risks: string[];
  plan: IdeaPlanStep[];
  questions: string[];
  /** Who should do it: a name, and its id (null = no suggestion). */
  bot: string | null;
  bot_id: string | null;
}

export interface Idea {
  id: string;
  space_id: string;
  text: string;
  status: IdeaStatus;
  /** While peeking: the bot peeking. Once the take is written: its author. */
  kwant_id: string | null;
  take: IdeaTake | null;
  /** The draft mission Do it made. */
  mission_id: string | null;
  /** Why the last take failed. For logs only: never shown to the person. */
  error: string | null;
  created_at: string;
  updated_at: string;
  peeked_at: string | null;
}

export type IdeaCounts = Record<IdeaStatus, number>;

export interface IdeaList {
  space_id: string;
  ideas: Idea[];
  counts: IdeaCounts;
  /** 0 = bots don't peek on their own. */
  daily_cap: number;
  /** Every take started today (scan and Look now). Missing on older builds. */
  peeks_today: number | null;
  /** Scan takes have hit the cap (or the cap is 0, or a turn went over the token cap today). */
  cap_reached: boolean;
  /** Scan peeks left today, if the server says so (preferred over working it out). */
  peeks_left: number | null;
}

export interface LookResult {
  idea_id: string;
  status: 'peeking';
  kwant_id: string | null;
  kwant_name: string | null;
}

export interface PickResult {
  idea_id: string;
  mission_id: string;
  mission_status: string | null;
  tasks: number | null;
}

// ---------------------------------------------------------------------------
// Parsing (defensive: a bad field never breaks the jar)

const obj = (v: unknown): Record<string, unknown> | null => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const str = (v: unknown): string | null => (typeof v === 'string' ? v : null);
const strs = (v: unknown, max = 5): string[] =>
  Array.isArray(v) ? v.map((x) => (typeof x === 'string' ? x : obj(x) && typeof obj(x)!.text === 'string' ? (obj(x)!.text as string) : null)).filter((x): x is string => !!x && !!x.trim()).slice(0, max) : [];
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

export function parseTake(raw: unknown): IdeaTake | null {
  const t = obj(raw);
  if (!t) return null;
  const plan: IdeaPlanStep[] = Array.isArray(t.plan)
    ? t.plan
        .map((s): IdeaPlanStep | null => {
          if (typeof s === 'string') return s.trim() ? { title: s, done_when: '' } : null;
          const o = obj(s);
          const title = o && str(o.title);
          return title && title.trim() ? { title, done_when: str(o!.done_when) ?? '' } : null;
        })
        .filter((s): s is IdeaPlanStep => !!s)
        .slice(0, 6)
    : [];
  const verdict = (['worth_it', 'maybe', 'not_now'] as const).find((v) => v === t.verdict);
  const effort = (['S', 'M', 'L'] as const).find((v) => v === t.effort);
  if (!verdict || !effort) return null;
  return {
    title: str(t.title) ?? '',
    verdict,
    why: str(t.why) ?? '',
    effort,
    risks: strs(t.risks),
    plan,
    questions: strs(t.questions),
    bot: str(t.bot) || null,
    bot_id: str(t.bot_id) || null,
  };
}

export function parseIdea(raw: unknown): Idea | null {
  const o = obj(raw);
  const id = o && str(o.id);
  if (!o || !id) return null;
  const status = IDEA_STATUSES.find((s) => s === o.status) ?? 'new';
  const take = parseTake(o.take);
  return {
    id,
    space_id: str(o.space_id) ?? '',
    text: str(o.text) ?? '',
    // A ready idea whose take didn't parse can't be acted on: treat it as still in the jar.
    status: status === 'ready' && !take ? 'new' : status,
    kwant_id: str(o.kwant_id) || null,
    take,
    mission_id: str(o.mission_id) || null,
    error: str(o.error),
    created_at: str(o.created_at) ?? '',
    updated_at: str(o.updated_at) ?? str(o.created_at) ?? '',
    peeked_at: str(o.peeked_at) || null,
  };
}

export function parseIdeaList(raw: unknown): IdeaList {
  const o = obj(raw) ?? {};
  const ideas = Array.isArray(o.ideas) ? o.ideas.map(parseIdea).filter((i): i is Idea => !!i) : [];
  return {
    space_id: str(o.space_id) ?? '',
    ideas,
    counts: countIdeas(ideas),
    daily_cap: Math.max(0, num(o.daily_cap) ?? 3),
    peeks_today: num(o.peeks_today),
    cap_reached: o.cap_reached === true,
    peeks_left: num(o.peeks_left),
  };
}

/** Counted from the ideas themselves, so the sections and the badge always agree. */
export function countIdeas(ideas: Idea[]): IdeaCounts {
  const c: IdeaCounts = { new: 0, peeking: 0, ready: 0, picked: 0, later: 0, failed: 0 };
  for (const i of ideas) c[i.status] += 1;
  return c;
}

// ---------------------------------------------------------------------------
// Sections

export type SectionKey = 'ready' | 'peeking' | 'jar' | 'later' | 'picked';
export const SECTION_ORDER: readonly SectionKey[] = ['ready', 'peeking', 'jar', 'later', 'picked'];
export const SECTION_TITLE: Record<SectionKey, string> = {
  ready: 'Takes ready',
  peeking: 'Being peeked at',
  jar: 'In the jar',
  later: 'Later',
  picked: 'Picked',
};

export const sectionOf = (i: Idea): SectionKey =>
  i.status === 'ready' ? 'ready' : i.status === 'peeking' ? 'peeking' : i.status === 'later' ? 'later' : i.status === 'picked' ? 'picked' : 'jar';

const t = (s: string) => {
  const v = Date.parse(s);
  return Number.isNaN(v) ? 0 : v;
};

/** Ideas grouped in display order. Ready, Later and Picked newest first; the jar oldest first (the order bots take them). */
export function groupIdeas(ideas: Idea[]): Record<SectionKey, Idea[]> {
  const g: Record<SectionKey, Idea[]> = { ready: [], peeking: [], jar: [], later: [], picked: [] };
  for (const i of ideas) g[sectionOf(i)].push(i);
  const newest = (a: Idea, b: Idea) => t(b.updated_at) - t(a.updated_at);
  g.ready.sort(newest);
  g.peeking.sort((a, b) => t(a.peeked_at ?? a.updated_at) - t(b.peeked_at ?? b.updated_at));
  g.jar.sort((a, b) => t(a.created_at) - t(b.created_at));
  g.later.sort(newest);
  g.picked.sort(newest);
  return g;
}

// ---------------------------------------------------------------------------
// Words

export const VERDICT_WORD: Record<IdeaVerdict, string> = { worth_it: 'Worth it', maybe: 'Maybe', not_now: 'Not now' };
export const EFFORT_WORD: Record<IdeaEffort, string> = { S: 'small', M: 'medium', L: 'large' };
export const EFFORT_PIPS: Record<IdeaEffort, number> = { S: 1, M: 2, L: 3 };

/** "/idea" at the very start, then a space or the end. Display only: the server decides what is an idea. */
export const isIdeaCommand = (text: string): boolean => /^\/idea(\s|$)/i.test(text.trimStart());

/** Short, for "1 min" / "2 h" with no "ago". */
export function since(ts: string | null, now: number = Date.now()): string {
  if (!ts) return '';
  const r = relativeTime(ts, now);
  return r === 'just now' ? 'just now' : r.replace(/ ago$/, '');
}

export function peekingText(name: string | null, peekedAt: string | null, now: number = Date.now()): string {
  const who = name ? `${name} is peeking` : 'A bot is peeking';
  const s = since(peekedAt, now);
  return s ? `${who} · ${s}` : who;
}

export const failedLine = (name: string | null): string =>
  name ? `${name}’s take came back garbled twice. Nothing was changed.` : 'The take came back garbled twice. Nothing was changed.';

/** The 409 copy for Look now. The idea stays where it is. Never the raw `error` code. */
export function lookRefusalText(code: string | null, name: string | null, message: string | null): string {
  if (code === 'no_free_bot') return 'No bot is free right now. Try again in a few minutes.';
  if (code === 'already_peeking') return name ? `${name} is already peeking at this one.` : 'A bot is already peeking at this one.';
  if (code === 'picked') return 'This idea is already a mission.';
  return message && message.trim() ? sentence(message) : 'Couldn’t ask a bot just now. Try again.';
}

export const doItHint = (bot: string | null): string =>
  bot ? `for ${bot}. Nothing starts until you approve it.` : '. Nothing starts until you approve it.';

export const readOnlyLine = (name: string | null): string =>
  `Read only: ${name ?? 'the bot'} used no tools and changed nothing.`;

/**
 * The header chip, "2 of 3 peeks left today". The server's `peeks_left` when it sends one; else 0 once
 * `cap_reached`; else `daily_cap − peeks_today`, never below 1 while the cap isn't reached (Look now
 * takes count in `peeks_today` but not toward the cap). Null when the cap is 0 or the counts are missing.
 */
export function peeksLeft(list: Pick<IdeaList, 'daily_cap' | 'peeks_today' | 'cap_reached' | 'peeks_left'>): number | null {
  if (list.daily_cap <= 0) return null;
  if (list.peeks_left !== null) return Math.max(0, Math.min(list.daily_cap, list.peeks_left));
  if (list.cap_reached) return 0;
  if (list.peeks_today === null) return null;
  return Math.max(1, Math.min(list.daily_cap, list.daily_cap - list.peeks_today));
}

export type JarNotice = { kind: 'off' } | { kind: 'cap'; cap: number } | null;

export function jarNotice(list: Pick<IdeaList, 'daily_cap' | 'cap_reached'>): JarNotice {
  if (list.daily_cap <= 0) return { kind: 'off' };
  if (list.cap_reached) return { kind: 'cap', cap: list.daily_cap };
  return null;
}

/** For the live region: cut to 60 characters on a word where it can. */
export function clip(text: string, max = 60): string {
  const s = text.replace(/\s+/g, ' ').trim();
  if (s.length <= max) return s;
  const cut = s.slice(0, max);
  const sp = cut.lastIndexOf(' ');
  return `${(sp > max * 0.6 ? cut.slice(0, sp) : cut).trimEnd()}…`;
}

export function takeArrivedText(name: string | null, take: IdeaTake, text: string): string {
  return `${name ? `${name}’s take` : 'A take'} is ready: ${VERDICT_WORD[take.verdict]}, effort ${EFFORT_WORD[take.effort]}. ${clip(take.title || text)}.`;
}

const sentence = (s: string) => {
  const v = s.trim();
  if (!v) return v;
  const c = v[0].toUpperCase() + v.slice(1);
  return /[.!?]$/.test(c) ? c : `${c}.`;
};

// ---------------------------------------------------------------------------
// API (all person-only)

export class IdeaError extends Error {
  constructor(
    message: string,
    public status: number,
    public code: string | null = null,
  ) {
    super(message);
  }
}

async function call<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new IdeaError(e.message, 401);
    throw new IdeaError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  if (res.status === 204) return undefined as T;
  const answer = await res.json().catch(() => null);
  if (res.ok) return answer as T;
  const a = obj(answer) ?? {};
  const code = str(a.error);
  const message = str(a.message);
  if (res.status === 403) throw new IdeaError('Only you can change the idea jar.', 403, code);
  throw new IdeaError(message ? sentence(message) : `Something went wrong (HTTP ${res.status}).`, res.status, code);
}

const enc = encodeURIComponent;
const ideaOf = (r: unknown): Idea => {
  const i = parseIdea(obj(r)?.idea);
  if (!i) throw new IdeaError('The server sent back something unexpected. Try again.', 0);
  return i;
};

export const ideasApi = {
  list: (spaceId: string) => call<unknown>(`/api/ideas?space_id=${enc(spaceId)}`).then(parseIdeaList),
  create: (spaceId: string, text: string) => call<unknown>('/api/ideas', 'POST', { space_id: spaceId, text }).then(ideaOf),
  edit: (id: string, text: string) => call<unknown>(`/api/ideas/${enc(id)}`, 'PATCH', { text }).then(ideaOf),
  look: (id: string) =>
    call<unknown>(`/api/ideas/${enc(id)}/look`, 'POST', {}).then((r): LookResult => {
      const o = obj(r) ?? {};
      return { idea_id: str(o.idea_id) ?? id, status: 'peeking', kwant_id: str(o.kwant_id) || null, kwant_name: str(o.kwant_name) || null };
    }),
  later: (id: string) => call<unknown>(`/api/ideas/${enc(id)}/later`, 'POST', {}).then(ideaOf),
  restore: (id: string) => call<unknown>(`/api/ideas/${enc(id)}/restore`, 'POST', {}).then(ideaOf),
  pick: (id: string) =>
    call<unknown>(`/api/ideas/${enc(id)}/pick`, 'POST', {}).then((r): PickResult => {
      const o = obj(r) ?? {};
      const mission_id = str(o.mission_id);
      if (!mission_id) throw new IdeaError('The server sent back something unexpected. Try again.', 0);
      return { idea_id: str(o.idea_id) ?? id, mission_id, mission_status: str(o.mission_status), tasks: num(o.tasks) };
    }),
  remove: (id: string) => call<void>(`/api/ideas/${enc(id)}`, 'DELETE'),
};

/** Drops an idea in (the sidebar quick-add, the phone tab). Tells every open jar and badge. */
export async function dropIdea(spaceId: string, text: string): Promise<Idea> {
  const idea = await ideasApi.create(spaceId, text);
  emitIdeaUpdated({ id: idea.id, status: idea.status, space_id: idea.space_id || spaceId, idea });
  return idea;
}

/** Takes an idea back out (the drop toast's Undo, Delete). */
export async function deleteIdea(id: string, spaceId?: string): Promise<void> {
  await ideasApi.remove(id);
  emitIdeaUpdated({ id, status: 'deleted', space_id: spaceId ?? '' });
}

// ---------------------------------------------------------------------------
// Live updates. App's EventSource re-broadcasts SSE `idea_updated` here with emitIdeaUpdated(event.data).

export interface IdeaUpdated {
  id: string;
  status: IdeaStatus | 'deleted' | string;
  space_id: string;
  /** Set when the change came from this tab (a drop), so the jar can show it without a refetch. */
  idea?: Idea;
}

const bus = typeof EventTarget !== 'undefined' ? new EventTarget() : null;

export function emitIdeaUpdated(data: IdeaUpdated) {
  if (!data || typeof data.id !== 'string') return;
  bus?.dispatchEvent(new CustomEvent('idea_updated', { detail: data }));
}

export function onIdeaUpdated(fn: (data: IdeaUpdated) => void): () => void {
  const listener = (ev: Event) => fn((ev as CustomEvent).detail);
  bus?.addEventListener('idea_updated', listener);
  return () => bus?.removeEventListener('idea_updated', listener);
}

/** An event for this space? A blank space id (a local delete) matches every space. */
export const forSpace = (e: IdeaUpdated, spaceId: string) => !e.space_id || !spaceId || e.space_id === spaceId;

// The ready count, shared by the open jar and the badge so they never disagree.
const readyBySpace = new Map<string, number>();
const readyListeners = new Set<() => void>();
export function publishReadyCount(spaceId: string, n: number) {
  if (readyBySpace.get(spaceId) === n) return;
  readyBySpace.set(spaceId, n);
  readyListeners.forEach((l) => l());
}

/** Ready takes only (spec §0.4): the sidebar badge and the phone tab. 0 while loading or on an error. */
export function useIdeaJarBadge(spaceId: string | null | undefined): number {
  const key = spaceId ?? '';
  const [n, setN] = useState(() => readyBySpace.get(key) ?? 0);
  useEffect(() => {
    const sync = () => setN(readyBySpace.get(key) ?? 0);
    readyListeners.add(sync);
    sync();
    if (!key) return () => void readyListeners.delete(sync);
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const load = () =>
      ideasApi.list(key).then(
        (l) => alive && publishReadyCount(key, l.counts.ready),
        () => undefined,
      );
    load();
    const off = onIdeaUpdated((e) => {
      if (!forSpace(e, key)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(load, 250);
    });
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
      off();
      readyListeners.delete(sync);
    };
  }, [key]);
  return n;
}
