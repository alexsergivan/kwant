// Holds the product timers a test is about, so the test fires them when it is ready instead of racing
// wall-clock time (22e). `holdTimers((ms) => ms === 40)` swaps the global setTimeout/clearTimeout: a
// timer whose delay matches is kept, not scheduled; every other timer (React's, the test's own waits)
// runs as normal. `pending()` lists the held delays, `run()` fires the held timers once, and
// `restore()` puts the real functions back (call it in a `finally`).
export const holdTimers = (match) => {
  const realSet = globalThis.setTimeout;
  const realClear = globalThis.clearTimeout;
  const held = new Map();
  let next = 0;
  globalThis.setTimeout = (fn, ms = 0, ...args) => {
    if (!match(ms)) return realSet(fn, ms, ...args);
    const id = { held: (next += 1) };
    held.set(id, { fn, ms, args });
    return id;
  };
  globalThis.clearTimeout = (id) => {
    if (id && typeof id === 'object' && 'held' in id) held.delete(id);
    else realClear(id);
  };
  return {
    pending: () => [...held.values()].map((t) => t.ms),
    run: () => {
      const due = [...held.values()];
      held.clear();
      for (const t of due) t.fn(...t.args);
      return due.length;
    },
    restore: () => {
      globalThis.setTimeout = realSet;
      globalThis.clearTimeout = realClear;
    },
  };
};

// Waits until `ok()` is true, yielding to timers and React between checks; gives up after `ms` of
// wall time so the test's own assertion reports what went wrong. It reads performance.now(), because
// some files freeze Date.now (ideaJar does).
export const until = async (ok, ms = 3000) => {
  const end = performance.now() + ms;
  while (!ok() && performance.now() < end) await new Promise((r) => setTimeout(r, 5));
};
