// The Dev desk: a row pinned at the top of the sidebar in a Dev space, and its page with two tabs,
// What landed and Reviews. The page fetches plan §3.2's routes, updates live on the `dev_card`
// SSE (App re-sends it as DEV_CARD_EVENT) and mounts Iris's cards (#314) through DevDeskSlots.

import React, { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { ChevronLeft, Code, RefreshCw } from 'lucide-react';
import { DEV_OFF, DevModeState, checkedLabel, DevTeamRef, cardRepoLabel, cardsOfRepo, devApi, devStore, deskStatus, firstRun, repoFilters, reposSummary, statusLine } from '../devMode';
import { FindingAction, LandedPage, ShellCard, asCard, parseCards, parseLandedPage } from '../dev/shellTypes';
import { LandedList, ReleaseNotesSheet, ReviewCard, Rollup } from './DevDeskSlots';
import { useDevMode } from './DevModeSwitch';
import { DevStatusLine } from './DevStatusLine';

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
/** The cache key: the space, or the space and one repo (0.7.2 `?repo=`). The row reads the space's. */
const deskKey = (space: string, repo: string | null = null) => (repo ? `${space}\u0000${repo}` : space);
const deskListeners = new Set<() => void>();
const inflight = new Map<string, Promise<void>>();

function setDesk(space: string, patch: Partial<DeskData>) {
  desks.set(space, { ...(desks.get(space) ?? EMPTY), ...patch });
  deskListeners.forEach((l) => l());
}

export const deskStore = {
  get: (space: string, repo: string | null = null): DeskData => desks.get(deskKey(space, repo)) ?? EMPTY,
  subscribe(l: () => void) {
    deskListeners.add(l);
    return () => void deskListeners.delete(l);
  },
  /** Loads both lists, of the whole space or of one repo; concurrent calls share one request. */
  load(space: string, repo: string | null = null): Promise<void> {
    const key = deskKey(space, repo);
    const running = inflight.get(key);
    if (running) return running;
    setDesk(key, { loading: true });
    const p = Promise.all([devApi.landed(space, undefined, repo), devApi.reviews(space, repo)])
      .then(([l, r]) => {
        const page = parseLandedPage(l);
        setDesk(key, { landed: { ...page, cards: cardsOfRepo(page.cards, repo) }, reviews: cardsOfRepo(parseCards(r), repo), loading: false, error: null, checkedAt: Date.now() });
      })
      .catch((e: unknown) => setDesk(key, { loading: false, error: e instanceof Error ? e.message : 'The Dev desk couldn’t load.' }))
      .finally(() => inflight.delete(key));
    inflight.set(key, p);
    return p;
  },
  /** Tests and shots: `space`, or `space` and a repo key. */
  _set: (space: string, patch: Partial<DeskData>, repo: string | null = null) => setDesk(deskKey(space, repo), patch),
  _reset() {
    desks.clear();
    inflight.clear();
  },
};

/** A finding or post changed a review card: the space's list and the filtered one both show it. */
function patchReview(space: string, repo: string | null, card: ShellCard) {
  for (const r of repo ? [null, repo] : [null]) {
    const list = deskStore.get(space, r).reviews;
    if (list) deskStore._set(space, { reviews: list.map((c) => (c.id === card.id ? card : c)) }, r);
  }
}

/** "14:32", 24-hour local time (now in devMode.ts, shared with the repo rows). */
export { checkedLabel };

export function useDesk(space: string, on: boolean, repo: string | null = null): DeskData {
  const data = useSyncExternalStore(deskStore.subscribe, () => deskStore.get(space, repo), () => EMPTY);
  useEffect(() => {
    if (!on) return undefined;
    void deskStore.load(space, repo);
    const onCard = (e: Event) => {
      const d = (e as CustomEvent<DevCardEvent>).detail;
      if (d?.space_id === space && d.kind !== 'release_notes') void deskStore.load(space, repo);
    };
    window.addEventListener(DEV_CARD_EVENT, onCard);
    return () => window.removeEventListener(DEV_CARD_EVENT, onCard);
  }, [space, on, repo]);
  return data;
}

export const openReviews = (reviews: ShellCard[] | null) => (reviews ?? []).filter((c) => c.status !== 'stale').length;

/** The Dev desk row badge (flow F6): new review cards from the server, else the open reviews. */
export const rowBadge = (dev: DevModeState | null, openReviewCount: number): number => dev?.new_reviews ?? openReviewCount;

/** Flow F4: while the first poll reads the last merges, the desk shows still placeholder cards. */
export const firstPolling = (dev: DevModeState | null, landedCount: number): boolean =>
  !!dev?.on && dev.status === 'watching' && !dev.polled_at && !dev.repos.some((r) => r.polled_at) && landedCount === 0;

// ---------------------------------------------------------------------------
// The sidebar row

export const DevDeskRow: React.FC<{ spaceId: string; active: boolean; onOpen: () => void; className?: string }> = ({ spaceId, active, onOpen, className }) => {
  const desk = useDesk(spaceId, true);
  const dev = useDevMode(spaceId);
  const landed = desk.landed?.rollup?.count ?? 0;
  const reviews = openReviews(desk.reviews);
  // Flow F6: the badge is review cards new since the desk was last opened (the server keeps "seen",
  // so phone and desktop agree). Landed cards are news, not needs; floor approvals stay in the chat.
  // An older server without new_reviews falls back to the open reviews.
  const count = rowBadge(dev, reviews);
  // Flow F4: one soft highlight the first time the row appears in this space; never again, never looping.
  const [fresh] = useState(() => !firstRun.rowSeen(spaceId));
  useEffect(() => {
    firstRun.claimRow(spaceId);
  }, [spaceId]);
  // Flow copy §4: one 160 ms pop when the count goes up, never when it goes down.
  const prev = useRef(count);
  const [pop, setPop] = useState(0);
  useEffect(() => {
    if (count > prev.current) setPop((n) => n + 1);
    prev.current = count;
  }, [count]);
  const name = count > 0 ? `Dev desk, ${count} new ${count === 1 ? 'review' : 'reviews'}` : undefined;
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-current={active ? 'page' : undefined}
      aria-label={name}
      data-dev-desk-row
      data-dev-row-fresh={fresh || undefined}
      className={`${
        className ??
        `w-full flex items-center gap-2.5 rounded-xl px-2.5 py-1.5 text-left ${focusRing} ${active ? 'bg-accent-soft text-zinc-50' : 'text-zinc-300 hover:bg-white/[0.05] hover:text-zinc-50'}`
      }${fresh ? ' kw-dv-row-fresh' : ''}`}
    >
      <span className="w-7 h-7 shrink-0 rounded-lg bg-accent-soft text-sky-300 flex items-center justify-center" aria-hidden="true">
        <Code size={14} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-body font-medium truncate">
          Dev desk<span className="sr-only">, </span>
        </span>
        <span className="block text-2xs text-zinc-400 truncate" data-dev-desk-status>
          {desk.landed || desk.reviews ? deskStatus(landed, reviews) : desk.error ? 'Couldn’t load' : 'Loading…'}
        </span>
      </span>
      {count > 0 && (
        <span
          key={pop}
          className={`kw-dv-row-badge shrink-0${pop ? ' kw-dv-row-badge-pop' : ''}`}
          aria-hidden="true"
          title={`${count === 1 ? '1 new review' : `${count} new reviews`} to look at`}
          data-dev-row-badge
        >
          {count > 9 ? '9+' : count}
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
  /** Opens Loose ends, for the status line's Connect. */
  onOpenLooseEnds?: () => void;
  /** The space's teams, so a team with no repo gets "Link a repo" in the repo list. */
  teams?: DevTeamRef[];
}

export const DevDeskView: React.FC<DevDeskViewProps> = ({ spaceId, onBack, onSaveToNotion, initialTab = 'landed', now, onOpenLooseEnds, teams }) => {
  const dev = useDevMode(spaceId);
  // 0.7.2: with several repos, both lists filter by one (All · app · site) through `?repo=`.
  const filters = repoFilters(dev);
  const [repoPick, setRepoPick] = useState<string | null>(null);
  const repo = repoPick && filters.some((f) => f.key === repoPick) ? repoPick : null;
  const desk = useDesk(spaceId, true, repo);
  const repoLabel = useCallback((key: string | null, name?: string | null) => cardRepoLabel(dev, key, name), [dev]);
  const [tab, setTab] = useState<Tab>(initialTab);
  // Flow F6: opening the desk clears the row badge on every device.
  useEffect(() => {
    const s = devStore.get(spaceId);
    if (s?.new_reviews) devStore.set(spaceId, { ...s, new_reviews: 0 });
    devApi.deskSeen(spaceId).catch(() => undefined);
  }, [spaceId]);
  // Flow F4: what Dev mode does, on the first visit only; it can be closed and stays gone.
  const [intro, setIntro] = useState(() => !firstRun.introSeen(spaceId));
  // Flow copy §6: on the first visit the empty states speak in git terms and never say "Nothing has landed yet".
  const [firstVisit] = useState(() => !firstRun.introSeen(spaceId));
  useEffect(() => {
    firstRun.claimIntro(spaceId);
  }, [spaceId]);
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
  const [openPr, setOpenPr] = useState<{ pr: number; repo: string | null } | null>(null);

  const openReview = useCallback((pr: number, repoKey: string | null = null) => {
    setTab('reviews');
    setOpenPr({ pr, repo: repoKey });
  }, []);

  useEffect(() => {
    if (openPr === null || tab !== 'reviews') return;
    const panel = panelRef.current;
    // The same PR number can be open in two repos: the landed card's repo picks which.
    const card =
      (openPr.repo ? panel?.querySelector<HTMLElement>(`[data-dev-card="review"][data-pr="${openPr.pr}"][data-repo="${openPr.repo.replace(/["\\]/g, '')}"]`) : null) ??
      panel?.querySelector<HTMLElement>(`[data-dev-card="review"][data-pr="${openPr.pr}"]`);
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
      .then(() => Promise.all([deskStore.load(spaceId), repo ? deskStore.load(spaceId, repo) : undefined]))
      .catch((e: unknown) => setActionError(e instanceof Error ? e.message : 'Couldn’t mark them seen.'))
      .finally(() => setMarking(false));
  }, [spaceId, marking, repo]);

  const draftNotes = useCallback(() => {
    setActionError(null);
    setNotesError(null);
    devApi.releaseNotes(spaceId, repo).then(
      (raw) => {
        const c = asCard(raw);
        if (c) setNotes(c);
        else setActionError('The draft came back in a shape this app doesn’t know.');
      },
      (e: unknown) => setActionError(e instanceof Error ? e.message : 'Couldn’t start the draft.'),
    );
  }, [spaceId, repo]);

  const onFinding = useCallback(
    async (cardId: string, findingId: string, action: FindingAction): Promise<ShellCard> => {
      const card = asCard(await devApi.finding(cardId, findingId, action));
      if (!card) throw new Error('The review came back in a shape this app doesn’t know.');
      patchReview(spaceId, repo, card);
      return card;
    },
    [spaceId, repo],
  );

  const onPost = useCallback(
    async (cardId: string): Promise<ShellCard> => {
      const card = asCard(await devApi.post(cardId));
      if (!card) throw new Error('The review came back in a shape this app doesn’t know.');
      patchReview(spaceId, repo, card);
      return card;
    },
    [spaceId, repo],
  );

  const landed = desk.landed;
  const reviews = desk.reviews ?? [];
  const landedCards = (landed?.cards ?? []).filter((c) => c.kind === 'landed');
  const newest = landedCards[0];
  const lastMerge = newest && typeof newest.pr === 'number' ? { pr: newest.pr, merged_at: newest.merged_at ?? null } : null;
  const unseen = landed?.rollup?.count ?? 0;
  // Style-v2 review (Mira, 23:30): the rollup, the tab count and the list must agree.
  // No cards and a zero rollup: the empty card alone says it (no merges yet, or nothing new); a
  // "Nothing landed since you last looked" rollup above it would only repeat it.
  const nothingLanded = !landedCards.length && unseen === 0;
  const rollupSaysLanded = unseen > 0 && !landedCards.length;
  const showRollup =
    dev?.status !== 'not_connected' && dev?.status !== 'no_repo' && !firstVisit && !nothingLanded && !rollupSaysLanded;
  const open = openReviews(desk.reviews);
  const first = !landed && !desk.reviews;
  const polling = firstPolling(dev, landedCards.length);
  const statusShown = statusLine(dev) !== null;

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
        <span className="kw-dv-glyph" aria-hidden="true">
          &lt;/&gt;
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-[17px] leading-6 font-semibold text-zinc-50 truncate">Dev desk</h2>
          <p className="hidden sm:block text-meta text-zinc-400 truncate" data-dev-desk-sub>
            {dev && !dev.on ? DEV_OFF : 'What landed and what needs a look'}
          </p>
        </div>
        {dev?.repo && !dev.status && (
          <span className="hidden sm:inline-flex shrink-0 font-mono text-[11px] text-zinc-300 px-2 py-0.5 rounded-md bg-white/[0.04] border border-zinc-700/60" data-dev-repo-chip>
            {dev.repo} · {dev.branch || 'main'}
          </span>
        )}
        {/* An older server has no status line, so "Checked" stays up here for it. */}
        {desk.checkedAt != null && !statusShown && (
          <span className="shrink-0 text-[11px] text-zinc-500 tabular-nums" data-dev-checked>
            Checked {checkedLabel(desk.checkedAt)}
          </span>
        )}
        <button
          type="button"
          onClick={() => void deskStore.load(spaceId, repo)}
          disabled={desk.loading}
          aria-label="Refresh"
          data-dev-refresh
          className="kw-btn-ghost shrink-0 max-sm:w-11 max-sm:h-11 max-sm:px-0 disabled:opacity-60"
        >
          <RefreshCw size={14} className={desk.loading ? 'animate-spin motion-reduce:animate-none' : ''} aria-hidden="true" />
          <span className="hidden sm:inline">Refresh</span>
        </button>
      </div>
      <DevStatusLine
        spaceId={spaceId}
        state={dev}
        teams={teams}
        onConnect={onOpenLooseEnds}
        checked={desk.checkedAt != null ? checkedLabel(desk.checkedAt) : null}
        checkedAt={desk.checkedAt ?? null}
      />

      <div className="flex-1 overflow-y-auto overflow-x-hidden p-4 sm:p-6" aria-busy={desk.loading}>
        <div className="max-w-[760px] mx-auto space-y-4">
          {intro && dev?.on && (
            <DevIntro cmd="kwant dev --on" repo={(dev.status ?? (dev.repo ? 'watching' : 'no_repo')) !== 'watching' ? null : dev.repos.length > 1 ? reposSummary(dev).replace(/^Watching /, '') : dev.repos[0]?.repo ?? dev.repo} onClose={() => setIntro(false)} />
          )}
          <div role="tablist" aria-label="Dev desk" className="kw-dv-tabs">
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
                  className={`kw-dv-tab ${focusRing}`}
                >
                  {t.label}
                  {t.count > 0 && <span className="kw-dv-tab-count">{t.count}</span>}
                </button>
              );
            })}
          </div>

          {filters.length > 1 && (
            <div role="group" aria-label="Show cards from" className="kw-dv-filters" data-dev-repo-filters>
              {[{ key: null as string | null, label: 'All', repo: 'All repos' }, ...filters].map((f) => (
                <button
                  key={f.key ?? 'all'}
                  type="button"
                  aria-pressed={repo === f.key}
                  title={f.key ? f.repo : undefined}
                  onClick={() => setRepoPick(f.key)}
                  data-dev-repo-filter={f.key ?? 'all'}
                  className={`kw-dv-filter ${focusRing}`}
                >
                  {f.label}
                </button>
              ))}
            </div>
          )}

          {(desk.error || actionError) && (
            <div role="alert" className="px-3 py-2 rounded-lg border border-rose-400/30 bg-rose-500/10 flex flex-wrap items-center gap-2 text-xs text-rose-100">
              <span className="min-w-0 flex-1">{actionError ?? desk.error}</span>
              {desk.error && !actionError && (
                <button type="button" onClick={() => void deskStore.load(spaceId, repo)} className={`px-2 py-0.5 rounded border border-rose-400/40 hover:bg-rose-500/10 min-h-[32px] ${focusRing}`}>
                  Try again
                </button>
              )}
            </div>
          )}

          <div ref={panelRef} role="tabpanel" id={`${tabsId}-panel`} aria-labelledby={`${tabsId}-${tab}`} className="space-y-3">
            {tab === 'landed' && polling ? (
              <SkeletonCards />
            ) : first && desk.loading ? (
              // Flow copy §6 on the first visit; style-v2 §9 "Reading main" with 2 cards on later loads.
              firstVisit ? <SkeletonCards /> : <SkeletonCards line="Reading main" n={2} />
            ) : tab === 'landed' ? (
              <>
                {/* No repo or no connection: the empty below says why; a "nothing landed" rollup would contradict it.
                    First visit: there is no "last look" yet; the first-run block and the empty say it all.
                    A non-zero rollup only shows with the cards it counts. */}
                {showRollup && landed?.rollup && (
                  <Rollup
                    rollup={landed.rollup}
                    last_tag={landed.last_tag}
                    last_merge={lastMerge}
                    cards={landedCards}
                    branch={(repo ? dev?.repos.find((r) => r.key === repo)?.branch : null) || dev?.branch || dev?.repos[0]?.branch || 'main'}
                    onMarkSeen={markSeen}
                    onDraftNotes={draftNotes}
                    drafting={!!notes && notes.status === 'pending'}
                    marking={marking}
                  />
                )}
                {landedCards.length ? (
                  <LandedList cards={landedCards} reviews={reviews} now={now} onOpenReview={openReview} forge={dev?.forge ?? null} repoLabel={repoLabel} />
                ) : dev?.status === 'not_connected' ? (
                  <TermEmpty out="(needs a GitHub connection)" title="Connect GitHub to see what landed." line="The last 5 merges show up here right after." />
                ) : dev?.status === 'no_repo' ? (
                  <TermEmpty out="(no repo yet)" title="Cards show up once there’s a repo." line="The production floor already works." />
                ) : (
                  landed &&
                  !rollupSaysLanded &&
                  (firstVisit ? (
                    <TermEmpty out="(no merges yet)" title="No merges on main yet." line="They show up here as short cards in plain words." />
                  ) : (
                    <TermEmpty
                      cmd='git log main --since="your last look"'
                      out="(nothing new)"
                      title="Nothing new on main"
                      line="Merges show up here as short cards in plain words. kwant checks every 5 minutes."
                    />
                  ))
                )}
              </>
            ) : reviews.length ? (
              reviews.map((c) => <ReviewCard key={c.id} card={c} onFinding={onFinding} onPost={onPost} forge={dev?.forge ?? null} forgeLogin={dev?.forge_login ?? null} now={now} repoLabel={repoLabel} />)
            ) : (
              desk.reviews && (
                <TermEmpty
                  cmd="kwant reviews --open"
                  out="0 open changes"
                  title="No open changes to review"
                  line="When a bot opens a pull request, a second bot reads it and the card lands here."
                />
              )
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

/** Still placeholders (no shimmer, style v2 §9) while the first poll reads the last 5 merges. */
const SkeletonCards: React.FC<{ line?: string; n?: number }> = ({ line = 'Reading the last 5 merges…', n = 3 }) => (
  <div data-dev-skeleton data-dev-loading>
    <p role="status" className="flex items-center gap-2 pb-1 font-mono text-[12px] text-zinc-300">
      <span className="dv-led" data-s="run" aria-hidden="true" />
      {line}
      <span className="kw-dv-intro-caret" aria-hidden="true" />
    </p>
    <div className="space-y-3" aria-hidden="true">
      {[0, 1, 2].slice(0, n).map((i) => (
        <div key={i} className="kw-dv-skel rounded-2xl border border-hairline px-4 py-3.5">
          <div className="flex items-center gap-2">
            <span className="kw-dv-skel-bar w-12 h-3" />
            <span className="kw-dv-skel-bar w-20 h-3" />
          </div>
          <span className="kw-dv-skel-bar block mt-3 h-3.5" style={{ width: `${78 - i * 12}%` }} />
          <span className="kw-dv-skel-bar block mt-2 h-3" style={{ width: `${56 - i * 8}%` }} />
        </div>
      ))}
    </div>
  </div>
);

/** Flow copy §6: a first-visit empty state as a tiny terminal, `$ git log main -5` and its output. */
const TermEmpty: React.FC<{ cmd?: string; out: string; title: string; line: string }> = ({ cmd = 'git log main -5', out, title, line }) => (
  <div className="rounded-2xl border border-dashed border-hairline-strong px-5 py-6" data-dev-empty data-dev-empty-term>
    <p className="font-mono text-[12px] text-zinc-500 break-words" aria-hidden="true">
      <span className="text-sky-300">$</span> {cmd}
      <br />
      {out}
      <span className="kw-dv-intro-caret" />
    </p>
    <p className="mt-3 text-body font-medium text-zinc-200">{title}</p>
    <p className="mt-1 text-meta text-zinc-400">{line}</p>
  </div>
);

// ---------------------------------------------------------------------------
// First run (flow copy §5): a terminal-style block above the cards on the first visit only. Got it hides
// it for good; it is the only control.

export const DEV_FEATURES = [
  { key: 'landed', n: '01', title: 'What landed.', line: 'Every merge to main in plain words, and when main is ready to ship.' },
  { key: 'reviews', n: '02', title: 'Reviews.', line: 'A second bot reads each pull request before you merge it.' },
  { key: 'floor', n: '03', title: 'Production floor.', line: 'Deploys and live-database commands wait for your OK.' },
] as const;

// cmd is the terminal line the intro types out (copy.md §5), not prose.
const DevIntro: React.FC<{ repo: string | null; onClose: () => void; cmd: string }> = ({ repo, onClose, cmd }) => (
  <section className="kw-dv-intro relative" aria-labelledby="dv-intro-h" data-dev-intro>
    <p className="font-mono text-[11px] leading-relaxed text-zinc-500" aria-hidden="true">
      <span className="text-sky-300">$</span> {cmd}
      <br />
      <span className="text-sky-300">HEAD → dev</span> · {repo ? `watching ${repo}` : 'guards up'}
      <span className="kw-dv-intro-caret" />
    </p>
    <h3 id="dv-intro-h" className="mt-2 text-body font-semibold text-zinc-100">
      Here’s what Dev mode does
    </h3>
    <ol className="mt-2.5 flex flex-col gap-2">
      {DEV_FEATURES.map((f) => (
        <li key={f.key} className="flex items-baseline gap-2.5 text-meta" data-dev-feature={f.key}>
          <span className="font-mono text-[11px] font-semibold text-sky-300" aria-hidden="true">
            {f.n}
          </span>
          <span className="min-w-0 flex-1 text-zinc-400">
            <strong className="font-semibold text-zinc-100">{f.title}</strong> {f.line}
          </span>
        </li>
      ))}
    </ol>
    <button
      type="button"
      onClick={onClose}
      data-dev-intro-close
      className={`mt-2.5 -ml-1 inline-flex items-center min-h-[44px] sm:min-h-[28px] px-2 rounded-lg text-meta font-semibold text-sky-300 hover:bg-sky-500/10 ${focusRing}`}
    >
      Got it
    </button>
  </section>
);
