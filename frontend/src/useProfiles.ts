// LLM accounts (GET /api/profiles) for the Engines & accounts cards.
import { useCallback, useEffect, useRef, useState } from 'react';
import { Profile, profilesApi } from './profiles';

export interface ProfilesState {
  /** null until the first load. */
  profiles: Profile[] | null;
  error: string | null;
  reload: () => Promise<void>;
}

/** Loads the accounts while `active`; `onChanged` gets every fresh list, so kwant pickers stay current. */
export function useProfiles(active: boolean, onChanged?: (profiles: Profile[]) => void): ProfilesState {
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const changed = useRef(onChanged);
  changed.current = onChanged;
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => void (alive.current = false);
  }, []);

  const reload = useCallback(async () => {
    try {
      const list = await profilesApi.list();
      if (!alive.current) return;
      setProfiles(list);
      setError(null);
      changed.current?.(list);
    } catch (e) {
      if (alive.current) setError(e instanceof Error ? e.message : 'Could not load accounts.');
    }
  }, []);

  useEffect(() => {
    if (active) reload();
  }, [active, reload]);

  return { profiles, error, reload };
}
