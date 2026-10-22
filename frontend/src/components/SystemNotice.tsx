import React, { useRef, useState } from 'react';
import { AlertTriangle, Check, Info, Loader2, RotateCcw } from 'lucide-react';
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
