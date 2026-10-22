import React, { useState } from 'react';
import { CheckCircle2, Clock, Globe, Hand, Loader2, MinusCircle, MonitorPlay } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { HumanStep, HumanStepMeta, KIND_TEXT, domainOf, humanStepView } from '../humanSteps';

export interface HumanStepCardProps {
  meta: HumanStepMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  /** Live record from `human_step_updated`, when known; wins over `meta`. */
  live?: HumanStep | null;
  /** A turn is paused on this step right now (else Done starts a new reply). */
  waiting: boolean;
  /** Shows the bot's browser panel. */
  onOpenBrowser?: (sandboxTarget: string | null) => void;
  /** Resolve to an error message on failure. */
  onDone: (id: string) => Promise<string | void>;
  onSkip: (id: string) => Promise<string | void>;
  flash?: boolean;
}

/**
 * A bot asks the person to do one step in its browser: sign in, solve a CAPTCHA or enter a code.
 * Only the site's domain is shown. Done and Skip answer it; a closed card keeps the outcome.
 */
export const HumanStepCard: React.FC<HumanStepCardProps> = ({ meta, messageId, kwantName, kwantId, live, waiting, onOpenBrowser, onDone, onSkip, flash }) => {
  const view = humanStepView(meta, live);
  const [busy, setBusy] = useState<'done' | 'skip' | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Shown right away; the server's human_step_updated confirms it.
  const [sent, setSent] = useState<'done' | 'declined' | null>(null);

  const status = sent && view.status === 'open' ? sent : view.status;
  const open = status === 'open';
  const text = KIND_TEXT[view.step_kind];
  const domain = domainOf(view.url);
  const headingId = `hs-${messageId}`;

  const answer = async (which: 'done' | 'skip') => {
    if (!open || busy) return;
    setBusy(which);
    setError(null);
    const err = await (which === 'done' ? onDone : onSkip)(meta.human_step_id);
    setBusy(null);
    if (err) setError(err);
    else setSent(which === 'done' ? 'done' : 'declined');
  };

  const line =
    status === 'done' ? (
      <span className="inline-flex items-center gap-1.5 text-emerald-300">
        <CheckCircle2 size={13} aria-hidden="true" /> Done. {kwantName} is carrying on{view.status === 'open' ? '…' : '.'}
      </span>
    ) : status === 'declined' ? (
      <span className="inline-flex items-center gap-1.5 text-zinc-400">
        <MinusCircle size={13} aria-hidden="true" /> Skipped. {kwantName} continues without it.
      </span>
    ) : status === 'cancelled' ? (
      <span className="inline-flex items-center gap-1.5 text-zinc-400">
        <MinusCircle size={13} aria-hidden="true" /> Closed: the reply was stopped.
      </span>
    ) : waiting ? (
      <span className="inline-flex items-center gap-1.5 text-amber-300">
        <Clock size={13} aria-hidden="true" /> {kwantName} is paused and waiting for you.
      </span>
    ) : (
      <span className="inline-flex items-center gap-1.5 text-zinc-400">
        <Clock size={13} aria-hidden="true" /> {kwantName} stopped waiting. Press Done any time and it picks up in a new reply.
      </span>
    );

  const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-400';

  return (
    <section
      data-message-id={messageId}
      aria-labelledby={headingId}
      className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}
    >
      <ModernAvatar role={kwantName} name={kwantName} size={34} kwantId={kwantId} />
      <div className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${open ? 'border-amber-500/40' : 'border-zinc-800'}`}>
        <div className="px-4 pt-3 pb-2 space-y-1">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <Hand size={12} className="text-amber-300" aria-hidden="true" />
            <span className="text-zinc-200">{kwantName}</span> needs you in the browser
            {view.source === 'auto' && <span className="text-zinc-400">· spotted automatically</span>}
          </span>
          <h3 id={headingId} className="text-sm font-semibold text-zinc-100">
            {text.heading}
          </h3>
          {view.reason && <p className="text-sm text-zinc-300 whitespace-pre-wrap break-words leading-snug">{view.reason}</p>}
          {domain && (
            <p className="inline-flex items-center gap-1.5 text-xs text-zinc-400">
              <Globe size={12} aria-hidden="true" />
              <span className="font-mono text-zinc-200">{domain}</span>
            </p>
          )}
        </div>

        {open && (
          <div className="px-4 pb-2 flex flex-col sm:flex-row sm:flex-wrap gap-2">
            {onOpenBrowser && (
              <button
                type="button"
                onClick={() => onOpenBrowser(view.sandbox_target)}
                className={`inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3 py-1.5 rounded-xl text-sm border bg-zinc-800/80 border-zinc-700 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600 ${focusRing}`}
              >
                <MonitorPlay size={14} aria-hidden="true" /> Open browser
              </button>
            )}
            <button
              type="button"
              onClick={() => answer('done')}
              disabled={busy !== null}
              className={`inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3 py-1.5 rounded-xl text-sm font-semibold bg-amber-400 text-zinc-950 hover:bg-amber-300 disabled:opacity-60 ${focusRing}`}
            >
              {busy === 'done' ? <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <CheckCircle2 size={14} aria-hidden="true" />}
              {text.done}
            </button>
            <button
              type="button"
              onClick={() => answer('skip')}
              disabled={busy !== null}
              className={`inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[34px] px-3 py-1.5 rounded-xl text-sm text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 disabled:opacity-60 ${focusRing}`}
            >
              {busy === 'skip' ? 'Skipping…' : 'Skip'}
            </button>
          </div>
        )}

        <div className="px-4 pb-3 pt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
          <p role="status" aria-live="polite" className="min-w-0">
            {line}
          </p>
          {open && <p className="basis-full text-zinc-400">Do it in the browser panel, then press Done. What you type into fields there isn’t passed to {kwantName}.</p>}
          {error && (
            <span role="alert" className="basis-full text-rose-300">
              {error}
            </span>
          )}
        </div>
      </div>
    </section>
  );
};
