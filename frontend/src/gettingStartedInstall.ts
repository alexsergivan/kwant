// O2: the install inside step 2 (design §5.2, §9 F1/F2/F2c/F7/F8). It lives outside the card so it keeps
// going when the person leaves #general, and the banner there can show "Getting Claude ready… 9 s".
// One tracker per space; the card and the banner read it with useInstallTrack.

import { useSyncExternalStore } from 'react';
import { type SetupApi, setupApi } from './setupInstall';
import { type InstallFailKind, type LoginEngine, getStartVersion, installFailKind, onStartChange } from './gettingStarted';

export interface ManualStep {
  label: string;
  command: string;
}

export interface InstallTrack {
  item: LoginEngine;
  state: 'running' | 'done' | 'failed' | 'needs_admin';
  startedAt: number;
  /** The job's lines, only for Copy details; never shown in the card. */
  lines: string[];
  error: string | null;
  kind: InstallFailKind | null;
  manual: ManualStep[];
}

const tracks = new Map<string, InstallTrack>();
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
const runs = new Map<string, number>();

export const getTrack = (space: string): InstallTrack | null => tracks.get(space) ?? null;
const set = (space: string, t: InstallTrack | null) => {
  if (t) tracks.set(space, t);
  else tracks.delete(space);
  emit();
};

export function subscribeTracks(l: () => void) {
  listeners.add(l);
  return () => void listeners.delete(l);
}

export const useInstallTrack = (space: string) =>
  useSyncExternalStore(
    subscribeTracks,
    () => getTrack(space),
    () => getTrack(space),
  );

/** Forget it (Change, or another AI chosen): a running job keeps running on the server, it just isn't shown. */
export function forgetInstall(space: string) {
  runs.set(space, (runs.get(space) ?? 0) + 1);
  set(space, null);
}

export interface RunOpts {
  api?: SetupApi;
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
  online?: () => boolean;
  pollMs?: number;
  busyMs?: number;
  /** Called once the CLI is installed (read the engines again). */
  onDone?: () => void;
}

const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const isOnline = () => (typeof navigator === 'undefined' ? true : navigator.onLine !== false);

/**
 * Installs `item` and follows the job. 409 (another install runs, e.g. one started before the app was
 * closed, F7) is waited out and tried again. 422 with manual steps is F2c: it is shown with its command
 * and the status is read every few seconds until the part is there.
 */
export async function startInstall(space: string, item: LoginEngine, opts: RunOpts = {}): Promise<InstallTrack> {
  const api = opts.api ?? setupApi;
  const sleep = opts.sleep ?? wait;
  const now = opts.now ?? Date.now;
  const online = opts.online ?? isOnline;
  const pollMs = opts.pollMs ?? 1000;
  const busyMs = opts.busyMs ?? 3000;
  const run = (runs.get(space) ?? 0) + 1;
  runs.set(space, run);
  const live = () => runs.get(space) === run;
  const prev = getTrack(space);
  const startedAt = prev && prev.item === item && prev.state === 'running' ? prev.startedAt : now();
  let t: InstallTrack = { item, state: 'running', startedAt, lines: [], error: null, kind: null, manual: [] };
  const put = (patch: Partial<InstallTrack>) => {
    t = { ...t, ...patch };
    if (live()) set(space, t);
    return t;
  };
  put({});
  const fail = (error: string | null) => put({ state: 'failed', error, kind: installFailKind([error, ...t.lines.slice(-3)].join('\n'), online()) });
  try {
    if (!online()) return fail('offline');
    let started: { status: number; body: any };
    for (;;) {
      started = await api.install(item);
      if (!live()) return t;
      if (started.status !== 409) break;
      await sleep(busyMs);
      if (!live()) return t;
    }
    if (started.status === 200) {
      put({ state: 'done' });
      opts.onDone?.();
      return t;
    }
    if (started.status === 422) {
      const manual: ManualStep[] = Array.isArray(started.body?.manual)
        ? started.body.manual.filter((m: any) => typeof m?.command === 'string').map((m: any) => ({ label: String(m.label ?? ''), command: m.command }))
        : [];
      put({ state: 'needs_admin', manual, error: typeof started.body?.error === 'string' ? started.body.error : null });
      // Checks every few seconds; moves on by itself once the part is there.
      for (;;) {
        await sleep(4000);
        if (!live()) return t;
        try {
          const items = await api.status();
          const row = items.find((i) => i.id === item);
          if (row && row.state !== 'missing') {
            put({ state: 'done' });
            opts.onDone?.();
            return t;
          }
        } catch {
          /* look again */
        }
      }
    }
    if (started.status !== 202 || !started.body?.job_id) return fail(typeof started.body?.error === 'string' ? started.body.error : `HTTP ${started.status}`);
    const id: string = started.body.job_id;
    let after = 0;
    for (;;) {
      const job = await api.job(id, after);
      if (!live()) return t;
      if (job.lines?.length) put({ lines: [...t.lines, ...job.lines].slice(-200) });
      after = typeof job.next === 'number' ? job.next : after;
      if (job.state === 'done') {
        put({ state: 'done' });
        opts.onDone?.();
        return t;
      }
      if (job.state === 'failed') return fail(job.error ?? null);
      await sleep(pollMs);
    }
  } catch (e) {
    if (!live()) return t;
    return fail(e instanceof Error ? e.message : null);
  }
}

/** Copy details: what went wrong, for a support message. Never shown in the card. */
export function installDetails(t: InstallTrack): string {
  return [`Getting ${t.item} ready failed.`, t.error ? `Error: ${t.error}` : null, ...t.lines.slice(-40)].filter(Boolean).join('\n');
}

/** Re-renders when the stored choice, "kept" or "dismissed" changes in this browser. */
export const useStartVersion = () => useSyncExternalStore(onStartChange, getStartVersion, getStartVersion);
