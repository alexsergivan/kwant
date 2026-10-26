import React, { useCallback, useEffect, useState } from 'react';
import { Loader2, RotateCcw } from 'lucide-react';
import { DailyUsage, compactTokens, formatCost, freshOf, modelUsageApi } from '../modelUsage';

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

/** Settings → Usage: fresh and cached tokens and cost per day and bot. */
export const DailyUsageView: React.FC<{ rows: DailyUsage[] | null; error?: string | null; onReload?: () => void; now?: Date }> = ({
  rows,
  error,
  onReload,
  now,
}) => {
  if (error)
    return (
      <p role="alert" className="flex items-center gap-2 text-[11px] text-rose-300">
        <span className="flex-1">{error}</span>
        {onReload && (
          <button type="button" onClick={onReload} className="inline-flex items-center gap-1 text-zinc-300 hover:text-zinc-100 underline underline-offset-2">
            <RotateCcw size={11} aria-hidden="true" /> Retry
          </button>
        )}
      </p>
    );
  if (!rows)
    return (
      <p className="flex items-center gap-2 text-[11px] text-zinc-500" role="status">
        <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading usage…
      </p>
    );
  if (!rows.length) return <p className="text-[11px] text-zinc-500">No bot replies in the last two weeks.</p>;
  return (
    <div className="space-y-2">
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
  );
};

/** Loads the last two weeks while `open`. */
export const DailyUsagePanel: React.FC<{ open: boolean }> = ({ open }) => {
  const [rows, setRows] = useState<DailyUsage[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    setError(null);
    setRows(null);
    modelUsageApi
      .daily(14)
      .then((r) => setRows(r.rows))
      .catch((e) => setError(e instanceof Error ? e.message : 'Could not load usage.'));
  }, []);
  useEffect(() => {
    if (open) load();
  }, [open, load]);
  return <DailyUsageView rows={rows} error={error} onReload={load} />;
};
