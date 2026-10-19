// "Local computer" vs "VM": which computer kwant's browser and shell use.
// Contract: GET/PUT /api/settings/computer (docs/browser-service.md)

export type ComputerMode = 'local' | 'vm';

/** Local mode only: the bots' Chrome runs without a window and shows in the app, unless the
 *  person opened it as a real window (to sign in or solve a CAPTCHA). */
export interface LocalWindow {
  visible: boolean;
  available: boolean;
  reason: string | null;
}

export interface ComputerSettings {
  mode: ComputerMode;
  available: Record<ComputerMode, boolean>;
  reasons: Record<ComputerMode, string | null>;
  window: LocalWindow;
  /** What vm mode runs in containers, as the server reports it ("browser", "kwnt_shell", later "cli_agents"). */
  isolates: string[];
}

/** What vm mode isolated before the server said so: never the CLI agents. */
export const DEFAULT_VM_ISOLATES = ['browser', 'kwnt_shell'];

const ISOLATED_PARTS: Record<string, string> = { browser: 'browser', kwnt_shell: 'built-in shell' };

/** VM copy built from what it really isolates, so it can't promise more than the server does. */
export function vmCopy(isolates: string[]): { label: string; hint: string; note: string | null } {
  if (isolates.includes('cli_agents')) {
    return { label: 'VM (isolated)', hint: 'Runs bots in Docker containers, away from your files.', note: null };
  }
  const parts = Object.keys(ISOLATED_PARTS).filter((k) => isolates.includes(k)).map((k) => ISOLATED_PARTS[k]);
  return {
    label: parts.length ? `VM (${parts.join(' and ')} isolated)` : 'VM',
    hint: parts.length ? `The ${parts.join(' and ')} run in Docker containers, away from your files.` : 'Uses Docker containers.',
    note: 'Claude, Codex and Grok still run on this computer, with their own file and shell tools.',
  };
}

export interface ModeCopy {
  id: ComputerMode;
  label: string;
  short: string;
  hint: string;
  /** A caveat shown under the hint. */
  note: string | null;
}

export const COMPUTER_MODES: ModeCopy[] = [
  { id: 'local', label: 'Local computer', short: 'Local', hint: 'Uses your Chrome and your files.', note: null },
  { id: 'vm', short: 'VM', ...vmCopy(DEFAULT_VM_ISOLATES) },
];

/** Both options, with the VM copy taken from what this server says it isolates. */
export const computerModes = (s: ComputerSettings | null | undefined): ModeCopy[] =>
  s ? COMPUTER_MODES.map((m) => (m.id === 'vm' ? { ...m, ...vmCopy(s.isolates) } : m)) : COMPUTER_MODES;

export const LOCAL_BROWSER_NOTE = "Your bots' Chrome is open as a window on your computer.";

/** Whether the browser is in its own window instead of the app's live view. */
export const showsLocalWindow = (s: ComputerSettings | null | undefined) => s?.mode === 'local' && s.window.visible;

/** A tab the bot hasn't opened anything in yet. Its frame is an empty page (black on macOS), so the panel shows "Browser idle". */
export const isBlankPage = (url: string | null | undefined) => {
  const u = (url ?? '').trim();
  return u === '' || u === 'about:blank';
};

/** The browser panel's footer: where Chrome runs and where you see it. */
export function browserFooterLabel(s: ComputerSettings | null | undefined, ready: boolean): string {
  if (s?.mode !== 'local') return ready ? 'Browser Ready' : 'Browser Offline';
  if (s.window.visible) return 'Chrome window on your computer';
  return ready ? 'Chrome shown here (no window)' : 'Chrome offline';
}

/** Fired on window after a successful switch so every open control shows the new mode. */
export const COMPUTER_MODE_EVENT = 'kwnt:computer-mode';

const isMode = (v: unknown): v is ComputerMode => v === 'local' || v === 'vm';

/** The response, or null when it isn't the expected shape. */
export function parseComputerSettings(data: unknown): ComputerSettings | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, any>;
  if (!isMode(d.mode) || !d.available || typeof d.available !== 'object') return null;
  const reason = (m: ComputerMode) => (typeof d.reasons?.[m] === 'string' && d.reasons[m].trim() ? d.reasons[m] : null);
  const w = d.window && typeof d.window === 'object' ? d.window : {};
  return {
    mode: d.mode,
    available: { local: d.available.local === true, vm: d.available.vm === true },
    reasons: { local: reason('local'), vm: reason('vm') },
    window: {
      visible: w.visible === true,
      available: w.available === true,
      reason: typeof w.reason === 'string' && w.reason.trim() ? w.reason : null,
    },
    // Older servers don't send it; they isolated the same things.
    isolates: Array.isArray(d.isolates) ? d.isolates.filter((x: unknown): x is string => typeof x === 'string') : DEFAULT_VM_ISOLATES,
  };
}

export const otherMode = (m: ComputerMode): ComputerMode => (m === 'local' ? 'vm' : 'local');

export const modeLabel = (m: ComputerMode, s?: ComputerSettings | null) => computerModes(s).find((x) => x.id === m)!.label;

/** Why `mode` can't be picked right now, or null when it can. */
export function unavailableReason(settings: ComputerSettings, mode: ComputerMode): string | null {
  if (settings.available[mode]) return null;
  return settings.reasons[mode] ?? `${modeLabel(mode, settings)} isn't available on this computer.`;
}

export interface SwitchState {
  settings: ComputerSettings | null;
  /** The mode being saved; shown as selected until the server answers. */
  pending: ComputerMode | null;
  error: string | null;
}

export type SwitchAction =
  | { type: 'loaded'; settings: ComputerSettings }
  | { type: 'start'; mode: ComputerMode }
  | { type: 'saved'; settings: ComputerSettings }
  | { type: 'failed'; error: string }
  | { type: 'dismiss' };

export const initialSwitchState: SwitchState = { settings: null, pending: null, error: null };

export function switchReducer(state: SwitchState, action: SwitchAction): SwitchState {
  switch (action.type) {
    case 'loaded':
      return state.pending ? state : { ...state, settings: action.settings };
    case 'start':
      return { ...state, pending: action.mode, error: null };
    case 'saved':
      return { settings: action.settings, pending: null, error: null };
    case 'failed':
      // Dropping `pending` puts the control back on the saved mode.
      return { ...state, pending: null, error: action.error };
    case 'dismiss':
      return { ...state, error: null };
  }
}

/** The mode a control should show as selected. */
export const shownMode = (s: SwitchState): ComputerMode | null => s.pending ?? s.settings?.mode ?? null;

/** null when this server has no computer setting (older build) or it couldn't be read. */
export async function fetchComputerSettings(): Promise<ComputerSettings | null> {
  const res = await fetch('/api/settings/computer', { cache: 'no-store' });
  if (!res.ok) return null;
  return parseComputerSettings(await res.json().catch(() => null));
}

type SaveResult = { ok: true; settings: ComputerSettings } | { ok: false; error: string };

async function putSettings(path: string, body: unknown, failed: string): Promise<SaveResult> {
  let res: Response;
  try {
    res = await fetch(path, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  } catch {
    return { ok: false, error: 'Could not reach the server.' };
  }
  const data = await res.json().catch(() => null);
  if (!res.ok) {
    const msg = data && typeof data.error === 'string' && data.error.trim() ? data.error : `${failed} (HTTP ${res.status}).`;
    return { ok: false, error: msg };
  }
  const settings = parseComputerSettings(data);
  return settings ? { ok: true, settings } : { ok: false, error: 'The server sent an unexpected answer.' };
}

export const saveComputerMode = (mode: ComputerMode) => putSettings('/api/settings/computer', { mode }, 'Switching failed');

/** Opens the bots' Chrome as a window (`true`) or puts it back in the app (`false`). */
export const saveLocalWindow = (visible: boolean) =>
  putSettings('/api/settings/computer/window', { visible }, visible ? 'Could not open the window' : 'Could not bring the browser back');
