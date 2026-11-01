// Inbox: one place for what came in while you were elsewhere. "New" is the old While you were
// away page, unchanged; "Loose ends" is the Loose ends page, unchanged. The tabs sit above them.
import React from 'react';
import { ChevronLeft } from 'lucide-react';
import type { InboxTab } from '../nav';
import { playSfx } from '../audio/chiptune';
import { badgeText } from '../away';

const TABS: { id: InboxTab; label: string }[] = [
  { id: 'new', label: 'New' },
  { id: 'loose', label: 'Loose ends' },
];

export const InboxPage: React.FC<{
  tab: InboxTab;
  onTab: (tab: InboxTab) => void;
  /** Unseen items, shown on the New tab. */
  count?: number;
  /** Phones: back to the list. */
  onBack?: () => void;
  newPage: React.ReactNode;
  loosePage: React.ReactNode;
}> = ({ tab, onTab, count = 0, onBack, newPage, loosePage }) => (
  <div className="flex-1 min-w-0 flex flex-col h-full bg-zinc-950 overflow-hidden" data-inbox>
    <div className={`shrink-0 flex items-center gap-2 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 pt-2 bg-zinc-900/30 select-none`}>
      {onBack && (
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to chats"
          className="shrink-0 mb-1.5 p-2 -ml-0.5 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
        >
          <ChevronLeft size={18} />
        </button>
      )}
      <div role="tablist" aria-label="Inbox" className="flex items-end gap-1">
        {TABS.map((t) => {
          const on = t.id === tab;
          return (
            <button
              key={t.id}
              type="button"
              role="tab"
              id={`inbox-tab-${t.id}`}
              aria-selected={on}
              aria-controls="inbox-panel"
              data-inbox-tab={t.id}
              onClick={() => {
                if (on) return;
                playSfx('click');
                onTab(t.id);
              }}
              onKeyDown={(e) => {
                if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
                e.preventDefault();
                const next = TABS[(TABS.findIndex((x) => x.id === tab) + 1) % TABS.length].id;
                onTab(next);
                requestAnimationFrame(() => document.getElementById(`inbox-tab-${next}`)?.focus());
              }}
              tabIndex={on ? 0 : -1}
              className={`relative flex items-center gap-1.5 px-3 pb-2 pt-1.5 text-xs font-medium border-b-2 -mb-px transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 ${
                on ? 'border-sky-400 text-zinc-100' : 'border-transparent text-zinc-400 hover:text-zinc-200'
              }`}
            >
              {t.label}
              {t.id === 'new' && count > 0 && (
                <span className="min-w-[16px] px-1 rounded-full bg-amber-500 text-zinc-950 text-[9px] font-semibold text-center">
                  {badgeText(count)}
                  <span className="sr-only"> new</span>
                </span>
              )}
            </button>
          );
        })}
      </div>
    </div>
    <div id="inbox-panel" role="tabpanel" aria-labelledby={`inbox-tab-${tab}`} className="flex-1 min-h-0 flex">
      {tab === 'loose' ? loosePage : newPage}
    </div>
  </div>
);
