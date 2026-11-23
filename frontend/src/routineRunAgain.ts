// routine-run-again (Mira's routine-for-a-week pass, fix 2): a failed routine run gets one Run again button in
// Needs you and in its Inbox row, and the routine_failed toast opens the routine. Pure, so node tests cover it.
import type { AwayItem } from './away';
import { continuityApi, type NotifyEvent, type Routine, type RoutineRun } from './continuity';

/** The run statuses that leave a routine run in Needs you (`FAILED_RUNS`, src/db/away.rs). */
const FAILED: ReadonlySet<string> = new Set(['error', 'incomplete', 'needs_you']);

/**
 * The routine a failed run's Run again starts: its `routine_id` (Vega's #1114). Null for a run that didn't fail,
 * for any other kind, and on an older server that leaves the field out; then no button shows.
 */
export function runAgainId(item: Pick<AwayItem, 'kind' | 'status'> & { routine_id?: unknown }): string | null {
  if (item.kind !== 'routine_run' || !FAILED.has(item.status)) return null;
  return typeof item.routine_id === 'string' && item.routine_id.trim() ? item.routine_id : null;
}

export type RunAgainResult = 'started' | 'skipped';

/** Run again: the same call as Routines' Run now. The item leaves Needs you on its own once the run starts. */
export async function runAgain(
  item: Pick<AwayItem, 'kind' | 'status'> & { routine_id?: unknown },
  run: (id: string) => Promise<Pick<RoutineRun, 'status'>> = continuityApi.runRoutine,
): Promise<RunAgainResult> {
  const id = runAgainId(item);
  if (!id) throw new Error('this run has no routine to run again');
  const r = await run(id);
  return r && r.status === 'skipped' ? 'skipped' : 'started';
}


/**
 * The routine a `routine_failed` notice is about. The notice carries no routine id, so: its bot, and the name in
 * its title (`Routine "<name>" failed`, src/server/scheduler.rs). Two of that bot's routines with the same name:
 * the one whose last run failed, newest first. Null when none matches (Routines then opens unfocused).
 */
export function routineOfNotice(n: Pick<NotifyEvent, 'kwant_id' | 'title'>, routines: Routine[]): string | null {
  const m = /^Routine "([\s\S]*)" failed$/.exec((n.title ?? '').trim());
  if (!m) return null;
  const hits = routines.filter((r) => r.kwant_id === n.kwant_id && r.name === m[1]);
  if (hits.length < 2) return hits[0]?.id ?? null;
  const failed = (r: Routine) => (r.last_status === 'error' || r.last_status === 'incomplete' ? 1 : 0);
  const ran = (r: Routine) => (r.last_run_at ? Date.parse(r.last_run_at) || 0 : 0);
  return [...hits].sort((a, b) => failed(b) - failed(a) || ran(b) - ran(a))[0].id;
}

/** What the toast's Open routine does: Routines opens at once, then focuses the routine once it is found. */
export interface OpenRoutineDeps {
  routines: () => Promise<Routine[]>;
  /** Space settings › Routines, unfocused. */
  openRoutines: () => void;
  /** Focus that routine's row (scroll, highlight, History open). */
  focusRoutine: (id: string) => void;
}

export async function openFailedRoutine(n: Pick<NotifyEvent, 'kwant_id' | 'title'>, deps: OpenRoutineDeps): Promise<string | null> {
  deps.openRoutines();
  let id: string | null = null;
  try {
    id = routineOfNotice(n, await deps.routines());
  } catch {
    // The list didn't load: Routines stays open, unfocused.
  }
  if (id) deps.focusRoutine(id);
  return id;
}

/** The routine_failed toast's one button (it stays silent: no sound for this toast). */
export const failedRoutineToastAction = (onClick: () => void) => ({ label: 'Open routine', onClick });
