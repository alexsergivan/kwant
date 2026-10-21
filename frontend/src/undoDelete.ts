// Delete with Undo instead of "Delete?": the item goes from view at once and the request is only
// sent after UNDO_MS, so Undo just cancels it and nothing has to be restored on the server. Used
// for things that are cheap to lose (routines, skills, memories, Parallel Universe scenarios).
// Leaving the screen sends the delete straight away; closing the page before it goes keeps the item.

import { useCallback, useEffect, useRef, useState } from 'react';

export const UNDO_MS = 6000;

export interface Timers {
  set: (fn: () => void, ms: number) => unknown;
  clear: (handle: unknown) => void;
}

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

/** One delete waiting out its Undo window. Framework-free so it can be tested with fake timers. */
export class DeferredDelete {
  private handle: unknown = null;
  private run: (() => Promise<unknown>) | null = null;

  constructor(
    private readonly onCommitted: (error: string | null) => void,
    private readonly ms = UNDO_MS,
    private readonly timers: Timers = realTimers,
  ) {}

  get pending(): boolean {
    return this.run !== null;
  }

  /** Starts the window. One already waiting is sent at once, so a quick second delete never drops it. */
  start(run: () => Promise<unknown>): void {
    const earlier = this.run;
    this.cancelTimer();
    this.run = run;
    this.handle = this.timers.set(() => void this.flush(), this.ms);
    if (earlier) void this.send(earlier);
  }

  /** Cancels the waiting delete; nothing is sent. Returns whether there was one. */
  undo(): boolean {
    const had = this.pending;
    this.cancelTimer();
    this.run = null;
    return had;
  }

  /** Sends the waiting delete now (the window ran out, or the screen is going away). */
  async flush(): Promise<void> {
    const run = this.run;
    if (!run) return;
    this.cancelTimer();
    this.run = null;
    await this.send(run);
  }

  private async send(run: () => Promise<unknown>): Promise<void> {
    try {
      await run();
      this.onCommitted(null);
    } catch (e) {
      this.onCommitted(e instanceof Error ? e.message : 'That failed.');
    }
  }

  private cancelTimer() {
    if (this.handle !== null) this.timers.clear(this.handle);
    this.handle = null;
  }
}

/**
 * `pending` while the Undo window is open. `start(run)` hides the item, `undo()` brings it back,
 * and `run` is sent when the window closes or the component unmounts. A failed delete brings the
 * item back with `error` set; `onDone` runs after a successful one (e.g. to reload the list), and
 * `gone` stays true so the item doesn't show again before the list catches up.
 */
export function useUndoableDelete(onDone?: () => void, initialPending = false) {
  const [pending, setPending] = useState(initialPending);
  const [error, setError] = useState<string | null>(null);
  const [gone, setGone] = useState(false);
  const alive = useRef(true);
  const done = useRef(onDone);
  done.current = onDone;
  const box = useRef<DeferredDelete | null>(null);
  if (!box.current) {
    box.current = new DeferredDelete((err) => {
      if (!alive.current) return;
      // An earlier delete can finish while a later one is still waiting
      setPending(box.current?.pending ?? false);
      setError(err);
      if (!err) {
        setGone(true);
        done.current?.();
      }
    });
  }

  useEffect(() => {
    alive.current = true;
    const d = box.current;
    return () => {
      alive.current = false;
      void d?.flush();
    };
  }, []);

  const start = useCallback((run: () => Promise<unknown>) => {
    setError(null);
    setPending(true);
    box.current?.start(run);
  }, []);
  const undo = useCallback(() => {
    box.current?.undo();
    setPending(false);
  }, []);

  return { pending, gone, error, start, undo };
}
