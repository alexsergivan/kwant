// "New" stickers for what 0.6.9 changed: shown until the person uses the thing once, then gone for good.
// Stored per browser in localStorage; a missing or blocked storage just shows nothing.

export const NEW_VERSION = '0.6.9';
export type NewId = 'mention' | 'starters';

export interface KeyValueStore {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

export const newKey = (id: NewId, version = NEW_VERSION) => `kwant:new:${version}:${id}`;

const defaultStore = (): KeyValueStore | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

/** Still to be shown: not used yet. With no usable storage it stays hidden rather than nagging every visit. */
export function isNewThing(id: NewId, store: KeyValueStore | null = defaultStore()): boolean {
  if (!store) return false;
  try {
    return store.getItem(newKey(id)) === null;
  } catch {
    return false;
  }
}

const listeners = new Set<() => void>();
export const subscribeNew = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

/** The person used it: hide its sticker everywhere, for good. */
export function markNewSeen(id: NewId, store: KeyValueStore | null = defaultStore()): void {
  if (!store) return;
  try {
    if (store.getItem(newKey(id)) !== null) return;
    store.setItem(newKey(id), '1');
  } catch {
    return;
  }
  listeners.forEach((l) => l());
}
