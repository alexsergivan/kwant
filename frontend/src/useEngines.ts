// Engine status from GET /api/engines, shared by Settings, the kwant form and the sidebar.
import { useCallback, useEffect, useRef, useState } from 'react';
import type { EngineInfo } from './engines';

export interface EnginesApi {
  /** null until the first load; [] from a server that has no engine checks. */
  engines: EngineInfo[] | null;
  error: string | null;
  rechecking: boolean;
  recheck: () => Promise<void>;
  /** Reads the cached status again, without re-running the checks. */
  reload: () => Promise<void>;
  setEnabled: (id: EngineInfo['id'], enabled: boolean) => Promise<void>;
  /** Shows a list the server already answered with (e.g. after signing out). */
  apply: (engines: EngineInfo[]) => void;
}

/** Loads /api/engines while `active`; polls briefly while any engine is still "checking". */
export function useEngines(active: boolean): EnginesApi {
  const [engines, setEngines] = useState<EngineInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rechecking, setRechecking] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    // Set again on mount: StrictMode mounts twice.
    alive.current = true;
    return () => void (alive.current = false);
  }, []);

  const load = useCallback(async () => {
    try {
      const r = await fetch('/api/engines');
      if (r.status === 404) {
        if (alive.current) setEngines([]);
        return;
      }
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const d = await r.json();
      if (alive.current && Array.isArray(d?.engines)) {
        setEngines(d.engines);
        setError(null);
      }
    } catch {
      if (alive.current) setError('Could not load engine status.');
    }
  }, []);

  useEffect(() => {
    if (active) load();
  }, [active, load]);

  // Startup checks take a moment; look again until none is "checking" (each check caps at 10s).
  const anyChecking = !!engines?.some((e) => e.status === 'checking');
  useEffect(() => {
    if (!active || !anyChecking) return;
    const t = window.setTimeout(load, 1500);
    return () => window.clearTimeout(t);
  }, [active, anyChecking, engines, load]);

  const recheck = useCallback(async () => {
    setRechecking(true);
    setError(null);
    try {
      const r = await fetch('/api/engines/recheck', { method: 'POST' });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      const d = await r.json();
      if (alive.current && Array.isArray(d?.engines)) setEngines(d.engines);
    } catch {
      if (alive.current) setError('Re-check failed. Is the server running?');
    } finally {
      if (alive.current) setRechecking(false);
    }
  }, []);

  const setEnabled = useCallback(
    async (id: EngineInfo['id'], enabled: boolean) => {
      // Shown at once; put back if the server says no.
      setEngines((list) => list?.map((e) => (e.id === id ? { ...e, enabled } : e)) ?? list);
      try {
        const r = await fetch(`/api/engines/${id}`, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ enabled }),
        });
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        const d = await r.json().catch(() => null);
        if (alive.current && Array.isArray(d?.engines)) setEngines(d.engines);
        else await load();
      } catch {
        if (!alive.current) return;
        setEngines((list) => list?.map((e) => (e.id === id ? { ...e, enabled: !enabled } : e)) ?? list);
        setError(`Couldn't switch ${enabled ? 'on' : 'off'} that engine.`);
      }
    },
    [load],
  );

  const apply = useCallback((list: EngineInfo[]) => {
    setEngines(list);
    setError(null);
  }, []);

  return { engines, error, rechecking, recheck, reload: load, setEnabled, apply };
}
