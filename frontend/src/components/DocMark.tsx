import React from 'react';
import { docKind, markOf } from '../spaceDocs';

/** The type mark: PDF, MD or TXT (or the odd extension). It tells the types apart; it is not a status. */
export const DocMark: React.FC<{ name: string; size?: 28 | 32 }> = ({ name, size = 32 }) => {
  const k = docKind(name);
  const tone = k === 'pdf' ? 'text-rose-200' : k === 'md' ? 'text-violet-200' : 'text-zinc-200';
  return (
    <span
      aria-hidden="true"
      className={`shrink-0 inline-flex items-center justify-center rounded-lg border border-zinc-700 bg-zinc-900 font-bold tracking-[.04em] ${tone} ${
        size === 28 ? 'w-7 h-7 text-[9px]' : 'w-8 h-8 text-[9.5px]'
      }`}
    >
      {markOf(name)}
    </span>
  );
};
