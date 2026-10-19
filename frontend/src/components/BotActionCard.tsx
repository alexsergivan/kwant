import React, { useState } from 'react';
import { AlertTriangle, CalendarClock, CheckCircle2, Loader2, MinusCircle, Pause, Pencil, ShieldQuestion, Undo2, Wrench } from 'lucide-react';
import { ModernAvatar } from './ModernAvatar';
import { BotActionMeta, actionStateText, botGone, actionVerb, botActionView, routineButtons, routineFacts, sentence } from '../botActions';

export type BotActionDecision = 'undo' | 'confirm' | 'decline';

export interface BotActionCardProps {
  meta: BotActionMeta;
  messageId: string;
  kwantName: string;
  kwantId?: string;
  /** Newer card from `bot_action_updated` or a button's reply; wins over `meta` unless older. */
  live?: BotActionMeta | null;
  /** Resolves to an error message on failure. */
  onDecide: (actionId: string, decision: BotActionDecision) => Promise<string | void>;
  /** Opens Space settings → Routines at this routine. */
  onOpenRoutine?: (routineId: string, mode: 'edit' | 'pause') => void;
  flash?: boolean;
  /** Ids of the bots that exist now; null until loaded. A card whose bot is gone settles. */
  botIds?: ReadonlySet<string> | null;
}

const btn =
  'inline-flex items-center justify-center gap-1.5 min-h-[40px] sm:min-h-[32px] px-3 rounded-xl text-xs font-medium transition-colors disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2';
const quiet = `${btn} text-zinc-200 border border-zinc-700 hover:bg-zinc-800 focus-visible:outline-zinc-500`;

/**
 * A change a bot made in the app (or wants to make), in the chat: what it did in one sentence,
 * the routine's facts when it's a routine, and the person's buttons (Undo, or Confirm/Decline).
 * Once resolved it says so plainly, so an undone change never still reads "Done".
 */
export const BotActionCard: React.FC<BotActionCardProps> = ({ meta, messageId, kwantName, kwantId, live, onDecide, onOpenRoutine, flash, botIds = null }) => {
  const shown = botActionView(meta, live);
  const gone = botGone(shown, botIds);
  // Nothing to undo once the bot it made or changed is deleted.
  const view = gone ? { ...shown, can_undo: false } : shown;
  const [busy, setBusy] = useState<BotActionDecision | null>(null);
  const [error, setError] = useState<string | null>(null);

  const decide = async (d: BotActionDecision) => {
    if (busy) return;
    setBusy(d);
    setError(null);
    const err = await onDecide(view.action_id, d);
    setBusy(null);
    if (err) setError(err);
  };

  const headingId = `bot-action-${messageId}`;
  const state = actionStateText(view, gone);
  const facts = routineFacts(view);
  const { edit, pause } = routineButtons(view);
  const reason = view.status === 'pending' && typeof view.details.reason === 'string' ? view.details.reason : '';
  const isRoutine = view.target_kind === 'routine';
  const resolved = view.status === 'undone' || view.status === 'declined';
  const border = view.status === 'pending' ? 'border-amber-500/40' : resolved || gone ? 'border-zinc-800' : 'border-emerald-500/25';
  const Icon = view.status === 'pending' ? ShieldQuestion : isRoutine ? CalendarClock : Wrench;

  return (
    <section data-message-id={messageId} aria-labelledby={headingId} className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}>
      <ModernAvatar role={kwantName} name={kwantName} size={34} kwantId={kwantId} />
      <div className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${border}`}>
        <div className="px-4 pt-3 pb-2 space-y-1.5">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <Icon size={12} className={view.status === 'pending' ? 'text-amber-300' : 'text-sky-300'} aria-hidden="true" />
            <span id={headingId}>
              <span className="text-zinc-200">{kwantName}</span>
              {` ${actionVerb(view.action)}`}
            </span>
          </span>
          <p className={`text-sm break-words leading-relaxed ${resolved ? 'text-zinc-400 line-through decoration-zinc-600' : 'text-zinc-100'}`}>
            {sentence(view.summary) || 'A change in the app.'}
          </p>
          {reason && (
            <p className="text-sm text-zinc-300 break-words leading-relaxed">
              <span className="text-zinc-500">Why: </span>
              {reason}
            </p>
          )}
        </div>

        {facts.length > 0 && (
          <dl className="mx-4 mb-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2 text-xs">
            {facts.map((f) => (
              <React.Fragment key={f.label}>
                <dt className="text-zinc-500">{f.label}</dt>
                <dd className={`min-w-0 break-words ${resolved ? 'text-zinc-500' : 'text-zinc-200'}`}>{f.value}</dd>
              </React.Fragment>
            ))}
          </dl>
        )}

        <div className="px-4 pb-3 pt-0.5 space-y-2">
          <p role="status" aria-live="polite" className="text-[11px]">
            <span
              className={`inline-flex items-center gap-1.5 ${state.tone === 'ok' ? 'text-emerald-300' : state.tone === 'wait' ? 'text-amber-200' : 'text-zinc-400'}`}
            >
              {state.tone === 'ok' ? (
                <CheckCircle2 size={13} aria-hidden="true" />
              ) : state.tone === 'wait' ? (
                <ShieldQuestion size={13} aria-hidden="true" />
              ) : (
                <MinusCircle size={13} aria-hidden="true" />
              )}
              {state.label}
            </span>
          </p>

          {(view.can_undo || view.needs_confirmation || edit || pause) && (
            <div className="flex flex-wrap items-center gap-2">
              {view.needs_confirmation && (
                <>
                  <button
                    type="button"
                    onClick={() => decide('confirm')}
                    disabled={busy !== null}
                    className={`${btn} bg-amber-500 text-zinc-950 hover:bg-amber-400 focus-visible:outline-amber-400`}
                  >
                    {busy === 'confirm' && <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                    {busy === 'confirm' ? 'Confirming…' : 'Confirm'}
                  </button>
                  <button type="button" onClick={() => decide('decline')} disabled={busy !== null} className={quiet}>
                    {busy === 'decline' ? 'Declining…' : 'Decline'}
                  </button>
                </>
              )}
              {view.can_undo && (
                <button type="button" onClick={() => decide('undo')} disabled={busy !== null} className={quiet}>
                  {busy === 'undo' ? (
                    <Loader2 size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
                  ) : (
                    <Undo2 size={13} aria-hidden="true" />
                  )}
                  {busy === 'undo' ? 'Undoing…' : 'Undo'}
                </button>
              )}
              {edit && onOpenRoutine && view.target_id && (
                <button type="button" onClick={() => onOpenRoutine(view.target_id!, 'edit')} disabled={busy !== null} className={quiet}>
                  <Pencil size={12} aria-hidden="true" /> Edit
                </button>
              )}
              {pause && onOpenRoutine && view.target_id && (
                <button type="button" onClick={() => onOpenRoutine(view.target_id!, 'pause')} disabled={busy !== null} className={quiet}>
                  <Pause size={12} aria-hidden="true" /> Pause
                </button>
              )}
            </div>
          )}

          {error && (
            <p role="alert" className="flex items-start gap-1.5 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-200 break-words">
              <AlertTriangle size={13} className="mt-px shrink-0" aria-hidden="true" />
              <span>{error}</span>
            </p>
          )}
        </div>
      </div>
    </section>
  );
};
