import React, { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, CheckCircle2, ChevronLeft, Inbox, Loader2, RefreshCw } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { ModernApprovalCard } from './ModernApprovalCard';
import { approvalFromAway, lateAnswerText } from '../approvals';
import {
  AWAY_SECTIONS,
  ApprovalDecision,
  AwayError,
  AwayItem,
  AwayResponse,
  AwayNotice,
  AwaySection,
  Tone,
  actionFailureNotice,
  awayApi,
  isAwayEmpty,
  itemLabel,
  relativeTime,
  windowLabel,
  withoutItem,
} from '../away';

const TONES: Record<Tone, string> = {
  amber: 'border-amber-500/30 bg-amber-500/10 text-amber-200',
  emerald: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  rose: 'border-rose-500/30 bg-rose-500/10 text-rose-200',
  sky: 'border-sky-500/30 bg-sky-500/10 text-sky-200',
  zinc: 'border-zinc-700 bg-zinc-800 text-zinc-300',
};

const SECTION_ICON: Record<AwaySection, React.ReactNode> = {
  waiting: <Inbox size={13} className="text-amber-300" aria-hidden="true" />,
  finished: <CheckCircle2 size={13} className="text-emerald-400" aria-hidden="true" />,
  failed: <AlertTriangle size={13} className="text-rose-400" aria-hidden="true" />,
};

export interface AwayViewProps {
  data: AwayResponse | null;
  loading: boolean;
  error: string | null;
  /** Outcome of the last one-tap action (e.g. "This request expired"); the item may be gone by now. */
  notice?: AwayNotice | null;
  /** Item id being acted on. */
  busy?: string | null;
  now?: number;
  onRefresh: () => void;
  /** Phone only: back to the chat list, since the page hides the tab bar. */
  onBack?: () => void;
  onOpen: (item: AwayItem) => void;
  onAnswer: (item: AwayItem, choice: number) => void;
  onResolve: (item: AwayItem, decision: ApprovalDecision) => void;
}

const actionBtn =
  'inline-flex items-center gap-1.5 min-h-[36px] sm:min-h-[30px] px-3 py-1 rounded-lg text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400';

/** The "While you were away" page: what's waiting on you, what finished and what failed. */
export const AwayView: React.FC<AwayViewProps> = ({ data, loading, error, notice = null, busy = null, now = Date.now(), onRefresh, onBack, onOpen, onAnswer, onResolve }) => {
  const renderItem = (item: AwayItem) => {
    const { label, tone } = itemLabel(item);
    const when = relativeTime(item.timestamp, now);
    const disabled = busy !== null;
    return (
      <li key={`${item.kind}:${item.id}`} className="rounded-xl border border-zinc-800 bg-zinc-900/60 hover:border-zinc-700 transition-colors">
        <button
          type="button"
          onClick={() => onOpen(item)}
          className="w-full flex items-start gap-3 p-3 text-left rounded-xl focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          aria-label={`${item.title}, ${label}, from ${item.kwant_name}, ${when}. Open the chat.`}
        >
          <ModernAvatar role={item.kwant_name} name={item.kwant_name} size={30} kwantId={item.kwant_id} />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 min-w-0 text-[11px]">
              <span className="font-semibold text-zinc-200 truncate">{item.kwant_name}</span>
              <span className={`shrink-0 px-1.5 py-px rounded-full border ${TONES[tone]}`}>{label}</span>
              <time dateTime={item.timestamp} title={new Date(item.timestamp).toLocaleString()} className="ml-auto shrink-0 text-zinc-500">
                {when}
              </time>
            </div>
            <p className="mt-0.5 text-sm font-medium text-zinc-100 truncate">{item.title}</p>
            {/* A question's summary just lists its choices, which are buttons below. */}
            {item.summary && item.kind !== 'approval' && !(item.kind === 'question' && item.choices?.length) && <p className="text-xs text-zinc-400 truncate">{item.summary}</p>}
          </div>
        </button>

        {item.kind === 'question' && (item.choices?.length ?? 0) > 0 && (
          <div role="group" aria-label={`Answer: ${item.title}`} className="px-3 pb-3 sm:pl-[54px] flex flex-wrap gap-2">
            {item.choices!.map((c, i) => (
              <button
                key={i}
                type="button"
                disabled={disabled}
                onClick={() => onAnswer(item, i)}
                className={`${actionBtn} border border-zinc-700 bg-zinc-800/80 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600`}
              >
                {c}
              </button>
            ))}
          </div>
        )}

        {item.kind === 'approval' && (
          <div className="px-3 pb-1 sm:pl-[54px] [&>div]:my-0 [&>div]:mb-2">
            <ModernApprovalCard approval={approvalFromAway(item)} disabled={disabled} onResolve={(_, decision) => onResolve(item, decision)} />
          </div>
        )}

        {busy === item.id && (
          <p className="px-3 pb-3 sm:pl-[54px] inline-flex items-center gap-1.5 text-[11px] text-zinc-400">
            <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Sending…
          </p>
        )}
      </li>
    );
  };

  const empty = isAwayEmpty(data);

  return (
    <div className="flex-1 flex flex-col h-full bg-zinc-950 overflow-hidden">
      <div className={`h-14 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`}>
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            aria-label="Back to chats"
            className="shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <ChevronLeft size={18} />
          </button>
        )}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">While you were away</h2>
          <p className="text-[11px] text-zinc-400 truncate mt-0.5">
            {data ? windowLabel(data.since, now) : 'What your bots need, finished and failed'}
          </p>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={loading}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <RefreshCw size={12} className={loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          Refresh
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-4 sm:p-6" aria-busy={loading}>
        <div className="max-w-3xl mx-auto space-y-6">
          {error && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-500/20 bg-rose-500/5 flex items-center gap-2 text-xs text-rose-300">
              <span>Couldn't load what happened: {error}</span>
              <button
                type="button"
                onClick={onRefresh}
                className="ml-auto px-2 py-0.5 rounded border border-rose-500/30 hover:bg-rose-500/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-500"
              >
                Retry
              </button>
            </div>
          )}

          <div role="status" aria-live="polite">
            {notice && (
              <p
                className={`px-3 py-2 rounded-lg border text-xs ${
                  notice.error
                    ? 'border-amber-500/30 bg-amber-500/10 text-amber-200'
                    : notice.info
                      ? 'border-slate-500/30 bg-slate-500/10 text-slate-200'
                      : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'
                }`}
              >
                {notice.text}
              </p>
            )}
          </div>

          {!data && loading && (
            <p className="flex items-center gap-2 text-xs text-zinc-400">
              <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
            </p>
          )}

          {data && empty && (
            <div className="py-16 flex flex-col items-center text-center gap-2">
              <CheckCircle2 size={28} className="text-emerald-400" aria-hidden="true" />
              <p className="text-sm font-medium text-zinc-200">You're all caught up</p>
              <p className="text-xs text-zinc-500 max-w-xs">Nothing new since you last looked. Questions, approvals and finished work will show up here.</p>
            </div>
          )}

          {data &&
            !empty &&
            AWAY_SECTIONS.map(({ key, label, empty: none }) => (
              <section key={key} aria-labelledby={`away-${key}`}>
                <h3 id={`away-${key}`} className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
                  {SECTION_ICON[key]}
                  {label}
                  <span className="font-mono normal-case text-zinc-600">({data[key].length})</span>
                </h3>
                {data[key].length > 0 ? (
                  <ul className="space-y-2">{data[key].map(renderItem)}</ul>
                ) : (
                  <p className="px-1 text-xs text-zinc-500">{none}</p>
                )}
              </section>
            ))}
        </div>
      </div>
    </div>
  );
};

export interface AwayPageProps {
  /** Window start kept from the first visit this session, so coming back from an item keeps the list. */
  since?: string | null;
  onSince?: (since: string) => void;
  onOpen: (item: AwayItem) => void;
  /** New unseen counts for the sidebar badge. */
  onCounts: (total: number) => void;
  /** An approval was answered here, so the chat drops its card too. */
  onApprovalResolved?: (id: string) => void;
  onBack?: () => void;
}

/** Loads the page, marks it seen, and runs the one-tap actions. */
export const AwayPage: React.FC<AwayPageProps> = ({ since = null, onSince, onOpen, onCounts, onApprovalResolved, onBack }) => {
  const [data, setData] = useState<AwayResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<AwayNotice | null>(null);
  // Refreshes keep the first load's window, so marking it seen doesn't empty the page.
  const sinceRef = useRef<string | null>(since);
  const onCountsRef = useRef(onCounts);
  onCountsRef.current = onCounts;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await awayApi.get(sinceRef.current ?? undefined);
      if (!sinceRef.current) {
        sinceRef.current = next.since;
        onSince?.(next.since);
      }
      setData(next);
      setError(null);
      return next;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Something went wrong.');
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    load().then(async (first) => {
      if (cancelled || !first) return;
      try {
        await awayApi.seen();
        // Finished and failed are now seen; what still waits on you keeps the badge.
        onCountsRef.current(first.unseen.waiting);
      } catch {
        // the badge just stays until the next visit
      }
    });
    return () => {
      cancelled = true;
    };
  }, [load]);

  const act = async (item: AwayItem, run: () => Promise<unknown>, denied = false) => {
    if (busy) return;
    setBusy(item.id);
    setNotice(null);
    try {
      const reply = await run();
      setData((d) => (d ? withoutItem(d, 'waiting', item.id) : d));
      const late = item.kind === 'approval' && !denied ? lateAnswerText(reply, item.kwant_name) : null;
      const done = denied ? 'Denied, nothing ran:' : late ?? `Sent to ${item.kwant_name}:`;
      setNotice({ text: `${done} “${item.title}”`, error: false });
    } catch (err) {
      const status = err instanceof AwayError ? err.status : 0;
      setNotice(actionFailureNotice(status, err instanceof Error ? err.message : 'Something went wrong.', item.title));
    } finally {
      setBusy(null);
    }
    const next = await load();
    if (next) onCountsRef.current(next.unseen.total);
  };

  return (
    <AwayView
      data={data}
      loading={loading}
      error={error}
      busy={busy}
      notice={notice}
      onRefresh={() => load().then((next) => next && onCountsRef.current(next.unseen.total))}
      onBack={onBack}
      onOpen={onOpen}
      onAnswer={(item, choice) => act(item, () => awayApi.answer(item.id, choice))}
      onResolve={(item, decision) =>
        act(item, async () => {
          try {
            const res = await awayApi.resolve(item.id, decision);
            onApprovalResolved?.(item.id);
            return res;
          } catch (err) {
            // Lapsed meanwhile: the chat's card is stale too.
            if (err instanceof AwayError && (err.status === 409 || err.status === 404)) onApprovalResolved?.(item.id);
            throw err;
          }
        }, decision === 'deny')
      }
    />
  );
};
