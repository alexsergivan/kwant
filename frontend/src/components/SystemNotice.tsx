import React, { useEffect, useRef, useState } from 'react';
import { AlertTriangle, Check, Gauge, Info, Loader2, PauseCircle, Play, RotateCcw } from 'lucide-react';
import { BudgetPausedMeta, HandoffCappedMeta, handoffCappedText, isAlreadyContinued, isChainGone } from '../chainBudget';
import { compactTokens } from '../modelUsage';
import { Interruption, RetryState, RetryTarget, interruptionText, runRetryOnce } from '../interruptions';

/** A note kwnt itself wrote (e.g. "Growth wasn't asked: …"): a quiet line, not a bot's reply. */
export const SystemNotice: React.FC<{ messageId: string; text: string; flash?: boolean }> = ({ messageId, text, flash }) => (
  <div
    data-message-id={messageId}
    data-system-notice
    role="note"
    className={`w-full max-w-5xl flex justify-center ${flash ? 'kw-flash rounded-xl' : ''}`}
  >
    <p className="flex items-start gap-1.5 max-w-full px-3 py-1.5 rounded-full text-xs text-zinc-400 bg-zinc-900/60 border border-zinc-800 break-words">
      <Info size={12} className="mt-0.5 shrink-0 text-zinc-500" aria-hidden="true" />
      <span className="min-w-0">
        <span className="sr-only">Notice: </span>
        {text}
      </span>
    </p>
  </div>
);

export interface InterruptedNoticeProps {
  messageId: string;
  interruption: Interruption;
  /** Set only on the newest notice the person hasn't moved past; without it there's no Retry. */
  retry?: RetryTarget | null;
  onRetry?: (target: RetryTarget) => void | Promise<unknown>;
  flash?: boolean;
}

/** A reply that ended without an answer (restart or stall), with Retry for the person's message. */
export const InterruptedNotice: React.FC<InterruptedNoticeProps> = ({ messageId, interruption, retry, onRetry, flash }) => {
  const [state, setState] = useState<RetryState>('idle');
  // Set synchronously, so a second click before the re-render can't send twice.
  const lock = useRef<RetryState>('idle');
  const headingId = `intr-${messageId}`;
  const canRetry = !!retry && !!onRetry;

  const run = () => {
    if (retry && onRetry) void runRetryOnce(lock, retry, onRetry, setState);
  };

  return (
    <section
      data-message-id={messageId}
      data-interrupted={interruption.kind}
      aria-labelledby={headingId}
      className={`w-full max-w-5xl rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4 ${flash ? 'kw-flash' : ''}`}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <p id={headingId} className="flex items-start gap-2 min-w-0 flex-1 basis-56 text-sm text-amber-200 break-words">
          <AlertTriangle size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
          <span className="min-w-0">
            {interruptionText(interruption)}
            {canRetry && state !== 'sent' && <span className="text-amber-200/70"> Retry sends your message again.</span>}
          </span>
        </p>
        {canRetry && (
          <button
            type="button"
            onClick={run}
            disabled={state !== 'idle'}
            aria-describedby={headingId}
            className="shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer disabled:cursor-default text-amber-100 bg-amber-500/15 border-amber-500/40 hover:bg-amber-500/25 disabled:opacity-70 disabled:hover:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
          >
            {state === 'sending' ? (
              <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            ) : state === 'sent' ? (
              <Check size={13} aria-hidden="true" />
            ) : (
              <RotateCcw size={13} aria-hidden="true" />
            )}
            <span>{state === 'sending' ? 'Sending…' : state === 'sent' ? 'Sent again' : 'Retry'}</span>
          </button>
        )}
      </div>
    </section>
  );
};

export interface BudgetPausedNoticeProps {
  messageId: string;
  text: string;
  meta: BudgetPausedMeta;
  /** Carries the chain on; without it (a read-only view) there's no button. */
  onContinue?: (noticeId: string) => Promise<unknown>;
  flash?: boolean;
}

/** A chain that went over its token budget: who was waiting, and Continue to let them go on. */
export const BudgetPausedNotice: React.FC<BudgetPausedNoticeProps> = ({ messageId, text, meta, onContinue, flash }) => {
  const [state, setState] = useState<'idle' | 'sending' | 'sent' | 'error'>(meta.resumed ? 'sent' : 'idle');
  const [error, setError] = useState<string | null>(null);
  const lock = useRef(false);
  const headingId = `budget-${messageId}`;
  const run = async () => {
    if (!onContinue || lock.current) return;
    lock.current = true;
    setState('sending');
    setError(null);
    try {
      await onContinue(messageId);
      setState('sent');
    } catch (e) {
      lock.current = false;
      setState('error');
      setError(e instanceof Error ? e.message : 'Could not continue.');
    }
  };
  const done = state === 'sent';
  return (
    <section
      data-message-id={messageId}
      data-budget-paused
      aria-labelledby={headingId}
      className={`w-full max-w-5xl rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 sm:px-4 ${flash ? 'kw-flash' : ''}`}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="min-w-0 flex-1 basis-56">
          <p id={headingId} className="flex items-start gap-2 text-sm text-amber-200 break-words">
            <Gauge size={15} className="mt-0.5 shrink-0 text-amber-400" aria-hidden="true" />
            <span className="min-w-0">{text}</span>
          </p>
          <p className="mt-1 pl-[23px] text-[11px] text-amber-200/60 font-mono">
            {compactTokens(meta.used)} of {compactTokens(meta.budget)} fresh tokens
          </p>
        </div>
        {onContinue && (
          <button
            type="button"
            onClick={run}
            disabled={state === 'sending' || done}
            aria-describedby={headingId}
            className="shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[36px] px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer disabled:cursor-default text-amber-100 bg-amber-500/15 border-amber-500/40 hover:bg-amber-500/25 disabled:opacity-70 disabled:hover:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-400"
          >
            {state === 'sending' ? (
              <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            ) : done ? (
              <Check size={13} aria-hidden="true" />
            ) : (
              <Play size={13} aria-hidden="true" />
            )}
            <span>{state === 'sending' ? 'Continuing…' : done ? 'Continued' : 'Continue'}</span>
          </button>
        )}
      </div>
      {error && (
        <p role="alert" className="mt-2 pl-[23px] text-[11px] text-rose-300">
          {error}
        </p>
      )}
    </section>
  );
};

export interface HandoffCappedNoticeProps {
  messageId: string;
  meta: HandoffCappedMeta;
  /** Hands the turn to the waiting teammates; without it (a read-only view) there's no button. */
  onContinue?: (noticeId: string) => Promise<unknown>;
  flash?: boolean;
}

/** The team paused at the hand-off cap: one line and a one-click Continue, so nobody has to type a new message. */
export const HandoffCappedNotice: React.FC<HandoffCappedNoticeProps> = ({ messageId, meta, onContinue, flash }) => {
  // sent: continued here or on load; already: a 409, someone else got there first; gone: a 410, the lead was removed.
  const [state, setState] = useState<'idle' | 'sending' | 'sent' | 'already' | 'gone' | 'error'>(meta.resumed ? 'sent' : 'idle');
  const [error, setError] = useState<string | null>(null);
  const lock = useRef(false);
  const headingId = `capped-${messageId}`;
  // A typed "continue" resumes it too; the refreshed message then arrives with resumed set.
  useEffect(() => {
    if (meta.resumed) setState((s) => (s === 'already' ? s : 'sent'));
  }, [meta.resumed]);
  const done = state === 'sent' || state === 'already';
  const stopped = done || state === 'gone';
  const text = handoffCappedText({ ...meta, resumed: stopped });
  const headline = text.headline;
  const detail = state === 'already' ? 'Already continued.' : state === 'gone' ? null : text.detail;
  const run = async () => {
    if (!onContinue || lock.current) return;
    lock.current = true;
    setState('sending');
    setError(null);
    try {
      await onContinue(messageId);
      setState('sent');
    } catch (e) {
      // Typing "continue" (or another tab) got there first: that's the outcome the button wanted.
      if (isAlreadyContinued(e)) return setState('already');
      if (isChainGone(e)) {
        setState('gone');
        return setError("Can't continue: the lead of this chat was removed. Send a new message to start again.");
      }
      lock.current = false;
      setState('error');
      const why = e instanceof Error && e.message ? e.message : 'Could not continue.';
      setError(why.charAt(0).toUpperCase() + why.slice(1));
    }
  };
  return (
    <section
      data-message-id={messageId}
      data-handoff-capped
      data-resumed={done || undefined}
      data-state={state}
      aria-labelledby={headingId}
      className={`w-full max-w-5xl rounded-xl border px-3.5 py-2.5 sm:px-4 ${done ? 'border-zinc-800 bg-zinc-900/40' : 'border-sky-500/30 bg-sky-950/20'} ${flash ? 'kw-flash' : ''}`}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="min-w-0 flex-1 basis-56">
          <p id={headingId} className={`flex items-start gap-2 text-sm break-words ${done ? 'text-zinc-400' : 'text-sky-100'}`}>
            {done ? (
              <Check size={15} className="mt-0.5 shrink-0 text-emerald-400" aria-hidden="true" />
            ) : (
              <PauseCircle size={15} className="mt-0.5 shrink-0 text-sky-400" aria-hidden="true" />
            )}
            <span className="min-w-0">{headline}</span>
          </p>
          {detail && <p className="mt-1 pl-[23px] text-[11px] text-zinc-400 break-words">{detail}</p>}
        </div>
        {onContinue && !stopped && (
          <button
            type="button"
            onClick={run}
            disabled={state === 'sending'}
            aria-describedby={headingId}
            data-handoff-continue
            className="shrink-0 inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[36px] px-3.5 py-1.5 rounded-lg text-xs font-semibold transition-colors cursor-pointer disabled:cursor-default text-white bg-sky-600 hover:bg-sky-500 disabled:opacity-70 disabled:hover:bg-sky-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400"
          >
            {state === 'sending' ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Play size={13} aria-hidden="true" />}
            <span>{state === 'sending' ? 'Continuing…' : 'Continue'}</span>
          </button>
        )}
      </div>
      {error && (
        <p role="alert" className="mt-2 pl-[23px] text-[11px] text-rose-300">
          {error}
        </p>
      )}
    </section>
  );
};
