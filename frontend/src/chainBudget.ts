// The chain token budget: when one person message's chain of hand-offs goes over it, kwnt stops
// handing the turn on and posts one notice (message meta kind "budget_paused") with Continue.
import { request } from './threads';

export const BUDGET_MAX = 100_000_000;

export interface BudgetPausedMeta {
  kind: 'budget_paused';
  /** Fresh tokens the chain had used. */
  used: number;
  budget: number;
  /** Teammates who were about to be handed the turn. */
  waiting: string[];
  resumed: boolean;
}

export function parseBudgetPaused(meta: string | null | undefined): BudgetPausedMeta | null {
  if (!meta) return null;
  try {
    const m = JSON.parse(meta);
    if (!m || m.kind !== 'budget_paused') return null;
    const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
    return {
      kind: 'budget_paused',
      used: n(m.used),
      budget: n(m.budget),
      waiting: Array.isArray(m.waiting) ? m.waiting.filter((w: unknown): w is string => typeof w === 'string') : [],
      resumed: m.resumed === true,
    };
  } catch {
    return null;
  }
}

/** Error text for the budget field, or null for a whole number from 0 (off) to BUDGET_MAX. */
export function budgetError(raw: string): string | null {
  const s = raw.trim().replace(/[ _,]/g, '');
  if (!/^\d+$/.test(s)) return 'Enter a whole number of tokens, or 0 to turn it off.';
  if (Number(s) > BUDGET_MAX) return `At most ${BUDGET_MAX.toLocaleString('en-US')}.`;
  return null;
}

/** The field's text as a number, once `budgetError` is null. */
export const budgetValue = (raw: string): number => Number(raw.trim().replace(/[ _,]/g, ''));

export const chainBudgetApi = {
  /** Hands the turn to the teammates the notice was holding back (person only). */
  continue: (noticeId: string) => request<{ status: string }>('POST', `/api/chains/${encodeURIComponent(noticeId)}/continue`),
};
