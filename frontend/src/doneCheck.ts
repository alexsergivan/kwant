// 0.9.8 done-check (autonomy/done-check/design.md, Mira): the "Done, and here's the check" box and the list
// row marker read the `checked` object Vega's #566 puts on every done item in GET /api/inbox. Server text is
// text only; nothing here computes a new number, it only picks words for the server's state and results.

export type CheckState = 'checked' | 'partly' | 'not_checked' | 'failed';
export type CheckResult = 'passed' | 'failed' | 'waiting' | 'not_checked';

export interface CheckedItem {
  what: string;
  how: string;
  result: CheckResult | string;
  evidence: string | null;
}

export interface Checked {
  state: CheckState | string;
  items: CheckedItem[];
  items_dropped: number;
  line: string;
}

const STATES: readonly CheckState[] = ['checked', 'partly', 'not_checked', 'failed'];

/** The `checked` object when it has a known state; null for an older server or anything malformed. */
export function knownChecked(v: unknown): (Checked & { state: CheckState }) | null {
  if (!v || typeof v !== 'object') return null;
  const c = v as Partial<Checked>;
  if (typeof c.state !== 'string' || !(STATES as readonly string[]).includes(c.state)) return null;
  const items = Array.isArray(c.items)
    ? c.items.filter((i): i is CheckedItem => !!i && typeof i === 'object' && typeof (i as CheckedItem).what === 'string')
    : [];
  const dropped = typeof c.items_dropped === 'number' && Number.isFinite(c.items_dropped) && c.items_dropped > 0 ? Math.floor(c.items_dropped) : 0;
  return { state: c.state as CheckState, items, items_dropped: dropped, line: typeof c.line === 'string' ? c.line : '' };
}

/** design.md section 4: the line when the server's is blank. */
export const FALLBACK_LINE: Record<CheckState, string> = {
  checked: 'Checked: every check passed.',
  partly: 'Partly checked: some checks passed.',
  failed: 'Failed: a check failed.',
  not_checked: 'Not checked: nothing verified this result.',
};

/** Splits "Partly checked: 1 of 2 …" into its prefix (with the colon) and the rest; no match draws it whole. */
export function splitLine(state: CheckState, line: string): { prefix: string | null; rest: string } {
  const text = line.trim() ? line.trim() : FALLBACK_LINE[state];
  const m = /^(Checked|Partly checked|Failed|Not checked):\s*/.exec(text);
  if (!m) return { prefix: null, rest: text };
  return { prefix: `${m[1]}:`, rest: text.slice(m[0].length) };
}

/** Box tone per state (design.md section 1 table). */
export const STATE_STYLE: Record<CheckState, { box: string; icon: string; prefix: string; open: boolean }> = {
  checked: { box: 'border-teal-300/25 bg-teal-300/[0.05]', icon: 'text-ok', prefix: 'text-ok', open: false },
  partly: { box: 'border-amber-300/30 bg-amber-300/[0.05]', icon: 'text-warn', prefix: 'text-warn', open: false },
  failed: { box: 'border-rose-300/35 bg-rose-300/[0.06]', icon: 'text-bad', prefix: 'text-bad', open: true },
  not_checked: { box: 'border-dashed border-hairline-strong bg-transparent', icon: 'text-zinc-400', prefix: 'text-zinc-200', open: false },
};

export type ItemKind = 'passed' | 'failed' | 'waiting' | 'not_checked' | 'claim';

/** A claim: "marked met with no evidence (a claim)". Any unknown result reads as not checked, never passed. */
export function itemKind(i: Pick<CheckedItem, 'result' | 'how'>): ItemKind {
  if (i.result === 'passed' || i.result === 'failed' || i.result === 'waiting') return i.result;
  return typeof i.how === 'string' && i.how.includes('(a claim)') ? 'claim' : 'not_checked';
}

export const ITEM_WORD: Record<ItemKind, string> = {
  passed: 'Passed',
  failed: 'Failed',
  waiting: 'Still waiting',
  not_checked: 'Not checked',
  claim: 'Not checked',
};

export const ITEM_COLOUR: Record<ItemKind, string> = {
  passed: 'text-ok',
  failed: 'text-bad',
  waiting: 'text-zinc-300',
  not_checked: 'text-zinc-400',
  claim: 'text-warn',
};

/** how's first letter capitalised, unless it starts with "kwant". */
export function howText(how: string): string {
  const t = typeof how === 'string' ? how.trim() : '';
  if (!t || t.startsWith('kwant')) return t;
  return t.charAt(0).toUpperCase() + t.slice(1);
}

export const toggleLabel = (open: boolean, n: number): string => (open ? 'Hide what was checked' : `Show what was checked (${n})`);
export const droppedLine = (n: number): string => `${n} more not shown.`;
export const EMPTY_HELP = 'Read the answer before you rely on it.';
export const ADD_A_CHECK = 'Add a check';
export const SECTION_LABEL = 'How it was checked';

/** The review layout drops "What done looks like" when the box already lists the task's criteria. */
export function listsCriteria(c: Checked | null): boolean {
  return !!c && c.items.some((i) => typeof i.how === 'string' && i.how.startsWith('task criterion'));
}

/** List row marker words (design.md section 2). */
export const MARKER: Record<CheckState, { word: string; cls: string }> = {
  checked: { word: 'Checked', cls: '' },
  partly: { word: 'Partly checked', cls: 'font-semibold text-warn' },
  failed: { word: 'Check failed', cls: 'font-semibold text-bad' },
  not_checked: { word: 'Not checked', cls: 'font-medium text-zinc-300' },
};
