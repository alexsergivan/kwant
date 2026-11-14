import React from 'react';
import { ChevronRight, Globe } from 'lucide-react';
import { browsedChipText, pagesOfReply, type PageStep } from '../browserPages';

/**
 * 0.9.8 (design browser-panel/design.md A): the pages a reply opened in the bot's browser, as a chip.
 * Live: "● Ben is on roastco.example.com · 2 pages ›" (first row of the Working block).
 * After: "🌐 Ben opened 2 pages ›" (first item of the action row). No chip when the reply opened none.
 */
export const BrowsedChip = React.forwardRef<
  HTMLButtonElement,
  { botName: string; steps: readonly PageStep[] | null | undefined; live: boolean; phone?: boolean; onOpen: () => void }
>(({ botName, steps, live, phone = false, onOpen }, ref) => {
  const pages = pagesOfReply(steps);
  if (pages.length === 0) return null;
  const t = browsedChipText(botName, pages, live);
  const tone = live ? 'bg-emerald-400/[0.08] border-emerald-400/40' : 'bg-sky-400/10 border-sky-400/35';
  return (
    <button
      ref={ref}
      type="button"
      onClick={onOpen}
      aria-label={t.label}
      data-browsed-chip={live ? 'live' : 'finished'}
      className={`inline-flex max-w-full min-w-0 items-center gap-1.5 ${phone ? 'h-8' : 'h-7'} pl-2 pr-2.5 rounded-full border ${tone} text-[13px] font-medium text-zinc-100 transition-colors hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-300 focus-visible:ring-offset-2 focus-visible:ring-offset-[#120e26]`}
    >
      {live ? (
        <span aria-hidden="true" className="shrink-0 w-2 h-2 rounded-full bg-emerald-400 ring-[3px] ring-emerald-400/20 animate-[pulse_1.2s_ease-in-out_infinite] motion-reduce:animate-none" />
      ) : (
        <Globe size={14} aria-hidden="true" className="shrink-0 text-sky-300" />
      )}
      <span className="shrink-0">{t.lead}</span>
      {t.host && <span className={`min-w-0 truncate font-normal text-zinc-300 ${phone ? 'max-w-[96px]' : 'max-w-[160px]'}`}>{t.host}</span>}
      {t.count && <span className="shrink-0 font-normal text-zinc-300">· {t.count}</span>}
      <ChevronRight size={14} aria-hidden="true" className="shrink-0 text-zinc-400" />
    </button>
  );
});
BrowsedChip.displayName = 'BrowsedChip';
