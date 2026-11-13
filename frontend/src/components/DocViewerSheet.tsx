import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, ChevronLeft, ChevronRight, CircleCheck, Copy, RotateCw, X } from 'lucide-react';
import { CiteState, DocPage, copyQuote, docKind, formatSize, spaceDocsApi, updatedWhen } from '../spaceDocs';
import { DocMark } from './DocMark';

const focusRing = 'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400';
const secondaryBtn = `inline-flex items-center justify-center gap-1.5 px-3 rounded-lg text-xs font-medium text-zinc-200 border border-zinc-700 bg-zinc-900 hover:bg-zinc-800 disabled:opacity-40 disabled:pointer-events-none ${focusRing}`;

export interface DocViewerOpen {
  space: string;
  file: string;
  /** 1 from the list; the quote's page from a chip (PDFs). */
  page: number;
  /** Set when opened from a chip. */
  quote?: string | null;
  /** The chip's re-check, when opened from one. */
  check?: CiteState;
  /** For .md and .txt: the list's size and date for the sub line. */
  size?: number;
  modified?: string;
  /** PDFs: page count from the list, until the page answers. */
  pages?: number;
}

/**
 * The passage viewer (design §3): the page's plain text with the quote marked. A side sheet from 640 px,
 * a bottom sheet below. Focus moves to ✕, stays inside, Esc closes and focus goes back to the opener
 * (TaskPanel's pattern).
 */
export const DocViewerSheet: React.FC<{ open: DocViewerOpen; onClose: () => void }> = ({ open, onClose }) => {
  const { space, file, quote } = open;
  const pdf = docKind(file) === 'pdf';
  const [page, setPage] = useState(open.page);
  const [data, setData] = useState<DocPage | 'gone' | 'no_route' | 'failed' | null>(null);
  const [seq, setSeq] = useState(0);
  const [copied, setCopied] = useState(false);
  const [shown, setShown] = useState(false);
  const closeRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const markRef = useRef<HTMLElement>(null);
  const subId = useId();
  const gone = open.check === 'gone' || data === 'gone';
  const missing = open.check === 'missing';

  useEffect(() => {
    const back = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const raf = requestAnimationFrame(() => setShown(true));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== 'Tab' || !panelRef.current) return;
      const items = [...panelRef.current.querySelectorAll<HTMLElement>('button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])')];
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('keydown', onKey, true);
      back?.focus?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (open.check === 'gone') return;
    let live = true;
    setData(null);
    spaceDocsApi.page(space, file, page, quote).then((d) => live && setData(d));
    return () => {
      live = false;
    };
  }, [space, file, page, quote, seq, open.check]);

  // The mark at about the middle of the body, with no smooth scroll.
  useLayoutEffect(() => {
    if (data && typeof data === 'object' && data.match) markRef.current?.scrollIntoView({ block: 'center' });
    else bodyRef.current?.scrollTo?.(0, 0);
  }, [data]);

  const pages = data && typeof data === 'object' ? data.pages : open.pages ?? 1;
  const quotePage = open.page;
  const sub = pdf ? `Page ${page} of ${pages}` : [open.size != null ? formatSize(open.size) : '', open.modified ? `updated ${updatedWhen(open.modified)}` : ''].filter(Boolean).join(' · ');

  const copy = () => {
    if (!quote) return;
    navigator.clipboard?.writeText(copyQuote(quote, file, pdf ? quotePage : null)).catch(() => {});
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  const body = (() => {
    if (gone) return null;
    if (data === null)
      return (
        <div className="space-y-2.5 pt-1" aria-hidden="true" data-viewer-loading>
          {[92, 84, 96, 70, 88, 52].map((w, i) => (
            <div key={i} className="h-3 rounded bg-zinc-800" style={{ width: `${w}%` }} />
          ))}
        </div>
      );
    if (data === 'no_route')
      return quote ? (
        <>
          <div className="whitespace-pre-wrap break-words" data-viewer-text>
            <mark className="rounded-[3px] bg-violet-400/[.22] text-zinc-50 px-0.5 -mx-0.5 shadow-[inset_0_-2px_0_rgb(157_143_255/.65)]" data-viewer-mark>
              {quote}
            </mark>
          </div>
          <p className="mt-3 text-xs text-zinc-500" data-viewer-no-route>
            Only the quoted words are shown here for now. The rest of the page opens after the next update.
          </p>
        </>
      ) : (
        <p className="text-sm text-zinc-400" data-viewer-no-route>
          This file’s text opens here after the next update. Bots can already read and quote it.
        </p>
      );
    if (data === 'failed')
      return (
        <p className="flex flex-wrap items-center gap-2 text-sm text-zinc-300" role="status">
          <AlertTriangle size={14} className="text-amber-300" aria-hidden="true" />
          Couldn’t open this page.
          <button type="button" className={`${secondaryBtn} h-8`} onClick={() => setSeq((n) => n + 1)}>
            <RotateCw size={12} aria-hidden="true" /> Try again
          </button>
        </p>
      );
    return (
      <>
        {data.cut_before && <p className="mb-3 text-xs text-zinc-500">Earlier text on this page isn’t shown here. Bots still read all of it.</p>}
        <div className="whitespace-pre-wrap break-words" data-viewer-text>
          {data.text_before}
          {data.match && (
            <mark
              ref={markRef}
              className="rounded-[3px] bg-violet-400/[.22] text-zinc-50 px-0.5 -mx-0.5 shadow-[inset_0_-2px_0_rgb(157_143_255/.65)]"
              data-viewer-mark
            >
              {data.match}
            </mark>
          )}
          {data.text_after}
        </div>
        {data.cut_after && <p className="mt-3 text-xs text-zinc-500">Later text on this page isn’t shown here. Bots still read all of it.</p>}
      </>
    );
  })();

  const sheet = (
    <div className="fixed inset-0 z-50 flex items-end sm:items-stretch sm:justify-end" data-doc-viewer>
      <button type="button" aria-label="Close" tabIndex={-1} onClick={onClose} className="absolute inset-0 bg-black/50 cursor-default" />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={file}
        aria-describedby={subId}
        className={`relative flex flex-col w-full h-[85vh] rounded-t-2xl border-t border-zinc-700 sm:w-[480px] sm:max-w-full sm:h-full sm:rounded-none sm:border-t-0 sm:border-l sm:border-zinc-800 bg-zinc-950 shadow-2xl transition-transform duration-[160ms] ease-out motion-reduce:transition-none ${
          shown ? 'translate-x-0 translate-y-0' : 'translate-y-full sm:translate-y-0 sm:translate-x-full'
        }`}
      >
        <div className="sm:hidden flex justify-center pt-2" aria-hidden="true">
          <span className="w-9 h-1 rounded-full bg-zinc-700" />
        </div>
        <div className="flex items-start gap-2.5 py-3 pl-5 pr-3 border-b border-zinc-800/80 bg-zinc-900/30">
          <DocMark name={file} size={28} />
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-semibold text-zinc-100 truncate" title={file}>
              {file}
            </h3>
            <p id={subId} className="text-xs text-zinc-400">
              {sub}
            </p>
          </div>
          <button
            ref={closeRef}
            type="button"
            onClick={onClose}
            aria-label="Close"
            className={`shrink-0 inline-flex items-center justify-center w-11 h-11 sm:w-9 sm:h-9 -my-1 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 ${focusRing}`}
          >
            <X size={18} aria-hidden="true" />
          </button>
        </div>

        {quote && (gone || missing) ? (
          <div className="flex items-start gap-2 px-5 py-2.5 border-b border-zinc-800 bg-amber-300/[.05] text-[13px] text-zinc-200 leading-relaxed" data-viewer-check={gone ? 'gone' : 'missing'}>
            <AlertTriangle size={14} className="mt-[3px] shrink-0 text-amber-300" aria-hidden="true" />
            {gone ? (
              <p>{file} isn’t in this space’s documents any more.</p>
            ) : (
              <p>
                These words aren’t in {file} now: “{quote}”. The file may have changed since this reply, so check it before you rely on it.
              </p>
            )}
          </div>
        ) : quote && open.check === 'ok' ? (
          <div className="flex flex-wrap items-center gap-2 px-5 py-2.5 border-b border-zinc-800" data-viewer-check="ok">
            <span className="inline-flex items-center gap-1 h-[22px] px-2 rounded-full border border-teal-300/30 bg-teal-300/10 text-ok text-[11.5px] font-semibold">
              <CircleCheck size={12} aria-hidden="true" />
              {pdf ? `Word for word on page ${quotePage}` : `Word for word in ${file}`}
            </span>
          </div>
        ) : null}

        <div ref={bodyRef} className="flex-1 overflow-y-auto px-5 py-4 text-sm leading-6 text-zinc-300" data-viewer-body>
          {body}
        </div>

        {((pdf && pages > 1 && !gone && data !== 'no_route') || quote) && (
          <div
            className="grid grid-cols-2 gap-2 px-5 py-2.5 border-t border-zinc-800 pb-[max(0.625rem,env(safe-area-inset-bottom))] sm:flex sm:items-center sm:pb-2.5"
            data-viewer-footer
          >
            {pdf && pages > 1 && !gone && data !== 'no_route' && (
              <>
                <button type="button" className={`${secondaryBtn} h-11 sm:h-8`} disabled={page <= 1} onClick={() => setPage((n) => Math.max(1, n - 1))}>
                  <ChevronLeft size={14} aria-hidden="true" /> Page {Math.max(1, page - 1)}
                </button>
                <button type="button" className={`${secondaryBtn} h-11 sm:h-8`} disabled={page >= pages} onClick={() => setPage((n) => Math.min(pages, n + 1))}>
                  Page {Math.min(pages, page + 1)} <ChevronRight size={14} aria-hidden="true" />
                </button>
              </>
            )}
            {quote && (
              <button type="button" className={`${secondaryBtn} h-11 sm:h-8 col-span-2 sm:col-span-1 sm:ml-auto`} onClick={copy} data-copy-quote>
                <Copy size={13} aria-hidden="true" /> {copied ? 'Copied' : 'Copy quote'}
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
  return createPortal(sheet, document.body);
};
