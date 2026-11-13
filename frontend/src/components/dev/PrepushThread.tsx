// 0.8.0 B3: where pre-push cards live. One hook per space keeps the list (GET on open, then one
// GET per `dev_prepush` event); the bot's thread shows that bot's cards, and the Inbox shows the
// ones that need the person. The Inbox item is derived here from `needs_you` (the server makes
// none: fresh-eyes-audit.md §4.6, open question 7), so it resolves as soon as the card moves on.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowUpFromLine } from 'lucide-react';
import { PrepushCard } from './PrepushCard';
import {
  PREPUSH_EVENT,
  inboxLine,
  mergeList,
  needsYou,
  prepushApi,
  pressedBefore,
  pushApproval,
  storyOf,
  upsert,
  type PrepushCard as Card,
} from '../../dev/prepush';

export interface PrepushEventDetail {
  space_id: string;
  id: string;
  kwant_id?: string;
  status?: string;
}

export interface PrepushState {
  cards: Card[];
  /** Ids that turned clear while this page was open (P2 shows, then folds). */
  live: ReadonlySet<string>;
  put: (c: Card) => void;
  /** Fetch one card again (after a 409, so the card shows where it really is). */
  refresh: (id: string) => Promise<void>;
  /** 0.9.7: cards the person just retried or overrode. They leave Needs you at once, before the server answers. */
  acted: ReadonlySet<string>;
  /** Mark a card acted on (true) or put it back when the call failed (false). */
  setActed: (id: string, on: boolean) => void;
}

type Api = Pick<typeof prepushApi, 'list' | 'get'>;

export function usePrepushCards(spaceId: string, on: boolean, api: Api = prepushApi): PrepushState {
  const [cards, setCards] = useState<Card[]>([]);
  const [live, setLive] = useState<ReadonlySet<string>>(new Set());
  const [acted, setActedState] = useState<ReadonlySet<string>>(new Set());
  const setActed = useCallback((id: string, on: boolean) => {
    setActedState((s) => {
      if (s.has(id) === on) return s;
      const n = new Set(s);
      if (on) n.add(id);
      else n.delete(id);
      return n;
    });
  }, []);
  const seen = useRef(new Map<string, string>());
  // A list or card load that failed: load the list again on the next event or window focus, so a
  // card waiting for the person doesn't stay hidden until its own next event (Ada Later 1).
  const stale = useRef(false);
  const alive = useRef<() => boolean>(() => false);

  const put = useCallback((c: Card) => {
    const before = seen.current.get(c.id);
    seen.current.set(c.id, c.status);
    if (c.status === 'clear' && before !== undefined && before !== 'clear') setLive((s) => new Set(s).add(c.id));
    setCards((list) => upsert(list, c));
  }, []);

  const refresh = useCallback(
    async (id: string) => {
      const ok = alive.current;
      try {
        const c = await api.get(id);
        if (ok()) put(c);
      } catch {
        stale.current = true;
      }
    },
    [api, put],
  );

  useEffect(() => {
    setCards([]);
    setLive(new Set());
    setActedState(new Set());
    seen.current = new Map();
    stale.current = false;
    if (!on || !spaceId) {
      alive.current = () => false;
      return;
    }
    let on_ = true;
    const isAlive = () => on_;
    alive.current = isAlive;
    let loading = false;
    const load = () => {
      if (loading) return;
      loading = true;
      stale.current = false;
      api.list(spaceId).then(
        (list) => {
          loading = false;
          if (!on_) return;
          for (const c of list) if (!seen.current.has(c.id)) seen.current.set(c.id, c.status);
          setCards((cur) => mergeList(cur, list));
        },
        () => {
          loading = false;
          stale.current = true;
        },
      );
    };
    load();
    const onEvent = (e: Event) => {
      const d = (e as CustomEvent<PrepushEventDetail>).detail;
      if (!d || d.space_id !== spaceId || typeof d.id !== 'string') return;
      if (stale.current) load();
      api.get(d.id).then(
        (c) => on_ && put(c),
        () => {
          stale.current = true;
        },
      );
    };
    const onFocus = () => {
      if (stale.current) load();
    };
    window.addEventListener(PREPUSH_EVENT, onEvent);
    window.addEventListener('focus', onFocus);
    return () => {
      on_ = false;
      if (alive.current === isAlive) alive.current = () => false;
      window.removeEventListener(PREPUSH_EVENT, onEvent);
      window.removeEventListener('focus', onFocus);
    };
  }, [spaceId, on, api, put]);

  return { cards, live, put, refresh, acted, setActed };
}

/** How many of a bot's cards the thread shows: the newest few, oldest first like the chat. */
export const THREAD_CARDS = 3;

export function threadCards(cards: Card[], kwantId: string): Card[] {
  return cards
    .filter((c) => c.kwant_id === kwantId)
    .slice()
    .sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0))
    .slice(0, THREAD_CARDS)
    .reverse();
}

type ApprovalLike = Parameters<typeof pushApproval>[1][number];

/** Scroll to a card in the open chat and ring it, like an Inbox item does (none under reduced motion, in CSS). */
export function jumpToCard(id: string): void {
  const el = document.getElementById(`prepush-${id}`);
  if (!el) return;
  el.scrollIntoView?.({ block: 'center' });
  el.classList.remove('kw-prepush-flash');
  void el.offsetWidth;
  el.classList.add('kw-prepush-flash');
  window.setTimeout(() => el.classList.remove('kw-prepush-flash'), 1300);
}

export const PrepushThread: React.FC<{
  cards: Card[];
  bot: string;
  live: ReadonlySet<string>;
  onRetry?: (c: Card) => Promise<void>;
  onOverride?: (c: Card, reason: string) => Promise<void>;
  /** 0.9.7: every card of the space, so a pressed card can follow the new push its retry led to. */
  all?: Card[];
  /** The chat's pending approvals: a floor or payment approval the pressed push still needs is linked. */
  approvals?: readonly ApprovalLike[];
  onOpenApproval?: (id: string) => void;
}> = ({ cards, bot, live, onRetry, onOverride, all, approvals = [], onOpenApproval }) => {
  if (cards.length === 0) return null;
  const every = all ?? cards;
  return (
    <section aria-label={`Checks before ${bot} pushes`} data-prepush-thread className="space-y-2">
      {cards.map((c) => {
        const story = storyOf(c, every, bot, pushApproval(c, approvals));
        const after = pressedBefore(c, every);
        return (
          <div key={c.id} id={`prepush-${c.id}`} className="relative max-w-[640px] scroll-mt-4 rounded-xl">
            <PrepushCard
              card={c}
              bot={bot}
              live={live.has(c.id)}
              onRetry={onRetry}
              onOverride={onOverride}
              story={story}
              after={after}
              onOpenCard={jumpToCard}
              onOpenApproval={onOpenApproval}
            />
          </div>
        );
      })}
    </section>
  );
};

/** The Inbox's "Needs you" rows for pushes waiting on the person (Mira P4). */
export const PrepushNeedsYou: React.FC<{ cards: Card[]; names: Record<string, string>; onOpen: (c: Card) => void; acted?: ReadonlySet<string> }> = ({
  cards,
  names,
  onOpen,
  acted,
}) => {
  const list = needsYou(cards, acted);
  if (list.length === 0) return null;
  return (
    <section aria-labelledby="needs-prepush-h" data-needs-prepush data-needs-you>
      <h3 id="needs-prepush-h" className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">
        <ArrowUpFromLine size={12} aria-hidden="true" />
        Needs you
      </h3>
      <ul role="list" className="space-y-2">
        {list.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              data-needs-prepush-item={c.id}
              onClick={() => onOpen(c)}
              className="flex min-h-11 w-full items-center gap-2.5 rounded-xl border border-amber-500/30 bg-amber-950/20 px-3.5 py-2.5 text-left text-sm text-zinc-200 outline-none hover:bg-amber-950/30 focus-visible:ring-2 focus-visible:ring-sky-400 sm:px-4"
            >
              <span className="kw-chip kw-chip-warn shrink-0">Needs you</span>
              <span className="min-w-0 flex-1">
                <span className="block [overflow-wrap:anywhere]">{inboxLine(c, names[c.kwant_id] || 'A bot')}</span>
                {/* One item per branch: the branch goes under the line at every width, since it is what tells two items apart (Mira FIX 5). */}
                <code data-needs-branch title={c.branch} className="dv-sha mt-1 inline-block max-w-full truncate align-top">{c.branch}</code>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
};
