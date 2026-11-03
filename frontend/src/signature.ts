// Small signature elements that each state something the client already knows: the Verified stamp's
// counts, the lessons chip, the goal heartbeat, the engine hand-off and the once-only "first" moments.
import type { ChecksSummary, TaskEvent } from './missions';
import type { GoalEvent } from './goals';

type Counts = Pick<ChecksSummary, 'passed' | 'total'>;

/** A count the stamp may show: every check passed. A Verified task can still have an optional check waiting (1 of 2); that is not a count to put on a stamp. */
const full = (cs: Counts | null | undefined): cs is Counts => !!cs && Number.isFinite(cs.total) && cs.total > 0 && cs.passed === cs.total;

/** "Verified · 3/3" when every check passed, else just "Verified". */
export function stampText(cs: Counts | null | undefined): string {
  return full(cs) ? `Verified · ${cs.passed}/${cs.total}` : 'Verified';
}

/** What a screen reader hears for the stamp. */
export function stampLabel(cs: Counts | null | undefined): string {
  return full(cs) ? `Verified, ${cs.passed} of ${cs.total} checks passed. Open the evidence receipt` : 'Verified. Open the evidence receipt';
}

/** The "Learned from N earlier tasks" chip, from a task's `lessons_shown` events. Null when there is none. */
export function lessonsChip(events: Pick<TaskEvent, 'kind' | 'detail'>[]): { count: number; text: string; title: string } | null {
  const e = [...events].reverse().find((x) => x.kind === 'lessons_shown');
  if (!e) return null;
  const keys = Array.isArray(e.detail?.keys) ? e.detail.keys : [];
  const count = keys.length;
  if (count === 0) return null;
  const text = count === 1 ? 'Learned from 1 earlier task' : `Learned from ${count} earlier tasks`;
  // The event carries lesson keys only, never titles, so the hover says what it started with.
  return { count, text, title: `It started with ${count === 1 ? '1 lesson' : `${count} lessons`} from earlier work` };
}

export type HeartTone = 'ok' | 'failing' | 'fixing' | 'unknown';
export interface HeartDot {
  tone: HeartTone;
  label: string;
}

const verdictTone = (v: unknown): HeartTone => (v === 'met' ? 'ok' : v === 'not_met' ? 'failing' : 'unknown');

/** The goal's last checks, oldest first, one dot each: green OK, red failing, amber a fix underway, grey couldn't tell. */
export function heartbeatDots(events: Pick<GoalEvent, 'kind' | 'detail'>[], max = 7): HeartDot[] {
  const dots: HeartDot[] = [];
  for (const e of events) {
    if (e.kind === 'checked') {
      const tone = verdictTone(e.detail?.verdict);
      dots.push({ tone, label: tone === 'ok' ? 'OK' : tone === 'failing' ? 'failing' : 'couldn’t tell' });
    } else if (e.kind === 'fix_opened' || e.kind === 'replanned') {
      dots.push({ tone: 'fixing', label: 'fix underway' });
    }
  }
  return dots.slice(-max);
}

/** "Last 5 checks, oldest first: 3 OK, 1 failing, 1 fix underway." */
export function heartbeatText(dots: HeartDot[]): string {
  if (!dots.length) return 'No checks yet';
  const count = (t: HeartTone) => dots.filter((d) => d.tone === t).length;
  const parts = ([['ok', 'OK'], ['failing', 'failing'], ['fixing', 'fix underway'], ['unknown', 'couldn’t tell']] as const)
    .map(([t, l]) => [count(t), l] as const)
    .filter(([n]) => n > 0)
    .map(([n, l]) => `${n} ${l}`);
  return `Last ${dots.length} ${dots.length === 1 ? 'check' : 'checks'}, oldest first: ${parts.join(', ')}`;
}

// ---- Once per space ----

export type FirstKind = 'verified' | 'goal_met';

const FIRST_KEY = (kind: FirstKind, spaceId: string) => `kwant.first.${kind}.${spaceId || 'default'}`;

interface Store {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

const defaultStore = (): Store | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

/** The id the first moment was given to (the sticker stays on it), '' when it was claimed silently, null when unclaimed. */
export function firstHolder(kind: FirstKind, spaceId: string, store: Store | null = defaultStore()): string | null {
  try {
    return store?.getItem(FIRST_KEY(kind, spaceId)) ?? null;
  } catch {
    return null;
  }
}

/** True exactly once per space and kind, and never again; false when storage is unavailable (so nothing repeats). */
export function claimFirst(kind: FirstKind, spaceId: string, id: string, store: Store | null = defaultStore()): boolean {
  if (!store) return false;
  try {
    if (store.getItem(FIRST_KEY(kind, spaceId)) !== null) return false;
    store.setItem(FIRST_KEY(kind, spaceId), id);
    return true;
  } catch {
    return false;
  }
}

/** A space that already had one when this screen first loaded is not new: claim it without a celebration. */
export const claimQuietly = (kind: FirstKind, spaceId: string, store: Store | null = defaultStore()): void => {
  claimFirst(kind, spaceId, '', store);
};

export const FIRST_LINE: Record<FirstKind, string> = {
  verified: 'Your first verified task. A check confirmed it, so you can rely on it.',
  goal_met: 'Your first goal kept. Its checks pass, and you will be told if that changes.',
};
export const FIRST_STICKER: Record<FirstKind, string> = { verified: 'First verified', goal_met: 'First goal kept' };
