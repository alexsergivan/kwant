import React, { useEffect, useRef } from 'react';

/** A sideways scroller whose right edge fades (phones only, see .kw-scroll-row) while there is
 *  more to the right. `data-at-end` is "true" once it is scrolled to the end or everything already
 *  fits, and then there is no fade. */
export function ScrollRow({
  as: Tag = 'div',
  children,
  ...rest
}: React.HTMLAttributes<HTMLElement> & { as?: 'div' | 'nav' }) {
  const ref = useRef<HTMLElement>(null);
  const update = () => {
    const el = ref.current;
    if (el) el.dataset.atEnd = String(el.scrollLeft + el.clientWidth >= el.scrollWidth - 1);
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
