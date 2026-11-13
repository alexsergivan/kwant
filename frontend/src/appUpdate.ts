// Updates. The Mac shell checks GitHub for a newer signed release and installs it; elsewhere (VPS,
// Docker, Windows) the server checks GitHub's latest release itself and only reports it
// (can_install=false, with `how`: the command that updates this install). This module polls
// GET /api/app/update and drives the Mac banner and the kwant card in App settings. An off answer
// (404 or supported=false) is never final: the Mac app registers its updater a few seconds after the
// window first asks, so off asks again after 10 s, 60 s, then every 30 minutes, and a later supported
// answer turns everything back on.

export type UpdateState = 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error';

export interface AvailableUpdate {
  version: string;
  /** That version's CHANGELOG section, markdown. */
  notes: string;
  date: string | null;
  /** The release page: "Download manually" when the in-app update fails. */
  url: string;
}

/** How this install updates when the app can't do it: one command (none on Windows: a new installer). */
export interface HowToUpdate {
  kind: 'docker' | 'binary' | 'windows' | 'source';
  command: string | null;
}

export interface UpdateStatus {
  current: string;
  /** The Mac app installs; elsewhere Update opens How to update. */
  canInstall: boolean;
  /** When the server last heard back from GitHub (ms), null before the first answer. */
  lastChecked: number | null;
  how: HowToUpdate | null;
  state: UpdateState;
  available: AvailableUpdate | null;
  progress: number | null;
  error: string | null;
}

/** What the banner shows, or null for no banner. */
export type BannerView =
  | { kind: 'available'; version: string; notes: string; url: string }
  | { kind: 'downloading'; version: string; progress: number | null }
  | { kind: 'restarting'; version: string }
  | { kind: 'error'; version: string; error: string; url: string; notes: string }
  | { kind: 'updated'; version: string };

/** The client's own part: an install it started, and the relaunch after it. */
export type UpdatePhase = 'idle' | 'installing' | 'restarting' | 'updated' | 'stalled';

export interface UpdateModel {
  /** null until the first supported answer. */
  status: UpdateStatus | null;
  /** The last answer was off (404 or supported=false). Asked again later; a supported answer clears it. */
  unsupported: boolean;
  /** Check again pressed while off: the ask is out. */
  rechecking?: boolean;
  /** The running version, also when updates are unsupported (null when the server never said). */
  current: string | null;
  phase: UpdatePhase;
  /** The version being installed, or the one just installed. */
  target: string | null;
  /** A refused check or install (409, 403), in the person's words. */
  actionError: string | null;
  /** The same, when it was the install that was refused: the banner shows it with Retry. */
  installError: string | null;
  checkedAt: number | null;
}

export const UPDATE_DISMISSED_KEY = 'kwnt_update_dismissed';
export const UPDATE_PENDING_KEY = 'kwnt_update_pending';

export const IDLE_POLL_MS = 30 * 60 * 1000;
/** After an off answer: ask again after 10 s, then 60 s, then every IDLE_POLL_MS. */
export const OFF_RETRY_MS = [10_000, 60_000];
/** Where "Get the latest version" goes when this app can't check by itself. */
export const LATEST_RELEASE_URL = 'https://github.com/alexsergivan/kwant/releases/latest';
/** Opening the kwant card checks again when the last answer is older than this. */
export const STALE_CHECK_MS = 60 * 60 * 1000;
export const FAST_POLL_MS = 1000;
/** After this long without the app coming back, say so instead of spinning forever. */
export const RESTART_GIVE_UP_MS = 3 * 60 * 1000;

const STATES: UpdateState[] = ['idle', 'checking', 'available', 'downloading', 'ready', 'error'];

const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);

function parseAvailable(raw: unknown): AvailableUpdate | null {
  if (!raw || typeof raw !== 'object') return null;
  const a = raw as Record<string, unknown>;
  const version = str(a.version);
  if (!version) return null;
  const url = str(a.url);
  return {
    version,
    notes: typeof a.notes === 'string' ? a.notes : '',
    date: str(a.date),
    url: url && /^https:\/\//.test(url) ? url : `https://github.com/alexsergivan/kwant/releases/tag/mac-v${version}`,
  };
}

const HOW_KINDS: HowToUpdate['kind'][] = ['docker', 'binary', 'windows', 'source'];

function parseHow(raw: unknown): HowToUpdate | null {
  if (!raw || typeof raw !== 'object') return null;
  const h = raw as Record<string, unknown>;
  if (!HOW_KINDS.includes(h.kind as HowToUpdate['kind'])) return null;
  return { kind: h.kind as HowToUpdate['kind'], command: str(h.command) };
}

/** The server's answer, or null when updates are off there (or it isn't an answer at all). */
export function parseUpdateStatus(raw: unknown): UpdateStatus | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  if (r.supported !== true) return null;
  const checked = typeof r.checked_at === 'string' ? Date.parse(r.checked_at) : NaN;
  const state = STATES.includes(r.state as UpdateState) ? (r.state as UpdateState) : 'idle';
  const progress = typeof r.progress === 'number' && Number.isFinite(r.progress) ? Math.min(1, Math.max(0, r.progress)) : null;
  return {
    current: str(r.current) ?? '',
    // A Mac app from before can_install only ever answered supported=true from the app itself
    canInstall: r.can_install !== false,
    lastChecked: Number.isFinite(checked) ? checked : null,
    how: parseHow(r.how),
    state,
    available: parseAvailable(r.available),
    progress,
    error: str(r.error),
  };
}

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

const defaultStore = (): Store | undefined => {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
};

function read(key: string, store: Store | undefined): string | null {
  try {
    return store?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function write(key: string, value: string | null, store: Store | undefined) {
  try {
    if (value === null) store?.removeItem(key);
    else store?.setItem(key, value);
  } catch {
    // Private browsing or blocked storage: the banner just comes back next time.
  }
}

/** Dismissing hides the banner for that version only; a newer one shows it again. */
export const dismissedVersion = (store = defaultStore()) => read(UPDATE_DISMISSED_KEY, store);
export const dismissVersion = (version: string, store = defaultStore()) => write(UPDATE_DISMISSED_KEY, version, store);

/** The Mac app's bar. Elsewhere the update can't run from the app, so the kwant card says it. */
export function bannerView(m: UpdateModel, dismissed: string | null): BannerView | null {
  if (m.unsupported || !m.status || !m.status.canInstall) return null;
  if (m.phase === 'updated' && m.target) return { kind: 'updated', version: m.target };
  if (m.phase === 'restarting' && m.target) return { kind: 'restarting', version: m.target };
  if (m.phase === 'stalled' && m.target) {
    return {
      kind: 'error',
      version: m.target,
      error: 'The app didn’t come back after the update. Open kwant bots again from Applications.',
      url: m.status.available?.url ?? `https://github.com/alexsergivan/kwant/releases/tag/mac-v${m.target}`,
      notes: m.status.available?.notes ?? '',
    };
  }
  const a = m.status.available;
  if (!a) return null;
  const s = m.status.state;
  if (s === 'ready') return { kind: 'restarting', version: a.version };
  if (s === 'downloading' || m.phase === 'installing') return { kind: 'downloading', version: a.version, progress: m.status.progress };
  if (s === 'error' || m.installError) {
    return { kind: 'error', version: a.version, error: m.installError ?? m.status.error ?? 'The update failed.', url: a.url, notes: a.notes };
  }
  if (s !== 'available') return null;
  if (dismissed === a.version) return null;
  return { kind: 'available', version: a.version, notes: a.notes, url: a.url };
}

/** Fast while something is moving, every 30 minutes otherwise. */
export function nextPollMs(m: UpdateModel): number {
  if (m.phase === 'installing' || m.phase === 'restarting') return FAST_POLL_MS;
  const s = m.status?.state;
  return s === 'checking' || s === 'downloading' || s === 'ready' ? FAST_POLL_MS : IDLE_POLL_MS;
}

/** The wait before asking again after `offAnswers` off answers in a row (1 = the first). */
export function offRetryMs(offAnswers: number, phase: UpdatePhase = 'idle'): number {
  // A relaunch after an install: the new app's updater registers seconds later, keep asking fast
  if (phase === 'installing' || phase === 'restarting') return FAST_POLL_MS;
  return OFF_RETRY_MS[offAnswers - 1] ?? IDLE_POLL_MS;
}

/** A failed check (not a failed install): the Mac app and the server both start it so. */
export const isCheckError = (error: string | null) => !!error && /^Couldn[’']t check/i.test(error);

/** Line 2 of the kwant card: exactly one of these. */
export type VersionLine =
  | { kind: 'loading' }
  | { kind: 'off'; rechecking: boolean }
  | { kind: 'checking' }
  | { kind: 'latest'; checkedAt: number | null }
  | { kind: 'available'; version: string; notes: string; date: string | null; url: string; canInstall: boolean }
  | { kind: 'downloading'; version: string; progress: number | null }
  | { kind: 'restarting'; version: string }
  | { kind: 'install-error'; version: string; error: string; url: string; notes: string; date: string | null }
  | { kind: 'check-error'; error: string }
  | { kind: 'updated'; version: string };

/** Never says "latest" without a check that said so. */
export function versionLine(m: UpdateModel): VersionLine {
  if (m.unsupported) return { kind: 'off', rechecking: !!m.rechecking };
  const s = m.status;
  if (!s) return { kind: 'loading' };
  if (m.phase === 'updated' && m.target) return { kind: 'updated', version: m.target };
  if (m.phase === 'restarting' && m.target) return { kind: 'restarting', version: m.target };
  const a = s.available;
  if (m.phase === 'stalled' && m.target) {
    return {
      kind: 'install-error',
      version: m.target,
      error: 'The app didn’t come back after the update. Open kwant bots again from Applications.',
      url: a?.url ?? `https://github.com/alexsergivan/kwant/releases/tag/mac-v${m.target}`,
      notes: a?.notes ?? '',
      date: a?.date ?? null,
    };
  }
  if (s.state === 'checking') return { kind: 'checking' };
  if (a && s.state === 'ready') return { kind: 'restarting', version: a.version };
  if (a && (s.state === 'downloading' || m.phase === 'installing')) return { kind: 'downloading', version: a.version, progress: s.progress };
  const failed = m.installError ?? (s.state === 'error' ? s.error ?? 'The update failed.' : null);
  if (a && failed && !isCheckError(failed)) {
    return { kind: 'install-error', version: a.version, error: failed, url: a.url, notes: a.notes, date: a.date };
  }
  // A version found earlier is still there after a failed re-check: offering it isn't wrong
  if (a) return { kind: 'available', version: a.version, notes: a.notes, date: a.date, url: a.url, canInstall: s.canInstall };
  if (s.state === 'error') return { kind: 'check-error', error: s.error ?? 'Couldn’t check for updates.' };
  if (m.actionError) return { kind: 'check-error', error: m.actionError };
  // No answer from GitHub yet (the first check runs seconds after start)
  if (s.lastChecked == null) return { kind: 'checking' };
  return { kind: 'latest', checkedAt: s.lastChecked };
}

/** "Checked just now", "Checked 2 min ago", "Checked 3 h ago", "Checked 2 days ago". */
export function checkedAgo(at: number | null, now: number): string {
  if (at == null) return '';
  const min = Math.floor(Math.max(0, now - at) / 60000);
  if (min < 1) return 'Checked just now';
  if (min < 60) return `Checked ${min} min ago`;
  const h = Math.floor(min / 60);
  if (h < 24) return `Checked ${h} h ago`;
  const d = Math.floor(h / 24);
  return `Checked ${d} day${d === 1 ? '' : 's'} ago`;
}

async function errorText(res: Response): Promise<string | null> {
  try {
    const body = await res.json();
    return str(body?.error);
  } catch {
    return null;
  }
}

function refusal(res: Response, fallback: string, canInstall: boolean): Promise<string> | string {
  if (res.status === 401 || res.status === 403) {
    return canInstall ? 'Only you can update the app, from the Mac it runs on.' : 'Only the person who runs this kwant bots can check for updates.';
  }
  return errorText(res).then((e) => e ?? fallback);
}

export interface ControllerEnv {
  fetch: (url: string, init?: RequestInit) => Promise<Response>;
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (id: unknown) => void;
  now: () => number;
  store?: Store;
}

export interface UpdateController {
  get: () => UpdateModel;
  subscribe: (fn: () => void) => () => void;
  start: () => void;
  stop: () => void;
  check: () => Promise<void>;
  /** Checks quietly when the last answer is older than STALE_CHECK_MS (the kwant card opening); asks again when off. */
  checkIfStale: () => Promise<void>;
  install: () => Promise<void>;
  /** Hide the "Updated to …" note. */
  clearUpdated: () => void;
}

export function createUpdateController(env: ControllerEnv): UpdateController {
  let model: UpdateModel = { status: null, unsupported: false, current: null, phase: 'idle', target: null, actionError: null, installError: null, checkedAt: null };
  const listeners = new Set<() => void>();
  let timer: unknown = null;
  let running = false;
  let restartSince: number | null = null;
  /** Off answers to scheduled polls in a row: picks the next wait (OFF_RETRY_MS). Asks the person triggers don't count. */
  let offAnswers = 0;

  const set = (patch: Partial<UpdateModel>) => {
    model = { ...model, ...patch };
    listeners.forEach((fn) => fn());
  };

  const schedule = (ms = model.unsupported ? offRetryMs(offAnswers, model.phase) : nextPollMs(model)) => {
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    if (!running) return;
    timer = env.setTimeout(() => {
      timer = null;
      void load();
    }, ms);
  };

  const off = (current: string | null, asked: boolean) => {
    // A person's ask (Settings opened, Check again) doesn't push the next retry later; only the first off answer counts
    if (!asked || offAnswers === 0) offAnswers++;
    set({ unsupported: true, current: current ?? model.current });
  };

  const apply = (status: UpdateStatus) => {
    offAnswers = 0;
    const patch: Partial<UpdateModel> = { status, unsupported: false, current: status.current || model.current, checkedAt: status.state === 'checking' ? model.checkedAt : env.now() };
    const pending = read(UPDATE_PENDING_KEY, env.store);
    if (pending && status.current === pending) {
      // Back on the new version: in this page after the relaunch, or in the fresh window.
      write(UPDATE_PENDING_KEY, null, env.store);
      Object.assign(patch, { phase: 'updated', target: pending, actionError: null });
      restartSince = null;
    } else if (model.phase === 'installing' && status.state === 'ready') {
      Object.assign(patch, { phase: 'restarting' });
      restartSince = env.now();
    } else if ((model.phase === 'installing' || model.phase === 'restarting') && status.state === 'error') {
      write(UPDATE_PENDING_KEY, null, env.store);
      Object.assign(patch, { phase: 'idle', target: null });
      restartSince = null;
    } else if (pending && model.phase === 'idle' && status.state !== 'downloading' && status.state !== 'ready') {
      // An install from an earlier window that never landed: forget it, the banner offers it again.
      write(UPDATE_PENDING_KEY, null, env.store);
    }
    if (status.state !== 'error' && model.actionError && model.phase !== 'idle') patch.actionError = null;
    set(patch);
  };

  /** asked: the person triggered this ask. While still off it keeps the pending retry instead of a later one. */
  async function load(asked = false) {
    const next = (ms?: number) => (asked && model.unsupported && timer !== null ? undefined : schedule(ms));
    let res: Response;
    try {
      res = await env.fetch('/api/app/update', { headers: { Accept: 'application/json' } });
    } catch {
      // No server: it's relaunching after an install, or the network dropped.
      if (model.phase === 'installing' || model.status?.state === 'ready') {
        if (model.phase !== 'restarting') restartSince = env.now();
        set({ phase: 'restarting', target: model.target ?? model.status?.available?.version ?? null });
      }
      if (model.phase === 'restarting' && restartSince !== null && env.now() - restartSince > RESTART_GIVE_UP_MS) {
        set({ phase: 'stalled' });
        return next(IDLE_POLL_MS);
      }
      return next();
    }
    if (res.status === 404) {
      off(null, asked);
      return next();
    }
    if (!res.ok) return next(model.phase === 'restarting' ? FAST_POLL_MS : IDLE_POLL_MS);
    let body: unknown = null;
    try {
      body = await res.json();
    } catch {
      return next();
    }
    const status = parseUpdateStatus(body);
    if (!status) {
      off(body && typeof body === 'object' ? str((body as Record<string, unknown>).current) : null, asked);
      return next();
    }
    apply(status);
    next();
  }

  async function post(path: string, fallback: string, quiet = false): Promise<boolean> {
    try {
      const res = await env.fetch(path, { method: 'POST', headers: { Accept: 'application/json' } });
      if (res.ok) {
        const status = parseUpdateStatus(await res.json().catch(() => null));
        if (status) apply(status);
        set({ actionError: null });
        return true;
      }
      const why = await refusal(res, fallback, model.status?.canInstall ?? true);
      if (!quiet) set({ actionError: why });
    } catch {
      if (!quiet) set({ actionError: 'The app didn’t answer. Try again in a moment.' });
    }
    return false;
  }

  return {
    get: () => model,
    subscribe(fn) {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    start() {
      if (running) return;
      running = true;
      void load();
    },
    stop() {
      running = false;
      if (timer !== null) env.clearTimeout(timer);
      timer = null;
    },
    async check() {
      if (model.unsupported) {
        // Off here (so far): ask again now; a supported answer brings the card back
        set({ rechecking: true });
        try {
          await load(true);
        } finally {
          set({ rechecking: false });
        }
        return;
      }
      set({ actionError: null, installError: null, phase: model.phase === 'updated' ? 'idle' : model.phase });
      await post('/api/app/update/check', 'Couldn’t check for updates.');
      schedule();
    },
    async checkIfStale() {
      // Off is never final: opening Settings asks again
      if (model.unsupported) return load(true);
      const s = model.status;
      if (!s || model.phase !== 'idle') return;
      if (s.state === 'checking' || s.state === 'downloading' || s.state === 'ready') return;
      if (s.lastChecked != null && env.now() - s.lastChecked < STALE_CHECK_MS) return;
      await post('/api/app/update/check', 'Couldn’t check for updates.', true);
      schedule();
    },
    async install() {
      const version = model.status?.available?.version;
      if (model.unsupported || !version) return;
      write(UPDATE_PENDING_KEY, version, env.store);
      set({ phase: 'installing', target: version, actionError: null, installError: null });
      const ok = await post('/api/app/update/install', 'Couldn’t start the update.');
      if (!ok) {
        write(UPDATE_PENDING_KEY, null, env.store);
        set({ phase: 'idle', target: null, installError: model.actionError });
      }
      schedule();
    },
    clearUpdated() {
      if (model.phase === 'updated') set({ phase: 'idle', target: null });
    },
  };
}

let shared: UpdateController | null = null;

/** One poller for the whole page, shared by the banner and App settings. */
export function appUpdateController(): UpdateController {
  shared ??= createUpdateController({
    fetch: (url, init) => fetch(url, init),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (id) => window.clearTimeout(id as number),
    now: () => Date.now(),
    store: defaultStore(),
  });
  return shared;
}
