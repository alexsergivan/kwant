// Find a tool's matching (composio-simple/autocomplete.md, design.md "Autocomplete"): one rule for
// built-in tools, Composio's apps and the public directory, run in the browser on every key.
//
// Matching ignores case, spaces, dashes, dots and accents ("google sheets" = "googlesheets").
// Rank: the name starts with the query > a word in it does > an alias > the slug > the name contains
// it (2+ characters) > a typo (1 edit from 4 characters, 2 from 8). One character only matches the
// start of the name or of a word in it.

export type MatchKind = 'start' | 'word' | 'alias' | 'slug' | 'contains' | 'typo';

export interface ToolEntry {
  name: string;
  slug?: string;
}

export interface ToolMatch {
  kind: MatchKind;
  /** Lower ranks first. */
  rank: number;
  /** The typed letters inside the name, [from, to) in the name as written; none for alias, slug and typo matches. */
  bold: [number, number] | null;
}

const RANK: Record<MatchKind, number> = { start: 0, word: 1, alias: 2, slug: 3, contains: 4, typo: 5 };

/** Other names people type for an app, by the app's normalized name or slug. */
export const TOOL_ALIASES: Record<string, string[]> = {
  gmail: ['google mail'],
  googlecalendar: ['gcal', 'calendar'],
  googlesheets: ['sheets'],
  googledocs: ['docs'],
  googledrive: ['drive'],
  elevenlabs: ['11labs', 'eleven'],
  microsoftteams: ['teams'],
  x: ['twitter'],
  twitter: ['twitter'],
  postgresql: ['postgres', 'pg'],
  postgres: ['postgres', 'pg'],
};

const SKIPPED = /[\s\-._/'’&,:()+]/;

/** Lowercase, no accents, no spaces, dashes or dots. */
export const normalize = (s: string): string => normalizeWithMap(s).text;

/** The normalized text, and for each of its characters the index of the character it came from. */
function normalizeWithMap(s: string): { text: string; from: number[] } {
  let text = '';
  const from: number[] = [];
  for (let i = 0; i < s.length; i++) {
    const ch = s[i];
    if (SKIPPED.test(ch)) continue;
    const plain = ch.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();
    for (const c of plain) {
      text += c;
      from.push(i);
    }
  }
  return { text, from };
}

/** Where a normalized range [a, b) sits in the name as written. */
function spanOf(from: number[], a: number, b: number): [number, number] {
  return [from[a], from[b - 1] + 1];
}

/** Optimal string alignment distance (a swap of two letters is one edit), stopping early above `max`. */
export function editDistance(a: string, b: string, max = 2): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  const rows: number[][] = [];
  for (let i = 0; i <= a.length; i++) {
    rows.push(new Array(b.length + 1).fill(0));
    rows[i][0] = i;
  }
  for (let j = 0; j <= b.length; j++) rows[0][j] = j;
  for (let i = 1; i <= a.length; i++) {
    let best = Infinity;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let v = Math.min(rows[i - 1][j] + 1, rows[i][j - 1] + 1, rows[i - 1][j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) v = Math.min(v, rows[i - 2][j - 2] + 1);
      rows[i][j] = v;
      best = Math.min(best, v);
    }
    if (best > max) return max + 1;
  }
  return rows[a.length][b.length];
}

/** Indices (in the normalized name) where a word of the name starts, the first word excluded. */
function wordStarts(name: string, from: number[]): number[] {
  const starts: number[] = [];
  for (let k = 1; k < from.length; k++) {
    const i = from[k];
    if (i === from[k - 1]) continue;
    const prev = name[i - 1];
    const cur = name[i];
    const afterGap = i > 0 && SKIPPED.test(prev);
    // "ElevenLabs", "PostHog": a capital after a small letter starts a word too.
    const camel = /[a-z]/.test(prev ?? '') && /[A-Z]/.test(cur);
    const digitEdge = /[0-9]/.test(prev ?? '') !== /[0-9]/.test(cur);
    if (afterGap || camel || digitEdge) starts.push(k);
  }
  return starts;
}

const aliasesOf = (entry: ToolEntry): string[] => {
  const keys = new Set([normalize(entry.name), entry.slug ? normalize(entry.slug) : '']);
  const out: string[] = [];
  keys.forEach((k) => k && TOOL_ALIASES[k]?.forEach((a) => out.push(normalize(a))));
  return out;
};

/** How `query` matches `entry`, or null. */
export function matchTool(query: string, entry: ToolEntry): ToolMatch | null {
  const q = normalize(query);
  if (!q) return null;
  const { text: name, from } = normalizeWithMap(entry.name);
  const hit = (kind: MatchKind, at?: number): ToolMatch => ({
    kind,
    rank: RANK[kind],
    bold: at === undefined ? null : spanOf(from, at, at + q.length),
  });
  if (name.startsWith(q)) return hit('start', 0);
  const word = wordStarts(entry.name, from).find((k) => name.startsWith(q, k));
  if (word !== undefined) return hit('word', word);
  if (q.length < 2) return null;
  if (aliasesOf(entry).some((a) => a.startsWith(q))) return hit('alias');
  const slug = entry.slug ? normalize(entry.slug) : '';
  if (slug && slug !== name && slug.startsWith(q)) return hit('slug');
  const inside = name.indexOf(q);
  if (inside >= 0) return hit('contains', inside);
  if (q.length < 4) return null;
  const max = q.length >= 8 ? 2 : 1;
  // A typo in a whole name, or in the part typed so far ("gmial" → Gmail, "notoin" → Notion, "slak" → Slack).
  const candidates = [name, ...[-1, 0, 1].map((d) => name.slice(0, q.length + d)).filter((p) => p.length >= 3)];
  if (candidates.some((c) => editDistance(q, c, max) <= max)) return hit('typo');
  return null;
}

/**
 * The rest of `name` after what was typed, for the ghost completion, or '' when the name doesn't
 * start with it. "gm" + "Gmail" → "ail"; "google s" + "Google Sheets" → "heets".
 */
export function ghostRest(query: string, name: string): string {
  const q = normalize(query);
  const { text, from } = normalizeWithMap(name);
  if (!q || !text.startsWith(q)) return '';
  const lastTyped = from[q.length - 1];
  return name.slice(lastTyped + 1);
}

/** The rows of a group that match, best first. `tie` breaks equal ranks: higher first. */
export function rankTools<T extends ToolEntry>(query: string, entries: T[], tie: (e: T) => number = () => 0): Array<{ entry: T; match: ToolMatch }> {
  const out: Array<{ entry: T; match: ToolMatch; i: number }> = [];
  entries.forEach((entry, i) => {
    const match = matchTool(query, entry);
    if (match) out.push({ entry, match, i });
  });
  out.sort((a, b) => a.match.rank - b.match.rank || tie(b.entry) - tie(a.entry) || a.i - b.i);
  return out.map(({ entry, match }) => ({ entry, match }));
}

/** "{n} suggestions" for the live line. */
export const suggestionsLine = (n: number): string => (n === 0 ? 'No suggestions' : n === 1 ? '1 suggestion' : `${n} suggestions`);
