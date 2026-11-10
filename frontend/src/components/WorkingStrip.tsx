import React, { useEffect, useRef, useState } from 'react';
import { ArrowUp, Square } from 'lucide-react';
import type { LiveActivity } from '../activity';
import { formatElapsed, turnStart, waitingOn, workingLabel } from '../workingStrip';
import { usePhoneLayout } from '../usePhoneLayout';
import { ModernAvatar } from './ModernAvatar';
import { WatchButton } from './WatchLive';

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
   * Brings the card the bot waits on into view. With it, a waiting strip shows "Answer ↑": on a phone for a
   * question or an approval (standing in for the "Approval waiting" pill), on desktop for a question only.
   */
  onAnswer?: (on: 'question' | 'approval') => void;
  /** W1: this chat's browser is active and the panel is closed: "Watch" opens it. */
  onWatch?: () => void;
  /** Forces the phone or desktop look (tests); by default it follows the window. */
  phone?: boolean;
}

/**
 * Summary of the running turn (the composer holds the one Stop; onStop is unused there), kept in view
 * above the composer; the card in the stream holds the step details. On a phone it is the one live
 * line (0.6.13 spec A3), kept quiet (0.7.5): a plain ~28 px line, no card, 12 px text and a 16 px avatar,
 * with space below it so it never touches the composer. "Answer ↑" is the one thing with emphasis while
 * the bot waits on the person. Its buttons stay 28 px tall but tap like 44 px (kws-hit in modern.css).
 * On desktop (0.8.0) it is the same kind of quiet line at desktop size. Both sit inside the composer
 * section (ChatView renders them there; on a phone too since 0.8.1 F4), lined up with the box below.
 */
export const WorkingStrip: React.FC<WorkingStripProps> = ({ active, name, title, live, queued, awaitingAnswer, awaitingBrowser, writing, onStop, onAnswer, onWatch, phone: forcePhone }) => {
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
  // Desktop shows Answer only for a question: approvals already have the "Approval waiting" pill there.
  const answer = on && onAnswer && (phone || on === 'question') ? on : null;
  if (phone) {
    return (
      <div className="kws-strip kws-strip-phone mx-auto mb-2 max-w-4xl pl-[21px] pr-[9px]" data-working-strip="" data-waiting-on={on ?? undefined}>
        <div className="flex h-7 items-center gap-2">
          <span className={`shrink-0 inline-flex ${waiting ? '' : 'kw-avatar-working'}`}>
            <ModernAvatar role={title} name={name} size={16} />
          </span>
          <p className="min-w-0 flex-1 truncate text-xs leading-4 text-zinc-400" role="status" aria-live="polite">
            <span className="font-medium text-zinc-300">{name}</span> is {label}
          </p>
          {/* Hidden behind Answer: what to do next matters more than for how long. Below 400 px with
              Watch showing, the time goes so the line never wraps. */}
          {!answer && (
            <span
              className={`shrink-0 tabular-nums text-xs leading-4 text-zinc-500${onWatch ? ' hidden min-[400px]:inline' : ''}`}
              aria-hidden="true"
              data-working-elapsed=""
            >
              {elapsed}
            </span>
          )}
          {onWatch && !answer && <WatchButton name={name} phone onWatch={onWatch} />}
          {answer && (
            <button
              type="button"
              onClick={() => onAnswer?.(answer)}
              data-strip-answer=""
              data-compact=""
              className="kws-hit -mr-2 shrink-0 inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-semibold text-amber-200 hover:bg-amber-500/10 active:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]"
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
              data-compact=""
              className="kws-hit -mr-2 shrink-0 inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
            >
              <Square size={10} className="fill-current" aria-hidden="true" />
              Stop
            </button>
          )}
        </div>
      </div>
    );
  }
  // Desktop (0.8.0, Oleks 2026-11-09): the composer's status line, inside the composer section right
  // above its box, in the same max-w-4xl column. No card: one quiet 28 px line like the phone's. Its
  // edges follow the box below: 21 px in (the box's 1 px border + 8 px padding + the field's 12 px
  // inset), so the avatar starts where the placeholder text does, and 9 px in on the right, so the
  // last item ends where the Stop square does. mb-2 keeps 8 px to the box.
  return (
    <div
      className="kws-strip kws-strip-line mx-auto mb-2 flex h-7 max-w-4xl items-center gap-2 pl-[21px] pr-[9px]"
      data-working-strip=""
      data-waiting-on={on ?? undefined}
    >
      <span className={`shrink-0 inline-flex ${waiting ? '' : 'kw-avatar-working'}`}>
        <ModernAvatar role={title} name={name} size={20} />
      </span>
      <p className="min-w-0 flex-1 truncate text-xs leading-4 text-zinc-400" role="status" aria-live="polite">
        <span className="font-medium text-zinc-200">{name}</span> is {label}
      </p>
      {/* Own box, so a long step name never cuts the time off. Hidden behind Answer, as on the phone. */}
      {!answer && (
        <span className="shrink-0 tabular-nums text-xs leading-4 text-zinc-500" aria-hidden="true" data-working-elapsed="">
          {elapsed}
        </span>
      )}
      {onWatch && !answer && <WatchButton name={name} line onWatch={onWatch} />}
      {answer && (
        <button
          type="button"
          onClick={() => onAnswer?.(answer)}
          data-strip-answer=""
          className="-mr-2 shrink-0 inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-semibold text-amber-200 hover:bg-amber-500/10 active:bg-amber-500/15 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]"
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
          className="-mr-2 shrink-0 inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
        >
          <Square size={10} className="fill-current" aria-hidden="true" />
          Stop
        </button>
      )}
    </div>
  );
};
