import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  CheckCircle2,
  ChevronLeft,
  Circle,
  CircleDashed,
  ExternalLink,
  FileText,
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
  Insights,
  Mission,
  MissionError,
  NEEDS_REASON,
  Receipt,
  Task,
  TaskAction,
  TaskDetail,
  Tone,
  actionsFor,
  ago,
  buildReceipt,
  cardStatus,
  checkKindLabel,
  checksText,
  columnOf,
  costRatio,
  costText,
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
  const items: Array<{ label: string; value: string; hint: string; tone?: Tone }> = [
    { label: 'Verified this week', value: String(insights.verified_7d), hint: 'Tasks a check confirmed in the last 7 days', tone: 'emerald' },
    {
      label: 'False successes',
      value: String(insights.false_success_7d),
      hint: 'A bot said done, then the check failed (last 7 days)',
      tone: insights.false_success_7d > 0 ? 'rose' : undefined,
    },
    { label: 'Needs you now', value: String(insights.needs_you_now), hint: 'Waiting on a question, approval or step from you', tone: insights.needs_you_now > 0 ? 'amber' : undefined },
    { label: 'Blocked now', value: String(insights.blocked_now), hint: 'Tasks that can’t move until something changes', tone: insights.blocked_now > 0 ? 'rose' : undefined },
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
  const status = cardStatus(t, names, now);
  const dimmed = t.state === 'backlog';
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
          {t.priority >= 2 && <Badge tone={t.priority >= 3 ? 'rose' : 'amber'}>{priorityLabel(t.priority)}</Badge>}
          {showMission && t.mission_title && <span className="min-w-0 truncate text-[11px] text-zinc-500">{t.mission_title}</span>}
        </span>
        <span className="block text-sm font-medium text-zinc-100 break-words">{t.title}</span>
        {status && (
          <span className={`block text-[11px] break-words line-clamp-2 ${t.state === 'needs_you' ? 'text-amber-200' : t.state === 'blocked' || t.state === 'failed' ? 'text-rose-300' : 'text-zinc-400'}`}>
            {status}
          </span>
        )}
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
              <span className="mt-px">{statusIcon(l.status.tone)}</span>
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-center gap-1.5 text-xs">
                  <span className="font-medium text-zinc-100">{checkKindLabel(l.check.kind)}</span>
                  <Badge tone={l.status.tone}>{l.status.label}</Badge>
                  {!l.check.required && <span className="text-[10px] text-zinc-500">Optional</span>}
                </p>
                {l.target && <code className="block mt-1 text-[11px] text-zinc-300 break-all whitespace-pre-wrap">{l.target}</code>}
                <p className="mt-1 text-[10px] text-zinc-500">
                  {[l.by ? `Checked by ${l.by}` : null, l.check.finished_at ? ago(l.check.finished_at) : null].filter(Boolean).join(' · ') || 'Hasn’t run yet'}
                </p>
                {l.ownClaim && <p className="mt-1 text-[11px] text-amber-200">This is the owner’s own check, so it doesn’t count toward Verified.</p>}
              </div>
            </div>
            {l.check.status === 'needs_approval' && onApproveCheck && (
              <div className="flex flex-wrap items-center gap-2 pl-6">
                <p className="text-[11px] text-amber-200 flex-1 min-w-[10rem]">A bot added this check. It runs only after you OK it.</p>
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
}> = ({ detail, names, tasksById = {}, now = Date.now(), busy = false, notice, onAction, onApproveCheck, onOpenChat, onOpenTask }) => {
  const t = detail.task;
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
      </div>

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

      {t.needs_you.length > 0 && (
        <section aria-labelledby="ny-h" className="space-y-2" data-needs-you>
          <h4 id="ny-h" className="text-xs font-semibold text-amber-200">
            Waiting for you
          </h4>
          <ul className="space-y-1.5">
            {t.needs_you.map((n) => (
              <li key={n.id} className="flex flex-wrap items-center gap-2 px-3 py-2 rounded-lg border border-amber-500/30 bg-amber-500/10 text-xs text-amber-100">
                <span className="flex-1 min-w-[10rem] break-words">
                  <span className="font-medium">{n.kind === 'approval' ? 'Approval' : n.kind === 'human_step' ? 'Step in the browser' : 'Question'}: </span>
                  {n.text}
                </span>
                {chat && onOpenChat && (
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
}) => {
  const selectId = useId();
  const visibleMissions = missions.filter((m) => m.status !== 'cancelled');
  const mission = missionId ? missions.find((m) => m.id === missionId) ?? null : null;
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
              <span className="flex-1">{error}</span>
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
              {drafts.map((m) => (
                <MissionSummary key={m.id} mission={m} busy={busyMission} onApprove={() => onApproveMission(m)} onCancel={() => onCancelMission(m)} />
              ))}

              {empty ? (
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

  // Live: patch the list from each event; the open card re-reads its checks and activity.
  const insightsTimer = useRef<number | null>(null);
  useEffect(() => {
    const refreshInsights = () => {
      if (insightsTimer.current) window.clearTimeout(insightsTimer.current);
      insightsTimer.current = window.setTimeout(() => missionsApi.insights().then((i) => alive.current && setInsights(i), () => {}), 1500);
    };
    const offTask = onMission('task_updated', ({ task }) => {
      if (!task || task.space_id !== spaceId) return;
      setTasks((list) => (list ? upsertTask(list, task) : list));
      if (openIdRef.current === task.id) loadDetail(task.id);
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
    };
  }, [spaceId, loadDetail]);

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
        onRefresh={load}
        onBack={onBack}
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
                  const c = columnOf(dep.state);
                  if (c) setColumn(c);
                }
                openTask(id);
              }}
            />
          ) : detailError ? (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
              <span className="flex-1">{detailError}</span>
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
