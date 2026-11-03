import React, { useEffect, useRef, useState } from 'react';
import { Square } from 'lucide-react';
import type { LiveActivity } from '../activity';
import { formatElapsed, turnStart, workingLabel } from '../workingStrip';
import { ModernAvatar } from './ModernAvatar';

interface WorkingStripProps {
  active: boolean;
  name: string;
  /** Picks the avatar's creature. */
  title: string;
  live?: LiveActivity | null;
  queued?: boolean;
  awaitingAnswer?: boolean;
  awaitingBrowser?: boolean;
  writing?: boolean;
  onStop?: () => void;
}

/** Summary of the running turn (the composer holds the one Stop; onStop is unused there), kept in view above the composer; the card in the stream holds the step details. */
export const WorkingStrip: React.FC<WorkingStripProps> = ({ active, name, title, live, queued, awaitingAnswer, awaitingBrowser, writing, onStop }) => {
  const mountedAt = useRef(Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    mountedAt.current = Date.now();
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  if (!active) return null;

  const label = workingLabel({ live, queued, awaitingAnswer, awaitingBrowser, writing });
  const elapsed = formatElapsed(now - turnStart(live, mountedAt.current));
  const waiting = !!queued || !!awaitingAnswer;
  return (
    <div className="kws-strip px-3 sm:px-6 pt-2" data-working-strip="">
      <div className="mx-auto flex max-w-4xl items-center gap-3 rounded-xl border border-zinc-700/60 bg-zinc-900/80 px-3 py-1.5 shadow-[var(--kws-shadow)]">
        <span className={`shrink-0 inline-flex ${waiting ? '' : 'kw-avatar-working'}`}>
          <ModernAvatar role={title} name={name} size={24} />
        </span>
        <p className="min-w-0 flex-1 truncate text-xs text-zinc-300" role="status" aria-live="polite">
          <span className="font-semibold text-zinc-100">{name}</span> is {label}
        </p>
        {/* Own box, so a long step name never cuts the time off at 390 px. */}
        <span className="shrink-0 tabular-nums text-meta text-zinc-400" aria-hidden="true" data-working-elapsed="">
          {elapsed}
        </span>
        {onStop && (
          <button
            type="button"
            onClick={onStop}
            title="Stop"
            className="shrink-0 inline-flex items-center gap-1.5 min-h-8 rounded-lg border border-zinc-600/70 px-2.5 text-xs font-medium text-zinc-200 hover:bg-zinc-800 hover:text-zinc-50 active:scale-[0.97] motion-reduce:active:scale-100 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
          >
            <Square size={12} className="fill-current" aria-hidden="true" />
            Stop
          </button>
        )}
      </div>
    </div>
  );
};
