// 0.8.0 B3: "Checked with fresh eyes" (Mira b3-fresh-eyes.md §1). A plain grey chip, never
// holographic: it describes how the review was run. R1 opens a small, non-modal popover listing
// what the reviewer saw; R2 is a grey chip that isn't a button; R3 shows nothing.
//
// The popover is a flex item of the chip row: it wraps onto its own line, in flow, on phone, and
// floats under the row from 640 px up. Give the row `relative` so it has something to float under.

import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { Check, Eye, X } from 'lucide-react';
import { chipState, popoverTitle, sawLines, type FreshEyes } from '../../dev/freshEyes';

export interface FreshEyesChipProps {
  label: FreshEyes | null | undefined;
  reviewer: string | null;
  author: string | null;
  diff?: { files: number; add: number; del: number } | null;
  /** "#212" when the review was against a ticket. */
  specRef?: string | null;
}

export const FreshEyesChip: React.FC<FreshEyesChipProps> = ({ label, reviewer, author, diff = null, specRef = null }) => {
  const [open, setOpen] = useState(false);
  // From 640 px the popover floats under the row: put it under this chip, not the row's start, and
  // keep it inside the row (Mira ui-check-458 Later 1). On phone it's in flow, so `left` does nothing.
  const [left, setLeft] = useState(0);
  const btn = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const id = useId();
  const state = chipState(label);

  useLayoutEffect(() => {
    if (!open) return;
    const b = btn.current;
    const row = b?.offsetParent as HTMLElement | null;
    if (!b || !row) return;
    const max = Math.max(0, row.clientWidth - 300);
    setLeft(Math.min(Math.max(0, b.offsetLeft), max));
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent | TouchEvent) => {
      const t = e.target as Node | null;
      if (t && (btn.current?.contains(t) || pop.current?.contains(t))) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      setOpen(false);
      btn.current?.focus();
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('touchstart', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('touchstart', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  if (state === 'R3' || !label) return null;
  if (state === 'R2') {
    return (
      <span data-fresh-eyes="R2" className="kw-chip">
        Saw the author's chat
      </span>
    );
  }

  const lines = sawLines(label, { author, diff, ref: specRef });
  const onBlur = (e: React.FocusEvent) => {
    const next = e.relatedTarget as Node | null;
    if (next && (btn.current?.contains(next) || pop.current?.contains(next))) return;
    setOpen(false);
  };

  return (
    <>
      <button
        ref={btn}
        type="button"
        data-fresh-eyes="R1"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((o) => !o)}
        onBlur={onBlur}
        className="kw-chip fe-chip cursor-pointer outline-none hover:bg-white/[0.07] focus-visible:ring-2 focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-900"
      >
        <Eye size={12} className="text-sky-300" aria-hidden="true" />
        Checked with fresh eyes
      </button>
      {open && (
        <div
          ref={pop}
          id={id}
          role="dialog"
          aria-label="How this review was run"
          tabIndex={-1}
          onBlur={onBlur}
          data-fresh-eyes-pop
          style={{ left }}
          className="fe-pop kw-card w-full basis-full p-3 text-xs leading-[18px] text-zinc-300 sm:absolute sm:top-full sm:z-20 sm:mt-2 sm:w-auto sm:max-w-[300px] sm:basis-auto sm:bg-[#231d4b] sm:shadow-lg"
        >
          <p className="text-zinc-200 [overflow-wrap:anywhere]">{popoverTitle(reviewer)}</p>
          <ul role="list" className="mt-2 space-y-1">
            {lines.map((l) => (
              <li key={l.text} className={`flex items-start gap-1.5 [overflow-wrap:anywhere] ${l.ok ? 'text-zinc-300' : 'text-zinc-400'}`}>
                {l.ok ? (
                  <Check size={12} className="mt-[3px] shrink-0 text-emerald-300" aria-hidden="true" />
                ) : (
                  <X size={12} className="mt-[3px] shrink-0 text-zinc-500" aria-hidden="true" />
                )}
                <span className="min-w-0">
                  {l.ok && <span className="sr-only">Saw </span>}
                  {l.text}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
};
