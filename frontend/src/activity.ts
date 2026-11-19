// Live activity: what a bot is doing while it works (docs/live-activity.md). Steps arrive as
// scoped `activity` events while the turn runs, thinking text streams as `ThinkingDelta`, and the
// saved reply keeps them in `meta.activity`.
import { parseMetaObject } from './questions';

export type StepKind = 'thinking' | 'tool' | 'approval' | 'human_step' | 'note';
// 26b (#766): `unanswered` is a step whose approval card nobody answered in time, so it didn't run; `rehearsed`
// is a dry run (Skill Foundry) that recorded the call instead of running it. Neither is "ok".
export type StepStatus = 'running' | 'waiting' | 'ok' | 'failed' | 'done' | 'declined' | 'stopped' | 'unanswered' | 'rehearsed';

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
  /** A browser open/navigate step: the title the page reported once it loaded (server 0.9.8+, optional). */
  page_title?: string;
  started_at: string;
  ended_at?: string;
  duration_ms?: number;
  /** 28j: a human step's own settled word when `status` can't hold it ("resolved_auto", "timed_out"); words only. */
  outcome?: string;
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
const STATUSES: StepStatus[] = ['running', 'waiting', 'ok', 'failed', 'done', 'declined', 'stopped', 'unanswered', 'rehearsed'];
const str = (v: unknown) => (typeof v === 'string' && v ? v : undefined);

/** A step from the wire, or null when it isn't one. Unknown kinds draw as notes. */
export function parseStep(raw: unknown): ActivityStep | null {
  if (!raw || typeof raw !== 'object') return null;
  const s = raw as Record<string, unknown>;
  if (typeof s.id !== 'string' || typeof s.title !== 'string') return null;
  const kind = KINDS.includes(s.kind as StepKind) ? (s.kind as StepKind) : 'note';
  const status = STATUSES.includes(s.status as StepStatus) ? (s.status as StepStatus) : 'ok';
  const outcome = kind === 'human_step' && typeof s.status === 'string' && s.status !== status ? s.status : undefined;
  return {
    id: s.id,
    kind,
    status,
    title: s.title,
    tool: str(s.tool),
    detail: str(s.detail),
    text: str(s.text),
    ref_id: str(s.ref_id),
    page_title: str(s.page_title),
    started_at: typeof s.started_at === 'string' ? s.started_at : '',
    ended_at: str(s.ended_at),
    duration_ms: typeof s.duration_ms === 'number' ? s.duration_ms : undefined,
    ...(outcome ? { outcome } : {}),
  };
}

/** `meta.activity` of a saved reply, or null when it has none. */
export function parseActivityMeta(raw: unknown): SavedActivity | null {
  const a = parseMetaObject(raw)?.activity;
  if (!a || typeof a !== 'object' || !Array.isArray((a as { steps?: unknown }).steps)) return null;
  const steps = ((a as { steps: unknown[] }).steps).map(parseStep).filter((s): s is ActivityStep => !!s);
  const dropped = Number((a as { dropped?: unknown }).dropped) || 0;
  return steps.length || dropped ? { steps: readDenials(steps), dropped } : null;
}

/** What the claude CLI tells the bot (and kwnt saved as a failed step) when the person pressed Deny (server DENIED_TEXT). */
const CLI_DENIED = 'The user denied this tool call';
/** How a step the person said no to reads (28a); the same words as the server's DENIED_STEP_DETAIL (28b). */
export const DENIED_STEP_DETAIL = 'You said no; nothing ran';

const deniedFailure = (s: ActivityStep) => s.kind === 'tool' && s.status === 'failed' && !!s.detail?.startsWith(CLI_DENIED);
const readAsDenied = (s: ActivityStep) => s.kind === 'tool' && s.status === 'declined' && s.detail === DENIED_STEP_DETAIL;

/**
 * 28a: the person's Deny reads as their no, not a failure. Before 28b (and in every older saved reply) a Deny
 * leaves two steps: the card's approval step `declined` and the tool step `failed` with the CLI's "The user denied
 * this tool call." Each declined approval step turns at most one such tool step of the same tool into `declined`,
 * "You said no; nothing ran" (the nearest one, in either order: the CLI's tool step usually starts first). A step
 * already in that shape (28b's server) takes its card too, so this is idempotent. With no declined card (kwnt
 * refused the call itself) the step stays `failed`: that wasn't the person's no. Returns `steps` when nothing changes.
 */
export function readDenials(steps: ActivityStep[]): ActivityStep[] {
  const cards = steps.map((s, i) => [s, i] as const).filter(([s]) => s.kind === 'approval' && s.status === 'declined' && s.tool);
  if (!cards.length) return steps;
  const usedCards = new Set<number>();
  const usedSteps = new Set<number>();
  const turn = new Set<number>();
  // Steps already read as denied take their card first, so a second call of the same tool isn't paired with it again.
  for (const pass of [readAsDenied, deniedFailure]) {
    for (const [card, ci] of cards) {
      if (usedCards.has(ci)) continue;
      let best = -1;
      steps.forEach((s, i) => {
        if (s.tool !== card.tool || usedSteps.has(i) || !pass(s)) return;
        if (best < 0 || Math.abs(i - ci) < Math.abs(best - ci)) best = i;
      });
      if (best < 0) continue;
      usedCards.add(ci);
      usedSteps.add(best);
      if (pass === deniedFailure) turn.add(best);
    }
  }
  if (!turn.size) return steps;
  return steps.map((s, i) => (turn.has(i) ? { ...s, status: 'declined', detail: DENIED_STEP_DETAIL } : s));
}

export const isOpenStep = (s: ActivityStep) => s.status === 'running' || s.status === 'waiting';

/** The answer a settled approval card gave, as its command row says it (28s). */
export type CardAnswer = 'ok' | 'declined' | 'unanswered';
/** 28s: the command row's chip for the answer its folded card gave. */
export const ANSWER_LABEL: Record<CardAnswer, string> = { ok: 'You said OK', declined: 'You said no', unanswered: 'Not approved in time' };

/** Which tool steps a settled card of each answer can stand for: the command it let run, or the one it held. */
const answeredBy: Record<CardAnswer, (s: ActivityStep) => boolean> = {
  declined: (s) => s.status === 'declined',
  unanswered: (s) => s.status === 'unanswered',
  ok: (s) => s.status !== 'declined' && s.status !== 'unanswered' && s.status !== 'waiting',
};

export interface FoldedSteps {
  /** The rows Details draws: `steps` without the approval steps that folded into their command row. */
  rows: ActivityStep[];
  /** Command step id → the answer its card gave. */
  answers: Map<string, CardAnswer>;
  /** Folded approval step id → the command step id that now carries it (so a link to the card lands on its row). */
  foldedInto: Map<string, string>;
}

/**
 * 28s one-row-per-answer: Details says each answer once. A settled approval step (OK, no, not answered in time)
 * pairs with the nearest tool step of the same tool, in either order, the way `readDenials` pairs (each card at
 * most one step, each step at most one card); the approval row is hidden and the command row carries the answer.
 * A card with no such command row (kwnt held the call, nothing ran, or it is still open) stays its own row.
 * Draw time only: saved steps and follow-through read the steps as they are; the step count uses `rows` (28t).
 */
export function foldAnswers(steps: readonly ActivityStep[]): FoldedSteps {
  const answers = new Map<string, CardAnswer>();
  const foldedInto = new Map<string, string>();
  const used = new Set<number>();
  steps.forEach((card, ci) => {
    if (card.kind !== 'approval' || !card.tool || !(card.status in answeredBy)) return;
    const fits = answeredBy[card.status as CardAnswer];
    // Nearest by place; on a tie (one before, one after), the one that started closest in time to the card.
    const gap = (i: number) => [Math.abs(i - ci), Math.abs(Date.parse(steps[i].started_at) - Date.parse(card.started_at)) || 0] as const;
    let best = -1;
    steps.forEach((s, i) => {
      if (s.kind !== 'tool' || s.tool !== card.tool || used.has(i) || !fits(s)) return;
      if (best < 0) return void (best = i);
      const [a, b] = [gap(i), gap(best)];
      if (a[0] < b[0] || (a[0] === b[0] && a[1] < b[1])) best = i;
    });
    if (best < 0) return;
    used.add(best);
    answers.set(steps[best].id, card.status as CardAnswer);
    foldedInto.set(card.id, steps[best].id);
  });
  if (!foldedInto.size) return { rows: steps as ActivityStep[], answers, foldedInto };
  return { rows: steps.filter((s) => !foldedInto.has(s.id)), answers, foldedInto };
}

/**
 * Whether the live steps card folds to one row (18f): once the reply has words, the row moves below the bubble,
 * into the landed action row's place, so landing moves nothing above the words. A step waiting for the person
 * keeps the full card open above the bubble: an ask never hides.
 */
export function foldLiveSteps({ writing, steps }: { writing: boolean; steps: readonly ActivityStep[] }): boolean {
  return writing && !steps.some((s) => s.status === 'waiting');
}

/** An `activity` event: replaces the step with the same id, else appends it. A new turn starts over. */
export function applyActivity(prev: LiveActivity | undefined, turnId: string, step: ActivityStep): LiveActivity {
  const base = prev && prev.turnId === turnId ? prev : { turnId, steps: [], thinking: {} };
  const i = base.steps.findIndex((s) => s.id === step.id);
  // A settled step doesn't go back to running (an out-of-order event loses).
  if (i >= 0 && !isOpenStep(base.steps[i]) && isOpenStep(step)) return base;
  const steps = i >= 0 ? base.steps.map((s, j) => (j === i ? step : s)) : [...base.steps, step];
  return { ...base, steps: readDenials(steps) };
}

/**
 * After a load or reconnect: a running turn's steps so far, from `GET /api/turns` (its `activity`,
 * shaped like `meta.activity`). Each goes through `applyActivity`, so steps that already came live
 * keep their place and a settled step never goes back to running. Steps there carry no thinking
 * `text`, so a step that already has its text keeps it. Null when there is nothing to add.
 */
export function seedActivity(prev: LiveActivity | undefined, turnId: string, raw: unknown): LiveActivity | null {
  const saved = parseActivityMeta({ activity: raw });
  if (!saved?.steps.length) return null;
  // `/api/turns` sends no thinking text, so a thinking step that already ended live keeps its own.
  const keepText = (acc: LiveActivity, step: ActivityStep) => {
    const text = step.text ?? acc.steps.find((s) => s.id === step.id)?.text;
    return text === undefined ? step : { ...step, text };
  };
  return saved.steps.reduce<LiveActivity>(
    (acc, step) => applyActivity(acc, turnId, keepText(acc, step)),
    prev?.turnId === turnId ? prev : { turnId, steps: [], thinking: {} },
  );
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

/**
 * How many steps the summary and the live "Working · N steps" say: the rows Details draws, so a settled approval
 * folded into its command (28s) counts once with it (28t). Notes explain a limit and aren't work.
 */
export const shownStepCount = (steps: readonly ActivityStep[]) => foldAnswers(steps).rows.filter((s) => s.kind !== 'note').length;

/** "0:42", "12:05", "1:02:09": the working strip's clock. Negative or non-finite input reads 0:00. */
export function formatClock(ms: number): string {
  const total = Number.isFinite(ms) && ms > 0 ? Math.floor(ms / 1000) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

/** "Worked 0:41 · 12 steps" (the strip's last time, same start), "1 step", plus "· 2 failed" when any failed,
 *  "· 1 didn't run" when an approval went unanswered (26d) and "· you said no to 1" when the person denied a call (28a).
 *  Counts read the rows Details draws: a folded card and its command count once (28t). */
export function activitySummary(a: SavedActivity): string {
  // One step rule for the saved summary and the live "Working · N steps" (28z): shownStepCount.
  const n = shownStepCount(a.steps) + a.dropped;
  const steps = `${n} ${n === 1 ? 'step' : 'steps'}`;
  const span = activitySpan(a.steps);
  const parts = span !== null && span >= 1000 ? [`Worked ${formatClock(span)}`, steps] : [steps];
  const failed = a.steps.filter((s) => s.status === 'failed').length;
  if (failed) parts.push(`${failed} failed`);
  // Raw status, not shownStatus: a human step nobody finished in time (28n) doesn't count here.
  // Read the folded rows Details draws, so a card and its command count once (28t).
  const unanswered = foldAnswers(a.steps).rows.filter((s) => s.status === 'unanswered').length;
  if (unanswered) parts.push(`${unanswered} didn’t run`);
  const declined = a.steps.filter((s) => s.kind === 'tool' && s.status === 'declined').length;
  if (declined) parts.push(`you said no to ${declined}`);
  return parts.join(' · ');
}

/** 28n: a human step nobody finished in time (server `timed_out`, kept in `outcome` by 28j). */
const timedOutHumanStep = (s: ActivityStep) => s.kind === 'human_step' && s.outcome === 'timed_out';

/**
 * The status a step is drawn with (icon, chip). 28n: a human step nobody finished in time looks like an approval
 * nobody answered (⏱, amber) instead of ✓ Done. `status` itself stays `ok`, so counts and follow-through are as before.
 */
export function shownStatus(s: ActivityStep): StepStatus {
  return timedOutHumanStep(s) ? 'unanswered' : s.status;
}

/** What a step's status reads as, for screen readers and the row's label. */
export function statusLabel(s: ActivityStep): string {
  if (timedOutHumanStep(s)) return 'Not done in time';
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
      return 'You said no';
    case 'stopped':
      return 'Stopped';
    case 'unanswered':
      return 'Not approved in time';
    case 'rehearsed':
      return 'Dry run';
  }
}

/** The step's title without markdown backticks (titles quote commands like "Running `cargo test`"). */
export const plainTitle = (t: string) => t.replace(/`([^`]*)`/g, '$1');

/** Whether the saved activity has thinking text, so the old reasoning block would repeat it. */
export const hasThinkingText = (a: SavedActivity | null) => !!a?.steps.some((s) => s.kind === 'thinking' && s.text);

/**
 * 29l/29m: whether a step is a file tool, whose detail is a file path. Only these get 29i's path breaks: a shell step's
 * detail is the whole command, which may start with a path ("/usr/bin/git status", "./gradlew test") and must wrap as
 * it always did. The list is the server's one list, `FileTool::of` (src/drivers/activity.rs), so Codex, grok and MCP
 * file steps count too; keep the two in step. Like the server's `tool_part`, an `mcp__<server>__` prefix is dropped
 * (strip "mcp__", then split once at the first "__") and the name is lowercased, so `mcp__kwnt__read_file` is a file step.
 */
const FILE_TOOLS = new Set([
  // FileTool::Read
  'read', 'read_file', 'view', 'cat',
  // FileTool::Edit
  'edit', 'multiedit', 'edit_file', 'str_replace', 'str_replace_based_edit_tool', 'apply_patch', 'file_change', 'notebookedit',
  // FileTool::Write
  'write', 'write_file', 'create_file',
]);
const toolPart = (name: string) => {
  const rest = name.startsWith('mcp__') ? name.slice(5) : null;
  const at = rest?.indexOf('__') ?? -1;
  return rest !== null && at >= 0 ? rest.slice(at + 2) : name;
};
export const isFileStep = (step: Pick<ActivityStep, 'tool'>) =>
  !!step.tool && FILE_TOOLS.has(toolPart(step.tool).toLowerCase());

/**
 * 29i: a single-line detail that is a file path, split after each "/" or "\" (["/", "home/", "me/", "plan.md"]), so
 * StepRow can put a <wbr> between the parts and a long path breaks at a folder, not inside the file name. Null for
 * every other detail (commands, URLs, errors, more than one line), which renders exactly as before. Joining the parts
 * gives back the detail, so the text a person copies is unchanged.
 */
export function pathBreakParts(detail: string): string[] | null {
  if (!/^(\/|~\/|\.\.?\/|[A-Za-z]:[\\/])/.test(detail) || /[\r\n]/.test(detail)) return null;
  const parts = detail.match(/[^/\\]*(?:[/\\]+|$)/g)?.filter(Boolean) ?? [];
  return parts.length > 1 ? parts : null;
}
