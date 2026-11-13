// Settings › Apps & accounts (0.9.5, Mira's audit #3): one page for what bots reach outside kwant
// (apps through Composio, tool servers) and which AI they think with. Connecting an app is a
// three-step sheet; picking bots in step 3 turns the Composio server on for exactly those bots,
// with the same calls the Tools page and the bot editor use (PUT /api/mcp/servers/:id and
// PUT /api/kwants/:id/mcp-servers). Pure helpers here; the page is components/AppsAccounts.tsx.

import type { Connection } from './connections';
import { personFetch } from './auth';
import { formToPayload, serverToForm, type McpServer } from './mcp';

/** Where Apps & accounts opens: at the top, at AI accounts, or with Advanced open. */
export type { AppsSection } from './nav';

export interface AppTile {
  /** Composio's toolkit slug. */
  slug: string;
  name: string;
  /** Under the name on the tile. */
  blurb: string;
  /** "Your bots will be able to …", in plain words. Sending or changing things is said separately. */
  can: string[];
  /** The step that acts for the person (sends, posts, buys): the approvals line goes next to it. */
  acts: string;
  /** Our own tint for the initials on the #231d4b ground (not a brand colour); search hits have none. */
  tint?: string;
}

/** The tiles, in order (brief: Gmail, Google Calendar, Slack, Notion, Shopify, GitHub). */
export const APP_TILES: AppTile[] = [
  { slug: 'gmail', tint: 'text-rose-300', name: 'Gmail', blurb: 'Read mail, write drafts', can: ['Read and search your mail', 'Write drafts for you to check'], acts: 'Send mail' },
  { slug: 'googlecalendar', tint: 'text-blue-300', name: 'Google Calendar', blurb: 'See and add events', can: ['See your calendar', 'Find free time'], acts: 'Add or move events' },
  { slug: 'slack', tint: 'text-amber-200', name: 'Slack', blurb: 'Read and post messages', can: ['Read channels and threads', 'Search messages'], acts: 'Post messages' },
  { slug: 'notion', tint: 'text-zinc-200', name: 'Notion', blurb: 'Read and edit pages', can: ['Read and search your pages', 'Draft new pages'], acts: 'Change or delete pages' },
  { slug: 'shopify', tint: 'text-emerald-300', name: 'Shopify', blurb: 'Orders and products', can: ['See orders, products and customers', 'Look up stock'], acts: 'Change orders or products' },
  { slug: 'github', tint: 'text-violet-200', name: 'GitHub', blurb: 'Issues and pull requests', can: ['Read repositories, issues and pull requests', 'Draft issues and comments'], acts: 'Post, merge or close' },
];

/** A search hit opened in the sheet: what we can say about an app we have no copy for. */
export function tileFor(app: { slug: string; name: string; description?: string | null }): AppTile {
  const known = APP_TILES.find((t) => t.slug === app.slug);
  if (known) return known;
  const what = (app.description ?? '').trim().replace(/\.$/, '');
  return { slug: app.slug, name: app.name, blurb: what, can: [what ? `Use ${app.name}: ${what.charAt(0).toLowerCase()}${what.slice(1)}` : `Use ${app.name}’s tools`], acts: 'Send, post or change things' };
}

/** "M" for Gmail, "GC" for Google Calendar: the plain mark on a tile (no brand logos). */
export function appInitials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return '?';
  if (words.length === 1) return name.toLowerCase() === 'gmail' ? 'M' : words[0].charAt(0).toUpperCase();
  return (words[0].charAt(0) + words[1].charAt(0)).toUpperCase();
}

// ── Approvals: what the sheet promises about sending ─────────────────────────────────────────

export type Mode = 'ask' | 'smart' | 'off';

/**
 * The line next to "Send mail" etc., from the approvals the chosen bots actually run with
 * (each bot's effective mode; with no bot picked, the space's). Never promises "asks first"
 * when a bot is on "Don't ask".
 */
export function approvalLine(acts: string, bots: Array<{ name: string; approvals_effective?: string | null }>, spaceMode: string | undefined): { text: string; tone: 'ok' | 'warn' } {
  const mode = (m: string | null | undefined): Mode => (m === 'off' || m === 'smart' ? m : 'ask');
  const modes = bots.length ? bots.map((b) => mode(b.approvals_effective ?? spaceMode)) : [mode(spaceMode)];
  const free = bots.filter((b) => mode(b.approvals_effective ?? spaceMode) === 'off').map((b) => b.name);
  // Smart asks before any tool the server doesn't mark read-only, for every engine (Ada, fixes-542 F3), so it reads like ask.
  // Not "every time": the approval can be answered "always allow" for the session.
  if (modes.every((m) => m !== 'off')) return { text: `${acts} only after you say yes`, tone: 'ok' };
  if (modes.every((m) => m === 'off')) return { text: `${acts} without asking you first (approvals are off)`, tone: 'warn' };
  if (free.length) return { text: `${acts}: ${listNames(free)} won’t ask first (approvals off); the others ask you`, tone: 'warn' };
  return { text: `${acts}: ${listNames(free)} won’t ask first (approvals off); the others ask you`, tone: 'warn' };
}

export function listNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

// ── AI accounts: the needs-you count ─────────────────────────────────────────────────────────────

const BAD = new Set(['needs_sign_in', 'error', 'limited', 'locked']);

/** True when an account connection needs the person: it's broken and a bot uses it. */
export const accountNeedsYou = (c: Pick<Connection, 'kind' | 'status' | 'used_by'>): boolean =>
  c.kind === 'account' && BAD.has(c.status) && c.used_by.length > 0;

// ── Giving the Composio server to the chosen bots ────────────────────────────────────────────

/** The MCP server id inside a connection id (`mcp:<id>`). */
export const serverIdOf = (connectionId: string | null | undefined): string | null =>
  connectionId && connectionId.startsWith('mcp:') ? connectionId.slice(4) : null;

/**
 * Each bot's new tool list: the server added for the chosen bots and removed for the rest, with
 * their other tools left as they were. Only bots whose list changes are returned.
 */
export function assignmentsFor(serverId: string, current: Record<string, string[]>, chosen: string[]): Record<string, string[]> {
  const pick = new Set(chosen);
  const out: Record<string, string[]> = {};
  for (const [kwant, ids] of Object.entries(current)) {
    const has = ids.includes(serverId);
    if (pick.has(kwant) && !has) out[kwant] = [...ids, serverId];
    else if (!pick.has(kwant) && has) out[kwant] = ids.filter((id) => id !== serverId);
  }
  return out;
}

export class GiveError extends Error {}

type Fetch = (url: string, init?: RequestInit) => Promise<Response>;

async function json<T>(f: Fetch, url: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await f(url, init);
  } catch {
    throw new GiveError('Couldn’t reach kwant bots. Check your connection and try again.');
  }
  const body = await res.json().catch(() => null);
  if (!res.ok) {
    const err = body && typeof body === 'object' ? (body as { error?: unknown }).error : null;
    throw new GiveError(typeof err === 'string' && err ? err : `Something went wrong (HTTP ${res.status}).`);
  }
  return body as T;
}

/** Which of these bots have the server now. */
export async function botsWithServer(serverId: string, kwantIds: string[], f: Fetch = (u, i) => personFetch(u, i)): Promise<string[]> {
  const lists = await Promise.all(kwantIds.map((id) => json<{ server_ids: string[] }>(f, `/api/kwants/${encodeURIComponent(id)}/mcp-servers`)));
  return kwantIds.filter((_, i) => lists[i].server_ids.includes(serverId));
}

/**
 * Turns the server on and gives it to exactly `chosen` among `kwantIds`: the same two calls the
 * Tools page (enabled) and the bot editor (its tool list) make. With nobody chosen the server is
 * left as it is, only taken away from every bot.
 */
export async function giveServerTo(serverId: string, kwantIds: string[], chosen: string[], f: Fetch = (u, i) => personFetch(u, i)): Promise<void> {
  const current: Record<string, string[]> = {};
  const lists = await Promise.all(kwantIds.map((id) => json<{ server_ids: string[] }>(f, `/api/kwants/${encodeURIComponent(id)}/mcp-servers`)));
  kwantIds.forEach((id, i) => (current[id] = lists[i].server_ids));
  const changes = assignmentsFor(serverId, current, chosen);
  for (const [kwant, ids] of Object.entries(changes)) {
    await json(f, `/api/kwants/${encodeURIComponent(kwant)}/mcp-servers`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ server_ids: ids }),
    });
  }
  if (chosen.length === 0) return;
  const server = await json<McpServer>(f, `/api/mcp/servers/${encodeURIComponent(serverId)}`);
  if (server.enabled) return;
  // Sending the masks back keeps every stored secret (as Tools' on/off switch does).
  await json(f, `/api/mcp/servers/${encodeURIComponent(serverId)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ...formToPayload(serverToForm(server)), enabled: true }),
  });
}

/** Turns a tool server on or off, as Tools' switch does (the masks go back, so stored secrets stay). */
export async function setServerEnabled(serverId: string, enabled: boolean, f: Fetch = (u, i) => personFetch(u, i)): Promise<void> {
  const url = `/api/mcp/servers/${encodeURIComponent(serverId)}`;
  const server = await json<McpServer>(f, url);
  if (server.enabled === enabled) return;
  await json(f, url, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ ...formToPayload(serverToForm(server)), enabled }),
  });
}

/** Removes a tool server (Composio's included): every bot loses it. */
export async function removeServer(serverId: string, f: Fetch = (u, i) => personFetch(u, i)): Promise<void> {
  await json(f, `/api/mcp/servers/${encodeURIComponent(serverId)}`, { method: 'DELETE' });
}

/** What the page's header says: things that need the person, and what's connected. */
export function pageCounts(connections: Connection[]): string {
  const apps = connections.filter((c) => c.kind === 'mcp' && c.status === 'connected').length;
  const accounts = connections.filter((c) => c.kind === 'account' && c.status === 'connected').length;
  const needs = connections.filter((c) => (c.kind === 'account' ? accountNeedsYou(c) : BAD.has(c.status))).length;
  const parts = [`${accounts} AI ${accounts === 1 ? 'account' : 'accounts'} ready`, `${apps} tool ${apps === 1 ? 'server' : 'servers'} on`];
  if (needs) parts.push(`${needs} ${needs === 1 ? 'needs' : 'need'} you`);
  return parts.join(' · ');
}
