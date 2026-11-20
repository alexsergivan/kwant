import { useCallback, useLayoutEffect, useReducer, useRef } from 'react';

/**
 * Runs work after the next render of the calling component is on screen, never on a timer.
 *
 * A `setTimeout(() => ref.current?.focus(), 0)` races React: when its scheduler yields (a slow machine, a busy
 * CI runner), the timer runs before the render that mounts its target, finds nothing and focus drops to <body>
 * for good. Here the work waits in a ref and a layout effect runs it right after the commit. Scheduling also
 * asks for a render, so the work never waits for an unrelated later one. Call it in the component that stays
 * on screen: a component that unmounts in the same update never runs its effects.
 */
export function useAfterRender(): (work: () => void) => void {
  // Each piece of work waits for the render it asked for (its ticket), not just the next commit: a store update
  // (useSyncExternalStore) renders first in a sync commit of its own, before the state set next to it after an
  // await, and work run there would miss the target that state shows (raf-focus-after-render, DevStatusLine Undo).
  const queue = useRef<Array<{ at: number; work: () => void }>>([]);
  const asked = useRef(0);
  const [rendered, render] = useReducer((n: number) => n + 1, 0);
  useLayoutEffect(() => {
    if (!queue.current.some((q) => q.at <= rendered)) return;
    const run = queue.current.filter((q) => q.at <= rendered);
    queue.current = queue.current.filter((q) => q.at > rendered);
    for (const q of run) q.work();
  });
  return useCallback((work: () => void) => {
    queue.current.push({ at: ++asked.current, work });
    render();
  }, []);
}

/**
 * Focuses `target()` after the next render, so a target that the same update mounts or swaps in is there to take
 * focus. A target that is still missing then is skipped (and cleared), never retried later.
 */
export function useFocusAfterRender(): (target: () => HTMLElement | null | undefined, opts?: FocusOptions) => void {
  const after = useAfterRender();
  return useCallback((target: () => HTMLElement | null | undefined, opts?: FocusOptions) => after(() => target()?.focus(opts)), [after]);
}
