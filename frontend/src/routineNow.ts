// routine-card-stays-current: the routines as they are now, shared by every routine card in the chat.
// One GET /api/routines for all cards, only while at least one card is on screen; refetched on `routine_run`
// and `routines_changed` (any create, edit, pause or delete through continuityApi, a bot's routine action).
import { useSyncExternalStore } from 'react';
import { Routine, continuityApi, onContinuity } from './continuity';

type Api = { routines: () => Promise<Routine[]>; updateRoutine: (id: string, body: { enabled: boolean }) => Promise<Routine> };

export interface RoutineStore {
  /** The routine now; undefined before the first load, or while a miss waits for a newer list; null (deleted) only
   *  when a list fetched after the miss was noticed still doesn't have it. */
  get: (id: string) => Routine | null | undefined;
  subscribe: (fn: () => void) => () => void;
  load: () => Promise<void>;
  refresh: () => Promise<void>;
  /** PATCH { enabled } and keep the answer. Resolves to an error message on failure. */
  setEnabled: (id: string, enabled: boolean) => Promise<string | undefined>;
}

export function createRoutineStore(api: Api): RoutineStore {
  let byId: Map<string, Routine> | null = null;
  let inflight: Promise<void> | null = null;
  let again = false;
  // A list can predate a routine (a bot's card renders before its BotActionUpdated), so a miss isn't "deleted" yet:
  // remember which fetch was the newest when the miss was seen, refetch, and say deleted only if a later one lacks it too.
  let started = 0;
  let done = 0;
  const missedAt = new Map<string, number>();
  const subs = new Set<() => void>();
  const notify = () => subs.forEach((fn) => fn());

  const fetchOnce = async () => {
    const n = ++started;
    try {
      const all = await api.routines();
      byId = new Map(all.map((r) => [r.id, r]));
      done = n;
      notify();
    } catch {
      // Keep what we had: a card falls back to its stored facts until the next refresh.
    }
  };
  // Coalesces: a refresh asked for while one runs fetches once more after it.
  const refresh = (): Promise<void> => {
    if (inflight) {
      again = true;
      return inflight;
    }
    inflight = (async () => {
      do {
        again = false;
        await fetchOnce();
      } while (again);
      inflight = null;
    })();
    return inflight;
  };

  return {
    get: (id) => {
      if (!byId) return undefined;
      const r = byId.get(id);
      if (r) return r;
      const at = missedAt.get(id);
      if (at === undefined) {
        missedAt.set(id, started);
        queueMicrotask(() => void refresh());
        return undefined;
      }
      return done > at ? null : undefined;
    },
    subscribe: (fn) => {
      subs.add(fn);
      if (subs.size === 1 && !byId) void refresh();
      return () => {
        subs.delete(fn);
      };
    },
    load: () => (byId ? Promise.resolve() : inflight ?? refresh()),
    refresh,
    setEnabled: async (id, enabled) => {
      try {
        const r = await api.updateRoutine(id, { enabled });
        if (r && r.id === id) {
          byId = new Map(byId ?? []);
          byId.set(id, r);
          notify();
        } else void refresh();
        return undefined;
      } catch (e) {
        return e instanceof Error && e.message ? e.message : 'Could not change the routine.';
      }
    },
  };
}

export const routineStore = createRoutineStore(continuityApi);

let wired = false;
function wire() {
  if (wired) return;
  wired = true;
  onContinuity('routine_run', () => void routineStore.refresh());
  onContinuity('routines_changed', () => void routineStore.refresh());
}

const noop = () => () => {};
const unknown = () => undefined;
const subscribeStore = (fn: () => void) => {
  wire();
  return routineStore.subscribe(fn);
};

/** The routine `id` as it is now (see RoutineStore.get); no request at all while `id` is null. */
export function useRoutineNow(id: string | null): Routine | null | undefined {
  return useSyncExternalStore(
    id ? subscribeStore : noop,
    id ? () => routineStore.get(id) : unknown,
    unknown,
  );
}
