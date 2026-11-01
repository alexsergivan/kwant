// "Good for" / "Not for" on a bot (covers / avoids): short phrases routing reads. Same rules as the
// server (PUT /api/kwants/:id): trimmed, duplicates dropped ignoring case, at most 20 of 60 chars.

export const MAX_TAGS = 20;
export const MAX_TAG_CHARS = 60;

/** Adds what was typed (commas split it) to `list`, keeping the server's rules. */
export function addTags(list: string[], typed: string): { list: string[]; error: string | null } {
  const out = [...list];
  let error: string | null = null;
  for (const raw of typed.split(',')) {
    const t = raw.trim().replace(/\s+/g, ' ');
    if (!t) continue;
    if (t.length > MAX_TAG_CHARS) {
      error = `Keep each one under ${MAX_TAG_CHARS} characters.`;
      continue;
    }
    if (out.some((x) => x.toLowerCase() === t.toLowerCase())) continue;
    if (out.length >= MAX_TAGS) {
      error = `Up to ${MAX_TAGS}. Remove one to add another.`;
      break;
    }
    out.push(t);
  }
  return { list: out, error };
}
