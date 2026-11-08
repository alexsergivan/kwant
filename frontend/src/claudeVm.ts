// "Run Claude inside the VM" (agents-in-vm, design/ui.md + design/ui-check-446.md): the switch
// under the VM card, the "Right now" chips and the status line under them.
// Contract: GET/PUT /api/settings/computer adds `isolate_claude`, `isolate_claude_toggle` and
// `agents` (src/server/computer.rs, src/sandbox/agent_vm/mod.rs).

export type AgentEngine = 'claude' | 'codex' | 'grok';
export const AGENT_ENGINES: AgentEngine[] = ['claude', 'codex', 'grok'];
export const ENGINE_NAMES: Record<AgentEngine, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok' };

export type AgentRunState = 'off' | 'isolated' | 'not_isolated' | 'preparing' | 'unavailable';

export interface AgentVmState {
  state: AgentRunState;
  reason: string | null;
  /** People's words from the server, e.g. why the setup failed. */
  detail: string | null;
  /** Only while preparing, and only if the server measures it (0.7.7 doesn't). */
  progress: { done_bytes: number; total_bytes: number } | null;
}

export interface ClaudeVm {
  /** The saved switch, kept in both modes. */
  on: boolean;
  /** Can the switch be turned ON right now. */
  toggle: { available: boolean; reason: string | null };
  agents: Partial<Record<AgentEngine, AgentVmState>>;
}

export const SETTING_NAME = 'Run Claude inside the VM';

const STATES: AgentRunState[] = ['off', 'isolated', 'not_isolated', 'preparing', 'unavailable'];
const str = (v: unknown) => (typeof v === 'string' && v.trim() ? v : null);

function parseAgent(v: unknown): AgentVmState | null {
  if (!v || typeof v !== 'object') return null;
  const a = v as Record<string, any>;
  if (!STATES.includes(a.state)) return null;
  const p = a.progress;
  const progress =
    p && typeof p === 'object' && Number.isFinite(p.done_bytes) && Number.isFinite(p.total_bytes) && p.total_bytes > 0
      ? { done_bytes: Math.max(0, Math.min(p.done_bytes, p.total_bytes)), total_bytes: p.total_bytes }
      : null;
  return { state: a.state, reason: str(a.reason), detail: str(a.detail), progress };
}

/** The switch's fields from the computer view, or null on a server without them (no row then). */
export function parseClaudeVm(d: Record<string, any>): ClaudeVm | null {
  if (typeof d.isolate_claude !== 'boolean') return null;
  const t = d.isolate_claude_toggle && typeof d.isolate_claude_toggle === 'object' ? d.isolate_claude_toggle : {};
  const agents: ClaudeVm['agents'] = {};
  for (const e of AGENT_ENGINES) {
    const a = parseAgent(d.agents?.[e]);
    if (a) agents[e] = a;
  }
  return { on: d.isolate_claude, toggle: { available: t.available === true, reason: str(t.reason) }, agents };
}

/** Hosts where the switch never works in this release: the hint is replaced by this line. */
export const PLATFORM_HINTS: Record<string, string> = {
  mac_later: 'Coming to Mac later. For now, Claude runs on this computer in both modes.',
  not_supported_yet: 'Only on Linux for now. Claude runs on this computer in both modes.',
};

export const ISOLATION_HINT =
  'Claude sees only the task folder. Its own tools, web search included, can’t go online, so it browses with the app’s browser. Your Claude sign-in or key stays on this computer; the app adds it on the way out.';

/** Whether this host could run Claude in the VM (Linux), whatever Docker is doing. */
export const hostSupportsIsolation = (vm: ClaudeVm | null | undefined) => !!vm && !(vm.toggle.reason && vm.toggle.reason in PLATFORM_HINTS);

// ---- chips ----

export type ChipTone = 'isolated' | 'neutral' | 'preparing' | 'stopped';

export interface Chip {
  engine: AgentEngine;
  name: string;
  tone: ChipTone;
  /** The state after the "·". */
  label: string;
  /** What a screen reader hears, e.g. "Claude: isolated". */
  spoken: string;
  title: string | null;
  /** Preparing with no measured progress: a spinner instead of the download icon. */
  indeterminate?: boolean;
}

const pct = (p: AgentVmState['progress']) => (p ? Math.floor((p.done_bytes / p.total_bytes) * 100) : null);

export function chipFor(engine: AgentEngine, a: AgentVmState | undefined): Chip {
  const name = ENGINE_NAMES[engine];
  const chip = (tone: ChipTone, label: string, title: string | null = null, extra: Partial<Chip> = {}): Chip => ({
    engine,
    name,
    tone,
    label,
    spoken: `${name}: ${label.toLowerCase()}`,
    title,
    ...extra,
  });
  const later = `${name} still runs on this computer, with its own file and shell tools. Moving it inside the VM comes later.`;
  if (engine !== 'claude') return chip('neutral', 'Not isolated yet', later);
  if (!a) return chip('neutral', 'Not isolated');
  switch (a.state) {
    case 'isolated':
      return chip('isolated', 'Isolated', 'Claude runs inside the VM, in a container of its own with no network.');
    case 'preparing': {
      const n = pct(a.progress);
      return n === null
        ? chip('preparing', 'Getting ready', 'Getting the VM ready for Claude.', { indeterminate: true })
        : chip('preparing', `Getting ready · ${n}%`, 'Getting the VM ready for Claude.');
    }
    case 'unavailable':
      return chip('stopped', 'Stopped', "Claude can't reply until this is fixed. Nothing runs on this computer instead.");
    case 'not_isolated':
      return chip('neutral', a.reason === 'not_supported_yet' ? 'Not isolated yet' : 'Not isolated', a.detail);
    default:
      return chip('neutral', 'Not isolated', a.detail);
  }
}

export const chipsFor = (vm: ClaudeVm): Chip[] => AGENT_ENGINES.map((e) => chipFor(e, vm.agents[e]));

// ---- the row ----

export type RowStatus =
  | { kind: 'preparing'; progress: AgentVmState['progress'] }
  | { kind: 'stopped'; text: string; action: 'check' | 'retry' | null }
  | { kind: 'quiet'; text: string }
  | null;

export interface RowModel {
  checked: boolean;
  /** Can't be turned on now. Never set while it's on: the person can always turn it off (fix 9). */
  disabled: boolean;
  /** Replaces the hint on a Mac or Windows host. */
  hint: string;
  status: RowStatus;
}

const sentence = (s: string) => s.trim().replace(/[.\s]+$/, '');

/** The amber line for `unavailable`, by reason (states D, E, E2). */
export function stoppedLine(a: AgentVmState): Extract<RowStatus, { kind: 'stopped' }> {
  switch (a.reason) {
    case 'docker_unavailable':
      return {
        kind: 'stopped',
        text: 'Docker isn’t running, so Claude can’t reply. Start Docker, or turn this off to run Claude on this computer.',
        action: 'check',
      };
    case 'download_failed':
      return {
        kind: 'stopped',
        text: `The setup stopped: ${sentence(a.detail ?? 'the setup failed (the app’s log has the details)')}. Claude can’t reply until it finishes.`,
        action: 'retry',
      };
    case 'download_needed':
      return { kind: 'stopped', text: 'Claude’s VM isn’t set up yet, so Claude can’t reply. Try again to start the one-time setup.', action: 'retry' };
    case 'relay_down':
      return { kind: 'stopped', text: 'The app couldn’t connect Claude to your sign-in. Restart the app; if it keeps happening, turn this off.', action: null };
    case 'sign_in_expired':
      return { kind: 'stopped', text: 'Your Claude sign-in has expired, so Claude can’t reply. Sign in again in Settings › This space › Accounts.', action: null };
    default: {
      const why = a.detail ? sentence(a.detail) : a.reason ? `code ${a.reason}` : null;
      return {
        kind: 'stopped',
        text: `Claude can’t reply right now${why ? ` (${why})` : ''}. Check again, or turn this off to run Claude on this computer.`,
        action: 'check',
      };
    }
  }
}

/** Everything the row shows, from the saved setting (or the one being saved) and the live state. */
export function rowModel(vm: ClaudeVm, pending: boolean | null = null): RowModel {
  const checked = pending ?? vm.on;
  const platform = vm.toggle.reason ? PLATFORM_HINTS[vm.toggle.reason] : undefined;
  // Fix 9: the toggle's `available` ignores the saved setting, so a switch that is on stays usable.
  const disabled = !vm.on && !vm.toggle.available;
  const hint = platform ?? ISOLATION_HINT;
  let status: RowStatus = null;
  const claude = vm.agents.claude;
  if (platform) status = null;
  else if (!vm.on) status = disabled && vm.toggle.reason === 'docker_unavailable' ? { kind: 'quiet', text: 'Start Docker to turn this on.' } : null;
  else if (claude?.state === 'preparing') status = { kind: 'preparing', progress: claude.progress };
  else if (claude?.state === 'unavailable') status = stoppedLine(claude);
  return { checked, disabled, hint, status };
}

/** How often to re-read the setting: every second while preparing, every 10 s while stopped. */
export function pollInterval(vm: ClaudeVm | null | undefined): number | null {
  const states = vm ? Object.values(vm.agents).map((a) => a?.state) : [];
  if (states.includes('preparing')) return 1000;
  if (states.includes('unavailable')) return 10_000;
  return null;
}

/** The progress block's copy. Without measured progress (0.7.7) it's indeterminate (fix 6). */
export function preparingCopy(progress: AgentVmState['progress']) {
  if (!progress)
    return {
      line1: 'Getting the VM ready for Claude. This one-time setup downloads about 860 MB and takes a few minutes.',
      line2: 'Claude can’t reply until it’s done. Turning this off lets Claude reply on this computer meanwhile; the setup keeps going.',
      figure: null,
    };
  const mb = (b: number) => Math.round(b / 1_000_000);
  return {
    line1: 'Getting the VM ready for Claude. This one-time download takes a few minutes.',
    line2: 'Claude can’t reply until it’s done.',
    figure: `${mb(progress.done_bytes)} of ${mb(progress.total_bytes)} MB`,
  };
}

/** One sentence for the live region: only at the start, the end and on a failure, never per tick. */
export function announcement(prev: AgentVmState | undefined, next: AgentVmState | undefined): string | null {
  if (!next || prev?.state === next.state) return null;
  if (next.state === 'preparing') return 'Getting the VM ready for Claude.';
  if (next.state === 'isolated' && prev) return 'Claude now runs inside the VM.';
  if (next.state === 'unavailable' && prev) return `Claude stopped. ${stoppedLine(next).text}`;
  return null;
}

// ---- saving ----

/** The PUT's 400 codes in people's words; older servers send the bare code as `error` (fix 5). */
export const PUT_ERRORS: Record<string, string> = {
  mac_later: PLATFORM_HINTS.mac_later,
  not_supported_yet: PLATFORM_HINTS.not_supported_yet,
  docker_unavailable: 'Start Docker to turn this on.',
};

export function putErrorText(status: number, data: unknown): string {
  const d = data && typeof data === 'object' ? (data as Record<string, unknown>) : {};
  const error = str(d.error);
  const code = str(d.code);
  // A known code reads in our words; then a human `error`; never a bare code.
  if (code && PUT_ERRORS[code]) return PUT_ERRORS[code];
  if (error && PUT_ERRORS[error]) return PUT_ERRORS[error];
  if (error && !/^[a-z_]+$/.test(error)) return error;
  return `Couldn’t change this (HTTP ${status}).`;
}

/** Lin's #446 contract: a reply waiting for a free spot inside the VM. */
export const queuedVmText = (slots?: number): string =>
  `Waiting for a free spot in the VM.${slots ? ` ${slots === 1 ? '1 reply runs' : `${slots} replies run`} at once there.` : ''} This one starts as soon as one finishes.`;
