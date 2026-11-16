// A bot's "Always allow" answers (25e): which tool runs without a card in which conversation,
// kept across restarts, and taking one or all of them back. Lin's person-only routes:
// GET and DELETE /api/kwants/:id/always-allowed.
import { toolAction } from './toolLabel';

export type ConversationKind = 'thread' | 'channel' | 'direct' | 'unknown';

export interface AlwaysAllowed {
  tool_name: string;
  conversation: string;
  conversation_kind: ConversationKind;
  conversation_label: string;
  created_at: string;
}

const KINDS: ConversationKind[] = ['thread', 'channel', 'direct', 'unknown'];

/** The server's rows, skipping anything it can't name; null when it isn't a list. */
export function parseAlwaysAllowed(body: unknown): AlwaysAllowed[] | null {
  if (!Array.isArray(body)) return null;
  const out: AlwaysAllowed[] = [];
  for (const r of body) {
    if (!r || typeof r !== 'object') continue;
    const x = r as Record<string, unknown>;
    if (typeof x.tool_name !== 'string' || typeof x.conversation !== 'string') continue;
    const kind = KINDS.includes(x.conversation_kind as ConversationKind) ? (x.conversation_kind as ConversationKind) : 'unknown';
    out.push({
      tool_name: x.tool_name,
      conversation: x.conversation,
      conversation_kind: kind,
      conversation_label: typeof x.conversation_label === 'string' ? x.conversation_label : '',
      created_at: typeof x.created_at === 'string' ? x.created_at : '',
    });
  }
  return out;
}

const cap = (t: string) => (t ? t.charAt(0).toUpperCase() + t.slice(1) : t);

/** Built-in file tools, written the way the card says them. */
const BUILTIN: Record<string, string> = {
  write: 'write a file',
  edit: 'change a file',
  multiedit: 'change a file',
  notebookedit: 'change a notebook',
};

/** The tool as the card names it: "Open a pull request on GitHub", "Run a command". */
export const grantTool = (name: string): string => cap(BUILTIN[name.toLowerCase()] ?? toolAction(name));

/** Where it applies: "in Weekly report", "in #general", "in your chat". */
export function grantWhere(g: AlwaysAllowed): string {
  switch (g.conversation_kind) {
    case 'direct':
      return 'in your chat';
    case 'channel':
      return `in ${g.conversation_label.startsWith('#') ? g.conversation_label : `#${g.conversation_label}`}`;
    case 'thread':
      return g.conversation_label.trim() ? `in ${g.conversation_label.trim()}` : 'in a thread';
    default:
      return 'in a conversation that’s gone';
  }
}

const url = (kwantId: string) => `/api/kwants/${encodeURIComponent(kwantId)}/always-allowed`;

async function errorText(res: Response, fallback: string): Promise<string> {
  try {
    const b = (await res.json()) as { error?: unknown };
    if (typeof b?.error === 'string' && b.error) return cap(b.error.endsWith('.') ? b.error : `${b.error}.`);
  } catch {
    /* not JSON */
  }
  return fallback;
}

export const alwaysAllowedApi = {
  /** null when this server has no list (an older kwnt): show nothing. Throws when it can't be read. */
  list: async (kwantId: string): Promise<AlwaysAllowed[] | null> => {
    let res: Response;
    try {
      res = await fetch(url(kwantId), { cache: 'no-store' });
    } catch {
      throw new Error('Couldn’t reach the server. Check your connection and try again.');
    }
    if (res.status === 404 || res.status === 405) return null;
    if (!res.ok) throw new Error(await errorText(res, 'Couldn’t load what this bot does without asking.'));
    const rows = parseAlwaysAllowed(await res.json().catch(() => null));
    if (!rows) throw new Error('The server sent an answer the app can’t read. Reload and try again.');
    return rows;
  },
  /** Takes back one grant, or all of them without one; resolves to what is left. */
  revoke: async (kwantId: string, one?: Pick<AlwaysAllowed, 'conversation' | 'tool_name'>): Promise<AlwaysAllowed[]> => {
    let res: Response;
    try {
      res = await fetch(url(kwantId), {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(one ? { conversation: one.conversation, tool_name: one.tool_name } : {}),
      });
    } catch {
      throw new Error('Couldn’t reach the server, so nothing was taken back. Try again.');
    }
    if (!res.ok) throw new Error(await errorText(res, 'Couldn’t take that back. Try again.'));
    const body = (await res.json().catch(() => null)) as { always_allowed?: unknown } | null;
    return parseAlwaysAllowed(body?.always_allowed) ?? [];
  },
};
