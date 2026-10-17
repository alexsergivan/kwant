import React, { useState } from 'react';
import { ArrowRightLeft, CheckCircle2, Clock, ExternalLink, Loader2, Square, XCircle, AlertTriangle } from 'lucide-react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ModernAvatar } from './ModernAvatar';
import { Delegation, DelegationMeta, DelegationStatus, delegationStatusLine, isOpenDelegation } from '../threads';

export interface DelegationCardProps {
  meta: DelegationMeta;
  /** The message text: the result for `delegation_result`, kwnt's note otherwise. */
  text: string;
  messageId: string;
  /** Live record from `delegation_updated`, when known; wins over `meta.status`. */
  live?: Delegation | null;
  /** Place in line while the specialist's turn waits for a slot. */
  queuePos?: number | null;
  onOpenThread: (meta: DelegationMeta) => void;
  /** Stops the delegation; resolves to an error message on failure. */
  onCancel: (delegationId: string) => Promise<string | void>;
  flash?: boolean;
}

const LONG_RESULT = 1400;

const tone: Record<DelegationStatus, { icon: React.ReactNode; ring: string; text: string }> = {
  queued: { icon: <Clock size={13} aria-hidden="true" />, ring: 'border-amber-500/30', text: 'text-amber-300' },
  running: { icon: <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />, ring: 'border-sky-500/30', text: 'text-sky-300' },
  completed: { icon: <CheckCircle2 size={13} aria-hidden="true" />, ring: 'border-emerald-500/30', text: 'text-emerald-300' },
  failed: { icon: <XCircle size={13} aria-hidden="true" />, ring: 'border-rose-500/30', text: 'text-rose-300' },
  cancelled: { icon: <Square size={11} aria-hidden="true" />, ring: 'border-zinc-700', text: 'text-zinc-400' },
  interrupted: { icon: <AlertTriangle size={13} aria-hidden="true" />, ring: 'border-amber-500/30', text: 'text-amber-300' },
};

/**
 * A delegation in the lead's chat: who got the task, its live status, a link to the specialist's
 * thread and Stop while it runs. The result card also shows the specialist's answer.
 */
export const DelegationCard: React.FC<DelegationCardProps> = ({ meta, text, messageId, live, queuePos, onOpenThread, onCancel, flash }) => {
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

  const status: DelegationStatus = live?.status ?? meta.status;
  const t = tone[status] ?? tone.queued;
  const isResult = meta.kind === 'delegation_result';
  const running = isOpenDelegation({ status });
  const line = delegationStatusLine(status, meta.specialist_name, { queuePos, error: live?.error });
  const headingId = `dlg-${messageId}`;
  const long = isResult && text.length > LONG_RESULT;

  const stop = async () => {
    setStopping(true);
    setError(null);
    const err = await onCancel(meta.delegation_id);
    if (err) setError(err);
    setStopping(false);
  };

  return (
    <section
      data-message-id={messageId}
      aria-labelledby={headingId}
      className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}
    >
      {isResult ? (
        <ModernAvatar role={meta.specialist_name} name={meta.specialist_name} size={34} kwantId={meta.specialist_id || undefined} />
      ) : (
        <div className="w-[34px] h-[34px] flex-shrink-0 rounded-full bg-zinc-900 border border-zinc-800 flex items-center justify-center">
          <ArrowRightLeft size={15} className="text-amber-300" aria-hidden="true" />
        </div>
      )}
      <div className={`flex-1 min-w-0 max-w-4xl rounded-2xl rounded-tl-sm border ${t.ring} bg-zinc-900/60 shadow-sm`}>
        <div className="px-4 pt-3 pb-2 flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="text-[11px] font-medium text-zinc-400">
            {isResult ? 'Result from' : meta.kind === 'delegation_stalled' ? 'Still running at' : 'Delegated to'}{' '}
            <span className="text-zinc-200">{meta.specialist_name}</span>
          </span>
          <h3 id={headingId} className="basis-full text-sm font-semibold text-zinc-100 break-words">
            {meta.title}
          </h3>
          <p className={`basis-full flex items-center gap-1.5 text-[11px] ${t.text}`} role="status" aria-live="polite">
            {t.icon}
            <span className="min-w-0 break-words">{line}</span>
          </p>
        </div>

        {isResult ? (
          <div className="px-4 pb-2 text-sm text-zinc-200 leading-relaxed border-t border-zinc-800/80 pt-3">
            <div className={long && !expanded ? 'max-h-72 overflow-hidden relative' : ''}>
              <MarkdownRenderer content={text} />
              {long && !expanded && (
                <div className="absolute inset-x-0 bottom-0 h-12 bg-gradient-to-t from-zinc-900 to-transparent" aria-hidden="true" />
              )}
            </div>
            {long && (
              <button
                type="button"
                onClick={() => setExpanded((v) => !v)}
                aria-expanded={expanded}
                className="mt-1 text-[11px] font-medium text-sky-300 hover:text-sky-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 rounded"
              >
                {expanded ? 'Show less' : 'Show the whole answer'}
              </button>
            )}
          </div>
        ) : (
          meta.kind === 'delegation_stalled' && <p className="px-4 pb-2 text-xs text-zinc-400">{text}</p>
        )}

        <div className="px-4 pb-3 pt-1 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => onOpenThread(meta)}
            className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-sky-300 bg-sky-500/10 border border-sky-500/30 hover:bg-sky-500/20 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
          >
            <ExternalLink size={12} aria-hidden="true" />
            Open {meta.specialist_name}'s thread
          </button>
          {running && (
            <button
              type="button"
              onClick={stop}
              disabled={stopping}
              aria-label={`Stop “${meta.title}”`}
              className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-rose-300 bg-rose-500/10 border border-rose-500/30 hover:bg-rose-500/20 transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-500"
            >
              <Square size={10} fill="currentColor" aria-hidden="true" />
              {stopping ? 'Stopping…' : 'Stop'}
            </button>
          )}
          {error && (
            <span role="alert" className="text-[11px] text-rose-300">
              {error}
            </span>
          )}
        </div>
      </div>
    </section>
  );
};
