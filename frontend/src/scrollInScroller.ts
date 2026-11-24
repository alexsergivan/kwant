/** The nearest ancestor that really scrolls vertically (null: none does). */
function scrollerOf(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    // overflow-x-auto alone computes overflow-y to auto too: only a box that overflows scrolls.
    if (/(auto|scroll)/.test(getComputedStyle(p).overflowY) && p.scrollHeight > p.clientHeight) return p;
  }
  return null;
}

/**
 * Scrolls `el` to the start or centre of its nearest scrolling ancestor, and moves nothing else.
 * scrollIntoView also scrolls clipped (overflow-hidden) ancestors: on a phone that shifts the fixed
 * Settings layer up off its own header and leaves a blank band at the bottom (34h).
 */
export function scrollInScroller(el: HTMLElement, block: 'start' | 'center', behavior: ScrollBehavior = 'auto'): void {
  const p = scrollerOf(el);
  if (!p) {
    el.scrollIntoView?.({ behavior, block: 'nearest' });
    return;
  }
  const r = el.getBoundingClientRect();
  const box = p.getBoundingClientRect();
  const gap = block === 'center' ? (box.height - r.height) / 2 : 0;
  p.scrollTo?.({ top: p.scrollTop + r.top - box.top - gap, behavior });
}

/**
 * Centres `el` in its scroller and keeps it there for `ms` while content above it is still loading
 * (a section that arrives after the scroll pushed the row off a phone screen, 34h round 2). Only a
 * move of `el` within the content re-centres it; the person's own scrolling is never undone.
 * Returns a function that stops watching.
 */
export function keepCentred(el: HTMLElement, ms: number, behavior: ScrollBehavior = 'auto'): () => void {
  const at = () => {
    const p = scrollerOf(el);
    return p ? el.getBoundingClientRect().top - p.getBoundingClientRect().top + p.scrollTop : null;
  };
  scrollInScroller(el, 'center', behavior);
  let last = at();
  const end = performance.now() + ms;
  let frame = 0;
  const tick = () => {
    const now = at();
    if (now !== null && last !== null && Math.abs(now - last) > 1) scrollInScroller(el, 'center', behavior);
    last = now;
    if (performance.now() < end && el.isConnected) frame = requestAnimationFrame(tick);
  };
  frame = requestAnimationFrame(tick);
  return () => cancelAnimationFrame(frame);
}
