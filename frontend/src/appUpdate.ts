// In-app updates for the Mac app. The Mac shell checks GitHub for a newer signed release and hands
// the result to the in-process server; this module polls GET /api/app/update and drives the banner
// and the About row in App settings. Off the Mac app (VPS, Docker, dev) the endpoint answers 404 or
// supported=false, and then nothing renders and nothing polls again.

export type UpdateState = 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error';

export interface AvailableUpdate {
  version: string;
  /** That version's CHANGELOG section, markdown. */
  notes: string;
  date: string | null;
  /** The release page: "Download manually" when the in-app update fails. */
  url: string;
}

export interface UpdateStatus {
  current: string;
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
  /** null until the first answer, and forever when unsupported. */
  status: UpdateStatus | null;
  unsupported: boolean;
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

/** The server's answer, or null when it isn't the Mac app (or isn't an answer at all). */
export function parseUpdateStatus(raw: unknown): UpdateStatus | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  if (r.supported !== true) return null;
  const state = STATES.includes(r.state as UpdateState) ? (r.state as UpdateState) : 'idle';
  const progress = typeof r.progress === 'number' && Number.isFinite(r.progress) ? Math.min(1, Math.max(0, r.progress)) : null;
  return {
    current: str(r.current) ?? '',
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

export function bannerView(m: UpdateModel, dismissed: string | null): BannerView | null {
  if (m.unsupported || !m.status) return null;
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

/** The About row's one-line result. */
export function aboutLine(m: UpdateModel): string {
  const s = m.status;
  if (!s) return '';
  if (m.actionError) return m.actionError;
  if (m.phase === 'updated' && m.target) return `Updated to ${m.target}.`;
  switch (s.state) {
    case 'checking':
      return 'Checking for updates…';
    case 'available':
      return s.available ? `kwant bots ${s.available.version} is ready.` : 'You’re up to date.';
    case 'downloading':
      return s.progress == null ? 'Downloading the update…' : `Downloading the update… ${Math.round(s.progress * 100)}%`;
    case 'ready':
      return 'Restarting the app…';
    case 'error':
      return s.error ?? 'The last check failed.';
    default:
      return m.checkedAt != null ? 'You’re up to date.' : '';
  }
}

async function errorText(res: Response): Promise<string | null> {
  try {
    const body = await res.json();
    return str(body?.error);
  } catch {
    return null;
  }
}

function refusal(res: Response, fallback: string): Promise<string> | string {
  if (res.status === 401 || res.status === 403) return 'Only you can update the app, from the Mac it runs on.';
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
  install: () => Promise<void>;
  /** Hide the "Updated to …" note. */
  clearUpdated: () => void;
}

export function createUpdateController(env: ControllerEnv): UpdateController {
  let model: UpdateModel = { status: null, unsupported: false, phase: 'idle', target: null, actionError: null, installError: null, checkedAt: null };
  const listeners = new Set<() => void>();
  let timer: unknown = null;
  let running = false;
  let restartSince: number | null = null;

  const set = (patch: Partial<UpdateModel>) => {
    model = { ...model, ...patch };
    listeners.forEach((fn) => fn());
  };

  const schedule = (ms = nextPollMs(model)) => {
    if (timer !== null) env.clearTimeout(timer);
    timer = null;
    if (!running || model.unsupported) return;
    timer = env.setTimeout(() => {
      timer = null;
      void load();
    }, ms);
  };

  const apply = (status: UpdateStatus) => {
    const patch: Partial<UpdateModel> = { status, checkedAt: status.state === 'checking' ? model.checkedAt : env.now() };
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

  async function load() {
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
        return schedule(IDLE_POLL_MS);
      }
      return schedule();
    }
    if (res.status === 404) {
      set({ unsupported: true });
      return schedule();
    }
    if (!res.ok) return schedule(model.phase === 'restarting' ? FAST_POLL_MS : IDLE_POLL_MS);
    let body: unknown = null;
    try {
      body = await res.json();
    } catch {
      return schedule();
    }
    const status = parseUpdateStatus(body);
    if (!status) {
      set({ unsupported: true });
      return schedule();
    }
    apply(status);
    schedule();
  }

  async function post(path: string, fallback: string): Promise<boolean> {
    try {
      const res = await env.fetch(path, { method: 'POST', headers: { Accept: 'application/json' } });
      if (res.ok) {
        const status = parseUpdateStatus(await res.json().catch(() => null));
        if (status) apply(status);
        set({ actionError: null });
        return true;
      }
      set({ actionError: await refusal(res, fallback) });
    } catch {
      set({ actionError: 'The app didn’t answer. Try again in a moment.' });
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
      if (model.unsupported) return;
      set({ actionError: null, installError: null, phase: model.phase === 'updated' ? 'idle' : model.phase });
      await post('/api/app/update/check', 'Couldn’t check for updates.');
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
