// 26d (autonomy/details-say-what-failed/brief.md): a saved reply's Details say which step failed or didn't run,
// most specific first, and a quiet line under the reply asks the person to check it when a step didn't run or
// failed for good. "Failed, then the same tool worked" is told in Details only: the reply is right.
import type { ActivityStep } from './activity';

export type ReplyOutcome =
  /** A step's approval card went unanswered, so it didn't run (26b `unanswered`). */
  | { kind: 'unanswered'; stepId: string }
  /** A step failed and no later step of the same tool worked. */
  | { kind: 'failed'; stepId: string }
  /** Every failed step was followed by a step of the same tool that worked; `count` failed steps. */
  | { kind: 'recovered'; count: number; stepId: string }
  /** desk-needs-you-buttons m6: the only failed steps are a customer's calls nobody allowed. Nothing ran, as it
   *  should: the reply is right, so there is nothing to check. */
  | { kind: 'refused'; stepId: string };

/** The server's words for a customer-turn call nobody allowed (mcp_runtime.rs EVERY_CALL_REFUSED), kept as the
 *  failed step's detail. */
export const CUSTOMER_REFUSED_DETAIL = "This step needs the person's OK first.";
const refusedCall = (s: ActivityStep) => s.status === 'failed' && !!s.detail && s.detail.replace(/’/g, "'").startsWith(CUSTOMER_REFUSED_DETAIL);

// Only a tool step that worked is a retry that worked. An approval step names its tool too and settles ok/done
// when the person says yes, but that is permission, not the call succeeding (Rigel's D1).
const worked = (s: ActivityStep) => s.kind === 'tool' && (s.status === 'ok' || s.status === 'done');

/**
 * What the reply's steps add up to, or null when nothing failed or went unanswered. A failed step counts as
 * recovered only when a later tool step names the same tool and worked (an approved approval card doesn't count);
 * a step with no tool name never recovers.
 * `stepId` is the step Details opens to: the first unanswered or failed-for-good step, else the first failed.
 */
export function replyOutcome(steps: readonly ActivityStep[] | null | undefined): ReplyOutcome | null {
  if (!steps?.length) return null;
  const unanswered = steps.find((s) => s.status === 'unanswered');
  if (unanswered) return { kind: 'unanswered', stepId: unanswered.id };
  const failed = steps.map((s, i) => [s, i] as const).filter(([s]) => s.status === 'failed' && !refusedCall(s));
  if (!failed.length) {
    const refused = steps.find(refusedCall);
    return refused ? { kind: 'refused', stepId: refused.id } : null;
  }
  const stuck = failed.find(([s, i]) => !s.tool || !steps.slice(i + 1).some((t) => t.tool === s.tool && worked(t)));
  if (stuck) return { kind: 'failed', stepId: stuck[0].id };
  return { kind: 'recovered', count: failed.length, stepId: failed[0][0].id };
}

/** The folded Details toggle's words. */
export function detailsLabel(o: ReplyOutcome | null): string {
  if (!o) return 'Details';
  if (o.kind === 'unanswered') return 'Details · a step didn’t run (no answer)';
  if (o.kind === 'failed') return 'Details · a step failed';
  if (o.kind === 'refused') return 'Details · nothing ran, as it should';
  return `Details · ${o.count} ${o.count === 1 ? 'step' : 'steps'} failed, then worked`;
}

/** The quiet line under the reply: only when a step didn't run or failed without a later success. */
export function checkLine(o: ReplyOutcome | null): string | null {
  if (o?.kind === 'unanswered') return 'A step didn’t run, so check this before relying on it.';
  if (o?.kind === 'failed') return 'A step failed, so check this before relying on it.';
  return null;
}
