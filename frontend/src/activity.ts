// Live activity: what a bot is doing while it works (docs/live-activity.md). Steps arrive as
// scoped `activity` events while the turn runs, thinking text streams as `ThinkingDelta`, and the
// saved reply keeps them in `meta.activity`.
import { parseMetaObject } from './questions';

export type StepKind = 'thinking' | 'tool' | 'approval' | 'human_step' | 'note';
export type StepStatus = 'running' | 'waiting' | 'ok' | 'failed' | 'done' | 'declined' | 'stopped';

export interface ActivityStep {
  id: string;
  kind: StepKind;
  status: StepStatus;
  title: string;
  tool?: string;
  detail?: string;
  /** The thinking step's whole text (set once it ends; live text is in `LiveActivity.thinking`). */
  text?: string;
  ref_id?: string;
  started_at: string;
  ended_at?: string;
  duration_ms?: number;
}

export interface SavedActivity {
  steps: ActivityStep[];
  dropped: number;
}

/** One running turn's steps, as the open chat draws them. */
export interface LiveActivity {
  turnId: string;
  steps: ActivityStep[];
  /** Thinking streamed so far, by step id. */
  thinking: Record<string, string>;
}

const KINDS: StepKind[] = ['thinking', 'tool', 'approval', 'human_step', 'note'];
const STATUSES: StepStatus[] = ['running', 'waiting', 'ok', 'failed', 'done', 'declined', 'stopped'];
const str = (v: unknown) => (typeof v === 'string' && v ? v : undefined);

/** A step from the wire, or null when it isn't one. Unknown kinds draw as notes. */
export function parseStep(raw: unknown): ActivityStep | null {
  if (!raw || typeof raw !== 'object') return null;
  const s = raw as Record<string, unknown>;
  if (typeof s.id !== 'string' || typeof s.title !== 'string') return null;
  const kind = KINDS.includes(s.kind as StepKind) ? (s.kind as StepKind) : 'note';
  const status = STATUSES.includes(s.status as StepStatus) ? (s.status as StepStatus) : 'ok';
  return {
    id: s.id,
    kind,
    status,
    title: s.title,
    tool: str(s.tool),
    detail: str(s.detail),
    text: str(s.text),
    ref_id: str(s.ref_id),
    started_at: typeof s.started_at === 'string' ? s.started_at : '',
    ended_at: str(s.ended_at),
    duration_ms: typeof s.duration_ms === 'number' ? s.duration_ms : undefined,
  };
}

/** `meta.activity` of a saved reply, or null when it has none. */
export function parseActivityMeta(raw: unknown): SavedActivity | null {
  const a = parseMetaObject(raw)?.activity;
  if (!a || typeof a !== 'object' || !Array.isArray((a as { steps?: unknown }).steps)) return null;
  const steps = ((a as { steps: unknown[] }).steps).map(parseStep).filter((s): s is ActivityStep => !!s);
  const dropped = Number((a as { dropped?: unknown }).dropped) || 0;
  return steps.length || dropped ? { steps, dropped } : null;
}

export const isOpenStep = (s: ActivityStep) => s.status === 'running' || s.status === 'waiting';

/** An `activity` event: replaces the step with the same id, else appends it. A new turn starts over. */
export function applyActivity(prev: LiveActivity | undefined, turnId: string, step: ActivityStep): LiveActivity {
  const base = prev && prev.turnId === turnId ? prev : { turnId, steps: [], thinking: {} };
  const i = base.steps.findIndex((s) => s.id === step.id);
  // A settled step doesn't go back to running (an out-of-order event loses).
  if (i >= 0 && !isOpenStep(base.steps[i]) && isOpenStep(step)) return base;
  const steps = i >= 0 ? base.steps.map((s, j) => (j === i ? step : s)) : [...base.steps, step];
  return { ...base, steps };
}

/** The thinking step text is streaming into right now, if any. */
export function runningThinking(live: LiveActivity | undefined, turnId?: string): ActivityStep | null {
  if (!live || (turnId && live.turnId !== turnId)) return null;
  const last = live.steps[live.steps.length - 1];
  return last && last.kind === 'thinking' && last.status === 'running' ? last : null;
}

/**
 * A `ThinkingDelta` chunk. Returns the new state, or null when no thinking step of this turn is
 * open (then the chunk is a status line, like "Cancelled by user", and is shown as before).
 */
export function appendThinking(live: LiveActivity | undefined, turnId: string | undefined, chunk: string): LiveActivity | null {
  const step = runningThinking(live, turnId);
  if (!live || !step) return null;
  return { ...live, thinking: { ...live.thinking, [step.id]: (live.thinking[step.id] ?? '') + chunk } };
}

/** The text to show under a thinking step: the saved text once it ended, else what streamed. */
export const thinkingText = (step: ActivityStep, live?: LiveActivity) => step.text ?? live?.thinking[step.id] ?? '';

const ms = (iso?: string) => {
  const t = iso ? Date.parse(iso) : NaN;
  return Number.isFinite(t) ? t : null;
};

/** From the first step's start to the last end, in ms (null when unknown). */
export function activitySpan(steps: ActivityStep[]): number | null {
  let start: number | null = null;
  let end: number | null = null;
  for (const s of steps) {
    const a = ms(s.started_at);
    const b = ms(s.ended_at) ?? (a !== null && typeof s.duration_ms === 'number' ? a + s.duration_ms : a);
    if (a !== null && (start === null || a < start)) start = a;
    if (b !== null && (end === null || b > end)) end = b;
  }
  return start !== null && end !== null && end >= start ? end - start : null;
}

/** "840 ms", "8.4s", "41s", "2m 05s". */
export function formatDuration(msTotal: number): string {
  if (msTotal < 1000) return `${Math.round(msTotal)} ms`;
  const s = msTotal / 1000;
  if (s < 10) return `${s.toFixed(1)}s`;
  if (s < 60) return `${Math.round(s)}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${String(Math.round(s % 60)).padStart(2, '0')}s`;
}

/** Steps that count in the summary: notes explain a limit and aren't work. */
const counted = (steps: ActivityStep[]) => steps.filter((s) => s.kind !== 'note');

/** "12 steps · 41s", "1 step", plus "· 2 failed" when any failed. */
export function activitySummary(a: SavedActivity): string {
  const n = counted(a.steps).length + a.dropped;
  const parts = [`${n} ${n === 1 ? 'step' : 'steps'}`];
  const span = activitySpan(a.steps);
  if (span !== null && span >= 1000) parts.push(formatDuration(span));
  const failed = a.steps.filter((s) => s.status === 'failed').length;
  if (failed) parts.push(`${failed} failed`);
  return parts.join(' · ');
}

/** What a step's status reads as, for screen readers and the row's label. */
export function statusLabel(s: ActivityStep): string {
  switch (s.status) {
    case 'running':
      return s.kind === 'thinking' ? 'Thinking' : 'Running';
    case 'waiting':
      return 'Waiting for you';
    case 'ok':
      return s.kind === 'approval' ? 'Allowed' : 'Done';
    case 'done':
      return 'Done';
    case 'failed':
      return 'Failed';
    case 'declined':
      return s.kind === 'approval' ? 'Denied' : 'Skipped';
    case 'stopped':
      return 'Stopped';
  }
}

/** The step's title without markdown backticks (titles quote commands like "Running `cargo test`"). */
export const plainTitle = (t: string) => t.replace(/`([^`]*)`/g, '$1');

/** Whether the saved activity has thinking text, so the old reasoning block would repeat it. */
export const hasThinkingText = (a: SavedActivity | null) => !!a?.steps.some((s) => s.kind === 'thinking' && s.text);
