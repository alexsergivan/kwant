// 0.9.7 routine row (Mira's UX audit #5, autonomy/routine-row/brief.md): a routine's run posts its
// instructions into the chat as a user message whose sender id is `Routine: <name>` (scheduler.rs
// `execute`). The chat drew it as the person's own "You" bubble; it now draws a routine row instead.
// The check is on sender_id, never on the text, so a person who types "Routine: ..." stays "You".

import { ROUTINE_PREFIX } from './doneAway';

export interface RoutineSource {
  sender_role: string;
  sender_id?: string | null;
}

/** The routine's name when this message is a routine's post, else null. An unnamed routine gives ''. */
export function routineOf(msg: RoutineSource): string | null {
  if (msg.sender_role !== 'user') return null;
  const id = msg.sender_id ?? '';
  if (!ROUTINE_PREFIX.test(id)) return null;
  return id.replace(ROUTINE_PREFIX, '').trim();
}

/** The row's label: "Routine · <name>", or "Routine" when the name is blank. */
export const routineLabel = (name: string): string => (name ? `Routine · ${name}` : 'Routine');

/** The instructions on one line for the collapsed row: whitespace folded; CSS adds the ellipsis. */
export const routineLine = (text: string | null | undefined): string => (text ?? '').replace(/\s+/g, ' ').trim();

// ---------------------------------------------------------------- 26d: a run's row in Routines › History

/** What the live `routine_run` events said about a run that waits on an approval card (26b). */
export interface RunWait {
  routine_id: string | null;
  waiting: boolean;
  /** The card the run waits on (or last waited on, this session); null when no event named one. */
  approval_id: string | null;
}

export type RunTone = 'running' | 'ok' | 'error' | 'skipped' | 'waiting' | 'needs_you' | 'said_no';

export interface RunRowState {
  label: string;
  tone: RunTone;
  /** The approval card the row links to ("Open the card"), when there is one to open. */
  cardId: string | null;
}

const RUN_LABEL: Record<string, { label: string; tone: RunTone }> = {
  running: { label: 'Running', tone: 'running' },
  ok: { label: 'OK', tone: 'ok' },
  error: { label: 'Failed', tone: 'error' },
  skipped: { label: 'Skipped', tone: 'skipped' },
  needs_you: { label: 'Needed your approval, nothing ran', tone: 'needs_you' },
  incomplete: { label: 'A step failed', tone: 'error' },
};

// 28f: after 28b (#792) a run whose step the person said no to ends `incomplete` with exactly
// "You said no to <tool>; nothing ran" (scheduler.rs `said_no`). That is their choice, not a failure.
// Only that sentence counts: main's "<tool> was denied" is also written for the pre-push gate, kwnt's
// secret files, a ticket that couldn't be stored and a wait that ran out, so it keeps "A step failed".
const SAID_NO_RUN = /^You said no to .+; nothing ran$/;

/** True only for 28b's said-no reason on a run (anchored, case-sensitive). */
export const saidNoRun = (error: string | null | undefined): boolean => typeof error === 'string' && SAID_NO_RUN.test(error);

// unanswered-question-not-ok: a `needs_you` run whose bot asked a question nobody answered ends with exactly this
// error (scheduler.rs `QUESTION_ERROR`). It waits on an answer, not an approval, and has no card to open.
const QUESTION_ERROR = "A step is waiting for your answer, so it didn't finish";

/** True only for the question error on a run (exact match). */
export const questionRun = (error: string | null | undefined): boolean => error === QUESTION_ERROR;

/**
 * A run's badge in History. A running run that waits on a card reads "Waiting for you" and links to it; a
 * `needs_you` run keeps that link when this session saw the card, since the card stays open after the wait.
 * An unknown status reads as itself, in the skipped tone. `incomplete`'s reason shows under it (routineRunNote);
 * an `incomplete` run the person said no to (28b's sentence) reads "You said no" in grey (28f). A `needs_you` run that
 * waits on a question reads "Needed your answer, didn’t finish", with no card link (unanswered-question-not-ok).
 */
export function runRowState(run: { status: string; error?: string | null }, wait?: RunWait | null): RunRowState {
  if (run.status === 'running' && wait?.waiting) return { label: 'Waiting for you', tone: 'waiting', cardId: wait.approval_id };
  if (run.status === 'incomplete' && saidNoRun(run.error)) return { label: 'You said no', tone: 'said_no', cardId: null };
  if (run.status === 'needs_you' && questionRun(run.error)) return { label: 'Needed your answer, didn’t finish', tone: 'needs_you', cardId: null };
  const known = RUN_LABEL[run.status];
  const cardId = run.status === 'needs_you' ? wait?.approval_id ?? null : null;
  return known ? { ...known, cardId } : { label: run.status, tone: 'skipped', cardId: null };
}

/** Folds a `routine_run` event into the waits by run id. The card id outlives the wait (needs_you links to it). */
export function applyRunWait(
  prev: Record<string, RunWait>,
  data: { routine_id?: unknown; run_id?: unknown; waiting?: unknown; approval_id?: unknown } | null | undefined,
): Record<string, RunWait> {
  const id = typeof data?.run_id === 'string' ? data.run_id : '';
  if (!id) return prev;
  const waiting = data?.waiting === true;
  const card = typeof data?.approval_id === 'string' && data.approval_id ? data.approval_id : null;
  const had = prev[id];
  if (!had && !waiting && !card) return prev;
  const routine = typeof data?.routine_id === 'string' && data.routine_id ? data.routine_id : had?.routine_id ?? null;
  const next: RunWait = { routine_id: routine, waiting, approval_id: card ?? had?.approval_id ?? null };
  if (had && had.waiting === next.waiting && had.approval_id === next.approval_id && had.routine_id === next.routine_id) return prev;
  return { ...prev, [id]: next };
}

/** The wait of a routine's run that waits right now, for the routine card's last-run badge; null when none. */
export const liveWaitOf = (waits: Record<string, RunWait>, routineId: string): RunWait | null =>
  Object.values(waits).find((w) => w.waiting && w.routine_id === routineId) ?? null;

// 26d (Rigel's D2): live waits outlive the Routines panel. App records every `routine_run` event here, so opening
// Routines after a run started waiting still shows "Waiting for you" and its card link.
let runWaits: Record<string, RunWait> = {};
const runWaitSubs = new Set<() => void>();
export function recordRunWait(data: Parameters<typeof applyRunWait>[1]): void {
  const next = applyRunWait(runWaits, data);
  if (next === runWaits) return;
  runWaits = next;
  runWaitSubs.forEach((fn) => fn());
}
export function subscribeRunWaits(fn: () => void): () => void {
  runWaitSubs.add(fn);
  return () => {
    runWaitSubs.delete(fn);
  };
}
export const runWaitsNow = (): Record<string, RunWait> => runWaits;
