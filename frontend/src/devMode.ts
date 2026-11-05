// Dev mode (0.7): the switch's safety cover, the first-switch-on setup, the per-space store and the
// API client. Person-only on the server: a bot's token gets 403, so the switch shows disabled.
// Contract: dev-mode/plan/plan.md §3.2 and §3.6, the route JSON in dev-mode/impl/lin.md.

import { personFetch } from './auth';

// ---------------------------------------------------------------------------
// Server shapes

export type DeployHint = 'docker' | 'cloudflare_pages' | 'fly' | null;

/** GET/PUT /api/spaces/:id/dev_mode */
export interface DevModeState {
  on: boolean;
  repo: string | null;
  branch: string | null;
  deploy_hint: DeployHint;
  backup_command: string | null;
  last_seen_sha: string | null;
  polled_at: string | null;
  poll_error: string | null;
  calls_today: number | null;
  calls_cap: number | null;
}

export type CheckState = 'found' | 'not_found' | 'not_connected';

/** POST /api/spaces/:id/dev_mode/check */
export interface RepoCheck {
  state: CheckState;
  repo: string | null;
  branch: string | null;
  deploy_hint: DeployHint;
}

const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const hint = (v: unknown): DeployHint => (v === 'docker' || v === 'cloudflare_pages' || v === 'fly' ? v : null);

export function parseDevMode(raw: unknown): DevModeState | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  if (typeof o.on !== 'boolean') return null;
  return {
    on: o.on,
    repo: str(o.repo),
    branch: str(o.branch),
    deploy_hint: hint(o.deploy_hint),
    backup_command: str(o.backup_command),
    last_seen_sha: str(o.last_seen_sha),
    polled_at: str(o.polled_at),
    poll_error: str(o.poll_error),
    calls_today: num(o.calls_today),
    calls_cap: num(o.calls_cap),
  };
}

export function parseCheck(raw: unknown): RepoCheck | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  if (o.state !== 'found' && o.state !== 'not_found' && o.state !== 'not_connected') return null;
  return { state: o.state, repo: str(o.repo), branch: str(o.branch), deploy_hint: hint(o.deploy_hint) };
}

/** A space that was never switched on (or has no repo) gets the setup sheet on its first switch-on. */
export const needsSetup = (s: DevModeState | null): boolean => !s || !s.repo || (!s.on && !s.polled_at && !s.last_seen_sha);

// ---------------------------------------------------------------------------
// Repo input: a URL, `owner/name` or `git@github.com:owner/name.git`, normalised to `owner/name`

const PART = /^[A-Za-z0-9_.-]{1,100}$/;

export function normaliseRepo(input: string): string | null {
  let s = input.trim();
  if (!s) return null;
  s = s.replace(/^git@github\.com:/i, '');
  s = s.replace(/^(?:https?:\/\/|git:\/\/|ssh:\/\/git@)?(?:www\.)?github\.com\//i, '');
  s = s.replace(/[?#].*$/, '').replace(/\/+$/, '').replace(/\.git$/i, '');
  const [owner, name, ...rest] = s.split('/');
  // github.com/owner/name/tree/main and /pull/3 still name the repo
  if (!owner || !name || (rest.length && !['tree', 'blob', 'pull', 'pulls', 'issues', 'actions', 'commit', 'commits'].includes(rest[0]))) return null;
  if (!PART.test(owner) || !PART.test(name) || name === '.' || name === '..') return null;
  return `${owner}/${name}`;
}

// ---------------------------------------------------------------------------
// The switch: a safety cover over a launch switch (design §1)

/** How long a lifted cover waits before it drops back with nothing changed. */
export const RECOVER_MS = 5000;
/** After a flip the cover drops back at 450 ms (150 ms drop), and presses are ignored until then. */
export const SETTLE_MS = 450;
export const DROP_MS = 150;

export type SwitchPhase = 'covered' | 'lifted' | 'pending' | 'settling';

export interface SwitchState {
  /** The committed value, as the server last said (or as just pressed, while saving). */
  on: boolean;
  phase: SwitchPhase;
  /** When the cover was lifted, for the 5 s re-cover. */
  liftedAt: number | null;
  /** Presses are ignored until this time (the settle after a flip). */
  settleUntil: number;
  /** The space hasn't been set up: the second press asks for the setup sheet. */
  setup: boolean;
  /** Not the person (403), or the server doesn't have Dev mode. */
  disabled: boolean;
  /** Under prefers-reduced-motion every change is instant, so there is nothing to settle. */
  reduced: boolean;
  /** What the polite live region says next; seq changes on every announcement. */
  announce: { text: string; seq: number } | null;
  /** A save is in flight (the press already flipped the switch). */
  saving: boolean;
  error: string | null;
}

export type SwitchAction =
  | { type: 'press'; now: number }
  | { type: 'recover'; now: number }
  | { type: 'escape' }
  | { type: 'blur' }
  | { type: 'settled'; now: number }
  | { type: 'setup_done'; now: number }
  | { type: 'setup_cancel' }
  | { type: 'saved'; on: boolean }
  | { type: 'save_failed'; error: string }
  | { type: 'server'; state: DevModeState | null; disabled?: boolean }
  | { type: 'reduced'; reduced: boolean };

export const initialSwitch = (over: Partial<SwitchState> = {}): SwitchState => ({
  on: false,
  phase: 'covered',
  liftedAt: null,
  settleUntil: 0,
  setup: true,
  disabled: false,
  reduced: false,
  announce: null,
  saving: false,
  error: null,
  ...over,
});

const say = (s: SwitchState, text: string) => ({ text, seq: (s.announce?.seq ?? 0) + 1 });

export const onText = (space: string) => `Dev mode on for ${space}.`;
export const offText = (space: string) => `Dev mode off for ${space}. Reviews, the ship gate and the production floor stopped.`;
export const liftText = (on: boolean) => `Unlocked. Press again to turn Dev mode ${on ? 'off' : 'on'}.`;

/**
 * The switch's state machine. Pure: time comes in on the actions, and the component runs the timers
 * (re-cover at liftedAt + 5 s, settled at settleUntil). `space` names the space in announcements.
 */
export function switchReducer(s: SwitchState, a: SwitchAction, space = 'this space'): SwitchState {
  switch (a.type) {
    case 'press': {
      if (s.disabled || s.saving) return s;
      if (s.phase === 'pending') return s;
      // A fast double-click: the second press lands while the cover settles, and is ignored.
      if (s.phase === 'settling' || a.now < s.settleUntil) return s;
      if (s.phase === 'covered') return { ...s, phase: 'lifted', liftedAt: a.now, error: null, announce: say(s, liftText(s.on)) };
      // phase lifted: the second press flips, or opens the setup the first time
      if (!s.on && s.setup) return { ...s, phase: 'pending', liftedAt: null, announce: say(s, 'Dev mode, setting up.') };
      return flip(s, !s.on, a.now, space);
    }
    case 'recover':
      if (s.phase !== 'lifted' || s.liftedAt === null || a.now - s.liftedAt < RECOVER_MS) return s;
      return { ...s, phase: 'covered', liftedAt: null, announce: say(s, 'Locked.') };
    case 'escape':
    case 'blur':
      if (s.phase !== 'lifted') return s;
      return { ...s, phase: 'covered', liftedAt: null, announce: say(s, 'Locked.') };
    case 'settled':
      if (s.phase !== 'settling' || a.now < s.settleUntil) return s;
      return { ...s, phase: 'covered' };
    case 'setup_done':
      if (s.phase !== 'pending') return s;
      return flip({ ...s, setup: false }, true, a.now, space);
    case 'setup_cancel':
      if (s.phase !== 'pending') return s;
      return { ...s, phase: 'covered', liftedAt: null, announce: say(s, 'Locked.') };
    case 'saved':
      return { ...s, saving: false, on: a.on, setup: a.on ? false : s.setup };
    case 'save_failed':
      // The press didn't take: back to what it was, covered, with the reason.
      return { ...s, saving: false, on: !s.on, phase: 'covered', liftedAt: null, settleUntil: 0, error: a.error, announce: say(s, a.error) };
    case 'server': {
      const disabled = a.disabled ?? s.disabled;
      if (!a.state) return { ...s, disabled };
      // A save in flight, or a cover the person is holding open, keeps what they see.
      if (s.saving || s.phase === 'pending') return { ...s, disabled, setup: needsSetup(a.state) };
      return { ...s, disabled, on: a.state.on, setup: needsSetup(a.state), phase: s.phase === 'lifted' && a.state.on !== s.on ? 'covered' : s.phase };
    }
    case 'reduced':
      return { ...s, reduced: a.reduced };
  }
}

function flip(s: SwitchState, on: boolean, now: number, space: string): SwitchState {
  return {
    ...s,
    on,
    saving: true,
    liftedAt: null,
    // Reduced motion: no hinge to settle, it locks at once. Still two presses to the next flip.
    phase: s.reduced ? 'covered' : 'settling',
    settleUntil: s.reduced ? 0 : now + SETTLE_MS,
    error: null,
    announce: say(s, on ? onText(space) : offText(space)),
  };
}

/** What the switch looks like: cover open or closed, knob side, fill opacity, ring and hint. */
export interface SwitchLook {
  coverOpen: boolean;
  knobRight: boolean;
  fill: number;
  ring: boolean;
  hint: string | null;
  label: string;
  tooltip: string;
}

export function switchLook(s: SwitchState): SwitchLook {
  const unavailable = s.disabled ? ', unavailable' : '';
  const tooltip = s.disabled ? 'Only the space owner can change Dev mode' : `Dev mode is ${s.on ? 'on' : 'off'}. Press to unlock.`;
  switch (s.phase) {
    case 'lifted':
      return {
        coverOpen: true,
        knobRight: s.on,
        fill: s.on ? 1 : 0,
        ring: true,
        hint: `Press again to turn Dev mode ${s.on ? 'off' : 'on'}`,
        label: `Dev mode, ${s.on ? 'on' : 'off'}. Unlocked. Press again to turn it ${s.on ? 'off' : 'on'}`,
        tooltip,
      };
    case 'pending':
      return { coverOpen: true, knobRight: true, fill: 0.45, ring: true, hint: null, label: 'Dev mode, setting up.', tooltip };
    case 'settling':
      // Open while it settles, then it drops; the label is already the final state.
      return { coverOpen: true, knobRight: s.on, fill: s.on ? 1 : 0, ring: false, hint: null, label: `Dev mode, ${s.on ? 'on' : 'off'}. Press to unlock`, tooltip };
    default:
      return { coverOpen: false, knobRight: s.on, fill: s.on ? 1 : 0, ring: false, hint: null, label: `Dev mode, ${s.on ? 'on' : 'off'}. Press to unlock${unavailable}`, tooltip };
  }
}

// ---------------------------------------------------------------------------
// Setup sheet: the Repository field (design §3 table)

export type FieldState = 'empty' | 'invalid' | 'checking' | 'found' | 'not_found' | 'not_connected' | 'error';

export interface FieldView {
  chip: { tone: 'info' | 'ok' | 'bad' | 'warn'; text: string; spinner?: boolean } | null;
  /** Plain help text, or null for the "filled in from this team's repo" default. */
  help: string | null;
  /** The repo name shown bold inside the not-found help line. */
  helpRepo: string | null;
  connect: boolean;
  canTurnOn: boolean;
}

export const DEFAULT_HELP = 'Filled in from this team’s repo. Read with your GitHub connection from Loose ends; nothing is written.';
export const EMPTY_HELP = 'Paste a GitHub repo, like owner/name or its URL.';

export function fieldView(state: FieldState, repo: string | null, error: string | null = null): FieldView {
  switch (state) {
    case 'checking':
      return { chip: { tone: 'info', text: 'Checking', spinner: true }, help: null, helpRepo: null, connect: false, canTurnOn: false };
    case 'found':
      return { chip: { tone: 'ok', text: 'Found' }, help: DEFAULT_HELP, helpRepo: null, connect: false, canTurnOn: true };
    case 'not_found':
      return {
        chip: { tone: 'bad', text: 'Can’t see it' },
        help: 'kwant can’t see {repo} with your GitHub connection. Check the name, or give Loose ends access to it.',
        helpRepo: repo,
        connect: false,
        canTurnOn: false,
      };
    case 'not_connected':
      return { chip: { tone: 'warn', text: 'Not connected' }, help: 'Connect GitHub to read this repo. kwant only reads.', helpRepo: null, connect: true, canTurnOn: false };
    case 'invalid':
      return { chip: { tone: 'bad', text: 'Not a repo' }, help: EMPTY_HELP, helpRepo: null, connect: false, canTurnOn: false };
    case 'error':
      return { chip: { tone: 'warn', text: 'Can’t check' }, help: error ?? 'GitHub can’t be reached right now. Try again in a minute.', helpRepo: null, connect: false, canTurnOn: false };
    default:
      return { chip: null, help: EMPTY_HELP, helpRepo: null, connect: false, canTurnOn: false };
  }
}

export const CHECK_DEBOUNCE_MS = 400;

export interface CheckerTimers {
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (h: unknown) => void;
}

export interface CheckerUpdate {
  state: FieldState;
  check: RepoCheck | null;
  error: string | null;
}

/**
 * The Repository field's checker: each input waits 400 ms, then asks the server. Only the latest
 * input's answer counts, so a slow reply to an earlier name never overwrites a newer one.
 */
export function createRepoChecker(
  check: (repo: string) => Promise<RepoCheck>,
  onUpdate: (u: CheckerUpdate) => void,
  timers: CheckerTimers = { setTimeout: (f, ms) => setTimeout(f, ms), clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) },
  delay = CHECK_DEBOUNCE_MS,
) {
  let handle: unknown = null;
  let seq = 0;
  return {
    input(raw: string) {
      const mine = ++seq;
      if (handle !== null) timers.clearTimeout(handle);
      handle = null;
      if (!raw.trim()) return onUpdate({ state: 'empty', check: null, error: null });
      const repo = normaliseRepo(raw);
      if (!repo) return onUpdate({ state: 'invalid', check: null, error: null });
      onUpdate({ state: 'checking', check: null, error: null });
      handle = timers.setTimeout(() => {
        handle = null;
        check(repo).then(
          (c) => mine === seq && onUpdate({ state: c.state, check: c, error: null }),
          (e: unknown) => mine === seq && onUpdate({ state: 'error', check: null, error: e instanceof Error ? e.message : null }),
        );
      }, delay);
    },
    cancel() {
      seq++;
      if (handle !== null) timers.clearTimeout(handle);
      handle = null;
    },
  };
}

export const deployLabel = (h: DeployHint): { name: string; detail: string } | null =>
  h === 'docker' ? { name: 'Docker', detail: 'compose file' } : h === 'cloudflare_pages' ? { name: 'Cloudflare Pages', detail: 'static site' } : h === 'fly' ? { name: 'Fly.io', detail: 'fly.toml' } : null;

export const approvalsWord = (a: string | undefined) => (a === 'off' ? 'Off' : a === 'smart' ? 'Smart' : 'Ask');

// ---------------------------------------------------------------------------
// API

export class DevModeError extends Error {
  constructor(message: string, public status: number) {
    super(message);
  }
}

async function call<T>(url: string, method = 'GET', body?: unknown, send: typeof fetch = (...a) => personFetch(a[0] as string, a[1])): Promise<{ status: number; data: T }> {
  let res: Response;
  try {
    res = await send(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new DevModeError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const data = await res.json().catch(() => null);
  if (res.ok) return { status: res.status, data: data as T };
  const err = data && typeof data === 'object' ? (data as Record<string, unknown>).error : null;
  if (res.status === 403) throw new DevModeError('Only the space owner can change Dev mode.', 403);
  if (res.status === 404 && !err) throw new DevModeError('This server doesn’t have Dev mode yet. Update the app.', 404);
  throw new DevModeError(typeof err === 'string' && err.trim() ? err : `Something went wrong (HTTP ${res.status}).`, res.status);
}

const enc = encodeURIComponent;

export const devApi = {
  async get(space: string): Promise<DevModeState> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode`);
    const s = parseDevMode(r.data);
    if (!s) throw new DevModeError('The server sent Dev mode in a shape this app doesn’t know.', 500);
    return s;
  },
  async put(space: string, body: { on: boolean; repo?: string | null; branch?: string | null; backup_command?: string | null }): Promise<DevModeState> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode`, 'PUT', body);
    const s = parseDevMode(r.data);
    if (!s) throw new DevModeError('The server sent Dev mode in a shape this app doesn’t know.', 500);
    return s;
  },
  async check(space: string, repo: string): Promise<RepoCheck> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode/check`, 'POST', { repo });
    const c = parseCheck(r.data);
    if (!c) throw new DevModeError('The server sent a repo check this app doesn’t know.', 500);
    return c;
  },
  landed: (space: string, before?: string) =>
    call<unknown>(`/api/spaces/${enc(space)}/dev/landed?limit=30${before ? `&before=${enc(before)}` : ''}`).then((r) => r.data),
  seen: (space: string) => call<{ seen_at: string }>(`/api/spaces/${enc(space)}/dev/landed/seen`, 'POST').then((r) => r.data),
  reviews: (space: string) => call<unknown>(`/api/spaces/${enc(space)}/dev/reviews`).then((r) => r.data),
  finding: (card: string, fid: string, body: { action: string; reason?: string; note?: string }) =>
    call<unknown>(`/api/dev/cards/${enc(card)}/findings/${enc(fid)}`, 'POST', body).then((r) => r.data),
  /** Not in lin-dev-core 492019f yet (asked Lin): the ready release-notes card after its SSE. */
  card: (id: string) => call<unknown>(`/api/dev/cards/${enc(id)}`).then((r) => r.data),
  releaseNotes: (space: string) => call<unknown>(`/api/spaces/${enc(space)}/dev/release_notes`, 'POST').then((r) => r.data),
  production: (connection: string, space: string, production: boolean) =>
    call<{ id: string; production: boolean }>(`/api/connections/${enc(connection)}/production?space=${enc(space)}`, 'PUT', { production }).then((r) => r.data),
};

export const _call = call;

// ---------------------------------------------------------------------------
// The per-space store: the header, rail badge, hard hats and grid all read it

type Listener = () => void;
const states = new Map<string, DevModeState>();
const listeners = new Set<Listener>();
let current: string | null = null;

export const devStore = {
  get: (space: string | null | undefined): DevModeState | null => (space ? states.get(space) ?? null : null),
  isOn: (space: string | null | undefined): boolean => !!(space && states.get(space)?.on),
  set(space: string, s: DevModeState) {
    states.set(space, s);
    listeners.forEach((l) => l());
  },
  /** The `dev_mode {space_id, on}` SSE event: flips the cached value at once, then the caller refetches. */
  setOn(space: string, on: boolean) {
    const s = states.get(space);
    if (s && s.on === on) return;
    states.set(space, { ...(s ?? emptyState()), on });
    listeners.forEach((l) => l());
  },
  setCurrent(space: string | null) {
    if (current === space) return;
    current = space;
    listeners.forEach((l) => l());
  },
  current: () => current,
  subscribe(l: Listener) {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
  /** Tests only. */
  _reset() {
    states.clear();
    current = null;
  },
};

export const emptyState = (): DevModeState => ({
  on: false,
  repo: null,
  branch: null,
  deploy_hint: null,
  backup_command: null,
  last_seen_sha: null,
  polled_at: null,
  poll_error: null,
  calls_today: null,
  calls_cap: null,
});

// ---------------------------------------------------------------------------
// Dev desk

/** "Today", "Yesterday", or "Tue 3 Nov", in the viewer's local time. */
export function dayLabel(iso: string, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(now) - day(d)) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  return d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });
}

/** The day separator goes on the first card of each day, newest first. */
export function withDayLabels<T extends { created_at?: string | null; merged_at?: string | null }>(cards: T[], now = new Date()): Array<{ card: T; dayLabel: string | null }> {
  let last = '';
  return cards.map((card) => {
    const label = dayLabel(card.merged_at ?? card.created_at ?? '', now);
    const show = label && label !== last ? label : null;
    if (label) last = label;
    return { card, dayLabel: show };
  });
}

/** The Dev desk row's status: "3 landed · 1 review open". */
export function deskStatus(unseenLanded: number, openReviews: number): string {
  const parts = [];
  parts.push(`${unseenLanded} landed`);
  parts.push(`${openReviews} ${openReviews === 1 ? 'review' : 'reviews'} open`);
  return parts.join(' · ');
}
