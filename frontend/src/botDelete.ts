// Deleting a bot: what it takes with it, and the call. The person starts it from the bot's
// settings; a bot can only propose it (bot_delete action card).

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
      res = await fetch(`/api/kwants/${enc(id)}`, { method: 'DELETE' });
    } catch {
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
