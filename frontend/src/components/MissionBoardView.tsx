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
  Hand,
  Image as ImageIcon,
  Link2,
  Loader2,
  MessageSquare,
  RefreshCw,
  ShieldCheck,
  StickyNote,
  Target,
  X,
  XCircle,
} from 'lucide-react';
import { PERSON_SIGNED_IN_EVENT, requestPersonToken } from '../auth';
import { playSfx } from '../audio/chiptune';
import {
  ACTION_DONE,
  ACTION_LABELS,
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
  sourceLabel,
  stateLabel,
  untilText,
  upsertMission,
  upsertTask,
} from '../missions';

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

export const InsightsStrip: React.FC<{ insights: Insights | null }> = ({ insights }) => {
  if (!insights) return null;
  const blockedMin = insights.blocked_minutes_7d;
  const items: Array<{ label: string; value: string; hint: string; tone?: Tone; sub?: string }> = [
    { label: 'Verified this week', value: String(insights.verified_7d), hint: 'Tasks a check confirmed in the last 7 days', tone: 'emerald' },
    {
      label: 'False successes',
      value: String(insights.false_success_7d),
      hint: 'A bot said done, then the check failed (last 7 days)',
      tone: insights.false_success_7d > 0 ? 'rose' : undefined,
    },
    { label: 'Needs you now', value: String(insights.needs_you_now), hint: 'Waiting on a question, approval or step from you', tone: insights.needs_you_now > 0 ? 'amber' : undefined },
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
  const valueTone: Partial<Record<Tone, string>> = { emerald: 'text-emerald-300', rose: 'text-rose-300', amber: 'text-amber-300' };
  return (
    <section aria-label="This week" data-insights>
      <dl className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-2">
        {items.map((it, i) => (
          <div key={it.label} title={it.hint} className={`${card} px-3 py-2 ${i === items.length - 1 ? 'col-span-2 sm:col-span-1' : ''}`}>
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
  const dimmed = t.state === 'backlog' && !flags.length;
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
          {t.state === 'backlog' && <Badge tone="zinc">Waiting</Badge>}
          {t.state === 'review' && approvalHolds(t.checks_summary) && <Badge tone="amber">Needs you</Badge>}
          {flags.length > 0 && (
            <span data-flag-badge>
              <Badge tone="amber">May be invalid</Badge>
            </span>
          )}
          {t.state === 'verified' && flags.length > 0 && <Badge tone="emerald">Verified</Badge>}
          {t.priority >= 2 && <Badge tone={t.priority >= 3 ? 'rose' : 'amber'}>{priorityLabel(t.priority)}</Badge>}
          {showMission && t.mission_title && <span className="min-w-0 truncate text-[11px] text-zinc-500">{t.mission_title}</span>}
        </span>
        <span className="block text-sm font-medium text-zinc-100 break-words">{t.title}</span>
        {status && (
          <span className={`block text-[11px] break-words line-clamp-2 ${t.state === 'needs_you' || (t.state === 'review' && approvalHolds(t.checks_summary)) ? 'text-amber-200' : t.state === 'blocked' || t.state === 'failed' ? 'text-rose-300' : 'text-zinc-400'}`}>
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

// ---------- Task detail ----------

const Fact: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className="min-w-0">
    <dt className="text-[11px] text-zinc-500">{label}</dt>
    <dd className="text-xs text-zinc-200 break-words">{children}</dd>
  </div>
);

export const TaskDetailBody: React.FC<{
  detail: TaskDetail;
  names: Record<string, string>;
  tasksById?: Record<string, Task>;
  now?: number;
  busy?: boolean;
  notice?: { text: string; error: boolean } | null;
  onAction: (a: TaskAction, reason?: string) => void;
  onApproveCheck: (c: Check) => void;
  onOpenChat?: (t: Task) => void;
  onOpenTask?: (id: string) => void;
  onResolveFlag?: (f: OpenFlag, choice: ResolveChoice) => void;
  /** Copy a verified task to redo it, with the reason typed. */
  onFork?: (reason: string) => void;
}> = ({ detail, names, tasksById = {}, now = Date.now(), busy = false, notice, onAction, onApproveCheck, onOpenChat, onOpenTask, onResolveFlag, onFork }) => {
  const t = detail.task;
  const flags = openFlags(t);
  const others = t.needs_you.filter((n) => n.kind !== 'may_be_invalid');
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
                    {n.kind === 'approval' ? 'Approval' : n.kind === 'human_step' ? 'Step in the browser' : n.kind === 'check_approval' ? 'Check to allow' : 'Question'}:{' '}
                  </span>
                  {n.text}
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
                ) : chat && onOpenChat && (
                  <button type="button" onClick={() => onOpenChat(t)} className="px-2 py-1 rounded border border-amber-500/40 hover:bg-amber-500/10 min-h-[32px]">
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
              <InsightsStrip insights={insights} />

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
  onOpenChat?: (t: Task) => void;
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
              onOpenChat={
                onOpenChat
                  ? (t) => {
                      closeTask();
                      onOpenChat(t);
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
