// Which kwants are working right now, from `turn_started` / `turn_ended` events. Avatars subscribe to
// their own kwant only, so a turn starting re-renders just the avatars of that kwant.

type Listener = () => void;

/** kwant id -> its running turns, by turn id (or context id from servers that don't send turn ids). */
const active = new Map<string, Set<string>>();
const listeners = new Map<string, Set<Listener>>();

const notify = (kwantId: string) => listeners.get(kwantId)?.forEach((l) => l());

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
  if (!set) return;
  if (!set.delete(contextKey(contextId))) set.clear();
  if (set.size === 0) {
    active.delete(kwantId);
    notify(kwantId);
  }
}

/** Forgets every turn, e.g. after the event stream reconnected and may have missed `turn_ended`. */
export function clearAllTurns(): void {
  const ids = Array.from(active.keys());
  active.clear();
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
