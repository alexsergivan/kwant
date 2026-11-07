// Settings → Coding agents (0.7.4): connections that let Claude Code, Codex or Cursor ask kwant bots for
// work. The `kag_` token comes back from create only; the server keeps its hash. It lives in the card's
// state while the "copy it now" view is open and nowhere else: not in a list, URL, log or storage.

export type BotScope = 'all' | string[];

export interface AgentConnection {
  id: string;
  name: string;
  space_ids: string[];
  bots: BotScope;
  created_at: string;
  last_used_at: string | null;
}

export interface AgentConnectionList {
  connections: AgentConnection[];
  max: number;
}

export interface AgentCall {
  connection_id: string;
  connection_name: string;
  tool: string;
  bot: string | null;
  outcome: 'ok' | 'refused' | string;
  at: string;
}

export interface NewConnection {
  name: string;
  space_ids: string[];
  bots: BotScope;
}

export interface Created {
  connection: AgentConnection;
  token: string;
}

export const NAME_MAX = 40;
export const DEFAULT_MAX = 10;
export const limitText = (max = DEFAULT_MAX) => `You already have ${max} coding agents connected. Revoke one first.`;
export const LIMIT_TEXT = limitText();
export const TOKEN_WARNING = 'Shown once. Anyone with it can ask these bots for work, like a password.';

/** The same rules as the server: trimmed, 1–40 characters, no line breaks, control characters, [ ] or < >. */
export function nameError(raw: string): string | null {
  const name = raw.trim();
  if (!name) return 'Give it a name, for example “Claude Code on my laptop”.';
  if ([...name].length > NAME_MAX) return `Keep the name to ${NAME_MAX} characters.`;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f\u0080-\u009f]/.test(name)) return 'The name can’t have line breaks.';
  if (/[[\]<>]/.test(name)) return 'The name can’t have [ ] or < >.';
  return null;
}

/** What Create may send, or null while something is missing. */
export function createBody(name: string, spaceIds: string[], allBots: boolean, botIds: string[]): NewConnection | null {
  if (nameError(name)) return null;
  const spaces = [...new Set(spaceIds)];
  if (!spaces.length) return null;
  if (!allBots && !botIds.length) return null;
  return { name: name.trim(), space_ids: spaces, bots: allBots ? 'all' : [...new Set(botIds)] };
}

/** A plain error for the person. Never echoes a request body, so a token can't end up in one. */
export class ConnectionError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function errorOf(r: Response, fallback: string): Promise<ConnectionError> {
  let text = '';
  try {
    const body = await r.json();
    if (body && typeof body.error === 'string') text = body.error.trim();
  } catch {
    /* not JSON */
  }
  if (r.status === 409) return new ConnectionError(LIMIT_TEXT, 409);
  if (r.status === 400 && text) return new ConnectionError(text, 400);
  if (r.status === 404) return new ConnectionError('That connection is already gone.', 404);
  if (r.status === 401 || r.status === 403) return new ConnectionError('Only you can manage coding agents, signed in on this device.', r.status);
  return new ConnectionError(fallback, r.status);
}

type Fetch = typeof fetch;

const OFFLINE = 'Couldn’t reach the server. Check the connection and try again.';

async function call(send: Fetch, url: string, init: RequestInit | undefined, fallback: string): Promise<Response> {
  let r: Response;
  try {
    r = await send(url, { cache: 'no-store', ...init });
  } catch {
    throw new ConnectionError(OFFLINE, 0);
  }
  if (!r.ok) throw await errorOf(r, fallback);
  return r;
}

export function parseConnection(x: unknown): AgentConnection | null {
  if (!x || typeof x !== 'object') return null;
  const o = x as Record<string, unknown>;
  if (typeof o.id !== 'string' || typeof o.name !== 'string') return null;
  const spaces = Array.isArray(o.space_ids) ? o.space_ids.filter((s): s is string => typeof s === 'string') : [];
  const bots: BotScope = Array.isArray(o.bots) ? o.bots.filter((b): b is string => typeof b === 'string') : 'all';
  return {
    id: o.id,
    name: o.name,
    space_ids: spaces,
    bots: Array.isArray(bots) && !bots.length ? 'all' : bots,
    created_at: typeof o.created_at === 'string' ? o.created_at : '',
    last_used_at: typeof o.last_used_at === 'string' && o.last_used_at ? o.last_used_at : null,
  };
}

export const agentConnectionsApi = (send: Fetch = (...a) => fetch(...a)) => ({
  async list(): Promise<AgentConnectionList> {
    const r = await call(send, '/api/agent-connections', undefined, 'Couldn’t load coding agents.');
    const body = (await r.json()) as { connections?: unknown[]; max?: unknown };
    return {
      connections: (body.connections ?? []).map(parseConnection).filter((c): c is AgentConnection => !!c),
      max: typeof body.max === 'number' && body.max > 0 ? body.max : DEFAULT_MAX,
    };
  },
  async create(body: NewConnection): Promise<Created> {
    const r = await call(
      send,
      '/api/agent-connections',
      { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) },
      'Couldn’t connect it. Try again.',
    );
    const out = (await r.json()) as { connection?: unknown; token?: unknown };
    const connection = parseConnection(out.connection);
    if (!connection || typeof out.token !== 'string' || !out.token.startsWith('kag_')) throw new ConnectionError('Couldn’t connect it. Try again.', r.status);
    return { connection, token: out.token };
  },
  async revoke(id: string): Promise<void> {
    await call(send, `/api/agent-connections/${encodeURIComponent(id)}`, { method: 'DELETE' }, 'Couldn’t revoke it. Try again.');
  },
  async activity(id: string): Promise<AgentCall[]> {
    const r = await call(send, `/api/agent-connections/${encodeURIComponent(id)}/activity`, undefined, 'Couldn’t load recent activity.');
    const body = (await r.json()) as { calls?: AgentCall[] };
    return Array.isArray(body.calls) ? body.calls.filter((c) => c && typeof c.tool === 'string' && typeof c.at === 'string') : [];
  },
});

// ---------------------------------------------------------------- setup snippets (docs/agents-connect.md)

export type SetupTool = 'claude' | 'codex' | 'cursor';

export const SETUP_TOOLS: Array<{ id: SetupTool; label: string; where: string }> = [
  { id: 'claude', label: 'Claude Code', where: 'Run this once in a terminal. Add --scope user to have it in every project.' },
  { id: 'codex', label: 'Codex', where: 'Add this to ~/.codex/config.toml, then set the token in your shell profile.' },
  { id: 'cursor', label: 'Cursor', where: 'Add this to ~/.cursor/mcp.json, or .cursor/mcp.json in a project you don’t commit, then set the token in your shell profile.' },
];

/** The placeholders the docs use; the card fills in this server's address and the new token. */
export const DOC_SERVER = 'https://<server>';
export const DOC_TOKEN = 'kag_…';

/** The address the agent calls: this page's origin, or the docs placeholder. */
export function serverUrl(origin: string | null | undefined): string {
  const o = (origin ?? '').replace(/\/+$/, '');
  return /^https?:\/\/[^\s"'<>]+$/i.test(o) ? o : DOC_SERVER;
}

/** One or more copyable blocks per tool, the same text as docs/agents-connect.md. */
/** The name the tool lists this server under, as in the docs. */
const SERVER_NAME = 'kwant';

export function setupSnippets(tool: SetupTool, server: string, token: string): string[] {
  const url = `${server}/api/mcp/agents`;
  const n = SERVER_NAME;
  switch (tool) {
    case 'claude':
      return [`claude mcp add --transport http ${n} ${url} --header "Authorization: Bearer ${token}"`];
    case 'codex':
      return [`[mcp_servers.${n}]\nurl = "${url}"\nbearer_token_env_var = "KWANT_AGENT_TOKEN"`, `export KWANT_AGENT_TOKEN=${token}`];
    case 'cursor':
      return [
        `{\n  "mcpServers": {\n    "${n}": {\n      "url": "${url}",\n      "headers": { "Authorization": "Bearer \${env:KWANT_AGENT_TOKEN}" }\n    }\n  }\n}`,
        `export KWANT_AGENT_TOKEN=${token}`,
      ];
  }
}

// ---------------------------------------------------------------- how a row reads

/** "Vega and Lin", "All bots", or "3 bots" when names aren't known. */
export function botsLine(bots: BotScope, nameOf: (id: string) => string | undefined): string {
  if (bots === 'all') return 'All bots';
  const names = bots.map((id) => nameOf(id)).filter((n): n is string => !!n);
  if (names.length !== bots.length) return bots.length === 1 ? '1 bot' : `${bots.length} bots`;
  return listJoin(names);
}

export function spacesLine(ids: string[], nameOf: (id: string) => string | undefined): string {
  const names = ids.map((id) => nameOf(id) ?? 'a removed space');
  return listJoin(names);
}

export function listJoin(xs: string[]): string {
  if (xs.length <= 1) return xs[0] ?? '';
  if (xs.length === 2) return `${xs[0]} and ${xs[1]}`;
  return `${xs.slice(0, -1).join(', ')} and ${xs[xs.length - 1]}`;
}

/** "7 Nov 2026" in the person's zone. */
export function shortDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
}

/** "Never", "Just now", "12 min ago", "3 h ago", or a date. */
export function lastUsed(iso: string | null, now = Date.now()): string {
  if (!iso) return 'Never';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return 'Never';
  const mins = Math.floor((now - t) / 60_000);
  if (mins < 1) return 'Just now';
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  return shortDate(iso);
}

const TOOL_LABELS: Record<string, string> = {
  list_bots: 'Looked up the bots',
  ask: 'Asked',
  get_reply: 'Checked a reply from',
  list_threads: 'Looked up its threads',
};

/** "Asked Vega", "Looked up the bots", with "(refused)" when kwant said no. */
export function callLine(c: Pick<AgentCall, 'tool' | 'bot' | 'outcome'>): string {
  const verb = TOOL_LABELS[c.tool] ?? c.tool.replace(/_/g, ' ');
  const line = c.bot && (c.tool === 'ask' || c.tool === 'get_reply' || !TOOL_LABELS[c.tool]) ? `${verb} ${c.bot}` : c.bot ? `${verb} (${c.bot})` : verb;
  return c.outcome === 'refused' ? `${line}, refused` : line;
}

/** "00:12" today, else "6 Nov, 23:40". */
export function callTime(iso: string, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const time = d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  return d.toDateString() === now.toDateString() ? time : `${d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })}, ${time}`;
}

/** "Claude Code (coding agent)" for a message a coding agent sent, else null (the person's own). */
export function agentSenderLabel(meta: string | null | undefined): string | null {
  if (!meta) return null;
  try {
    const m = JSON.parse(meta) as { from_agent?: unknown };
    const name = typeof m?.from_agent === 'string' ? m.from_agent.trim() : '';
    return name ? `${name} (coding agent)` : null;
  } catch {
    return null;
  }
}
