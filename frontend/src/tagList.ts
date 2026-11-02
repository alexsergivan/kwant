// "Good for" / "Not for" on a bot (covers / avoids): short phrases routing reads. Same rules as the
// server (PUT /api/kwants/:id): trimmed, duplicates dropped ignoring case, at most 20 of 60 chars.

export const MAX_TAGS = 20;
export const MAX_TAG_CHARS = 60;

export type TagLimits = {
  max: number;
  /** Longest entry; 0 leaves the length to the server. */
  maxChars: number;
  /** What separates entries in typed or pasted text. */
  split: RegExp;
  /** The words for one entry in the messages ("one" for phrases, "site" for sites). */
  noun?: string;
};

export const PHRASE_LIMITS: TagLimits = { max: MAX_TAGS, maxChars: MAX_TAG_CHARS, split: /[,\n]/ };

/** Adds what was typed (commas split it) to `list`, keeping the server's rules. */
export function addTags(list: string[], typed: string, limits: TagLimits = PHRASE_LIMITS): { list: string[]; error: string | null } {
  const out = [...list];
  let error: string | null = null;
  for (const raw of typed.split(limits.split)) {
    const t = raw.trim().replace(/\s+/g, ' ');
    if (!t) continue;
    if (limits.maxChars && t.length > limits.maxChars) {
      error = `Keep each one under ${limits.maxChars} characters.`;
      continue;
    }
    if (out.some((x) => x.toLowerCase() === t.toLowerCase())) continue;
    if (out.length >= limits.max) {
      error = `Up to ${limits.max}. Remove ${limits.noun ? `a ${limits.noun}` : 'one'} to add another.`;
      break;
    }
    out.push(t);
  }
  return { list: out, error };
}
