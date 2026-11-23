// 34i: the "suggests a tool" card shows the server's real state from /api/connections, so a reload,
// another device or a delete never leaves a false "Ready". The in-memory flag from Install (or a
// sign-in from the card) is only the optimistic value until a list fetched after it answers.

import { type Suggestion, parseSuggestionMeta, suggestionView } from './mcpSuggestions';

/** What an installed suggestion's card says about its server. */
export type SuggestionServerState = 'ready' | 'needs_sign_in' | 'removed';

/** One answer of the connections list: `states` by MCP server id, and the fetch it came from. */
export interface ServerStates {
  states: Record<string, 'ready' | 'needs_sign_in'>;
  /** Sequence number taken when the fetch started; newer local knowledge wins over this answer. */
  seq: number;
}

/** MCP rows of the connections list (`mcp:<server_id>`) to `ready` / `needs_sign_in`, by server id. */
export function mcpServerStates(connections: unknown): Record<string, 'ready' | 'needs_sign_in'> {
  const out: Record<string, 'ready' | 'needs_sign_in'> = {};
  if (!Array.isArray(connections)) return out;
  for (const c of connections) {
    if (!c || typeof c !== 'object') continue;
    const { id, status } = c as { id?: unknown; status?: unknown };
    if (typeof id !== 'string' || !id.startsWith('mcp:') || id.length <= 4) continue;
    out[id.slice(4)] = status === 'needs_sign_in' ? 'needs_sign_in' : 'ready';
  }
  return out;
}

/**
 * The card's state for one server. `listed` is the last list answer (null before one arrived or when
 * the list call failed); `local` is what Install or a card sign-in said this session (undefined when
 * nothing did) and `localSeq` when it was said. Without a usable list answer the card keeps today's
 * behaviour, so an error never shows "Removed".
 */
export function suggestionServerState(
  serverId: string,
  listed: ServerStates | null,
  local: boolean | undefined,
  localSeq: number,
): SuggestionServerState {
  const optimistic: SuggestionServerState = local ? 'needs_sign_in' : 'ready';
  if (!listed) return optimistic;
  if (local !== undefined && localSeq > listed.seq) return optimistic;
  return listed.states[serverId] ?? 'removed';
}

/**
 * Reads the connections list without the person-token prompt `connectionsApi.list()` can open: the
 * card asks on its own when a chat opens, so a viewer without the token just keeps today's card.
 */
export async function fetchServerStates(seq: number, send: typeof fetch = (...a) => fetch(...a)): Promise<ServerStates> {
  const res = await send('/api/connections', { cache: 'no-store' });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const body = (await res.json()) as { connections?: unknown } | null;
  if (!body || !Array.isArray(body.connections)) throw new Error('No connections in the answer.');
  return { states: mcpServerStates(body.connections), seq };
}

/**
 * Whether a chat's messages hold an installed tool suggestion (with its server id), so the card needs
 * the list. Message meta arrives as an object or as JSON text; other messages are skipped cheaply.
 */
export function chatHasInstalledServer(messages: ReadonlyArray<{ meta?: unknown }>, live: Record<string, Suggestion>): boolean {
  return messages.some((m) => {
    const raw = m.meta;
    if (!raw) return false;
    if (typeof raw === 'string' ? !raw.includes('mcp_suggestion') : (raw as { kind?: unknown }).kind !== 'mcp_suggestion') return false;
    const meta = parseSuggestionMeta(raw);
    if (!meta) return false;
    const v = suggestionView(meta, live[meta.suggestion_id]);
    return v.status === 'installed' && !!v.server_id;
  });
}
