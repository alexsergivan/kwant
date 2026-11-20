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
  const queue = useRef<Array<() => void>>([]);
  const [, render] = useReducer((n: number) => n + 1, 0);
  useLayoutEffect(() => {
    if (!queue.current.length) return;
    const run = queue.current;
    queue.current = [];
    for (const work of run) work();
  });
  return useCallback((work: () => void) => {
    queue.current.push(work);
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
