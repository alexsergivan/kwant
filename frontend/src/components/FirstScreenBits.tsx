import React from 'react';
import { Settings, X } from 'lucide-react';
import { spaceSublineParts, sublineText, type SpaceApprovals } from '../firstScreen';
import type { SpaceCounts } from '../spaces';

/**
 * The space header's sub-line (first-screen A): "3 bots · Dev mode · Approvals off · 1 account". The status
 * words are words, not controls; their settings live in Space settings › General.
 */
export function SpaceSubline({ counts, devOn, approvals }: { counts: SpaceCounts | undefined; devOn: boolean; approvals: SpaceApprovals }) {
  const parts = spaceSublineParts(counts, { devOn, approvals });
  if (!parts.length) return null;
  return (
    <span className="block text-meta text-zinc-400 truncate" title={sublineText(parts)} data-space-subline>
      {parts.map((p, i) => (
        <React.Fragment key={i}>
          {i > 0 && ' · '}
          {p.tone ? (
            <span className={p.tone === 'dev' ? 'text-sky-300' : 'text-warn'} data-subline-status={p.tone}>
              {p.text}
            </span>
          ) : (
            p.text
          )}
        </React.Fragment>
      ))}
    </span>
  );
}

/**
 * "Dev mode and Approvals moved to this space's settings." (0.9.8 and 0.9.9 only), where the controls block
 * used to sit. Remove with its key in 0.9.10.
 */
export function MovedPointer({ phone, onOpen, onHide }: { phone: boolean; onOpen: () => void; onHide: () => void }) {
  const hit = phone ? ' min-h-11 min-w-11 justify-center' : '';
  return (
    <div
      role="note"
      data-moved-pointer
      className="mt-1.5 flex items-center gap-2 rounded-xl border border-white/[0.07] bg-white/[0.025] py-2 pl-2.5 pr-1.5 text-[12px] leading-[18px] text-zinc-300"
    >
      <Settings size={14} className="shrink-0 text-zinc-400" aria-hidden="true" />
      <span className="flex-1 min-w-0">Dev mode and Approvals moved to this space's settings.</span>
      <button
        type="button"
        onClick={onOpen}
        data-moved-open
        className={`shrink-0 inline-flex items-center rounded-md px-1.5 text-sky-300 font-medium hover:text-sky-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400${hit}`}
      >
        Open
      </button>
      <button
        type="button"
        onClick={onHide}
        aria-label="Hide"
        title="Hide"
        data-moved-hide
        className={`shrink-0 inline-flex items-center justify-center rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 ${phone ? 'min-h-11 min-w-11' : 'h-6 w-6'}`}
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  );
}
