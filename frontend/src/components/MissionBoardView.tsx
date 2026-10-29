import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  CheckCircle2,
  ChevronLeft,
  Circle,
  CircleDashed,
  Copy,
  ExternalLink,
  FileText,
  GitBranch,
  History,
  Hand,
  Image as ImageIcon,
  Link2,
  Loader2,
  MessageSquare,
  PauseCircle,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  StickyNote,
  Target,
  Wrench,
  X,
  XCircle,
} from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { playSfx } from '../audio/chiptune';
import {
  ACTION_DONE,
  ACTION_LABELS,
  AutofixAttempt,
  Checkpoint,
  NeedsYouItem,
  RESTORE_ACTION_LABELS,
  RestorePreview,
  RestoreResult,
  TaskFailure,
  WorkflowChange,
  autofixCardText,
  autofixStatusText,
  changeRows,
  checkpointLabel,
  conflictText,
  failureClassLabel,
  failureTone,
  autofixErrorText,
  fixTextShown,
  needsYouCount,
  recoveryText,
  restoreErrorText,
  skippedWithFixText,
  restoredText,
  workflowTargetLabel,
  COLUMNS,
  Check,
  Column,
  Evidence,
  GraphLevel,
  Insights,
  Mission,
  MissionError,
  OpenFlag,
  ResolveChoice,
  NEEDS_REASON,
  Receipt,
  Task,
  TaskAction,
  TaskDetail,
  Tone,
  WorkGraph,
  actionsFor,
  ago,
  buildReceipt,
  cardStatus,
  checkKindLabel,
  checksText,
  optionalChecksNote,
  approvalHolds,
  columnOfTask,
  flagText,
  flagErrorText,
  graphLevels,
  newIdempotencyKey,
  openFlags,
  resolveOptions,
  resolvedText,
  costRatio,
  costText,
  durationText,
  eventText,
  formatBytes,
  formatTokens,
  groupByColumn,
  missionStatusLabel,
  missionsApi,
  onMission,
  priorityLabel,
  safeHttps,
  sourceChat,
  answerChat,
  type ChatPlace,
  sourceLabel,
  stateLabel,
  untilText,
  upsertMission,
  upsertTask,
} from '../missions';
import { DEFAULT_CLASS, ENGINE_CANT_SANDBOX, TIER_HINTS, classLabel, engineCantSandboxText, showsTier, tierLabel, toClass, validClass } from '../flightSchool';

const TONES: Record<Tone, string> = {
  emerald: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  amber: 'border-amber-500/30 bg-amber-500/10 text-amber-200',
  violet: 'border-violet-500/30 bg-violet-500/10 text-violet-200',
  rose: 'border-rose-500/30 bg-rose-500/10 text-rose-200',
  sky: 'border-sky-500/30 bg-sky-500/10 text-sky-200',
  zinc: 'border-zinc-700 bg-zinc-800 text-zinc-300',
};

const actionBtn =
  'inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[32px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const quietBtn = `${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`;
/** A secondary action that shouldn't pull the eye: text only, same tap target. */
const subtleBtn = `${actionBtn} -ml-3 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/60 underline underline-offset-2 decoration-zinc-600`;
const mainBtn = `${actionBtn} bg-sky-600 text-white hover:bg-sky-500`;
const goodBtn = `${actionBtn} bg-emerald-600 text-white hover:bg-emerald-500`;
const dangerBtn = `${actionBtn} border border-rose-500/30 text-rose-200 hover:bg-rose-500/10`;
const card = 'rounded-xl border border-zinc-800 bg-zinc-900/40';
const field =
  'w-full min-h-[40px] sm:min-h-[36px] px-2.5 py-1.5 bg-zinc-950 border border-zinc-800 rounded-lg text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none focus:border-sky-500';
const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';

export const Badge: React.FC<{ tone: Tone; children: React.ReactNode; title?: string }> = ({ tone, children, title }) => (
  <span title={title} className={`inline-flex items-center gap-1 shrink-0 px-1.5 py-px rounded border text-[10px] font-semibold uppercase tracking-wide ${TONES[tone]}`}>
    {children}
  </span>
);

const Notice: React.FC<{ text: string; error?: boolean }> = ({ text, error = false }) => (
  <p role={error ? 'alert' : 'status'} className={`px-3 py-2 rounded-lg border text-xs ${error ? 'border-amber-500/30 bg-amber-500/10 text-amber-200' : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'}`}>
    {text}
  </p>
);

const SpendBar: React.FC<{ spent: number; budget: number; label: string }> = ({ spent, budget, label }) => {
  const r = costRatio(spent, budget);
  return (
    <div
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={budget || 1}
      aria-valuenow={Math.min(spent, budget || spent)}
      aria-valuetext={costText(spent, budget)}
      className="h-1.5 rounded-full bg-zinc-800 overflow-hidden"
    >
      <div className={`h-full rounded-full ${r >= 1 ? 'bg-rose-500' : r >= 0.8 ? 'bg-amber-500' : 'bg-sky-500'}`} style={{ width: `${Math.round(r * 100)}%` }} />
    </div>
  );
};

/** Two-letter initials for the owner chip. */
const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join('') || '?';

// ---------- Insights ----------

export const InsightsStrip: React.FC<{
  insights: Insights | null;
  /** The Needs you column's count across all work; the tile shows it instead of the server's count. */
  needsYou?: number;
}> = ({ insights, needsYou }) => {
  if (!insights) return null;
  const needsNow = needsYou ?? insights.needs_you_now;
  const blockedMin = insights.blocked_minutes_7d;
  const items: Array<{ label: string; value: string; hint: string; tone?: Tone; sub?: string }> = [
    { label: 'Verified this week', value: String(insights.verified_7d), hint: 'Tasks a check confirmed in the last 7 days', tone: 'emerald' },
    {
      label: 'False successes',
      value: String(insights.false_success_7d),
      hint: 'A bot said done, then the check failed (last 7 days)',
      tone: insights.false_success_7d > 0 ? 'rose' : undefined,
    },
    { label: 'Needs you now', value: String(needsNow), hint: 'Waiting on a question, approval or step from you', tone: needsNow > 0 ? 'amber' : undefined },
    typeof blockedMin === 'number'
      ? {
          label: 'Blocked this week',
          value: durationText(blockedMin),
          hint: 'Time tasks spent blocked in the last 7 days',
          tone: insights.blocked_now > 0 ? 'rose' : undefined,
          sub: insights.blocked_now > 0 ? `${insights.blocked_now} blocked now` : undefined,
        }
      : { label: 'Blocked now', value: String(insights.blocked_now), hint: 'Tasks that can’t move until something changes', tone: insights.blocked_now > 0 ? 'rose' : undefined },
    {
      label: 'Tokens per verified',
      value: insights.tokens_per_verified_7d === null ? '—' : formatTokens(insights.tokens_per_verified_7d),
      hint: `${formatTokens(insights.tokens_7d)} tokens used in the last 7 days`,
    },
  ];
  const af = insights.autofix_7d;
  if (af) {
    const byClass = Object.entries(insights.failures_by_class_7d ?? {})
      .filter(([, n]) => (n ?? 0) > 0)
      .sort((a, b) => (b[1] ?? 0) - (a[1] ?? 0))
      .map(([c, n]) => `${failureClassLabel(c)}: ${n}`);
    items.push({
      label: 'Fixed by AutoFix',
      value: af.attempts > 0 ? `${af.succeeded} of ${af.attempts}` : '0',
      hint: `Retries AutoFix ran in the last 7 days, and how many ended Verified.${byClass.length ? ` Failures: ${byClass.join(', ')}.` : ''}`,
      tone: af.succeeded > 0 ? 'emerald' : undefined,
      sub: af.attempts > 0 ? `${formatTokens(af.tokens)} tokens on retries` : 'No retries this week',
    });
  }
  const valueTone: Partial<Record<Tone, string>> = { emerald: 'text-emerald-300', rose: 'text-rose-300', amber: 'text-amber-300' };
  return (
    <section aria-label="This week" data-insights>
      <dl className={`grid grid-cols-2 sm:grid-cols-3 ${items.length > 5 ? 'lg:grid-cols-6' : 'lg:grid-cols-5'} gap-2`}>
        {items.map((it, i) => (
          <div key={it.label} title={it.hint} className={`${card} px-3 py-2 ${i === items.length - 1 && items.length % 2 === 1 ? 'col-span-2 sm:col-span-1' : ''}`}>
            <dt className="text-[11px] text-zinc-400 truncate">{it.label}</dt>
            <dd className={`text-lg font-semibold tabular-nums ${it.tone ? valueTone[it.tone] ?? 'text-zinc-100' : 'text-zinc-100'}`}>{it.value}</dd>
            {it.sub && <dd className="text-[10px] text-zinc-400 truncate">{it.sub}</dd>}
          </div>
        ))}
      </dl>
    </section>
  );
};

// ---------- Missions ----------

export const MissionSummary: React.FC<{
  mission: Mission;
  busy: boolean;
  onApprove: () => void;
  onCancel: () => void;
}> = ({ mission: m, busy, onApprove, onCancel }) => {
  const st = missionStatusLabel(m.status);
  const total = Object.entries(m.counts)
    .filter(([k]) => k !== 'cancelled')
    .reduce((n, [, v]) => n + v, 0);
  return (
    <section className={`${card} p-3 sm:p-4 space-y-2`} data-mission={m.id} aria-label={`Mission: ${m.title}`}>
      <div className="flex flex-wrap items-start gap-2">
        <Target size={16} className="mt-0.5 shrink-0 text-sky-400" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold text-zinc-100 break-words">{m.title}</h3>
          {m.goal && <p className="mt-0.5 text-xs text-zinc-400 break-words">{m.goal}</p>}
        </div>
        <Badge tone={st.tone}>{st.label}</Badge>
        {m.over_budget && <Badge tone="rose">Over budget</Badge>}
      </div>
      <div className="space-y-1">
        <div className="flex flex-wrap justify-between gap-x-3 text-[11px] text-zinc-400">
          <span>
            {m.counts.verified} of {total} task{total === 1 ? '' : 's'} verified
            {m.lead_name ? ` · Led by ${m.lead_name}` : ''}
          </span>
          <span className="tabular-nums">{costText(m.spent_tokens, m.budget_tokens)}</span>
        </div>
        <SpendBar spent={m.spent_tokens} budget={m.budget_tokens} label="Mission budget used" />
      </div>
      {m.over_budget && <p className="text-[11px] text-rose-300">No new task starts until you raise the budget.</p>}
      {m.status === 'draft' && (
        <div className="flex flex-wrap items-center gap-2 pt-1">
          <p className="text-xs text-amber-200 flex-1 min-w-[12rem]">{m.lead_name ?? 'A bot'} planned this. Nothing runs until you approve it.</p>
          <button type="button" disabled={busy} onClick={onApprove} className={mainBtn}>
            Approve mission
          </button>
          <button type="button" disabled={busy} onClick={onCancel} className={quietBtn}>
            Don’t run it
          </button>
        </div>
      )}
    </section>
  );
};

// ---------- Cards ----------

export const TaskCard: React.FC<{
  task: Task;
  names: Record<string, string>;
  now?: number;
  showMission?: boolean;
  onOpen: (t: Task) => void;
}> = ({ task: t, names, now = Date.now(), showMission = true, onOpen }) => {
  const owner = (t.owner_kwant_id && names[t.owner_kwant_id]) || t.owner_name;
  const checks = checksText(t.checks_summary);
  const optionalNote = optionalChecksNote(t.checks_summary);
  const status = cardStatus(t, names, now);
  const flags = openFlags(t);
  const dimmed = t.state === 'backlog' && !flags.length && !t.held_at;
  const autofixLive = !!autofixCardText(t, now);
  return (
    <li data-task={t.id} data-state={t.state}>
      <button
        type="button"
        onClick={() => onOpen(t)}
        className={`${card} w-full text-left p-3 space-y-2 hover:border-zinc-700 hover:bg-zinc-900/70 ${focusRing} ${dimmed ? 'opacity-60' : ''}`}
      >
        <span className="flex flex-wrap items-center gap-1.5">
          {t.state === 'blocked' && <Badge tone="rose">Blocked</Badge>}
          {t.state === 'failed' && <Badge tone="rose">Failed</Badge>}
          {t.state === 'backlog' && !t.held_at && <Badge tone="zinc">Waiting</Badge>}
          {t.held_at && (t.state === 'ready' || t.state === 'backlog') && (
            <span data-held-badge>
              <Badge tone="amber">On hold</Badge>
            </span>
          )}
          {autofixLive && (
            <span data-autofix-badge>
              <Badge tone="violet">AutoFix</Badge>
            </span>
          )}
          {t.state === 'review' && approvalHolds(t.checks_summary) && <Badge tone="amber">Needs you</Badge>}
          {flags.length > 0 && (
            <span data-flag-badge>
              <Badge tone="amber">May be invalid</Badge>
            </span>
          )}
          {t.state === 'verified' && flags.length > 0 && <Badge tone="emerald">Verified</Badge>}
          {t.priority >= 2 && <Badge tone={t.priority >= 3 ? 'rose' : 'amber'}>{priorityLabel(t.priority)}</Badge>}
          {showsTier(t) && t.owner_tier !== 'act' && (
            <span data-tier-badge={t.owner_tier} title={`Flight School: ${classLabel(t.task_class ?? DEFAULT_CLASS)} work at ${tierLabel(t.owner_tier)}`}>
              <Badge tone="sky">{tierLabel(t.owner_tier)}</Badge>
            </span>
          )}
          {showMission && t.mission_title && <span className="min-w-0 truncate text-[11px] text-zinc-500">{t.mission_title}</span>}
        </span>
        <span className="block text-sm font-medium text-zinc-100 break-words">{t.title}</span>
        {status && (
          <span className={`block text-[11px] break-words line-clamp-2 ${t.state === 'needs_you' || t.held_at || (t.state === 'review' && approvalHolds(t.checks_summary)) ? 'text-amber-200' : autofixLive ? 'text-violet-200' : t.state === 'blocked' || t.state === 'failed' ? 'text-rose-300' : 'text-zinc-400'}`}>
            {status}
          </span>
        )}
        {flags.length > 0 && (
          <span className="flex items-start gap-1 text-[11px] text-amber-200 break-words" data-flag-line>
            <AlertTriangle size={11} className="mt-0.5 shrink-0" aria-hidden="true" />
            <span className="line-clamp-2">
              {flags[0].text}
              {flags.length > 1 ? ` (+${flags.length - 1} more)` : ''}
            </span>
          </span>
        )}
        {t.forked_from && <span className="block text-[11px] text-zinc-500" data-forked-from>A copy, made to redo an earlier task</span>}
        <span className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-zinc-400">
          {owner ? (
            <span className="inline-flex items-center gap-1 min-w-0">
              <span aria-hidden="true" className="shrink-0 w-4 h-4 rounded-full bg-zinc-700 text-[8px] font-semibold text-zinc-100 grid place-items-center">
                {initials(owner)}
              </span>
              <span className="truncate">{owner}</span>
            </span>
          ) : (
            <span className="text-zinc-500">No owner yet</span>
          )}
          <span className="tabular-nums">{costText(t.spent_tokens, t.budget_tokens)}</span>
          {(t.state === 'review' || t.state === 'verified' || t.state === 'failed' || t.checks_summary.total > 0) && (
            <span className={`inline-flex items-center gap-1 ${checks.tone === 'emerald' ? 'text-emerald-300' : checks.tone === 'rose' ? 'text-rose-300' : checks.tone === 'amber' ? 'text-amber-200' : 'text-zinc-400'}`}>
              {checks.tone === 'emerald' ? <CheckCircle2 size={11} aria-hidden="true" /> : checks.tone === 'rose' ? <XCircle size={11} aria-hidden="true" /> : <CircleDashed size={11} aria-hidden="true" />}
              {checks.text}
            </span>
          )}
          {optionalNote && (
            <span className="text-zinc-500" data-optional-note>
              {optionalNote}
            </span>
          )}
        </span>
      </button>
    </li>
  );
};

// ---------- Board ----------

export const Board: React.FC<{
  tasks: Task[];
  names: Record<string, string>;
  column: Column;
  now?: number;
  showMission?: boolean;
  onColumn: (c: Column) => void;
  onOpen: (t: Task) => void;
}> = ({ tasks, names, column, now, showMission, onColumn, onOpen }) => {
  const grouped = useMemo(() => groupByColumn(tasks), [tasks]);
  const tabsId = useId();
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const onTabKey = (e: React.KeyboardEvent, i: number) => {
    const d = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const next = (i + d + COLUMNS.length) % COLUMNS.length;
    onColumn(COLUMNS[next].key);
    tabRefs.current[next]?.focus();
  };
  const list = (key: Column) => {
    const meta = COLUMNS.find((c) => c.key === key)!;
    const items = grouped[key];
    return items.length ? (
      <ul className="space-y-2">
        {items.map((t) => (
          <TaskCard key={t.id} task={t} names={names} now={now} showMission={showMission} onOpen={onOpen} />
        ))}
      </ul>
    ) : (
      <p className="px-3 py-6 rounded-xl border border-dashed border-zinc-800 text-center text-xs text-zinc-500" data-empty-column={key}>
        {meta.empty}
      </p>
    );
  };
  return (
    <>
      {/* Phone and tablet: one column at a time, picked from tabs with counts */}
      <div className="lg:hidden space-y-3" data-board="tabs">
        <div role="tablist" aria-label="Board columns" className="flex gap-1 overflow-x-auto -mx-1 px-1 pb-1 [scrollbar-width:none]">
          {COLUMNS.map((c, i) => {
            const on = c.key === column;
            const n = grouped[c.key].length;
            return (
              <button
                key={c.key}
                ref={(el) => (tabRefs.current[i] = el)}
                type="button"
                role="tab"
                id={`${tabsId}-${c.key}`}
                aria-selected={on}
                aria-controls={`${tabsId}-panel`}
                tabIndex={on ? 0 : -1}
                onKeyDown={(e) => onTabKey(e, i)}
                onClick={() => onColumn(c.key)}
                className={`shrink-0 inline-flex items-center gap-1.5 min-h-[40px] px-3 rounded-lg text-xs font-medium ${focusRing} ${
                  on ? 'bg-zinc-800 text-zinc-100 border border-zinc-700' : 'text-zinc-400 border border-transparent hover:text-zinc-200'
                }`}
              >
                {c.label}
                <span className={`min-w-[18px] px-1 rounded-full text-[10px] text-center tabular-nums ${c.key === 'needs_you' && n > 0 ? 'bg-amber-500 text-zinc-950' : 'bg-zinc-700/70 text-zinc-200'}`}>{n}</span>
              </button>
            );
          })}
        </div>
        <div role="tabpanel" id={`${tabsId}-panel`} aria-labelledby={`${tabsId}-${column}`}>
          <p className="mb-2 text-[11px] text-zinc-500">{COLUMNS.find((c) => c.key === column)!.hint}</p>
          {list(column)}
        </div>
      </div>
      {/* Desktop: all five columns side by side */}
      <div className="hidden lg:grid grid-cols-5 gap-3 items-start" data-board="columns">
        {COLUMNS.map((c) => (
          <section key={c.key} aria-labelledby={`${tabsId}-h-${c.key}`} className="min-w-0 space-y-2" data-column={c.key}>
            <header className="px-1">
              <h3 id={`${tabsId}-h-${c.key}`} className="flex items-center gap-2 text-xs font-semibold text-zinc-200">
                {c.label}
                <span className={`min-w-[18px] px-1 rounded-full text-[10px] text-center tabular-nums ${c.key === 'needs_you' && grouped[c.key].length > 0 ? 'bg-amber-500 text-zinc-950' : 'bg-zinc-800 text-zinc-300'}`}>
                  {grouped[c.key].length}
                </span>
              </h3>
              <p className="text-[11px] text-zinc-500 truncate" title={c.hint}>
                {c.hint}
              </p>
            </header>
            {list(c.key)}
          </section>
        ))}
      </div>
    </>
  );
};

// ---------- Evidence receipt ----------

const statusIcon = (tone: Tone) =>
  tone === 'emerald' ? (
    <CheckCircle2 size={15} className="text-emerald-400" aria-hidden="true" />
  ) : tone === 'rose' ? (
    <XCircle size={15} className="text-rose-400" aria-hidden="true" />
  ) : tone === 'violet' ? (
    <Loader2 size={15} className="text-violet-300 animate-spin motion-reduce:animate-none" aria-hidden="true" />
  ) : tone === 'amber' ? (
    <Hand size={15} className="text-amber-300" aria-hidden="true" />
  ) : (
    <Circle size={15} className="text-zinc-500" aria-hidden="true" />
  );

const evidenceIcon = (k: Evidence['kind']) =>
  k === 'url' ? <Link2 size={13} aria-hidden="true" /> : k === 'note' ? <StickyNote size={13} aria-hidden="true" /> : k === 'screenshot' ? <ImageIcon size={13} aria-hidden="true" /> : <FileText size={13} aria-hidden="true" />;

const EvidenceItem: React.FC<{ e: Evidence; names: Record<string, string> }> = ({ e, names }) => {
  const href = safeHttps(e.uri);
  const size = formatBytes(e.bytes);
  const by = e.captured_by === 'person' ? 'you' : e.captured_by === 'kwant' ? 'kwant' : names[e.captured_by] || 'a bot';
  return (
    <li className="flex items-start gap-2 text-xs" data-evidence={e.id}>
      <span className="mt-0.5 text-zinc-400">{evidenceIcon(e.kind)}</span>
      <div className="min-w-0 flex-1">
        {href ? (
          <a href={href} target="_blank" rel="noopener noreferrer" className={`inline-flex items-center gap-1 text-sky-300 hover:text-sky-200 underline-offset-2 hover:underline break-all ${focusRing} rounded`}>
            {e.label}
            <ExternalLink size={11} aria-hidden="true" />
          </a>
        ) : (
          <span className="text-zinc-200 break-words">{e.label}</span>
        )}
        {e.path && <code className="block mt-0.5 text-[11px] text-zinc-400 break-all">{e.path}</code>}
        {e.excerpt && <p className="mt-0.5 text-[11px] text-zinc-400 whitespace-pre-wrap break-words line-clamp-4">{e.excerpt}</p>}
        <p className="mt-0.5 text-[10px] text-zinc-500">
          {[`Added by ${by}`, ago(e.captured_at), size, e.sha256 ? `SHA-256 ${e.sha256.slice(0, 12)}…` : null].filter(Boolean).join(' · ')}
        </p>
      </div>
    </li>
  );
};

export const EvidenceReceipt: React.FC<{
  receipt: Receipt;
  names: Record<string, string>;
  busy?: boolean;
  onApproveCheck?: (c: Check) => void;
}> = ({ receipt, names, busy = false, onApproveCheck }) => (
  <section aria-labelledby="receipt-h" className="space-y-3" data-receipt>
    <h4 id="receipt-h" className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
      <ShieldCheck size={14} className="text-emerald-400" aria-hidden="true" /> Evidence receipt
    </h4>
    {receipt.verifiedBy && (
      <p className="px-3 py-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 text-xs text-emerald-200" data-verified-by>
        Verified: {checkKindLabel(receipt.verifiedBy.check.kind).toLowerCase()} passed
        {receipt.verifiedBy.by ? `, checked by ${receipt.verifiedBy.by}` : ''}
        {receipt.verifiedBy.check.finished_at ? ` ${ago(receipt.verifiedBy.check.finished_at)}` : ''}.
      </p>
    )}
    {receipt.waitingOnYou && (
      <p className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-200" data-waiting-on-you>
        Not verified yet. {receipt.waitingOnYou}. A check a bot adds runs and counts only after you allow it below.
      </p>
    )}
    {receipt.optionalWaiting && (
      <p className="px-3 py-2 rounded-lg border border-zinc-800 text-xs text-zinc-400" data-optional-waiting>
        {receipt.optionalWaiting}.{' '}
        {receipt.settledVerified ? 'The task is verified without it. You can still run it.' : 'It doesn’t hold the task: it can be verified without it.'}
      </p>
    )}
    {receipt.notChecked && (
      <div className="px-3 py-2 rounded-lg border border-dashed border-zinc-700 text-xs" data-not-checked>
        <p className="font-semibold text-zinc-200">Not checked</p>
        <p className="mt-0.5 text-zinc-400">
          No check has run on this task yet. It can’t be marked verified on a bot’s word: a check has to pass, a teammate has to review it with evidence, or you verify it yourself.
        </p>
      </div>
    )}
    {receipt.lines.length > 0 && (
      <ol className="divide-y divide-zinc-800 rounded-xl border border-zinc-800">
        {receipt.lines.map((l) => (
          <li key={l.check.id} className="p-3 space-y-1.5" data-check={l.check.id} data-check-status={l.check.status}>
            <div className="flex items-start gap-2">
              <span className="mt-px">{statusIcon(receipt.quietOptional(l.check) ? 'zinc' : l.status.tone)}</span>
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-center gap-1.5 text-xs">
                  <span className="font-medium text-zinc-100">{checkKindLabel(l.check.kind)}</span>
                  {receipt.quietOptional(l.check) ? (
                    <Badge tone="zinc">Not run</Badge>
                  ) : (
                    <Badge tone={l.status.tone}>{l.status.label}</Badge>
                  )}
                  {!l.check.required && <span className="text-[10px] text-zinc-500">Optional</span>}
                </p>
                {l.target && <code className="block mt-1 text-[11px] text-zinc-300 break-all whitespace-pre-wrap">{l.target}</code>}
                <p className="mt-1 text-[10px] text-zinc-500">
                  {[l.by ? `Checked by ${l.by}` : null, l.check.finished_at ? ago(l.check.finished_at) : null].filter(Boolean).join(' · ') || 'Hasn’t run yet'}
                </p>
                {l.notCounting && !l.ownClaim && !receipt.quietOptional(l.check) && !(receipt.settledVerified && !l.check.required) && (
                  <p className="mt-1 text-[11px] text-amber-200" data-not-counting>
                    {l.notCounting}
                  </p>
                )}
                {l.ownClaim && <p className="mt-1 text-[11px] text-amber-200">This is the owner’s own check, so it doesn’t count toward Verified.</p>}
              </div>
            </div>
            {onApproveCheck && receipt.quietOptional(l.check) ? (
              <div className="pl-6">
                <button type="button" disabled={busy} onClick={() => onApproveCheck(l.check)} className={subtleBtn} data-run-optional>
                  Run this optional check
                </button>
              </div>
            ) : l.check.status === 'needs_approval' && onApproveCheck && (
              <div className="flex flex-wrap items-center gap-2 pl-6">
                <p className="text-[11px] text-amber-200 flex-1 min-w-[10rem]">{l.notCounting ? '' : 'A bot added this check. It runs only after you OK it.'}</p>
                <button type="button" disabled={busy} onClick={() => onApproveCheck(l.check)} className={quietBtn}>
                  Allow this check
                </button>
              </div>
            )}
            {(l.check.output_excerpt || l.output?.excerpt) && (
              <details className="pl-6 group">
                <summary className={`cursor-pointer text-[11px] text-zinc-400 hover:text-zinc-200 rounded ${focusRing}`}>Show output</summary>
                <pre className="mt-1 max-h-48 overflow-auto p-2 rounded-lg bg-zinc-950 border border-zinc-800 text-[11px] text-zinc-300 whitespace-pre-wrap break-words">
                  {l.check.output_excerpt || l.output?.excerpt}
                </pre>
              </details>
            )}
          </li>
        ))}
      </ol>
    )}
    <div>
      <h5 className="text-[11px] font-medium text-zinc-400 mb-1.5">Attached by the team</h5>
      {receipt.attachments.length ? (
        <ul className="space-y-2">
          {receipt.attachments.map((e) => (
            <EvidenceItem key={e.id} e={e} names={names} />
          ))}
        </ul>
      ) : (
        <p className="text-[11px] text-zinc-500">Nothing attached.</p>
      )}
      <p className="mt-2 text-[10px] text-zinc-500">Attached files and links are evidence to look at. They never mark a task verified on their own.</p>
    </div>
  </section>
);

// ---------- May be invalid ----------

/** One open flag with the person's three choices. Choices that would be refused are off, with why. */
export const FlagBox: React.FC<{
  flag: OpenFlag;
  state: Task['state'];
  causeTitle?: string | null;
  busy?: boolean;
  onResolve?: (f: OpenFlag, choice: ResolveChoice) => void;
  onOpenTask?: (id: string) => void;
}> = ({ flag, state, causeTitle, busy = false, onResolve, onOpenTask }) => {
  const opts = resolveOptions(state);
  const hintId = useId();
  const off = opts.filter((o) => o.disabled);
  return (
    <li className="px-3 py-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100 space-y-2" data-flag={flag.id} data-flag-cause={flag.cause ?? ''}>
      <p className="flex items-start gap-1.5 break-words">
        <AlertTriangle size={14} className="mt-px shrink-0 text-amber-300" aria-hidden="true" />
        <span>
          <span className="font-semibold">May be invalid: </span>
          {flag.text}.{' '}
          {state === 'verified' ? 'It stays Verified until you decide.' : 'Nothing has changed on the task until you decide.'}
          {flag.causeTaskId && flag.cause !== 'evidence_changed' && onOpenTask && (
            <>
              {' '}
              <button type="button" onClick={() => onOpenTask(flag.causeTaskId!)} className={`text-sky-300 hover:underline rounded ${focusRing}`} data-flag-cause-link>
                Open {causeTitle ? `“${causeTitle}”` : 'that task'}
              </button>
            </>
          )}
        </span>
      </p>
      {onResolve && (
        <>
          <div className="flex flex-wrap gap-2" role="group" aria-label="Decide on this flag" aria-describedby={hintId}>
            {opts.map((o) => (
              <button
                key={o.choice}
                type="button"
                disabled={busy || !!o.disabled}
                title={o.disabled ?? o.hint}
                onClick={() => onResolve(flag, o.choice)}
                className={o.choice === 'still_valid' ? goodBtn : o.choice === 'redo' ? mainBtn : dangerBtn}
                data-resolve={o.choice}
              >
                {o.choice === 'redo' && state === 'verified' && <Copy size={13} aria-hidden="true" />}
                {o.label}
              </button>
            ))}
          </div>
          <ul id={hintId} className="space-y-0.5 text-[11px] text-amber-100/80">
            {opts
              .filter((o) => !o.disabled && o.hint)
              .map((o) => (
                <li key={o.choice} data-resolve-hint={o.choice}>
                  <span className="font-medium">{o.label}:</span> {o.hint}
                </li>
              ))}
            {off.map((o) => (
              <li key={o.choice} className="text-zinc-400" data-resolve-off={o.choice}>
                {o.disabled}
              </li>
            ))}
          </ul>
        </>
      )}
    </li>
  );
};

// ---------- AutoFix, workflow changes and restore holds ----------

/** A task a restore put on hold: nothing picks it up until the person presses Run (D10). */
export const HeldBox: React.FC<{ task: Task; busy?: boolean; now?: number; onRun?: () => void }> = ({ task, busy = false, now = Date.now(), onRun }) => (
  <section aria-labelledby="held-h" className="px-3 py-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100 space-y-2" data-held>
    <h4 id="held-h" className="flex items-center gap-1.5 font-semibold">
      <PauseCircle size={14} className="shrink-0 text-amber-300" aria-hidden="true" />
      On hold after restore
    </h4>
    <p>
      You restored its files {task.held_at ? ago(task.held_at, now) : ''}. No bot picks it up until you press Run, so you can look at the files first.
    </p>
    {onRun && (
      <button type="button" disabled={busy} onClick={onRun} className={mainBtn} data-run-held>
        <Play size={13} aria-hidden="true" /> Run
      </button>
    )}
  </section>
);

/** Why the task failed last, what AutoFix is doing about it, and what retries cost. */
export const FailureBox: React.FC<{
  task: Task;
  failures?: TaskFailure[];
  /** The "fix it yourself" text for a failure AutoFix never retries. */
  fixText?: string | null;
  busy?: boolean;
  now?: number;
  onRetry?: (failureId?: string) => void;
  onSkip?: (a: AutofixAttempt) => void;
}> = ({ task, failures = [], fixText, busy = false, now = Date.now(), onRetry, onSkip }) => {
  const f = task.last_failure;
  const a = task.autofix ?? null;
  const recovery = recoveryText(task);
  if (!f && !a) return null;
  const tone = f ? failureTone(f.class) : 'violet';
  const fix = fixText ? fixTextShown(fixText, f?.summary, f ? failureClassLabel(f.class) : null) : null;
  // With a "To fix", a skipped retry gets one short line (or none), not the reason again.
  const folded = fix && a ? skippedWithFixText(a) : undefined;
  const status = folded !== undefined ? folded : a ? autofixStatusText(a, now) : null;
  const waiting = a && a.status === 'suggested' && a.strategy !== 'propose_change' && (task.state === 'failed' || task.state === 'blocked');
  const scheduled = a && a.status === 'scheduled';
  const earlier = failures.slice(0, -1).reverse();
  const latest = failures[failures.length - 1];
  return (
    <section aria-labelledby="fail-h" className="space-y-2" data-failure>
      <h4 id="fail-h" className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
        <Wrench size={13} className="text-zinc-400" aria-hidden="true" />
        What went wrong
      </h4>
      <div className={`${card} px-3 py-2.5 space-y-2 text-xs`}>
        {f && (
          <p className="flex flex-wrap items-start gap-1.5 break-words">
            <span data-failure-class={f.class}>
              <Badge tone={tone}>{failureClassLabel(f.class)}</Badge>
            </span>
            <span className="flex-1 min-w-[10rem] text-zinc-200">{f.summary}</span>
            <span className="text-[11px] text-zinc-500">{ago(f.created_at, now)}</span>
          </p>
        )}
        {fix && (
          <p className="px-2.5 py-2 rounded-md border border-amber-500/30 bg-amber-500/10 text-amber-100" data-fix-needed>
            <span className="font-medium">To fix: </span>
            {fix}
          </p>
        )}
        {latest?.excerpt && (
          <details className="text-[11px]">
            <summary className={`cursor-pointer text-zinc-400 hover:text-zinc-200 rounded ${focusRing}`}>Show the output</summary>
            <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-zinc-950 p-2 text-zinc-300">{latest.excerpt}</pre>
          </details>
        )}
        {status && (
          <p className="flex items-start gap-1.5 text-violet-200" data-autofix-status={a!.status}>
            <RefreshCw size={13} className="mt-px shrink-0" aria-hidden="true" />
            <span>{status}</span>
          </p>
        )}
        {(waiting || scheduled) && (onRetry || onSkip) && (
          <div className="flex flex-wrap gap-2">
            {onRetry && (
              <button type="button" disabled={busy} onClick={() => onRetry(latest?.id)} className={mainBtn} data-autofix-retry>
                <RefreshCw size={13} aria-hidden="true" /> Retry now
              </button>
            )}
            {onSkip && (
              <button type="button" disabled={busy} onClick={() => onSkip(a!)} className={quietBtn} data-autofix-skip>
                Don’t retry
              </button>
            )}
          </div>
        )}
        <dl className="grid grid-cols-2 gap-3 pt-1">
          <Fact label="AutoFix tries">
            {task.autofix_attempts ?? 0}
          </Fact>
          {recovery && <Fact label="Spent on retries">{recovery}</Fact>}
        </dl>
        {earlier.length > 0 && (
          <details className="text-[11px]" data-earlier-failures>
            <summary className={`cursor-pointer text-zinc-400 hover:text-zinc-200 rounded ${focusRing}`}>Earlier failures ({earlier.length})</summary>
            <ul className="mt-1 space-y-1.5">
              {earlier.map((x) => (
                <li key={x.id} className="break-words text-zinc-400">
                  <span className="text-zinc-300">Try {x.attempt}:</span> {failureClassLabel(x.class)}. {x.summary}
                  {x.autofix && <span className="block text-zinc-500">{autofixStatusText(x.autofix, now)}</span>}
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </section>
  );
};

/** A change AutoFix proposes to the task's setup, before and after; only the person approves it. */
export const WorkflowChangeBox: React.FC<{
  item: NeedsYouItem;
  change?: WorkflowChange | null;
  names?: Record<string, string>;
  busy?: boolean;
  now?: number;
  onDecide?: (id: string, approve: boolean) => void;
}> = ({ item, change, names = {}, busy = false, now = Date.now(), onDecide }) => {
  const rows = change ? changeRows(change.before, change.after) : [];
  const changed = rows.filter((r) => r.changed);
  const same = rows.filter((r) => !r.changed);
  const by = change ? (change.proposed_by === 'kwant' ? 'AutoFix' : change.proposed_by_name || names[change.proposed_by] || 'A bot') : null;
  const Cell: React.FC<{ label: string; value: string; tone: 'before' | 'after' }> = ({ label, value, tone }) => (
    <div className={`min-w-0 rounded-md px-2 py-1.5 ${tone === 'before' ? 'bg-zinc-950/60 text-zinc-400' : 'bg-emerald-500/10 text-emerald-100'}`}>
      <p className="text-[10px] uppercase tracking-wide opacity-70">{label}</p>
      <p className="whitespace-pre-wrap break-words font-mono text-[11px]">{value}</p>
    </div>
  );
  return (
    <li className="px-3 py-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100 space-y-2" data-workflow-change={item.id}>
      <p className="break-words">
        <span className="font-semibold">Proposed change{change ? ` to ${workflowTargetLabel(change.target).toLowerCase()}` : ''}: </span>
        {change ? change.reason : item.text}
      </p>
      {change && by && (
        <p className="text-[11px] text-amber-100/70">
          {by} proposed it {ago(change.created_at, now)}.
        </p>
      )}
      {changed.length > 0 && (
        <ul className="space-y-2" aria-label="What changes">
          {changed.map((r) => (
            <li key={r.key} className="space-y-1" data-change-row={r.key}>
              <p className="text-[11px] font-medium text-amber-100">{r.label}</p>
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-1.5">
                <Cell label="Now" value={r.before} tone="before" />
                <Cell label="After" value={r.after} tone="after" />
              </div>
            </li>
          ))}
        </ul>
      )}
      {same.length > 0 && (
        <details className="text-[11px]">
          <summary className={`cursor-pointer text-amber-100/70 hover:text-amber-100 rounded ${focusRing}`}>Unchanged ({same.length})</summary>
          <dl className="mt-1 space-y-1">
            {same.map((r) => (
              <div key={r.key} className="min-w-0">
                <dt className="text-amber-100/70">{r.label}</dt>
                <dd className="whitespace-pre-wrap break-words font-mono text-zinc-300">{r.before}</dd>
              </div>
            ))}
          </dl>
        </details>
      )}
      {!change && <p className="text-[11px] text-amber-100/70">Loading the details…</p>}
      {onDecide && (
        <>
          <div className="flex flex-wrap gap-2" role="group" aria-label="Decide on this change">
            <button type="button" disabled={busy || !change} onClick={() => onDecide(item.id, true)} className={goodBtn} data-change-approve>
              <CheckCircle2 size={13} aria-hidden="true" /> Approve and apply
            </button>
            <button type="button" disabled={busy || !change} onClick={() => onDecide(item.id, false)} className={dangerBtn} data-change-reject>
              Reject
            </button>
          </div>
          <p className="text-[11px] text-amber-100/70">Nothing changes until you approve.</p>
        </>
      )}
    </li>
  );
};

/** A restore preview with the person's choices: which conflicting files to overwrite, and run again or not. */
export const RestorePreviewForm: React.FC<{
  preview: RestorePreview;
  checkpoint?: Checkpoint | null;
  titleOf?: (id: string) => string | undefined;
  busy?: boolean;
  onRestore?: (opts: { force_paths: string[]; rerun: boolean }) => void;
  onCancel?: () => void;
}> = ({ preview, checkpoint, titleOf, busy = false, onRestore, onCancel }) => {
  const [force, setForce] = useState<string[]>([]);
  useEffect(() => setForce([]), [preview.preview_sha256]);
  const toggle = (p: string) => setForce((xs) => (xs.includes(p) ? xs.filter((x) => x !== p) : [...xs, p]));
  const nothing = preview.changes.length === 0 && force.length === 0;
  return (
    <div className="space-y-3 text-xs" data-restore-preview>
      {checkpoint && (
        <p className="text-zinc-400">
          Back to: <span className="text-zinc-200">{checkpointLabel(checkpoint.kind)}</span>, try {checkpoint.attempt}
        </p>
      )}
      {preview.refused && (
        <p className="px-3 py-2 rounded-lg border border-rose-500/30 bg-rose-500/10 text-rose-200" data-restore-refused={preview.refused}>
          {preview.refused === 'running' ? 'A bot is working on it now. Restore once it stops.' : 'A verified task isn’t restored. Use “Redo as a copy” instead.'}
        </p>
      )}
      <section aria-label="Files that change" className="space-y-1">
        <h5 className="text-[11px] font-semibold text-zinc-300">Files that change ({preview.changes.length})</h5>
        {preview.changes.length ? (
          <ul className="space-y-0.5 max-h-40 overflow-auto">
            {preview.changes.map((c) => (
              <li key={c.path} className="flex gap-2 min-w-0" data-restore-change={c.action}>
                <span className="shrink-0 w-20 text-zinc-500">{RESTORE_ACTION_LABELS[c.action]}</span>
                <span className="min-w-0 break-all font-mono text-zinc-200">{c.path}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-zinc-500">No files need changing.</p>
        )}
      </section>
      {preview.conflicts.length > 0 && (
        <fieldset className="space-y-1.5" data-restore-conflicts>
          <legend className="text-[11px] font-semibold text-amber-200">Changed since then ({preview.conflicts.length})</legend>
          <p className="text-[11px] text-zinc-400">These are left alone unless you tick them.</p>
          <ul className="space-y-1">
            {preview.conflicts.map((c) => (
              <li key={c.path}>
                <label className="flex items-start gap-2 min-h-[32px] cursor-pointer">
                  <input type="checkbox" className="mt-0.5 shrink-0 accent-amber-500" checked={force.includes(c.path)} onChange={() => toggle(c.path)} data-force={c.path} />
                  <span className="min-w-0">
                    <span className="block break-all font-mono text-zinc-200">{c.path}</span>
                    <span className="block text-[11px] text-amber-200/80" data-conflict-by>
                      {conflictText(c, titleOf)}. Tick to put it back anyway.
                    </span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </fieldset>
      )}
      {preview.skipped.length > 0 && (
        <details className="text-[11px]">
          <summary className={`cursor-pointer text-zinc-400 hover:text-zinc-200 rounded ${focusRing}`}>Not restored ({preview.skipped.length})</summary>
          <ul className="mt-1 space-y-0.5">
            {preview.skipped.map((s) => (
              <li key={s.path} className="break-all text-zinc-400">
                <span className="font-mono text-zinc-300">{s.path}</span>: {s.why}
              </li>
            ))}
          </ul>
        </details>
      )}
      {(preview.external_not_undone.length > 0 || !preview.effects_known) && (
        <section className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-amber-100 space-y-1" data-not-undone>
          <p className="font-medium">{preview.not_undone_note}</p>
          {preview.external_not_undone.length > 0 && (
            <ul className="space-y-0.5 text-[11px]">
              {preview.external_not_undone.map((x, i) => (
                <li key={`${x.at}-${i}`} className="break-words">
                  {x.summary}
                </li>
              ))}
            </ul>
          )}
          {!preview.effects_known && <p className="text-[11px]" data-effects-unknown>Side effects unknown: a run changed files without a logged tool call, so something else may have happened too.</p>}
        </section>
      )}
      {preview.task_state_after && (
        <p className="text-[11px] text-zinc-400">
          The task goes back to {stateLabel(preview.task_state_after).label}. With “Restore”, it stays on hold until you press Run.
        </p>
      )}
      {onRestore && !preview.refused && (
        <div className="flex flex-wrap gap-2">
          <button type="button" disabled={busy || nothing} onClick={() => onRestore({ force_paths: force, rerun: false })} className={mainBtn} data-restore>
            <RotateCcw size={13} aria-hidden="true" /> Restore
          </button>
          <button type="button" disabled={busy || nothing} onClick={() => onRestore({ force_paths: force, rerun: true })} className={quietBtn} data-restore-rerun>
            Restore and run again
          </button>
          {onCancel && (
            <button type="button" onClick={onCancel} className={quietBtn}>
              Never mind
            </button>
          )}
        </div>
      )}
    </div>
  );
};

type RestoreApi = Pick<typeof missionsApi, 'checkpoints' | 'restorePreview' | 'restore'>;

/** Time Machine for one task: pick a checkpoint, look at the preview, restore. */
export const RestoreSection: React.FC<{
  task: Task;
  api?: RestoreApi;
  titleOf?: (id: string) => string | undefined;
  now?: number;
  onRestored?: (r: RestoreResult, rerun: boolean) => void;
  onError?: (e: unknown) => void;
}> = ({ task, api = missionsApi, titleOf, now = Date.now(), onRestored, onError }) => {
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<Checkpoint[] | null>(null);
  const [picked, setPicked] = useState<Checkpoint | null>(null);
  const [preview, setPreview] = useState<RestorePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [undoId, setUndoId] = useState<string | null>(null);
  const keys = useRef<Record<string, string>>({});
  const inFlight = useRef(false);

  useEffect(() => {
    setOpen(false);
    setList(null);
    setPicked(null);
    setPreview(null);
    setErr(null);
    setUndoId(null);
  }, [task.id]);

  const load = async () => {
    setErr(null);
    try {
      setList(await api.checkpoints(task.id));
    } catch (e) {
      setErr(restoreErrorText(e));
      onError?.(e);
    }
  };
  const pick = async (c: Checkpoint) => {
    setPicked(c);
    setPreview(null);
    setErr(null);
    try {
      setPreview(await api.restorePreview(task.id, c.id));
    } catch (e) {
      setErr(restoreErrorText(e));
      onError?.(e);
    }
  };
  const restore = async (opts: { force_paths: string[]; rerun: boolean }) => {
    if (!picked || !preview || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setErr(null);
    const k = `${picked.id}|${preview.preview_sha256}|${[...opts.force_paths].sort().join(',')}|${opts.rerun}`;
    keys.current[k] ??= newIdempotencyKey();
    try {
      const r = await api.restore(task.id, picked.id, { preview_sha256: preview.preview_sha256, force_paths: opts.force_paths, rerun: opts.rerun }, keys.current[k]);
      setUndoId(r.before_restore_checkpoint_id);
      setPicked(null);
      setPreview(null);
      setList(null);
      onRestored?.(r, opts.rerun);
    } catch (e) {
      setErr(restoreErrorText(e));
      if (e instanceof MissionError && e.code === 'stale_preview') void pick(picked);
      onError?.(e);
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  const undo = async () => {
    if (!undoId) return;
    const cps = list ?? (await api.checkpoints(task.id).catch(() => [] as Checkpoint[]));
    setList(cps);
    setOpen(true);
    const c = cps.find((x) => x.id === undoId);
    if (c) void pick(c);
    else setErr('The checkpoint taken before the restore isn’t there any more.');
  };

  if (!task.checkpoint_count) return null;
  const blocked = task.state === 'verified' || task.state === 'cancelled';

  return (
    <section aria-labelledby="tm-h" className="space-y-2" data-restore-section>
      <h4 id="tm-h" className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
        <History size={13} className="text-zinc-400" aria-hidden="true" />
        Checkpoints
      </h4>
      {err && <Notice text={err} error />}
      {undoId && !picked && (
        <button type="button" onClick={() => void undo()} className={quietBtn} data-restore-undo>
          <RotateCcw size={13} aria-hidden="true" /> Undo this restore
        </button>
      )}
      {blocked ? (
        <p className="text-[11px] text-zinc-500">
          {task.state === 'verified' ? 'A verified task isn’t restored. Use “Redo as a copy” to work on it again.' : 'A cancelled task isn’t restored.'}
        </p>
      ) : task.state === 'working' ? (
        <p className="text-[11px] text-zinc-500">A bot is working on it now. You can restore once it stops.</p>
      ) : !open ? (
        <button
          type="button"
          onClick={() => {
            setOpen(true);
            void load();
          }}
          className={quietBtn}
          data-restore-open
        >
          <History size={13} aria-hidden="true" /> Restore files from a checkpoint ({task.checkpoint_count})
        </button>
      ) : picked ? (
        preview ? (
          <RestorePreviewForm
            preview={preview}
            checkpoint={picked}
            titleOf={titleOf}
            busy={busy}
            onRestore={(o) => void restore(o)}
            onCancel={() => {
              setPicked(null);
              setPreview(null);
            }}
          />
        ) : (
          <p className="flex items-center gap-1.5 text-[11px] text-zinc-400">
            <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Working out what would change…
          </p>
        )
      ) : list === null ? (
        <p className="flex items-center gap-1.5 text-[11px] text-zinc-400">
          <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading checkpoints…
        </p>
      ) : (
        <ul className="space-y-1" data-checkpoints>
          {[...list].reverse().map((c) => (
            <li key={c.id}>
              <button
                type="button"
                disabled={!c.restorable}
                onClick={() => void pick(c)}
                title={c.error ?? undefined}
                className={`w-full text-left px-3 py-2 min-h-[44px] rounded-lg border border-zinc-800 bg-zinc-900/40 hover:border-zinc-600 disabled:opacity-50 disabled:hover:border-zinc-800 ${focusRing}`}
                data-checkpoint={c.id}
              >
                <span className="block text-xs text-zinc-200">
                  {checkpointLabel(c.kind)} · try {c.attempt}
                </span>
                <span className="block text-[11px] text-zinc-500">
                  {ago(c.created_at, now)} · {c.files} {c.files === 1 ? 'file' : 'files'}
                  {c.effects_after.external.length > 0 && ` · ${c.effects_after.external.length} outside actions after it`}
                  {!c.restorable && ' · can’t be restored'}
                </span>
              </button>
            </li>
          ))}
          <li>
            <button type="button" onClick={() => setOpen(false)} className={subtleBtn}>
              Close
            </button>
          </li>
        </ul>
      )}
    </section>
  );
};

// ---------- Task detail ----------

/** Needs-you kinds shown in their own section, not under "Waiting for you". */
const OWN_SECTION = new Set<NeedsYouItem['kind']>(['may_be_invalid', 'autofix_suggested', 'fix_needed', 'workflow_change']);

const Fact: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className="min-w-0">
    <dt className="text-[11px] text-zinc-500">{label}</dt>
    <dd className="text-xs text-zinc-200 break-words">{children}</dd>
  </div>
);

/** The task's kind of work (Flight School), which the person can change; its bot's tier for it follows. */
const TaskClassEditor: React.FC<{ task: Task; disabled: boolean; onSetClass?: (c: string) => void }> = ({ task, disabled, onSetClass }) => {
  const current = task.task_class ?? DEFAULT_CLASS;
  const [value, setValue] = useState(current);
  const id = useId();
  useEffect(() => setValue(current), [current]);
  const slug = toClass(value);
  if (!onSetClass || task.state === 'verified' || task.state === 'cancelled') return <span data-task-class={current}>{classLabel(current)}</span>;
  return (
    <form
      className="flex items-center gap-1.5"
      onSubmit={(e) => {
        e.preventDefault();
        if (validClass(slug) && slug !== current) onSetClass(slug);
      }}
    >
      <label htmlFor={id} className="sr-only">
        Kind of work
      </label>
      <input
        id={id}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        data-task-class={current}
        className="min-w-0 flex-1 bg-zinc-950 border border-zinc-800 rounded px-2 py-1 min-h-[32px] text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
      />
      {slug !== current && (
        <button type="submit" disabled={disabled || !validClass(slug)} className="px-2 py-1 min-h-[32px] rounded border border-zinc-700 text-[11px] hover:bg-zinc-800 disabled:opacity-50">
          Save
        </button>
      )}
    </form>
  );
};

export const TaskDetailBody: React.FC<{
  detail: TaskDetail;
  names: Record<string, string>;
  tasksById?: Record<string, Task>;
  now?: number;
  busy?: boolean;
  notice?: { text: string; error: boolean } | null;
  onAction: (a: TaskAction, reason?: string) => void;
  onApproveCheck: (c: Check) => void;
  onOpenChat?: (t: Task, place?: ChatPlace | null) => void;
  onOpenTask?: (id: string) => void;
  onResolveFlag?: (f: OpenFlag, choice: ResolveChoice) => void;
  /** Copy a verified task to redo it, with the reason typed. */
  onFork?: (reason: string) => void;
  /** GET /api/tasks/:id/failures, oldest first. */
  failures?: TaskFailure[];
  /** Proposed workflow changes for this task. */
  changes?: WorkflowChange[];
  onRunHeld?: () => void;
  onRetryAutofix?: (failureId?: string) => void;
  onSkipAutofix?: (a: AutofixAttempt) => void;
  onDecideChange?: (id: string, approve: boolean) => void;
  restoreApi?: RestoreApi;
  onRestored?: (r: RestoreResult, rerun: boolean) => void;
  onRestoreError?: (e: unknown) => void;
  /** Flight School: the person sets the task's kind of work. */
  onSetClass?: (taskClass: string) => void;
}> = ({
  detail,
  names,
  tasksById = {},
  now = Date.now(),
  busy = false,
  notice,
  onAction,
  onApproveCheck,
  onOpenChat,
  onOpenTask,
  onResolveFlag,
  onFork,
  failures = [],
  changes = [],
  onRunHeld,
  onRetryAutofix,
  onSkipAutofix,
  onDecideChange,
  restoreApi,
  onRestored,
  onRestoreError,
  onSetClass,
}) => {
  const t = detail.task;
  const flags = openFlags(t);
  const others = t.needs_you.filter((n) => !OWN_SECTION.has(n.kind));
  const changeItems = t.needs_you.filter((n) => n.kind === 'workflow_change');
  const fixText = t.needs_you.find((n) => n.kind === 'fix_needed')?.text ?? null;
  const held = !!t.held_at && (t.state === 'ready' || t.state === 'backlog');
  const copies = Object.values(tasksById).filter((x) => x.forked_from === t.id && x.id !== t.id);
  const source = t.forked_from ? tasksById[t.forked_from] : undefined;
  const [forking, setForking] = useState(false);
  const [forkReason, setForkReason] = useState('');
  const forkId = useId();
  const receipt = useMemo(() => buildReceipt(detail, names), [detail, names]);
  const [asking, setAsking] = useState<TaskAction | null>(null);
  const [reason, setReason] = useState('');
  const reasonId = useId();
  const owner = (t.owner_kwant_id && names[t.owner_kwant_id]) || t.owner_name;
  const st = stateLabel(t.state);
  const actions = actionsFor(t);
  const chat = sourceChat(t);
  const [showAll, setShowAll] = useState(false);
  const events = showAll ? detail.events : detail.events.slice(-12);

  useEffect(() => {
    setAsking(null);
    setReason('');
    setForking(false);
    setForkReason('');
  }, [t.id, t.state]);

  const press = (a: TaskAction) => {
    if (NEEDS_REASON[a]) {
      setAsking(a);
      return;
    }
    onAction(a);
  };

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-1.5">
          <Badge tone={st.tone}>{st.label}</Badge>
          {t.priority >= 2 && <Badge tone={t.priority >= 3 ? 'rose' : 'amber'}>{priorityLabel(t.priority)}</Badge>}
          {t.mission_title && <span className="text-[11px] text-zinc-500 break-words">{t.mission_title}</span>}
        </div>
        {(t.state === 'blocked' || t.state === 'failed') && (
          <p className="px-3 py-2 rounded-lg border border-rose-500/30 bg-rose-500/10 text-xs text-rose-200">{cardStatus(t, names, now)}</p>
        )}
        {notice && <Notice text={notice.text} error={notice.error} />}
        {held && <HeldBox task={t} busy={busy} now={now} onRun={onRunHeld} />}
        {actions.length > 0 && !asking && (
          <div className="flex flex-wrap gap-2" data-actions>
            {actions.map((a) => (
              <button
                key={a}
                type="button"
                disabled={busy}
                onClick={() => press(a)}
                className={a === 'verify' ? goodBtn : a === 'retry' || a === 'unblock' ? mainBtn : a === 'cancel' ? dangerBtn : quietBtn}
              >
                {a === 'verify' && <CheckCircle2 size={13} aria-hidden="true" />}
                {ACTION_LABELS[a]}
              </button>
            ))}
            {busy && <Loader2 size={14} className="self-center animate-spin motion-reduce:animate-none text-zinc-400" aria-label="Working" />}
          </div>
        )}
        {asking && (
          <form
            className="space-y-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (!reason.trim()) return;
              onAction(asking, reason.trim());
            }}
          >
            <label htmlFor={reasonId} className="block text-[11px] font-medium text-zinc-300">
              {NEEDS_REASON[asking]}
            </label>
            <textarea id={reasonId} autoFocus rows={2} value={reason} onChange={(e) => setReason(e.target.value)} className={`${field} resize-y`} />
            <div className="flex flex-wrap gap-2">
              <button type="submit" disabled={busy || !reason.trim()} className={mainBtn}>
                {ACTION_LABELS[asking]}
              </button>
              <button type="button" onClick={() => setAsking(null)} className={quietBtn}>
                Never mind
              </button>
            </div>
          </form>
        )}
        {t.state === 'review' && <p className="text-[11px] text-zinc-500">“Mark verified” records that you checked it yourself.</p>}
        {t.state === 'verified' && onFork && !forking && (
          <button type="button" disabled={busy} onClick={() => setForking(true)} className={quietBtn} data-fork>
            <Copy size={13} aria-hidden="true" /> Redo as a copy
          </button>
        )}
        {t.state === 'verified' && onFork && forking && (
          <form
            className="space-y-2"
            data-fork-form
            onSubmit={(e) => {
              e.preventDefault();
              if (!forkReason.trim()) return;
              onFork(forkReason.trim());
            }}
          >
            <label htmlFor={forkId} className="block text-[11px] font-medium text-zinc-300">
              Why redo it?
            </label>
            <textarea id={forkId} autoFocus rows={2} maxLength={500} value={forkReason} onChange={(e) => setForkReason(e.target.value)} className={`${field} resize-y`} />
            <p className="text-[11px] text-zinc-400">The copy starts running on its own. This task stays Verified, and tasks that build on it get a “May be invalid” flag.</p>
            <div className="flex flex-wrap gap-2">
              <button type="submit" disabled={busy || !forkReason.trim()} className={mainBtn}>
                Make the copy
              </button>
              <button type="button" onClick={() => setForking(false)} className={quietBtn}>
                Never mind
              </button>
            </div>
          </form>
        )}
      </div>

      {flags.length > 0 && (
        <section aria-labelledby="flag-h" className="space-y-2" data-flags>
          <h4 id="flag-h" className="text-xs font-semibold text-amber-200">
            May be invalid
          </h4>
          <ul className="space-y-2">
            {flags.map((f) => (
              <FlagBox
                key={f.id}
                flag={f}
                state={t.state}
                causeTitle={(t.flags ?? []).find((x) => x.id === f.id)?.cause_title ?? (f.causeTaskId ? tasksById[f.causeTaskId]?.title : null)}
                busy={busy}
                onResolve={onResolveFlag}
                onOpenTask={onOpenTask}
              />
            ))}
          </ul>
        </section>
      )}

      {changeItems.length > 0 && (
        <section aria-labelledby="wc-h" className="space-y-2" data-workflow-changes>
          <h4 id="wc-h" className="text-xs font-semibold text-amber-200">
            Change to approve
          </h4>
          <ul className="space-y-2">
            {changeItems.map((n) => (
              <WorkflowChangeBox key={n.id} item={n} change={changes.find((c) => c.id === n.id)} names={names} busy={busy} now={now} onDecide={onDecideChange} />
            ))}
          </ul>
        </section>
      )}

      <FailureBox task={t} failures={failures} fixText={fixText} busy={busy} now={now} onRetry={onRetryAutofix} onSkip={onSkipAutofix} />

      {(t.forked_from || copies.length > 0) && (
        <section aria-label="Copies" className="space-y-1 text-xs" data-copies>
          {t.forked_from && (
            <p className="flex flex-wrap items-center gap-1.5 text-zinc-400" data-copied-from>
              <GitBranch size={13} aria-hidden="true" />
              Copied from{' '}
              {onOpenTask ? (
                <button type="button" onClick={() => onOpenTask(t.forked_from!)} className={`text-left text-sky-300 hover:underline rounded ${focusRing}`}>
                  {source ? `“${source.title}”` : 'an earlier task'}
                </button>
              ) : (
                <span className="text-zinc-200">{source ? `“${source.title}”` : 'an earlier task'}</span>
              )}
              {source && <span className="text-zinc-500">· {stateLabel(source.state).label}</span>}
            </p>
          )}
          {copies.map((c) => (
            <p key={c.id} className="flex flex-wrap items-center gap-1.5 text-zinc-400" data-redone-as={c.id}>
              <Copy size={13} aria-hidden="true" />
              Redone as{' '}
              {onOpenTask ? (
                <button type="button" onClick={() => onOpenTask(c.id)} className={`text-left text-sky-300 hover:underline rounded ${focusRing}`}>
                  “{c.title}”
                </button>
              ) : (
                <span className="text-zinc-200">“{c.title}”</span>
              )}
              <span className="text-zinc-500">· {stateLabel(c.state).label}</span>
            </p>
          ))}
        </section>
      )}

      <section aria-labelledby="acc-h" className="space-y-1">
        <h4 id="acc-h" className="text-xs font-semibold text-zinc-200">
          What done looks like
        </h4>
        {t.acceptance.trim() ? (
          <p className="text-xs text-zinc-300 whitespace-pre-wrap break-words">{t.acceptance}</p>
        ) : (
          <p className="text-xs text-zinc-500">No acceptance criteria were written for this task.</p>
        )}
      </section>

      <dl className="grid grid-cols-2 gap-3">
        <Fact label="Owner">{owner ?? <span className="text-zinc-500">No owner yet</span>}</Fact>
        <Fact label="From">{sourceLabel(t.source_kind)}</Fact>
        <Fact label="Cost">
          <span className="tabular-nums">{costText(t.spent_tokens, t.budget_tokens)}</span>
          {t.budget_tokens > 0 && (
            <span className="block mt-1">
              <SpendBar spent={t.spent_tokens} budget={t.budget_tokens} label="Task budget used" />
            </span>
          )}
        </Fact>
        <Fact label="Tries">
          {t.attempts} of {t.max_attempts}
        </Fact>
        {showsTier(t) && (
          <>
            <Fact label="Kind of work">
              <TaskClassEditor task={t} disabled={busy} onSetClass={onSetClass} />
            </Fact>
            <Fact label="Flight School tier">
              <span data-owner-tier={t.owner_tier}>{tierLabel(t.owner_tier)}</span>
              <span className="block text-[11px] text-zinc-500">{TIER_HINTS[t.owner_tier]}</span>
            </Fact>
          </>
        )}
      </dl>

      {chat && onOpenChat && (
        <button type="button" onClick={() => onOpenChat(t)} className={quietBtn} data-open-chat>
          <MessageSquare size={13} aria-hidden="true" /> Open the chat it came from
        </button>
      )}

      {t.claim && (
        <section aria-labelledby="live-h" className="px-3 py-2 rounded-lg border border-violet-500/30 bg-violet-500/10 text-xs" data-live>
          <h4 id="live-h" className="flex items-center gap-1.5 font-semibold text-violet-100">
            <span className="relative flex w-2 h-2" aria-hidden="true">
              <span className="absolute inline-flex w-full h-full rounded-full bg-violet-400 opacity-75 animate-ping motion-reduce:animate-none" />
              <span className="relative inline-flex w-2 h-2 rounded-full bg-violet-400" />
            </span>
            Working now
          </h4>
          <p className="mt-0.5 text-violet-200">
            {names[t.claim.kwant_id] || owner || 'A bot'} started {ago(t.claim.claimed_at, now)}. If nothing happens, it goes back to Ready {untilText(t.claim.expires_at, now)}.
          </p>
        </section>
      )}

      {others.length > 0 && (
        <section aria-labelledby="ny-h" className="space-y-2" data-needs-you>
          <h4 id="ny-h" className="text-xs font-semibold text-amber-200">
            Waiting for you
          </h4>
          <ul className="space-y-1.5">
            {others.map((n) => (
              <li key={n.id} className="flex flex-wrap items-center gap-2 px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100">
                <span className="flex-1 min-w-[10rem] break-words">
                  <span className="font-medium">
                    {n.kind === 'approval'
                      ? 'Approval'
                      : n.kind === 'human_step'
                        ? 'Step in the browser'
                        : n.kind === 'check_approval'
                          ? 'Check to allow'
                          : n.kind === ENGINE_CANT_SANDBOX
                            ? 'Engine can’t be sandboxed'
                            : 'Question'}
                    :{' '}
                  </span>
                  {n.kind === ENGINE_CANT_SANDBOX ? <span data-engine-cant-sandbox>{engineCantSandboxText(owner ?? null)}</span> : n.text}
                </span>
                {n.kind === 'check_approval' ? (
                  (() => {
                    const c = detail.checks.find((x) => x.id === n.id);
                    return c && c.status === 'needs_approval' ? (
                      <button type="button" disabled={busy} onClick={() => onApproveCheck(c)} className="px-2 py-1 rounded border border-amber-500/40 hover:bg-amber-500/10 min-h-[32px] disabled:opacity-50">
                        Allow this check
                      </button>
                    ) : null;
                  })()
                ) : onOpenChat && answerChat(t, n) && (
                  <button type="button" onClick={() => onOpenChat(t, answerChat(t, n))} className="px-2 py-1 rounded border border-amber-500/40 hover:bg-amber-500/10 min-h-[32px]" data-answer-in-chat>
                    Answer in chat
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {t.waiting_on.length > 0 && (
        <section aria-labelledby="wait-h" className="space-y-1">
          <h4 id="wait-h" className="text-xs font-semibold text-zinc-200">
            Waiting on
          </h4>
          <ul className="space-y-1">
            {t.waiting_on.map((id) => {
              const dep = tasksById[id];
              return (
                <li key={id} className="text-xs">
                  {dep && onOpenTask ? (
                    <button type="button" onClick={() => onOpenTask(id)} className={`text-left text-sky-300 hover:underline rounded ${focusRing}`}>
                      {dep.title}
                    </button>
                  ) : (
                    <span className="text-zinc-400">{dep?.title ?? 'Another task'}</span>
                  )}
                  {dep && <span className="text-zinc-500"> · {stateLabel(dep.state).label}</span>}
                </li>
              );
            })}
          </ul>
        </section>
      )}

      <EvidenceReceipt receipt={receipt} names={names} busy={busy} onApproveCheck={onApproveCheck} />

      <RestoreSection task={t} api={restoreApi} titleOf={(id) => tasksById[id]?.title} now={now} onRestored={onRestored} onError={onRestoreError} />

      {detail.events.length > 0 && (
        <section aria-labelledby="ev-h" className="space-y-1.5">
          <h4 id="ev-h" className="text-xs font-semibold text-zinc-200">
            Activity
          </h4>
          <ol className="space-y-1 border-l border-zinc-800 pl-3">
            {events.map((e) => (
              <li key={e.id} className="text-[11px] text-zinc-400 break-words">
                {eventText(e, names)} <span className="text-zinc-600">· {ago(e.created_at, now)}</span>
              </li>
            ))}
          </ol>
          {!showAll && detail.events.length > events.length && (
            <button type="button" onClick={() => setShowAll(true)} className={`text-[11px] text-sky-300 hover:underline rounded ${focusRing}`}>
              Show all {detail.events.length}
            </button>
          )}
        </section>
      )}
    </div>
  );
};

/** The detail panel: a side sheet on desktop, the whole screen on a phone. */
export const TaskPanel: React.FC<{
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}> = ({ title, onClose, children }) => {
  const closeRef = useRef<HTMLButtonElement>(null);
  const headingId = useId();
  useEffect(() => {
    const back = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      back?.focus?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="fixed inset-0 z-40 flex justify-end" data-task-panel>
      <button type="button" aria-label="Close task" tabIndex={-1} onClick={onClose} className="absolute inset-0 bg-black/50 cursor-default" />
      <div role="dialog" aria-modal="true" aria-labelledby={headingId} className="relative w-full sm:max-w-xl h-full flex flex-col bg-zinc-950 sm:border-l border-zinc-800 shadow-2xl">
        <div className="min-h-14 py-2 px-3 sm:px-5 border-b border-zinc-800/80 flex items-start gap-2 bg-zinc-900/30">
          <h3 id={headingId} className="flex-1 min-w-0 pt-1.5 text-sm font-semibold text-zinc-100 break-words">
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

// ---------- Work graph ----------

/** How a mission's tasks connect, as steps: each task sits one step after what it builds on. */
export const WorkGraphView: React.FC<{
  graph: WorkGraph | null;
  names: Record<string, string>;
  loading?: boolean;
  error?: string | null;
  scanning?: boolean;
  now?: number;
  onOpenTask: (id: string) => void;
  onScan?: () => void;
  onRetry?: () => void;
}> = ({ graph, names, loading = false, error = null, scanning = false, now = Date.now(), onOpenTask, onScan, onRetry }) => {
  const levels: GraphLevel[] = useMemo(() => (graph ? graphLevels(graph) : []), [graph]);
  const titles = useMemo(() => Object.fromEntries((graph?.nodes ?? []).map((n) => [n.id, n.title])), [graph]);
  const flagged = (graph?.nodes ?? []).filter((n) => n.flags.length > 0).length;
  return (
    <section aria-labelledby="graph-h" className="space-y-3" data-graph>
      <div className="flex flex-wrap items-center gap-2">
        <div className="min-w-0 flex-1">
          <h3 id="graph-h" className="flex items-center gap-1.5 text-xs font-semibold text-zinc-200">
            <GitBranch size={14} className="text-sky-400" aria-hidden="true" /> How the tasks connect
          </h3>
          <p className="text-[11px] text-zinc-500">
            {graph?.scanned_at ? `Files last checked for changes ${ago(graph.scanned_at, now)}` : 'Files haven’t been checked for changes yet'}
            {flagged > 0 ? ` · ${flagged} may be invalid` : ''}
          </p>
        </div>
        {onScan && (
          <button type="button" disabled={scanning || loading} onClick={onScan} className={quietBtn} data-scan>
            <RefreshCw size={12} className={scanning ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
            Check files now
          </button>
        )}
      </div>
      {error && (
        <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
          <span className="flex-1 min-w-0 break-words">Couldn’t load how the tasks connect. {error}</span>
          {onRetry && (
            <button type="button" onClick={onRetry} className="shrink-0 px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 min-h-[32px]">
              Retry
            </button>
          )}
        </div>
      )}
      {!graph && loading && (
        <p className="flex items-center gap-2 text-xs text-zinc-400">
          <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
        </p>
      )}
      {graph && graph.nodes.length === 0 && <p className="px-3 py-6 rounded-xl border border-dashed border-zinc-800 text-center text-xs text-zinc-500" data-graph-empty>This mission has no tasks yet.</p>}
      {graph && graph.scan_skipped.length > 0 && (
        <p className="text-[11px] text-zinc-500" data-scan-skipped>
          {graph.scan_skipped.length === 1 ? '1 file wasn’t checked' : `${graph.scan_skipped.length} files weren’t checked`} (too big, too many, or outside the task folder). A file that isn’t checked never counts as changed.
        </p>
      )}
      {levels.length > 0 && (
        <ol className="grid gap-3 lg:grid-flow-col lg:auto-cols-[minmax(13rem,1fr)] lg:overflow-x-auto pb-1">
          {levels.map((lv) => (
            <li key={lv.step} className="min-w-0 space-y-2" data-graph-step={lv.step}>
              <h4 className="px-1 text-[11px] font-medium text-zinc-500">{lv.step === 1 ? 'Step 1 · builds on nothing' : `Step ${lv.step}`}</h4>
              <ul className="space-y-2">
                {lv.rows.map(({ node: n, after, notFlaggedText }) => {
                  const st = stateLabel(n.state);
                  const owner = (n.owner_kwant_id && names[n.owner_kwant_id]) || n.owner_name;
                  return (
                    <li key={n.id} data-graph-node={n.id} data-state={n.state}>
                      <button
                        type="button"
                        onClick={() => onOpenTask(n.id)}
                        className={`${card} w-full text-left p-2.5 space-y-1 hover:border-zinc-700 hover:bg-zinc-900/70 ${focusRing} ${n.flags.length ? 'border-amber-500/40' : ''}`}
                      >
                        <span className="flex flex-wrap items-center gap-1.5">
                          <Badge tone={st.tone}>{st.label}</Badge>
                          {n.flags.length > 0 && <Badge tone="amber">May be invalid</Badge>}
                        </span>
                        <span className="block text-xs font-medium text-zinc-100 break-words">{n.title}</span>
                        {after.length > 0 && (
                          <span className="block text-[11px] text-zinc-400 break-words" data-graph-after>
                            After {after.map((a) => `“${a.title}”`).join(', ')}
                          </span>
                        )}
                        {n.forked_from && (
                          <span className="block text-[11px] text-zinc-500 break-words" data-graph-forked>
                            Copied from {titles[n.forked_from] ? `“${titles[n.forked_from]}”` : 'an earlier task'}
                          </span>
                        )}
                        {n.flags.map((f) => (
                          <span key={f.id} className="flex items-start gap-1 text-[11px] text-amber-200 break-words" data-graph-flag={f.cause}>
                            <AlertTriangle size={11} className="mt-0.5 shrink-0" aria-hidden="true" />
                            {flagText(f)}
                          </span>
                        ))}
                        {notFlaggedText && (
                          <span className="block text-[11px] text-zinc-500" data-graph-not-flagged>
                            {notFlaggedText}
                          </span>
                        )}
                        {owner && <span className="block text-[11px] text-zinc-500 truncate">{owner}</span>}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
};

// ---------- Page ----------

export interface MissionBoardViewProps {
  tasks: Task[] | null;
  missions: Mission[];
  insights: Insights | null;
  names: Record<string, string>;
  missionId: string | null;
  column: Column;
  loading: boolean;
  error: string | null;
  needsToken?: boolean;
  busyMission?: boolean;
  notice?: { text: string; error: boolean } | null;
  now?: number;
  onMission: (id: string | null) => void;
  onColumn: (c: Column) => void;
  onOpenTask: (t: Task) => void;
  onApproveMission: (m: Mission) => void;
  onCancelMission: (m: Mission) => void;
  onRefresh: () => void;
  onBack?: () => void;
  /** With a mission picked: the board, or how its tasks connect. */
  view?: 'board' | 'graph';
  onView?: (v: 'board' | 'graph') => void;
  graph?: WorkGraph | null;
  graphLoading?: boolean;
  graphError?: string | null;
  scanning?: boolean;
  onScan?: () => void;
  onRetryGraph?: () => void;
}

export const MissionBoardView: React.FC<MissionBoardViewProps> = ({
  tasks,
  missions,
  insights,
  names,
  missionId,
  column,
  loading,
  error,
  needsToken = false,
  busyMission = false,
  notice,
  now,
  onMission,
  onColumn,
  onOpenTask,
  onApproveMission,
  onCancelMission,
  onRefresh,
  onBack,
  view = 'board',
  onView,
  graph = null,
  graphLoading = false,
  graphError = null,
  scanning = false,
  onScan,
  onRetryGraph,
}) => {
  const selectId = useId();
  const visibleMissions = missions.filter((m) => m.status !== 'cancelled');
  const mission = missionId ? missions.find((m) => m.id === missionId) ?? null : null;
  const showGraph = !!mission && !!onView && view === 'graph';
  const drafts = mission ? [] : visibleMissions.filter((m) => m.status === 'draft');
  const shown = (tasks ?? []).filter((t) => t.state !== 'cancelled' && (!missionId || t.mission_id === missionId));
  const empty = tasks !== null && shown.length === 0 && !mission && drafts.length === 0;
  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      <div className={`h-14 shrink-0 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`}>
        {onBack && (
          <button type="button" onClick={onBack} aria-label="Back to chats" className={`shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 ${focusRing}`}>
            <ChevronLeft size={18} />
          </button>
        )}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">Mission board</h2>
          <p className="text-[11px] text-zinc-400 truncate mt-0.5">Your team’s work, and proof it’s done</p>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading || needsToken}
          aria-label="Refresh"
          className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 ${focusRing}`}
        >
          <RefreshCw size={12} className={loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          <span className="hidden sm:inline">Refresh</span>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-6" aria-busy={loading}>
        <div className="max-w-[1500px] mx-auto space-y-4">
          {needsToken && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 flex flex-wrap items-center gap-2 text-xs text-amber-100">
              <span>Only you can change the board. Confirm it’s you to continue.</span>
              <button type="button" onClick={requestPersonToken} className="ml-auto px-2 py-0.5 rounded border border-amber-500/40 hover:bg-amber-500/10 min-h-[32px]">
                Confirm it’s you
              </button>
            </div>
          )}
          {error && !needsToken && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
              <AlertTriangle size={14} className="shrink-0" aria-hidden="true" />
              <span className="flex-1 min-w-0">
                <span className="block font-medium">Couldn’t load the board.</span>
                <span className="block mt-0.5 text-[11px] text-rose-300/70 break-words" data-error-detail>
                  {error}
                </span>
              </span>
              <button type="button" onClick={onRefresh} className="shrink-0 px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 min-h-[32px]">
                Retry
              </button>
            </div>
          )}
          {notice && <Notice text={notice.text} error={notice.error} />}

          {tasks === null && loading && (
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading the board…
            </p>
          )}

          {tasks !== null && (
            <>
              <InsightsStrip insights={insights} needsYou={needsYouCount(tasks)} />

              {visibleMissions.length > 0 && (
                <div className="flex flex-wrap items-center gap-2">
                  <label htmlFor={selectId} className="text-[11px] font-medium text-zinc-400">
                    Show
                  </label>
                  <select
                    id={selectId}
                    value={missionId ?? ''}
                    onChange={(e) => onMission(e.target.value || null)}
                    className="min-h-[40px] sm:min-h-[32px] max-w-full bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
                  >
                    <option value="">All work</option>
                    {visibleMissions.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.title}
                        {m.status === 'draft' ? ' (waiting for your OK)' : m.status === 'done' ? ' (done)' : ''}
                      </option>
                    ))}
                  </select>
                </div>
              )}

              {mission && <MissionSummary mission={mission} busy={busyMission} onApprove={() => onApproveMission(mission)} onCancel={() => onCancelMission(mission)} />}
              {mission && onView && (
                <div role="group" aria-label="Show as" className="inline-flex rounded-lg border border-zinc-800 p-0.5" data-view-toggle>
                  {(['board', 'graph'] as const).map((v) => (
                    <button
                      key={v}
                      type="button"
                      aria-pressed={view === v}
                      onClick={() => onView(v)}
                      className={`min-h-[40px] sm:min-h-[32px] px-3 rounded-md text-xs font-medium ${focusRing} ${view === v ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-400 hover:text-zinc-200'}`}
                    >
                      {v === 'board' ? 'Board' : 'How tasks connect'}
                    </button>
                  ))}
                </div>
              )}
              {drafts.map((m) => (
                <MissionSummary key={m.id} mission={m} busy={busyMission} onApprove={() => onApproveMission(m)} onCancel={() => onCancelMission(m)} />
              ))}

              {showGraph ? (
                <WorkGraphView
                  graph={graph}
                  names={names}
                  loading={graphLoading}
                  error={graphError}
                  scanning={scanning}
                  now={now}
                  onOpenTask={(id) => {
                    const t = (tasks ?? []).find((x) => x.id === id);
                    if (t) onOpenTask(t);
                  }}
                  onScan={onScan}
                  onRetry={onRetryGraph}
                />
              ) : empty ? (
                <div className="py-12 flex flex-col items-center text-center gap-2" data-empty>
                  <Target size={28} className="text-sky-400" aria-hidden="true" />
                  <p className="text-sm font-medium text-zinc-200">No tasks yet</p>
                  <p className="text-xs text-zinc-500 max-w-sm">
                    When a lead plans a mission, hands work to a teammate, or a routine runs, the task shows up here. It reaches Verified only when a check passes.
                  </p>
                </div>
              ) : (
                <Board tasks={shown} names={names} column={column} now={now} showMission={!missionId} onColumn={onColumn} onOpen={onOpenTask} />
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
};

/** The live board for the open space. */
export const MissionBoardPage: React.FC<{
  spaceId: string;
  names: Record<string, string>;
  onBack?: () => void;
  onOpenChat?: (t: Task, place?: ChatPlace | null) => void;
}> = ({ spaceId, names, onBack, onOpenChat }) => {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [missions, setMissions] = useState<Mission[]>([]);
  const [insights, setInsights] = useState<Insights | null>(null);
  const [missionId, setMissionId] = useState<string | null>(null);
  const [column, setColumn] = useState<Column>('ready');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [needsToken, setNeedsToken] = useState(false);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [busyMission, setBusyMission] = useState(false);
  const [openId, setOpenId] = useState<string | null>(null);
  const [detail, setDetail] = useState<TaskDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [detailNotice, setDetailNotice] = useState<{ text: string; error: boolean } | null>(null);
  const [failures, setFailures] = useState<TaskFailure[]>([]);
  const [changes, setChanges] = useState<WorkflowChange[]>([]);
  const [busy, setBusy] = useState(false);
  const [view, setView] = useState<'board' | 'graph'>('board');
  const [graph, setGraph] = useState<WorkGraph | null>(null);
  const [graphLoading, setGraphLoading] = useState(false);
  const [graphError, setGraphError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  // One key per intent: a repeat of the same press (double click, retry after a dropped answer) reuses it.
  const pendingKey = useRef<{ what: string; key: string } | null>(null);
  const inFlight = useRef(false);
  const keyFor = (what: string) => {
    if (pendingKey.current?.what !== what) pendingKey.current = { what, key: newIdempotencyKey() };
    return pendingKey.current.key;
  };
  // Run on a held task: one key per hold, kept after success, so a second press replays the first 200.
  const runKeys = useRef<Record<string, string>>({});
  const alive = useRef(true);
  const openIdRef = useRef<string | null>(null);
  openIdRef.current = openId;

  const failMsg = (e: unknown) => (e instanceof Error ? e.message : 'Something went wrong.');

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [t, m] = await Promise.all([missionsApi.tasks(), missionsApi.missions()]);
      if (!alive.current) return;
      setTasks(t.filter((x) => x.state !== 'cancelled'));
      setMissions(m);
      setError(null);
      setNeedsToken(false);
      missionsApi.insights().then((i) => alive.current && setInsights(i), () => {});
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      else setError(failMsg(e));
    } finally {
      if (alive.current) setLoading(false);
    }
  }, []);

  const loadDetail = useCallback(async (id: string) => {
    try {
      const d = await missionsApi.task(id);
      if (!alive.current || openIdRef.current !== id) return;
      setDetail(d);
      setDetailError(null);
      // AutoFix extras: never block the panel on them.
      const t = d.task;
      if (t.last_failure || t.autofix) {
        missionsApi.failures(id).then((f) => alive.current && openIdRef.current === id && setFailures(f), () => {});
      } else setFailures([]);
      if (t.needs_you.some((n) => n.kind === 'workflow_change')) {
        missionsApi.workflowChanges('proposed', id).then((cs) => alive.current && openIdRef.current === id && setChanges(cs), () => {});
      } else setChanges([]);
    } catch (e) {
      if (alive.current && openIdRef.current === id) setDetailError(failMsg(e));
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    load();
    const again = () => load();
    window.addEventListener(PERSON_SIGNED_IN_EVENT, again);
    return () => {
      alive.current = false;
      window.removeEventListener(PERSON_SIGNED_IN_EVENT, again);
    };
  }, [load]);

  const loadGraph = useCallback(async (id: string) => {
    setGraphLoading(true);
    try {
      const g = await missionsApi.graph(id);
      if (!alive.current) return;
      setGraph(g.mission_id === id ? g : { ...g, mission_id: id });
      setGraphError(null);
    } catch (e) {
      if (alive.current) setGraphError(failMsg(e));
    } finally {
      if (alive.current) setGraphLoading(false);
    }
  }, []);

  useEffect(() => {
    if (view === 'graph' && missionId) loadGraph(missionId);
    if (!missionId) setGraph(null);
  }, [view, missionId, loadGraph]);

  // Live: patch the list from each event; the open card re-reads its checks and activity.
  const insightsTimer = useRef<number | null>(null);
  const graphTimer = useRef<number | null>(null);
  const graphMission = useRef<string | null>(null);
  graphMission.current = view === 'graph' ? missionId : null;
  useEffect(() => {
    const refreshGraph = () => {
      if (graphTimer.current) window.clearTimeout(graphTimer.current);
      graphTimer.current = window.setTimeout(() => graphMission.current && loadGraph(graphMission.current), 800);
    };
    const refreshInsights = () => {
      if (insightsTimer.current) window.clearTimeout(insightsTimer.current);
      insightsTimer.current = window.setTimeout(() => missionsApi.insights().then((i) => alive.current && setInsights(i), () => {}), 1500);
    };
    const offTask = onMission('task_updated', ({ task }) => {
      if (!task || task.space_id !== spaceId) return;
      setTasks((list) => (list ? upsertTask(list, task) : list));
      if (openIdRef.current === task.id) loadDetail(task.id);
      if (graphMission.current && task.mission_id === graphMission.current) refreshGraph();
      refreshInsights();
    });
    const offMission = onMission('mission_updated', ({ mission }) => {
      if (!mission || mission.space_id !== spaceId) return;
      setMissions((list) => upsertMission(list, mission));
    });
    return () => {
      offTask();
      offMission();
      if (insightsTimer.current) window.clearTimeout(insightsTimer.current);
      if (graphTimer.current) window.clearTimeout(graphTimer.current);
    };
  }, [spaceId, loadDetail, loadGraph]);

  const openTask = (id: string) => {
    setOpenId(id);
    setDetail(null);
    setFailures([]);
    setChanges([]);
    setDetailError(null);
    setDetailNotice(null);
    loadDetail(id);
    playSfx('click');
  };
  const closeTask = () => {
    setOpenId(null);
    setDetail(null);
  };

  const act = async (a: TaskAction, reason?: string) => {
    if (!openId) return;
    setBusy(true);
    setDetailNotice(null);
    try {
      const t = await missionsApi.act(openId, a, reason);
      if (!alive.current) return;
      setTasks((list) => (list ? upsertTask(list, t) : list));
      setDetailNotice({ text: ACTION_DONE[a], error: false });
      playSfx(a === 'verify' ? 'coin' : 'click');
      if (t.state === 'cancelled') closeTask();
      else await loadDetail(openId);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      setDetailNotice({ text: failMsg(e), error: true });
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  const setClass = async (taskClass: string) => {
    if (!openId) return;
    setBusy(true);
    setDetailNotice(null);
    try {
      const t = await missionsApi.setTaskClass(openId, taskClass);
      if (!alive.current) return;
      setTasks((list) => (list ? upsertTask(list, t) : list));
      setDetailNotice({ text: `Its kind of work is now ${taskClass}. Its bot’s tier for that applies from its next action.`, error: false });
      await loadDetail(openId);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      setDetailNotice({ text: failMsg(e), error: true });
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  const approveCheck = async (c: Check) => {
    setBusy(true);
    setDetailNotice(null);
    try {
      await missionsApi.approveCheck(c.id);
      if (!alive.current) return;
      setDetailNotice({ text: 'Allowed. The check runs now.', error: false });
      if (openId) await loadDetail(openId);
    } catch (e) {
      if (alive.current) setDetailNotice({ text: failMsg(e), error: true });
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  const afterChange = async (changed: Task[], openNext: string | null) => {
    setTasks((list) => (list ? changed.reduce((l, x) => upsertTask(l, x), list) : list));
    if (graphMission.current) loadGraph(graphMission.current);
    missionsApi.insights().then((i) => alive.current && setInsights(i), () => {});
    if (openNext) await loadDetail(openNext);
  };

  const resolveFlag = async (f: OpenFlag, choice: ResolveChoice) => {
    if (!openId || inFlight.current) return;
    const taskId = openId;
    inFlight.current = true;
    setBusy(true);
    setDetailNotice(null);
    try {
      const r = await missionsApi.resolveFlag(taskId, f.id, choice, keyFor(`resolve:${f.id}:${choice}`));
      pendingKey.current = null;
      if (!alive.current) return;
      setDetailNotice({ text: resolvedText(choice, r), error: false });
      playSfx('click');
      if (r.task.state === 'cancelled') {
        await afterChange([r.task], null);
        closeTask();
      } else await afterChange(r.fork ? [r.task, r.fork] : [r.task], taskId);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      if (e instanceof MissionError && e.status === 409) pendingKey.current = null;
      setDetailNotice({ text: flagErrorText(e), error: true });
      // A 409 means the task moved on: show what it is now.
      if (e instanceof MissionError && e.status === 409) await loadDetail(taskId);
    } finally {
      inFlight.current = false;
      if (alive.current) setBusy(false);
    }
  };

  const forkTask = async (reason: string) => {
    if (!openId || inFlight.current) return;
    const taskId = openId;
    inFlight.current = true;
    setBusy(true);
    setDetailNotice(null);
    try {
      const fork = await missionsApi.fork(taskId, reason, keyFor(`fork:${taskId}`));
      pendingKey.current = null;
      if (!alive.current) return;
      setDetailNotice({ text: `Made a copy to redo it: “${fork.title}”. It starts running on its own; this task stays Verified.`, error: false });
      playSfx('click');
      await afterChange([fork], taskId);
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      if (e instanceof MissionError && e.status >= 400 && e.status < 500 && e.status !== 401) pendingKey.current = null;
      setDetailNotice({ text: flagErrorText(e), error: true });
    } finally {
      inFlight.current = false;
      if (alive.current) setBusy(false);
    }
  };

  /** One person action on the open task: keyed, one at a time, 401 asks for the token, 409 shows what it is now. */
  const personAction = async (what: string, run: (taskId: string, key: string) => Promise<string>, errorText: (e: unknown) => string, key?: string) => {
    if (!openId || inFlight.current) return;
    const taskId = openId;
    inFlight.current = true;
    setBusy(true);
    setDetailNotice(null);
    try {
      const text = await run(taskId, key ?? keyFor(what));
      pendingKey.current = null;
      if (!alive.current) return;
      setDetailNotice({ text, error: false });
      playSfx('click');
      await afterChange([], taskId);
      load();
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      if (e instanceof MissionError && e.status >= 400 && e.status < 500 && e.status !== 401) pendingKey.current = null;
      setDetailNotice({ text: errorText(e), error: true });
      if (e instanceof MissionError && e.status === 409) await loadDetail(taskId);
    } finally {
      inFlight.current = false;
      if (alive.current) setBusy(false);
    }
  };

  const runHeld = () => {
    const hold = `${openId}:${detail?.task.held_at ?? ''}`;
    runKeys.current[hold] ??= newIdempotencyKey();
    return personAction(
      `run:${openId}`,
      async (id, key) => {
        const t = await missionsApi.runHeld(id, key);
        setTasks((list) => (list ? upsertTask(list, t) : list));
        return 'Off hold. A bot picks it up next.';
      },
      restoreErrorText,
      runKeys.current[hold],
    );
  };
  const retryAutofix = (failureId?: string) =>
    personAction(
      `autofix:${openId}:${failureId ?? ''}`,
      async (id, key) => {
        await missionsApi.runAutofix(id, key, failureId);
        return 'AutoFix retries it now.';
      },
      autofixErrorText,
    );
  const skipAutofix = (a: AutofixAttempt) =>
    personAction(
      `autofix-skip:${a.id}`,
      async (id) => {
        await missionsApi.cancelAutofix(id, a.id);
        return 'AutoFix won’t retry it. It stays failed until you try again.';
      },
      autofixErrorText,
    );
  const decideChange = (cid: string, approve: boolean) =>
    personAction(
      `change:${cid}:${approve}`,
      async () => {
        const c = await missionsApi.decideChange(cid, approve);
        if (!approve) return 'Rejected. Nothing changed.';
        return c.status === 'stale' ? 'This change no longer fits: what it changes was edited since. Nothing changed.' : 'Approved and applied.';
      },
      (e) => (e instanceof MissionError && e.status === 409 ? 'Someone already decided on this change.' : failMsg(e)),
    );
  const restored = async (r: RestoreResult, rerun: boolean) => {
    setDetailNotice({ text: restoredText(r, rerun), error: false });
    playSfx('click');
    await afterChange([r.task], r.task.id);
  };
  const restoreError = (e: unknown) => {
    if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
  };

  const scan = async () => {
    if (!missionId) return;
    setScanning(true);
    setNotice(null);
    try {
      const r = await missionsApi.scan(missionId);
      if (!alive.current) return;
      const n = r.flagged.length;
      setNotice({ text: n === 0 ? `Checked ${r.scanned === 1 ? '1 file' : `${r.scanned} files`}. Nothing changed.` : n === 1 ? 'A file changed: 1 new “May be invalid” flag.' : `Files changed: ${n} new “May be invalid” flags.`, error: false });
      await loadGraph(missionId);
      load();
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      else setNotice({ text: failMsg(e), error: true });
    } finally {
      if (alive.current) setScanning(false);
    }
  };

  const missionAction = async (m: Mission, approve: boolean) => {
    setBusyMission(true);
    setNotice(null);
    try {
      const next = approve ? await missionsApi.approveMission(m.id) : await missionsApi.cancelMission(m.id);
      if (!alive.current) return;
      setMissions((list) => upsertMission(list, next));
      setNotice({ text: approve ? `Approved “${m.title}”. Its tasks can start now.` : `“${m.title}” won’t run.`, error: false });
      if (!approve && missionId === m.id) setMissionId(null);
      load();
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof MissionError && e.status === 401) setNeedsToken(true);
      else setNotice({ text: failMsg(e), error: true });
    } finally {
      if (alive.current) setBusyMission(false);
    }
  };

  const tasksById = useMemo(() => Object.fromEntries((tasks ?? []).map((t) => [t.id, t])), [tasks]);
  const listed = openId ? tasksById[openId] : undefined;

  return (
    <>
      <MissionBoardView
        tasks={tasks}
        missions={missions}
        insights={insights}
        names={names}
        missionId={missionId}
        column={column}
        loading={loading}
        error={error}
        needsToken={needsToken}
        busyMission={busyMission}
        notice={notice}
        onMission={setMissionId}
        onColumn={setColumn}
        onOpenTask={(t) => openTask(t.id)}
        onApproveMission={(m) => missionAction(m, true)}
        onCancelMission={(m) => missionAction(m, false)}
        onRefresh={() => {
          load();
          if (view === 'graph' && missionId) loadGraph(missionId);
        }}
        onBack={onBack}
        view={view}
        onView={setView}
        graph={graph && graph.mission_id === missionId ? graph : null}
        graphLoading={graphLoading}
        graphError={graphError}
        scanning={scanning}
        onScan={scan}
        onRetryGraph={() => missionId && loadGraph(missionId)}
      />
      {openId && (
        <TaskPanel title={detail?.task.title ?? listed?.title ?? 'Task'} onClose={closeTask}>
          {detail ? (
            <TaskDetailBody
              detail={detail}
              names={names}
              tasksById={tasksById}
              busy={busy}
              notice={detailNotice}
              onAction={act}
              onApproveCheck={approveCheck}
              onResolveFlag={resolveFlag}
              onFork={forkTask}
              failures={failures}
              changes={changes}
              onRunHeld={runHeld}
              onSetClass={setClass}
              onRetryAutofix={retryAutofix}
              onSkipAutofix={skipAutofix}
              onDecideChange={decideChange}
              onRestored={restored}
              onRestoreError={restoreError}
              onOpenChat={
                onOpenChat
                  ? (t, place) => {
                      closeTask();
                      onOpenChat(t, place);
                    }
                  : undefined
              }
              onOpenTask={(id) => {
                const dep = tasksById[id];
                if (dep) {
                  const c = columnOfTask(dep);
                  if (c) setColumn(c);
                }
                openTask(id);
              }}
            />
          ) : detailError ? (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
              <span className="flex-1 min-w-0">
                <span className="block font-medium">Couldn’t load this task.</span>
                <span className="block mt-0.5 text-[11px] text-rose-300/70 break-words">{detailError}</span>
              </span>
              <button type="button" onClick={() => loadDetail(openId)} className="shrink-0 px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 min-h-[32px]">
                Retry
              </button>
            </div>
          ) : (
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
            </p>
          )}
        </TaskPanel>
      )}
    </>
  );
};
