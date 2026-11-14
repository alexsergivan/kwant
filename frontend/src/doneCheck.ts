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
  /** request-parts (#619, 0.9.11): present only when the task has criteria. Every field is optional, so an older
   * server draws today's card. `parts_written: false` means each criterion stands in as a part. */
  parts_checked?: number;
  parts_total?: number;
  parts_written?: boolean;
  parts?: CheckedPart[];
}

export type PartResult = 'passed' | 'failed' | 'not_checked';

export interface CheckedCriterion {
  id: string;
  text: string;
  result: PartResult | string;
}

export interface CheckedPart {
  /** The person's own words for this part of the request. */
  text: string;
  result: PartResult | string;
  criteria: CheckedCriterion[];
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
  const out: Checked & { state: CheckState } = { state: c.state as CheckState, items, items_dropped: dropped, line: typeof c.line === 'string' ? c.line : '' };
  if (typeof c.parts_written === 'boolean') out.parts_written = c.parts_written;
  if (typeof c.parts_total === 'number' && Number.isFinite(c.parts_total)) out.parts_total = c.parts_total;
  if (typeof c.parts_checked === 'number' && Number.isFinite(c.parts_checked)) out.parts_checked = c.parts_checked;
  if (Array.isArray(c.parts)) out.parts = c.parts.flatMap((p) => knownPart(p));
  return out;
}

function knownPart(v: unknown): CheckedPart[] {
  if (!v || typeof v !== 'object') return [];
  const p = v as Partial<CheckedPart>;
  if (typeof p.text !== 'string' || !p.text.trim()) return [];
  const criteria = Array.isArray(p.criteria)
    ? p.criteria.filter((x): x is CheckedCriterion => !!x && typeof x === 'object' && typeof (x as CheckedCriterion).text === 'string' && (x as CheckedCriterion).text.trim() !== '')
    : [];
  return [{ text: p.text, result: typeof p.result === 'string' ? p.result : 'not_checked', criteria }];
}

/**
 * Which layout the parts call for (request-parts ui-brief.md):
 * - `many`: written parts, two or more: one row per part, then the checks under "How it was checked".
 * - `one`: a single written part: named, with no counter.
 * - `null`: no parts, or `parts_written` false/missing: today's card, with no "parts" words.
 */
export function partsLayout(c: Pick<Checked, 'parts' | 'parts_total' | 'parts_written'>): 'many' | null {
  if (c.parts_written !== true || !c.parts) return null;
  // One written part: today's card; the server line already names it (design-check-ui.md fix 4).
  return typeof c.parts_total === 'number' && c.parts_total >= 2 && c.parts.length >= 2 ? 'many' : null;
}

/** A part's or criterion's result; anything unknown reads as "no check", never passed. */
export function partKind(result: unknown): PartResult {
  return result === 'passed' || result === 'failed' ? result : 'not_checked';
}

/** impl.md "Notes for Mira's card": copy to reuse. */
export const PART_WORD: Record<PartResult, string> = { passed: 'Passed', failed: 'Not met', not_checked: 'No check' };
export const PART_COLOUR: Record<PartResult, string> = { passed: 'text-ok', failed: 'text-bad', not_checked: 'text-warn' };
export const PARTS_HEADING = 'Parts of your request';
export const NO_CRITERIA_HELP = 'Nothing checked this part.';
export const criteriaToggle = (open: boolean, n: number): string => (open ? 'Hide what proves it' : `Show what proves it (${n})`);

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
