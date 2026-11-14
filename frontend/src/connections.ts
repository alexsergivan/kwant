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
  /** When an account that ran out of quota can run again (RFC 3339); only on limited accounts. */
  limited_until?: string | null;
  last_check: string | null;
  used_by: UsedBy[];
  logins_stored: LoginsStored | string;
  manage: string;
  /** Dev mode D10: marked as a production connection in this space (0.7 servers). */
  production?: boolean;
  /** 0.9.0 P5: open bot notes about this MCP server's tools; `broke` of them are "Didn't work". MCP rows only. */
  notes?: { open: number; broke: number };
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
  /** Composio only (#548): `session` (an ak_ key), `connect` (a ck_ key), `url` (a pasted link), null when not added. */
  mode?: 'session' | 'connect' | 'url' | null;
  user_id?: string | null;
  apps_connected?: Array<{ slug: string; name: string }>;
  /** What's wrong with the setup and the one action that fixes it. */
  problem?: { message: string; action: ComposioAction } | null;
  can_switch?: boolean;
  /** Where a person copies a Composio key: open it, never a hard-coded address. */
  key_url?: string;
  switch?: string;
  replace_key?: string;
}

/** The one action that fixes a Composio error (#548). */
export type ComposioAction = 'add_composio' | 'replace_key' | 'try_again' | 'switch' | 'turn_on' | string;

/** One app from Composio's catalogue (Composio calls it a toolkit). */
export interface ComposioApp {
  slug: string;
  name: string;
  description: string;
  logo: string | null;
  tools_count: number | null;
  no_auth: boolean;
}

/** One row of GET /composio/apps without `q`: the popular 120 (no tool count) or the whole catalogue. */
export interface ComposioListedApp {
  slug: string;
  name: string;
  logo?: string | null;
  tools_count?: number | null;
  connected?: boolean;
}

export interface ComposioAppList {
  apps: ComposioListedApp[];
  /** `popular`: kwant's bundled top apps (before setup, and with a ck_ key); `all`: Composio's whole catalogue. */
  source: 'popular' | 'all' | string;
  mode: 'session' | 'connect' | 'url' | null;
  can_connect: boolean;
  connected_checked?: boolean;
}

/** One entry of the public directory (mcp-finder/brief.md). Text is plain text, never HTML. */
export interface DirectoryEntry {
  registry_name: string;
  version?: string;
  title: string;
  description: string;
  publisher: { label: string; kind?: string };
  kind: 'remote' | 'stdio' | string;
  url?: string | null;
  transport?: string | null;
  command?: string | null;
  inputs: Array<{ name: string; description?: string; required?: boolean; secret?: boolean }>;
  already_added?: boolean;
  /** Optional (one-app-search, server side): made by the app's maker. The frontend has the same rule when it's missing. */
  by_maker?: boolean;
  /** Optional (one-app-search, server side): how you sign in. */
  sign_in?: 'account' | 'key' | 'none' | 'unknown';
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
  constructor(
    message: string,
    public status: number,
    /** #548: the one action that fixes it, and where to copy a key. */
    public action: ComposioAction | null = null,
    public keyUrl: string | null = null,
  ) {
    super(message);
  }
}

/** An MCP server's local command ("/usr/bin/python3", "npx"), as opposed to the host of a remote one. */
export const isRawCommand = (c: Pick<Connection, 'kind' | 'detail'>): boolean =>
  c.kind === 'mcp' && !!c.detail && !/^[\w-]+(\.[\w-]+)+(:\d+)?$/.test(c.detail);

const sentence = (s: string) => {
  const t = s.trim();
  const up = t.charAt(0).toUpperCase() + t.slice(1);
  return /[.!?]$/.test(up) ? up : `${up}.`;
};

async function call<T>(url: string, method = 'GET', body?: unknown, missing = 'This server doesn’t list connections yet. Update the app and try again.'): Promise<T> {
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
  throw errorOf(res.status, answer, missing);
}

function errorOf(status: number, answer: unknown, missing: string): ConnectionsError {
  const o = answer && typeof answer === 'object' ? (answer as Record<string, unknown>) : {};
  const err = o.error;
  if (status === 403) return new ConnectionsError('Only you can see and change connections.', 403);
  if (status === 404 && !err) return new ConnectionsError(missing, 404);
  const action = typeof o.action === 'string' ? o.action : null;
  const keyUrl = typeof o.key_url === 'string' && /^https:\/\//.test(o.key_url) ? o.key_url : null;
  return new ConnectionsError(typeof err === 'string' && err.trim() ? sentence(err) : `Something went wrong (HTTP ${status}).`, status, action, keyUrl);
}

let appListCache: { etag: string; list: ComposioAppList } | null = null;

/**
 * GET /composio/apps without `q`, once per open, with the last answer's ETag: a 304 reuses it.
 * Matched in the browser on every key (autocomplete.md), so there's no call per key.
 */
async function composioAppList(): Promise<ComposioAppList> {
  let res: Response;
  try {
    res = await personFetch('/api/connections/composio/apps', {
      cache: 'no-store',
      headers: appListCache ? { 'If-None-Match': appListCache.etag } : undefined,
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new ConnectionsError(e.message, 401);
    throw new ConnectionsError('Couldn’t reach the server. Check your connection and try again.', 0, 'try_again');
  }
  if (res.status === 304 && appListCache) return appListCache.list;
  const answer = await res.json().catch(() => null);
  if (!res.ok) throw errorOf(res.status, answer, 'This server can’t list Composio’s apps yet.');
  const o = (answer ?? {}) as Partial<ComposioAppList>;
  const list: ComposioAppList = {
    apps: Array.isArray(o.apps) ? o.apps.filter((a) => a && typeof a.slug === 'string' && typeof a.name === 'string') : [],
    source: typeof o.source === 'string' ? o.source : 'all',
    mode: (o.mode ?? null) as ComposioAppList['mode'],
    can_connect: !!o.can_connect,
    connected_checked: !!o.connected_checked,
  };
  const etag = res.headers.get('ETag');
  appListCache = etag ? { etag, list } : null;
  return list;
}

/** The public directory, from 2 letters. `null` when this server has no directory yet (404): the group hides. */
async function searchDirectory(q: string, signal?: AbortSignal): Promise<{ entries: DirectoryEntry[]; unavailable: boolean } | null> {
  let res: Response;
  try {
    res = await personFetch(`/api/mcp/registry/search?q=${encodeURIComponent(q)}`, { cache: 'no-store', signal });
  } catch (e) {
    if ((e as Error)?.name === 'AbortError') throw e;
    return { entries: [], unavailable: true };
  }
  if (res.status === 404) return null;
  const answer = (await res.json().catch(() => null)) as { entries?: unknown; unavailable?: unknown } | null;
  if (!res.ok || !answer) return { entries: [], unavailable: true };
  const entries = Array.isArray(answer.entries) ? (answer.entries as DirectoryEntry[]).filter((e) => e && typeof e.registry_name === 'string') : [];
  return { entries: entries.slice(0, 20), unavailable: answer.unavailable === true };
}

const enc = encodeURIComponent;

export const connectionsApi = {
  /** The same person-only call, for other Connections sections (0.9.0 P5 tool notes). */
  raw: <T>(url: string, method = 'GET', body?: unknown, missing?: string) => call<T>(url, method, body, missing),
  list: () => call<ConnectionsList>('/api/connections'),
  addComposio: (body: ComposioBody) => call<{ connection_id: string; warning: string }>('/api/connections/composio', 'POST', body),
  /** #548: one Composio key (ck_ or ak_), Composio added off for every bot. */
  addComposioKey: (apiKey: string) =>
    call<{ connection_id: string; mode?: string; warning: string }>('/api/connections/composio', 'POST', { api_key: apiKey.trim(), enabled: false }),
  /** #548: a new key for the saved setup. */
  replaceComposioKey: (apiKey: string) => call<{ mode?: string }>('/api/connections/composio/key', 'PUT', { api_key: apiKey.trim() }),
  /** #548: moves a pasted-link setup to the one-key setup; the key only when none is saved. */
  switchComposio: (apiKey?: string) =>
    call<{ mode?: string }>('/api/connections/composio/switch', 'POST', apiKey?.trim() ? { api_key: apiKey.trim() } : {}),
  composioAppList,
  searchDirectory,
  /** Adds a public-directory entry with the values the person filled in. */
  installFromDirectory: (registryName: string, name: string, env: Record<string, string>, headers: Record<string, string>) =>
    call<{ server?: { id: string } }>('/api/mcp/registry/install', 'POST', { registry_name: registryName, name, env, headers }),
  /** Searches Composio's apps with the person's own Composio key (only once Composio is added). */
  searchComposioApps: (q: string) =>
    call<{ apps: ComposioApp[]; can_connect: boolean; warning: string; hint?: string }>(`/api/connections/composio/apps?q=${enc(q)}`),
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
  // An AI account no bot uses isn't something the person has to fix (Apps & accounts, 0.9.5).
  const attention = list.filter((c) => needsAttention(c.status) && !(c.kind === 'account' && c.used_by.length === 0)).length;
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
/** Dev mode D10: only MCP servers and Composio apps can be marked production. */
export const canMarkProduction = (c: Pick<Connection, 'id'>): boolean => c.id.startsWith('mcp:') || /^composio:[a-z0-9_-]{1,64}$/.test(c.id);

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
  url?: string;
  api_key?: string;
  enabled: boolean;
}

/** The add form as the API wants it, or why it can't be sent yet. */
export function composioBody(url: string, apiKey: string, enabled: boolean): ComposioBody | { error: string } {
  const u = url.trim();
  if (!u) return { error: 'Paste your Composio link first. It’s under MCP in Composio.' };
  if (!isComposioUrl(u)) return { error: 'That isn’t a Composio link. Copy the one under MCP in Composio; it starts with https:// and is on composio.dev.' };
  const key = apiKey.trim();
  return key ? { url: u, api_key: key, enabled } : { url: u, enabled };
}

/** What the add form says when the server refuses. */
export function composioErrorText(e: unknown): string {
  if (e instanceof ConnectionsError && e.status === 409) return 'Composio is already added. Change it under Advanced instead.';
  if (e instanceof ConnectionsError && e.status === 400) return e.message;
  return e instanceof Error ? e.message : 'Something went wrong.';
}
