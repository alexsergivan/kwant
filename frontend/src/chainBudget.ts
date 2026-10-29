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

/** Hand-offs past the per-message caps: kwnt holds them in one notice (meta kind "handoff_capped") with Continue. */
export interface HandoffCappedMeta {
  kind: 'handoff_capped';
  /** Hand-offs this message already started. */
  sent: number;
  /** Teammates not handed the turn yet. */
  waiting: string[];
  resumed: boolean;
}

export function parseHandoffCapped(meta: string | null | undefined): HandoffCappedMeta | null {
  if (!meta) return null;
  try {
    const m = JSON.parse(meta);
    if (!m || m.kind !== 'handoff_capped') return null;
    return {
      kind: 'handoff_capped',
      sent: typeof m.sent === 'number' && Number.isFinite(m.sent) && m.sent > 0 ? Math.floor(m.sent) : 0,
      waiting: Array.isArray(m.waiting) ? m.waiting.filter((w: unknown): w is string => typeof w === 'string' && w.trim() !== '') : [],
      resumed: m.resumed === true,
    };
  } catch {
    return null;
  }
}

/** The notice's plain line: "Paused after 16 hand-offs so it doesn't loop. Continue?" */
export function handoffCappedText(meta: HandoffCappedMeta): { headline: string; detail: string | null } {
  const n = meta.sent;
  const headline = n > 0 ? `Paused after ${n} hand-off${n === 1 ? '' : 's'} so it doesn't loop.` : "Paused so the hand-offs don't loop.";
  const who = meta.waiting.length <= 3 ? meta.waiting.join(', ') : `${meta.waiting.slice(0, 3).join(', ')} and ${meta.waiting.length - 3} more`;
  if (meta.resumed) return { headline, detail: who ? `Continued: handed on to ${who}.` : 'Continued.' };
  return { headline: `${headline} Continue?`, detail: who ? `Waiting: ${who}` : null };
}

const httpStatus = (e: unknown): number | undefined => {
  const status = e instanceof Error ? (e as Error & { status?: unknown }).status : undefined;
  return typeof status === 'number' ? status : undefined;
};

/** 409: someone (or a typed "continue") already carried the chain on. */
export const isAlreadyContinued = (e: unknown): boolean =>
  httpStatus(e) === 409 || (e instanceof Error && /already continued/i.test(e.message));

/** 410: the lead (or the chain's bots) were removed, so nothing can carry it on. */
export const isChainGone = (e: unknown): boolean => httpStatus(e) === 410;

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
