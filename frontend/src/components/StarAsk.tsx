import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { ExternalLink, Star } from 'lucide-react';
import { INSTALL_HINT_DISMISSED_KEY, installHintKind, isIos, isStandalone } from '../pwa';
import { useInstall } from '../useInstall';
import { STAR_REPO_URL, StarAskBlockers, StarAskStore, shouldAskForStar, starAskStore } from '../starAsk';

const PHONE_QUERY = '(max-width: 767px)';

const btn =
  'inline-flex min-h-[36px] items-center justify-center gap-1.5 rounded-lg px-3 text-xs font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-amber-300';

interface ViewProps {
  onStar: () => void;
  onLater: () => void;
  onNever: () => void;
}

/** The card itself, without the rules: what tests and screenshots render. */
export const StarAskView: React.FC<ViewProps> = ({ onStar, onLater, onNever }) => (
  <aside
    aria-labelledby="star-ask-title"
    data-star-ask
    className="fixed inset-x-3 z-[55] bottom-[calc(env(safe-area-inset-bottom)+4.25rem)] md:inset-x-auto md:right-4 md:bottom-4 md:w-[22rem] flex items-start gap-3 rounded-xl border border-amber-400/30 bg-zinc-900/95 p-3 shadow-xl text-xs backdrop-blur"
  >
    <img src="/icons/icon-192.png" alt="" width={36} height={36} className="shrink-0 rounded-lg [image-rendering:pixelated]" />
    <div className="flex-1 min-w-0">
      <p id="star-ask-title" className="font-medium text-zinc-100">
        kwant bots run on coffee and GitHub stars <span aria-hidden="true">⭐</span>
      </p>
      <p className="mt-0.5 text-zinc-400 leading-relaxed">If your bots have been handy, toss one our way? It helps other people find kwant bots.</p>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <a
          href={STAR_REPO_URL}
          target="_blank"
          rel="noopener noreferrer"
          onClick={onStar}
          className={`${btn} bg-amber-400 text-zinc-950 hover:bg-amber-300`}
        >
          <Star size={14} className="fill-current" aria-hidden="true" /> Star on GitHub
        </a>
        <button type="button" onClick={onLater} className={`${btn} text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100`}>
          Maybe later
        </button>
        <button type="button" onClick={onNever} className={`${btn} text-zinc-500 hover:bg-zinc-800 hover:text-zinc-300`}>
          Don’t ask again
        </button>
      </div>
    </div>
  </aside>
);

const installHintDismissed = () => {
  try {
    return localStorage.getItem(INSTALL_HINT_DISMISSED_KEY) === '1';
  } catch {
    return false;
  }
};

/** Shows the star card when the rules in starAsk.ts allow it. On a phone it waits for the
 *  "Install on your phone" hint to go, since both sit in the same spot above the tab bar. */
export const StarAsk: React.FC<StarAskBlockers & { store?: StarAskStore }> = ({ needsYou, waiting, store = starAskStore() }) => {
  const state = useSyncExternalStore(store.subscribe, store.get, store.get);
  const { prompt, installed } = useInstall();
  const [phone, setPhone] = useState(() => !!window.matchMedia?.(PHONE_QUERY).matches);
  useEffect(() => {
    const mq = window.matchMedia?.(PHONE_QUERY);
    const onMq = () => setPhone(!!mq?.matches);
    mq?.addEventListener('change', onMq);
    return () => mq?.removeEventListener('change', onMq);
  }, []);

  if (!shouldAskForStar(state, { needsYou, waiting }, Date.now())) return null;
  const installHint = installHintKind({
    standalone: installed || isStandalone(window),
    dismissed: installHintDismissed(),
    phone,
    secure: window.isSecureContext,
    ios: isIos(navigator),
    canPrompt: !!prompt,
  });
  if (installHint) return null;

  return <StarAskView onStar={store.starred} onLater={store.later} onNever={store.never} />;
};

/** The quiet, always-there link in App settings. */
export const StarAskSettingsLink: React.FC<{ store?: StarAskStore }> = ({ store = starAskStore() }) => (
  <a
    href={STAR_REPO_URL}
    target="_blank"
    rel="noopener noreferrer"
    onClick={store.starred}
    data-star-settings
    className="flex items-center gap-3 bg-zinc-950/60 p-3 rounded-xl border border-zinc-800 text-xs font-medium text-zinc-200 hover:border-amber-400/40 hover:text-zinc-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-300"
  >
    <Star size={15} className="text-amber-300" aria-hidden="true" />
    <span className="flex-1">
      Star kwant bots on GitHub<span className="sr-only"> (opens in your browser)</span>
    </span>
    <ExternalLink size={13} className="text-zinc-500" aria-hidden="true" />
  </a>
);
