import React, { useEffect, useRef, useState } from 'react';
import { ArrowUp, Square } from 'lucide-react';
import type { LiveActivity } from '../activity';
import { formatElapsed, turnStart, waitingOn, workingLabel } from '../workingStrip';
import { usePhoneLayout } from '../usePhoneLayout';
import { ModernAvatar } from './ModernAvatar';

interface WorkingStripProps {
  active: boolean;
  name: string;
  /** Picks the avatar's creature. */
  title: string;
  live?: LiveActivity | null;
  queued?: boolean;
  awaitingAnswer?: boolean;
  awaitingBrowser?: boolean;
  writing?: boolean;
  onStop?: () => void;
  /**
   * Phone only: brings the card the bot waits on (its question, or an approval) into view. With it,
   * a waiting strip shows "Answer ↑", which stands in for the separate "Approval waiting" pill.
   */
  onAnswer?: (on: 'question' | 'approval') => void;
  /** Forces the phone or desktop look (tests); by default it follows the window. */
  phone?: boolean;
}

/**
 * Summary of the running turn (the composer holds the one Stop; onStop is unused there), kept in view
 * above the composer; the card in the stream holds the step details. On a phone it is the one live
 * line (0.6.13 spec A3): a borderless 28 px line, 12 px text, and "Answer ↑" while the bot waits on the person.
 */
export const WorkingStrip: React.FC<WorkingStripProps> = ({ active, name, title, live, queued, awaitingAnswer, awaitingBrowser, writing, onStop, onAnswer, phone: forcePhone }) => {
  const mountedAt = useRef(Date.now());
  const [now, setNow] = useState(() => Date.now());
  const phoneLayout = usePhoneLayout();
  const phone = forcePhone ?? phoneLayout;
  useEffect(() => {
    if (!active) return;
    mountedAt.current = Date.now();
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  if (!active) return null;

  const state = { live, queued, awaitingAnswer, awaitingBrowser, writing };
  const label = workingLabel(state);
  const elapsed = formatElapsed(now - turnStart(live, mountedAt.current));
  const on = waitingOn(state);
  const waiting = !!queued || !!awaitingAnswer || on !== null;
  const answer = phone && on && onAnswer ? on : null;
  return (
    <div className={`kws-strip px-3 sm:px-6 ${phone ? 'pt-1 pb-1.5' : 'pt-2 pb-2'}`} data-working-strip="" data-waiting-on={on ?? undefined}>
      {/* A quiet status line, not a card (Oleks, 0.7.5 phone shot: "too big, less focus"): on a phone no
          box at all, smaller text and avatar; the space below keeps it off the composer's edge. */}
      <div
        className={`mx-auto flex max-w-4xl items-center ${
          phone ? 'min-h-[28px] gap-2 px-1' : 'gap-3 rounded-xl border border-zinc-700/60 bg-zinc-900/80 px-3 py-1.5 shadow-[var(--kws-shadow)]'
        }`}
      >
        <span className={`shrink-0 inline-flex ${waiting ? '' : 'kw-avatar-working'}`}>
          <ModernAvatar role={title} name={name} size={phone ? 16 : 24} />
        </span>
        <p className={`min-w-0 flex-1 truncate ${phone ? 'text-xs leading-4 text-zinc-400' : 'text-xs text-zinc-300'}`} role="status" aria-live="polite">
          <span className={phone ? 'font-medium text-zinc-300' : 'font-semibold text-zinc-100'}>{name}</span> is {label}
        </p>
        {/* Own box, so a long step name never cuts the time off at 390 px. Hidden behind Answer on a
            phone: the line is short there, and what to do next matters more than for how long. */}
        {!answer && (
          <span
            className={`shrink-0 tabular-nums ${phone ? 'text-xs leading-4 text-zinc-500' : 'text-meta text-zinc-400'}`}
            aria-hidden="true"
            data-working-elapsed=""
          >
            {elapsed}
          </span>
        )}
        {answer && (
          <button
            type="button"
            onClick={() => onAnswer?.(answer)}
            data-strip-answer=""
            className="shrink-0 -my-2 inline-flex items-center gap-1 min-h-[44px] min-w-[44px] rounded-lg px-3 text-xs font-semibold text-amber-200 hover:bg-amber-500/10 active:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]"
          >
            Answer
            <ArrowUp size={12} aria-hidden="true" />
          </button>
        )}
        {onStop && (
          <button
            type="button"
            onClick={onStop}
            title="Stop"
            className="shrink-0 inline-flex items-center gap-1.5 min-h-8 rounded-lg border border-zinc-600/70 px-2.5 text-xs font-medium text-zinc-200 hover:bg-zinc-800 hover:text-zinc-50 active:scale-[0.97] motion-reduce:active:scale-100 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
          >
            <Square size={12} className="fill-current" aria-hidden="true" />
            Stop
          </button>
        )}
      </div>
    </div>
  );
};
