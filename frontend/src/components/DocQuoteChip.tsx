import React, { useEffect, useState } from 'react';
import { AlertTriangle, CircleCheck, FileText } from 'lucide-react';
import { CiteState, parseDocUrl, recheckQuote } from '../spaceDocs';
import { DocViewerSheet } from './DocViewerSheet';

/**
 * A `kwant-doc://` link in a reply (design §2): the file and page the address names, and a teal tick once
 * kwant has checked the quote against the file right now. Amber when the words aren't there now. Never holo:
 * the reply's own sticker stays the one earned mark. The bot's link text is not shown.
 */
export const DocQuoteChip: React.FC<{ href: string; fallback?: React.ReactNode }> = ({ href, fallback = null }) => {
  const ref = parseDocUrl(href);
  const [state, setState] = useState<CiteState | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!ref?.quote) return;
    let live = true;
    recheckQuote(ref.space, ref.file, ref.quote).then((s) => live && setState(s));
    return () => {
      live = false;
    };
  }, [ref?.space, ref?.file, ref?.quote]);

  if (!ref) return <>{fallback}</>;
  const bad = state === 'missing' || state === 'gone';
  const where = ref.page ? `${ref.file}, page ${ref.page}` : ref.file;
  const tip = state === 'ok' ? `Checked word for word in ${where}` : bad ? `Not found in ${ref.file} now` : where;
  const label =
    state === 'ok'
      ? `${where}, quote checked word for word. Open the passage.`
      : bad
        ? `${where}, quote not found in the file now. Open the passage.`
        : `${where}. Open the passage.`;

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={label}
        data-tip={tip}
        data-doc-chip={state ?? 'checking'}
        className={`kws-tip relative inline-flex items-center gap-1 h-7 sm:h-6 max-w-[260px] px-1.5 mx-0.5 align-[2px] rounded-md border text-xs font-medium not-italic whitespace-nowrap transition-colors motion-reduce:transition-none before:absolute before:-inset-y-2 before:inset-x-0 before:content-[''] sm:before:hidden focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950 ${
          bad ? 'border-amber-300/40 bg-zinc-900/70 text-amber-200 hover:bg-zinc-800' : 'border-zinc-700 bg-zinc-900/70 text-zinc-200 hover:bg-zinc-800 hover:border-zinc-600'
        }`}
      >
        <FileText size={12} className="shrink-0 text-zinc-400" aria-hidden="true" />
        <span className="truncate max-w-[118px] sm:max-w-[150px]">{ref.file}</span>
        {ref.page && <span className="shrink-0 font-normal text-zinc-400">· p. {ref.page}</span>}
        {state === 'ok' && <CircleCheck size={12} className="shrink-0 text-ok motion-safe:animate-[kw-dd-in_150ms_ease-out]" aria-hidden="true" />}
        {bad && <AlertTriangle size={12} className="shrink-0 text-amber-300" aria-hidden="true" />}
      </button>
      {open && (
        <DocViewerSheet
          open={{ space: ref.space, file: ref.file, page: ref.page ?? 1, quote: ref.quote, check: state ?? undefined }}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
};
