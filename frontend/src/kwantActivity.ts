// Which kwants are working right now, from `turn_started` / `turn_ended` events. Avatars subscribe to
// their own kwant only, so a turn starting re-renders just the avatars of that kwant.

import type { RunsOn } from './engines';

type Listener = () => void;

/** kwant id -> its running turns, by turn id (or context id from servers that don't send turn ids). */
const active = new Map<string, Set<string>>();
const listeners = new Map<string, Set<Listener>>();

const notify = (kwantId: string) => listeners.get(kwantId)?.forEach((l) => l());

// What each bot's running turn runs on (`turn_runs_on`), and, once it ends, what it last ran on until the
// bot list is read again (which then carries it as `last_ran_on`). The avatar's model badge reads both.
const liveRun = new Map<string, RunsOn>();
const lastRun = new Map<string, RunsOn>();

/** A turn started on (or failed over to) this engine and model. */
export function turnRunsOn(kwantId: string, run: RunsOn): void {
  if (!kwantId || !run?.engine) return;
  liveRun.set(kwantId, { ...run, at: run.at ?? new Date().toISOString() });
  notify(kwantId);
}

/** The bot is no longer working: its live engine becomes its last one. */
function settleRun(kwantId: string): void {
  const run = liveRun.get(kwantId);
  if (!run) return;
  liveRun.delete(kwantId);
  lastRun.set(kwantId, run);
}

/** The bot list was read again, with each bot's `last_ran_on`: the ones kept here are no longer needed. */
export function forgetLastRuns(): void {
  const ids = Array.from(lastRun.keys());
  lastRun.clear();
  ids.forEach(notify);
}

/** The running turn's engine and model, or undefined while resting (or before the server says). */
export const liveRunOf = (kwantId: string | undefined): RunsOn | undefined => (kwantId ? liveRun.get(kwantId) : undefined);
/** What the bot ran on in a turn that ended since the bot list was last read. */
export const lastRunOf = (kwantId: string | undefined): RunsOn | undefined => (kwantId ? lastRun.get(kwantId) : undefined);

const contextKey = (contextId: string | null | undefined) => contextId ?? '';

export function turnStarted(kwantId: string, contextId?: string | null): void {
  if (!kwantId) return;
  const set = active.get(kwantId) ?? new Set<string>();
  const before = set.size;
  set.add(contextKey(contextId));
  active.set(kwantId, set);
  if (before === 0) notify(kwantId);
}

/** Ends a turn whatever its outcome; an unknown context clears the kwant entirely. */
export function turnEnded(kwantId: string, contextId?: string | null): void {
  const set = active.get(kwantId);
  if (!set) {
    // Its start went unseen (a reconnect): still let go of what it ran on.
    if (liveRun.has(kwantId)) {
      settleRun(kwantId);
      notify(kwantId);
    }
    return;
  }
  if (!set.delete(contextKey(contextId))) set.clear();
  if (set.size === 0) {
    active.delete(kwantId);
    settleRun(kwantId);
    notify(kwantId);
  }
}

/** Forgets every turn, e.g. after the event stream reconnected and may have missed `turn_ended`. */
export function clearAllTurns(): void {
  const ids = Array.from(active.keys());
  active.clear();
  ids.forEach(settleRun);
  ids.forEach(notify);
}

/** Replaces everything with the server's running turns (GET /api/turns after a (re)connect). */
export function resetTurns(running: Array<{ kwant_id: string; turn_id: string }>): void {
  const before = new Set(active.keys());
  active.clear();
  for (const t of running) {
    const set = active.get(t.kwant_id) ?? new Set<string>();
    set.add(t.turn_id);
    active.set(t.kwant_id, set);
  }
  before.forEach((id) => active.has(id) || settleRun(id));
  new Set([...before, ...active.keys()]).forEach(notify);
}

export const isKwantWorking = (kwantId: string | undefined): boolean => !!kwantId && active.has(kwantId);

export function subscribeKwant(kwantId: string, listener: Listener): () => void {
  const set = listeners.get(kwantId) ?? new Set<Listener>();
  set.add(listener);
  listeners.set(kwantId, set);
  return () => {
    set.delete(listener);
    if (set.size === 0) listeners.delete(kwantId);
  };
}
