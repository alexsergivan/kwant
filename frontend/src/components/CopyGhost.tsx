// reply-chrome part 3 (autonomy/reply-chrome/design.md): Copy as a quiet icon, after Reply and the thumbs, with
// its label in a tooltip on hover (not on touch) and on keyboard focus. Copied shows a check and "Copied" for the
// parent's copied window, announced once.

import React from 'react';
import { Check, Copy } from 'lucide-react';

export const CopyGhost: React.FC<{ copied: boolean; tipId: string; onCopy: () => void }> = ({ copied, tipId, onCopy }) => (
  <span className="relative inline-flex shrink-0" data-copy-ghost="">
    <button
      type="button"
      onClick={onCopy}
      aria-label="Copy reply"
      aria-describedby={tipId}
      data-copy-reply=""
      className="peer relative inline-flex h-11 w-11 sm:h-8 sm:w-8 items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-zinc-100 transition-colors duration-[120ms] motion-reduce:transition-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 cursor-pointer"
    >
      {copied ? <Check size={14} className="text-ok" aria-hidden="true" /> : <Copy size={14} aria-hidden="true" />}
    </button>
    <span
      id={tipId}
      role="tooltip"
      className={`pointer-events-none absolute bottom-full left-1/2 z-20 mb-1.5 -translate-x-1/2 whitespace-nowrap rounded-md border border-hairline-strong bg-surface-overlay px-2 py-1 text-xs text-zinc-100 shadow transition-opacity duration-[120ms] motion-reduce:transition-none [@media(hover:hover)]:peer-hover:opacity-100 peer-focus-visible:opacity-100 ${
        copied ? 'opacity-100' : 'opacity-0'
      }`}
    >
      {copied ? 'Copied' : 'Copy reply'}
    </span>
    <span className="sr-only" aria-live="polite">
      {copied ? 'Copied' : ''}
    </span>
  </span>
);
