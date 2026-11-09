// 0.8.0 B3: the "Checked with fresh eyes" label (Mira b3-fresh-eyes.md §1, Juno fresh-eyes-audit.md §3).
// It says how a review was run; it isn't earned, so it never looks like a badge.

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

export const SAW_PARTS = ['brief', 'spec', 'result', 'diff'] as const;
export type SawPart = (typeof SAW_PARTS)[number];
export const SPEC_SOURCES = ['written', 'acceptance', 'brief', 'ticket', 'pr', 'none'] as const;
export type SpecSource = (typeof SPEC_SOURCES)[number];

export interface FreshEyes {
  checked: boolean;
  saw: SawPart[];
  spec_source: SpecSource;
  reviewer_kwant_id: string | null;
}

/** null for anything that isn't a label (cards from before 0.8.0 have none: R3). */
export function parseFreshEyes(v: unknown): FreshEyes | null {
  if (!isObj(v) || typeof v.checked !== 'boolean') return null;
  const saw: SawPart[] = [];
  for (const s of Array.isArray(v.saw) ? v.saw : []) {
    const p = SAW_PARTS.find((x) => x === s);
    if (p && !saw.includes(p)) saw.push(p);
  }
  const src = SPEC_SOURCES.find((x) => x === v.spec_source) ?? 'none';
  const rk = typeof v.reviewer_kwant_id === 'string' && v.reviewer_kwant_id.trim() ? v.reviewer_kwant_id.slice(0, 120) : null;
  return { checked: v.checked, saw, spec_source: src, reviewer_kwant_id: rk };
}

/** R1: the chip with its popover. R2: a grey "Saw the author's chat". R3: no chip. */
export type ChipState = 'R1' | 'R2' | 'R3';

export function chipState(fe: FreshEyes | null | undefined): ChipState {
  if (!fe) return 'R3';
  if (fe.checked) return 'R1';
  // A failed review is checked:false with nothing seen: no chip, never R2 (contract §3.3).
  return fe.saw.length > 0 ? 'R2' : 'R3';
}

export interface SawLine {
  ok: boolean;
  text: string;
}

const SPEC_TEXT: Record<SpecSource, string | null> = {
  ticket: "the ticket's brief and done-when lines",
  brief: "the task's brief",
  written: "the owner's written spec",
  acceptance: "the task's acceptance lines",
  pr: "only the PR's title and description, in the author's own words",
  none: null,
};

/**
 * The popover's list, in the server's order. `diff` is the diffstat the card already shows
 * (never recounted here); `ref` names the ticket when the card has one ("#212").
 */
export function sawLines(
  fe: FreshEyes,
  opts: { author: string | null; diff?: { files: number; add: number; del: number } | null; ref?: string | null },
): SawLine[] {
  const out: SawLine[] = [];
  let briefDone = false;
  for (const part of fe.saw) {
    if (part === 'brief' || part === 'spec') {
      if (briefDone) continue;
      briefDone = true;
      const base = SPEC_TEXT[fe.spec_source] ?? (part === 'spec' ? 'the spec' : 'the brief');
      out.push({ ok: true, text: opts.ref && fe.spec_source === 'ticket' ? `${base} (${opts.ref})` : base });
    } else if (part === 'diff') {
      const d = opts.diff;
      out.push({
        ok: true,
        text: d ? `the diff (${d.files} ${d.files === 1 ? 'file' : 'files'}, +${d.add} −${d.del})` : 'the diff',
      });
    } else if (part === 'result') {
      out.push({ ok: true, text: 'the result' });
    }
  }
  // `pr` reviews see no brief entry in `saw`, only the diff; still say what the brief was.
  if (!briefDone && fe.spec_source === 'pr') out.unshift({ ok: true, text: SPEC_TEXT.pr! });
  if (fe.checked) out.push({ ok: false, text: `Not ${opts.author ?? "the author"}'s conversation or reasoning` });
  return out;
}

export const popoverTitle = (reviewer: string | null): string =>
  `${reviewer ?? 'The reviewer'} reviewed with fresh eyes. They saw only what a new teammate would get:`;
