// Failover order (Settings › This space › Accounts) and the switch notice in chat. Pure helpers, so the
// components and the node tests share them. Every word and time shown comes from the server's data;
// nothing here makes up a number.

import type { EngineInfo } from './engines';
import { accountStatus } from './components/EngineAccounts';
import { Engine, Profile, ProfileSwitchedEvent, accountState, activeLimit } from './profiles';

export const FAILOVER_CAPTION = 'When one runs out, kwant bots move to the next that still works, in this order.';

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

/** Where a drag or an arrow press puts an account: the new order, or null when nothing moves. */
export function moveAccount(order: Profile[], from: number, to: number): Profile[] | null {
  if (from === to || from < 0 || to < 0 || from >= order.length || to >= order.length) return null;
  const next = [...order];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

export const PRIORITY_STEP = 10;

/** The priority updates that make `order` the failover order: ranks 10, 20, 30…, only for accounts
 * whose number changes. Uses the existing PATCH /api/profiles/:id `priority` field (0 to 10000). */
export function priorityUpdates(order: Profile[]): Array<{ id: string; priority: number }> {
  const out: Array<{ id: string; priority: number }> = [];
  order.forEach((p, i) => {
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
