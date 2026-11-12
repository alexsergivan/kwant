import { useCallback, useEffect, useRef, useState } from 'react';

import { BrowserPageList, fetchBrowserPages, onBrowserPagesChanged } from './browserPages';

/** How often the strip asks for the team's pages while the pane is open. */
export const PAGES_REFRESH_MS = 10_000;

/**
 * The team's browser pages (0.9.3), for the pane's strip: loaded while `visible`, again on every
 * BrowserNavigated, SandboxStatusChanged and human_step_updated for the team, and every 10 s.
 * `at` is when the list was asked for, so a page picked after that isn't judged closed by it.
 * An older server without the route leaves it null (no strip).
 */
export function useBrowserPages(team: string | undefined, visible: boolean) {
  const [list, setList] = useState<(BrowserPageList & { at: number }) | null>(null);
  const seq = useRef(0);
  const live = !!team && team !== 'default' && visible;
  // Another team's list must never show under this one (the pane is keyed by team, so this is a guard)
  const shownTeam = useRef(team);
  if (shownTeam.current !== team) {
    shownTeam.current = team;
    seq.current++;
    if (list) setList(null);
  }

  const load = useCallback(() => {
    if (!team || team === 'default') return;
    const n = ++seq.current;
    const at = Date.now();
    fetchBrowserPages(team)
      .then((l) => {
        if (n !== seq.current) return;
        setList(l && l.team_id === team ? { ...l, at } : null);
      })
      .catch(() => {
        // offline for a moment: keep the last list until the next refresh
      });
  }, [team]);

  useEffect(() => {
    if (!live) return;
    load();
    let soon: ReturnType<typeof setTimeout> | undefined;
    // Events come in bursts (a bot navigating fires several): one load for each burst
    const off = onBrowserPagesChanged((t) => {
      if (t !== team) return;
      clearTimeout(soon);
      soon = setTimeout(load, 250);
    });
    const tick = setInterval(() => {
      if (typeof document === 'undefined' || !document.hidden) load();
    }, PAGES_REFRESH_MS);
    return () => {
      off();
      clearTimeout(soon);
      clearInterval(tick);
    };
  }, [live, team, load]);

  return { list, reload: load };
}
