// 0.9.0 V1: the words and small rules behind ReviewItemView (Mira, release-090/designs/v1-review-inbox.md
// §3-§9; Ada's D2/D3). Pure functions, so tests can pin the copy. Nothing here computes a new number:
// every count shown comes from the server (criteria_summary, decisions.total, preview rows) or is the
// length of a list the server sent.

import type { CriterionStatus, InboxItem, ReviewItem, VerdictResult } from './api/inbox';
import { VERDICT_WORD } from './api/inbox';
import type { RestorePreview, RestoreResult } from './missions';
import { TIER_LABELS } from './flightSchool';

/** What the item view hands back when an action is done (Pixel's pane removes the item, opens the next). */
export interface ReviewDone {
  kind: 'approved' | 'sent_back' | 'rolled_back' | 'stale';
  /** The status-line sentence, already worded. */
  text: string;
  task_id: string;
  /** rolled_back by restore: the safety checkpoint, for "Undo the roll back". */
  undo_checkpoint_id?: string | null;
  /** rolled_back after approval: the copy waiting on hold. */
  copy_task_id?: string | null;
  /** stale: the paths that changed after the check. */
  paths?: string[];
}

export type ReviewEntry = 'inbox' | 'board' | 'dev';

const isStr = (v: unknown): v is string => typeof v === 'string';
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const isArr = Array.isArray;

/**
 * Whether the item has every field the view draws. Anything else falls back to the plain card (title and
 * summary as text), never a half-drawn verdict.
 */
export function validReviewItem(i: InboxItem | null | undefined): i is ReviewItem {
  if (!i || i.kind !== 'review' || !isStr(i.task_id) || !i.task_id || !isStr(i.title)) return false;
  const v = i.verdict;
  if (!v || !(v.result in VERDICT_WORD) || !isStr(v.line)) return false;
  if (!i.round || !isNum(i.round.n) || !isNum(i.round.max)) return false;
  if (!i.criteria_summary || !isNum(i.criteria_summary.met) || !isNum(i.criteria_summary.total)) return false;
  if (!i.decisions || !isNum(i.decisions.total) || !isArr(i.decisions.top)) return false;
  if (!i.evidence || !isArr(i.evidence.checks) || !isArr(i.evidence.screenshots)) return false;
  return isArr(i.criteria) && isArr(i.couldnt_verify) && isArr(i.held_actions) && !!i.risk;
}

/** "14:41" in the person's own clock, or null for a missing or bad time. */
export function clock(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
}

/** "Needs you · medium confidence"'s second half, or null (no verdict, or no confidence). */
export function confidenceText(v: ReviewItem['verdict']): string | null {
  if (v.result === 'none' || !v.confidence) return null;
  return `${v.confidence} confidence`;
}

/**
 * The verdict box's meta line (Mira §3.2): who checked it, on what, when, and how many steps. Null with
 * no checker (Not checked yet).
 */
export function checkerMeta(i: Pick<ReviewItem, 'verdict' | 'risk'>): string | null {
  const v = i.verdict;
  if (!v.checker || v.result === 'none') return null;
  const at = clock(v.checked_at);
  const parts: string[] = [];
  if (v.checker.tier_note === 'different_tier') parts.push(`Checked on a different tier of ${v.checker.engine} (no other engine set up)`);
  else parts.push(`Checked by fresh eyes on ${v.checker.engine}${at ? ` at ${at}` : ''}`);
  if (isNum(v.steps_run) && isNum(v.steps_total) && v.steps_total > 0) {
    if (v.steps_run === v.steps_total) parts.push(i.risk.level === 'high' ? `all ${v.steps_total} steps, because it’s high risk` : `all ${v.steps_total} steps`);
    else parts.push(`${v.steps_run} of ${v.steps_total} steps`);
  }
  return parts.join(' · ');
}

export const RISK_WORD: Record<'low' | 'medium' | 'high', string> = { low: 'Low risk', medium: 'Medium risk', high: 'High risk' };

export const CRITERION_WORD: Record<CriterionStatus, string> = { met: 'Met', not_met: 'Not met', not_checked: 'Not checked', claim_only: 'Claim only' };

/** "3 of 4 met · 1 not met · 1 not checked", from the server's summary. */
export function criteriaSummaryText(s: ReviewItem['criteria_summary']): string {
  const parts = [`${s.met} of ${s.total} met`];
  if (s.not_met) parts.push(`${s.not_met} not met`);
  if (s.not_checked) parts.push(`${s.not_checked} not checked`);
  return parts.join(' · ');
}

/** "2 for you, of 4". */
export const decisionsSummaryText = (d: ReviewItem['decisions']): string => `${d.top.length} for you, of ${d.total}`;

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "3 checks · 1 screenshot · shop #512". */
export function evidenceSummaryText(e: ReviewItem['evidence']): string {
  const parts: string[] = [];
  if (e.checks.length) parts.push(plural(e.checks.length, 'check', 'checks'));
  if (e.screenshots.length) parts.push(plural(e.screenshots.length, 'screenshot', 'screenshots'));
  if (e.pr) parts.push(`${e.pr.repo} #${e.pr.number}`);
  return parts.length ? parts.join(' · ') : 'Nothing attached';
}

/** The last `n` lines of a check's output, and whether there was more. */
export function tailLines(out: string, n = 12): { lines: string; more: boolean } {
  const all = out.replace(/\s+$/, '').split('\n');
  return all.length > n ? { lines: all.slice(-n).join('\n'), more: true } : { lines: all.join('\n'), more: false };
}

/**
 * The criterion number a send-back note becomes ("Becomes criterion 5"): one after the highest index the
 * server sent.
 */
export const nextCriterionIndex = (criteria: ReviewItem['criteria']): number => criteria.reduce((m, c) => Math.max(m, c.index), 0) + 1;

/** "Change: {question} → ", the Send back prefill from a decision's Change button. */
export const changePrefill = (question: string): string => `Change: ${question} → `;

export const lastRound = (r: ReviewItem['round']): boolean => r.n >= r.max;

/** The Send back panel's "what happens" list (Mira §6), in order. */
export function sendBackSteps(i: Pick<ReviewItem, 'kwant_name' | 'round' | 'held_actions' | 'evidence'>, openHeld: number, noChecks = false): string[] {
  const out: string[] = [];
  if (openHeld > 0) out.push(heldDeniedFirst(i, openHeld));
  if (noChecks) {
    // A run with no checks has no checker: the note and the work so far are all the next round gets.
    out.push(`${i.kwant_name} starts round ${i.round.n + 1} with your note and the work so far.`);
  } else {
    out.push(`${i.kwant_name} starts round ${i.round.n + 1} with your note, the checker’s findings and the work so far.`);
    out.push('The checker then checks everything again, including what passed.');
  }
  out.push(`This is round ${i.round.n} of ${i.round.max}. After round ${i.round.max} it comes back to you, whatever the verdict.`);
  return out;
}

function heldDeniedFirst(i: Pick<ReviewItem, 'held_actions' | 'evidence'>, openHeld: number): string {
  const h = i.held_actions.find((x) => x.status === 'pending');
  if (openHeld === 1 && h?.gate === 'merge' && i.evidence.pr) return `The held merge of #${i.evidence.pr.number} is denied first, so nothing runs.`;
  return openHeld === 1 ? 'The held action is denied first, so nothing runs.' : 'The held actions are denied first, so nothing runs.';
}

export const GATE_TEXT: Record<'merge' | 'payment' | 'approval', string> = {
  merge: 'held at the merge gate',
  payment: 'payment floor',
  approval: 'held for your approval',
};

// ---- Roll back ----

/** "Not undone: …" heading for the roll back dialog, or null when nothing left the computer and kwant knows it. */
export function notUndoneTitle(p: Pick<RestorePreview, 'effects_known' | 'external_not_undone'>): string | null {
  if (!p.effects_known) return 'Not undone: unknown';
  const n = p.external_not_undone.length;
  if (!n) return null;
  return `Not undone: ${n === 1 ? '1 thing' : `${n} things`} already left this computer`;
}

export const NOT_UNDONE_UNKNOWN =
  'Files changed, but no logged call explains the change, so something may have left this computer. Check the chat before you rely on this roll back.';

/** "Roll back 3 files" (the preview's own change rows). */
export const rollBackLabel = (p: Pick<RestorePreview, 'changes'>): string => (p.changes.length === 1 ? 'Roll back 1 file' : `Roll back ${p.changes.length} files`);

/** The step-down line after approval, or null. */
export function stepDownText(s: RestorePreview['step_down']): { cls: string; from: string; to: string } | null {
  if (!s) return null;
  const word = (t: string) => (TIER_LABELS as Record<string, string>)[t] ?? t;
  return { cls: s.class, from: word(s.from), to: word(s.to) };
}

/** "Rolled back: 2 files put back, 1 removed. The task is on hold until you press Run." */
export function rolledBackText(r: Pick<RestoreResult, 'restored' | 'deleted' | 'failed' | 'task'>): string {
  const parts: string[] = [];
  if (r.restored.length) parts.push(`${plural(r.restored.length, 'file', 'files')} put back`);
  if (r.deleted.length) parts.push(`${r.deleted.length} removed`);
  const what = parts.length ? `Rolled back: ${parts.join(', ')}.` : 'Rolled back. No files needed changing.';
  const failed = r.failed.length ? ` ${plural(r.failed.length, 'file', 'files')} couldn’t be written.` : '';
  const held = r.task?.held_at ? ' The task is on hold until you press Run.' : '';
  return `${what}${failed}${held}`;
}

/** After approval: "Rolled back. The copy is on hold until you press Run. Lin is now at Sandbox for auth changes." */
export function rolledBackForkText(bot: string, stepped: RestorePreview['step_down']): string {
  const s = stepDownText(stepped ?? null);
  return `Rolled back. The copy is on hold until you press Run.${s ? ` ${bot} is now at ${s.to} for ${s.cls}.` : ''}`;
}

export const verdictWord = (r: VerdictResult): string => VERDICT_WORD[r];
