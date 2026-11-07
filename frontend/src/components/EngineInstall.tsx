import React, { useEffect, useRef, useState } from 'react';
import { Download, ExternalLink, Loader2 } from 'lucide-react';
import { playSfx } from '../audio/chiptune';
import { SetupApi, SetupItemId, canInstall, runInstall, setupApi } from '../setupInstall';

const primaryBtn =
  'inline-flex items-center gap-1.5 px-2.5 py-1 min-h-[28px] rounded-md text-[11px] font-medium text-white bg-violet-600 hover:bg-violet-500 disabled:opacity-50 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-300';
const linkCls = 'inline-flex items-center gap-0.5 text-violet-300 hover:text-violet-200 underline-offset-2 hover:underline';

/** How long an install usually takes, for the progress line. */
const ABOUT: Partial<Record<SetupItemId, string>> = { claude: 'about 15 seconds' };

type Phase = { kind: 'idle' } | { kind: 'installing' } | { kind: 'done' } | { kind: 'failed'; lastLine: string | null };

/** "Install {name} ↗, then press Re-check.": the way to do it by hand. */
const ManualLink: React.FC<{ name: string; url?: string; lead: string }> = ({ name, url, lead }) => (
  <p className="text-[11px] text-zinc-400 leading-snug">
    {lead}{' '}
    {url && (
      <a href={url} target="_blank" rel="noreferrer" className={linkCls}>
        Install {name} <ExternalLink size={10} aria-hidden="true" />
      </a>
    )}
    {url && ', then press Re-check.'}
  </p>
);

/**
 * An engine that isn't on this computer: "Install {name}" when the server can install it itself
 * (/api/setup/status says can_install), one quiet progress line while it runs, then `children` (Claude's
 * sign-in) straight away. Where it can't, the link to install it by hand, as before.
 *
 * `needed` false shows `children` at once; the same element holds both, so finishing an install doesn't
 * remount the sign-in that follows. `autoStart` changing starts the install without a click
 * ("Set up Claude" in the banner and the welcome).
 */
export const EngineInstall: React.FC<{
  item: SetupItemId;
  name: string;
  url?: string;
  needed: boolean;
  onInstalled?: () => void;
  autoStart?: number;
  api?: SetupApi;
  /** Tests: no waiting between polls. */
  sleep?: (ms: number) => Promise<void>;
  children?: React.ReactNode;
}> = ({ item, name, url, needed, onInstalled, autoStart, api = setupApi, sleep, children }) => {
  const [installable, setInstallable] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' });
  const alive = useRef(true);
  const btn = useRef<HTMLButtonElement>(null);
  const startedFor = useRef<number | undefined>(undefined);
  useEffect(() => {
    alive.current = true;
    return () => void (alive.current = false);
  }, []);

  useEffect(() => {
    if (!needed) return;
    let gone = false;
    api
      .status()
      .then((items) => !gone && setInstallable(canInstall(items, item)))
      .catch(() => !gone && setInstallable(false));
    return () => void (gone = true);
  }, [api, item, needed]);

  const start = async () => {
    if (phase.kind === 'installing') return;
    setPhase({ kind: 'installing' });
    const r = await runInstall(api, item, { sleep, cancelled: () => !alive.current });
    if (!alive.current) return;
    if (r.ok) {
      playSfx('coin');
      setPhase({ kind: 'done' });
      onInstalled?.();
    } else {
      setPhase({ kind: 'failed', lastLine: r.lastLine });
    }
  };

  useEffect(() => {
    if (!autoStart || startedFor.current === autoStart || !needed || installable !== true || phase.kind !== 'idle') return;
    startedFor.current = autoStart;
    void start();
    // start reads the latest phase through the render it was made in; this only runs once per request
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoStart, needed, installable, phase.kind]);

  if (phase.kind === 'done' || (!needed && phase.kind === 'idle')) return <>{children}</>;

  if (phase.kind === 'installing') {
    const about = ABOUT[item];
    return (
      <p role="status" aria-live="polite" data-engine-install="installing" className="flex items-center gap-1.5 text-[11px] text-zinc-300">
        <Loader2 size={12} className="shrink-0 motion-safe:animate-spin text-violet-300" aria-hidden="true" />
        Installing {name}…{about ? ` (${about})` : ''}
      </p>
    );
  }

  if (phase.kind === 'failed') {
    return (
      <div role="alert" data-engine-install="failed" className="space-y-1.5">
        <p className="text-[11px] text-rose-200 leading-snug">{name} didn’t install.</p>
        {phase.lastLine && (
          <details className="text-[11px] text-zinc-400">
            <summary className="cursor-pointer select-none text-zinc-300 hover:text-zinc-100">Details</summary>
            <code className="mt-1 block whitespace-pre-wrap break-words font-mono text-[10.5px] text-zinc-400 select-text">{phase.lastLine}</code>
          </details>
        )}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <button type="button" onClick={() => void start()} className={primaryBtn}>
            <Download size={12} aria-hidden="true" />
            Try again
          </button>
          {url && (
            <a href={url} target="_blank" rel="noreferrer" className={`text-[11px] ${linkCls}`}>
              Install it yourself <ExternalLink size={10} aria-hidden="true" />
            </a>
          )}
        </div>
      </div>
    );
  }

  if (installable === true) {
    return (
      <div data-engine-install="can-install" className="space-y-1.5">
        <p className="text-[11px] text-zinc-400 leading-snug">{name} isn’t on this computer yet. This app can install it for you.</p>
        <button
          ref={btn}
          type="button"
          onClick={() => {
            playSfx('click');
            void start();
          }}
          className={primaryBtn}
        >
          <Download size={12} aria-hidden="true" />
          Install {name}
        </button>
      </div>
    );
  }

  // Can't install it here, or still asking: the link, as before.
  return (
    <div data-engine-install={installable === null ? 'checking' : 'manual'}>
      <ManualLink name={name} url={url} lead={`${name} isn’t on this computer yet.`} />
    </div>
  );
};
