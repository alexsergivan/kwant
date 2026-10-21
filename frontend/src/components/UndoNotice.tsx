import React, { useEffect, useRef } from 'react';
import { Undo2 } from 'lucide-react';

/** Takes the place of something just deleted while Undo is still possible. Focus moves to Undo,
 *  since the Delete button that had it is gone. */
export const UndoNotice: React.FC<{ text: string; onUndo: () => void; className?: string; as?: 'li' | 'div' }> = ({ text, onUndo, className = '', as = 'div' }) => {
  const btn = useRef<HTMLButtonElement>(null);
  useEffect(() => btn.current?.focus({ preventScroll: true }), []);
  const Tag = as;
  return (
    <Tag data-undo className={`flex flex-wrap items-center gap-2 px-2.5 py-2 rounded-lg border border-zinc-800 bg-zinc-900/60 text-[11px] text-zinc-400 ${className}`}>
      <span role="status" className="flex-1 min-w-0 break-words">
        {text}
      </span>
      <button
        ref={btn}
        type="button"
        onClick={onUndo}
        className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium border border-sky-500/40 bg-sky-500/10 text-sky-200 hover:bg-sky-500/20 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-sky-500"
      >
        <Undo2 size={11} aria-hidden="true" /> Undo
      </button>
    </Tag>
  );
};
