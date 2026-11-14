import React, { useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, Download, Loader2 } from 'lucide-react';
import { RECEIPT_RANGES, RECEIPTS_OTHER_ERROR, RECEIPTS_TRUNCATED, ReceiptDays, ReceiptsDownload as Saved, downloadReceipts } from '../receipts';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';

/** How long "Saved …" stays before it clears. The truncated and error lines stay until the next try. */
export const RECEIPTS_DONE_MS = 4000;

type State = { kind: 'idle' } | { kind: 'busy' } | { kind: 'done'; saved: Saved } | { kind: 'error'; text: string };

/**
 * The quiet footer of "This week" (MeasuresPanel): download the space's finished tasks as a CSV, with
 * who checked each, how, the verdict and the evidence. `download` is for tests and shots.
 */
export const ReceiptsDownload: React.FC<{ download?: (days: number) => Promise<Saved> }> = ({ download = downloadReceipts }) => {
  const [days, setDays] = useState<ReceiptDays>(30);
  const [state, setState] = useState<State>({ kind: 'idle' });
  const busy = useRef(false);
  const alive = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const selectId = useId();
  const captionId = useId();

  useEffect(
    () => () => {
      alive.current = false;
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const start = async () => {
    // A second click while the file is on its way does nothing (the button stays focusable).
    if (busy.current) return;
    busy.current = true;
    if (timer.current) clearTimeout(timer.current);
    setState({ kind: 'busy' });
    try {
      const saved = await download(days);
      if (!alive.current) return;
      setState({ kind: 'done', saved });
      if (!saved.truncated) timer.current = setTimeout(() => alive.current && setState({ kind: 'idle' }), RECEIPTS_DONE_MS);
    } catch (e) {
      if (!alive.current) return;
      setState({ kind: 'error', text: e instanceof Error && e.message ? e.message : RECEIPTS_OTHER_ERROR });
    } finally {
      busy.current = false;
    }
  };

  const isBusy = state.kind === 'busy';
  return (
    <div className="mt-3 border-t border-zinc-800/70 pt-2.5" data-receipts={state.kind}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={start}
            aria-busy={isBusy}
            aria-describedby={captionId}
            className={`inline-flex items-center gap-1.5 min-h-[40px] sm:min-h-[30px] px-3 rounded-lg text-xs font-medium text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100 ${focusRing}`}
            data-receipts-download
          >
            {isBusy ? (
              <Loader2 size={13} className="shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
            ) : (
              <Download size={13} className="shrink-0" aria-hidden="true" />
            )}
            Download receipts
          </button>
          <label htmlFor={selectId} className="sr-only">
            Receipts from
          </label>
          <select
            id={selectId}
            value={days}
            onChange={(e) => setDays(Number(e.target.value) as ReceiptDays)}
            className="min-h-[40px] sm:min-h-[30px] bg-zinc-950 border border-zinc-800 rounded-lg px-2 py-1 text-xs text-zinc-100 focus:border-sky-500 focus:outline-none"
            data-receipts-range
          >
            {RECEIPT_RANGES.map((r) => (
              <option key={r.days} value={r.days}>
                {r.label}
              </option>
            ))}
          </select>
        </div>
        <p id={captionId} className="min-w-0 text-[11px] leading-4 text-zinc-500">
          CSV of finished tasks: who checked, how, the verdict and the evidence.
        </p>
      </div>
      <div aria-live="polite" className="empty:hidden mt-1.5 space-y-1 text-[11px] leading-4" data-receipts-status>
        {state.kind === 'done' && (
          <p className="text-zinc-400 break-all" data-receipts-saved>
            Saved {state.saved.name}
          </p>
        )}
        {state.kind === 'done' && state.saved.truncated && (
          <p className="flex items-start gap-1 text-amber-300" data-receipts-truncated>
            <AlertTriangle size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
            <span>{RECEIPTS_TRUNCATED}</span>
          </p>
        )}
        {state.kind === 'error' && (
          <p className="flex items-start gap-1 text-rose-300" data-receipts-error>
            <AlertTriangle size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
            <span>{state.text}</span>
          </p>
        )}
      </div>
    </div>
  );
};
