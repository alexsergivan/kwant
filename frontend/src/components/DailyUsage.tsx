import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Loader2, RotateCcw } from 'lucide-react';
import { DailyUsage, ModelTotals, compactTokens, formatCost, freshOf, modelUsageApi } from '../modelUsage';
import { type EngineInfo, usageModelLabel } from '../engines';
import { useEngines } from '../useEngines';

interface Day {
  day: string;
  fresh: number;
  cached: number;
  /** Null when no call that day reported a cost. */
  cost: number | null;
  bots: DailyUsage[];
}

/** Rows (one per bot and day, newest day first) grouped into days with their totals. */
export function groupDays(rows: DailyUsage[]): Day[] {
  const days: Day[] = [];
  for (const r of rows) {
    let d = days.find((x) => x.day === r.day);
    if (!d) days.push((d = { day: r.day, fresh: 0, cached: 0, cost: null, bots: [] }));
    d.fresh += freshOf(r);
    d.cached += r.cache_read_tokens;
    if (r.cost_usd != null) d.cost = (d.cost ?? 0) + r.cost_usd;
    d.bots.push(r);
  }
  for (const d of days) d.bots.sort((a, b) => freshOf(b) - freshOf(a));
  return days;
}

/** "Today", "Yesterday" or "Mon 20 Oct" for a UTC day. */
export function dayLabel(day: string, now = new Date()): string {
  const today = now.toISOString().slice(0, 10);
  const yesterday = new Date(now.getTime() - 86_400_000).toISOString().slice(0, 10);
  if (day === today) return 'Today';
  if (day === yesterday) return 'Yesterday';
  const d = new Date(`${day}T00:00:00Z`);
  return Number.isNaN(d.getTime())
    ? day
    : d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short', timeZone: 'UTC' });
}

/** The windows the switch offers. */
export const RANGES = [7, 30] as const;
export type Range = (typeof RANGES)[number];

/** What the trend and the model shares measure: cost once any call in the window reported one, else fresh tokens. */
export type Metric = 'cost' | 'tokens';

export interface TrendDay {
  day: string;
  /** Cost or fresh tokens, by the window's metric; 0 on a day without use. */
  value: number;
  used: boolean;
}

const utcDay = (t: number) => new Date(t).toISOString().slice(0, 10);

/** One entry per UTC day of the window, oldest first and today last, empty days included. */
export function trendDays(rows: DailyUsage[], days: number, now = new Date()): { metric: Metric; days: TrendDay[]; total: number } {
  const metric: Metric = rows.some((r) => r.cost_usd != null) ? 'cost' : 'tokens';
  const by = new Map(groupDays(rows).map((d) => [d.day, d]));
  const out: TrendDay[] = [];
  for (let i = days - 1; i >= 0; i--) {
    const day = utcDay(now.getTime() - i * 86_400_000);
    const d = by.get(day);
    out.push({ day, value: d ? (metric === 'cost' ? d.cost ?? 0 : d.fresh) : 0, used: !!d });
  }
  return { metric, days: out, total: out.reduce((n, d) => n + d.value, 0) };
}

/** "$0.42" or "1.2M tokens". */
export function metricText(metric: Metric, v: number): string {
  return metric === 'cost' ? formatCost(v) ?? '$0' : `${compactTokens(v)} tokens`;
}

/** Each model's share of the window, by the window's metric; models with none of it last. */
export function modelShares(models: ModelTotals[], metric: Metric): Array<{ model: string; value: number; share: number; calls: number }> {
  const val = (m: ModelTotals) => (metric === 'cost' ? m.cost_usd ?? 0 : freshOf(m));
  const total = models.reduce((n, m) => n + val(m), 0);
  return models
    .map((m) => ({ model: m.model, value: val(m), share: total > 0 ? Math.round((val(m) / total) * 100) : 0, calls: m.calls }))
    .sort((a, b) => b.value - a.value);
}

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-400';

const RangeSwitch: React.FC<{ range: Range; onRange?: (r: Range) => void }> = ({ range, onRange }) => (
  <div role="group" aria-label="Show" className="flex gap-1.5">
    {RANGES.map((r) => (
      <button
        key={r}
        type="button"
        aria-pressed={range === r}
        disabled={!onRange}
        onClick={() => onRange?.(r)}
        data-usage-range={r}
        className={`px-2.5 min-h-[36px] sm:min-h-[28px] rounded-full border text-[11px] ${focusRing} ${
          range === r ? 'border-sky-500/60 bg-sky-500/15 text-sky-100' : 'border-zinc-700 text-zinc-400 hover:text-zinc-200'
        }`}
      >
        {r} days
      </button>
    ))}
  </div>
);

/** One bar per day, today on the right. Arrow keys move between days; the line under it names the day in focus. */
const Trend: React.FC<{ days: TrendDay[]; metric: Metric; now?: Date }> = ({ days, metric, now }) => {
  const [active, setActive] = useState(days.length - 1);
  const [shown, setShown] = useState<number | null>(null);
  const bars = useRef<Array<HTMLLIElement | null>>([]);
  const max = Math.max(...days.map((d) => d.value), 0);
  const label = (d: TrendDay) => `${dayLabel(d.day, now)}: ${d.used ? metricText(metric, d.value) : 'no AI use'}`;
  const move = (i: number) => {
    const j = Math.max(0, Math.min(days.length - 1, i));
    setActive(j);
    setShown(j);
    bars.current[j]?.focus();
  };
  const at = shown == null ? null : days[shown];
  return (
    <div>
      <ul
        aria-label={`${metric === 'cost' ? 'Cost' : 'Fresh tokens'} per day`}
        className={`flex items-end h-20 ${days.length > 14 ? 'gap-[2px]' : 'gap-1'}`}
        onMouseLeave={() => setShown(null)}
        data-usage-trend
      >
        {days.map((d, i) => {
          const pct = max > 0 ? (d.value / max) * 100 : 0;
          return (
            <li
              key={d.day}
              ref={(el) => void (bars.current[i] = el)}
              tabIndex={i === active ? 0 : -1}
              aria-label={label(d)}
              title={label(d)}
              data-day={d.day}
              data-used={d.used ? '' : undefined}
              onMouseEnter={() => setShown(i)}
              onFocus={() => (setActive(i), setShown(i))}
              onBlur={() => setShown(null)}
              onKeyDown={(e) => {
                const step = { ArrowLeft: -1, ArrowRight: 1, Home: -days.length, End: days.length }[e.key];
                if (step === undefined) return;
                e.preventDefault();
                move(i + step);
              }}
              className={`group relative flex-1 min-w-0 h-full flex items-end rounded-sm ${focusRing}`}
            >
              <span
                aria-hidden="true"
                className={`block w-full rounded-sm ${
                  !d.used || d.value === 0 ? 'h-px bg-zinc-700' : shown === i ? 'bg-sky-300' : 'bg-sky-500/70 group-hover:bg-sky-300'
                }`}
                style={d.used && d.value > 0 ? { height: `${Math.max(pct, 3)}%` } : undefined}
              />
            </li>
          );
        })}
      </ul>
      <div className="mt-1 flex items-center gap-2 text-[10px] text-zinc-500 tabular-nums" aria-hidden="true">
        <span className="flex-1 truncate">{dayLabel(days[0].day, now)}</span>
        <span className="text-zinc-300 truncate" data-usage-readout>
          {at ? label(at) : ''}
        </span>
        <span className="flex-1 truncate text-right">Today</span>
      </div>
    </div>
  );
};

const ByModel: React.FC<{ models: ModelTotals[]; metric: Metric; engines?: EngineInfo[] | null }> = ({ models, metric, engines }) => {
  const rows = modelShares(models, metric);
  if (!rows.length) return null;
  return (
    <div className="space-y-1.5">
      <h4 className="text-[11px] font-medium text-zinc-400">By model</h4>
      <ul className="space-y-1.5" aria-label="Use by model" data-usage-models>
        {rows.map((r) => {
          const name = usageModelLabel(r.model, engines ?? null);
          return (
            <li key={r.model} className="space-y-1" data-model={r.model}>
              <div className="flex items-center gap-2">
                <span className="flex-1 min-w-0 truncate text-[11px] text-zinc-300" title={r.model}>
                  {name}
                </span>
                <span className="shrink-0 font-mono text-[11px] tabular-nums text-zinc-400">
                  {metric === 'cost' && r.value === 0 ? 'no cost reported' : metricText(metric, r.value)}
                  <span className="text-zinc-600"> · </span>
                  <span className="text-zinc-200">{r.share}%</span>
                </span>
              </div>
              <span
                aria-hidden="true"
                className="relative block h-1 w-full overflow-hidden rounded-full bg-zinc-800"
              >
                <span className="absolute inset-y-0 left-0 rounded-full bg-sky-500/70" style={{ width: `${r.share}%` }} />
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
};

const Numbers: React.FC<{ fresh: number; cached: number; cost: number | null; strong?: boolean }> = ({ fresh, cached, cost, strong }) => (
  <span className={`shrink-0 font-mono text-[11px] tabular-nums ${strong ? 'text-zinc-200' : 'text-zinc-400'}`}>
    <span title={`${fresh.toLocaleString('en-US')} fresh tokens`}>{compactTokens(fresh)}</span>
    <span className="text-zinc-600"> · </span>
    <span className="text-zinc-500" title={`${cached.toLocaleString('en-US')} read from cache`}>
      {compactTokens(cached)}
    </span>
    {formatCost(cost) && (
      <>
        <span className="text-zinc-600"> · </span>
        <span>{formatCost(cost)}</span>
      </>
    )}
  </span>
);

/** Settings → Usage: the window's trend and spend per model, then fresh and cached tokens and cost per day and bot. */
export const DailyUsageView: React.FC<{
  rows: DailyUsage[] | null;
  models?: ModelTotals[] | null;
  range?: Range;
  onRange?: (r: Range) => void;
  engines?: EngineInfo[] | null;
  error?: string | null;
  onReload?: () => void;
  now?: Date;
}> = ({ rows, models, range = 7, onRange, engines, error, onReload, now }) => {
  const head = (
    <div className="flex flex-wrap items-center gap-2">
      <h3 className="flex-1 text-[11px] font-semibold text-zinc-200">AI use</h3>
      <RangeSwitch range={range} onRange={onRange} />
    </div>
  );
  if (error)
    return (
      <div className="space-y-2" data-usage-history>
        {head}
        <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
          <span className="flex-1">{error}</span>
          {onReload && (
            <button type="button" onClick={onReload} className="inline-flex items-center gap-1 text-zinc-300 hover:text-zinc-100 underline underline-offset-2">
              <RotateCcw size={11} aria-hidden="true" /> Retry
            </button>
          )}
        </p>
      </div>
    );
  if (!rows)
    return (
      <div className="space-y-2" data-usage-history>
        {head}
        <p className="flex items-center gap-2 text-[11px] text-zinc-500" role="status">
          <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading usage…
        </p>
      </div>
    );
  if (!rows.length)
    return (
      <div className="space-y-2" data-usage-history>
        {head}
        <p className="text-[11px] text-zinc-500" data-usage-empty>
          No AI use in the last {range} days.
        </p>
      </div>
    );
  const trend = trendDays(rows, range, now);
  return (
    <div className="space-y-3" data-usage-history>
      {head}
      <div className="space-y-2">
        <p className="text-[13px] text-zinc-100" data-usage-total>
          <span className="font-medium tabular-nums">{metricText(trend.metric, trend.total)}</span>
          <span className="text-zinc-400"> in the last {range} days</span>
        </p>
        <Trend key={`${range}-${trend.days[trend.days.length - 1].day}`} days={trend.days} metric={trend.metric} now={now} />
      </div>
      {models && <ByModel models={models} metric={trend.metric} engines={engines} />}
      <div className="space-y-2">
        <h4 className="text-[11px] font-medium text-zinc-400">By day</h4>
        <p className="text-[11px] text-zinc-500">
          Fresh tokens · read from cache · cost where the engine reports it. Fresh is new input, cache writes and output: what makes a
          reply heavy.
        </p>
        <ul className="space-y-2" aria-label="Usage per day">
          {groupDays(rows).map((d) => (
            <li key={d.day} className="rounded-lg border border-zinc-800 bg-zinc-950/60">
              <div className="flex items-center gap-2 px-3 py-1.5 border-b border-zinc-800/80">
                <span className="flex-1 min-w-0 truncate text-[11px] font-medium text-zinc-200">{dayLabel(d.day, now)}</span>
                <Numbers fresh={d.fresh} cached={d.cached} cost={d.cost} strong />
              </div>
              <ul className="px-3 py-1">
                {d.bots.map((b) => (
                  <li key={b.kwant_id} className="flex items-center gap-2 py-0.5">
                    <span className="flex-1 min-w-0 truncate text-[11px] text-zinc-400">{b.kwant_name ?? 'Deleted bot'}</span>
                    <Numbers fresh={freshOf(b)} cached={b.cache_read_tokens} cost={b.cost_usd ?? null} />
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
};

/** Loads the last 7 or 30 days while `open`. */
export const DailyUsagePanel: React.FC<{ open: boolean }> = ({ open }) => {
  const [range, setRange] = useState<Range>(7);
  const [rows, setRows] = useState<DailyUsage[] | null>(null);
  const [models, setModels] = useState<ModelTotals[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { engines } = useEngines(open);
  const seq = useRef(0);
  const load = useCallback((days: Range) => {
    const n = ++seq.current;
    setError(null);
    setRows(null);
    modelUsageApi
      .daily(days)
      .then((r) => {
        if (n !== seq.current) return;
        setRows(r.rows);
        setModels(r.models ?? null);
      })
      .catch((e) => n === seq.current && setError(e instanceof Error ? e.message : 'Could not load usage.'));
  }, []);
  useEffect(() => {
    if (open) load(range);
  }, [open, range, load]);
  return (
    <DailyUsageView
      rows={rows}
      models={models}
      range={range}
      onRange={setRange}
      engines={engines}
      error={error}
      onReload={() => load(range)}
    />
  );
};
