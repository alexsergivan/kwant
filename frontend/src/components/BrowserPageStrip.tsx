import React, { useId } from 'react';

import { BrowserPage, PAGE_COPY, pageLabel, pageOwner, pageWhere } from '../browserPages';

const FOCUS = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#b8adff]';

interface Props {
  pages: BrowserPage[];
  /** The page on screen. */
  current: string | null;
  botName: (id: string) => string | undefined;
  onPick: (page: string) => void;
  /** The person holds the browser: switching would drop their control, so it waits for Hand back. */
  disabled?: boolean;
  narrow?: boolean;
}

/**
 * 0.9.3: the team's pages in one row above the address bar, shown when there are 2 or more. Each
 * tab names its bot and site; a page waiting for the person has the amber "needs you" dot the
 * browser switcher uses.
 */
export const BrowserPageStrip: React.FC<Props> = ({ pages, current, botName, onPick, disabled, narrow }) => {
  const hintId = useId();
  if (pages.length < 2) return null;
  return (
    <div role="group" aria-label={PAGE_COPY.stripName} data-page-strip="" className="flex items-center gap-2 -m-1">
      <div className="kw-scroll-row min-w-0 flex items-center gap-1 overflow-x-auto p-1">
        {pages.map((p) => {
          const here = p.page === current;
          const label = pageLabel(p, botName);
          return (
            <button
              key={p.page}
              type="button"
              data-page-tab={p.page}
              data-page-held={p.held ? '' : undefined}
              title={label}
              aria-label={label}
              aria-current={here || undefined}
              aria-disabled={(disabled && !here) || undefined}
              aria-describedby={disabled && !here ? hintId : undefined}
              onClick={() => !disabled && !here && onPick(p.page)}
              className={`shrink-0 inline-flex items-center gap-1.5 max-w-[11rem] ${narrow ? 'h-11' : 'h-7'} px-2.5 rounded-[10px] border text-[12px] transition-colors ${FOCUS} ${
                here
                  ? 'border-[#b8adff] bg-[rgb(44_37_82/0.8)] text-zinc-100'
                  : `border-zinc-800 bg-zinc-900 text-zinc-300 ${disabled ? 'opacity-60 cursor-not-allowed' : 'hover:bg-zinc-800 hover:text-zinc-100 cursor-pointer'}`
              }`}
            >
              {p.held && <span data-page-needs-you="" aria-hidden="true" className="shrink-0 h-2 w-2 rounded-full bg-[#fcd34d]" />}
              <span className="font-semibold truncate">{pageOwner(p, botName)}</span>
              <span className="text-zinc-400 truncate">{pageWhere(p)}</span>
            </button>
          );
        })}
      </div>
      {disabled && (
        <span id={hintId} data-page-strip-hint="" className="sr-only">
          {PAGE_COPY.handBackFirst}
        </span>
      )}
    </div>
  );
};
