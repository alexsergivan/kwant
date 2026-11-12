// Settings › Apps & accounts (0.9.5, Mira's audit #3): one page for what bots reach outside kwant
// (apps through Composio, tool servers) and which AI they think with. Connecting an app is a
// three-step sheet; picking bots in step 3 turns the Composio server on for exactly those bots,
// with the same calls the Tools page and the bot editor use (PUT /api/mcp/servers/:id and
// PUT /api/kwants/:id/mcp-servers). Pure helpers here; the page is components/AppsAccounts.tsx.

import type { Connection } from './connections';
import { personFetch } from './auth';
import { formToPayload, serverToForm, type McpServer } from './mcp';
import type { Engine, Profile } from './profiles';

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
}

/** The tiles, in order (brief: Gmail, Google Calendar, Slack, Notion, Shopify, GitHub). */
export const APP_TILES: AppTile[] = [
  { slug: 'gmail', name: 'Gmail', blurb: 'Read mail, write drafts', can: ['Read and search your mail', 'Write drafts for you to check'], acts: 'Send mail' },
  { slug: 'googlecalendar', name: 'Google Calendar', blurb: 'See and add events', can: ['See your calendar', 'Find free time'], acts: 'Add or move events' },
  { slug: 'slack', name: 'Slack', blurb: 'Read channels, post messages', can: ['Read channels and threads', 'Search messages'], acts: 'Post messages' },
  { slug: 'notion', name: 'Notion', blurb: 'Read and edit pages', can: ['Read and search your pages', 'Draft new pages'], acts: 'Change or delete pages' },
  { slug: 'shopify', name: 'Shopify', blurb: 'Orders and products', can: ['See orders, products and customers', 'Look up stock'], acts: 'Change orders or products' },
  { slug: 'github', name: 'GitHub', blurb: 'Issues and pull requests', can: ['Read repositories, issues and pull requests', 'Draft issues and comments'], acts: 'Post, merge or close' },
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
  if (modes.every((m) => m === 'ask')) return { text: `${acts} only after you say yes, every time`, tone: 'ok' };
  if (modes.every((m) => m === 'off')) return { text: `${acts} without asking you first (approvals are off)`, tone: 'warn' };
  if (free.length) return { text: `${acts}: ${listNames(free)} won’t ask first (approvals off); the others ask you`, tone: 'warn' };
  return { text: `${acts}: asks you first when the step looks risky (approvals: ask only when risky)`, tone: 'warn' };
}

export function listNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

// ── AI accounts, named by engine ─────────────────────────────────────────────────────────────

export const ENGINE_NAMES: Record<string, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', copilot: 'Copilot', ollama: 'Ollama', custom: 'Custom API' };

export interface AccountRow {
  id: string;
  profileId: string;
  engine: Engine | string;
  /** "Claude", "Codex" … The profile's own name follows when an engine has more than one. */
  title: string;
  /** "Signed in on this computer · me@example.com" */
  detail: string;
  usedBy: string[];
  /** ready: works. sign_in: nobody uses it and it isn't signed in (quiet, never "needs you"). needs_you: a bot uses it and it can't run. off. */
  state: 'ready' | 'sign_in' | 'needs_you' | 'off' | 'unknown';
  statusText: string | null;
}

const BAD = new Set(['needs_sign_in', 'error', 'limited', 'locked']);

/** True when an account connection needs the person: it's broken and a bot uses it. */
export const accountNeedsYou = (c: Pick<Connection, 'kind' | 'status' | 'used_by'>): boolean =>
  c.kind === 'account' && BAD.has(c.status) && c.used_by.length > 0;

/** The AI accounts list: one row per account, named by its engine, with who thinks with it. */
export function accountRows(connections: Connection[], profiles: Profile[] | null): AccountRow[] {
  const byId = new Map((profiles ?? []).map((p) => [p.id, p]));
  const rows = connections
    .filter((c) => c.kind === 'account')
    .map((c) => {
      const profileId = c.id.replace(/^account:/, '');
      const p = byId.get(profileId);
      const engine = p?.engine ?? (c.detail ?? '').split(' · ')[0] ?? 'custom';
      const label = (c.detail ?? '').includes(' · ') ? (c.detail ?? '').split(' · ').slice(1).join(' · ') : null;
      const system = p ? p.kind === 'system' : c.name === 'This machine';
      const where = p?.custom ? (c.detail ?? 'Custom server') : system ? 'Signed in on this computer' : p?.has_key ? 'API key' : 'Added account';
      const bad = BAD.has(c.status);
      const state: AccountRow['state'] =
        c.status === 'off' ? 'off' : c.status === 'connected' ? 'ready' : bad ? (c.used_by.length ? 'needs_you' : 'sign_in') : 'unknown';
      const notSignedIn = c.status === 'needs_sign_in';
      return {
        id: c.id,
        profileId,
        engine,
        title: ENGINE_NAMES[engine] ?? c.name,
        detail: notSignedIn && !label ? 'Not signed in' : [where, label].filter(Boolean).join(' · '),
        usedBy: c.used_by.map((u) => u.name),
        state,
        statusText: c.status_detail,
        name: c.name,
      };
    });
  // An engine with several accounts: say which one ("Claude · Work"). "This machine" stays plain.
  const counts = new Map<string, number>();
  rows.forEach((r) => counts.set(r.title, (counts.get(r.title) ?? 0) + 1));
  return rows
    .map(({ name, ...r }) => (counts.get(r.title)! > 1 && name !== 'This machine' ? { ...r, title: `${r.title} · ${name}` } : r))
    .sort((a, b) => rank(a) - rank(b) || a.title.localeCompare(b.title));
}

// Used accounts first, then unused ones, then off.
const rank = (r: AccountRow) => (r.state === 'off' ? 3 : r.usedBy.length ? 0 : r.state === 'ready' ? 1 : 2);

export function usedByLine(names: string[]): string {
  if (names.length === 0) return 'No bot uses it yet';
  if (names.length > 3) return `Used by ${names.slice(0, 2).join(', ')} and ${names.length - 2} more`;
  return `Used by ${listNames(names)}`;
}

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

/** What the page's header says: things that need the person, and what's connected. */
export function pageCounts(connections: Connection[]): string {
  const apps = connections.filter((c) => c.kind === 'mcp' && c.status === 'connected').length;
  const accounts = connections.filter((c) => c.kind === 'account' && c.status === 'connected').length;
  const needs = connections.filter((c) => (c.kind === 'account' ? accountNeedsYou(c) : BAD.has(c.status))).length;
  const parts = [`${accounts} AI ${accounts === 1 ? 'account' : 'accounts'} ready`, `${apps} tool ${apps === 1 ? 'server' : 'servers'} on`];
  if (needs) parts.push(`${needs} ${needs === 1 ? 'needs' : 'need'} you`);
  return parts.join(' · ');
}
