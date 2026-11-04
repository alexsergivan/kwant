import React, { useState } from 'react';
import { Loader2, Sparkles } from 'lucide-react';

export interface Reroute {
  /** The reply's id, for element ids. */
  key: string;
  targets: Array<{ id: string; name: string; title: string }>;
  onPick: (kwantId: string) => Promise<string | void>;
}

export interface RouteInfo {
  /** The two or three words on the chip ("Matched skills"). */
  label: string;
  /** The whole reason, shown when the chip is opened and as the chip's tooltip. */
  text: string;
  fallback?: boolean;
  reroute?: Reroute;
}

/**
 * Why this bot answered, split in two so the header can put the chip on its own line next to the name
 * while the opened reason and the "Send to another bot" picker sit under the header. With `reroute`,
 * "Wrong bot?" opens the other bots; picking one teaches routing and asks that bot.
 */
export function useRouteChip(route: RouteInfo | null | undefined): { chip: React.ReactNode; panel: React.ReactNode } {
  const [open, setOpen] = useState(false);
  const [picking, setPicking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sentTo, setSentTo] = useState<string | null>(null);
  if (!route) return { chip: null, panel: null };

  const { reroute } = route;
  const warn = !!route.fallback;
  const reasonId = `why-${reroute?.key ?? 'reply'}`;
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

  const chip = (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1" data-routing-line>
      <button
        type="button"
        data-compact
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-controls={open ? reasonId : undefined}
        title={route.text}
        className={`inline-flex min-h-6 max-w-full min-w-0 items-center gap-1 rounded-full border px-2 text-[11px] font-medium transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 ${
          warn
            ? 'border-amber-400/40 bg-amber-400/10 text-amber-200 hover:bg-amber-400/15'
            : 'border-sky-400/35 bg-sky-400/10 text-sky-200 hover:bg-sky-400/15'
        }`}
      >
        <Sparkles size={11} className="shrink-0" aria-hidden="true" />
        <span className="truncate">{route.label}</span>
        <span className="sr-only">: why this bot answered</span>
      </button>
      {canReroute && !sentTo && (
        <button
          type="button"
          data-compact
          onClick={() => setPicking((v) => !v)}
          aria-expanded={picking}
          aria-controls={picking ? pickerId : undefined}
          className="shrink-0 rounded-full px-2 min-h-6 text-[11px] text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 underline decoration-dotted underline-offset-2 transition-colors motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
        >
          Wrong bot?
        </button>
      )}
    </span>
  );

  const panel = (
    <>
      {open && (
        <p id={reasonId} className="mb-1.5 max-w-2xl break-words text-xs leading-snug text-zinc-300">
          {route.text}
        </p>
      )}
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

  return { chip, panel };
}
