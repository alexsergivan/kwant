import React, { useEffect, useRef, useState } from 'react';
import { claimFirst, claimQuietly, FIRST_LINE, FIRST_STICKER, type FirstKind } from '../signature';
import { ShieldCheck, Target } from 'lucide-react';
import { PixelConfetti } from './Signature';
import { Sticker } from './Sticker';

const reducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

/**
 * Once per space: the first time one of these items reaches its goal state while this screen is open
 * (a task verified, a goal met). A space that already had one at load is not new, so it is claimed
 * without a celebration. Returns the item id to celebrate, or null. Under reduced motion it claims and shows nothing.
 */
export function useFirstMoment(kind: FirstKind, spaceId: string, items: Array<{ id: string; reached: boolean }> | null): string | null {
  const seen = useRef<{ space: string; map: Map<string, boolean> } | null>(null);
  const [shown, setShown] = useState<string | null>(null);
  const key = items ? items.map((i) => `${i.id}:${i.reached ? 1 : 0}`).join(',') : null;
  const latest = useRef(items);
  latest.current = items;
  useEffect(() => {
    const list = latest.current;
    if (!list) return;
    const prev = seen.current;
    if (!prev || prev.space !== spaceId) {
      if (list.some((i) => i.reached)) claimQuietly(kind, spaceId);
      seen.current = { space: spaceId, map: new Map(list.map((i) => [i.id, i.reached])) };
      setShown(null);
      return;
    }
    for (const i of list) {
      const was = prev.map.get(i.id);
      prev.map.set(i.id, i.reached);
      if (i.reached && was === false && claimFirst(kind, spaceId, i.id) && !reducedMotion()) setShown(i.id);
    }
  }, [key, spaceId, kind]);
  return shown;
}

/** The one line that goes with the first-time sticker; dismissable, never a toast. */
export const FirstMomentLine: React.FC<{ kind: FirstKind; onDone: () => void }> = ({ kind, onDone }) => (
  <div
    role="status"
    data-first-moment={kind}
    className="relative flex items-center gap-3 rounded-xl border border-zinc-700/60 bg-zinc-900/70 px-3 py-2 shadow-[var(--kws-shadow)]"
  >
    <span className="relative inline-flex kws-stamp">
      <Sticker tone="holo" icon={kind === 'verified' ? <ShieldCheck size={12} aria-hidden="true" /> : <Target size={12} aria-hidden="true" />}>{FIRST_STICKER[kind]}</Sticker>
      <PixelConfetti n={1} />
    </span>
    <p className="min-w-0 flex-1 text-xs text-zinc-200">{FIRST_LINE[kind]}</p>
    <button
      type="button"
      onClick={onDone}
      className="shrink-0 min-h-8 rounded-lg px-2.5 text-xs font-medium text-zinc-300 hover:bg-zinc-800 hover:text-zinc-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
    >
      Got it
    </button>
  </div>
);
