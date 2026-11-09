// 0.8.0 B3: where pre-push cards live. One hook per space keeps the list (GET on open, then one
// GET per `dev_prepush` event); the bot's thread shows that bot's cards, and the Inbox shows the
// ones that need the person. The Inbox item is derived here from `needs_you` (the server makes
// none: fresh-eyes-audit.md §4.6, open question 7), so it resolves as soon as the card moves on.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowUpFromLine } from 'lucide-react';
import { PrepushCard } from './PrepushCard';
import { PREPUSH_EVENT, inboxLine, needsYou, prepushApi, upsert, type PrepushCard as Card } from '../../dev/prepush';

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
}

type Api = Pick<typeof prepushApi, 'list' | 'get'>;

export function usePrepushCards(spaceId: string, on: boolean, api: Api = prepushApi): PrepushState {
  const [cards, setCards] = useState<Card[]>([]);
  const [live, setLive] = useState<ReadonlySet<string>>(new Set());
  const seen = useRef(new Map<string, string>());

  const put = useCallback((c: Card) => {
    const before = seen.current.get(c.id);
    seen.current.set(c.id, c.status);
    if (c.status === 'clear' && before !== undefined && before !== 'clear') setLive((s) => new Set(s).add(c.id));
    setCards((list) => upsert(list, c));
  }, []);

  useEffect(() => {
    setCards([]);
    setLive(new Set());
    seen.current = new Map();
    if (!on || !spaceId) return;
    let alive = true;
    api.list(spaceId).then(
      (list) => {
        if (!alive) return;
        for (const c of list) seen.current.set(c.id, c.status);
        setCards(list);
      },
      () => undefined,
    );
    const onEvent = (e: Event) => {
      const d = (e as CustomEvent<PrepushEventDetail>).detail;
      if (!d || d.space_id !== spaceId || typeof d.id !== 'string') return;
      api.get(d.id).then((c) => alive && put(c), () => undefined);
    };
    window.addEventListener(PREPUSH_EVENT, onEvent);
    return () => {
      alive = false;
      window.removeEventListener(PREPUSH_EVENT, onEvent);
    };
  }, [spaceId, on, api, put]);

  return { cards, live, put };
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

export const PrepushThread: React.FC<{
  cards: Card[];
  bot: string;
  live: ReadonlySet<string>;
  onRetry?: (c: Card) => Promise<void>;
  onOverride?: (c: Card, reason: string) => Promise<void>;
}> = ({ cards, bot, live, onRetry, onOverride }) => {
  if (cards.length === 0) return null;
  return (
    <section aria-label={`Checks before ${bot} pushes`} data-prepush-thread className="space-y-2">
      {cards.map((c) => (
        <div key={c.id} id={`prepush-${c.id}`} className="scroll-mt-4">
          <PrepushCard card={c} bot={bot} live={live.has(c.id)} onRetry={onRetry} onOverride={onOverride} />
        </div>
      ))}
    </section>
  );
};

/** The Inbox's "Needs you" rows for pushes waiting on the person (Mira P4). */
export const PrepushNeedsYou: React.FC<{ cards: Card[]; names: Record<string, string>; onOpen: (c: Card) => void }> = ({ cards, names, onOpen }) => {
  const list = needsYou(cards);
  if (list.length === 0) return null;
  return (
    <section aria-labelledby="needs-prepush-h" data-needs-prepush>
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
              <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">{inboxLine(c, names[c.kwant_id] || 'A bot')}</span>
              <code className="dv-sha max-w-[40%] shrink truncate max-sm:hidden">{c.branch}</code>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
};
