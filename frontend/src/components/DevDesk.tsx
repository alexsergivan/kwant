// The Dev desk: a row pinned at the top of the sidebar in a Dev space, and its page with two tabs,
// What landed and Reviews. The page fetches plan §3.2's routes, updates live on the `dev_card`
// SSE (App re-sends it as DEV_CARD_EVENT) and mounts Iris's cards (#314) through DevDeskSlots.

import React, { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { ChevronLeft, Code, Loader2, RefreshCw } from 'lucide-react';
import { devApi, deskStatus } from '../devMode';
import { FindingAction, LandedPage, ShellCard, asCard, parseCards, parseLandedPage } from '../dev/shellTypes';
import { LandedList, ReleaseNotesSheet, ReviewCard, Rollup } from './DevDeskSlots';
import { useDevMode } from './DevModeSwitch';

/** App re-sends the `dev_card` SSE as this window event: detail `{space_id, id, kind, status}`. */
export const DEV_CARD_EVENT = 'kwant:dev-card';
export interface DevCardEvent {
  space_id: string;
  id: string;
  kind: string;
  status: string;
}

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400';

// ---------------------------------------------------------------------------
// One shared cache per space: the sidebar row and the page read the same lists.

export interface DeskData {
  landed: LandedPage | null;
  reviews: ShellCard[] | null;
  loading: boolean;
  error: string | null;
  /** When the last load succeeded (ms), for the header's "Checked 14:32". */
  checkedAt: number | null;
}

const EMPTY: DeskData = { landed: null, reviews: null, loading: false, error: null, checkedAt: null };
const desks = new Map<string, DeskData>();
const deskListeners = new Set<() => void>();
const inflight = new Map<string, Promise<void>>();

function setDesk(space: string, patch: Partial<DeskData>) {
  desks.set(space, { ...(desks.get(space) ?? EMPTY), ...patch });
  deskListeners.forEach((l) => l());
}

export const deskStore = {
  get: (space: string): DeskData => desks.get(space) ?? EMPTY,
  subscribe(l: () => void) {
    deskListeners.add(l);
    return () => void deskListeners.delete(l);
  },
  /** Loads both lists; concurrent calls share one request. */
  load(space: string): Promise<void> {
    const running = inflight.get(space);
    if (running) return running;
    setDesk(space, { loading: true });
    const p = Promise.all([devApi.landed(space), devApi.reviews(space)])
      .then(([l, r]) => setDesk(space, { landed: parseLandedPage(l), reviews: parseCards(r), loading: false, error: null, checkedAt: Date.now() }))
      .catch((e: unknown) => setDesk(space, { loading: false, error: e instanceof Error ? e.message : 'The Dev desk couldn’t load.' }))
      .finally(() => inflight.delete(space));
    inflight.set(space, p);
    return p;
  },
  /** Tests and shots. */
  _set: setDesk,
  _reset() {
    desks.clear();
    inflight.clear();
  },
};

/** "14:32", 24-hour local time. */
export function checkedLabel(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

export function useDesk(space: string, on: boolean): DeskData {
  const data = useSyncExternalStore(deskStore.subscribe, () => deskStore.get(space), () => EMPTY);
  useEffect(() => {
    if (!on) return undefined;
    void deskStore.load(space);
    const onCard = (e: Event) => {
      const d = (e as CustomEvent<DevCardEvent>).detail;
      if (d?.space_id === space && d.kind !== 'release_notes') void deskStore.load(space);
    };
    window.addEventListener(DEV_CARD_EVENT, onCard);
    return () => window.removeEventListener(DEV_CARD_EVENT, onCard);
  }, [space, on]);
  return data;
}

export const openReviews = (reviews: ShellCard[] | null) => (reviews ?? []).filter((c) => c.status !== 'stale').length;

// ---------------------------------------------------------------------------
// The sidebar row

export const DevDeskRow: React.FC<{ spaceId: string; active: boolean; onOpen: () => void; className?: string }> = ({ spaceId, active, onOpen, className }) => {
  const desk = useDesk(spaceId, true);
  const landed = desk.landed?.rollup?.count ?? 0;
  const reviews = openReviews(desk.reviews);
  const count = landed + reviews;
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-current={active ? 'page' : undefined}
      data-dev-desk-row
      className={
        className ??
        `w-full flex items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left ${focusRing} ${active ? 'bg-accent-soft text-zinc-50' : 'text-zinc-300 hover:bg-white/[0.05] hover:text-zinc-50'}`
      }
    >
      <span className="w-7 h-7 shrink-0 rounded-lg bg-accent-soft text-sky-300 flex items-center justify-center" aria-hidden="true">
        <Code size={14} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-body font-medium truncate">Dev desk</span>
        <span className="block text-2xs text-zinc-400 truncate" data-dev-desk-status>
          {desk.landed || desk.reviews ? deskStatus(landed, reviews) : desk.error ? 'Couldn’t load' : 'Loading…'}
        </span>
      </span>
      {count > 0 && (
        <span className="shrink-0 min-w-[20px] h-5 px-1.5 rounded-full bg-accent-gradient text-2xs font-semibold text-white flex items-center justify-center tabular-nums" aria-label={`${count} new`}>
          {count}
        </span>
      )}
    </button>
  );
};

// ---------------------------------------------------------------------------
// The page

type Tab = 'landed' | 'reviews';

export interface DevDeskViewProps {
  spaceId: string;
  spaceName: string;
  onBack?: () => void;
  /** Shown only when Notion is connected (Iris's sheet hides "Save to Notion" otherwise). */
  onSaveToNotion?: (markdown: string) => Promise<void>;
  initialTab?: Tab;
  now?: number;
}

export const DevDeskView: React.FC<DevDeskViewProps> = ({ spaceId, spaceName, onBack, onSaveToNotion, initialTab = 'landed', now }) => {
  const desk = useDesk(spaceId, true);
  const dev = useDevMode(spaceId);
  const [tab, setTab] = useState<Tab>(initialTab);
  const [marking, setMarking] = useState(false);
  const [notes, setNotes] = useState<ShellCard | null>(null);
  const [notesError, setNotesError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const tabsId = useId();
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const panelRef = useRef<HTMLDivElement>(null);
  const notesId = notes?.id ?? null;
  // "Open review" on a landed card: the PR whose review card gets scrolled to and focused once the
  // Reviews tab has rendered.
  const [openPr, setOpenPr] = useState<number | null>(null);

  const openReview = useCallback((pr: number) => {
    setTab('reviews');
    setOpenPr(pr);
  }, []);

  useEffect(() => {
    if (openPr === null || tab !== 'reviews') return;
    const panel = panelRef.current;
    const card = panel?.querySelector<HTMLElement>(`[data-dev-card="review"][data-pr="${openPr}"]`);
    const heading = card?.querySelector<HTMLElement>('h3');
    setOpenPr(null);
    if (!card || !heading) {
      // Not on the desk (closed since, or not reviewed yet): focus the Reviews tab so the move is still visible.
      tabRefs.current[1]?.focus();
      return;
    }
    if (!heading.hasAttribute('tabindex')) heading.setAttribute('tabindex', '-1');
    const reduce = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    card.scrollIntoView?.({ block: 'start', behavior: reduce ? 'auto' : 'smooth' });
    heading.focus({ preventScroll: true });
  }, [openPr, tab, desk.reviews]);

  // The release-notes draft: 202 gives the pending card; its ready card comes after the SSE.
  useEffect(() => {
    if (!notesId) return undefined;
    const onCard = (e: Event) => {
      const d = (e as CustomEvent<DevCardEvent>).detail;
      if (!d || d.id !== notesId || d.status === 'pending') return;
      devApi.card(notesId).then(
        (raw) => {
          const c = asCard(raw);
          if (c) setNotes(c);
          else setNotesError('The draft came back in a shape this app doesn’t know.');
        },
        (err: unknown) =>
          setNotesError(
            err && typeof err === 'object' && 'status' in err && ((err as { status: number }).status === 404 || (err as { status: number }).status === 405)
              ? 'The draft is ready, but this server can’t send it to the app yet. Update the server.'
              : err instanceof Error
                ? err.message
                : 'The draft couldn’t load.',
          ),
      );
    };
    window.addEventListener(DEV_CARD_EVENT, onCard);
    return () => window.removeEventListener(DEV_CARD_EVENT, onCard);
  }, [notesId]);

  const markSeen = useCallback(() => {
    if (marking) return;
    setMarking(true);
    setActionError(null);
    devApi
      .seen(spaceId)
      .then(() => deskStore.load(spaceId))
      .catch((e: unknown) => setActionError(e instanceof Error ? e.message : 'Couldn’t mark them seen.'))
      .finally(() => setMarking(false));
  }, [spaceId, marking]);

  const draftNotes = useCallback(() => {
    setActionError(null);
    setNotesError(null);
    devApi.releaseNotes(spaceId).then(
      (raw) => {
        const c = asCard(raw);
        if (c) setNotes(c);
        else setActionError('The draft came back in a shape this app doesn’t know.');
      },
      (e: unknown) => setActionError(e instanceof Error ? e.message : 'Couldn’t start the draft.'),
    );
  }, [spaceId]);

  const onFinding = useCallback(
    async (cardId: string, findingId: string, action: FindingAction): Promise<ShellCard> => {
      const card = asCard(await devApi.finding(cardId, findingId, action));
      if (!card) throw new Error('The review came back in a shape this app doesn’t know.');
      const reviews = deskStore.get(spaceId).reviews ?? [];
      deskStore._set(spaceId, { reviews: reviews.map((c) => (c.id === card.id ? card : c)) });
      return card;
    },
    [spaceId],
  );

  const onPost = useCallback(
    async (cardId: string): Promise<ShellCard> => {
      const card = asCard(await devApi.post(cardId));
      if (!card) throw new Error('The review came back in a shape this app doesn’t know.');
      const reviews = deskStore.get(spaceId).reviews ?? [];
      deskStore._set(spaceId, { reviews: reviews.map((c) => (c.id === card.id ? card : c)) });
      return card;
    },
    [spaceId],
  );

  const landed = desk.landed;
  const reviews = desk.reviews ?? [];
  const landedCards = (landed?.cards ?? []).filter((c) => c.kind === 'landed');
  const newest = landedCards[0];
  const lastMerge = newest && typeof newest.pr === 'number' ? { pr: newest.pr, merged_at: newest.merged_at ?? null } : null;
  const unseen = landed?.rollup?.count ?? 0;
  const open = openReviews(desk.reviews);
  const first = !landed && !desk.reviews;

  const tabs: Array<{ key: Tab; label: string; count: number }> = [
    { key: 'landed', label: 'What landed', count: unseen },
    { key: 'reviews', label: 'Reviews', count: open },
  ];
  const onTabKey = (e: React.KeyboardEvent, i: number) => {
    const n = e.key === 'ArrowRight' ? i + 1 : e.key === 'ArrowLeft' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : null;
    if (n === null) return;
    e.preventDefault();
    const k = (n + tabs.length) % tabs.length;
    setTab(tabs[k].key);
    tabRefs.current[k]?.focus();
  };

  return (
    <div className="flex-1 flex flex-col h-full overflow-hidden" data-dev-desk>
      <div className={`h-14 shrink-0 border-b border-zinc-800/80 ${onBack ? 'px-2' : 'px-4'} sm:px-6 flex items-center justify-between gap-3 bg-zinc-900/30 backdrop-blur-sm select-none`}>
        {onBack && (
          <button type="button" onClick={onBack} aria-label="Back to chats" className={`shrink-0 p-2 -ml-0.5 -mr-2 rounded-lg text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 ${focusRing}`}>
            <ChevronLeft size={18} />
          </button>
        )}
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">Dev desk</h2>
          <p className="text-[11px] text-zinc-400 truncate mt-0.5">What landed in {spaceName}, and the open pull requests</p>
        </div>
        {dev?.repo && (
          <span className="hidden sm:inline-flex shrink-0 font-mono text-[11px] text-zinc-300 px-2 py-0.5 rounded-md bg-white/[0.04] border border-zinc-700/60" data-dev-repo-chip>
            {dev.repo} · {dev.branch || 'main'}
          </span>
        )}
        {desk.checkedAt != null && (
          <span className="shrink-0 text-[11px] text-zinc-500 tabular-nums" data-dev-checked>
            Checked {checkedLabel(desk.checkedAt)}
          </span>
        )}
        <button
          type="button"
          onClick={() => void deskStore.load(spaceId)}
          disabled={desk.loading}
          aria-label="Refresh"
          className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-300 border border-zinc-700/60 hover:bg-zinc-800 disabled:opacity-60 ${focusRing}`}
        >
          <RefreshCw size={12} className={desk.loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          <span className="hidden sm:inline">Refresh</span>
        </button>
      </div>

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-6" aria-busy={desk.loading}>
        <div className="max-w-[760px] mx-auto space-y-4">
          <div role="tablist" aria-label="Dev desk" className="flex gap-1">
            {tabs.map((t, i) => {
              const on = t.key === tab;
              return (
                <button
                  key={t.key}
                  ref={(el) => (tabRefs.current[i] = el)}
                  type="button"
                  role="tab"
                  id={`${tabsId}-${t.key}`}
                  aria-selected={on}
                  aria-controls={`${tabsId}-panel`}
                  tabIndex={on ? 0 : -1}
                  onKeyDown={(e) => onTabKey(e, i)}
                  onClick={() => setTab(t.key)}
                  data-dev-tab={t.key}
                  className={`inline-flex items-center gap-1.5 min-h-[44px] sm:min-h-[36px] px-3 rounded-lg text-xs font-medium ${focusRing} ${
                    on ? 'bg-zinc-800 text-zinc-100 border border-zinc-700' : 'text-zinc-400 border border-transparent hover:text-zinc-200'
                  }`}
                >
                  {t.label}
                  {t.count > 0 && <span className="min-w-[18px] h-[18px] px-1 rounded-full bg-zinc-700 text-2xs text-zinc-100 flex items-center justify-center tabular-nums">{t.count}</span>}
                </button>
              );
            })}
          </div>

          {(desk.error || actionError) && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-400/30 bg-rose-500/10 flex flex-wrap items-center gap-2 text-xs text-rose-100">
              <span className="min-w-0 flex-1">{actionError ?? desk.error}</span>
              {desk.error && !actionError && (
                <button type="button" onClick={() => void deskStore.load(spaceId)} className={`px-2 py-0.5 rounded border border-rose-400/40 hover:bg-rose-500/10 min-h-[32px] ${focusRing}`}>
                  Try again
                </button>
              )}
            </div>
          )}

          <div ref={panelRef} role="tabpanel" id={`${tabsId}-panel`} aria-labelledby={`${tabsId}-${tab}`} className="space-y-3">
            {first && desk.loading ? (
              <p className="flex items-center gap-2 py-8 justify-center text-meta text-zinc-400" data-dev-loading>
                <Loader2 size={14} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
                Loading the Dev desk…
              </p>
            ) : tab === 'landed' ? (
              <>
                {landed?.rollup && (
                  <Rollup
                    rollup={landed.rollup}
                    last_tag={landed.last_tag}
                    last_merge={lastMerge}
                    cards={landedCards}
                    branch={dev?.branch || 'main'}
                    onMarkSeen={markSeen}
                    onDraftNotes={draftNotes}
                    drafting={!!notes && notes.status === 'pending'}
                    marking={marking}
                  />
                )}
                {landedCards.length ? (
                  <LandedList cards={landedCards} reviews={reviews} now={now} onOpenReview={openReview} forge={dev?.forge ?? null} />
                ) : (
                  landed && (
                    <Empty title="Nothing has landed yet" line="Each merge to main shows up here as a short card in plain words. kwant checks every 5 minutes." />
                  )
                )}
              </>
            ) : reviews.length ? (
              reviews.map((c) => <ReviewCard key={c.id} card={c} onFinding={onFinding} onPost={onPost} forge={dev?.forge ?? null} forgeLogin={dev?.forge_login ?? null} now={now} />)
            ) : (
              desk.reviews && <Empty title="No open pull requests" line="Each open PR gets a review here from a bot that didn’t write it." />
            )}
          </div>
        </div>
      </div>

      {notes && (
        <>
          <ReleaseNotesSheet
            card={notes}
            onSaveToNotion={onSaveToNotion}
            onClose={() => setNotes(null)}
            onDiscard={() => {
              setNotes(null);
              setNotesError(null);
            }}
          />
          {notesError && (
            <p role="alert" className="fixed left-1/2 -translate-x-1/2 bottom-6 z-[60] max-w-[90vw] rounded-lg border border-amber-400/30 bg-surface-overlay px-3 py-2 text-meta text-amber-100">
              {notesError}
            </p>
          )}
        </>
      )}
    </div>
  );
};

const Empty: React.FC<{ title: string; line: string }> = ({ title, line }) => (
  <div className="rounded-2xl border border-dashed border-hairline-strong px-5 py-8 text-center" data-dev-empty>
    <p className="text-body font-medium text-zinc-200">{title}</p>
    <p className="mt-1 text-meta text-zinc-400">{line}</p>
  </div>
);
