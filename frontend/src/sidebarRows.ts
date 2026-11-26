// 36i sidebar-fewer-rows: which of the sidebar's optional rows sit at the top and which wait under "More".
// Search, Inbox and Routines are always top level. Mission board, Idea jar and Parallel Universe start
// under a closed "More" row and move up for good (per space, per device) once they have been opened, or
// while they have something live: the jar has ready takes, or the page is the one open now.

export type MoreRowKey = 'missions' | 'jar' | 'parallel';

/** The order the rows keep, top level or inside More. */
export const MORE_ROW_ORDER: readonly MoreRowKey[] = ['missions', 'jar', 'parallel'];

export interface RowPlacementInput {
  /** Which rows exist at all (the Mission board setting, a handler, phone tabs). */
  present: Partial<Record<MoreRowKey, boolean>>;
  /** Rows opened before in this space. */
  used: ReadonlySet<MoreRowKey> | readonly MoreRowKey[];
  /** The page open now. */
  active?: Partial<Record<MoreRowKey, boolean>>;
  /** The jar's ready-takes count. */
  jarCount?: number;
}

export interface RowPlacement {
  top: MoreRowKey[];
  more: MoreRowKey[];
}

export function placeRows({ present, used, active = {}, jarCount = 0 }: RowPlacementInput): RowPlacement {
  const seen = new Set(used);
  const top: MoreRowKey[] = [];
  const more: MoreRowKey[] = [];
  for (const k of MORE_ROW_ORDER) {
    if (!present[k]) continue;
    const live = !!active[k] || (k === 'jar' && jarCount > 0);
    (seen.has(k) || live ? top : more).push(k);
  }
  return { top, more };
}

const KEY = (spaceId: string) => `kw.sidebar.used.${spaceId}`;
type Store = Pick<Storage, 'getItem' | 'setItem'> | undefined;
const store = (): Store => {
  try {
    return typeof localStorage === 'undefined' ? undefined : localStorage;
  } catch {
    return undefined;
  }
};

/** The rows opened before in this space; unknown words and broken JSON are ignored. */
export function readUsedRows(spaceId: string, s: Store = store()): MoreRowKey[] {
  try {
    const raw = JSON.parse(s?.getItem(KEY(spaceId)) ?? '[]');
    return Array.isArray(raw) ? MORE_ROW_ORDER.filter((k) => raw.includes(k)) : [];
  } catch {
    return [];
  }
}

/** Remembers that a row was opened in this space. Returns the new list. */
export function markRowUsed(spaceId: string, key: MoreRowKey, s: Store = store()): MoreRowKey[] {
  const now = readUsedRows(spaceId, s);
  if (now.includes(key)) return now;
  const next = MORE_ROW_ORDER.filter((k) => k === key || now.includes(k));
  try {
    s?.setItem(KEY(spaceId), JSON.stringify(next));
  } catch {
    /* private mode: it still moves up for this visit */
  }
  return next;
}
