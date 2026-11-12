// "What's new in kwant X.Y.Z" (spec: next-076/whats-new, design: Mira's whatsnew.html).
// WhatsNewSheetView draws one version; WhatsNewHost decides when it opens. Settings opens it from the kwant card (VersionCard).
import React, { useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { ArrowRight, ChevronDown, ChevronLeft, ExternalLink, List, X } from 'lucide-react';
import { MarkdownRenderer } from './MarkdownRenderer';
import { useEscapeLayer } from '../escape';
import { useMedia } from '../useMedia';
import { PHONE_QUERY } from '../layout';
import type { SpaceView } from '../nav';
import {
  FIXES_FOLD_OVER,
  FIXES_SHOWN,
  type WhatsNewEntry,
  decideOnLoad,
  highlightView,
  markSeen,
  openRequest,
  openWhatsNew,
  otherDialogOpen,
  releaseUrl,
  seenVersion,
  shortDate,
  subscribeOpen,
} from '../whatsNewSheet';
import { changelogFor, latestWhatsNew, whatsNewFor } from '../whatsNewContent';

/** kwant's own pixel bot with a teal hat: the sheet's and the Settings row's mark. */
export const WhatsNewMark: React.FC<{ size: number }> = ({ size }) => {
  const r = (x: number, y: number, w = 1, h = 1, f = '#b8adff') => <rect x={x} y={y} width={w} height={h} fill={f} />;
  const eye = '#120e26';
  return (
    <svg viewBox="0 0 10 10" width={size} height={size} aria-hidden="true" shapeRendering="crispEdges">
      {r(4, 0, 2, 1, '#5eead4')}
      {r(2, 1, 6, 1)}
      {r(1, 2, 8, 6)}
      {r(2, 8, 6, 1)}
      {r(3, 4, 1, 2, eye)}
      {r(6, 4, 1, 2, eye)}
      {r(3, 7, 4, 1, eye)}
      {r(2, 7, 1, 1, eye)}
      {r(7, 7, 1, 1, eye)}
    </svg>
  );
};

const focus = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

export interface SheetProps {
  entry: WhatsNewEntry;
  /** That version's CHANGELOG section; without it there's no "All changes". */
  changelog: string | null;
  phone: boolean;
  /** Opened after an update: "Updated from <from>" (or "Updated" with no earlier version known). Null when opened by hand. */
  updatedFrom: string | null | undefined;
  onClose: () => void;
  /** "Show me": open a place in the app. */
  onOpen: (view: SpaceView) => void;
  /** Start on the All changes view (screenshots). */
  initialPage?: 'main' | 'all';
  /** Only the release notes (`changelog`), for a version this app has no sheet for yet. */
  notesOnly?: boolean;
}

/** The sheet itself, without deciding when: what tests and screenshots render. */
export const WhatsNewSheetView: React.FC<SheetProps> = ({ entry, changelog, phone, updatedFrom, onClose, onOpen, initialPage = 'main', notesOnly = false }) => {
  const [page, setPage] = useState<'main' | 'all'>(notesOnly || (initialPage === 'all' && changelog) ? 'all' : 'main');
  const [fixesOpen, setFixesOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const fixesId = useId();
  useEscapeLayer(rootRef, true, onClose, { modal: true });

  // Switching views starts at the top, with focus on the new title (as a page change would).
  const first = useRef(true);
  useEffect(() => {
    if (first.current) return void (first.current = false);
    bodyRef.current?.scrollTo?.({ top: 0 });
    titleRef.current?.focus();
  }, [page]);

  const date = shortDate(entry.date);
  const folded = entry.fixes.length > FIXES_FOLD_OVER && !fixesOpen;
  const fixes = folded ? entry.fixes.slice(0, FIXES_SHOWN) : entry.fixes;
  const leadId = entry.highlights.find((h) => h.image)?.id;
  const img = (name: string) => `/whatsnew/${entry.version}/${name}`;

  const btnH = phone ? 'h-12' : 'h-10';
  const ghost = `inline-flex ${btnH} items-center justify-center gap-1.5 rounded-xl border border-white/[0.14] bg-white/[0.03] px-3.5 text-[13px] font-medium text-zinc-200 hover:bg-white/[0.08] hover:text-zinc-50 ${focus}`;
  const primary = `inline-flex ${btnH} min-w-[120px] items-center justify-center rounded-xl bg-[linear-gradient(135deg,#7a62f0_0%,#5b47d6_100%)] px-5 text-sm font-semibold text-white shadow-[inset_0_1px_0_0_rgb(255_255_255/.04),0_1px_2px_0_rgb(6_3_20/.5)] hover:brightness-110 ${phone ? 'flex-1' : ''} ${focus}`;
  const close = (
    <button
      type="button"
      onClick={onClose}
      aria-label="Close"
      className={`absolute ${phone ? 'right-2 top-3.5 h-11 w-11' : 'right-3 top-3 h-8 w-8'} grid place-items-center rounded-lg text-zinc-400 hover:bg-white/[0.06] hover:text-zinc-100 ${focus}`}
    >
      <X size={15} aria-hidden="true" />
    </button>
  );
  const gotIt = (
    <button type="button" onClick={onClose} className={primary} data-whats-new-done>
      Got it
    </button>
  );

  const main = (
    <>
      <div className={`flex shrink-0 items-center ${phone ? 'gap-3 py-3 pl-[18px] pr-[52px]' : 'gap-3.5 pb-3.5 pl-6 pr-14 pt-5'}`}>
        <span className={`grid shrink-0 place-items-center rounded-xl bg-surface-overlay shadow-[inset_0_0_0_1px_rgb(255_255_255/.14)] ${phone ? 'h-10 w-10' : 'h-11 w-11'}`}>
          <WhatsNewMark size={phone ? 26 : 30} />
        </span>
        <div className="min-w-0">
          <p className="m-0 text-xs leading-[18px] text-zinc-400" data-whats-new-eyebrow>
            {updatedFrom !== null ? (
              <>
                <b className="font-medium text-teal-300">Updated</b>
                {updatedFrom ? ` from ${updatedFrom}` : ''} · {date}
              </>
            ) : (
              `Released ${date}`
            )}
          </p>
          <h2 ref={titleRef} id={titleId} tabIndex={-1} className={`m-0 font-semibold tracking-[-.005em] text-zinc-50 outline-none ${phone ? 'text-[17px] leading-6' : 'text-lg leading-[26px]'}`}>
            What’s new in kwant bots {entry.version}
          </h2>
        </div>
      </div>
      {close}
      <div ref={bodyRef} className={`min-h-0 flex-1 overflow-y-auto overscroll-contain ${phone ? 'px-[18px] pb-4' : 'px-6 pb-[18px]'}`}>
        <ul className="m-0 list-none p-0">
          {entry.highlights.map((h, i) => {
            const view = highlightView(h);
            const lead = h.image && h.id === leadId;
            const text = (
              <div className="min-w-0 flex-1">
                <h3 className="m-0 flex items-baseline gap-2.5 text-[15px] font-semibold leading-[22px] text-zinc-100">
                  <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 -translate-y-0.5 bg-sky-400" />
                  {h.title}
                </h3>
                <p className={`mb-0 ml-4 mt-0.5 text-sm text-zinc-300 ${phone ? 'leading-5' : 'leading-[21px]'}`}>{h.text}</p>
                {view && (
                  <button
                    type="button"
                    onClick={() => onOpen(view)}
                    aria-label={`Show me: ${h.title}`}
                    className={`-mb-1 ml-2.5 mt-1 inline-flex items-center gap-1.5 rounded-lg pl-1.5 text-[13px] font-semibold text-sky-300 hover:bg-sky-500/[0.14] hover:text-sky-200 ${phone ? 'h-11 pr-2.5' : 'h-8 pr-2'} ${focus}`}
                  >
                    Show me <ArrowRight size={14} aria-hidden="true" />
                  </button>
                )}
              </div>
            );
            return (
              <li key={h.id} className={`border-t border-white/[0.07] ${i === 0 ? 'border-t-0 pb-3 pt-1' : 'py-3'}`} data-highlight={h.id}>
                {lead ? (
                  <>
                    <img src={img(h.image!)} alt="" className="mb-3 block aspect-[2/1] w-full rounded-[10px] bg-[#0f0b22] object-cover shadow-[inset_0_0_0_1px_rgb(255_255_255/.14)]" />
                    {text}
                  </>
                ) : (
                  <div className={`flex items-start ${phone ? 'gap-3' : 'gap-4'}`}>
                    {text}
                    {h.image && (
                      <img
                        src={img(h.image)}
                        alt=""
                        className={`mt-1 block aspect-[16/10] shrink-0 rounded-[10px] bg-[#0f0b22] object-cover ${phone ? 'w-[88px]' : 'w-[120px]'}`}
                      />
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
        {entry.fixes.length > 0 && (
          <section aria-labelledby={fixesId} className={`mt-1.5 rounded-xl bg-white/[0.03] shadow-[inset_0_0_0_1px_rgb(255_255_255/.07)] ${phone ? 'px-3.5 py-3' : 'px-4 pb-3.5 pt-3'}`}>
            <h4 id={fixesId} className="m-0 mb-1.5 text-[13px] font-semibold leading-5 text-zinc-300">
              Smaller changes and fixes: {entry.fixes.length}
            </h4>
            <ul className="m-0 grid list-none gap-1 p-0" data-whats-new-fixes>
              {fixes.map((f, i) => (
                <li key={i} className="relative pl-3.5 text-[13px] leading-5 text-zinc-400">
                  <span aria-hidden="true" className="absolute left-0.5 top-2 h-1 w-1 rounded-[1px] bg-zinc-600" />
                  {f}
                </li>
              ))}
            </ul>
            {folded && (
              <button
                type="button"
                aria-expanded={false}
                onClick={() => setFixesOpen(true)}
                className={`-ml-1.5 mt-1.5 inline-flex h-7 items-center gap-1 rounded-md px-1.5 text-[13px] font-medium text-sky-300 hover:bg-sky-500/[0.12] ${phone ? 'min-h-[44px]' : ''} ${focus}`}
              >
                Show {entry.fixes.length - FIXES_SHOWN} more <ChevronDown size={15} aria-hidden="true" />
              </button>
            )}
          </section>
        )}
      </div>
      <div className={`relative flex shrink-0 items-center gap-2.5 border-t border-white/[0.07] bg-surface-raised ${phone ? 'px-4 pb-[calc(14px+env(safe-area-inset-bottom))] pt-3' : 'px-6 pb-[18px] pt-3.5'}`}>
        <span aria-hidden="true" className="pointer-events-none absolute inset-x-0 bottom-full h-6 bg-gradient-to-t from-[var(--kw-surface-raised)] to-transparent" />
        {changelog && (
          <button type="button" onClick={() => setPage('all')} className={ghost}>
            <List size={15} aria-hidden="true" /> All changes
          </button>
        )}
        {!phone && <span className="flex-1" />}
        {gotIt}
      </div>
    </>
  );

  const all = (
    <>
      <div className={`flex shrink-0 flex-col items-start gap-1 ${phone ? 'py-3 pl-[18px] pr-[52px]' : 'pb-3.5 pl-6 pr-14 pt-5'}`}>
        {notesOnly ? (
          <p className="m-0 text-xs leading-[18px] text-zinc-400" data-whats-new-eyebrow>
            {date ? `Released ${date}` : 'New version'}
          </p>
        ) : (
        <button
          type="button"
          onClick={() => setPage('main')}
          className={`-ml-1 inline-flex items-center gap-1 rounded-lg pl-1 pr-2 text-[13px] font-medium text-zinc-300 hover:bg-white/[0.06] hover:text-zinc-100 ${phone ? 'h-11' : 'h-8'} ${focus}`}
        >
          <ChevronLeft size={15} aria-hidden="true" /> What’s new
        </button>
        )}
        <h2 ref={titleRef} id={titleId} tabIndex={-1} className={`m-0 font-semibold text-zinc-50 outline-none ${phone ? 'text-[17px] leading-6' : 'text-lg leading-[26px]'}`}>
          {notesOnly ? `What’s new in kwant bots ${entry.version}` : `All changes in ${entry.version}`}
        </h2>
      </div>
      {close}
      <div ref={bodyRef} className={`min-h-0 flex-1 overflow-y-auto overscroll-contain select-text text-[13.5px] text-zinc-300 ${phone ? 'px-[18px] pb-4' : 'px-6 pb-[18px]'}`} data-whats-new-all>
        <MarkdownRenderer content={changelog ?? ''} />
      </div>
      <div className={`flex shrink-0 items-center gap-2.5 border-t border-white/[0.07] bg-surface-raised ${phone ? 'px-4 pb-[calc(14px+env(safe-area-inset-bottom))] pt-3' : 'px-6 pb-[18px] pt-3.5'}`}>
        <a href={releaseUrl(entry.version)} target="_blank" rel="noreferrer" className={ghost}>
          <ExternalLink size={15} aria-hidden="true" /> On GitHub
        </a>
        {!phone && <span className="flex-1" />}
        {gotIt}
      </div>
    </>
  );

  return (
    <div
      className={`fixed inset-0 z-[80] bg-[rgb(6_3_20/.6)] ${phone ? 'flex items-end' : 'grid place-items-center p-10'}`}
      onClick={onClose}
      data-whats-new={entry.version}
    >
      <div
        ref={rootRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onClick={(e) => e.stopPropagation()}
        className={
          phone
            ? 'relative flex max-h-[88vh] w-full flex-col overflow-hidden rounded-t-[20px] border border-b-0 border-white/[0.14] bg-surface-raised outline-none animate-[t-sheet_200ms_ease-out] motion-reduce:animate-none'
            : 'relative flex max-h-[min(760px,calc(100vh-80px))] w-[560px] max-w-[calc(100vw-32px)] flex-col overflow-hidden rounded-2xl border border-white/[0.14] bg-surface-raised shadow-[inset_0_1px_0_0_rgb(255_255_255/.05),0_24px_64px_-16px_rgb(6_3_20/.85)] outline-none animate-[t-rise_180ms_ease-out] motion-reduce:animate-none'
        }
      >
        {phone && <span aria-hidden="true" className="mx-auto mt-2 block h-1 w-9 shrink-0 rounded bg-zinc-700" />}
        {page === 'all' ? all : main}
      </div>
    </div>
  );
};

interface Open {
  entry: WhatsNewEntry;
  updatedFrom: string | null | undefined;
  /** Release notes only: a version with no sheet in this app (the one an update offers). */
  notes?: string;
}

/** A sheet for a version this app doesn't bundle: its release notes, nothing else. */
const notesEntry = (version: string, date: string | null | undefined): WhatsNewEntry => ({
  version,
  date: date && /^\d{4}-\d{2}-\d{2}/.test(date) ? date.slice(0, 10) : '',
  highlights: [],
  fixes: [],
});

/**
 * Opens the sheet once per device after an update, and whenever the update banner or Settings › About asks.
 * `established`: this person has used kwant before (an account works, or they've written in a chat); false
 * for a brand-new person, null while that isn't known yet. Mounted once inside the space.
 */
export const WhatsNewHost: React.FC<{ established: boolean | null; onOpen: (view: SpaceView) => void; latest?: WhatsNewEntry | null }> = ({
  established,
  onOpen,
  latest = latestWhatsNew(),
}) => {
  const phone = useMedia(PHONE_QUERY);
  const [open, setOpen] = useState<Open | null>(null);
  const isOpen = !!open;

  // After an update: once per version per device, never over another dialog.
  useEffect(() => {
    if (!latest || isOpen) return;
    const seen = seenVersion();
    const todo = decideOnLoad(seen, latest.version, established);
    if (todo === 'record') markSeen(latest.version);
    if (todo !== 'show') return;
    const tryOpen = () => {
      if (otherDialogOpen()) return false;
      markSeen(latest.version);
      // No stored key: someone upgrading from 0.7.5 or earlier, from a version we can't name.
      setOpen({ entry: latest, updatedFrom: seen ?? '' });
      return true;
    };
    if (tryOpen() || typeof MutationObserver === 'undefined') return;
    const watch = new MutationObserver(() => {
      if (tryOpen()) watch.disconnect();
    });
    watch.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['aria-modal', 'role', 'open'] });
    return () => watch.disconnect();
  }, [latest, established, isOpen]);

  // Asked for by hand (banner, Settings): opens over Settings, since that's where the person is.
  const req = useSyncExternalStore(subscribeOpen, openRequest, openRequest);
  const handled = useRef(req?.seq ?? 0);
  useEffect(() => {
    if (!req || req.seq === handled.current) return;
    handled.current = req.seq;
    const entry = whatsNewFor(req.version);
    if (!entry) {
      if (req.notes?.trim()) setOpen({ entry: notesEntry(req.version, req.date), updatedFrom: null, notes: req.notes });
      return;
    }
    markSeen(entry.version);
    setOpen({ entry, updatedFrom: null });
  }, [req]);

  if (!open) return null;
  return (
    <WhatsNewSheetView
      key={open.entry.version}
      entry={open.entry}
      changelog={open.notes ?? changelogFor(open.entry.version)}
      notesOnly={open.notes !== undefined}
      phone={phone}
      updatedFrom={open.updatedFrom}
      onClose={() => setOpen(null)}
      onOpen={(view) => {
        setOpen(null);
        onOpen(view);
      }}
    />
  );
};

/** Whether the banner's "See what's new" has something to open for `version`. */
export const hasWhatsNew = (version: string) => !!whatsNewFor(version);
