// Failover order (Settings › This space › Apps & accounts) and the switch notice in chat. Pure helpers, so the
// components and the node tests share them. Every word and time shown comes from the server's data;
// nothing here makes up a number.

import type { EngineInfo } from './engines';
import { accountStatus } from './components/EngineAccounts';
import { Engine, Profile, ProfileSwitchedEvent, accountState, activeLimit } from './profiles';
import { parseMetaObject } from './questions';

export const FAILOVER_CAPTION = 'When one runs out, kwant bots move to the next that still works, in this order.';

/** The AI accounts intro: what the order means for the person's bots. */
export const orderIntro = (space: string) =>
  `Your bots in ${space} use the first account that’s ready. If it runs out, they switch to the next one by themselves.`;

export const ENGINE_NAMES: Record<Engine, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', custom: 'Custom API' };

/** The engine and kind in plain words: "Claude", "Claude API key", "Custom API". */
export function engineLabel(p: Pick<Profile, 'engine' | 'kind'>): string {
  const base = ENGINE_NAMES[p.engine] ?? p.engine;
  return p.kind === 'api_key' ? `${base} API key` : base;
}

/** The accounts in the order turns try them: lowest priority number first; ties keep "This machine"
 * first, then the name, so the list never jumps around between loads. */
export function failoverOrder(profiles: Profile[]): Profile[] {
  return [...profiles].sort(
    (a, b) =>
      a.priority - b.priority ||
      Number(b.kind === 'system') - Number(a.kind === 'system') ||
      a.engine.localeCompare(b.engine) ||
      a.name.localeCompare(b.name) ||
      a.id.localeCompare(b.id),
  );
}

/** A clock time in the person's own zone: "03:00", with the weekday or date when it isn't today. */
export function clockTime(at: Date, now = Date.now()): string {
  const time = at.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
  const today = new Date(now);
  if (at.toDateString() === today.toDateString()) return time;
  const days = (at.getTime() - now) / 86400_000;
  const day = days < 6 ? at.toLocaleDateString(undefined, { weekday: 'short' }) : at.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  return `${day} ${time}`;
}

export interface PriorityStatus {
  tone: 'ok' | 'warn' | 'error' | 'muted' | 'busy' | 'info';
  /** "Ready", "Out until 03:00", "Signed out", else the account's own state word. */
  label: string;
}

/** The status on a priority row. `engine` is the card the account sits on, for "This machine". */
export function priorityStatus(p: Profile, engine: EngineInfo | undefined, now = Date.now()): PriorityStatus {
  const limit = activeLimit(p.limited_until, now);
  // Off outranks a stale limit: a switched-off account is skipped either way.
  if (p.enabled && limit) return { tone: 'warn', label: `Out until ${clockTime(limit, now)}` };
  const s = engine ? accountStatus(p, engine, now) : accountState(p, now);
  return { tone: s.tone, label: s.label === 'Needs sign-in' ? 'Signed out' : s.label };
}

/** Not-installed and signed-out accounts can't take a turn, so they sit outside the ranked order. A saved
 * limit ("Out until 03:00") or "Off" is the person's or the provider's state, so those stay ranked. */
export function isRanked(p: Profile, engine: EngineInfo | undefined, now = Date.now()): boolean {
  const { label } = priorityStatus(p, engine, now);
  return label !== 'Not installed' && label !== 'Signed out';
}

/** The order split for display: the ranked list (reorderable) and the rest, each in failover order.
 * Display only: nothing here changes an account's saved priority. */
export function splitRanked(
  order: Profile[],
  engineOf: (p: Profile) => EngineInfo | undefined,
  now = Date.now(),
): { ranked: Profile[]; unranked: Profile[] } {
  const ranked: Profile[] = [];
  const unranked: Profile[] = [];
  for (const p of order) (isRanked(p, engineOf(p), now) ? ranked : unranked).push(p);
  return { ranked, unranked };
}

/** Why an account sits under "Not used", and so its one button: Turn on, Sign in or Set up. */
export type NotUsedReason = 'off' | 'signed_out' | 'not_installed';

export interface AccountGroups {
  /** On and able to take a turn, in failover order: [0] is "Used first", the rest "If it runs out". */
  used: Profile[];
  /** Off, signed out or not installed: never in the order, each with its reason. */
  notUsed: Array<{ p: Profile; reason: NotUsedReason; engineOff: boolean }>;
}

/** The order split the way the AI accounts list shows it. An account out of usage keeps its place: it
 * comes back by itself. Display only: nothing here changes an account's saved priority. */
export function accountGroups(order: Profile[], engineOf: (p: Profile) => EngineInfo | undefined, now = Date.now()): AccountGroups {
  const used: Profile[] = [];
  const notUsed: AccountGroups['notUsed'] = [];
  for (const p of order) {
    const e = engineOf(p);
    const engineOff = !!e && (e.enabled === false || e.status === 'disabled');
    if (!p.enabled || engineOff) {
      notUsed.push({ p, reason: 'off', engineOff });
      continue;
    }
    const { label } = priorityStatus(p, e, now);
    if (label === 'Not installed') notUsed.push({ p, reason: 'not_installed', engineOff });
    else if (label === 'Signed out') notUsed.push({ p, reason: 'signed_out', engineOff });
    else used.push(p);
  }
  return { used, notUsed };
}

/** Which account a row is, under the AI's name: "Signed in on this computer", or the account's own name. */
export const accountWhere = (p: Pick<Profile, 'kind' | 'name'>): string => (p.kind === 'system' ? 'Signed in on this computer' : p.name);

/** Where a drag or an arrow press puts an account: the new order, or null when nothing moves. */
export function moveAccount(order: Profile[], from: number, to: number): Profile[] | null {
  if (from === to || from < 0 || to < 0 || from >= order.length || to >= order.length) return null;
  const next = [...order];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

export const PRIORITY_STEP = 10;

/** The priority updates that make `order` the failover order: the whole list numbered 10, 20, 30…, with
 * `rest` (the accounts outside the ranked list, in their current relative order) after it, so one that
 * becomes ready later joins at the bottom, never above what the person ordered. Only accounts whose number
 * changes are returned. Uses the existing PATCH /api/profiles/:id `priority` field (0 to 10000). */
export function priorityUpdates(order: Profile[], rest: Profile[] = []): Array<{ id: string; priority: number }> {
  const out: Array<{ id: string; priority: number }> = [];
  [...order, ...rest].forEach((p, i) => {
    const priority = Math.min(10_000, (i + 1) * PRIORITY_STEP);
    if (p.priority !== priority) out.push({ id: p.id, priority });
  });
  return out;
}

/** What a screen reader hears after a move: "Codex moved to position 2 of 5." */
export const moveAnnouncement = (name: string, index: number, total: number) => `${name} moved to position ${index + 1} of ${total}.`;

// ---- The switch notice in chat ----

/** `profile_switched` as Vega's backend extends it; the engine and model fields are absent on older servers. */
export type FailoverEvent = ProfileSwitchedEvent & {
  from_engine?: string | null;
  from_model?: string | null;
  to_engine?: string | null;
  to_model?: string | null;
  /** The model as people read it ("GPT-6.1"), when the server sends one. */
  to_model_label?: string | null;
  /** Set when the server scopes the switch to one chat. */
  context_id?: string | null;
};

const engineName = (engine: string | null | undefined, fallback: string): string => {
  if (engine && engine in ENGINE_NAMES) return ENGINE_NAMES[engine as Engine];
  return fallback;
};

/** "Claude is out until 03:00 · answered on Codex (GPT-6.1)". Kept to one short line. */
export function failoverNotice(e: FailoverEvent, now = Date.now()): string {
  const from = engineName(e.from_engine, e.from_name);
  const to = engineName(e.to_engine, e.to_name);
  const limit = activeLimit(e.retry_at ?? null, now);
  const why = e.class === 'auth' ? `${from} is signed out` : limit ? `${from} is out until ${clockTime(limit, now)}` : `${from} is out of usage`;
  const model = e.to_model_label || e.to_model;
  return `${why} · answered on ${to}${model ? ` (${model})` : ''}`;
}

/** One outage, for "once per chat": the same chat, the same account that ran out, the same reset. */
export function outageKey(e: Pick<FailoverEvent, 'kwant_id' | 'from_profile_id' | 'class' | 'retry_at'>, chat?: string | null): string {
  return [chat || e.kwant_id, e.from_profile_id, e.class, e.retry_at ?? ''].join('|');
}

/** Remembers which outages a chat has already been told about. */
export function createOutageLog() {
  const seen = new Set<string>();
  return {
    /** True the first time this outage is seen in this chat. */
    first(key: string): boolean {
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    },
    size: () => seen.size,
  };
}

// ---- The saved notice (survives a reload) ----

const text = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);

/** One saved switch, tolerant of anything else in the array. */
function savedSwitch(v: unknown): FailoverEvent | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null;
  const r = v as Record<string, unknown>;
  const from_name = text(r.from_name);
  const to_name = text(r.to_name);
  if (!from_name && !text(r.from_engine)) return null;
  if (!to_name && !text(r.to_engine)) return null;
  return {
    kwant_id: text(r.kwant_id) ?? '',
    from_profile_id: text(r.from_profile_id) ?? '',
    from_name: from_name ?? '',
    to_profile_id: text(r.to_profile_id) ?? '',
    to_name: to_name ?? '',
    class: text(r.class) ?? 'quota',
    retry_at: text(r.retry_at) ?? undefined,
    from_engine: text(r.from_engine),
    from_model: text(r.from_model),
    to_engine: text(r.to_engine),
    to_model: text(r.to_model),
    to_model_label: text(r.to_model_label),
  };
}

/**
 * The notice a saved reply carries in `meta.failover` (an array of switches, or one object), as one
 * line. A reply that moved twice reads from the first account that ran out to the one that answered.
 * Null for anything missing or malformed, so the reply simply shows no notice.
 */
export function savedFailoverNotice(meta: unknown, now = Date.now()): string | null {
  const m = parseMetaObject(meta);
  const raw = m?.failover;
  const list = (Array.isArray(raw) ? raw : raw ? [raw] : []).map(savedSwitch).filter((s): s is FailoverEvent => !!s);
  if (list.length === 0) return null;
  const first = list[0];
  const last = list[list.length - 1];
  return failoverNotice({ ...first, to_name: last.to_name, to_engine: last.to_engine, to_model: last.to_model, to_model_label: last.to_model_label }, now);
}

/** The engines a saved reply moved between ("Claude" to "Codex") and the notice line, for the engine chip. Null when none. */
export function savedFailoverHandoff(meta: unknown, now = Date.now()): { from: string; to: string; why: string } | null {
  const m = parseMetaObject(meta);
  const raw = m?.failover;
  const list = (Array.isArray(raw) ? raw : raw ? [raw] : []).map(savedSwitch).filter((s): s is FailoverEvent => !!s);
  if (list.length === 0) return null;
  const first = list[0];
  const last = list[list.length - 1];
  const why = savedFailoverNotice(meta, now);
  if (!why) return null;
  return { from: engineName(first.from_engine, first.from_name), to: engineName(last.to_engine, last.to_name), why };
}
