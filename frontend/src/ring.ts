// 22i: the ring a Needs-attention row's landing gets (design.md §1): the search jump's amber `kw-flash`,
// a 2.4 s glow that becomes a static outline under reduced motion, so the landing is shown either way.
// ChatView rings messages through its own flash state; this is for cards outside the message list
// (an approval card), which have no flash class of their own.

export const RING_MS = 2400;

/** Rings an element once with `kw-flash`, tagged `data-flash-kind="ask"` so tests can tell it from search. */
export function ringElement(el: HTMLElement): void {
  el.classList.remove('kw-flash');
  void el.offsetWidth;
  el.classList.add('kw-flash');
  el.dataset.flashKind = 'ask';
  window.setTimeout(() => {
    el.classList.remove('kw-flash');
    delete el.dataset.flashKind;
  }, RING_MS);
}
