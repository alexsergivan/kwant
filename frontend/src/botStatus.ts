// One status per bot for the dot on its avatar: what it's doing right now, from its turns and whether
// anything can run. A bot with a running turn is never "Available", the open chat included.

export type BotStatus = 'needs_you' | 'working' | 'queued' | 'cant_run' | 'available';

export interface StatusTurn {
  turn_id: string;
  kwant_id: string;
  state: 'queued' | 'running';
  context_id?: string;
}

export interface StatusInputs {
  turns: Iterable<StatusTurn>;
  /** Running turns paused on a question or a browser step for the person. */
  awaiting?: ReadonlySet<string>;
  /** Open browser steps per bot. */
  needsYou?: Readonly<Record<string, number>>;
  /** false when no account works here; null or undefined while that is still unknown. */
  canRun?: boolean | null;
}

/** Highest first: the status a bot shows when several apply. */
export const STATUS_PRIORITY: readonly BotStatus[] = ['needs_you', 'working', 'queued', 'cant_run', 'available'];

export const STATUS_LABEL: Record<BotStatus, string> = {
  needs_you: 'Needs you',
  working: 'Working',
  queued: 'Queued',
  cant_run: "Can't run",
  available: 'Available',
};

const rank = (s: BotStatus) => STATUS_PRIORITY.indexOf(s);
const higher = (a: BotStatus | undefined, b: BotStatus): BotStatus => (a && rank(a) <= rank(b) ? a : b);

/** Status for every bot that has turns or open steps; bots not listed are idle. */
export function botStatuses({ turns, awaiting, needsYou = {} }: StatusInputs): Record<string, BotStatus> {
  const out: Record<string, BotStatus> = {};
  for (const t of turns) {
    if (!t?.kwant_id) continue;
    const s: BotStatus = t.state === 'queued' ? 'queued' : awaiting?.has(t.turn_id) ? 'needs_you' : 'working';
    out[t.kwant_id] = higher(out[t.kwant_id], s);
  }
  for (const [id, n] of Object.entries(needsYou)) if (n > 0) out[id] = 'needs_you';
  return out;
}

/** One bot's status: its turns first, then whether it can run at all. */
export function statusOf(kwantId: string, statuses: Readonly<Record<string, BotStatus>>, canRun?: boolean | null): BotStatus {
  return statuses[kwantId] ?? (canRun === false ? 'cant_run' : 'available');
}

/** Words for the avatar's tooltip and screen readers: "Editor · Working". */
export const statusText = (name: string | undefined, status: BotStatus): string =>
  name ? `${name} · ${STATUS_LABEL[status]}` : STATUS_LABEL[status];

/** The dot's classes; Working pulses unless the person asked for less motion. */
export const STATUS_DOT: Record<BotStatus, string> = {
  needs_you: 'bg-amber-400',
  working: 'bg-sky-400 animate-pulse motion-reduce:animate-none',
  queued: 'bg-zinc-950 border-2 border-zinc-400',
  cant_run: 'bg-zinc-600',
  available: 'bg-emerald-400',
};

/**
 * The line under each busy bot's name. Running beats queued so it agrees with the dot; other chats
 * come before the open one, which reads "here" instead of naming itself.
 */
export function busyLines({ turns, openCtx, awaiting, browser, describe }: {
  turns: Iterable<StatusTurn>;
  openCtx: string;
  awaiting?: ReadonlySet<string>;
  browser?: ReadonlySet<string>;
  describe: (contextId: string, kwantId: string) => string;
}): Record<string, string> {
  const out: Record<string, string> = {};
  const later = (t: StatusTurn) => (t.state === 'queued' ? 2 : 0) + (t.context_id === openCtx ? 1 : 0);
  for (const t of [...turns].sort((x, y) => later(x) - later(y))) {
    if (!t?.kwant_id || out[t.kwant_id]) continue;
    const what = t.state === 'queued' ? 'Waiting to work' : browser?.has(t.turn_id) ? 'Needs you in the browser' : awaiting?.has(t.turn_id) ? 'Asked you a question' : 'Working';
    out[t.kwant_id] =
      t.context_id === openCtx ? `${what} here` : `${what} ${t.state === 'queued' || what === 'Working' ? 'on' : 'in'} ${describe(t.context_id ?? '', t.kwant_id)}`;
  }
  return out;
}
