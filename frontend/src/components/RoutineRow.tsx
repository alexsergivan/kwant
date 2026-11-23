import React, { useId, useState } from 'react';
import { ChevronDown, Clock, History } from 'lucide-react';
import { localClock } from '../clock';
import { routineLabel, routineLine } from '../routineRow';

export interface RoutineRowProps {
  messageId: string;
  /** The routine's name (routineOf); '' for an unnamed one. */
  name: string;
  /** The instructions the routine posted, as stored. */
  text: string;
  /** The full instructions, formatted the way the chat formats a message; plain text when absent. */
  body?: React.ReactNode;
  at?: string | null;
  flash?: boolean;
  /** Start expanded (tests and shots); the person toggles it from there. */
  defaultOpen?: boolean;
  /**
   * routine-row-opens-history: opens Routines on this routine with its History open. A named routine with this set
   * gets a History control beside Show/Hide, outside the toggle button; an unnamed one never does.
   */
  onOpenHistory?: () => void;
}

/**
 * A routine's run in the chat (UX audit #5): a quiet row, not the person's bubble. Clock, "Routine · <name>",
 * the time, and the instructions on one line; the row is one button that shows them in full. Beside it, outside it
 * (no nested controls), a small History button opens the routine's History in Routines.
 */
export const RoutineRow: React.FC<RoutineRowProps> = ({ messageId, name, text, body, at, flash, defaultOpen = false, onOpenHistory }) => {
  const [open, setOpen] = useState(defaultOpen);
  const bodyId = `routine-${useId().replace(/:/g, '')}`;
  const label = routineLabel(name);
  const time = localClock(at);
  const line = routineLine(text);
  const history = !!name && !!onOpenHistory;
  // With History beside it, the toggle keeps only its left corners; History takes the right ones.
  const toggleRound = history ? (open ? 'rounded-tl-xl' : 'rounded-l-xl') : open ? 'rounded-t-xl' : 'rounded-xl';
  return (
    <div
      data-message-id={messageId}
      data-routine-row=""
      data-open={open ? '' : undefined}
      tabIndex={-1}
      className={`w-full max-w-5xl min-w-0 rounded-xl focus:outline-none ${flash ? 'kw-flash' : ''}`}
    >
      <div className="max-w-full sm:max-w-2xl min-w-0 rounded-xl border border-zinc-800 bg-zinc-900/40">
        <div className="flex min-w-0 items-stretch">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-controls={bodyId}
          className={`flex w-full min-w-0 flex-1 min-h-[44px] items-start gap-2.5 px-3 py-2 text-left ${toggleRound} cursor-pointer hover:bg-zinc-800/40 transition-colors motion-reduce:transition-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--kw-info)]`}
        >
          <Clock size={15} aria-hidden="true" className="mt-[3px] shrink-0 text-[var(--kw-muted)]" />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="flex min-w-0 items-baseline gap-2 text-xs">
              <span className="min-w-0 truncate font-medium text-zinc-300" data-routine-label="">
                {label}
              </span>
              {time && (
                <time dateTime={at ?? undefined} className="shrink-0 whitespace-nowrap text-meta text-[var(--kw-muted)]">
                  {time}
                </time>
              )}
            </span>
            {!open && line && (
              <span className="mt-0.5 block min-w-0 truncate text-[13px] leading-5 text-[var(--kw-muted)]" data-routine-line="">
                {line}
              </span>
            )}
          </span>
          <span className="shrink-0 mt-[1px] inline-flex items-center gap-1 text-meta text-[var(--kw-muted)]">
            <span className="hidden sm:inline">{open ? 'Hide' : 'Show'}</span>
            <ChevronDown size={14} aria-hidden="true" className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
            <span className="sr-only sm:hidden">{open ? 'Hide instructions' : 'Show instructions'}</span>
          </span>
        </button>
        {history && (
          <button
            type="button"
            onClick={onOpenHistory}
            data-routine-history=""
            aria-label={`History of ${name}`}
            title="Open this routine’s History in Routines"
            className={`shrink-0 inline-flex min-h-[44px] min-w-[44px] items-start justify-center gap-1 border-l border-zinc-800 py-2 px-2.5 sm:pr-3 ${open ? 'rounded-tr-xl' : 'rounded-r-xl'} text-meta font-medium text-info hover:bg-zinc-800/40 hover:text-sky-200 cursor-pointer transition-colors motion-reduce:transition-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--kw-info)]`}
          >
            {/* In a span: the phone's icon-only rule (modern.css) would centre a bare svg; this keeps it on the label's line. */}
            <span className="mt-[2px] inline-flex shrink-0" aria-hidden="true">
              <History size={13} />
            </span>
            {/* Phone: the clock icon alone (44 x 44, named by aria-label), so the routine's name keeps its room. */}
            <span className="mt-[1px] hidden sm:inline">History</span>
          </button>
        )}
        </div>
        <div id={bodyId} hidden={!open} data-routine-body="" className="px-3 pb-3 pl-[37px] text-sm leading-relaxed text-zinc-300 whitespace-pre-wrap break-words">
          {open && (body ?? text)}
        </div>
      </div>
    </div>
  );
};
