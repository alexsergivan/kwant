/**
 * "Star kwant on GitHub": a one-time card after real use. It waits for 5 finished turns and 2 days
 * since this device first opened kwant, never shows while a bot needs the person, and once they
 * star or say "Don't ask again" it never comes back. "Maybe later" asks again in 2 weeks.
 * Everything lives in this device's localStorage; nothing is sent anywhere.
 */

export const STAR_REPO_URL = 'https://github.com/alexsergivan/kwant';
export const STAR_ASK_KEY = 'kwnt-star-ask';
export const STAR_MIN_TURNS = 5;
export const STAR_MIN_AGE_MS = 2 * 24 * 60 * 60 * 1000;
export const STAR_SNOOZE_MS = 14 * 24 * 60 * 60 * 1000;

export interface StarAskState {
  /** When this device first opened kwant (ms). */
  firstSeen: number;
  /** Turns that finished while the app was open, up to STAR_MIN_TURNS. */
  turns: number;
  /** "Maybe later": not before this time (ms). */
  snoozedUntil?: number;
  /** Starred or "Don't ask again": never again. */
  done?: 'starred' | 'never';
}

/** What else is on screen: the card waits while a bot needs the person. */
export interface StarAskBlockers {
  /** A bot shows "Needs you" (an open browser step). */
  needsYou: boolean;
  /** A question or approval is waiting for an answer. */
  waiting: boolean;
}

type Store = Pick<Storage, 'getItem' | 'setItem'>;

const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

/** The saved state; a missing or broken value starts the clock now, so first run never asks. */
export function parseStarAsk(raw: string | null, now: number): StarAskState {
  try {
    const v = raw ? JSON.parse(raw) : null;
    if (v && typeof v === 'object') {
      const firstSeen = num(v.firstSeen);
      if (firstSeen !== undefined) {
        const state: StarAskState = { firstSeen, turns: Math.max(0, Math.floor(num(v.turns) ?? 0)) };
        const snoozedUntil = num(v.snoozedUntil);
        if (snoozedUntil !== undefined) state.snoozedUntil = snoozedUntil;
        if (v.done === 'starred' || v.done === 'never') state.done = v.done;
        return state;
      }
    }
  } catch {
    // broken JSON: start over
  }
  return { firstSeen: now, turns: 0 };
}

export function shouldAskForStar(state: StarAskState, blockers: StarAskBlockers, now: number): boolean {
  if (state.done) return false;
  if (blockers.needsYou || blockers.waiting) return false;
  if (state.turns < STAR_MIN_TURNS) return false;
  if (now - state.firstSeen < STAR_MIN_AGE_MS) return false;
  if (state.snoozedUntil !== undefined && now < state.snoozedUntil) return false;
  return true;
}

/** One more finished turn; stops counting once it's enough, and after the card is done. */
export const countTurn = (state: StarAskState): StarAskState =>
  state.done || state.turns >= STAR_MIN_TURNS ? state : { ...state, turns: state.turns + 1 };

export const snoozeStarAsk = (state: StarAskState, now: number): StarAskState => ({ ...state, snoozedUntil: now + STAR_SNOOZE_MS });

export const finishStarAsk = (state: StarAskState, how: 'starred' | 'never'): StarAskState => ({ ...state, done: how });

const defaultStore = (): Store | undefined => {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
};

/** The shared state for the open app: reads storage once, writes every change back. */
export function createStarAskStore(store: Store | undefined = defaultStore(), clock: () => number = Date.now) {
  let raw: string | null = null;
  try {
    raw = store?.getItem(STAR_ASK_KEY) ?? null;
  } catch {
    raw = null;
  }
  let state = parseStarAsk(raw, clock());
  const listeners = new Set<() => void>();
  const save = (next: StarAskState) => {
    if (next === state) return;
    state = next;
    try {
      store?.setItem(STAR_ASK_KEY, JSON.stringify(state));
    } catch {
      // storage off (private browsing): the card just forgets, which is fine
    }
    listeners.forEach((l) => l());
  };
  // Save the first-seen time right away, so the 2 days count from the first launch.
  if (!raw) {
    try {
      store?.setItem(STAR_ASK_KEY, JSON.stringify(state));
    } catch {
      // as above
    }
  }
  return {
    get: () => state,
    subscribe: (l: () => void) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    turnFinished: () => save(countTurn(state)),
    later: () => save(snoozeStarAsk(state, clock())),
    starred: () => save(finishStarAsk(state, 'starred')),
    never: () => save(finishStarAsk(state, 'never')),
  };
}

export type StarAskStore = ReturnType<typeof createStarAskStore>;

let shared: StarAskStore | null = null;
/** The app's one store, made on first use. */
export const starAskStore = (): StarAskStore => (shared ??= createStarAskStore());
