import React, { useEffect, useRef } from 'react';

/** A sideways scroller whose edges fade (phones only, see .kw-scroll-row) while there is more that
 *  way. `data-at-end` is "true" once it is scrolled to the end or everything already fits;
 *  `data-at-start` is "true" while it is scrolled all the way left (14c). No fade on a "true" side. */
export const scrollEdges = (el: { scrollLeft: number; clientWidth: number; scrollWidth: number }) => ({
  atStart: el.scrollLeft <= 1,
  atEnd: el.scrollLeft + el.clientWidth >= el.scrollWidth - 1,
});

export function ScrollRow({
  as: Tag = 'div',
  children,
  ...rest
}: React.HTMLAttributes<HTMLElement> & { as?: 'div' | 'nav' }) {
  const ref = useRef<HTMLElement>(null);
  const update = () => {
    const el = ref.current;
    if (!el) return;
    const { atStart, atEnd } = scrollEdges(el);
    el.dataset.atStart = String(atStart);
    el.dataset.atEnd = String(atEnd);
  };
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.addEventListener('scroll', update, { passive: true });
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(update);
    resize?.observe(el);
    return () => {
      el.removeEventListener('scroll', update);
      resize?.disconnect();
    };
  }, []);
  // The items themselves can change without the row resizing.
  useEffect(update);
  return (
    <Tag ref={ref as React.RefObject<HTMLDivElement>} {...rest}>
      {children}
    </Tag>
  );
}
