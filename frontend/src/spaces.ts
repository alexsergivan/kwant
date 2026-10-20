// Spaces: separate sets of bots, teams, channels and accounts. The open space rides on every
// `/api` call as `?space=` (added by the fetch wrapper in auth.ts) and on the event stream.

import { serverReason } from './routeReason';

export interface SpaceCounts {
  bots: number;
  ready_accounts: number;
  unread: number;
}

export interface Space {
  id: string;
  name: string;
  /** An emoji, or "box" from an older server. */
  icon: string;
  color: string | null;
  sort: number;
  is_default: boolean;
  archived_at: string | null;
  created_at: string;
  /** Null until the space's first bot exists. */
  general_channel_id: string | null;
  counts: SpaceCounts;
  paused_routines?: number;
  /** The approvals switch; older servers leave it out ("ask"). */
  approvals?: 'ask' | 'off';
}

export interface NewSpace {
  name: string;
  icon: string;
  color: string;
  this_machine_enabled: boolean;
}

export interface SpacePatch {
  name?: string;
  icon?: string;
  color?: string | null;
  archived?: boolean;
}

export const DEFAULT_SPACE_ID = 'spc_default';

/** The icons offered when creating a space. */
export const SPACE_ICONS = ['💼', '🌿', '📚', '🎮', '🏠', '🚀', '🎨', '🧪'];

/** Space colours, in the order a new space takes the first unused one. */
export const SPACE_COLORS = ['#f59e0b', '#10b981', '#0ea5e9', '#8b5cf6', '#f43f5e', '#14b8a6', '#eab308', '#ec4899'];

const FALLBACK_ICON = '📦';

/** The emoji to draw for a space; older rows store a word like "box". */
export function spaceIcon(s: Pick<Space, 'icon'>): string {
  const icon = (s.icon || '').trim();
  return !icon || /^[a-z_-]+$/i.test(icon) ? FALLBACK_ICON : icon;
}

/** Colour words older rows and the server's Default use ("amber"). */
const NAMED_COLORS: Record<string, string> = {
  amber: '#f59e0b',
  emerald: '#10b981',
  sky: '#0ea5e9',
  violet: '#8b5cf6',
  rose: '#f43f5e',
  teal: '#14b8a6',
  yellow: '#eab308',
  pink: '#ec4899',
};

const hexOf = (c: string | null | undefined): string | null => {
  const v = (c || '').trim().toLowerCase();
  if (/^#[0-9a-f]{3,8}$/.test(v)) return v;
  return NAMED_COLORS[v] ?? null;
};

/** The space's colour, or a stable one from the palette when it has none. */
export function spaceColor(s: Pick<Space, 'id' | 'color'>): string {
  const hex = hexOf(s.color);
  if (hex) return hex;
  let h = 0;
  for (const ch of s.id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return SPACE_COLORS[h % SPACE_COLORS.length];
}

/** The first palette colour no other space uses yet (archived ones included); the first when all are taken. */
export function firstUnusedColor(spaces: Array<Pick<Space, 'color'>>): string {
  const used = new Set(spaces.map((s) => hexOf(s.color)));
  return SPACE_COLORS.find((c) => !used.has(c)) ?? SPACE_COLORS[0];
}

/** Why a space name can't be saved, or null when it can. `taken` are the other spaces' names. */
export function spaceNameError(name: string, taken: string[]): string | null {
  const n = name.trim();
  if (!n) return 'Give the space a name.';
  if (n.length > 60) return 'Keep the name under 60 characters.';
  if (taken.some((t) => t.trim().toLowerCase() === n.toLowerCase())) return `You already have a space called ${n}.`;
  return null;
}

/** "6 kwant bots · 2 accounts ready" under the space name. */
export function spaceSummary(c: SpaceCounts | undefined): string {
  if (!c) return '';
  const bots = `${c.bots} kwant ${c.bots === 1 ? 'bot' : 'bots'}`;
  const accounts = `${c.ready_accounts} ${c.ready_accounts === 1 ? 'account' : 'accounts'} ready`;
  return `${bots} · ${accounts}`;
}

// ---- The open space, for every request ----

let currentSpaceId: string | null = null;

export const setCurrentSpace = (id: string | null) => {
  currentSpaceId = id;
};
export const getCurrentSpace = () => currentSpaceId;

/** API paths that are app-wide, so they never get `?space=`. */
const GLOBAL_PREFIXES = [
  '/api/auth',
  '/api/spaces',
  '/api/health',
  '/api/settings',
  '/api/laya',
  '/api/system',
  '/api/mcp/oauth/callback',
  '/api/attachments',
  '/api/mcp/permission',
  '/api/mcp/gateway',
  '/api/mcp/self',
];

/** Adds `space=<id>` to a same-origin `/api` URL that's space-scoped and doesn't already name one. */
export function withSpace(url: string, origin: string, spaceId: string | null): string {
  if (!spaceId) return url;
  let parsed: URL;
  try {
    parsed = new URL(url, origin);
  } catch {
    return url;
  }
  if (parsed.origin !== origin || !parsed.pathname.startsWith('/api/')) return url;
  if (GLOBAL_PREFIXES.some((p) => parsed.pathname === p || parsed.pathname.startsWith(`${p}/`))) return url;
  if (parsed.searchParams.has('space')) return url;
  parsed.searchParams.set('space', spaceId);
  // Keep relative URLs relative.
  return /^https?:/i.test(url) ? parsed.href : `${parsed.pathname}${parsed.search}${parsed.hash}`;
}

/** The event stream for one space. */
export const eventsUrl = (spaceId: string | null) => (spaceId ? `/api/events?space=${encodeURIComponent(spaceId)}` : '/api/events');

// ---- Addresses: /s/<space>/c/<channel>, /s/<space>/b/<bot> ----

export interface SpaceRoute {
  spaceId: string | null;
  channelId?: string;
  botId?: string;
}

export function parseSpacePath(path: string): SpaceRoute {
  const m = path.match(/^\/s\/([^/]+)(?:\/(c|b)\/([^/]+))?\/?$/);
  if (!m) return { spaceId: null };
  const spaceId = decodeURIComponent(m[1]);
  if (m[2] === 'c') return { spaceId, channelId: decodeURIComponent(m[3]) };
  if (m[2] === 'b') return { spaceId, botId: decodeURIComponent(m[3]) };
  return { spaceId };
}

export function spacePath(spaceId: string, target?: { channelId?: string | null; botId?: string | null }): string {
  const base = `/s/${encodeURIComponent(spaceId)}`;
  if (target?.channelId) return `${base}/c/${encodeURIComponent(target.channelId)}`;
  if (target?.botId) return `${base}/b/${encodeURIComponent(target.botId)}`;
  return base;
}

const LAST_SPACE_KEY = 'kwant.lastSpace';
const lastChatKey = (spaceId: string) => `kwant.lastChat.${spaceId}`;

export function rememberSpace(id: string) {
  try {
    localStorage.setItem(LAST_SPACE_KEY, id);
  } catch {
    // private mode
  }
}
export function lastSpace(): string | null {
  try {
    return localStorage.getItem(LAST_SPACE_KEY);
  } catch {
    return null;
  }
}

/** The chat open in a space, so switching back reopens it. */
export function rememberChat(spaceId: string, target: { channelId?: string | null; botId?: string | null }) {
  try {
    localStorage.setItem(lastChatKey(spaceId), JSON.stringify({ channelId: target.channelId ?? null, botId: target.botId ?? null }));
  } catch {
    // private mode
  }
}
export function lastChat(spaceId: string): { channelId?: string; botId?: string } | null {
  try {
    const raw = localStorage.getItem(lastChatKey(spaceId));
    if (!raw) return null;
    const v = JSON.parse(raw);
    return { channelId: v?.channelId ?? undefined, botId: v?.botId ?? undefined };
  } catch {
    return null;
  }
}

/** Which space to open: the address, then the last one used, then Default, then the first active one. */
export function pickSpace(spaces: Space[], fromUrl: string | null, remembered: string | null): Space | null {
  const active = spaces.filter((s) => !s.archived_at);
  return (
    active.find((s) => s.id === fromUrl) ??
    active.find((s) => s.id === remembered) ??
    active.find((s) => s.is_default) ??
    active[0] ??
    null
  );
}

/**
 * The chat a space opens on: the bot or channel asked for (a link, a reload, a notification) if it
 * still exists, else #general, else the first channel, else the first bot. A bot target never
 * falls through to #general just because #general exists.
 */
export function pickInitialChat<B extends { id: string }, C extends { id: string }>(
  target: { channelId?: string | null; botId?: string | null } | null | undefined,
  bots: B[],
  channels: C[],
  generalId: string | null | undefined,
): { kind: 'bot'; bot: B } | { kind: 'channel'; channel: C } | null {
  const channel = target?.channelId ? channels.find((c) => c.id === target.channelId) : undefined;
  if (channel) return { kind: 'channel', channel };
  const bot = target?.botId ? bots.find((b) => b.id === target.botId) : undefined;
  if (bot) return { kind: 'bot', bot };
  const fallback = channels.find((c) => c.id === generalId) ?? channels[0];
  if (fallback) return { kind: 'channel', channel: fallback };
  return bots[0] ? { kind: 'bot', bot: bots[0] } : null;
}

// ---- #general: what Laya did ----

export interface RoutingMeta {
  kwant_id?: string;
  kwant_name?: string;
  reason?: string;
  candidate_count?: number | null;
  fallback?: boolean;
  matched_terms?: string[];
  /** How the bot was picked: skills, keywords, only, lead, several or discuss; see routeReason.ts. */
  basis?: string;
}

export function parseRoutingMeta(meta: string | null | undefined): RoutingMeta | null {
  if (!meta) return null;
  try {
    const r = JSON.parse(meta)?.routing;
    return r && typeof r === 'object' && typeof r.kwant_name === 'string' ? (r as RoutingMeta) : null;
  } catch {
    return null;
  }
}

/** The server's own reason when it sends one, else "Laya picked Lin from 6 bots in Work · matched 'migration', 'SQLite'", or the keyword-fallback line. */
export function routingLine(r: RoutingMeta, spaceName: string): string {
  const n = typeof r.candidate_count === 'number' && r.candidate_count > 0 ? r.candidate_count : null;
  const pool = n ? `from ${n} ${n === 1 ? 'bot' : 'bots'} in ${spaceName}` : `in ${spaceName}`;
  const own = serverReason(r);
  if (own) return `${own} Chosen ${pool}.`;
  const matched = r.matched_terms?.length ? ` · matched ${r.matched_terms.map((t) => `'${t}'`).join(', ')}` : '';
  if (r.fallback) return `Picked by keywords (Laya is offline) · ${r.kwant_name} ${pool}${matched}`;
  if (r.basis === 'lead') return `No one’s role matched, so the team lead ${r.kwant_name} took it ${pool}`;
  if (r.basis === 'only') return `${r.kwant_name} is the only bot that can answer ${pool}`;
  return `Laya picked ${r.kwant_name} ${pool}${matched}`;
}

/** "ran on Work account" from a reply's `ran_on`, or null. */
export function ranOnAccount(meta: string | null | undefined): string | null {
  if (!meta) return null;
  try {
    const a = JSON.parse(meta)?.ran_on?.account;
    return typeof a === 'string' && a.trim() ? a.trim() : null;
  } catch {
    return null;
  }
}

// ---- API ----

async function errorText(res: Response, fallback: string): Promise<string> {
  try {
    const body = await res.json();
    if (body && typeof body.error === 'string') return body.error;
  } catch {
    // not JSON
  }
  return `${fallback} (HTTP ${res.status}).`;
}

async function json<T>(res: Response, fallback: string): Promise<T> {
  if (!res.ok) throw new Error(await errorText(res, fallback));
  return res.json() as Promise<T>;
}

export const spacesApi = {
  list: (): Promise<Space[]> => fetch('/api/spaces', { cache: 'no-store' }).then((r) => json<Space[]>(r, 'Couldn’t load spaces')),
  archived: (): Promise<Space[]> =>
    fetch('/api/spaces?archived=1', { cache: 'no-store' })
      .then((r) => json<Space[]>(r, 'Couldn’t load archived spaces'))
      .then((all) => all.filter((s) => !!s.archived_at)),
  create: (body: NewSpace): Promise<Space> =>
    fetch('/api/spaces', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }).then((r) =>
      json<Space>(r, 'Couldn’t create the space'),
    ),
  patch: (id: string, body: SpacePatch): Promise<Space> =>
    fetch(`/api/spaces/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }).then((r) => json<Space>(r, 'Couldn’t save the space')),
};
