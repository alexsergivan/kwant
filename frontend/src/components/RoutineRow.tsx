import React, { useId, useState } from 'react';
import { ChevronDown, Clock } from 'lucide-react';
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
}

/**
 * A routine's run in the chat (UX audit #5): a quiet row, not the person's bubble. Clock, "Routine · <name>",
 * the time, and the instructions on one line; the whole row is one button that shows them in full.
 */
export const RoutineRow: React.FC<RoutineRowProps> = ({ messageId, name, text, body, at, flash, defaultOpen = false }) => {
  const [open, setOpen] = useState(defaultOpen);
  const bodyId = `routine-${useId().replace(/:/g, '')}`;
  const label = routineLabel(name);
  const time = localClock(at);
  const line = routineLine(text);
  return (
    <div
      data-message-id={messageId}
      data-routine-row=""
      data-open={open ? '' : undefined}
      tabIndex={-1}
      className={`w-full max-w-5xl min-w-0 rounded-xl focus:outline-none ${flash ? 'kw-flash' : ''}`}
    >
      <div className="max-w-full sm:max-w-2xl min-w-0 rounded-xl border border-zinc-800 bg-zinc-900/40">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-controls={bodyId}
          className="flex w-full min-w-0 min-h-[44px] items-start gap-2.5 px-3 py-2 text-left rounded-xl cursor-pointer hover:bg-zinc-800/40 transition-colors motion-reduce:transition-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--kw-info)]"
        >
          <Clock size={15} aria-hidden="true" className="mt-[3px] shrink-0 text-[var(--kw-muted)]" />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="flex min-w-0 items-baseline gap-2 text-xs">
              <span className="min-w-0 truncate font-medium text-zinc-300" data-routine-label="">
                {label}
              </span>
              {time && (
                <time dateTime={at ?? undefined} className="shrink-0 whitespace-nowrap text-[11px] text-[var(--kw-muted)]">
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
          <span className="shrink-0 mt-[1px] inline-flex items-center gap-1 text-[11px] text-[var(--kw-muted)]">
            <span className="hidden sm:inline">{open ? 'Hide' : 'Show'}</span>
            <ChevronDown size={14} aria-hidden="true" className={`transition-transform motion-reduce:transition-none ${open ? 'rotate-180' : ''}`} />
            <span className="sr-only sm:hidden">{open ? 'Hide instructions' : 'Show instructions'}</span>
          </span>
        </button>
        <div id={bodyId} hidden={!open} data-routine-body="" className="px-3 pb-3 pl-[37px] text-sm leading-relaxed text-zinc-200 whitespace-pre-wrap break-words">
          {open && (body ?? text)}
        </div>
      </div>
    </div>
  );
};
