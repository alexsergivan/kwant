import React, { useEffect, useRef, useState } from 'react';
import { scrollEdges } from './ScrollRow';

/** 31p: a reply's plain table that scrolls sideways shows a right-edge fade while there is more to the
 *  right. A table that fits looks exactly as before. The fade stays inside the rounded border and is
 *  only a cue (aria-hidden, no pointer events): the table itself is what scrolls. */
export function ScrollTable({ children }: { children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [more, setMore] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => setMore(!scrollEdges(el).atEnd);
    update();
    el.addEventListener('scroll', update, { passive: true });
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    resize?.observe(el);
    // A streamed table grows without its box resizing, so watch the table too.
    if (el.firstElementChild) resize?.observe(el.firstElementChild);
    return () => {
      el.removeEventListener('scroll', update);
      resize?.disconnect();
    };
  }, []);
  return (
    <div className="relative my-3.5">
      <div ref={ref} className="overflow-x-auto rounded-xl border border-zinc-700/60 bg-zinc-900/60 shadow-[var(--kws-shadow)]">
        {children}
      </div>
      {more && (
        <div
          aria-hidden="true"
          data-scroll-cue=""
          className="pointer-events-none absolute inset-y-px right-px w-8 rounded-r-[11px] bg-gradient-to-l from-zinc-900"
        />
      )}
    </div>
  );
}
