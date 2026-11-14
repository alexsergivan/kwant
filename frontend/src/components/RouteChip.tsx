import React, { useState } from 'react';
import { Loader2 } from 'lucide-react';

export interface Reroute {
  /** The reply's id, for element ids. */
  key: string;
  targets: Array<{ id: string; name: string; title: string }>;
  onPick: (kwantId: string) => Promise<string | void>;
}

export interface RouteInfo {
  /** Why this bot answered, in words (routeWhyText): Details' "Why {Name}" row. */
  text: string;
  fallback?: boolean;
  reroute?: Reroute;
}

/**
 * Why this bot answered, for a reply's Details (reply-chrome part 4: no chip on the name line). Returns the
 * sentence, the "Wrong bot?" button that goes after it, and the panel (the "Send to another bot" picker and
 * its result) that goes under the Details facts. With `reroute`, picking a bot teaches routing and asks it.
 */
export function useRouteChip(route: RouteInfo | null | undefined): { reason: string | null; wrongBot: React.ReactNode; panel: React.ReactNode } {
  const [picking, setPicking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sentTo, setSentTo] = useState<string | null>(null);
  if (!route) return { reason: null, wrongBot: null, panel: null };

  const { reroute } = route;
  const pickerId = reroute ? `reroute-${reroute.key}` : undefined;
  const canReroute = !!reroute && reroute.targets.length > 0;

  const pick = async (k: { id: string; name: string }) => {
    if (!reroute || busy) return;
    setBusy(k.id);
    setError(null);
    const err = await reroute.onPick(k.id);
    setBusy(null);
    if (err) setError(err);
    else {
      setSentTo(k.name);
      setPicking(false);
    }
  };

  const wrongBot =
    canReroute && !sentTo ? (
      <button
        type="button"
        data-compact
        data-wrong-bot=""
        onClick={() => setPicking((v) => !v)}
        aria-expanded={picking}
        aria-controls={picking ? pickerId : undefined}
        className="ml-1 inline rounded px-1 text-xs text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 underline decoration-dotted underline-offset-2 transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
      >
        Wrong bot?
      </button>
    ) : null;

  const panel = (
    <>
      {picking && reroute && (
        <div id={pickerId} role="group" aria-label="Send to another bot" className="mb-2 max-w-full rounded-xl border border-zinc-700/70 bg-zinc-900/80 px-3 py-2.5 shadow-[var(--kws-shadow)]">
          <p className="mb-2 text-xs text-zinc-300">Send your message to:</p>
          <div className="flex flex-wrap gap-1.5">
            {reroute.targets.map((k) => (
              <button
                key={k.id}
                type="button"
                disabled={busy !== null}
                onClick={() => pick(k)}
                title={k.title}
                className="inline-flex min-h-[36px] items-center gap-1.5 rounded-lg border border-zinc-600 bg-zinc-800 px-3 text-xs text-zinc-100 hover:bg-zinc-700 disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
              >
                {busy === k.id && <Loader2 size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                <span>{k.name}</span>
                {k.title && <span className="hidden text-zinc-400 sm:inline">· {k.title}</span>}
              </button>
            ))}
            <button
              type="button"
              onClick={() => setPicking(false)}
              className="min-h-[36px] rounded-lg px-3 text-xs text-zinc-300 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
            >
              Cancel
            </button>
          </div>
          <p className="mt-2 text-xs text-zinc-400">Messages like this will go to the bot you pick from now on.</p>
          {error && (
            <p role="alert" className="mt-1 break-words text-xs text-rose-300">
              {error}
            </p>
          )}
        </div>
      )}
      {sentTo && (
        <p role="status" className="mb-1.5 text-xs text-emerald-300">
          Sent to {sentTo}. Messages like this will go to {sentTo} from now on.
        </p>
      )}
    </>
  );

  return { reason: route.text, wrongBot, panel };
}
