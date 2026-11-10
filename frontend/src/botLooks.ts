// One look per bot, everywhere: the avatar style the bot has saved (`avatar_sprite`).
// Some avatars only know a bot's title or name (chat replies, the working line, cards);
// they look the bot up here so they draw the same creature as the sidebar and the Ask chips.

export interface BotLook {
  id: string;
  name: string;
  avatar_sprite?: string | null;
}

let byId = new Map<string, string>();
// A name two bots share maps to null: guessing would give one of them the other's look.
let byName = new Map<string, string | null>();
let rev = 0;
const listeners = new Set<() => void>();

/** Replaces the known bots (the app calls it whenever its bot list changes). */
export function setKnownBots(bots: readonly BotLook[]): void {
  const ids = new Map<string, string>();
  const names = new Map<string, string | null>();
  for (const b of bots) {
    const style = b.avatar_sprite?.trim();
    if (!style) continue;
    ids.set(b.id, style);
    const key = b.name.trim().toLowerCase();
    if (!key) continue;
    names.set(key, names.has(key) && names.get(key) !== style ? null : style);
  }
  byId = ids;
  byName = names;
  rev += 1;
  for (const fn of listeners) fn();
}

export function subscribeBotLooks(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

/** Changes whenever the known bots change; the snapshot for useSyncExternalStore. */
export const botLooksRev = (): number => rev;

/** The bot's saved style, by id first, then by a name only one bot has. */
export function savedStyleOf(kwantId?: string | null, name?: string | null): string | undefined {
  if (kwantId) {
    const s = byId.get(kwantId);
    if (s) return s;
  }
  const key = name?.trim().toLowerCase();
  if (!key) return undefined;
  return byName.get(key) ?? undefined;
}

/**
 * The style an avatar draws. A style id the caller passed (`isStyle`) wins: it is already a saved
 * or just-picked style. A title or name hint gives way to the bot's saved style when the bot is
 * known; otherwise the hint stays, as before.
 */
export function avatarStyleFor(
  role: string | undefined,
  kwantId: string | undefined,
  name: string | undefined,
  isStyle: (role: string) => boolean,
): string | undefined {
  if (role && isStyle(role)) return role;
  return savedStyleOf(kwantId, name) ?? role;
}
