import { useCallback, useEffect, useRef, useState } from 'react';

import { BrowserPageList, fetchBrowserPages, onBrowserPagesChanged } from './browserPages';

/** How often the strip asks for the team's pages while the pane is open. */
export const PAGES_REFRESH_MS = 10_000;

/** 28l: how long the pane waits for the first list before it streams the server's default anyway. */
export const PAGES_WAIT_MS = 1_500;

/**
 * The team's browser pages (0.9.3), for the pane's strip: loaded while `visible`, again on every
 * BrowserNavigated, SandboxStatusChanged and human_step_updated for the team, and every 10 s.
 * `at` is when the list was asked for, so a page picked after that isn't judged closed by it.
 * An older server without the route (a 404) leaves it null (no strip); any other failure keeps the last list.
 * `checked` (28l) is false from the moment the pane shows until the first answer since then, so the
 * pane knows whether to wait for the list or use its older fallback; the list is dropped when it hides.
 */
// 28l: teams whose server answered with no pages list (an older server): the pane doesn't wait for it again
const noPagesRoute = new Set<string>();

/** Marks a team's server as having no pages route (as its first answer would); for tests that render once. */
export function noteNoPagesRoute(team: string): void {
  noPagesRoute.add(team);
}

export function useBrowserPages(team: string | undefined, visible: boolean) {
  const [list, setList] = useState<(BrowserPageList & { at: number }) | null>(null);
  const [checked, setChecked] = useState(() => !!team && noPagesRoute.has(team));
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
        if (l) noPagesRoute.delete(team);
        else noPagesRoute.add(team);
        setChecked(true);
      })
      .catch(() => {
        // 28o: offline or a failed answer for a moment: keep the last list until the next refresh, and
        // don't judge the server. A wait for the first list still ends at PAGES_WAIT_MS.
      });
  }, [team]);

  useEffect(() => {
    if (!live) return;
    load();
    // A slow answer must not keep the pane waiting: after this, the older fallback
    const giveUp = setTimeout(() => setChecked(true), PAGES_WAIT_MS);
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
      clearTimeout(giveUp);
      // The pane closed: its list goes stale (a run may move pages before it opens again), so the next
      // open starts with none and waits for a fresh one, rather than showing this one for a moment
      seq.current++;
      setList(null);
      setChecked(!!team && noPagesRoute.has(team));
      clearInterval(tick);
    };
  }, [live, team, load]);

  return { list, checked, reload: load };
}
