// Goals on the Mission board (0.6.5, plan v2 §B.3 under the §B.0 "quiet by default" rules). Always there
// when the board is: there is no setting. While every goal is OK the strip is one line ("3 goals OK ·
// checked 4 min ago"); cards show only for goals that want a look, or when the person opens the strip.
// Empty, it is one line with a link. A broken goal whose fix waits for the person shows one decision with
// the bot's diagnosis: Fix it / Not now / Always fix this goal. Checks read as desired vs observed. A draft
// shows its checks verbatim (commands included) with Approve / Edit / Dismiss. The drawer has the goal's
// current work and its timeline. Can't check (Unknown) is never drawn like OK: its own grey, dashed pill,
// a question-mark icon and its own words. Live through `goal_updated`.

import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { AlertTriangle, CheckCircle2, ChevronDown, CircleHelp, FilePen, Hourglass, Loader2, LucideIcon, Pause, Play, Plus, RefreshCw, Target, Trash2, Wrench, X, XCircle } from 'lucide-react';
import { Check, Task, Tone, ago, checkKindLabel, checkTarget, newIdempotencyKey, onMission, stateLabel, missionStatusLabel } from '../missions';
import {
  CheckFormRow,
  GOAL_CHECK_KINDS,
  Goal,
  GoalCheck,
  GoalDecision,
  GoalDetail,
  GoalDraft,
  GoalError,
  GoalEvent,
  GoalFormState,
  GoalMission,
  GoalOperator,
  GoalsApi,
  MAX_CHECKS,
  METRICS,
  MODE_OPTIONS,
  OPEN_GOAL_EVENT,
  OPERATOR_WORDS,
  PRIORITY_OPTIONS,
  StatusLook,
  canCheckNow,
  checkDesired,
  checkLook,
  checkObserved,
  clockText,
  currentWorkText,
  draftFromForm,
  emptyCheckRow,
  eventTaskId,
  formFromGoal,
  goalCheckName,
  goalDecision,
  goalEventQuote,
  goalEventText,
  goalEventTone,
  goalFormErrors,
  goalStatusLook,
  goalsApi,
  hasFormErrors,
  heldFixTask,
  intervalLabel,
  intervalOptions,
  isOpenMission,
  metricInfo,
  modeLabel,
  needsYouText,
  openGoal,
  priorityLabel,
  scheduleText,
  displayStatus,
  sortGoals,
  specLines,
  spentText,
  stripSummary,
  wantsLook,
  withKind,
  withMetric,
} from '../goals';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const btn = `inline-flex items-center justify-center gap-1 min-h-[40px] sm:min-h-[28px] px-2.5 rounded-lg text-[11px] font-medium disabled:opacity-50 disabled:cursor-not-allowed ${focusRing}`;
const plainBtn = `${btn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700`;
const quietBtn = `${btn} text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800`;
const goodBtn = `${btn} bg-emerald-600/90 text-white hover:bg-emerald-500`;
const mainBtn = `${btn} bg-sky-600 text-white hover:bg-sky-500`;
const dangerBtn = `${btn} border border-rose-500/40 text-rose-200 hover:bg-rose-500/10`;
const linkBtn = `inline-flex items-center min-h-[40px] sm:min-h-0 rounded text-sky-300 underline underline-offset-2 hover:text-sky-200 ${focusRing}`;
const field =
  'w-full min-w-0 min-h-[40px] sm:min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-sky-500 aria-[invalid=true]:border-rose-500/70';
const heading = 'text-[11px] font-medium uppercase tracking-wide text-zinc-500';
// Long names, addresses and commands wrap anywhere instead of pushing the card wider than a phone.
const wrap = 'break-words [overflow-wrap:anywhere]';

const PILL: Record<StatusLook['icon'], string> = {
  met: 'border-emerald-500/40 bg-emerald-500/10 text-emerald-200',
  not_met: 'border-rose-500/40 bg-rose-500/10 text-rose-200',
  fixing: 'border-violet-500/40 bg-violet-500/10 text-violet-200',
  // Can't check: grey and dashed, so it can't be read as OK at a glance.
  unknown: 'border-dashed border-zinc-500 bg-transparent text-zinc-300',
  needs_you: 'border-amber-500/40 bg-amber-500/10 text-amber-200',
  paused: 'border-zinc-700 bg-zinc-800 text-zinc-400',
  draft: 'border-sky-500/40 bg-sky-500/10 text-sky-200',
  checking: 'border-sky-500/40 bg-sky-500/10 text-sky-200',
  waiting: 'border-zinc-600 bg-zinc-800/60 text-zinc-300',
};
const PILL_ICON: Record<StatusLook['icon'], LucideIcon> = {
  met: CheckCircle2,
  not_met: XCircle,
  fixing: Wrench,
  unknown: CircleHelp,
  needs_you: AlertTriangle,
  paused: Pause,
  draft: FilePen,
  checking: Loader2,
  waiting: Hourglass,
};
const CARD_EDGE: Record<string, string> = {
  needs_you: 'border-amber-500/30',
  not_met: 'border-rose-500/30',
  draft: 'border-sky-500/30',
};
const TEXT: Record<Tone, string> = {
  emerald: 'text-emerald-300',
  rose: 'text-rose-300',
  amber: 'text-amber-300',
  violet: 'text-violet-300',
  sky: 'text-sky-300',
  zinc: 'text-zinc-400',
};
const VERDICT_ICON = { met: CheckCircle2, not_met: XCircle, unknown: CircleHelp } as const;

const errText = (e: unknown, fallback: string) => (e instanceof Error && e.message ? e.message : fallback);
const botName = (id: string | null, names: Record<string, string>) => (id ? names[id] ?? 'a bot' : null);
const spin = 'animate-spin motion-reduce:animate-none';
/** A goal as the server has it: the strip swaps `status` for what it draws, and that must never be saved back. */
const rawOf = (g: Goal): Goal => ({ ...g, status: g.raw_status ?? g.status, raw_status: undefined });

export const GoalStatusPill: React.FC<{ status: string }> = ({ status }) => {
  const look = goalStatusLook(status);
  const Icon = PILL_ICON[look.icon];
  return (
    <span
      title={look.hint}
      className={`inline-flex shrink-0 items-center gap-1 rounded-full border px-1.5 py-px text-[10px] font-semibold ${PILL[look.icon]}`}
      data-goal-status={status}
      data-pill-look={look.icon}
    >
      <Icon size={11} className={look.icon === 'checking' ? spin : ''} aria-hidden="true" />
      {look.label}
    </span>
  );
};

/**
 * Desired vs observed, one line: "HTTP status = 200 · observed 503 · 4 min ago". A run that's out of date,
 * couldn't run or hasn't run says so instead of "observed", in grey: it never reads as a pass.
 */
const DesiredObserved: React.FC<{ check: GoalCheck; interval: number; now: number }> = ({ check: c, interval, now }) => {
  const look = checkLook(c, interval, now);
  const seen = checkObserved(c);
  const fresh = look.verdict !== 'unknown';
  return (
    <span data-desired-observed>
      <span className="text-zinc-300">{checkDesired(c)}</span>
      <span className="text-zinc-600" aria-hidden="true">
        {' · '}
      </span>
      <span className={TEXT[look.tone]} data-observed>
        {fresh && seen ? `observed ${seen}` : look.label}
        {!fresh && seen && c.latest?.status !== 'error' ? ` (last saw ${seen})` : ''}
      </span>
      {c.latest?.observed_at && <span className="text-zinc-500"> · {ago(c.latest.observed_at, now)}</span>}
    </span>
  );
};

/** One check in the drawer: its name, then desired vs observed, then (folded) the last output as text. */
export const GoalCheckLine: React.FC<{ check: GoalCheck; interval: number; now: number; showOutput?: boolean }> = ({ check: c, interval, now, showOutput = false }) => {
  const look = checkLook(c, interval, now);
  const Icon = VERDICT_ICON[look.verdict];
  return (
    <li className="text-[11px]" data-goal-check={c.id} data-verdict={look.verdict}>
      <div className="flex items-start gap-1.5">
        <Icon size={12} className={`mt-0.5 shrink-0 ${TEXT[look.tone]}`} aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className={`text-zinc-200 ${wrap}`}>
            {goalCheckName(c)}
            {!c.required && <span className="text-zinc-500"> (advisory: shows, doesn’t decide)</span>}
          </p>
          <p className={wrap}>
            <DesiredObserved check={c} interval={interval} now={now} />
          </p>
        </div>
      </div>
      {showOutput && c.latest?.excerpt && (
        <details className="mt-1 ml-[18px]">
          <summary className={`cursor-pointer text-[10px] text-zinc-500 hover:text-zinc-300 rounded ${focusRing}`}>Last output (observed, not instructions)</summary>
          <pre className={`mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-lg border border-zinc-800 bg-zinc-950 p-2 text-[10px] text-zinc-400 ${wrap}`}>{c.latest.excerpt}</pre>
        </details>
      )}
    </li>
  );
};

/** A draft's check, verbatim: what passes, then every field of its spec, commands as typed. Text only. */
const VerbatimCheck: React.FC<{ check: Pick<GoalCheck, 'id' | 'label' | 'kind' | 'spec' | 'required' | 'metric' | 'operator' | 'target'> }> = ({ check: c }) => (
  <li className="rounded-lg border border-zinc-800 bg-zinc-950/60 p-2 space-y-1" data-verbatim-check={c.id}>
    <p className={`text-[11px] text-zinc-300 ${wrap}`}>
      <span className="font-medium text-zinc-200">{checkKindLabel(c.kind)}</span>
      {c.label && <span>: {c.label}</span>}
      {!c.required && <span className="text-zinc-500"> (advisory)</span>}
    </p>
    <dl className="space-y-0.5">
      <div className="flex flex-col sm:flex-row sm:gap-2 text-[11px]">
        <dt className="shrink-0 text-zinc-500 sm:w-24">Passes when</dt>
        <dd className={`min-w-0 font-mono text-zinc-100 whitespace-pre-wrap ${wrap}`} data-verbatim-passes>
          {checkDesired(c)}
        </dd>
      </div>
      {specLines(c.spec).map((l) => (
        <div key={l.name} className="flex flex-col sm:flex-row sm:gap-2 text-[11px]">
          <dt className="shrink-0 text-zinc-500 sm:w-24">{l.name}</dt>
          <dd className={`min-w-0 font-mono text-zinc-100 whitespace-pre-wrap ${wrap}`}>{l.value}</dd>
        </div>
      ))}
    </dl>
  </li>
);

export type GoalAction = 'check' | 'pause' | 'resume' | 'edit' | 'delete' | 'approve' | 'dismiss' | 'fix' | 'always' | 'not_now';

/** The buttons a goal offers, with the right ones disabled. Delete asks first, in place. */
const GoalButtons: React.FC<{ goal: Goal; busy: boolean; onAction: (a: GoalAction) => void }> = ({ goal: g, busy, onAction }) => {
  const [confirm, setConfirm] = useState(false);
  const yesRef = useRef<HTMLButtonElement>(null);
  const draft = g.status === 'draft';
  useEffect(() => {
    if (confirm) yesRef.current?.focus();
  }, [confirm]);
  if (confirm)
    return (
      <div role="group" aria-label="Confirm" className="flex flex-wrap items-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/5 p-2" data-confirm-delete>
        <p className="w-full text-[11px] text-rose-200">{draft ? 'Dismiss this proposal? Nothing has run.' : 'Delete this goal? Its checks and history go too. Open fix work stays on the board.'}</p>
        <button
          ref={yesRef}
          type="button"
          disabled={busy}
          onClick={() => {
            setConfirm(false);
            onAction(draft ? 'dismiss' : 'delete');
          }}
          className={dangerBtn}
          data-confirm-yes
        >
          {draft ? 'Dismiss' : 'Delete goal'}
        </button>
        <button type="button" onClick={() => setConfirm(false)} className={quietBtn}>
          Keep it
        </button>
      </div>
    );
  if (draft)
    return (
      <div className="flex flex-wrap items-center gap-1.5">
        <button type="button" disabled={busy} onClick={() => onAction('approve')} className={goodBtn} data-goal-approve>
          {busy ? <Loader2 size={12} className={spin} aria-hidden="true" /> : <CheckCircle2 size={12} aria-hidden="true" />} Approve
        </button>
        <button type="button" disabled={busy} onClick={() => onAction('edit')} className={plainBtn} data-goal-edit>
          Edit
        </button>
        <button type="button" disabled={busy} onClick={() => setConfirm(true)} className={quietBtn} data-goal-dismiss>
          Dismiss
        </button>
      </div>
    );
  const paused = g.status === 'paused';
  const checkOk = canCheckNow(g);
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <button type="button" disabled={busy || !checkOk} onClick={() => onAction('check')} title={checkOk ? undefined : 'Checking now'} className={plainBtn} data-goal-check-now>
        {busy ? <Loader2 size={12} className={spin} aria-hidden="true" /> : <RefreshCw size={12} aria-hidden="true" />} Check now
      </button>
      <button type="button" disabled={busy} onClick={() => onAction(paused ? 'resume' : 'pause')} className={quietBtn} data-goal-pause={paused ? 'resume' : 'pause'}>
        {paused ? <Play size={12} aria-hidden="true" /> : <Pause size={12} aria-hidden="true" />} {paused ? 'Resume' : 'Pause'}
      </button>
      <button type="button" disabled={busy} onClick={() => onAction('edit')} className={quietBtn} data-goal-edit>
        Edit
      </button>
      <button type="button" disabled={busy} onClick={() => setConfirm(true)} className={quietBtn} data-goal-delete>
        <Trash2 size={12} aria-hidden="true" /> Delete
      </button>
    </div>
  );
};

/**
 * The one decision of an incident (B.0 rule 2): what broke and since when, the bot's diagnosis, and
 * Fix it / Not now / Always fix this goal. The diagnosis is the bot's report, shown as text and marked as such.
 * No buttons before there is a diagnosis (D5): until then the bot is still looking, and a decision without
 * its reason would only be a question. If it gives up, the card says so and offers the buttons anyway.
 */
export const DecisionCard: React.FC<{
  goal: Goal;
  decision: Extract<GoalDecision, { kind: 'run' }>;
  names: Record<string, string>;
  now: number;
  busy: boolean;
  onAction: (a: GoalAction) => void;
  compact?: boolean;
}> = ({ goal: g, decision: { diagnosis: d, phase, taskId }, names, now, busy, onAction, compact = false }) => {
  const bot = botName(g.kwant_id, names) ?? 'The bot';
  const since = clockText(d.since, now);
  const report = !!(d.cause || d.proposed_fix || d.summary);
  const looking = phase === 'looking';
  return (
    <div role="group" aria-label={`Decision: ${g.title}`} className={`rounded-lg border p-2.5 space-y-1.5 text-[11px] ${looking ? 'border-zinc-700 bg-zinc-900/60' : 'border-amber-500/30 bg-amber-500/5'}`} data-goal-decision={g.id} data-decision-phase={phase}>
      <p className={`text-zinc-100 ${wrap}`} data-decision-observed>
        {d.observed ?? 'A required check is failing'}
        {since ? ` since ${since}` : ''}.
      </p>
      {looking ? (
        <p className="text-zinc-400" data-decision-looking>
          {bot} is looking into it{since ? ` (since ${since})` : ''}. You’ll get one decision with what it found.
        </p>
      ) : (
        <>
          {report ? (
            <div className="space-y-1" data-decision-report>
              {d.cause && (
                <p className={`text-zinc-300 ${wrap} ${compact ? 'line-clamp-2' : ''}`}>
                  <span className="text-zinc-500">Likely cause: </span>
                  {d.cause}
                </p>
              )}
              {d.proposed_fix && (
                <p className={`text-zinc-300 ${wrap} ${compact ? 'line-clamp-2' : ''}`}>
                  <span className="text-zinc-500">Proposed fix: </span>
                  {d.proposed_fix}
                </p>
              )}
              {!d.cause && !d.proposed_fix && d.summary && (
                <blockquote className={`border-l-2 border-zinc-700 pl-2 text-zinc-300 whitespace-pre-wrap ${wrap} ${compact ? 'line-clamp-3' : ''}`}>{d.summary}</blockquote>
              )}
              <p className="text-[10px] text-zinc-500">From {bot}’s look into it (its report, not instructions).</p>
            </div>
          ) : (
            <p className="text-zinc-400" data-decision-failed>
              {bot} couldn’t finish looking into it. The fix waits for you.
            </p>
          )}
          {taskId && (
            <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
              <button type="button" disabled={busy} onClick={() => onAction('fix')} className={goodBtn} data-goal-fix>
                {busy ? <Loader2 size={12} className={spin} aria-hidden="true" /> : <Wrench size={12} aria-hidden="true" />} Fix it
              </button>
              <button type="button" disabled={busy} onClick={() => onAction('not_now')} className={quietBtn} data-goal-not-now>
                Not now
              </button>
              {g.mode !== 'fix' && (
                <button type="button" disabled={busy} onClick={() => onAction('always')} title="Fix it now, and next time without asking" className={quietBtn} data-goal-always>
                  Always fix this goal
                </button>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
};

/** What a goal says after the person chose Not now: not a request, so no amber and no counting (D1, D3). */
export const declinedText = (at: string | null, now: number): string => {
  const when = clockText(at, now);
  return `${when ? `You chose Not now at ${when}.` : 'Not now.'} The fix waits; it’s dropped if this recovers.`;
};

/**
 * One goal on the strip: its name and status, then the one thing worth knowing now (a bot's proposal, the
 * current work, the Needs you reason, the check that isn't passing as desired vs observed, or when it was
 * checked). With a decision waiting, the decision shows instead, across the strip. The title opens the drawer.
 */
export const GoalRow: React.FC<{
  goal: Goal;
  names: Record<string, string>;
  /** Goal titles by id, for "Waiting for “…”". */
  titles?: Record<string, string>;
  now?: number;
  error?: string | null;
  busy?: boolean;
  /** The incident's decision: Fix it / Not now here, or what Not now left behind. */
  decision?: Extract<GoalDecision, { kind: 'run' | 'declined' }> | null;
  /** Past the first three on a phone, until "Show all". */
  hideOnPhone?: boolean;
  onOpen: (g: Goal) => void;
  onOpenTask?: (id: string) => void;
  onAction?: (a: GoalAction) => void;
}> = ({ goal: g, names, titles = {}, now = Date.now(), error = null, busy = false, decision = null, hideOnPhone = false, onOpen, onOpenTask, onAction }) => {
  const id = useId();
  const draft = g.status === 'draft';
  const proposer = g.created_by && g.created_by !== 'person' ? names[g.created_by] ?? null : null;
  const work = currentWorkText(g, titles);
  const broken = !draft && !work && (g.status === 'not_met' || g.status === 'unknown' || g.status === 'needs_you') ? g.checks.find((c) => c.required && checkLook(c, g.interval_minutes, now).verdict !== 'met') : undefined;
  const fixTask = g.open_fix_task && !g.open_mission ? g.open_fix_task : null;
  const run = decision?.kind === 'run' ? decision : null;
  const declined = decision?.kind === 'declined' ? decision : null;
  const wide = !!(run && onAction);
  return (
    <li
      aria-labelledby={`${id}-t`}
      className={`relative min-w-0 rounded-lg border bg-zinc-900/40 px-2.5 py-1.5 ${wide ? 'sm:col-span-2 xl:col-span-3 space-y-1.5' : 'flex items-center gap-2 hover:bg-zinc-900/70'} ${CARD_EDGE[g.status] ?? 'border-zinc-800'} ${hideOnPhone ? 'max-sm:hidden' : ''}`}
      data-goal={g.id}
      data-status={g.status}
    >
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          {/* Stretched over a compact row, so the whole row is one big target; links sit above it. */}
          <button
            type="button"
            id={`${id}-t`}
            title={g.title}
            onClick={() => onOpen(g)}
            className={`min-w-0 flex-1 truncate text-left text-xs font-medium text-zinc-100 rounded ${wide ? 'hover:underline' : "after:absolute after:inset-0 after:content-['']"} ${focusRing}`}
            data-goal-open
          >
            {g.title}
          </button>
          <GoalStatusPill status={g.status} />
        </div>
        {wide ? null : draft ? (
          <p className="mt-0.5 truncate text-[11px] text-zinc-400" data-goal-line="draft">
            {proposer ? `${proposer} proposed this.` : 'A bot proposed this.'} Nothing runs until you approve it.
          </p>
        ) : declined ? (
          <p className={`mt-0.5 text-[11px] text-zinc-400 ${wrap}`} data-goal-line="declined">
            {declinedText(declined.at, now)}
          </p>
        ) : g.status === 'needs_you' ? (
          <p className="mt-0.5 truncate text-[11px] text-amber-200" data-goal-line="needs-you">
            {needsYouText(g.status_reason)}
          </p>
        ) : fixTask && onOpenTask ? (
          <p className="mt-0.5 flex min-w-0 items-center gap-1 text-[11px]" data-goal-fix-link={fixTask.id}>
            <Wrench size={11} className="shrink-0 text-violet-300" aria-hidden="true" />
            <button type="button" onClick={() => onOpenTask(fixTask.id)} className={`relative z-10 min-w-0 truncate text-left text-sky-300 underline underline-offset-2 hover:text-sky-200 rounded ${focusRing}`} data-open-fix>
              {fixTask.title}
            </button>
            <span className="shrink-0 text-zinc-500">· {fixTask.held ? 'fix ready, waiting for your OK' : stateLabel(fixTask.state).label}</span>
          </p>
        ) : work ? (
          <p className="mt-0.5 flex min-w-0 items-center gap-1 text-[11px] text-zinc-400" data-goal-line="work">
            {g.status === 'waiting' ? <Hourglass size={11} className="shrink-0 text-zinc-500" aria-hidden="true" /> : <Wrench size={11} className="shrink-0 text-violet-300" aria-hidden="true" />}
            <span className="min-w-0 truncate">{work}</span>
          </p>
        ) : broken ? (
          <p className="mt-0.5 truncate text-[11px] text-zinc-400" data-goal-line="check">
            <DesiredObserved check={broken} interval={g.interval_minutes} now={now} />
          </p>
        ) : (
          <p className="mt-0.5 truncate text-[11px] text-zinc-500" data-goal-schedule>
            {scheduleText(g, now)}
          </p>
        )}
      </div>
      {wide && run && onAction && <DecisionCard goal={g} decision={run} names={names} now={now} busy={busy} onAction={onAction} compact />}
      {declined?.taskId && onAction && (
        <button type="button" disabled={busy} onClick={() => onAction('fix')} className={`${quietBtn} relative z-10 shrink-0`} data-goal-fix data-goal-fix-quiet>
          {busy && <Loader2 size={12} className={spin} aria-hidden="true" />} Fix it
        </button>
      )}
      {error && (
        <p className={`text-[11px] text-rose-300 ${wrap}`} data-goal-error>
          {error}
        </p>
      )}
      {draft && !wide && (
        <span className={`${plainBtn} relative z-10 pointer-events-none shrink-0`} aria-hidden="true" data-goal-review>
          Review
        </span>
      )}
    </li>
  );
};

// ---------- New goal: one sentence, and a bot drafts the checks ----------

const BOT_KEY = 'kwnt.goals.bot';
const readBot = () => {
  try {
    return localStorage.getItem(BOT_KEY) ?? '';
  } catch {
    return '';
  }
};
const saveBot = (id: string) => {
  try {
    localStorage.setItem(BOT_KEY, id);
  } catch {
    /* private mode */
  }
};
const sortedBots = (bots: { id: string; name: string }[]) => [...bots].sort((a, b) => a.name.localeCompare(b.name));
/** The bot last used for a goal, else the first by name. */
const defaultBot = (bots: { id: string; name: string }[]) => {
  const last = readBot();
  return bots.some((b) => b.id === last) ? last : sortedBots(bots)[0]?.id ?? '';
};

export const MAX_DRAFTS = 3;

/**
 * "What should stay true?" in a sentence plus the bot, then Draft checks: the bot looks at the project and
 * proposes a draft. "Add checks myself" opens the full form with the sentence carried over.
 */
export const GoalDescribe: React.FC<{
  bots: { id: string; name: string }[];
  drafts: number;
  busy?: boolean;
  error?: string | null;
  onDraft: (text: string, kwantId: string) => void;
  onManual: (text: string) => void;
  onCancel: () => void;
}> = ({ bots, drafts, busy = false, error = null, onDraft, onManual, onCancel }) => {
  const id = useId();
  const [text, setText] = useState('');
  const [bot, setBot] = useState(() => defaultBot(bots));
  const [tried, setTried] = useState(false);
  const textRef = useRef<HTMLTextAreaElement>(null);
  const textErr = text.trim().length < 3 ? 'Say in a sentence what should stay true.' : null;
  const botErr = !bot ? 'Pick the bot that drafts the checks.' : null;
  const full = drafts >= MAX_DRAFTS;
  useEffect(() => {
    textRef.current?.focus();
  }, []);
  return (
    <form
      className="space-y-4"
      noValidate
      data-goal-describe
      onSubmit={(e) => {
        e.preventDefault();
        setTried(true);
        if (textErr || botErr || full) {
          (textErr ? textRef.current : document.getElementById(`${id}-bot`))?.focus();
          return;
        }
        onDraft(text.trim(), bot);
      }}
    >
      <div className="space-y-1">
        <label htmlFor={`${id}-text`} className="block text-xs font-medium text-zinc-200">
          What should stay true?
        </label>
        <textarea
          ref={textRef}
          id={`${id}-text`}
          rows={3}
          maxLength={1000}
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="kwant.si answers in under a second, and sign-up still works."
          aria-invalid={tried && !!textErr}
          aria-describedby={tried && textErr ? `${id}-text-err` : `${id}-text-hint`}
          className={`${field} resize-y`}
          data-field="text"
        />
        {tried && textErr ? (
          <p id={`${id}-text-err`} className="text-[11px] text-rose-300">
            {textErr}
          </p>
        ) : (
          <p id={`${id}-text-hint`} className="text-[10px] text-zinc-500">
            One sentence is enough. The bot looks at the project and proposes checks that prove it.
          </p>
        )}
      </div>
      <div className="space-y-1">
        <label htmlFor={`${id}-bot`} className="block text-xs font-medium text-zinc-200">
          Which bot drafts the checks
        </label>
        <select id={`${id}-bot`} value={bot} onChange={(e) => setBot(e.target.value)} aria-invalid={tried && !!botErr} className={field} data-field="bot">
          {bots.length === 0 && <option value="">No bots on this board yet</option>}
          {sortedBots(bots).map((b) => (
            <option key={b.id} value={b.id}>
              {b.name}
            </option>
          ))}
        </select>
        <p className="text-[10px] text-zinc-500">It changes nothing now. If the goal breaks later, this bot looks into it and suggests the fix.</p>
        {tried && botErr && <p className="text-[11px] text-rose-300">{botErr}</p>}
      </div>
      {full && (
        <p className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-amber-200" data-drafts-full>
          {MAX_DRAFTS} drafts are waiting for you. Approve or dismiss one first, or add the checks yourself.
        </p>
      )}
      {error && (
        <p role="alert" className={`rounded-lg border border-rose-500/30 bg-rose-500/5 px-3 py-2 text-[11px] text-rose-200 ${wrap}`} data-form-error>
          {error}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <button type="submit" disabled={busy || full} className={mainBtn} data-goal-draft>
          {busy && <Loader2 size={12} className={spin} aria-hidden="true" />}
          Draft checks
        </button>
        <button type="button" onClick={() => onManual(text.trim())} className={quietBtn} data-goal-manual>
          Add checks myself
        </button>
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        <p className="w-full text-[10px] text-zinc-500">The draft shows on the board. Nothing is checked until you approve it.</p>
      </div>
    </form>
  );
};

// ---------- Add checks myself / Edit ----------

const TARGET_LABEL: Record<string, { label: string; placeholder: string }> = {
  status: { label: 'Status', placeholder: '200' },
  latency_ms: { label: 'Milliseconds', placeholder: '500' },
  body_contains: { label: 'Text', placeholder: 'ok' },
  json_value: { label: 'Value', placeholder: 'ok' },
  exit_code: { label: 'Code', placeholder: '0' },
  stdout_contains: { label: 'Text', placeholder: 'passed' },
  stdout_number: { label: 'Number', placeholder: '10' },
};

const CheckRowFields: React.FC<{
  row: CheckFormRow;
  index: number;
  error: string | null;
  canRemove: boolean;
  onChange: (r: CheckFormRow) => void;
  onRemove: () => void;
}> = ({ row: r, index, error, canRemove, onChange, onRemove }) => {
  const id = useId();
  const errId = `${id}-err`;
  const set = (patch: Partial<CheckFormRow>) => onChange({ ...r, ...patch });
  const invalid = !!error;
  const describedBy = invalid ? errId : undefined;
  const input = (key: keyof CheckFormRow, label: string, placeholder: string, mono = false, extra = '') => (
    <div className={`min-w-0 space-y-1 ${extra}`}>
      <label htmlFor={`${id}-${key}`} className="block text-[11px] text-zinc-400">
        {label}
      </label>
      <input
        id={`${id}-${key}`}
        value={r[key] as string}
        onChange={(e) => set({ [key]: e.target.value } as Partial<CheckFormRow>)}
        placeholder={placeholder}
        aria-invalid={invalid}
        aria-describedby={describedBy}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        className={`${field} ${mono ? 'font-mono' : ''}`}
        data-row-field={key}
      />
    </div>
  );
  const typed = r.kind === 'http' || r.kind === 'command';
  const info = typed ? metricInfo(r.kind, r.metric) : null;
  const target = TARGET_LABEL[r.metric] ?? { label: 'Value', placeholder: '' };
  return (
    <li className="rounded-lg border border-zinc-800 bg-zinc-950/40 p-2.5 space-y-2" data-check-row={r.key}>
      <div className="flex items-end gap-2">
        <div className="min-w-0 flex-1 space-y-1">
          <label htmlFor={`${id}-kind`} className="block text-[11px] text-zinc-400">
            Check {index + 1}
          </label>
          <select id={`${id}-kind`} value={r.kind} onChange={(e) => onChange(withKind(r, e.target.value as CheckFormRow['kind']))} className={field} data-row-field="kind">
            {GOAL_CHECK_KINDS.map((k) => (
              <option key={k} value={k}>
                {checkKindLabel(k)}
              </option>
            ))}
          </select>
        </div>
        {canRemove && (
          <button type="button" onClick={onRemove} aria-label={`Remove check ${index + 1}`} className={`${quietBtn} shrink-0`}>
            <X size={14} aria-hidden="true" />
          </button>
        )}
      </div>
      {r.kind === 'command' && input('command', 'Command to run in the bot’s folder', 'npm test', true)}
      {r.kind === 'http' && input('url', 'Web address', 'https://example.com/health')}
      {typed && info && (
        <fieldset className="space-y-1" data-predicate>
          <legend className="text-[11px] text-zinc-400">Passes when</legend>
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1fr)]">
            <select aria-label="What to look at" value={r.metric} onChange={(e) => onChange(withMetric(r, e.target.value))} className={`${field} col-span-2 sm:col-span-1`} data-row-field="metric">
              {METRICS[r.kind as 'http' | 'command'].map((m) => (
                <option key={m.value} value={m.value}>
                  {m.label}
                </option>
              ))}
            </select>
            {info.operators.length > 1 ? (
              <select aria-label="How to compare" value={r.operator} onChange={(e) => set({ operator: e.target.value as GoalOperator })} className={field} data-row-field="operator">
                {info.operators.map((o) => (
                  <option key={o} value={o}>
                    {OPERATOR_WORDS[o]}
                  </option>
                ))}
              </select>
            ) : (
              <span className="flex min-h-[40px] sm:min-h-[36px] items-center px-1 text-xs text-zinc-400">{info.operators.length === 1 ? OPERATOR_WORDS[info.operators[0]] : 'has'}</span>
            )}
            <input
              aria-label={target.label}
              value={r.target}
              onChange={(e) => set({ target: e.target.value })}
              placeholder={target.placeholder}
              inputMode={info.textTarget || r.metric === 'json_value' ? undefined : 'numeric'}
              aria-invalid={invalid}
              aria-describedby={describedBy}
              spellCheck={false}
              className={`${field} ${info.textTarget ? 'col-span-2 sm:col-span-1' : ''}`}
              data-row-field="target"
            />
          </div>
          {r.metric === 'json_value' && input('pointer', 'Where the value is in the answer', '/status', true)}
          {r.metric === 'stdout_number' && <p className="text-[10px] text-zinc-500">Uses the first number the command prints.</p>}
        </fieldset>
      )}
      {(r.kind === 'file_exists' || r.kind === 'file_contains' || r.kind === 'file_sha256') && input('path', 'File, in the bot’s folder', 'dist/index.html', true)}
      {r.kind === 'file_contains' && input('text', 'Text it should contain', 'Version 2')}
      {r.kind === 'file_sha256' && input('sha256', 'Fingerprint (SHA-256)', '64 letters and digits', true)}
      <div className="grid gap-2 sm:grid-cols-[1fr_auto] sm:items-end">
        {input('label', 'Name (optional)', 'Site answers')}
        <label className="inline-flex items-center gap-2 min-h-[40px] sm:min-h-[36px] text-[11px] text-zinc-300" title="An advisory check shows on the card but doesn’t decide OK">
          <input type="checkbox" checked={r.required} onChange={(e) => set({ required: e.target.checked })} className="h-4 w-4 accent-sky-500" data-row-field="required" />
          Required for OK
        </label>
      </div>
      {error && (
        <p id={errId} className="text-[11px] text-rose-300" data-row-error>
          {error}
        </p>
      )}
    </li>
  );
};

export const GoalForm: React.FC<{
  goal: Goal | null;
  bots: { id: string; name: string }[];
  /** The sentence from "What should stay true?", carried over. */
  seed?: string;
  saving?: boolean;
  error?: string | null;
  onSave: (d: GoalDraft) => void;
  onCancel: () => void;
}> = ({ goal, bots, seed = '', saving = false, error = null, onSave, onCancel }) => {
  const id = useId();
  const formRef = useRef<HTMLFormElement>(null);
  const [f, setF] = useState<GoalFormState>(() => {
    const base = formFromGoal(goal);
    if (goal) return base;
    const s = seed.trim();
    return { ...base, kwantId: defaultBot(bots), title: s.length <= 120 ? s : '', description: s };
  });
  const [tried, setTried] = useState(false);
  const [more, setMore] = useState(false);
  const errors = useMemo(() => goalFormErrors(f), [f]);
  const show = tried ? errors : {};
  const set = (patch: Partial<GoalFormState>) => setF((cur) => ({ ...cur, ...patch }));
  const draft = goal?.status === 'draft';
  // A bot this person can't see any more still shows, so editing doesn't silently move the goal.
  const botOptions = useMemo(() => {
    const list = sortedBots(bots);
    if (f.kwantId && !list.some((b) => b.id === f.kwantId)) list.unshift({ id: f.kwantId, name: 'A bot not on this board' });
    return list;
  }, [bots, f.kwantId]);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setTried(true);
    if (hasFormErrors(errors)) {
      // A problem behind "More options" opens it. Then move to the first problem.
      if (errors.budget) setMore(true);
      requestAnimationFrame(() => formRef.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus());
      return;
    }
    if (f.kwantId) saveBot(f.kwantId);
    onSave(draftFromForm(f));
  };

  return (
    <form ref={formRef} onSubmit={submit} noValidate className="space-y-4" data-goal-form>
      <div className="space-y-1">
        <label htmlFor={`${id}-title`} className="block text-xs font-medium text-zinc-200">
          Name
        </label>
        <input
          id={`${id}-title`}
          value={f.title}
          onChange={(e) => set({ title: e.target.value })}
          placeholder="The site stays up"
          aria-invalid={!!show.title}
          aria-describedby={show.title ? `${id}-title-err` : undefined}
          className={field}
          data-field="title"
        />
        {show.title && (
          <p id={`${id}-title-err`} className="text-[11px] text-rose-300">
            {show.title}
          </p>
        )}
      </div>
      <div className="space-y-1">
        <label htmlFor={`${id}-desc`} className="block text-xs font-medium text-zinc-200">
          What should stay true <span className="font-normal text-zinc-500">(optional)</span>
        </label>
        <textarea
          id={`${id}-desc`}
          value={f.description}
          onChange={(e) => set({ description: e.target.value })}
          rows={2}
          placeholder="kwant.si answers, and the sign-up page loads."
          className={`${field} resize-y`}
          data-field="description"
        />
        <p className="text-[10px] text-zinc-500">If it breaks, the fix starts from this and the failing check’s output.</p>
      </div>

      <fieldset className="space-y-2" data-field="checks">
        <legend className="text-xs font-medium text-zinc-200">Checks</legend>
        <p className="text-[10px] text-zinc-500">These run on their own, without a bot. OK needs every required check to pass.</p>
        <ul className="space-y-2">
          {f.checks.map((r, i) => (
            <CheckRowFields
              key={r.key}
              row={r}
              index={i}
              error={show.rows?.[r.key] ?? null}
              canRemove={f.checks.length > 1}
              onChange={(nr) => set({ checks: f.checks.map((x) => (x.key === r.key ? nr : x)) })}
              onRemove={() => set({ checks: f.checks.filter((x) => x.key !== r.key) })}
            />
          ))}
        </ul>
        {show.checks && (
          <p className="text-[11px] text-rose-300" role="alert">
            {show.checks}
          </p>
        )}
        {f.checks.length < MAX_CHECKS && (
          <button type="button" onClick={() => set({ checks: [...f.checks, emptyCheckRow()] })} className={plainBtn} data-add-check>
            <Plus size={12} aria-hidden="true" /> Add a check
          </button>
        )}
      </fieldset>

      <div className="space-y-1">
        <label htmlFor={`${id}-bot`} className="block text-xs font-medium text-zinc-200">
          Which bot fixes it
        </label>
        <select
          id={`${id}-bot`}
          value={f.kwantId}
          onChange={(e) => set({ kwantId: e.target.value })}
          aria-invalid={!!show.kwantId}
          aria-describedby={show.kwantId ? `${id}-bot-err` : undefined}
          className={field}
          data-field="bot"
        >
          <option value="">{f.mode === 'watch' ? 'No bot: just tell me' : 'Pick a bot'}</option>
          {botOptions.map((b) => (
            <option key={b.id} value={b.id}>
              {b.name}
            </option>
          ))}
        </select>
        {show.kwantId && (
          <p id={`${id}-bot-err`} className="text-[11px] text-rose-300">
            {show.kwantId}
          </p>
        )}
        {!show.kwantId && f.mode !== 'watch' && <p className="text-[10px] text-zinc-500">If it breaks: {MODE_OPTIONS.find((o) => o.value === f.mode)?.hint}</p>}
      </div>

      {/* Everything with a default sits behind More options (B.0 rule 6). */}
      <div className="rounded-lg border border-zinc-800" data-more-options>
        <button
          type="button"
          aria-expanded={more}
          aria-controls={`${id}-more`}
          onClick={() => setMore((v) => !v)}
          className={`flex w-full items-center justify-between gap-2 min-h-[40px] px-2.5 text-left text-xs text-zinc-200 rounded-lg hover:bg-zinc-900 ${focusRing}`}
          data-more-toggle
        >
          <span>
            More options
            <span className="text-zinc-500">
              {' '}
              · every {intervalLabel(f.interval)}, {modeLabel(f.mode).toLowerCase()}, {priorityLabel(f.priority).toLowerCase()} priority
            </span>
          </span>
          <ChevronDown size={14} className={`shrink-0 ${more ? 'rotate-180' : ''}`} aria-hidden="true" />
        </button>
        {more && (
          <div id={`${id}-more`} className="space-y-4 border-t border-zinc-800 p-2.5">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="min-w-0 space-y-1">
                <label htmlFor={`${id}-every`} className="block text-xs font-medium text-zinc-200">
                  Check every
                </label>
                <select id={`${id}-every`} value={f.interval} onChange={(e) => set({ interval: Number(e.target.value) })} className={field} data-field="interval">
                  {intervalOptions(f.interval).map((m) => (
                    <option key={m} value={m}>
                      {intervalLabel(m)}
                    </option>
                  ))}
                </select>
              </div>
              <div className="min-w-0 space-y-1">
                <label htmlFor={`${id}-prio`} className="block text-xs font-medium text-zinc-200">
                  Priority
                </label>
                <select id={`${id}-prio`} value={f.priority} onChange={(e) => set({ priority: Number(e.target.value) })} className={field} data-field="priority">
                  {PRIORITY_OPTIONS.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                      {o.value === 2 ? ' (default)' : ''}
                    </option>
                  ))}
                </select>
                <p className="text-[10px] text-zinc-500">When two goals in one folder break, the higher one is fixed first.</p>
              </div>
            </div>
            <fieldset className="space-y-1.5" data-field="mode">
              <legend className="text-xs font-medium text-zinc-200">When it breaks</legend>
              {MODE_OPTIONS.map((o) => (
                <label key={o.value} className={`flex items-start gap-2 rounded-lg border px-2.5 py-2 min-h-[40px] cursor-pointer ${f.mode === o.value ? 'border-sky-500/50 bg-sky-500/10' : 'border-zinc-800 hover:border-zinc-700'}`}>
                  <input type="radio" name={`${id}-mode`} value={o.value} checked={f.mode === o.value} onChange={() => set({ mode: o.value })} className="mt-0.5 h-4 w-4 accent-sky-500" />
                  <span className="min-w-0">
                    <span className="block text-xs text-zinc-100">
                      {o.label}
                      {o.value === 'suggest' && <span className="text-zinc-500"> (default)</span>}
                    </span>
                    <span className="block text-[11px] text-zinc-500">{o.hint}</span>
                  </span>
                </label>
              ))}
            </fieldset>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="min-w-0 space-y-1">
                <label htmlFor={`${id}-confirm`} className="block text-xs font-medium text-zinc-200">
                  Counts as broken after
                </label>
                <select id={`${id}-confirm`} value={f.confirmFailures} onChange={(e) => set({ confirmFailures: Number(e.target.value) })} className={field} data-field="confirm">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <option key={n} value={n}>
                      {n === 1 ? '1 failed check' : `${n} failed checks in a row`}
                      {n === 2 ? ' (default)' : ''}
                    </option>
                  ))}
                </select>
              </div>
              <div className="min-w-0 space-y-1">
                <label htmlFor={`${id}-budget`} className="block text-xs font-medium text-zinc-200">
                  Token limit for fixes
                </label>
                <input
                  id={`${id}-budget`}
                  value={f.budget}
                  onChange={(e) => set({ budget: e.target.value })}
                  inputMode="numeric"
                  placeholder="No limit"
                  aria-invalid={!!show.budget}
                  aria-describedby={show.budget ? `${id}-budget-err` : undefined}
                  className={field}
                  data-field="budget"
                />
                {show.budget && (
                  <p id={`${id}-budget-err`} className="text-[11px] text-rose-300">
                    {show.budget}
                  </p>
                )}
              </div>
            </div>
          </div>
        )}
      </div>

      {error && (
        <p role="alert" className={`rounded-lg border border-rose-500/30 bg-rose-500/5 px-3 py-2 text-[11px] text-rose-200 ${wrap}`} data-form-error>
          {error}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2 pt-1">
        <button type="submit" disabled={saving} className={mainBtn} data-goal-save>
          {saving && <Loader2 size={12} className={spin} aria-hidden="true" />}
          {goal ? 'Save changes' : 'Create goal'}
        </button>
        <button type="button" onClick={onCancel} className={quietBtn}>
          Cancel
        </button>
        {!goal && <p className="w-full text-[10px] text-zinc-500">It’s checked right away, then every {intervalLabel(f.interval)}. You hear about it only when it breaks or recovers.</p>}
        {draft && <p className="w-full text-[10px] text-zinc-500">It stays a draft until you approve it.</p>}
      </div>
    </form>
  );
};

// ---------- Drawer ----------

/** The goal drawer: a side panel on desktop, full screen on a phone. Escape closes it and focus goes back. */
export const GoalPanel: React.FC<{ title: string; onClose: () => void; children: React.ReactNode }> = ({ title, onClose, children }) => {
  const closeRef = useRef<HTMLButtonElement>(null);
  const headingId = useId();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const back = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close.current();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      // A goal deleted from its drawer takes its row with it: focus goes to New goal instead of the page.
      if (back?.isConnected) back.focus();
      else document.querySelector<HTMLElement>('[data-new-goal]')?.focus();
    };
  }, []);
  return (
    <div className="fixed inset-0 z-50 flex justify-end" data-goal-panel>
      <button type="button" aria-label="Close" tabIndex={-1} onClick={onClose} className="absolute inset-0 bg-black/50 cursor-default" />
      <div role="dialog" aria-modal="true" aria-labelledby={headingId} className="relative w-full sm:max-w-xl h-full flex flex-col bg-zinc-950 sm:border-l border-zinc-800 shadow-2xl">
        <div className="min-h-14 py-2 px-3 sm:px-5 border-b border-zinc-800/80 flex items-start gap-2 bg-zinc-900/30">
          <h3 id={headingId} className={`flex-1 min-w-0 pt-1.5 text-sm font-semibold text-zinc-100 ${wrap}`}>
            {title}
          </h3>
          <button ref={closeRef} type="button" onClick={onClose} aria-label="Close" className={`shrink-0 p-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 ${focusRing}`}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-5">{children}</div>
      </div>
    </div>
  );
};

const DOT: Record<ReturnType<typeof goalEventTone>, string> = {
  good: 'bg-emerald-400',
  bad: 'bg-rose-400',
  warn: 'bg-amber-400',
  plain: 'bg-zinc-500',
};

/** Oldest first, as the story went: broke → looked into → approved → fixed → verified. */
export const GoalTimeline: React.FC<{ events: GoalEvent[]; names: Record<string, string>; now: number; onOpenTask?: (id: string) => void }> = ({ events, names, now, onOpenTask }) => {
  if (events.length === 0) return <p className="text-[11px] text-zinc-500">Nothing has happened yet.</p>;
  return (
    <ol className="space-y-2 border-l border-zinc-800 pl-3" data-goal-timeline>
      {events.map((e) => {
        const task = eventTaskId(e);
        const quote = goalEventQuote(e);
        return (
          <li key={e.id} className="relative text-[11px]" data-event={e.kind}>
            <span className={`absolute -left-[17px] top-1 h-2 w-2 rounded-full ${DOT[goalEventTone(e.kind)]}`} aria-hidden="true" />
            <span className={`text-zinc-200 ${wrap}`}>{goalEventText(e, names)}</span>
            {task && onOpenTask && (
              <>
                {' '}
                <button type="button" onClick={() => onOpenTask(task)} className={`text-sky-300 underline underline-offset-2 hover:text-sky-200 rounded ${focusRing}`}>
                  Open task
                </button>
              </>
            )}
            <time dateTime={e.created_at} title={e.created_at ? new Date(e.created_at).toLocaleString() : undefined} className="text-zinc-500">
              {' '}
              · {ago(e.created_at, now)}
            </time>
            {quote && (
              <blockquote className={`mt-1 max-h-32 overflow-auto border-l-2 border-zinc-700 pl-2 text-zinc-400 whitespace-pre-wrap ${wrap}`} data-event-quote>
                {quote}
              </blockquote>
            )}
          </li>
        );
      })}
    </ol>
  );
};

const missionLine = (m: GoalMission) =>
  [m.status ? missionStatusLabel(m.status).label : null, m.attempt > 1 ? `attempt ${m.attempt}` : null, m.outcome ? m.outcome.replace(/_/g, ' ') : null].filter(Boolean).join(' · ');

export const GoalDetailBody: React.FC<{
  detail: GoalDetail;
  names: Record<string, string>;
  titles?: Record<string, string>;
  now: number;
  busy: boolean;
  error: string | null;
  onAction: (a: GoalAction) => void;
  onOpenTask?: (id: string) => void;
  onOpenMission?: (id: string) => void;
  /** The held task this person already said Not now to on this device. */
  notNowTask?: string | null;
}> = ({ detail: d, names, titles = {}, now, busy, error, onAction, onOpenTask, onOpenMission, notNowTask = null }) => {
  const raw: Goal = { ...d.goal, checks: d.checks.length ? d.checks : d.goal.checks };
  const g: Goal = { ...raw, raw_status: raw.status, status: displayStatus(raw, d.events, now, notNowTask) };
  const draft = g.status === 'draft';
  const look = goalStatusLook(g.status);
  const decision = goalDecision(g, d.events, now, notNowTask);
  const work = currentWorkText(g, titles);
  const missions = d.missions;
  return (
    <div className="space-y-5" data-goal-detail={g.id}>
      <div className="space-y-1.5">
        <div className="flex flex-wrap items-center gap-2">
          <GoalStatusPill status={g.status} />
          {/* A draft's hint already says nothing runs yet. */}
          {!draft && <span className="text-[11px] text-zinc-500">{scheduleText(g, now)}</span>}
        </div>
        <p className="text-[11px] text-zinc-400">{look.hint}</p>
      </div>

      {decision?.kind === 'run' && <DecisionCard goal={g} decision={decision} names={names} now={now} busy={busy} onAction={onAction} />}
      {decision?.kind === 'declined' && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-zinc-700 bg-zinc-900/60 px-3 py-2 text-[11px]" data-goal-declined>
          <p className={`min-w-0 flex-1 text-zinc-300 ${wrap}`}>{declinedText(decision.at, now)}</p>
          {decision.taskId && (
            <button type="button" disabled={busy} onClick={() => onAction('fix')} className={quietBtn} data-goal-fix data-goal-fix-quiet>
              Fix it
            </button>
          )}
        </div>
      )}
      {decision?.kind === 'needs_you' && (
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] space-y-1" data-goal-needs-you>
          <p className={`text-amber-200 ${wrap}`}>{decision.text}.</p>
          <p className="text-zinc-400">Nothing runs on its own until you act: look at the checks below, then Check now, or edit the goal.</p>
        </div>
      )}
      {draft && (
        <p className="rounded-lg border border-sky-500/30 bg-sky-500/5 px-3 py-2 text-[11px] text-sky-100" data-goal-draft-note>
          {(g.created_by && g.created_by !== 'person' && names[g.created_by]) || 'A bot'} proposed this. Read the checks below: they run exactly as shown, and only after you approve.
        </p>
      )}

      {g.description && (
        <section className="space-y-1">
          <h4 className={heading}>What should stay true</h4>
          <p className={`text-xs text-zinc-200 whitespace-pre-wrap ${wrap}`}>{g.description}</p>
        </section>
      )}
      <section className="space-y-1.5">
        <h4 className={heading}>{draft ? 'Checks it would run' : 'Checks'}</h4>
        {g.checks.length === 0 ? (
          <p className="text-[11px] text-zinc-500">No checks.</p>
        ) : draft ? (
          <ul className="space-y-1.5">
            {g.checks.map((c) => (
              <VerbatimCheck key={c.id} check={c} />
            ))}
          </ul>
        ) : (
          <ul className="space-y-2">
            {g.checks.map((c) => (
              <GoalCheckLine key={c.id} check={c} interval={g.interval_minutes} now={now} showOutput />
            ))}
          </ul>
        )}
      </section>

      {(missions.length > 0 || d.fix_tasks.length > 0 || work) && (
        <section className="space-y-1.5" data-goal-work>
          <h4 className={heading}>Current work</h4>
          {g.status === 'waiting' && work && <p className="text-[11px] text-zinc-300">{work}</p>}
          {missions.length > 0 && (
            <ul className="space-y-1">
              {missions.map((m) => (
                <li key={`${m.id}-${m.attempt}`} className="flex items-start gap-1.5 text-[11px]" data-goal-mission={m.id} data-open={isOpenMission(m) ? '1' : '0'}>
                  <Wrench size={12} className={`mt-0.5 shrink-0 ${isOpenMission(m) ? 'text-violet-300' : 'text-zinc-600'}`} aria-hidden="true" />
                  <span className="min-w-0">
                    {onOpenMission ? (
                      <button type="button" onClick={() => onOpenMission(m.id)} className={`text-left text-sky-300 underline underline-offset-2 hover:text-sky-200 rounded ${wrap} ${focusRing}`}>
                        {m.title}
                      </button>
                    ) : (
                      <span className={`text-zinc-200 ${wrap}`}>{m.title}</span>
                    )}
                    {missionLine(m) && <span className="text-zinc-500"> · {missionLine(m)}</span>}
                    {isOpenMission(m) && m.run_task_id && <span className="text-amber-200"> · fix ready, waiting for your OK</span>}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {missions.length === 0 && d.fix_tasks.length > 0 && (
            <ul className="space-y-1">
              {d.fix_tasks.map((t) => (
                <li key={t.id} className="flex items-start gap-1.5 text-[11px]" data-goal-fix-task={t.id}>
                  <Wrench size={12} className="mt-0.5 shrink-0 text-zinc-500" aria-hidden="true" />
                  <span className="min-w-0">
                    {onOpenTask ? (
                      <button type="button" onClick={() => onOpenTask(t.id)} className={`text-left text-sky-300 underline underline-offset-2 hover:text-sky-200 rounded ${wrap} ${focusRing}`}>
                        {t.title}
                      </button>
                    ) : (
                      <span className={`text-zinc-200 ${wrap}`}>{t.title}</span>
                    )}
                    <span className="text-zinc-500"> · {t.held ? 'fix ready, waiting for your OK' : stateLabel(t.state).label}</span>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11px]">
        {draft && (
          <>
            <dt className="text-zinc-500">Proposed by</dt>
            <dd className={`text-zinc-200 ${wrap}`}>{(g.created_by && names[g.created_by]) ?? 'A bot'}</dd>
          </>
        )}
        <dt className="text-zinc-500">Fixed by</dt>
        <dd className={`text-zinc-200 ${wrap}`}>{botName(g.kwant_id, names) ?? 'No bot: you just hear about it'}</dd>
        <dt className="text-zinc-500">When it breaks</dt>
        <dd className="text-zinc-200">{modeLabel(g.mode)}</dd>
        <dt className="text-zinc-500">Check every</dt>
        <dd className="text-zinc-200">{intervalLabel(g.interval_minutes)}</dd>
        <dt className="text-zinc-500">Priority</dt>
        <dd className="text-zinc-200">{priorityLabel(g.priority)}</dd>
        <dt className="text-zinc-500">Spent on fixes</dt>
        <dd className="text-zinc-200">{spentText(d.spent_tokens) ?? 'Nothing yet'}</dd>
      </dl>

      <section className="space-y-1.5">
        <h4 className={heading}>What happened</h4>
        <GoalTimeline events={d.events} names={names} now={now} onOpenTask={onOpenTask} />
      </section>
      {error && (
        <p role="alert" className={`text-[11px] text-rose-300 ${wrap}`} data-goal-detail-error>
          {error}
        </p>
      )}
      <GoalButtons goal={g} busy={busy} onAction={onAction} />
    </div>
  );
};

// ---------- Keep this true (on a verified task) ----------

const passingOutcomeChecks = (checks: Check[]) => checks.filter((c) => c.required && c.status === 'passed');

/**
 * "Keep this true" on a Verified task with passing outcome checks: one confirm sheet showing the checks
 * (B.0 rule 6). Confirming makes an active goal: the server copies the task's title and checks, and a draft
 * it hands back is approved in the same tap.
 */
export const KeepTrueBox: React.FC<{
  task: Pick<Task, 'id' | 'title' | 'state' | 'owner_kwant_id'>;
  checks: Check[];
  names?: Record<string, string>;
  api?: Pick<GoalsApi, 'fromTask' | 'approve'>;
  onOpenGoal?: (id: string) => void;
}> = ({ task, checks, names = {}, api = goalsApi, onOpenGoal = openGoal }) => {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [made, setMade] = useState<Goal | null>(null);
  // The server sends the same goal back for a repeat (200): then it was kept true before this tap.
  const [already, setAlready] = useState(false);
  const keys = useRef<{ make?: string; approve?: string }>({});
  const busyRef = useRef(false);
  const yesRef = useRef<HTMLButtonElement>(null);
  const passing = passingOutcomeChecks(checks);
  useEffect(() => {
    if (open) yesRef.current?.focus();
  }, [open]);
  if (task.state !== 'verified' || passing.length === 0) return null;
  const bot = botName(task.owner_kwant_id ?? null, names);
  const confirm = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      const tapped = Date.now();
      let g = await api.fromTask(task.id, (keys.current.make ??= newIdempotencyKey()));
      if (g.status === 'draft') g = await api.approve(g.id, (keys.current.approve ??= newIdempotencyKey()));
      keys.current = {};
      const made_at = Date.parse(g.created_at);
      setAlready(!Number.isNaN(made_at) && made_at < tapped - 2000);
      setMade(g);
      setOpen(false);
    } catch (e) {
      setError(errText(e, 'Couldn’t make the goal. Try again.'));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  if (made)
    return (
      <p role="status" className={`flex flex-wrap items-center gap-x-2 text-[11px] text-emerald-300 ${wrap}`} data-keep-true-done>
        <CheckCircle2 size={13} aria-hidden="true" />{' '}
        {already ? `Already kept true: “${made.title}”.` : `Goal added: “${made.title}”. It’s checked every ${intervalLabel(made.interval_minutes)}.`}
        <button type="button" onClick={() => onOpenGoal(made.id)} className={linkBtn} data-keep-true-open>
          Open goal
        </button>
      </p>
    );
  if (!open)
    return (
      <button type="button" onClick={() => setOpen(true)} className={quietBtn} data-keep-true>
        <Target size={13} aria-hidden="true" /> Keep this true
      </button>
    );
  return (
    <div role="group" aria-label="Keep this true" className="rounded-lg border border-sky-500/30 bg-sky-500/5 p-3 space-y-2 text-[11px]" data-keep-true-sheet>
      <p className="text-xs font-medium text-zinc-100">Keep this true</p>
      <p className="text-zinc-300">
        These checks keep running every hour.{bot ? ` If one breaks, ${bot} looks into it and suggests a fix for your OK.` : ' If one breaks, you hear about it.'}
      </p>
      <ul className="space-y-1">
        {passing.map((c) => (
          <li key={c.id} className="rounded-lg border border-zinc-800 bg-zinc-950/60 px-2 py-1.5" data-keep-true-check={c.id}>
            <span className="text-zinc-400">{checkKindLabel(c.kind)}: </span>
            <span className={`font-mono text-zinc-100 ${wrap}`}>{checkTarget(c)}</span>
          </li>
        ))}
      </ul>
      {error && (
        <p role="alert" className={`text-rose-300 ${wrap}`} data-keep-true-error>
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button ref={yesRef} type="button" disabled={busy} onClick={() => void confirm()} className={mainBtn} data-keep-true-yes>
          {busy && <Loader2 size={12} className={spin} aria-hidden="true" />}
          Keep it true
        </button>
        <button type="button" disabled={busy} onClick={() => setOpen(false)} className={quietBtn}>
          Never mind
        </button>
      </div>
    </div>
  );
};

/** "Goal: <title>" on a mission made for a goal; opens the goal's drawer. */
export const GoalChip: React.FC<{ goal: { id: string; title: string }; onOpen?: (id: string) => void }> = ({ goal, onOpen = openGoal }) => (
  <button
    type="button"
    onClick={() => onOpen(goal.id)}
    title={`Open the goal “${goal.title}”`}
    className={`inline-flex max-w-full items-center gap-1 min-h-[32px] sm:min-h-[24px] rounded-full border border-sky-500/30 bg-sky-500/10 px-2 text-[11px] text-sky-200 hover:bg-sky-500/20 ${focusRing}`}
    data-goal-chip={goal.id}
  >
    <Target size={11} className="shrink-0" aria-hidden="true" />
    <span className="truncate">Goal: {goal.title}</span>
  </button>
);

// ---------- The strip ----------

type Panel = { kind: 'new' } | { kind: 'form'; seed: string } | { kind: 'edit'; goal: Goal } | { kind: 'goal'; id: string };

const OPEN_KEY = 'kwnt.goals.open';
const NOT_NOW_KEY = 'kwnt.goals.notnow';
const readJson = <T,>(key: string, fallback: T): T => {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
};
const writeJson = (key: string, v: unknown) => {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    /* private mode */
  }
};

// Cards shown on a phone before "Show all": three.
const PHONE_CAP = 3;
// The strip's own phone width (Tailwind's sm), so the rows and the default agree.
const PHONE_QUERY = '(max-width: 639px)';
const usePhone = (): boolean => {
  const [phone, setPhone] = useState(() => typeof window !== 'undefined' && !!window.matchMedia?.(PHONE_QUERY).matches);
  useEffect(() => {
    const mq = window.matchMedia?.(PHONE_QUERY);
    const onMq = () => setPhone(!!mq?.matches);
    mq?.addEventListener?.('change', onMq);
    return () => mq?.removeEventListener?.('change', onMq);
  }, []);
  return phone;
};

/** How long "is drafting checks" shows before it gives up waiting for the draft. */
const DRAFT_WAIT_MS = 10 * 60_000;

/**
 * The board's Goals strip. Loads its goals; tests and screenshots pass `initial` (the goals as loaded) and
 * the API. Renders nothing while loading, or on a server without Goals (404/405), so the board looks as in
 * 0.6.4. `hidden` keeps only the drawer (a mission's own view, where the strip doesn't show but its
 * "Goal:" chip still opens the goal).
 */
export const GoalsStrip: React.FC<{
  names: Record<string, string>;
  /** Bots that can draft and fix a goal; defaults to everyone in `names`. */
  bots?: { id: string; name: string }[];
  spaceId?: string | null;
  now?: number;
  onOpenTask?: (id: string) => void;
  onOpenMission?: (id: string) => void;
  initial?: { goals: Goal[] };
  /** Open the goal drawer for this id at first (screenshots, a notification tap). */
  openGoalId?: string;
  /** Open this panel at first (screenshots). */
  openPanel?: 'new' | 'form';
  hidden?: boolean;
  api?: GoalsApi;
}> = ({ names, bots, spaceId = null, now = Date.now(), onOpenTask, onOpenMission, initial, openGoalId, openPanel, hidden = false, api = goalsApi }) => {
  const headId = useId();
  const listId = useId();
  const [goals, setGoals] = useState<Goal[] | null>(initial ? initial.goals : null);
  const [unsupported, setUnsupported] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);
  const [open, setOpen] = useState<boolean>(() => readJson<unknown>(OPEN_KEY, false) === 1 || readJson<unknown>(OPEN_KEY, false) === true);
  const [notNow, setNotNow] = useState<Record<string, string>>(() => {
    const v = readJson<unknown>(NOT_NOW_KEY, {});
    return v && typeof v === 'object' ? (v as Record<string, string>) : {};
  });
  const phone = usePhone();
  const [busyId, setBusyId] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [panel, setPanel] = useState<Panel | null>(openGoalId ? { kind: 'goal', id: openGoalId } : openPanel === 'new' ? { kind: 'new' } : openPanel === 'form' ? { kind: 'form', seed: '' } : null);
  const [detail, setDetail] = useState<GoalDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [drafting, setDrafting] = useState<{ bot: string; text: string; at: number; known: string[] } | null>(null);
  // Detail reads for list rows that don't carry their incident (D8): the strip can show the decision without the drawer.
  const [reads, setReads] = useState<Record<string, GoalDetail>>({});
  const reading = useRef<Set<string>>(new Set());
  const keys = useRef<Record<string, string>>({});
  // A double press sends once: state updates land after the second click's handler has run.
  const busyRef = useRef(false);
  const savingRef = useRef(false);
  const alive = useRef(true);
  const panelRef = useRef<Panel | null>(panel);
  panelRef.current = panel;
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  const reload = useCallback(() => {
    setReloading(true);
    api.list().then(
      (list) => {
        if (!alive.current) return;
        setGoals(list);
        setReads({});
        setUnsupported(false);
        setLoadError(null);
        setReloading(false);
      },
      (e) => {
        if (!alive.current) return;
        setReloading(false);
        // An older server: show nothing, as in 0.6.4.
        if (e instanceof GoalError && (e.status === 404 || e.status === 405)) setUnsupported(true);
        else setLoadError(errText(e, 'Couldn’t load your goals.'));
      },
    );
  }, [api]);

  const loadDetail = useCallback(
    (id: string) => {
      api.get(id).then(
        (d) => {
          if (!alive.current || panelRef.current?.kind !== 'goal' || panelRef.current.id !== id) return;
          setDetail(d);
          setDetailError(null);
          // The list catches up with what the drawer just read.
          setGoals((cur) => (cur ? cur.map((x) => (x.id === id ? { ...x, ...d.goal } : x)) : cur));
        },
        (e) => {
          if (!alive.current) return;
          if (e instanceof GoalError && e.status === 404) {
            setPanel(null);
            setGoals((cur) => (cur ? cur.filter((x) => x.id !== id) : cur));
            setNotice('That goal was deleted.');
          } else setDetailError(errText(e, 'Couldn’t load this goal.'));
        },
      );
    },
    [api],
  );

  useEffect(() => {
    if (!initial) reload();
  }, [initial, reload]);

  // D8: until the list rows carry `open_mission`, read the detail for up to 3 goals that may have a decision.
  useEffect(() => {
    if (!goals) return;
    const want = goals
      .filter((g) => g.mode !== 'watch' && ['not_met', 'fixing', 'needs_you'].includes(g.status) && !g.has_mission_info && !reads[g.id] && !reading.current.has(g.id))
      .slice(0, 3);
    for (const g of want) {
      reading.current.add(g.id);
      api.get(g.id).then(
        (d) => {
          reading.current.delete(g.id);
          if (alive.current) setReads((cur) => ({ ...cur, [g.id]: d }));
        },
        () => {
          // Leave it as the list says; the next list read tries again.
          reading.current.delete(g.id);
        },
      );
    }
  }, [goals, reads, api]);

  useEffect(() => {
    if (panel?.kind === 'goal') {
      setDetail(null);
      setDetailError(null);
      loadDetail(panel.id);
    }
  }, [panel, loadDetail]);

  // Live: a burst of changes (a check run updates the goal several times) reads once.
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | null = null;
    const ids = new Set<string>();
    const off = onMission('goal_updated', ({ id }) => {
      ids.add(id);
      if (t) clearTimeout(t);
      t = setTimeout(() => {
        reload();
        const p = panelRef.current;
        if (p?.kind === 'goal' && ids.has(p.id)) loadDetail(p.id);
        ids.clear();
      }, 250);
    });
    return () => {
      off();
      if (t) clearTimeout(t);
    };
  }, [reload, loadDetail]);

  // "Goal: <title>" on a mission, or "Open goal" after Keep this true.
  useEffect(() => {
    const onOpen = (e: Event) => {
      const id = (e as CustomEvent<{ id?: unknown }>).detail?.id;
      if (typeof id === 'string' && id) setPanel({ kind: 'goal', id });
    };
    window.addEventListener(OPEN_GOAL_EVENT, onOpen);
    return () => window.removeEventListener(OPEN_GOAL_EVENT, onOpen);
  }, []);

  // A requested draft: done when a draft this strip hadn't seen shows up, or after ten minutes.
  useEffect(() => {
    if (!drafting) return;
    if (goals?.some((g) => g.status === 'draft' && !drafting.known.includes(g.id))) {
      setDrafting(null);
      return;
    }
    const left = drafting.at + DRAFT_WAIT_MS - Date.now();
    if (left <= 0) {
      setDrafting(null);
      return;
    }
    const t = setTimeout(() => setDrafting(null), left);
    return () => clearTimeout(t);
  }, [drafting, goals]);

  const botList = useMemo(() => bots ?? Object.entries(names).map(([id, name]) => ({ id, name })), [bots, names]);
  const titles = useMemo(() => Object.fromEntries((goals ?? []).map((g) => [g.id, g.title])), [goals]);

  const upsert = (g: Goal) => setGoals((cur) => (cur ? (cur.some((x) => x.id === g.id) ? cur.map((x) => (x.id === g.id ? { ...x, ...g, checks: g.checks.length ? g.checks : x.checks } : x)) : [g, ...cur]) : [g]));

  const setHidden = (id: string, task: string | null) =>
    setNotNow((cur) => {
      const { [id]: _was, ...rest } = cur;
      const next = task ? { ...rest, [id]: task } : rest;
      writeJson(NOT_NOW_KEY, next);
      return next;
    });

  /**
   * Not now (D1): hidden at once on this device, and sent to the server so it holds everywhere and the
   * bot stops asking. A server without the route (404/405) or no connection keeps the local hide only.
   * Silent on every surface: no notice, no toast (G-Q-28).
   */
  const declineIncident = async (g: Goal) => {
    const task = heldFixTask(g);
    if (!task) return;
    setHidden(g.id, task);
    const key = (keys.current[`not_now:${g.id}`] ??= newIdempotencyKey());
    try {
      upsert(rawOf(await api.notNow(g.id, key)));
      delete keys.current[`not_now:${g.id}`];
    } catch (e) {
      if (e instanceof GoalError && (e.status === 404 || e.status === 405 || e.status === 0)) return;
      setHidden(g.id, null);
      setErrors((x) => ({ ...x, [g.id]: errText(e, 'Couldn’t save that. Try again.') }));
    }
  };

  const act = async (shown: Goal, a: GoalAction) => {
    const g = rawOf(shown);
    if (a === 'edit') {
      setFormError(null);
      // Edit needs the checks; a list without them gets the detail first.
      if (g.checks.length === 0) {
        try {
          const d = await api.get(g.id);
          if (!alive.current) return;
          setPanel({ kind: 'edit', goal: { ...d.goal, checks: d.checks.length ? d.checks : d.goal.checks } });
        } catch (e) {
          setErrors((x) => ({ ...x, [g.id]: errText(e, 'Couldn’t open it for editing.') }));
        }
      } else setPanel({ kind: 'edit', goal: g });
      return;
    }
    if (a === 'not_now') {
      void declineIncident(g);
      return;
    }
    if (busyRef.current) return;
    busyRef.current = true;
    setBusyId(g.id);
    setErrors(({ [g.id]: _gone, ...rest }) => rest);
    setNotice(null);
    // One key per press intent: a repeat after a lost answer replays the first one.
    const k = `${a}:${g.id}`;
    const key = (keys.current[k] ??= newIdempotencyKey());
    try {
      if (a === 'approve') {
        upsert(await api.approve(g.id, key));
        setNotice(`Approved: “${g.title}” is being checked now.`);
      } else if (a === 'check') {
        const after = await api.check(g.id, key);
        if (after) upsert(after);
        else upsert({ ...g, status: 'checking', checking_since: new Date().toISOString() });
      } else if (a === 'pause' || a === 'resume') {
        upsert(await api.update(g.id, { paused: a === 'pause' }));
      } else if (a === 'fix' || a === 'always') {
        let task = heldFixTask(g);
        let after: Goal = g;
        if (a === 'always') {
          try {
            // Applies to the open incident too (G-Q-12): the server may release the held fix itself, so look again.
            after = rawOf(await api.alwaysFix(g.id, key));
            task = heldFixTask((await api.get(g.id).catch(() => null))?.goal ?? after) ?? null;
          } catch (e) {
            if (!(e instanceof GoalError && (e.status === 404 || e.status === 405))) throw e;
            after = rawOf(await api.update(g.id, { mode: 'fix' }));
          }
        }
        if (task) {
          await api.runFix(task, key);
          upsert({
            ...g,
            ...after,
            mode: a === 'always' ? 'fix' : g.mode,
            status: 'fixing',
            open_mission: g.open_mission ? { ...g.open_mission, run_task_id: null } : null,
            open_fix_task: g.open_fix_task ? { ...g.open_fix_task, held: false } : null,
          });
        } else if (a === 'always') upsert(after);
        setHidden(g.id, null);
        setNotice(a === 'always' ? `“${g.title}” now gets fixed without asking. Fixing it now.` : `Fixing “${g.title}”. Its checks confirm the fix.`);
        reload();
      } else if (a === 'delete' || a === 'dismiss') {
        await api.remove(g.id);
        setGoals((cur) => (cur ? cur.filter((x) => x.id !== g.id) : cur));
        if (panelRef.current?.kind === 'goal' && panelRef.current.id === g.id) setPanel(null);
        setNotice(a === 'dismiss' ? `Dismissed “${g.title}”.` : `Deleted “${g.title}”.`);
      }
      delete keys.current[k];
      if (panelRef.current?.kind === 'goal' && panelRef.current.id === g.id && a !== 'delete' && a !== 'dismiss') loadDetail(g.id);
    } catch (e) {
      if (e instanceof GoalError && e.status === 404) {
        setGoals((cur) => (cur ? cur.filter((x) => x.id !== g.id) : cur));
        if (panelRef.current?.kind === 'goal' && panelRef.current.id === g.id) setPanel(null);
        setNotice('That goal was already deleted.');
        delete keys.current[k];
      } else {
        setErrors((x) => ({ ...x, [g.id]: errText(e, 'Couldn’t save that. Try again.') }));
        if (e instanceof GoalError && e.status === 409) reload();
      }
    } finally {
      busyRef.current = false;
      if (alive.current) setBusyId(null);
    }
  };

  const save = async (d: GoalDraft) => {
    if (savingRef.current || !panel) return;
    savingRef.current = true;
    setSaving(true);
    setFormError(null);
    const k = panel.kind === 'edit' ? `edit:${panel.goal.id}` : 'create';
    const key = (keys.current[k] ??= newIdempotencyKey());
    try {
      if (panel.kind === 'edit') {
        const g = await api.update(panel.goal.id, d);
        if (!alive.current) return;
        upsert(g);
        setNotice(`Saved “${g.title}”.`);
      } else {
        const g = await api.create(d, key);
        if (!alive.current) return;
        upsert(g);
        setNotice(`Created “${g.title}”. Checking it now.`);
      }
      delete keys.current[k];
      setPanel(null);
    } catch (e) {
      setFormError(errText(e, 'Couldn’t save the goal. Try again.'));
    } finally {
      savingRef.current = false;
      if (alive.current) setSaving(false);
    }
  };

  const requestDraft = async (text: string, kwantId: string) => {
    if (savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setFormError(null);
    const k = `draft:${kwantId}:${text}`;
    const key = (keys.current[k] ??= newIdempotencyKey());
    try {
      const g = await api.draft({ space_id: spaceId, kwant_id: kwantId, text }, key);
      if (!alive.current) return;
      saveBot(kwantId);
      delete keys.current[k];
      if (g) {
        upsert(g);
        setPanel(g.status === 'draft' ? { kind: 'goal', id: g.id } : null);
      } else {
        setDrafting({ bot: names[kwantId] ?? 'The bot', text, at: Date.now(), known: (goals ?? []).map((x) => x.id) });
        setPanel(null);
      }
    } catch (e) {
      setFormError(errText(e, 'Couldn’t ask for a draft. Try again.'));
    } finally {
      savingRef.current = false;
      if (alive.current) setSaving(false);
    }
  };

  const openTask = onOpenTask
    ? (id: string) => {
        setPanel(null);
        onOpenTask(id);
      }
    : undefined;
  const openMission = onOpenMission
    ? (id: string) => {
        setPanel(null);
        onOpenMission(id);
      }
    : undefined;

  // What each row draws: the detail read where the list has no incident info (D8), then the display status (D3, D6).
  const merged = (goals ?? []).map((g) => {
    const r = reads[g.id];
    return r ? { ...g, ...r.goal, checks: g.checks.length ? g.checks : r.goal.checks } : g;
  });
  const eventsOf = (id: string) => reads[id]?.events ?? (detail && detail.goal.id === id ? detail.events : []);
  const sorted = sortGoals(merged.map((g) => ({ ...g, raw_status: g.status, status: displayStatus(g, eventsOf(g.id), now, notNow[g.id] ?? null) })));
  const drafts = sorted.filter((g) => g.status === 'draft').length;
  const panelTitle =
    panel?.kind === 'new'
      ? 'New goal'
      : panel?.kind === 'form'
        ? 'New goal: add checks yourself'
        : panel?.kind === 'edit'
          ? `Edit: ${panel.goal.title}`
          : detail?.goal.title ?? sorted.find((g) => panel?.kind === 'goal' && g.id === panel.id)?.title ?? 'Goal';

  const drawer = panel && (
    <GoalPanel title={panelTitle} onClose={() => setPanel(null)}>
      {panel.kind === 'new' ? (
        <GoalDescribe
          bots={botList}
          drafts={drafts}
          busy={saving}
          error={formError}
          onDraft={(text, bot) => void requestDraft(text, bot)}
          onManual={(seed) => {
            setFormError(null);
            setPanel({ kind: 'form', seed });
          }}
          onCancel={() => setPanel(null)}
        />
      ) : panel.kind === 'form' || panel.kind === 'edit' ? (
        <GoalForm
          goal={panel.kind === 'edit' ? panel.goal : null}
          seed={panel.kind === 'form' ? panel.seed : ''}
          bots={botList}
          saving={saving}
          error={formError}
          onSave={(d) => void save(d)}
          onCancel={() => setPanel(null)}
        />
      ) : detail ? (
        <GoalDetailBody
          detail={detail}
          names={names}
          titles={titles}
          notNowTask={notNow[detail.goal.id] ?? null}
          now={now}
          busy={busyId === detail.goal.id}
          error={errors[detail.goal.id] ?? null}
          onAction={(a) => void act({ ...detail.goal, checks: detail.checks.length ? detail.checks : detail.goal.checks }, a)}
          onOpenTask={openTask}
          onOpenMission={openMission}
        />
      ) : detailError ? (
        <div role="alert" className="flex items-center gap-2 rounded-lg border border-rose-500/20 bg-rose-500/5 px-3 py-2 text-xs text-rose-300">
          <span className={`min-w-0 flex-1 ${wrap}`}>{detailError}</span>
          <button type="button" onClick={() => loadDetail(panel.id)} className={`${dangerBtn} shrink-0`}>
            Retry
          </button>
        </div>
      ) : (
        <p className="flex items-center gap-2 text-xs text-zinc-400">
          <Loader2 size={14} className={spin} aria-hidden="true" /> Loading…
        </p>
      )}
    </GoalPanel>
  );

  if (hidden) return drawer || null;
  // Loading, or an older server: the board looks as in 0.6.4. The drawer can still open from a chip.
  if (unsupported || (goals === null && !loadError)) return drawer || null;

  const newGoal = () => {
    setFormError(null);
    setPanel({ kind: 'new' });
  };
  const draftingLine = drafting && (
    <p role="status" className={`flex min-w-0 items-center gap-1.5 text-[11px] text-sky-200 ${wrap}`} data-goals-drafting>
      <Loader2 size={11} className={`shrink-0 ${spin}`} aria-hidden="true" />
      {drafting.bot} is drafting checks for “{drafting.text.length > 80 ? `${drafting.text.slice(0, 79)}…` : drafting.text}”. The draft shows here.
    </p>
  );

  // Empty: one line with a link, not a panel (B.0 rule 5), so the board's columns stay in view.
  if (goals !== null && sorted.length === 0)
    return (
      <section aria-labelledby={headId} className="flex flex-wrap items-center gap-x-2 gap-y-0.5 px-1 text-[11px]" data-goals-strip data-count={0}>
        <h3 id={headId} className="flex items-center gap-1.5 font-medium text-zinc-300">
          <Target size={13} className="text-sky-300" aria-hidden="true" /> Goals
        </h3>
        <p className="min-w-0 text-zinc-500" data-goals-empty>
          What should stay true?<span className="hidden sm:inline"> Say it in a sentence and a bot drafts the checks.</span>
        </p>
        <button type="button" onClick={newGoal} className={linkBtn} data-new-goal>
          New goal
        </button>
        {draftingLine && <div className="basis-full">{draftingLine}</div>}
        {notice && (
          <p role="status" className={`basis-full text-emerald-300 ${wrap}`} data-goals-notice>
            {notice}
          </p>
        )}
        {drawer}
      </section>
    );

  const attention = sorted.filter((g) => wantsLook(g.status));
  const visible = open ? sorted : attention;
  const cap = !open && phone ? PHONE_CAP : Infinity;
  const hiddenCount = sorted.length - Math.min(visible.length, cap);
  const decisionFor = (g: Goal) => {
    const d = goalDecision(g, eventsOf(g.id), now, notNow[g.id] ?? null);
    return d?.kind === 'run' || d?.kind === 'declined' ? d : null;
  };
  const toggle = () => {
    setOpen(!open);
    writeJson(OPEN_KEY, !open);
  };

  return (
    <section aria-labelledby={headId} className="rounded-xl border border-zinc-800 bg-zinc-900/40 px-3 py-1.5 space-y-2" data-goals-strip data-count={goals?.length ?? -1}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
        <h3 id={headId} className="flex items-center gap-1.5 text-xs font-medium text-zinc-200">
          <Target size={14} className="text-sky-300" aria-hidden="true" /> Goals
        </h3>
        {sorted.length > 0 && (
          <p className="order-last basis-full sm:order-none sm:basis-auto min-w-0 text-[11px] text-zinc-400" data-goal-counts>
            {stripSummary(sorted, now)}
          </p>
        )}
        {reloading && <Loader2 size={11} className={`${spin} text-zinc-500`} aria-label="Updating" />}
        <div className="ml-auto flex items-center gap-1">
          {(open || hiddenCount > 0) && (
            <button type="button" aria-expanded={open} aria-controls={listId} onClick={toggle} className={`${quietBtn} text-sky-300 hover:text-sky-200`} data-goals-toggle>
              {open ? 'Show less' : `Show all ${sorted.length}`}
              <ChevronDown size={12} className={open ? 'rotate-180' : ''} aria-hidden="true" />
            </button>
          )}
          <button type="button" onClick={newGoal} className={plainBtn} data-new-goal>
            <Plus size={12} aria-hidden="true" /> New goal
          </button>
        </div>
      </div>

      {loadError && (
        <p role="alert" className="flex flex-wrap items-center gap-2 text-[11px] text-rose-300" data-goals-error>
          <AlertTriangle size={12} aria-hidden="true" /> {loadError}
          <button type="button" onClick={reload} className={`underline hover:text-rose-200 rounded min-h-[40px] sm:min-h-0 ${focusRing}`}>
            Try again
          </button>
        </p>
      )}
      {draftingLine}
      {notice && (
        <p role="status" className={`text-[11px] text-emerald-300 ${wrap}`} data-goals-notice>
          {notice}
        </p>
      )}

      {visible.length > 0 && (
        <ul id={listId} className="grid gap-1.5 sm:grid-cols-2 xl:grid-cols-3" aria-label="Goals">
          {visible.map((g, i) => {
            const decision = decisionFor(g);
            return (
              <GoalRow
                key={g.id}
                goal={g}
                names={names}
                titles={titles}
                now={now}
                busy={busyId === g.id}
                error={errors[g.id] ?? null}
                decision={decision}
                hideOnPhone={i >= cap}
                onOpen={(x) => setPanel({ kind: 'goal', id: x.id })}
                onOpenTask={openTask}
                onAction={(a) => void act(g, a)}
              />
            );
          })}
        </ul>
      )}
      {drawer}
    </section>
  );
};
