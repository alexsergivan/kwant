import { useSyncExternalStore } from 'react';

/**
 * Living avatars, the rail moment: the space whose rail tile plays the picker's "just picked" once (one 160 ms
 * press + one 200 ms foil sweep) after a Space settings save changed its icon or colour. Cleared on the sweep's
 * animationend or a 400 ms backstop. Never set under reduced motion; a save while it is on doesn't restart it.
 * Nothing plays on a space switch, and the rail keeps no always-on holo.
 */
export const RAIL_JUST_BACKSTOP_MS = 400;

let current: string | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
const subs = new Set<() => void>();
const emit = () => subs.forEach((f) => f());

const reduced = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

export const railJust = {
  get: (): string | null => current,
  /** Play the moment on this space's rail tile. Returns whether it started. */
  set(spaceId: string): boolean {
    if (current || reduced()) return false;
    current = spaceId;
    timer = setTimeout(railJust.clear, RAIL_JUST_BACKSTOP_MS);
    emit();
    return true;
  },
  clear(): void {
    clearTimeout(timer);
    timer = undefined;
    if (current === null) return;
    current = null;
    emit();
  },
  subscribe(on: () => void): () => void {
    subs.add(on);
    return () => subs.delete(on);
  },
};

/** The space id whose rail tile is playing the moment now, or null. */
export function useRailJust(): string | null {
  return useSyncExternalStore(railJust.subscribe, railJust.get, railJust.get);
}

/** The rail tile's animationend: the sweep finishing ends the moment. */
export function onRailJustEnd(e: { animationName: string }): void {
  if (e.animationName === 'kw-foil-sweep') railJust.clear();
}
