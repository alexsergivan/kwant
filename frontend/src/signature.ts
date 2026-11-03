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

/** The "Learned from N" chip, from a task's `lessons_shown` events. Null when there is none.
 *  The hover lists lesson titles only when the event carries them; otherwise "From N earlier tasks". */
export function lessonsChip(events: Pick<TaskEvent, 'kind' | 'detail'>[]): { count: number; text: string; title: string } | null {
  const e = [...events].reverse().find((x) => x.kind === 'lessons_shown');
  if (!e) return null;
  const keys = Array.isArray(e.detail?.keys) ? e.detail.keys : [];
  const count = keys.length;
  if (count === 0) return null;
  const titles = Array.isArray(e.detail?.titles) ? (e.detail.titles as unknown[]).filter((t): t is string => typeof t === 'string' && t.trim() !== '') : [];
  const title = titles.length ? `From earlier tasks: ${titles.join('; ')}` : `From ${count === 1 ? '1 earlier task' : `${count} earlier tasks`}`;
  return { count, text: `Learned from ${count}`, title };
}

export type HeartTone = 'ok' | 'failing' | 'fixing' | 'unknown';
export interface HeartDot {
  tone: HeartTone;
  label: string;
  /** When the check (or fix) happened, as the event gave it. */
  at?: string;
}

const verdictTone = (v: unknown): HeartTone => (v === 'met' ? 'ok' : v === 'not_met' ? 'failing' : 'unknown');

/** The goal's last checks, oldest first, one square each: ok, failing, a fix underway, or couldn't check. */
export function heartbeatDots(events: Array<Pick<GoalEvent, 'kind' | 'detail'> & Partial<Pick<GoalEvent, 'created_at'>>>, max = 7): HeartDot[] {
  const dots: HeartDot[] = [];
  for (const e of events) {
    const at = typeof e.created_at === 'string' ? e.created_at : undefined;
    if (e.kind === 'checked') {
      const tone = verdictTone(e.detail?.verdict);
      dots.push({ tone, label: tone === 'ok' ? 'ok' : tone === 'failing' ? 'failing' : 'can’t check', at });
    } else if (e.kind === 'fix_opened' || e.kind === 'replanned') {
      dots.push({ tone: 'fixing', label: 'fixing', at });
    }
  }
  return dots.slice(-max);
}

const TONE_WORD: Record<HeartTone, string> = { ok: 'ok', failing: 'failing', fixing: 'fixing', unknown: 'can’t check' };

/** "Last 7 checks: 5 ok, 1 fixing, 1 failing" (spec §3), counted from the squares shown. */
export function heartbeatText(dots: HeartDot[]): string {
  if (!dots.length) return 'No checks yet';
  const count = (t: HeartTone) => dots.filter((d) => d.tone === t).length;
  const parts = (['ok', 'fixing', 'failing', 'unknown'] as const).map((t) => [count(t), TONE_WORD[t]] as const).filter(([n]) => n > 0).map(([n, l]) => `${n} ${l}`);
  return `Last ${dots.length} ${dots.length === 1 ? 'check' : 'checks'}: ${parts.join(', ')}`;
}

/** "5/7 ok": how many of the squares shown are ok. */
export function heartbeatCount(dots: HeartDot[]): string {
  return `${dots.filter((d) => d.tone === 'ok').length}/${dots.length} ok`;
}

export type HeartWord = 'Steady' | 'Flapping' | 'Failing' | 'Fixing' | 'Can’t check' | 'Back to ok';

/** One word for the run of squares: the newest decides, except a run that flips between ok and failing twice or more. */
export function heartbeatWord(dots: HeartDot[]): HeartWord | null {
  if (!dots.length) return null;
  const newest = dots[dots.length - 1].tone;
  if (newest === 'unknown') return 'Can’t check';
  const settled = dots.filter((d) => d.tone === 'ok' || d.tone === 'failing');
  let flips = 0;
  for (let i = 1; i < settled.length; i++) if (settled[i].tone !== settled[i - 1].tone) flips++;
  if (flips >= 2) return 'Flapping';
  if (newest === 'failing') return 'Failing';
  if (newest === 'fixing') return 'Fixing';
  return dots.every((d) => d.tone === 'ok') ? 'Steady' : 'Back to ok';
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
