/**
 * Scrolls `el` to the start or centre of its nearest scrolling ancestor, and moves nothing else.
 * scrollIntoView also scrolls clipped (overflow-hidden) ancestors: on a phone that shifts the fixed
 * Settings layer up off its own header and leaves a blank band at the bottom (34h).
 */
export function scrollInScroller(el: HTMLElement, block: 'start' | 'center', behavior: ScrollBehavior = 'auto'): void {
  for (let p = el.parentElement; p; p = p.parentElement) {
    // overflow-x-auto alone computes overflow-y to auto too: only a box that overflows scrolls.
    if (!/(auto|scroll)/.test(getComputedStyle(p).overflowY) || p.scrollHeight <= p.clientHeight) continue;
    const r = el.getBoundingClientRect();
    const box = p.getBoundingClientRect();
    const gap = block === 'center' ? (box.height - r.height) / 2 : 0;
    p.scrollTo?.({ top: p.scrollTop + r.top - box.top - gap, behavior });
    return;
  }
  el.scrollIntoView?.({ behavior, block: 'nearest' });
}
