import React, { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Gauge, Loader2 } from 'lucide-react';
import {
  FEW_TASKS,
  PerfBy,
  PerfError,
  PerfGroup,
  PerfQuery,
  PerfReport,
  PerfSince,
  SINCE_OPTIONS,
  botName,
  classText,
  costText,
  latencyText,
  loadPerformance,
  mergeOptions,
  modelText,
  perTaskText,
  perfView,
  rateLabel,
  rateText,
  tokensText,
} from '../performance';

const card = 'rounded-xl border border-zinc-800 bg-zinc-900/40';
const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';
const select =
  'min-h-[40px] sm:min-h-[32px] max-w-full bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none';

export const EMPTY_TEXT = 'No finished tasks yet. Numbers appear as tasks finish.';
export const FILTERED_EMPTY_TEXT = 'No finished tasks match these filters.';
export const REPORT_ONLY_TEXT = 'Report only: these numbers don’t change which bot gets work.';

const FewTasks: React.FC<{ n: number; min: number }> = ({ n, min }) => (
  <span
    title={`Fewer than ${min} finished tasks, so this could change a lot`}
    data-few-tasks
    className="inline-flex shrink-0 px-1.5 py-px rounded border border-amber-500/30 bg-amber-500/10 text-[10px] font-medium text-amber-200"
  >
    {FEW_TASKS}
    <span className="sr-only">: only {n}, fewer than {min} finished</span>
  </span>
);

const Verified: React.FC<{ g: PerfGroup }> = ({ g }) => (
  <span aria-label={rateLabel(g.verified_rate, g.verified_ci95)} className="tabular-nums">
    {rateText(g.verified_rate, g.verified_ci95)}
  </span>
);

/** The table body for wide screens. */
export const PerformanceTable: React.FC<{ groups: PerfGroup[]; by: PerfBy; minN: number; names?: Record<string, string> }> = ({ groups, by, minN, names }) => {
  const th = 'px-2 py-1.5 font-medium text-left whitespace-nowrap';
  const num = 'px-2 py-1.5 text-right tabular-nums whitespace-nowrap';
  return (
    <table className="hidden sm:table w-full text-xs" data-perf-table>
      <thead className="text-[11px] text-zinc-400 border-b border-zinc-800">
        <tr>
          <th scope="col" className={th}>
            Kind of work
          </th>
          <th scope="col" className={th}>
            Bot
          </th>
          {by === 'model' && (
            <th scope="col" className={th}>
              Model
            </th>
          )}
          <th scope="col" className={`${th} text-right`}>
            Tasks
          </th>
          <th scope="col" className={`${th} text-right`} title="Share that ended Verified, with the range it likely falls in (95%)">
            Verified
          </th>
          <th scope="col" className={`${th} text-right`} title="Median time a bot held the task">
            Median time
          </th>
          <th scope="col" className={`${th} text-right`} title="Median fresh tokens per task">
            Median tokens
          </th>
          <th scope="col" className={`${th} text-right`}>
            Cost per task
          </th>
          <th scope="col" className={`${th} text-right`} title="Approvals, unblocks, retries and AutoFix runs per task">
            You stepped in
          </th>
        </tr>
      </thead>
      <tbody className="text-zinc-200">
        {groups.map((g) => (
          <tr key={`${g.task_class}|${g.kwant_id}|${g.model ?? ''}`} className={`border-b border-zinc-800/60 ${g.low_confidence ? 'text-zinc-400' : ''}`} data-perf-row={g.kwant_id}>
            <td className="px-2 py-1.5 break-words">{classText(g.task_class)}</td>
            <td className="px-2 py-1.5 break-words">{botName(g, names)}</td>
            {by === 'model' && <td className="px-2 py-1.5 break-all text-zinc-400">{modelText(g.model)}</td>}
            <td className={num}>
              <span className="inline-flex items-center gap-1.5 justify-end">
                {g.low_confidence && <FewTasks n={g.n} min={minN} />}
                {g.n}
              </span>
            </td>
            <td className={num}>
              <Verified g={g} />
            </td>
            <td className={num}>{latencyText(g.median_latency_ms)}</td>
            <td className={num}>{tokensText(g.median_fresh_tokens)}</td>
            <td className={num}>{costText(g.cost_per_task_usd)}</td>
            <td className={num}>{perTaskText(g.interventions_per_task)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
};

/** The same rows as cards on a phone: no sideways scroll. */
export const PerformanceCards: React.FC<{ groups: PerfGroup[]; by: PerfBy; minN: number; names?: Record<string, string> }> = ({ groups, by, minN, names }) => (
  <ul className="sm:hidden space-y-2" data-perf-cards>
    {groups.map((g) => (
      <li key={`${g.task_class}|${g.kwant_id}|${g.model ?? ''}`} className="rounded-lg border border-zinc-800 px-3 py-2">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="text-sm font-medium text-zinc-100 break-words min-w-0">{botName(g, names)}</span>
          <span className="text-[11px] text-zinc-400 break-words min-w-0">{classText(g.task_class)}</span>
          {g.low_confidence && <FewTasks n={g.n} min={minN} />}
        </div>
        {by === 'model' && <p className="text-[11px] text-zinc-500 break-all">{modelText(g.model)}</p>}
        <dl className="mt-1.5 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          {(
            [
              ['Tasks', String(g.n)],
              ['Verified', <Verified key="v" g={g} />],
              ['Median time', latencyText(g.median_latency_ms)],
              ['Median tokens', tokensText(g.median_fresh_tokens)],
              ['Cost per task', costText(g.cost_per_task_usd)],
              ['You stepped in', `${perTaskText(g.interventions_per_task)} per task`],
            ] as Array<[string, React.ReactNode]>
          ).map(([k, v]) => (
            <div key={k} className="min-w-0">
              <dt className="text-[10px] text-zinc-500">{k}</dt>
              <dd className="text-zinc-200 tabular-nums">{v}</dd>
            </div>
          ))}
        </dl>
      </li>
    ))}
  </ul>
);

/** What one load produced; exported so tests can render each state without a server. */
export interface PerformanceBodyProps {
  report: PerfReport | null;
  error: PerfError | null;
  loading: boolean;
  by: PerfBy;
  filtered: boolean;
  names?: Record<string, string>;
  onRetry: () => void;
}

export const PerformanceBody: React.FC<PerformanceBodyProps> = ({ report, error, loading, by, filtered, names, onRetry }) => {
  const view = perfView({ report, error, loading });
  if (view === 'hidden') return null;
  if (view === 'loading')
    return (
      <p className="flex items-center gap-2 text-xs text-zinc-400" data-perf-loading>
        <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading performance…
      </p>
    );
  if (view === 'error' && error)
    return (
      <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300" data-perf-error>
        <AlertTriangle size={14} className="shrink-0" aria-hidden="true" />
        <span className="flex-1 min-w-0 break-words">
          {error.status === 401 ? 'Only you can see these numbers. Confirm it’s you, then try again.' : `Couldn’t load performance. ${error.message}`}
        </span>
        <button type="button" onClick={onRetry} className={`shrink-0 px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 min-h-[32px] ${focusRing}`}>
          Retry
        </button>
      </div>
    );
  if (view === 'empty')
    return (
      <p className="py-4 text-center text-xs text-zinc-400" data-perf-empty>
        {filtered ? FILTERED_EMPTY_TEXT : EMPTY_TEXT}
      </p>
    );
  const r = report as PerfReport;
  return (
    <div className={loading ? 'opacity-60' : ''} aria-busy={loading}>
      <PerformanceTable groups={r.groups} by={by} minN={r.min_n} names={names} />
      <PerformanceCards groups={r.groups} by={by} minN={r.min_n} names={names} />
      {r.groups.some((g) => g.low_confidence) && (
        <p className="mt-2 text-[11px] text-zinc-500">“{FEW_TASKS}”: fewer than {r.min_n} finished tasks, so the range is wide and the numbers can still change a lot.</p>
      )}
    </div>
  );
};

/** Insights → Performance. Loads on its own; hides itself if the server says it isn't for this viewer (403). */
export const PerformancePanel: React.FC<{
  names?: Record<string, string>;
  /** Changes when the board refreshes, so the numbers reload with it. */
  refreshKey?: number;
  load?: (q: PerfQuery) => Promise<PerfReport>;
}> = ({ names, refreshKey = 0, load = loadPerformance }) => {
  const ids = { title: useId(), cls: useId(), bot: useId(), since: useId() };
  const [by, setBy] = useState<PerfBy>('bot');
  const [since, setSince] = useState<PerfSince>('30d');
  const [cls, setCls] = useState('');
  const [bot, setBot] = useState('');
  const [report, setReport] = useState<PerfReport | null>(null);
  const [error, setError] = useState<PerfError | null>(null);
  const [loading, setLoading] = useState(true);
  const [classes, setClasses] = useState<Record<string, string>>({});
  const [bots, setBots] = useState<Record<string, string>>({});
  const seq = useRef(0);

  const fetchNow = useCallback(() => {
    const mine = ++seq.current;
    setLoading(true);
    load({ by, since, class: cls || undefined, bot: bot || undefined }).then(
      (r) => {
        if (mine !== seq.current) return;
        setReport(r);
        setError(null);
        setLoading(false);
        setClasses((p) => mergeOptions(p, r.groups, (g) => [g.task_class, classText(g.task_class)]));
        setBots((p) => mergeOptions(p, r.groups, (g) => [g.kwant_id, botName(g, names)]));
      },
      (e) => {
        if (mine !== seq.current) return;
        setError(e instanceof PerfError ? e : new PerfError(e instanceof Error ? e.message : 'Something went wrong.', 0));
        setLoading(false);
      },
    );
  }, [load, by, since, cls, bot, names]);

  useEffect(() => {
    fetchNow();
  }, [fetchNow, refreshKey]);

  if (perfView({ report, error, loading }) === 'hidden') return null;
  const filtered = !!cls || !!bot;
  const options = (m: Record<string, string>) => Object.entries(m).sort((a, b) => a[1].localeCompare(b[1]));
  return (
    <section aria-labelledby={ids.title} className={`${card} p-3 sm:p-4 space-y-3`} data-performance>
      <div className="flex flex-wrap items-start gap-2">
        <Gauge size={16} className="mt-0.5 shrink-0 text-sky-400" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <h3 id={ids.title} className="text-sm font-semibold text-zinc-100">
            Performance
          </h3>
          <p className="mt-0.5 text-xs text-zinc-400">How often each bot’s finished work ends Verified, and what it takes.</p>
          <p className="mt-0.5 text-[11px] text-zinc-500" data-report-only>
            {REPORT_ONLY_TEXT}
          </p>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <div role="group" aria-label="Group by" className="inline-flex rounded-lg border border-zinc-800 p-0.5">
          {(['bot', 'model'] as const).map((v) => (
            <button
              key={v}
              type="button"
              aria-pressed={by === v}
              onClick={() => setBy(v)}
              className={`min-h-[40px] sm:min-h-[28px] px-3 rounded-md text-xs font-medium ${focusRing} ${by === v ? 'bg-zinc-800 text-zinc-100' : 'text-zinc-400 hover:text-zinc-200'}`}
            >
              {v === 'bot' ? 'By bot' : 'By model'}
            </button>
          ))}
        </div>
        <label htmlFor={ids.cls} className="sr-only">
          Kind of work
        </label>
        <select id={ids.cls} value={cls} onChange={(e) => setCls(e.target.value)} className={select}>
          <option value="">All kinds of work</option>
          {options(classes).map(([k, v]) => (
            <option key={k} value={k}>
              {v}
            </option>
          ))}
        </select>
        <label htmlFor={ids.bot} className="sr-only">
          Bot
        </label>
        <select id={ids.bot} value={bot} onChange={(e) => setBot(e.target.value)} className={select}>
          <option value="">All bots</option>
          {options(bots).map(([k, v]) => (
            <option key={k} value={k}>
              {v}
            </option>
          ))}
        </select>
        <label htmlFor={ids.since} className="sr-only">
          Period
        </label>
        <select id={ids.since} value={since} onChange={(e) => setSince(e.target.value as PerfSince)} className={select}>
          {SINCE_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>

      <PerformanceBody report={report} error={error} loading={loading} by={by} filtered={filtered} names={names} onRetry={fetchNow} />
    </section>
  );
};
