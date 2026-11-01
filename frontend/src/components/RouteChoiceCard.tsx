import React, { useRef, useState } from 'react';
import { Check, CheckCircle2, Loader2, MessagesSquare, Sparkles } from 'lucide-react';
import { ResolvePick, ResolveResult, RouteChoiceMeta, generalHint, routeChoiceOutcome, routeChoiceView } from '../routeChoice';

export interface RouteChoiceCardProps {
  meta: RouteChoiceMeta;
  messageId: string;
  /** Newer card from `route_choice_updated`; wins over `meta` unless it would reopen it. */
  live?: RouteChoiceMeta | null;
  /** The person's message this card is about, quoted when known. */
  question?: string;
  onResolve: (routeChoiceId: string, pick: ResolvePick) => Promise<ResolveResult>;
  flash?: boolean;
}

type Busy = string | 'general' | null;

/**
 * Laya's "Who should answer this?" card for a close call: two bots and "Answer generally". Nothing
 * answers until the person taps one. A settled card keeps the pick and disables the buttons; a
 * second tap (409) just shows it's already answered.
 */
export const RouteChoiceCard: React.FC<RouteChoiceCardProps> = ({ meta, messageId, live, question, onResolve, flash }) => {
  const view = routeChoiceView(meta, live);
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<string | null>(null);
  // Shown right away; route_choice_updated confirms it with the real pick.
  const [sent, setSent] = useState<Busy | 'already'>(null);
  const refs = useRef<Array<HTMLButtonElement | null>>([]);

  const open = view.status === 'open' && sent === null;
  const outcome = routeChoiceOutcome(view);
  const pickedId = view.status === 'resolved' ? (view.chose_general ? 'general' : view.chosen_kwant_id) : sent === 'already' ? null : sent;
  const headingId = `rc-${messageId}`;

  const pick = async (key: string | 'general') => {
    if (!open || busy !== null) return;
    setBusy(key);
    setError(null);
    const r = await onResolve(view.route_choice_id, key === 'general' ? { general: true } : { kwant_id: key });
    setBusy(null);
    if (r.kind === 'sent') setSent(key);
    else if (r.kind === 'already') setSent('already');
    else setError(r.message);
  };

  const options: Array<{ key: string; label: string; sub: string }> = [
    ...view.choices.slice(0, 2).map((b) => ({ key: b.kwant_id, label: b.name, sub: b.title })),
    { key: 'general', label: view.general.label || 'Answer generally', sub: generalHint(view) },
  ];

  const onKey = (e: React.KeyboardEvent, i: number) => {
    const n = options.length;
    const to = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? (i + 1) % n : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? (i - 1 + n) % n : -1;
    if (to < 0) return;
    e.preventDefault();
    refs.current[to]?.focus();
  };

  const status = outcome ? (
    <span className="inline-flex items-center gap-1.5 text-emerald-300">
      <CheckCircle2 size={13} aria-hidden="true" /> {outcome}
    </span>
  ) : sent === 'already' ? (
    <span className="inline-flex items-center gap-1.5 text-zinc-400">
      <CheckCircle2 size={13} aria-hidden="true" /> Already answered.
    </span>
  ) : sent !== null ? (
    <span className="inline-flex items-center gap-1.5 text-emerald-300">
      <CheckCircle2 size={13} aria-hidden="true" /> {sent === 'general' ? 'Sent. A general answer is coming.' : `Sent. ${options.find((o) => o.key === sent)?.label ?? 'The bot'} is answering.`}
    </span>
  ) : (
    <span className="text-zinc-400">Nothing is answered until you choose.</span>
  );

  return (
    <section
      data-message-id={messageId}
      data-route-choice={view.status}
      aria-labelledby={headingId}
      className={`flex items-start gap-3.5 w-full max-w-5xl rounded-xl ${flash ? 'kw-flash' : ''}`}
    >
      <div className="w-[34px] h-[34px] flex-shrink-0 rounded-full bg-violet-500/15 border border-violet-500/40 flex items-center justify-center" aria-hidden="true">
        <Sparkles size={16} className="text-violet-300" />
      </div>
      <div className={`flex-1 min-w-0 max-w-2xl rounded-2xl rounded-tl-sm border bg-zinc-900/60 shadow-sm ${open ? 'border-violet-500/40' : 'border-zinc-800'}`}>
        <div className="px-4 pt-3 pb-2 space-y-1">
          <span className="flex items-center gap-1.5 text-[11px] font-medium text-zinc-400">
            <MessagesSquare size={12} className="text-violet-300" aria-hidden="true" />
            <span className="text-zinc-200">Laya</span> · router
          </span>
          <h3 id={headingId} className="text-sm font-semibold text-zinc-100 leading-snug">
            Who should answer this?
          </h3>
          {question && <p className="text-xs text-zinc-400 break-words line-clamp-2">“{question}”</p>}
        </div>

        <div role="group" aria-labelledby={headingId} className="px-4 pb-2 grid grid-cols-1 sm:grid-cols-3 gap-2">
          {options.map((o, i) => {
            const chosen = pickedId === o.key;
            return (
              <button
                key={o.key}
                ref={(el) => (refs.current[i] = el)}
                type="button"
                disabled={!open || busy !== null}
                aria-pressed={chosen}
                onClick={() => pick(o.key)}
                onKeyDown={(e) => onKey(e, i)}
                className={`flex items-start gap-2 min-h-[44px] px-3 py-2 rounded-xl text-left border transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400 ${
                  chosen
                    ? 'bg-emerald-500/15 border-emerald-500/40 text-emerald-100'
                    : open
                      ? 'bg-zinc-800/80 border-zinc-700 text-zinc-100 hover:bg-zinc-700 hover:border-zinc-600 active:scale-[0.98] disabled:opacity-60'
                      : 'bg-zinc-900 border-zinc-800 text-zinc-500'
                }`}
              >
                <span className="mt-0.5 w-[13px] shrink-0" aria-hidden="true">
                  {busy === o.key ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : chosen ? <Check size={13} /> : null}
                </span>
                <span className="min-w-0">
                  <span className="block text-sm font-medium break-words">{o.label}</span>
                  {o.sub && <span className={`block text-[11px] break-words ${chosen ? 'text-emerald-200/80' : 'text-zinc-400'}`}>{o.sub}</span>}
                </span>
              </button>
            );
          })}
        </div>

        <div className="px-4 pb-3 text-[11px]" role="status">
          {status}
        </div>
        {error && (
          <p role="alert" className="px-4 pb-3 -mt-1 text-[11px] text-rose-400 break-words">
            {error}
          </p>
        )}
      </div>
    </section>
  );
};
