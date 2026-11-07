// One-click installs from the app (0.7.5 O1): the Setup API the server already has, person-only
// (/api/setup/status, POST /api/setup/install, GET /api/setup/jobs/:id?after=N). Only the person's own
// pages call these (through personFetch, so "Confirm it's you" works); nothing here runs for a bot.

import { personFetch } from './auth';

/** Setup items the server can install (src/setup.rs ItemId). */
export type SetupItemId = 'laya' | 'browser' | 'claude' | 'codex' | 'grok' | 'docker';

export interface SetupItem {
  id: SetupItemId;
  label: string;
  state: 'ready' | 'missing' | 'managed';
  /** kwant can install it itself. */
  can_install: boolean;
}

export interface SetupJob {
  id: string;
  state: 'running' | 'done' | 'failed';
  error: string | null;
  lines: string[];
  next: number;
}

export interface SetupApi {
  status: () => Promise<SetupItem[]>;
  /** status 202 (started), 200 (already there), 409 (another install runs), 422 (needs a manual step). */
  install: (item: SetupItemId) => Promise<{ status: number; body: any }>;
  job: (id: string, after: number) => Promise<SetupJob>;
}

export const setupApi: SetupApi = {
  async status() {
    const r = await personFetch('/api/setup/status');
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const d = await r.json();
    return Array.isArray(d?.items) ? d.items : [];
  },
  async install(item) {
    const r = await personFetch('/api/setup/install', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ item }) });
    let body: any = null;
    try {
      body = await r.json();
    } catch {
      body = null;
    }
    return { status: r.status, body };
  },
  async job(id, after) {
    const r = await personFetch(`/api/setup/jobs/${encodeURIComponent(id)}?after=${after}`);
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    return r.json();
  },
};

/** Whether the server says it can install `item` here. null: unknown (older server, or not loaded). */
export const canInstall = (items: SetupItem[] | null, item: SetupItemId): boolean | null => {
  if (!items) return null;
  const row = items.find((i) => i.id === item);
  return row ? row.state === 'missing' && row.can_install : false;
};

export type InstallResult = { ok: true } | { ok: false; lastLine: string | null };

export interface InstallOptions {
  sleep?: (ms: number) => Promise<void>;
  /** Between job reads. */
  pollMs?: number;
  /** While another install runs (409): how long between tries, and how many. */
  busyMs?: number;
  busyTries?: number;
  /** Stops following (the page went away). */
  cancelled?: () => boolean;
}

const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * Installs one item and follows its job to the end. Another install running (409) is waited out and
 * tried again. Keeps only the last output line, for the failure's "Details".
 */
export async function runInstall(api: SetupApi, item: SetupItemId, opts: InstallOptions = {}): Promise<InstallResult> {
  const sleep = opts.sleep ?? wait;
  const pollMs = opts.pollMs ?? 1000;
  const busyMs = opts.busyMs ?? 3000;
  const busyTries = opts.busyTries ?? 100;
  let lastLine: string | null = null;
  let started: { status: number; body: any } | null = null;
  try {
    for (let i = 0; ; i++) {
      started = await api.install(item);
      if (started.status !== 409 || i >= busyTries || opts.cancelled?.()) break;
      await sleep(busyMs);
    }
    if (started.status === 200) return { ok: true };
    if (started.status !== 202 || !started.body?.job_id) {
      return { ok: false, lastLine: typeof started.body?.error === 'string' ? started.body.error : null };
    }
    const id: string = started.body.job_id;
    let after = 0;
    for (;;) {
      if (opts.cancelled?.()) return { ok: false, lastLine };
      const job = await api.job(id, after);
      if (job.lines?.length) lastLine = job.lines[job.lines.length - 1];
      after = typeof job.next === 'number' ? job.next : after;
      if (job.state === 'done') return { ok: true };
      if (job.state === 'failed') return { ok: false, lastLine: lastLine ?? job.error ?? null };
      await sleep(pollMs);
    }
  } catch (e) {
    return { ok: false, lastLine: lastLine ?? (e instanceof Error ? e.message : null) };
  }
}
