import React, { useCallback, useEffect, useState } from 'react';
import { Loader2, RefreshCw } from 'lucide-react';
import { formatWhen } from '../continuity';
import { ModelUsage, UsageSums, cacheShare, compactTokens, modelUsageApi, purposeLabel } from '../modelUsage';

const quietBtn =
  'inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 border-zinc-700/70 bg-zinc-800/70 text-zinc-200 hover:bg-zinc-700';

const KINDS: Array<{ key: keyof UsageSums; label: string; hint: string }> = [
  { key: 'fresh_input_tokens', label: 'Fresh input', hint: 'Input tokens read at full price' },
  { key: 'cache_read_tokens', label: 'Cache read', hint: 'Input tokens served from the cache' },
  { key: 'cache_write_tokens', label: 'Cache write', hint: 'Input tokens written to the cache' },
  { key: 'output_tokens', label: 'Output', hint: 'Tokens the model wrote' },
];

/** Calls and tokens line, e.g. "fresh 40 · cache read 1.5k · cache write 0 · out 12". */
const tokenLine = (s: Pick<UsageSums, 'fresh_input_tokens' | 'cache_read_tokens' | 'cache_write_tokens' | 'output_tokens'>) =>
  `fresh ${compactTokens(s.fresh_input_tokens)} · cache read ${compactTokens(s.cache_read_tokens)} · cache write ${compactTokens(
    s.cache_write_tokens,
  )} · out ${compactTokens(s.output_tokens)}`;

/** What the Usage tab shows; `usage` null while loading. */
export const UsageView: React.FC<{
  kwantName: string;
  usage: ModelUsage | null;
  error: string | null;
  onReload: () => void;
}> = ({ kwantName, usage, error, onReload }) => {
  if (error)
    return (
      <div className="flex items-center justify-between gap-2 bg-rose-500/10 border border-rose-500/20 text-rose-300 px-3 py-2 rounded-lg text-xs" role="alert">
        <span>{error}</span>
        <button type="button" onClick={onReload} className={quietBtn}>
          <RefreshCw size={12} /> Retry
        </button>
      </div>
    );
  if (!usage)
    return (
      <p className="flex items-center gap-2 text-xs text-zinc-400" role="status">
        <Loader2 size={14} className="animate-spin motion-reduce:animate-none" /> Loading usage…
      </p>
    );
  const t = usage.totals;
  if (t.calls === 0)
    return (
      <p className="text-xs text-zinc-500 border border-dashed border-zinc-800 rounded-lg p-4 text-center" data-usage-empty>
        No model calls recorded for {kwantName} yet. Calls are counted here for bots on an API key or a custom API; Claude, Codex and Grok
        sign-ins report their usage to their own accounts.
      </p>
    );
  const share = cacheShare(t);
  return (
    <div className="space-y-4" data-usage>
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-zinc-400">
          {t.calls} {t.calls === 1 ? 'call' : 'calls'}
          {share !== null && `, ${share}% of input from the cache`}
        </p>
        <button type="button" onClick={onReload} className={quietBtn} aria-label="Refresh usage">
          <RefreshCw size={12} /> Refresh
        </button>
      </div>
      <dl className="grid grid-cols-2 sm:grid-cols-4 gap-2">
        {KINDS.map(({ key, label, hint }) => (
          <div key={key} className="bg-zinc-950 border border-zinc-800 rounded-lg px-3 py-2" title={hint}>
            <dt className="text-[10px] uppercase tracking-wide text-zinc-500">{label}</dt>
            <dd className="text-base font-semibold text-zinc-100 tabular-nums" data-kind={key}>
              {compactTokens(t[key])}
            </dd>
          </div>
        ))}
      </dl>

      {usage.by_purpose.length > 0 && (
        <section aria-labelledby="usage-by-purpose">
          <h3 id="usage-by-purpose" className="text-[11px] font-medium text-zinc-400 mb-1.5">
            By purpose
          </h3>
          <ul className="space-y-1">
            {usage.by_purpose.map((p) => (
              <li key={p.purpose} className="text-xs text-zinc-300 flex flex-wrap gap-x-2">
                <span className="font-medium text-zinc-200">
                  {purposeLabel(p.purpose)} ({p.calls})
                </span>
                <span className="text-zinc-500 tabular-nums">{tokenLine(p)}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {usage.recent.length > 0 && (
        <section aria-labelledby="usage-recent">
          <h3 id="usage-recent" className="text-[11px] font-medium text-zinc-400 mb-1.5">
            Recent calls
          </h3>
          <ul className="divide-y divide-zinc-800/70 border border-zinc-800 rounded-lg">
            {usage.recent.map((c) => (
              <li key={c.id} className="px-3 py-1.5 text-xs" data-call={c.purpose}>
                <div className="flex flex-wrap items-baseline gap-x-2">
                  <span className="font-medium text-zinc-200">{purposeLabel(c.purpose, true)}</span>
                  <span className="text-zinc-500 break-all">{c.model}</span>
                  <span className="ml-auto text-[10px] text-zinc-500">{formatWhen(c.created_at)}</span>
                </div>
                <div className="text-zinc-500 tabular-nums">{c.reported ? tokenLine(c) : 'The provider sent no token counts for this call.'}</div>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
};

/** Usage tab of the bot settings: loads `/model-usage` for one bot. */
export const UsagePanel: React.FC<{ kwantId: string; kwantName: string }> = ({ kwantId, kwantName }) => {
  const [usage, setUsage] = useState<ModelUsage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    setError(null);
    setUsage(null);
    modelUsageApi
      .get(kwantId)
      .then(setUsage)
      .catch(() => setError("Couldn't load usage."));
  }, [kwantId]);
  useEffect(load, [load]);
  return <UsageView kwantName={kwantName} usage={usage} error={error} onReload={load} />;
};
