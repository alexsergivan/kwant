// Deleting a bot: what it takes with it, and the call. The person starts it from the bot's
// settings; a bot can only propose it (bot_delete action card).

import { PersonCancelledError, personFetch } from './auth';

/** What deleting a bot removes, from GET /api/kwants/:id/usage; null when the server can't say. */
export interface BotUsage {
  conversations: number;
  /** Messages in its direct chats and threads; team-chat messages stay and aren't counted. */
  messages: number;
  memories: number;
  skills: number;
  routines: number;
  /** Teams and channels it leads; each passes to its next member. */
  leads: number;
  running: boolean;
}

const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? Math.floor(v) : 0);

export function parseUsage(raw: unknown): BotUsage | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  return {
    conversations: n(o.conversations),
    messages: n(o.messages),
    memories: n(o.memories),
    skills: n(o.skills),
    routines: n(o.routines),
    leads: n(o.leads),
    running: o.running === true,
  };
}

const count = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;

/** The lines the confirm dialog lists: counts when known, else the kinds of thing. */
export function lossLines(usage: BotUsage | null): string[] {
  if (!usage) return ['Its conversations and threads', 'Its memory', 'Its skills', 'Its routines'];
  return [
    usage.messages
      ? `${count(usage.conversations, 'conversation', 'conversations')} (${count(usage.messages, 'message', 'messages')})`
      : count(usage.conversations, 'conversation', 'conversations'),
    count(usage.memories, 'memory', 'memories'),
    count(usage.skills, 'skill', 'skills'),
    count(usage.routines, 'routine', 'routines'),
  ];
}

/** A note for teams and channels it leads, or null. */
export function leadsNote(usage: BotUsage | null): string | null {
  if (!usage?.leads) return null;
  return `It leads ${count(usage.leads, 'team or channel', 'teams or channels')}; the next member takes over as lead.`;
}

/** Why Delete is off right now, or null when it can be pressed. `busy` is the sidebar's activity line. */
export function deleteBlockedReason(name: string, busy: string | null | undefined, usage: BotUsage | null): string | null {
  if (busy || usage?.running) return `${name} is working right now. Stop it or wait until it finishes, then delete it.`;
  return null;
}

const enc = encodeURIComponent;

export const botDeleteApi = {
  /** null when the server has no usage endpoint or the call fails; the dialog then words it generally. */
  usage: async (id: string): Promise<BotUsage | null> => {
    try {
      const res = await fetch(`/api/kwants/${enc(id)}/usage`);
      return res.ok ? parseUsage(await res.json()) : null;
    } catch {
      return null;
    }
  },
  /** Resolves to an error sentence, or null once it's deleted (or already gone). */
  remove: async (id: string, name: string): Promise<string | null> => {
    let res: Response;
    try {
      res = await personFetch(`/api/kwants/${enc(id)}`, { method: 'DELETE' });
    } catch (e) {
      if (e instanceof PersonCancelledError) return e.message;
      return 'Couldn’t reach the server. Check your connection and try again.';
    }
    if (res.ok || res.status === 404) return null;
    let msg = '';
    let code = '';
    try {
      const body = await res.json();
      code = typeof body?.error === 'string' ? body.error : '';
      msg = typeof body?.message === 'string' ? body.message : typeof body?.error === 'string' && body.error.includes(' ') ? body.error : '';
    } catch {
      /* not JSON */
    }
    if (msg) return msg;
    if (code === 'last_bot') return `${name} is the only bot in this space, and a space needs at least one. Add another bot first.`;
    if (res.status === 409) return `${name} is working right now. Stop it or wait until it finishes, then delete it.`;
    if (res.status === 405) return 'This server can’t delete bots yet. Update the app and try again.';
    return `Couldn’t delete ${name} (HTTP ${res.status}).`;
  },
};

// #1301: deleting a bot blanked the page. /api/teams sends each team's `member_kwants` as a JSON string;
// the delete handler stored it raw, so the next render's devTeamRefs called `.filter` on a string and
// the whole app unmounted. Every place that loads teams goes through normalizeTeams now, and the view
// leaves a deleted bot's chat even when the space has no channels.

/** One team's `member_kwants` as ids, from the server's JSON string or an array. Junk gives []. */
export function memberIdsOfTeam(raw: unknown): string[] {
  let list: unknown = raw;
  if (typeof raw === 'string') {
    try {
      list = JSON.parse(raw || '[]');
    } catch {
      return [];
    }
  }
  return Array.isArray(list) ? list.filter((m): m is string => typeof m === 'string') : [];
}

/** Teams as the app keeps them (member ids as an array), from an /api/teams answer. Not a list gives []. */
export function normalizeTeams<T extends { member_kwants?: unknown }>(raw: unknown): Array<Omit<T, 'member_kwants'> & { member_kwants: string[] }> {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((t): t is T => !!t && typeof t === 'object')
    .map((t) => ({ ...t, member_kwants: memberIdsOfTeam(t.member_kwants) }));
}

/**
 * Channels as the app keeps them, from an /api/channels answer. The server sends `member_kwants` as a JSON
 * string there too, and App spread it into characters, so a non-lead member's approval missed its channel.
 */
export function normalizeChannels<T extends { member_kwants?: unknown }>(raw: unknown): Array<Omit<T, 'member_kwants'> & { member_kwants: string[] }> {
  return normalizeTeams<T>(raw);
}

/** The teams right after a bot is deleted, before the server's lists come back: it's no longer a member. */
export function dropBotFromTeams<T extends { member_kwants: string[] }>(teams: T[], botId: string): T[] {
  return teams.map((t) => (t.member_kwants.includes(botId) ? { ...t, member_kwants: t.member_kwants.filter((m) => m !== botId) } : t));
}

export type AfterBotDelete<C, K> = { kind: 'stay' } | { kind: 'channel'; channel: C } | { kind: 'bot'; bot: K } | { kind: 'none' };

/**
 * Where the view goes when a bot is deleted. A channel or team chat stays open (the bot's messages
 * there stay); the deleted bot's own chat moves to #general, else the first channel, else the first
 * bot left, else nothing.
 */
export function chatAfterBotDelete<C extends { id: string; name: string }, K extends { id: string }>(o: {
  deletedId: string;
  selectedId: string | null;
  isChannel: boolean;
  channels: C[];
  botsLeft: K[];
  generalChannelId?: string | null;
}): AfterBotDelete<C, K> {
  if (o.isChannel || o.selectedId !== o.deletedId) return { kind: 'stay' };
  const channel = o.channels.find((c) => c.id === o.generalChannelId) ?? o.channels.find((c) => c.name === 'general') ?? o.channels[0];
  if (channel) return { kind: 'channel', channel };
  const bot = o.botsLeft.find((k) => k.id !== o.deletedId);
  return bot ? { kind: 'bot', bot } : { kind: 'none' };
}
