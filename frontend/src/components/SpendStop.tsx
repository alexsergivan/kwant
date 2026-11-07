import React, { useEffect, useRef, useState } from 'react';
import { Check, Gauge, Loader2, Play, Settings as SettingsIcon } from 'lucide-react';
import {
  SpendStoppedItem,
  SpendStoppedMeta,
  UsageToday,
  fetchSpendStopped,
  spendStopApi,
  stopTokensError,
  stopUsdError,
  stoppedDetail,
  takeSpendStopFocus,
  todayLine,
  todayState,
} from '../spendStop';

type ContinueState = 'idle' | 'sending' | 'done' | 'error';

const btnAmber =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer disabled:cursor-default text-amber-100 bg-amber-500/15 border-amber-500/40 hover:bg-amber-500/25 disabled:opacity-70 disabled:hover:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400';
const btnQuiet =
  'shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border border-zinc-700 text-zinc-300 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500';

/** Continue today, once: a second click before the re-render can't post twice. */
function useContinueToday(onContinue: () => Promise<unknown>, initial: ContinueState = 'idle') {
  const [state, setState] = useState<ContinueState>(initial);
  const [error, setError] = useState<string | null>(null);
  const lock = useRef(initial === 'done');
  useEffect(() => {
    if (initial === 'done') {
      lock.current = true;
      setState('done');
    }
  }, [initial]);
  const run = async () => {
    if (lock.current) return;
    lock.current = true;
    setState('sending');
    setError(null);
    try {
      await onContinue();
      setState('done');
    } catch (e) {
      lock.current = false;
      setState('error');
      setError(e instanceof Error && e.message ? e.message : 'Couldn’t continue. Try again.');
    }
  };
  return { state, error, run };
}

/** The body the chat notice and the Needs-you item share: the server's line, the limit, Continue today. */
const StopBody: React.FC<{
  headingId: string;
  text: string;
  meta: Pick<SpendStoppedMeta, 'over' | 'used_tokens' | 'stop_tokens' | 'used_usd' | 'stop_usd'>;
  /** Without it (a read-only view) there's no button. */
  onContinue?: () => Promise<unknown>;
  /** Already continued (or the day is over) when it loads. */
  done?: boolean;
  onOpenSettings?: () => void;
}> = ({ headingId, text, meta, onContinue, done: doneAlready = false, onOpenSettings }) => {
  const { state, error, run } = useContinueToday(onContinue ?? (async () => undefined), doneAlready ? 'done' : 'idle');
  const done = state === 'done';
  const detail = stoppedDetail(meta);
  return (
    <>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="min-w-0 flex-1 basis-56">
          <p id={headingId} className="flex items-start gap-2 text-sm text-amber-200 break-words">
            <Gauge size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
            <span className="min-w-0">{text}</span>
          </p>
          {detail && <p className="mt-1 pl-[23px] text-[11px] text-amber-200/60 font-mono">{detail}</p>}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {onOpenSettings && (
            <button type="button" onClick={onOpenSettings} className={btnQuiet} data-spend-settings>
              <SettingsIcon size={13} aria-hidden="true" />
              <span>Change the limit</span>
            </button>
          )}
          {onContinue && (
            <button type="button" onClick={run} disabled={state === 'sending' || done} aria-describedby={headingId} className={btnAmber} data-spend-continue>
              {state === 'sending' ? (
                <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              ) : done ? (
                <Check size={13} aria-hidden="true" />
              ) : (
                <Play size={13} aria-hidden="true" />
              )}
              <span>{state === 'sending' ? 'Continuing…' : done ? 'Continued for today' : 'Continue today'}</span>
            </button>
          )}
        </div>
      </div>
      <p className="sr-only" role="status" aria-live="polite">
        {done && onContinue ? 'Continued for today. Bots can start new work again.' : ''}
      </p>
      {error && (
        <p role="alert" className="mt-2 pl-[23px] text-[11px] text-rose-300">
          {error}
        </p>
      )}
    </>
  );
};

// Today's state, shared by every notice on the page: one request, kept 20 s.
let todayCache: { at: number; p: Promise<UsageToday | null> } | null = null;
function todayOnce(api: ReturnType<typeof spendStopApi>): Promise<UsageToday | null> {
  if (todayCache && Date.now() - todayCache.at < 20_000) return todayCache.p;
  const p = api.today().catch(() => null);
  todayCache = { at: Date.now(), p };
  return p;
}
export const forgetToday = () => {
  todayCache = null;
};

export interface SpendStoppedNoticeProps {
  messageId: string;
  text: string;
  meta: SpendStoppedMeta;
  /** False in a read-only view: no Continue button. */
  canContinue?: boolean;
  onOpenSettings?: () => void;
  flash?: boolean;
  send?: typeof fetch;
}

/** The chat's notice: today's stop held the message. Continue today lifts it until midnight. */
export const SpendStoppedNotice: React.FC<SpendStoppedNoticeProps> = ({ messageId, text, meta, canContinue = true, onOpenSettings, flash, send }) => {
  const api = useRef(spendStopApi(send)).current;
  // Continued today already: the button reads so. An older day's notice, or a stop a raised limit
  // lifted: no button, the line stays as a record.
  const [settled, setSettled] = useState<'continued' | 'over' | null>(null);
  useEffect(() => {
    let live = true;
    todayOnce(api).then((u) => {
      if (!live || !u || u.stopped) return;
      setSettled(u.day === meta.day && u.continued ? 'continued' : 'over');
    });
    return () => {
      live = false;
    };
  }, [api, meta.day]);
  return (
    <section
      data-message-id={messageId}
      data-spend-stopped
      aria-labelledby={`spend-${messageId}`}
      className={`w-full max-w-5xl rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4 ${flash ? 'kw-flash' : ''}`}
    >
      <StopBody
        headingId={`spend-${messageId}`}
        text={text}
        meta={meta}
        done={settled === 'continued'}
        onContinue={canContinue && settled !== 'over' ? () => api.continueToday().finally(forgetToday) : undefined}
        onOpenSettings={onOpenSettings}
      />
    </section>
  );
};

/** Needs you: the stop's waiting item, first, with the same Continue today. Nothing when no stop holds work. */
export const SpendStopNeedsYou: React.FC<{ send?: typeof fetch; onOpenSettings?: () => void; refreshKey?: unknown }> = ({ send, onOpenSettings, refreshKey }) => {
  const api = useRef(spendStopApi(send)).current;
  const [item, setItem] = useState<SpendStoppedItem | null>(null);
  const box = useRef<HTMLElement>(null);
  const [focusSeq, setFocusSeq] = useState(0);
  useEffect(() => {
    let live = true;
    fetchSpendStopped(send).then((it) => {
      if (!live) return;
      if (it) setItem(it);
      // Opened from the notification: bring the item into view and put focus on Continue today
      if (takeSpendStopFocus() && it) setFocusSeq((n) => n + 1);
    });
    return () => {
      live = false;
    };
  }, [send, refreshKey]);
  useEffect(() => {
    if (!focusSeq || !box.current) return;
    box.current.scrollIntoView?.({ block: 'start', behavior: 'auto' });
    box.current.querySelector<HTMLElement>('[data-spend-continue]')?.focus({ preventScroll: true });
  }, [focusSeq]);
  if (!item) return null;
  return (
    <section ref={box} aria-labelledby="needs-spend-h" data-needs-spend-stop>
      <h3 id="needs-spend-h" className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">
        <Gauge size={12} aria-hidden="true" />
        Needs you
      </h3>
      <div className="rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4">
        <StopBody
          headingId="needs-spend-text"
          text={item.text || 'Today’s spending stop is on.'}
          meta={item}
          onContinue={() => api.continueToday().finally(forgetToday)}
          onOpenSettings={onOpenSettings}
        />
      </div>
    </section>
  );
};

// ---------------------------------------------------------------- Settings › All spaces › Usage

export interface StopFields {
  tokens: string;
  usd: string;
}

const field = (err: boolean) =>
  `w-full bg-zinc-950 border rounded-lg px-3 py-2 min-h-[40px] sm:min-h-[36px] text-xs text-zinc-100 focus:outline-none ${err ? 'border-rose-500/60 focus:border-rose-400' : 'border-zinc-800 focus:border-sky-500'}`;

/** Two optional daily limits; empty is off. Today's use from the server sits beside them. */
export const SpendStopFields: React.FC<{
  value: StopFields;
  onChange: (v: StopFields) => void;
  open: boolean;
  send?: typeof fetch;
  /** Bumped after a save, so today's line re-reads (a raised limit lifts the stop). */
  savedKey?: unknown;
}> = ({ value, onChange, open, send, savedKey }) => {
  const api = useRef(spendStopApi(send)).current;
  const [today, setToday] = useState<UsageToday | null>(null);
  const [todayError, setTodayError] = useState(false);
  useEffect(() => {
    if (!open) return;
    let live = true;
    api.today().then(
      (u) => live && (setToday(u), setTodayError(false)),
      () => live && setTodayError(true),
    );
    return () => {
      live = false;
    };
  }, [api, open, savedKey]);
  const tErr = stopTokensError(value.tokens);
  const uErr = stopUsdError(value.usd);
  const state = today ? todayState(today) : null;
  return (
    <div className="space-y-2" role="group" aria-labelledby="spend-stop-h" data-spend-stop>
      <div>
        <p id="spend-stop-h" className="text-[11px] font-semibold text-zinc-200">
          Daily spending stop
        </p>
        <p className="text-[11px] text-zinc-500">
          When today’s use reaches either limit, bots don’t start new work until tomorrow or until you continue. Leave both empty for no stop.
        </p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1 min-w-0">
          <label htmlFor="spend-stop-tokens" className="block text-[11px] font-medium text-zinc-300">
            Tokens per day
          </label>
          <input
            id="spend-stop-tokens"
            data-switch="spend-stop"
            type="text"
            inputMode="decimal"
            autoComplete="off"
            placeholder="No limit"
            value={value.tokens}
            onChange={(e) => onChange({ ...value, tokens: e.target.value })}
            aria-invalid={!!tErr}
            aria-describedby="spend-stop-tokens-hint"
            className={field(!!tErr)}
          />
          <p id="spend-stop-tokens-hint" className={`text-[11px] ${tErr ? 'text-rose-300' : 'text-zinc-500'}`}>
            {tErr ?? 'Like 500k or 2M. Cache reads don’t count.'}
          </p>
        </div>
        <div className="space-y-1 min-w-0">
          <label htmlFor="spend-stop-usd" className="block text-[11px] font-medium text-zinc-300">
            Dollars per day
          </label>
          <input
            id="spend-stop-usd"
            type="text"
            inputMode="decimal"
            autoComplete="off"
            placeholder="No limit"
            value={value.usd}
            onChange={(e) => onChange({ ...value, usd: e.target.value })}
            aria-invalid={!!uErr}
            aria-describedby="spend-stop-usd-hint"
            className={field(!!uErr)}
          />
          <p id="spend-stop-usd-hint" className={`text-[11px] ${uErr ? 'text-rose-300' : 'text-zinc-500'}`}>
            {uErr ?? 'Like 5 or 2.50. Counts only engines that report a cost.'}
          </p>
        </div>
      </div>
      <div role="status" aria-live="polite" className="text-[11px]" data-spend-today>
        {today ? (
          <>
            <p className="font-mono text-zinc-300">{todayLine(today)}</p>
            {state && <p className={today.stopped ? 'text-amber-300' : 'text-zinc-500'}>{state}</p>}
          </>
        ) : todayError ? (
          <p className="text-zinc-500">Couldn’t load today’s use.</p>
        ) : null}
      </div>
    </div>
  );
};
