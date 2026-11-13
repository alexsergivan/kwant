// Waiting state (UX audit #2, 0.9.7): one amber banner while the only open thing in a chat is a
// question for the person. It replaces the sky "summarizing" banner and the working card, and holds the
// only Stop (a ghost button). It takes a list of lines so later waits (waiting checks) can join it.
import { ArrowUp, Check, Circle, Square } from 'lucide-react';
import { RailStep, WaitingLine, waitingBannerText } from '../waitingState';

const WARN_BORDER = 'color-mix(in srgb, var(--kw-warn) 40%, transparent)';

export function WaitingBanner({
  lines,
  rail,
  stopLabel,
  onStop,
  onShow,
}: {
  lines: WaitingLine[];
  rail?: RailStep[];
  stopLabel: string;
  onStop?: () => void;
  /** Brings the open question card into view. */
  onShow?: () => void;
}) {
  if (!lines.length) return null;
  return (
    <div className="px-3 sm:px-6 pt-3 flex-shrink-0" data-waiting-banner="">
      <div
        role="status"
        className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5 px-3 sm:px-4 py-2 mb-3 rounded-xl border bg-surface-raised text-xs"
        style={{ borderColor: WARN_BORDER }}
      >
        <div className="flex flex-col gap-1 min-w-0 flex-1">
          {lines.map((l, i) => (
            <p key={`${l.kind}-${i}`} className="flex items-center gap-2 min-w-0 text-warn font-medium" data-waiting-line={l.kind}>
              <span aria-hidden="true" className="inline-block w-2 h-2 rounded-full shrink-0" style={{ background: 'var(--kw-warn)' }} />
              <span className="min-w-0">{waitingBannerText(l)}</span>
            </p>
          ))}
          {rail && rail.length > 0 && (
            <ol aria-label="Where the team is" className="flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-meta text-zinc-400" data-waiting-rail="">
              {rail.map((s, i) => (
                <li key={`${s.state}-${i}`} className={`flex items-center gap-1 ${s.state === 'you' ? 'text-warn' : ''}`}>
                  {s.state === 'done' ? (
                    <Check size={11} aria-hidden="true" className="text-ok" />
                  ) : (
                    <Circle size={8} aria-hidden="true" className={s.state === 'you' ? 'fill-current' : ''} />
                  )}
                  <span>
                    {s.state === 'done' && <span className="sr-only">Replied: </span>}
                    {s.state === 'next' && <span className="sr-only">Next: </span>}
                    {s.name}
                  </span>
                </li>
              ))}
            </ol>
          )}
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          {onShow && (
            <button
              type="button"
              onClick={onShow}
              className="min-h-[32px] max-sm:min-h-[44px] inline-flex items-center gap-1 px-2.5 rounded-lg text-warn hover:bg-white/5 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-300"
            >
              <ArrowUp size={12} aria-hidden="true" /> Answer
            </button>
          )}
          {onStop && (
            <button
              type="button"
              onClick={onStop}
              data-waiting-stop=""
              className="min-h-[32px] max-sm:min-h-[44px] inline-flex items-center gap-1 px-2.5 rounded-lg border border-hairline text-zinc-300 hover:text-zinc-100 hover:bg-white/5 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-rose-400"
            >
              <Square size={10} className="fill-current" aria-hidden="true" />
              {stopLabel}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
