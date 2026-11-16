// Connect Home Assistant (26e): the pure parts of the sheet. The person gives an address and an
// access token; kwant builds the MCP URL and the Bearer header, saves the server, tests it, and
// reads the result in plain words. design: autonomy/connect-home-assistant/design.md §1.4–1.5.

import type { McpServer, McpTestResult, StoredStatus } from './mcp';

/** Fired after Connect Home Assistant saved or tested a server, so the needs row re-reads. */
export const HA_CHANGED_EVENT = 'kwant:home-assistant-changed';

/** The server name kwant gives Home Assistant in Tools (server names can't have spaces). */
export const HA_SERVER_NAME = 'home-assistant';

/** What Home Assistant's MCP Server calls itself in `initialize` (HA's mcp_server/server.py). */
const HA_SERVER_INFO = 'home-assistant';

export interface HaAddress {
  /** The MCP endpoint kwant connects to: `<origin>/api/mcp`. */
  url: string;
  /** `scheme://host[:port]`, for the "Make one in Home Assistant" and integrations links. */
  origin: string;
  /** The host alone, for "kwant can't reach <host>". */
  host: string;
}

/**
 * The address as typed ("homeassistant.local:8123", "192.168.1.20", "https://x.ui.nabu.casa/")
 * → the MCP URL. No scheme means http; plain http with no port means HA's 8123; https keeps its
 * own port. Any path is replaced by /api/mcp. Null when it isn't an http(s) address.
 */
export function normaliseHaAddress(raw: string): HaAddress | null {
  const typed = raw.trim();
  if (!typed || /\s/.test(typed)) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(typed) ? typed : `http://${typed}`;
  let u: URL;
  try {
    u = new URL(withScheme);
  } catch {
    return null;
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
  if (!u.hostname || u.username || u.password) return null;
  // A bare word like "foo" parses as a host; a real address has a dot, a colon (IPv6) or is localhost.
  if (!/[.:]/.test(u.hostname) && u.hostname !== 'localhost' && !u.port) return null;
  const port = u.port || (u.protocol === 'http:' ? '8123' : '');
  const origin = `${u.protocol}//${u.hostname}${port ? `:${port}` : ''}`;
  return { url: `${origin}/api/mcp`, origin, host: port ? `${u.hostname}:${port}` : u.hostname };
}

/** What the result box shows. `ok` carries the number of actions kwant found. */
export type HaResult =
  | { kind: 'ok'; actions: number }
  | { kind: 'bad_token' }
  | { kind: 'mcp_off' }
  | { kind: 'unreachable' }
  | { kind: 'not_ha'; name: string }
  | { kind: 'other'; error: string };

/** The test result → one state, first match wins (design §1.5). */
export function mapHaTest(t: McpTestResult): HaResult {
  const name = t.server_info?.name;
  if (t.ok && name && name !== HA_SERVER_INFO) return { kind: 'not_ha', name };
  if (t.ok) return { kind: 'ok', actions: t.tools?.length ?? 0 };
  const err = t.error ?? '';
  if (t.auth_required || /HTTP 40[13]\b/.test(err)) return { kind: 'bad_token' };
  if (/HTTP 404\b/.test(err)) return { kind: 'mcp_off' };
  if (/error sending request|connect|timed out|dns|resolve|refused|unreachable/i.test(err)) return { kind: 'unreachable' };
  return { kind: 'other', error: err.trim() || 'Home Assistant didn’t answer the check.' };
}

/** Failed titles, shared by the sheet and the Home team's needs row. */
export function haResultTitle(r: HaResult, host = 'Home Assistant'): string {
  switch (r.kind) {
    case 'ok':
      return `Connected · ${actionsText(r.actions)}`;
    case 'bad_token':
      return 'Home Assistant didn’t accept this access token.';
    case 'mcp_off':
      return 'Home Assistant answered, but its MCP Server is off.';
    case 'unreachable':
      return `kwant can’t reach ${host}.`;
    case 'not_ha':
      return 'This address answered, but it isn’t Home Assistant.';
    case 'other':
      return 'Couldn’t connect to Home Assistant.';
  }
}

export const actionsText = (n: number) => `${n} ${n === 1 ? 'action' : 'actions'}`;

/** The needs row's failed line: what happened, then the next step. */
export function haNeedsFailText(r: HaResult): string {
  switch (r.kind) {
    case 'bad_token':
      return 'Home Assistant didn’t accept the access token. Make a new one and paste it.';
    case 'mcp_off':
      return 'Home Assistant’s MCP Server is off. Add it in Home Assistant, then try again.';
    case 'unreachable':
      return 'kwant can’t reach Home Assistant. Check it’s on and the address is right.';
    default:
      return 'Couldn’t connect to Home Assistant. Open it to see why.';
  }
}

/** The Home Assistant server in Tools: kwant's own name, or any URL server ending in /api/mcp. */
export function findHaServer<T extends Pick<McpServer, 'id' | 'name' | 'url' | 'transport'>>(servers: T[]): T | null {
  const isHaUrl = (s: Pick<McpServer, 'url' | 'transport'>) => s.transport === 'http' && /\/api\/mcp\/?$/.test(s.url ?? '');
  return (
    servers.find((s) => s.name === HA_SERVER_NAME && isHaUrl(s)) ??
    servers.find((s) => /^home[-_ ]?assistant/i.test(s.name) && isHaUrl(s)) ??
    null
  );
}

/** Whether a server is one this sheet should open for (Fix on a /api/mcp server). */
export const isHaServer = (s: Pick<McpServer, 'url' | 'transport' | 'name'>) => findHaServer([{ id: '', ...s }]) !== null;

/** A free server name: home-assistant, else home-assistant-2, -3… */
export function freeHaName(taken: string[]): string {
  const set = new Set(taken.map((n) => n.toLowerCase()));
  if (!set.has(HA_SERVER_NAME)) return HA_SERVER_NAME;
  for (let i = 2; i < 100; i++) if (!set.has(`${HA_SERVER_NAME}-${i}`)) return `${HA_SERVER_NAME}-${i}`;
  return `${HA_SERVER_NAME}-${Date.now() % 10000}`;
}

/** The Home team needs row from the servers list and the last stored test. */
export type HaNeedState = { state: 'none' | 'checking' | 'ok' | 'failed'; detail?: string | null; actions?: number };

export function haNeedState(
  servers: Pick<McpServer, 'id' | 'name' | 'url' | 'transport' | 'enabled'>[] | null,
  statuses: Record<string, Pick<StoredStatus, 'ok' | 'tools' | 'auth_required'>>,
): HaNeedState {
  if (servers === null) return { state: 'checking' };
  const s = findHaServer(servers);
  if (!s) return { state: 'none' };
  const st = statuses[s.id];
  if (!st) return { state: 'none' };
  if (st.ok) return { state: 'ok', actions: st.tools.length, detail: actionsText(st.tools.length) };
  return { state: 'failed', detail: haNeedsFailText(st.auth_required ? { kind: 'bad_token' } : { kind: 'other', error: '' }) };
}
