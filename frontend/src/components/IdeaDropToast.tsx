import React, { useEffect, useRef } from 'react';
import { X } from 'lucide-react';

const focusRing = 'focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500 focus-visible:outline-offset-2';

export type IdeaDropToastState = 'dropped' | 'undoing' | 'undone' | 'undo_failed';

/**
 * "Dropped in the jar · Undo · Open jar" (spec §2.3), 6 s. The board's UndoToast shape, plus Open jar
 * and the after-Undo line. The parent owns the timer so a hover or focus can't strand it.
 */
export const IdeaDropToast: React.FC<{
  state: IdeaDropToastState;
  onUndo: () => void;
  onOpenJar?: () => void;
  onClose: () => void;
  /** ms before it closes itself; paused while the pointer or focus is inside. */
  timeout?: number;
}> = ({ state, onUndo, onOpenJar, onClose, timeout = 6000 }) => {
  const box = useRef<HTMLDivElement>(null);
  const held = useRef(false);
  useEffect(() => {
    if (state === 'undoing') return;
    const ms = state === 'dropped' ? timeout : 3000;
    let t = setTimeout(function tick() {
      if (held.current) t = setTimeout(tick, 1000);
      else onClose();
    }, ms);
    return () => clearTimeout(t);
  }, [state, timeout, onClose]);
  const text =
    state === 'undone' ? 'Taken back out' : state === 'undo_failed' ? 'Couldn’t take it back out. It’s still in the jar.' : 'Dropped in the jar';
  // Never over the chat box: under the header on phones (the composer grows there), above it from sm up.
  return (
    <div className="fixed inset-x-0 top-[calc(env(safe-area-inset-top)+4.25rem)] sm:top-auto sm:bottom-32 z-40 flex justify-center px-4 pointer-events-none" data-idea-toast={state}>
      <div
        ref={box}
        role="status"
        aria-live="polite"
        onPointerEnter={() => (held.current = true)}
        onPointerLeave={() => (held.current = false)}
        onFocus={() => (held.current = true)}
        onBlur={() => (held.current = false)}
        className="pointer-events-auto flex items-center gap-2 max-w-md w-full sm:w-auto pl-4 pr-2 py-2 rounded-xl border border-[#2c2552] bg-[#1a1538] shadow-2xl shadow-black/50"
      >
        <span className={`min-w-0 flex-1 text-xs break-words ${state === 'undo_failed' ? 'text-rose-300' : 'text-[#ecebfa]'}`}>{text}</span>
        {(state === 'dropped' || state === 'undoing') && (
          <button
            type="button"
            onClick={onUndo}
            disabled={state === 'undoing'}
            className={`shrink-0 min-h-[36px] px-3 rounded-lg text-xs font-semibold text-sky-300 hover:bg-sky-500/10 disabled:opacity-60 ${focusRing}`}
            data-idea-undo
          >
            Undo
          </button>
        )}
        {onOpenJar && state !== 'undone' && (
          <button
            type="button"
            onClick={onOpenJar}
            className={`shrink-0 min-h-[36px] px-3 rounded-lg text-xs font-semibold text-[#b8adff] hover:bg-violet-500/10 ${focusRing}`}
            data-idea-open-jar
          >
            Open jar
          </button>
        )}
        <button type="button" onClick={onClose} aria-label="Close" className={`shrink-0 p-1.5 rounded-lg text-[#a39dcc] hover:text-white hover:bg-white/5 ${focusRing}`}>
          <X size={14} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
};
