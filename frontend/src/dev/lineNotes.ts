// 0.9.0 G1 line notes on a diff (Mira, release-090/designs/g1-line-notes.md; Juno's routes in
// release-090/g1-api.md). Types, parsers and the pure logic behind the Changes section of a review card.
// Every parse* takes the route's raw JSON and returns typed data (strings trimmed, never rendered as
// HTML), or null when it can't be drawn honestly. Nothing here computes a new figure for display: the
// counts on screen are the server's, or the length of a list the person can see.
//
// Built to Juno's answers (g1-api.md, 07:20 UTC), all 12 asks accepted:
// - A note's anchor: `start`/`end` objects ({side, line}, ask 3); `code` is the noted lines as diff lines.
// - Where a note sits in the current diff: `at_head` (ask 5), null when its lines aren't there; `seen_sha`
//   is the newer head (draft: "pushed … after you wrote this" with at_head, "gone" without).
// - Errors: `{error, code}` (ask 7); the UI branches on `code`.
// - Omitted patches: `omitted` (ask 8). `cut`: `{files_not_read}` (ask 9).

export type Side = 'new' | 'old';
export type LineKind = 'ctx' | 'add' | 'del';
export type NoteState = 'draft' | 'sent' | 'resolved' | 'open';
export type PrState = 'open' | 'merged' | 'closed';

export interface DiffLine {
  kind: LineKind;
  old: number | null;
  new: number | null;
  text: string;
}
export interface Hunk {
  header: string;
  lines: DiffLine[];
}
export interface ChangedFile {
  path: string;
  old_path: string | null;
  status: string;
  additions: number;
  deletions: number;
  omitted: 'binary' | 'too_large' | null;
  big: boolean;
  notes: number;
  hunks: Hunk[];
}
export interface Changes {
  card_id: string;
  repo: string | null;
  pr: number | null;
  head_sha: string | null;
  pr_url: string | null;
  pr_state: PrState;
  author_bot: { id: string; name: string } | null;
  author_login: string | null;
  files: ChangedFile[];
  /** Files the server didn't read (ask 9). */
  cut: { files_not_read: number } | null;
}

export interface LinePoint {
  side: Side;
  line: number;
}
export interface Note {
  id: string;
  path: string;
  start: LinePoint;
  end: LinePoint;
  /** The noted lines as written on, in the /changes line shape (the UI draws the markers). */
  code: DiffLine[];
  text: string;
  state: NoteState;
  head_sha: string | null;
  created_at: string | null;
  updated_at: string | null;
  sent_at: string | null;
  message_id: string | null;
  checked_sha: string | null;
  /** Where the note sits in the card's current head diff; null: not in it (shown at the top of its file). */
  at_head: { start: LinePoint; end: LinePoint } | null;
  /** The card's head when it differs from the note's own head_sha. */
  seen_sha: string | null;
  /** A draft written on an older head whose lines are still in the diff (maybe at other numbers). */
  moved: boolean;
  gone: boolean;
}
export interface NotesSummary {
  total: number;
  changed: number;
  open: number;
  sha: string | null;
}
export interface NotesPage {
  notes: Note[];
  summary: NotesSummary | null;
}
export interface SendResult {
  sent: number;
  message_id: string | null;
  kwant_id: string | null;
  kwant_name: string | null;
  sent_at: string | null;
  notes: Note[];
}

// ---------------------------------------------------------------------------------------------------
// Parsing

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const str = (v: unknown, max: number): string | null => (typeof v === 'string' && v.trim() ? v.slice(0, max) : null);
/** Code keeps its spaces: an empty or blank line is a line too. */
const code = (v: unknown, max = 2000): string | null => (typeof v === 'string' ? v.slice(0, max) : null);
const whole = (v: unknown): number | null => (typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : null);
const lineNo = (v: unknown): number | null => (typeof v === 'number' && Number.isInteger(v) && v >= 1 ? v : null);
const side = (v: unknown): Side | null => (v === 'new' || v === 'old' ? v : null);

/** Lines past this are not drawn (the server cuts big files long before). */
export const MAX_LINES_PER_FILE = 5000;

function diffLine(v: unknown): DiffLine | null {
  if (!isObj(v)) return null;
  const kind = v.kind === 'ctx' || v.kind === 'add' || v.kind === 'del' ? v.kind : null;
  const text = code(v.text);
  if (!kind || text === null) return null;
  const oldN = lineNo(v.old);
  const newN = lineNo(v.new);
  // Every line needs the number of the side it lives on.
  if (kind === 'add' && newN === null) return null;
  if (kind === 'del' && oldN === null) return null;
  if (kind === 'ctx' && (newN === null || oldN === null)) return null;
  return { kind, old: kind === 'add' ? null : oldN, new: kind === 'del' ? null : newN, text };
}

function changedFile(v: unknown): ChangedFile | null {
  if (!isObj(v)) return null;
  const path = str(v.path, 600);
  if (!path) return null;
  const omitted = v.omitted === 'binary' || v.omitted === 'too_large' ? v.omitted : null;
  let budget = MAX_LINES_PER_FILE;
  const hunks: Hunk[] = [];
  for (const h of arr(v.hunks)) {
    if (!isObj(h) || budget <= 0) continue;
    const lines = arr(h.lines)
      .map(diffLine)
      .filter((l): l is DiffLine => !!l)
      .slice(0, budget);
    budget -= lines.length;
    if (lines.length) hunks.push({ header: str(h.header, 400) ?? '', lines });
  }
  return {
    path,
    old_path: str(v.old_path, 600),
    status: str(v.status, 20) ?? 'modified',
    additions: whole(v.additions) ?? 0,
    deletions: whole(v.deletions) ?? 0,
    omitted,
    big: v.big === true,
    notes: whole(v.notes) ?? 0,
    hunks: omitted ? [] : hunks,
  };
}

export function parseChanges(raw: unknown): Changes | null {
  if (!isObj(raw) || !Array.isArray(raw.files)) return null;
  const ab = isObj(raw.author_bot) ? raw.author_bot : null;
  const abId = ab ? str(ab.id, 120) : null;
  const abName = ab ? str(ab.name, 80) : null;
  const notRead = isObj(raw.cut) ? whole(raw.cut.files_not_read) : null;
  return {
    card_id: str(raw.card_id, 200) ?? '',
    repo: str(raw.repo, 300),
    pr: whole(raw.pr),
    head_sha: str(raw.head_sha, 64),
    pr_url: str(raw.pr_url, 600) && /^https:\/\//.test(raw.pr_url as string) ? (raw.pr_url as string).slice(0, 600) : null,
    pr_state: raw.pr_state === 'merged' || raw.pr_state === 'closed' ? raw.pr_state : 'open',
    author_bot: abId && abName ? { id: abId, name: abName } : null,
    author_login: str(raw.author_login, 80),
    files: raw.files.map(changedFile).filter((f): f is ChangedFile => !!f),
    cut: notRead ? { files_not_read: notRead } : null,
  };
}

function point(v: unknown): LinePoint | null {
  if (!isObj(v)) return null;
  const s = side(v.side);
  const l = lineNo(v.line);
  return s && l ? { side: s, line: l } : null;
}

/** `{start, end}` objects (ask 3). */
function span(v: Record<string, unknown>): { start: LinePoint; end: LinePoint } | null {
  const a = point(v.start);
  const b = point(v.end);
  return a && b ? { start: a, end: b } : null;
}

export function parseNote(v: unknown): Note | null {
  if (!isObj(v)) return null;
  const id = str(v.id, 120);
  const path = str(v.path, 600);
  const where = span(v);
  if (!id || !path || !where) return null;
  const state: NoteState = v.state === 'sent' || v.state === 'resolved' || v.state === 'open' ? v.state : 'draft';
  const headSha = str(v.head_sha, 64);
  const atHead = isObj(v.at_head) ? span(v.at_head) : null;
  const seen = str(v.seen_sha, 64);
  return {
    id,
    path,
    start: where.start,
    end: where.end,
    code: arr(v.code)
      .map(diffLine)
      .filter((c): c is DiffLine => !!c)
      .slice(0, 200),
    text: typeof v.text === 'string' ? v.text.slice(0, 4000) : '',
    state,
    head_sha: headSha,
    created_at: str(v.created_at, 64),
    updated_at: str(v.updated_at, 64),
    sent_at: str(v.sent_at, 64),
    message_id: str(v.message_id, 120),
    checked_sha: str(v.checked_sha, 64),
    at_head: atHead,
    seen_sha: seen,
    moved: state === 'draft' && !!seen && !!atHead,
    gone: state === 'draft' && !!seen && !atHead,
  };
}

export function parseNotes(raw: unknown): NotesPage | null {
  if (!isObj(raw) || !Array.isArray(raw.notes)) return null;
  const s = isObj(raw.summary) ? raw.summary : null;
  const total = s ? whole(s.total) : null;
  return {
    notes: raw.notes.map(parseNote).filter((n): n is Note => !!n),
    summary:
      s && total !== null ? { total, changed: whole(s.changed) ?? 0, open: whole(s.open) ?? 0, sha: str(s.sha, 64) } : null,
  };
}

export function parseSendResult(raw: unknown): SendResult | null {
  if (!isObj(raw)) return null;
  const sent = whole(raw.sent);
  if (sent === null) return null;
  return {
    sent,
    message_id: str(raw.message_id, 120),
    kwant_id: str(raw.kwant_id, 120),
    kwant_name: str(raw.kwant_name, 80),
    sent_at: str(raw.sent_at, 64),
    notes: arr(raw.notes)
      .map(parseNote)
      .filter((n): n is Note => !!n),
  };
}

/** Ask 1: the card's own `changes` and `notes` counts, so the collapsed row needs no fetch. */
export function parseCardChanges(v: unknown): { files: number; additions: number; deletions: number } | null {
  if (!isObj(v)) return null;
  const files = whole(v.files);
  if (files === null) return null;
  return { files, additions: whole(v.additions) ?? 0, deletions: whole(v.deletions) ?? 0 };
}
export function parseCardNotes(v: unknown): { drafts: number; total: number } | null {
  if (!isObj(v)) return null;
  const total = whole(v.total);
  if (total === null) return null;
  return { drafts: whole(v.drafts) ?? 0, total };
}

// ---------------------------------------------------------------------------------------------------
// Lines and ranges

/** The side and number a line is anchored by: removed lines on the old side, the rest on the new. */
export function pointOf(l: DiffLine): LinePoint {
  return l.kind === 'del' ? { side: 'old', line: l.old! } : { side: 'new', line: l.new! };
}

const MARK: Record<LineKind, string> = { add: '+', del: '-', ctx: ' ' };
/** Quoted code as plain text, one diff marker per line (never HTML). */
export const codeText = (lines: readonly DiffLine[]) => lines.map((l) => `${MARK[l.kind]} ${l.text}`).join('\n');

/** The number shown for a line on the phone (new side; removed lines show their old number). */
export const shownNo = (l: DiffLine): number => (l.kind === 'del' ? l.old! : l.new!);

/** A file's lines in order, with the hunk each belongs to. Hunk headers are not lines. */
export interface FlatLine {
  i: number;
  hunk: number;
  line: DiffLine;
}
export function flatLines(f: ChangedFile): FlatLine[] {
  const out: FlatLine[] = [];
  f.hunks.forEach((h, hi) => h.lines.forEach((line) => out.push({ i: out.length, hunk: hi, line })));
  return out;
}

const samePoint = (a: LinePoint, b: LinePoint) => a.side === b.side && a.line === b.line;

/** Index of the line with this anchor, or -1. */
export function indexOfPoint(lines: FlatLine[], p: LinePoint): number {
  return lines.findIndex((x) => samePoint(pointOf(x.line), p));
}

/** Design board: a range covers at most 40 lines. */
export const MAX_RANGE = 40;

/**
 * The range from `anchor` toward `to`, kept inside the anchor's hunk (a note never crosses hunks) and at
 * most MAX_RANGE lines. Returns [from, to] in order.
 */
export function clampRange(lines: FlatLine[], anchor: number, to: number): [number, number] {
  const hunk = lines[anchor]?.hunk;
  let t = Math.max(0, Math.min(lines.length - 1, to));
  while (t !== anchor && lines[t].hunk !== hunk) t += t > anchor ? -1 : 1;
  if (Math.abs(t - anchor) >= MAX_RANGE) t = anchor + (t > anchor ? MAX_RANGE - 1 : -(MAX_RANGE - 1));
  return t < anchor ? [t, anchor] : [anchor, t];
}

/** "line 42" / "lines 40–42", by the numbers shown in the gutter. */
export function lineWords(a: LinePoint, b: LinePoint): string {
  return samePoint(a, b) ? `line ${a.line}` : `lines ${a.line}–${b.line}`;
}

/** Where a note sits in the diff being shown: [first, last] indexes, or null (top of the file). */
export function positionIn(lines: FlatLine[], n: Note): [number, number] | null {
  if (!n.at_head) return null;
  const a = indexOfPoint(lines, n.at_head.start);
  const b = indexOfPoint(lines, n.at_head.end);
  if (a < 0 || b < 0 || b < a) return null;
  return [a, b];
}

// ---------------------------------------------------------------------------------------------------
// Copy (Mira's "Copy (all)"; no emoji, no HTML)

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export const MAX_SEND = 20;
export const sha7 = (s: string | null) => (s ? s.trim().slice(0, 7) : '');

export const filesWord = (n: number) => plural(n, 'file');
export const notesWord = (n: number) => plural(n, 'note');
export const draftsWord = (n: number) => plural(n, 'draft');
export const addAria = (l: DiffLine) => (l.kind === 'del' ? `Add a note on removed line ${l.old}` : `Add a note on line ${l.new}`);
/** With no bot yet (no default, no pick: Fix it "Send to…" §7) the label ends in "…" and opens the picker. */
export const sendLabel = (n: number, bot: string | null) =>
  bot ? (n === 1 ? `Send note to ${bot}` : `Send ${n} notes to ${bot}`) : n === 1 ? 'Send note…' : `Send ${n} notes…`;
export const sendShort = (n: number) => `Send ${n}`;
export const sentLine = (n: number, bot: string, at: string | null) => `Sent ${notesWord(n)} to ${bot}${at ? ` · ${at}` : ''}`;
export const discardAsk = (n: number) => (n === 1 ? 'Discard 1 draft?' : `Discard ${n} drafts?`);
export const SEND_FAILED = 'The notes didn’t send. They’re still here.';
export const TOO_MANY = 'Up to 20 notes per send';
export const LOAD_FAILED = 'Couldn’t load the changes.';
export const NO_DIFF = 'No changes to show: GitHub sent no diff for these files.';
export const notBotLine = (login: string | null) =>
  `Notes go to bots in this space. This PR is by ${login ?? 'someone outside this space'} and no team owns this repo. Add it to a team (Manage team › Git Repo) and its lead gets the notes.`;
export const readOnlyLine = (s: PrState) => (s === 'merged' ? 'This PR is merged. Notes are read-only.' : 'This PR is closed. Notes are read-only.');
export const sentNoteLine = (at: string | null, bot: string) => `Sent${at ? ` ${at}` : ''} · waiting for ${bot}’s next push`;
export const changedIn = (sha: string | null) => `Changed in ${sha7(sha)}`.trimEnd();
export const stillOpen = (sha: string | null) => `Still open · line unchanged in ${sha7(sha)}`.trimEnd();
export const movedLine = (bot: string, sha: string | null, to: number | null) =>
  `${bot} pushed ${sha7(sha) || 'a new commit'} after you wrote this.${to !== null ? ` The line is now ${to}.` : ''}`;
export const goneLine = (sha: string | null) => `This line is gone in ${sha7(sha) || 'the newest push'}.`;
export const summaryLine = (s: NotesSummary) =>
  `${notesWord(s.total)} · ${s.changed} changed in ${sha7(s.sha)} · ${s.open} still open`;
export const addedSay = (at: string, drafts: number, bot: string) => `Note added on ${at}. ${notesWord(drafts)} for ${bot}.`;
export const resolvedSay = (s: NotesSummary) => `${s.changed} ${s.changed === 1 ? 'note' : 'notes'} changed in ${sha7(s.sha)}, ${s.open} still open.`;
export const showLines = (n: number) => `Show ${n.toLocaleString('en-US')} ${n === 1 ? 'line' : 'lines'}`;
export const cutLine = (c: NonNullable<Changes['cut']>) => `${plural(c.files_not_read, 'file')} not read: the diff was cut.`;

/** The note's state line for its header (drafts say "Draft"). */
export function stateWords(n: Note, bot: string, clockOf: (iso: string | null) => string | null): string {
  switch (n.state) {
    case 'draft':
      return 'Draft';
    case 'sent':
      return sentNoteLine(clockOf(n.sent_at), bot);
    case 'resolved':
      return changedIn(n.checked_sha);
    case 'open':
      return stillOpen(n.checked_sha);
  }
}

/** Lockfiles start collapsed like big files (design "Big file"). */
export const LOCKFILE = /(^|\/)(package-lock\.json|yarn\.lock|pnpm-lock\.yaml|Cargo\.lock|Gemfile\.lock|poetry\.lock|composer\.lock|go\.sum|bun\.lockb?)$/;
export const startsCollapsed = (f: ChangedFile) => f.big || LOCKFILE.test(f.path);
