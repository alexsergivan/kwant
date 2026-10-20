// The person's approvals switch: "ask" (a card before a bot's tool runs) or "off" (it runs and is
// recorded). Set per space, with a per-bot override. Only the person changes it; a bot's token
// gets 403. No event fires on a change, so callers load bots and spaces again after a save.

export type SpaceApprovals = 'ask' | 'off';
export type BotApprovals = 'inherit' | 'ask' | 'off';

/** GET/PUT /api/kwants/:id/approvals */
export interface BotApprovalsState {
  kwant_id: string;
  approvals: BotApprovals;
  approvals_effective: SpaceApprovals;
  space_approvals: SpaceApprovals;
}

const spaceMode = (v: unknown): SpaceApprovals => (v === 'off' ? 'off' : 'ask');
const botMode = (v: unknown): BotApprovals => (v === 'off' || v === 'ask' ? v : 'inherit');

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
    approvals_effective: o.approvals_effective === 'off' || o.approvals_effective === 'ask' ? o.approvals_effective : effectiveApprovals(approvals, space),
  };
}

export const SPACE_OPTIONS: Array<{ value: SpaceApprovals; label: string; hint: string }> = [
  { value: 'ask', label: 'Ask before running commands', hint: 'A card asks you before a bot runs a command, writes a file or uses a tool.' },
  { value: 'off', label: 'Don’t ask', hint: 'Bots run everything without approval.' },
];

/** The per-bot choices; "Same as space" names what the space is set to. */
export function botOptions(space: SpaceApprovals): Array<{ value: BotApprovals; label: string }> {
  return [
    { value: 'inherit', label: `Same as space (${space === 'off' ? 'Don’t ask' : 'Ask'})` },
    { value: 'ask', label: 'Ask before running commands' },
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

async function errorText(res: Response, fallback: string): Promise<string> {
  try {
    const body = await res.json();
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
  if (!res.ok) throw new Error(await errorText(res, fallback));
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
  setBot: async (kwantId: string, approvals: BotApprovals): Promise<BotApprovalsState> => {
    const state = parseBotApprovals(await put(`/api/kwants/${enc(kwantId)}/approvals`, approvals, 'Couldn’t change this bot’s approvals'));
    if (!state) throw new Error('The server sent an answer the app can’t read. Reload and check the setting.');
    return state;
  },
};
