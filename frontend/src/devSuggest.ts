// Dev mode visibility (autonomy/dev-mode-visibility/plan.md): the pure parts of the "this space has code"
// suggestion in #general and of the Off card in Settings › General, so tests can check the gating and the
// words without a DOM. Detection is the server's: GET /dev_mode sends `repos` even while Off.

import { devApi, devStore, type DevModeState, type DevRepo } from './devMode';

/** "Not now" for one space, for good, in this browser (the same pattern as the getting-started dismiss). */
export const suggestKey = (space: string) => `kwnt:dev-suggest:${space}`;

const store = (): Storage | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

export const suggestDismissed = (space: string): boolean => {
  try {
    return store()?.getItem(suggestKey(space)) === 'dismissed';
  } catch {
    return false;
  }
};

export function dismissSuggest(space: string): void {
  try {
    store()?.setItem(suggestKey(space), 'dismissed');
  } catch {
    /* private mode: it shows again next time, which is harmless */
  }
}

/** The repo to name: the first one a team brought (team order), else the first one found. */
export function foundRepo(state: DevModeState | null | undefined): DevRepo | null {
  if (!state || !state.repos.length) return null;
  return state.repos.find((r) => r.team) ?? state.repos[0];
}

/** Off and the server found a repo in one of the space's teams (or a hand-added one): the space has code. */
export const hasFoundCode = (state: DevModeState | null | undefined): boolean => !!state && !state.on && state.repos.length > 0;

export interface SuggestGate {
  /** The person is looking at the space's #general. */
  inGeneral: boolean;
  /** The space's Dev mode state; null while loading, after a failed GET or on a bot's 403. */
  state: DevModeState | null;
  dismissed: boolean;
  /** The getting-started card or the setup banner is up: getting started wins. */
  gettingStarted: boolean;
}

export const suggestShows = (g: SuggestGate): boolean => g.inGeneral && !g.dismissed && !g.gettingStarted && hasFoundCode(g.state);

const asked = new Set<string>();
/**
 * The suggestion needs the open space's state, which the desktop rail asks for but a phone may not: ask once
 * per space per page load when nothing is cached. A failed GET (or a bot's 403) leaves the store empty, so
 * nothing shows.
 */
export function ensureDevState(space: string, get: (s: string) => Promise<DevModeState> = devApi.get): void {
  if (devStore.get(space) || asked.has(space)) return;
  asked.add(space);
  get(space).then((st) => devStore.set(space, st), () => undefined);
}
/** Tests only. */
export const _resetAsked = () => asked.clear();
