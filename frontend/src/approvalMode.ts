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
  { value: 'off', label: 'Don’t ask', hint: 'Bots act without asking, including sending, deleting, publishing and signing in. Payments still ask you.' },
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

/** What turning approvals off means, said plainly on the confirm (Oleks, 2026-10-27: off means off, with the risks said first). */
export const OFF_CONSEQUENCES = [
  'Bots can send messages and emails, delete files or data, publish, and sign in to your accounts without asking you.',
  'Shell commands, file writes anywhere the computer allows, and tools (MCP servers, browser, web) run without a card.',
  'Payments still ask you first, though a payment made by filling in a card form on a website may not be caught.',
  'Everything that runs is still recorded.',
];

/** Under the list: who shouldn't be on "Don't ask". */
export const OFF_ADVICE = 'For bots with access to money or your accounts, keep approvals on or choose “Ask only when risky”.';

/** Next to the setting for as long as it's off. */
export const OFF_HINT = 'Approvals are off: bots act without asking, including deletions, publishing and sign-ins. Payments still ask you.';

/** True when a change newly turns approvals off, so the confirm comes before the save. Takes effective values. */
export const needsOffConfirm = (current: SpaceApprovals, next: SpaceApprovals): boolean => next === 'off' && current !== 'off';

/** The same, with the saved-passwords sentence for where passwords are kept. */
export const offConsequences = (store: StoreStatus | null | undefined): string[] => [...OFF_CONSEQUENCES, autofillGapNote(store)];

/** The server refused "off" because passwords are still saved on it (audit F-10). */
export class NeedsBitwardenError extends Error {
  constructor(message: string, public localCount: number | null, public canAccept = false) {
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

/** Explainer spec §2, Approvals row: the chip, its tone and the one line under it (or null). */
export interface ApprovalsRow {
  value: SpaceApprovals;
  chip: 'Ask' | 'When risky' | 'Off';
  tone: 'neutral' | 'warn';
  sub: string | null;
  /** The value button's name: "Approvals: Off. Change in space settings". */
  label: string;
  /** Desktop tooltip, only when approvals are off. */
  title: string | null;
}

const CHIP: Record<SpaceApprovals, ApprovalsRow['chip']> = { ask: 'Ask', smart: 'When risky', off: 'Off' };
export const APPROVALS_OFF_TITLE = 'Bots here act without asking, including deletions, publishing and sign-ins. Payments still ask you. Change it in Settings › This space.';
export const APPROVALS_OFF_TITLE_DEV = ' With Dev mode on, pushes to main, deploys and deleting data still ask.';

/** null on an older server that doesn't say the space's approvals: the row isn't drawn rather than guessed. */
export function approvalsRow(space: SpaceApprovals | undefined, bots: Array<{ approvals_effective?: string | null }>, devOn: boolean): ApprovalsRow | null {
  if (space !== 'ask' && space !== 'smart' && space !== 'off') return null;
  const chip = CHIP[space];
  const label = `Approvals: ${chip}. Change in space settings`;
  if (space === 'off') {
    const asking = bots.filter((b) => b.approvals_effective === 'ask' || b.approvals_effective === 'smart').length;
    const sub = asking ? (asking === 1 ? '1 bot still asks' : `${asking} bots still ask`) : devOn ? 'Main and deploys still ask' : 'Bots act without asking';
    return { value: space, chip, tone: 'warn', sub, label, title: APPROVALS_OFF_TITLE + (devOn ? APPROVALS_OFF_TITLE_DEV : '') };
  }
  const off = bots.filter(approvalsOff).length;
  return { value: space, chip, tone: 'neutral', sub: off ? `Off for ${off} ${off === 1 ? 'bot' : 'bots'}` : null, label, title: null };
}

async function errorText(res: Response, fallback: string): Promise<string | NeedsBitwardenError> {
  try {
    const body = await res.json();
    const needs = needsBitwardenFrom(res.status, body);
    if (needs) return new NeedsBitwardenError(needs.message, needs.local_count, needs.can_accept);
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

/** `acceptRisk` (0.7.3 A2): the person confirmed keeping passwords on this server; sent once, person-only on the server. */
async function put(url: string, approvals: string, fallback: string, acceptRisk = false): Promise<unknown> {
  let res: Response;
  try {
    const body = acceptRisk ? { approvals, accept_risk: true } : { approvals };
    res = await fetch(url, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
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
  setSpace: async (spaceId: string, approvals: SpaceApprovals, acceptRisk = false): Promise<SpaceApprovals> => {
    const body = (await put(`/api/spaces/${enc(spaceId)}/approvals`, approvals, 'Couldn’t change approvals', acceptRisk)) as { approvals?: unknown };
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
  setBot: async (kwantId: string, approvals: BotApprovals, spaceId?: string, acceptRisk = false): Promise<BotApprovalsState> => {
    const url = `/api/kwants/${enc(kwantId)}/approvals${spaceId ? `?space=${enc(spaceId)}` : ''}`;
    const state = parseBotApprovals(await put(url, approvals, 'Couldn’t change this bot’s approvals', acceptRisk));
    if (!state) throw new Error('The server sent an answer the app can’t read. Reload and check the setting.');
    return state;
  },
  /**
   * "Turn approvals back on" everywhere: every active space that's off or "Ask only when risky",
   * and every bot set to either, goes back to ask (both keep passwords off this server). Archived spaces are
   * skipped: they can't run anything and don't count (0.7.3 A1).
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
    // `/api/spaces` lists the active spaces only.
    let spaces: Array<Record<string, unknown>>;
    try {
      spaces = (await list('/api/spaces')).filter((sp) => !sp.archived_at);
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
      if (sp.approvals === 'off' || sp.approvals === 'smart') await attempt(() => approvalModeApi.setSpace(id, 'ask'));
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
