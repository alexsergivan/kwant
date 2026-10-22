// The person's approvals switch: "ask" (a card before a bot's tool runs), "smart" (a card only for
// risky calls; the rest run and are recorded) or "off" (it runs and is recorded). Set per space, with a per-bot override. Only the person changes it; a bot's token
// gets 403. No event fires on a change, so callers load bots and spaces again after a save.

import { StoreStatus, autofillGapNote, needsBitwardenFrom } from './passwordStore';

export type SpaceApprovals = 'ask' | 'smart' | 'off';
export type BotApprovals = 'inherit' | SpaceApprovals;

/** GET/PUT /api/kwants/:id/approvals */
export interface BotApprovalsState {
  kwant_id: string;
  approvals: BotApprovals;
  approvals_effective: SpaceApprovals;
  space_approvals: SpaceApprovals;
}

const isMode = (v: unknown): v is SpaceApprovals => v === 'ask' || v === 'smart' || v === 'off';
const spaceMode = (v: unknown): SpaceApprovals => (isMode(v) ? v : 'ask');
const botMode = (v: unknown): BotApprovals => (isMode(v) ? v : 'inherit');

/** True when this bot runs everything without asking. Older servers send nothing: that's "ask". */
export const approvalsOff = (k: { approvals_effective?: string | null } | null | undefined): boolean => k?.approvals_effective === 'off';

/** What a bot's setting comes to: its own choice, else the space's. */
export const effectiveApprovals = (bot: BotApprovals, space: SpaceApprovals): SpaceApprovals => (bot === 'inherit' ? space : bot);

export function parseBotApprovals(raw: unknown): BotApprovalsState | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  if (typeof o.kwant_id !== 'string') return null;
  const approvals = botMode(o.approvals);
  const space = spaceMode(o.space_approvals);
  return {
    kwant_id: o.kwant_id,
    approvals,
    space_approvals: space,
    approvals_effective: isMode(o.approvals_effective) ? o.approvals_effective : effectiveApprovals(approvals, space),
  };
}

export const SPACE_OPTIONS: Array<{ value: SpaceApprovals; label: string; hint: string }> = [
  { value: 'ask', label: 'Ask before running commands', hint: 'A card asks you before a bot runs a command, writes a file or uses a tool.' },
  {
    value: 'smart',
    label: 'Ask only when risky',
    hint: 'Ordinary work in the bot’s folder runs on its own; a card asks before network calls, git fetch and push, publishing, anything outside the folder or anything unclear. Not a sandbox: scripts, make and npm install inside the folder run without a card, and can do anything you can.',
  },
  { value: 'off', label: 'Don’t ask', hint: 'Bots run everything without approval.' },
];

/** The per-bot choices; "Same as space" names what the space is set to. */
export function botOptions(space: SpaceApprovals): Array<{ value: BotApprovals; label: string }> {
  return [
    { value: 'inherit', label: `Same as space (${space === 'off' ? 'Don’t ask' : space === 'smart' ? 'Ask only when risky' : 'Ask'})` },
    { value: 'ask', label: 'Ask before running commands' },
    { value: 'smart', label: 'Ask only when risky' },
    { value: 'off', label: 'Don’t ask' },
  ];
}

/** What turning approvals off means, said plainly on the confirm. */
export const OFF_CONSEQUENCES = [
  'Shell commands run without asking you.',
  'Files can be written or deleted anywhere the computer allows, not just in the bot’s folder.',
  'Tools (MCP servers, browser, web) run without asking.',
  'No approval cards appear. Everything that runs is still recorded.',
];

/** The same, with the saved-passwords sentence for where passwords are kept. */
export const offConsequences = (store: StoreStatus | null | undefined): string[] => [...OFF_CONSEQUENCES, autofillGapNote(store)];

/** The server refused "off" because passwords are still saved on it (audit F-10). */
export class NeedsBitwardenError extends Error {
  constructor(message: string, public localCount: number | null) {
    super(message);
  }
}

/** Fired after approvals change outside a settings screen, so lists and markers load again. */
export const APPROVALS_CHANGED_EVENT = 'kwnt:approvals-changed';

export function offConfirmTitle(who: { space: string } | { bot: string }): string {
  return 'space' in who ? `Turn off approvals for every bot in ${who.space}?` : `Let ${who.bot} run everything without approval?`;
}

/** A line for the space header: null when nothing runs without asking. */
export function spaceOffLine(space: SpaceApprovals | undefined, bots: Array<{ approvals_effective?: string | null }>): string | null {
  if (space === 'off') {
    const asking = bots.filter((b) => b.approvals_effective === 'ask').length;
    return asking ? `Approvals off (${asking} ${asking === 1 ? 'bot still asks' : 'bots still ask'})` : 'Approvals off';
  }
  const off = bots.filter(approvalsOff).length;
  return off ? `Approvals off for ${off} ${off === 1 ? 'bot' : 'bots'}` : null;
}

async function errorText(res: Response, fallback: string): Promise<string | NeedsBitwardenError> {
  try {
    const body = await res.json();
    const needs = needsBitwardenFrom(res.status, body);
    if (needs) return new NeedsBitwardenError(needs.message, needs.local_count);
    if (typeof body?.error === 'string' && body.error) {
      if (res.status === 403) return 'Only you can change approvals, from the app.';
      if (res.status === 409) return 'This space is archived. Restore it to change its approvals.';
      return `${fallback} (${body.error}).`;
    }
  } catch {
    /* not JSON */
  }
  if (res.status === 404 || res.status === 405) return 'This server can’t change approvals yet. Update the app and try again.';
  return `${fallback} (HTTP ${res.status}).`;
}

async function put(url: string, approvals: string, fallback: string): Promise<unknown> {
  let res: Response;
  try {
    res = await fetch(url, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ approvals }) });
  } catch {
    throw new Error('Couldn’t reach the server. Check your connection and try again.');
  }
  if (!res.ok) {
    const e = await errorText(res, fallback);
    throw e instanceof Error ? e : new Error(e);
  }
  return res.json();
}

const enc = encodeURIComponent;

export const approvalModeApi = {
  /** Resolves to the space's new value. */
  setSpace: async (spaceId: string, approvals: SpaceApprovals): Promise<SpaceApprovals> => {
    const body = (await put(`/api/spaces/${enc(spaceId)}/approvals`, approvals, 'Couldn’t change approvals')) as { approvals?: unknown };
    return spaceMode(body?.approvals);
  },
  /** null when the server has no such endpoint or the call fails. */
  bot: async (kwantId: string): Promise<BotApprovalsState | null> => {
    try {
      const res = await fetch(`/api/kwants/${enc(kwantId)}/approvals`, { cache: 'no-store' });
      return res.ok ? parseBotApprovals(await res.json()) : null;
    } catch {
      return null;
    }
  },
  setBot: async (kwantId: string, approvals: BotApprovals, spaceId?: string): Promise<BotApprovalsState> => {
    const url = `/api/kwants/${enc(kwantId)}/approvals${spaceId ? `?space=${enc(spaceId)}` : ''}`;
    const state = parseBotApprovals(await put(url, approvals, 'Couldn’t change this bot’s approvals'));
    if (!state) throw new Error('The server sent an answer the app can’t read. Reload and check the setting.');
    return state;
  },
  /**
   * "Turn approvals back on" everywhere: every active space that's off or "Ask only when risky",
   * and every bot set to either, goes back to ask (both keep passwords off this server). Archived spaces can't be changed until they're restored; they count as failed.
   */
  turnAllOn: async (): Promise<{ changed: number; failed: number }> => {
    let changed = 0;
    let failed = 0;
    const list = async (url: string): Promise<Array<Record<string, unknown>>> => {
      const res = await fetch(url, { cache: 'no-store' });
      if (!res.ok) throw new Error('Couldn’t load your spaces. Try again.');
      const body = await res.json();
      return Array.isArray(body) ? body : [];
    };
    // `/api/spaces` lists the active spaces only; `?archived=1` lists only the archived ones.
    let spaces: Array<Record<string, unknown>>;
    try {
      const [active, archived] = await Promise.all([list('/api/spaces'), list('/api/spaces?archived=1').catch(() => [])]);
      spaces = [...active, ...archived];
    } catch {
      throw new Error('Couldn’t reach the server. Check your connection and try again.');
    }
    const attempt = async (f: () => Promise<unknown>) => {
      try {
        await f();
        changed++;
      } catch {
        failed++;
      }
    };
    for (const sp of spaces) {
      const id = typeof sp.id === 'string' ? sp.id : null;
      if (!id) continue;
      if (sp.approvals === 'off' || sp.approvals === 'smart') {
        if (sp.archived_at) failed++;
        else await attempt(() => approvalModeApi.setSpace(id, 'ask'));
      }
      if (sp.archived_at) continue;
      const bots = await list(`/api/kwants?space=${enc(id)}`).catch(() => null);
      if (!bots) {
        failed++;
        continue;
      }
      for (const b of bots) {
        if ((b.approvals === 'off' || b.approvals === 'smart') && typeof b.id === 'string') await attempt(() => approvalModeApi.setBot(b.id as string, 'ask', id));
      }
    }
    if (changed) window.dispatchEvent(new Event(APPROVALS_CHANGED_EVENT));
    return { changed, failed };
  },
};
