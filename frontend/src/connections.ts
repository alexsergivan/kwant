// Connections: one list of everything the bots reach outside the app (MCP servers, engine
// accounts, website logins in the vault, the mail and GitHub sources of Loose ends), with status,
// last check, who uses it and where its sign-in is stored. Person-only (docs/connections.md).
//
// Composio is an optional connector, off by default. Wherever it is offered or listed, the page
// says that it keeps sign-ins on Composio's servers.

import { PersonCancelledError, personFetch } from './auth';

export type ConnectionKind = 'mcp' | 'account' | 'browser_login' | 'email' | 'github';
export type ConnectionStatus = 'connected' | 'needs_sign_in' | 'limited' | 'error' | 'off' | 'locked' | 'unknown';
export type LoginsStored = 'vault' | 'kwnt' | 'composio';

export interface UsedBy {
  id: string;
  name: string;
}

export interface Connection {
  id: string;
  kind: ConnectionKind | string;
  name: string;
  detail: string | null;
  status: ConnectionStatus | string;
  status_detail: string | null;
  last_check: string | null;
  used_by: UsedBy[];
  logins_stored: LoginsStored | string;
  manage: string;
}

export interface CatalogEntry {
  id: string;
  kind: string;
  name: string;
  description: string;
  optional: boolean;
  added: boolean;
  connection_id: string | null;
  enabled: boolean;
  logins_stored: string;
  warning: string;
  add: string;
  /** Composio only: kwant has the person's Composio API key, so "Find an app" works. */
  can_search?: boolean;
  /** Composio only: the Composio URL names a user, so kwant can start an app's sign-in. */
  can_connect?: boolean;
  search?: string;
}

/** One app from Composio's catalogue (Composio calls it a toolkit). */
export interface ComposioApp {
  slug: string;
  name: string;
  description: string;
  logo: string | null;
  tools_count: number | null;
  no_auth: boolean;
}

export interface VaultSummary {
  status: 'ok' | 'locked' | 'unavailable' | string;
  logins?: number;
  detail?: string;
}

export interface ConnectionsList {
  connections: Connection[];
  counts: Partial<Record<string, number>>;
  vault: VaultSummary;
  catalog: CatalogEntry[];
}

export class ConnectionsError extends Error {
  constructor(message: string, public status: number) {
    super(message);
  }
}

const sentence = (s: string) => {
  const t = s.trim();
  const up = t.charAt(0).toUpperCase() + t.slice(1);
  return /[.!?]$/.test(up) ? up : `${up}.`;
};

async function call<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new ConnectionsError(e.message, 401);
    throw new ConnectionsError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const answer = await res.json().catch(() => null);
  if (res.ok) return answer as T;
  const err = answer && typeof answer === 'object' ? (answer as Record<string, unknown>).error : null;
  if (res.status === 403) throw new ConnectionsError('Only you can see and change connections.', 403);
  if (res.status === 404 && !err) throw new ConnectionsError('This server doesn’t list connections yet. Update the app and try again.', 404);
  throw new ConnectionsError(typeof err === 'string' && err.trim() ? sentence(err) : `Something went wrong (HTTP ${res.status}).`, res.status);
}

const enc = encodeURIComponent;

export const connectionsApi = {
  list: () => call<ConnectionsList>('/api/connections'),
  addComposio: (body: ComposioBody) => call<{ connection_id: string; warning: string }>('/api/connections/composio', 'POST', body),
  /** Searches Composio's apps with the person's own Composio key (only once Composio is added). */
  searchComposioApps: (q: string) =>
    call<{ apps: ComposioApp[]; can_connect: boolean; warning: string }>(`/api/connections/composio/apps?q=${enc(q)}`),
  /** Starts signing in to one Composio app; the answer is the address to open. */
  connectComposioApp: (slug: string) =>
    call<{ redirect_url: string; warning: string }>(`/api/connections/composio/apps/${enc(slug)}/connect`, 'POST'),
  /**
   * Whether one Composio app is connected for the connector's user. A server without this route (or one
   * that answers something else) is a 404, so callers stop asking instead of polling forever.
   */
  composioAppStatus: async (slug: string): Promise<{ connected: boolean }> => {
    const answer = await call<unknown>(`/api/connections/composio/apps/${enc(slug)}/status`);
    const connected = answer && typeof answer === 'object' ? (answer as { connected?: unknown }).connected : undefined;
    if (typeof connected !== 'boolean') throw new ConnectionsError('This server can’t tell whether an app is connected yet.', 404);
    return { connected };
  },
  /** "Check now" for an MCP server: the server records the result as its last check. */
  testMcp: (serverId: string) => call<{ ok?: boolean; error?: string }>(`/api/mcp/servers/${enc(serverId)}/test`, 'POST'),
};

/** The groups the page shows, in order. */
export const GROUPS: Array<{ key: string; kinds: string[]; label: string; empty: string }> = [
  { key: 'mcp', kinds: ['mcp'], label: 'Tool servers (MCP)', empty: 'No tool servers yet.' },
  { key: 'account', kinds: ['account'], label: 'Engine accounts', empty: 'No engine accounts yet.' },
  { key: 'browser_login', kinds: ['browser_login'], label: 'Saved website logins', empty: 'No saved website logins.' },
  { key: 'sources', kinds: ['email', 'github'], label: 'Mail and GitHub (Loose ends)', empty: 'No mail or GitHub sources.' },
];

export function groupConnections(list: Connection[]): Array<{ key: string; label: string; empty: string; items: Connection[] }> {
  const known = new Set(GROUPS.flatMap((g) => g.kinds));
  const groups = GROUPS.map((g) => ({ key: g.key, label: g.label, empty: g.empty, items: list.filter((c) => g.kinds.includes(c.kind)) }));
  const other = list.filter((c) => !known.has(c.kind));
  if (other.length) groups.push({ key: 'other', label: 'Other', empty: '', items: other });
  return groups;
}

export type Tone = 'emerald' | 'amber' | 'rose' | 'zinc';

/** The badge for a status: always words, never colour alone. */
export function statusBadge(status: string): { label: string; tone: Tone } {
  switch (status) {
    case 'connected':
      return { label: 'Connected', tone: 'emerald' };
    case 'needs_sign_in':
      return { label: 'Needs sign-in', tone: 'amber' };
    case 'limited':
      return { label: 'Limited', tone: 'amber' };
    case 'error':
      return { label: 'Error', tone: 'rose' };
    case 'off':
      return { label: 'Off', tone: 'zinc' };
    case 'locked':
      return { label: 'Locked', tone: 'amber' };
    default:
      return { label: 'Not checked', tone: 'zinc' };
  }
}

/** Statuses that need the person. */
export const needsAttention = (status: string) => status === 'needs_sign_in' || status === 'error' || status === 'limited' || status === 'locked';

/** "4 connected · 1 needs you · 1 off" for the page header. */
export function countsLine(list: Connection[]): string {
  if (list.length === 0) return 'Nothing connected yet';
  const connected = list.filter((c) => c.status === 'connected').length;
  const attention = list.filter((c) => needsAttention(c.status)).length;
  const off = list.filter((c) => c.status === 'off').length;
  const parts = [`${connected} connected`];
  if (attention) parts.push(`${attention} ${attention === 1 ? 'needs' : 'need'} you`);
  if (off) parts.push(`${off} off`);
  return parts.join(' · ');
}

export function storedLabel(stored: string, kind = ''): string {
  if (kind === 'mcp' && stored === 'kwnt') return 'Keys and settings kept on this computer';
  switch (stored) {
    case 'vault':
      return 'Login in your vault';
    case 'kwnt':
      return 'Login kept on this computer';
    case 'composio':
      return 'Login on Composio’s servers';
    default:
      return `Login stored: ${stored}`;
  }
}

/** "Ada, Lin and 3 more", or who uses it when nobody does. */
export function usedByText(used: UsedBy[], status: string): string {
  if (used.length === 0) return status === 'off' ? 'No bot while it’s off' : 'No bot uses it';
  const names = used.map((u) => u.name);
  if (names.length <= 3) return names.length === 1 ? names[0] : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  return `${names.slice(0, 2).join(', ')} and ${names.length - 2} more`;
}

/** The MCP server id inside a connection id (`mcp:<id>`), for "Check now". */
export const mcpServerId = (c: Pick<Connection, 'id' | 'kind'>): string | null =>
  c.kind === 'mcp' && c.id.startsWith('mcp:') ? c.id.slice(4) : null;

export const isComposioUrl = (raw: string): boolean => {
  try {
    const u = new URL(raw.trim());
    const h = u.hostname.toLowerCase();
    return u.protocol === 'https:' && (h === 'composio.dev' || h.endsWith('.composio.dev'));
  } catch {
    return false;
  }
};

export interface ComposioBody {
  url: string;
  api_key?: string;
  enabled: boolean;
}

/** The add form as the API wants it, or why it can't be sent yet. */
export function composioBody(url: string, apiKey: string, enabled: boolean): ComposioBody | { error: string } {
  const u = url.trim();
  if (!u) return { error: 'Paste the MCP URL from your Composio dashboard.' };
  if (!isComposioUrl(u)) return { error: 'That isn’t a Composio address. It should start with https:// and end in composio.dev.' };
  const key = apiKey.trim();
  return key ? { url: u, api_key: key, enabled } : { url: u, enabled };
}

/** What the add form says when the server refuses. */
export function composioErrorText(e: unknown): string {
  if (e instanceof ConnectionsError && e.status === 409) return 'Composio is already added. Change it under Tools instead.';
  if (e instanceof ConnectionsError && e.status === 400) return e.message;
  return e instanceof Error ? e.message : 'Something went wrong.';
}
