// MCP suggestions: a bot found a tool server it needs and suggests it; only the person installs it.
// Contract: docs/mcp-api.md → "Suggestions".
import { parseMetaObject } from './questions';
import { McpServer, McpTestResult, joinCommandLine } from './mcp';
import { request } from './threads';

export type SuggestionStatus = 'pending' | 'installing' | 'installed' | 'dismissed';

export interface SuggestionInput {
  name: string;
  kind: 'env' | 'header';
  description: string;
  required: boolean;
  secret: boolean;
  /** e.g. `Bearer {api_key}`: the person types only the part in braces. */
  template: string | null;
  placeholder: string | null;
}

/** `meta` of the card message (`kind: "mcp_suggestion"`); never holds secret values. */
export interface SuggestionMeta {
  kind: 'mcp_suggestion';
  suggestion_id: string;
  kwant_id: string | null;
  registry_name: string;
  version: string;
  title: string;
  description: string;
  reason: string;
  repository_url: string | null;
  transport: 'http' | 'stdio' | null;
  url: string | null;
  command: string | null;
  args: string[];
  package: string | null;
  runs_on_your_computer: boolean;
  fixed_env: string[];
  fixed_headers: string[];
  inputs: SuggestionInput[];
  status: SuggestionStatus;
  server_id: string | null;
  last_error: string | null;
}

/** A suggestion record from the API or `mcp_suggestion_updated`; `card` is its meta. */
export interface Suggestion {
  id: string;
  kwant_id: string;
  status: SuggestionStatus;
  server_id: string | null;
  last_error: string | null;
  message_id: string | null;
  created_at: string;
  card?: unknown;
}

export interface InstallResult {
  suggestion: Suggestion;
  server: McpServer;
  test: McpTestResult;
  oauth_required: boolean;
}

const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);
const strs = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
const STATUSES: SuggestionStatus[] = ['pending', 'installing', 'installed', 'dismissed'];

function parseInput(raw: unknown): SuggestionInput | null {
  if (!raw || typeof raw !== 'object') return null;
  const i = raw as Record<string, unknown>;
  const name = str(i.name);
  if (!name || (i.kind !== 'env' && i.kind !== 'header')) return null;
  return {
    name,
    kind: i.kind,
    description: str(i.description) ?? '',
    required: i.required === true,
    secret: i.secret !== false,
    template: str(i.template),
    placeholder: str(i.placeholder),
  };
}

export function parseSuggestionMeta(raw: unknown): SuggestionMeta | null {
  const m = parseMetaObject(raw);
  if (!m || m.kind !== 'mcp_suggestion') return null;
  const id = str(m.suggestion_id);
  const name = str(m.registry_name);
  if (!id || !name) return null;
  return {
    kind: 'mcp_suggestion',
    suggestion_id: id,
    kwant_id: str(m.kwant_id),
    registry_name: name,
    version: str(m.version) ?? '',
    title: str(m.title) ?? name,
    description: str(m.description) ?? '',
    reason: str(m.reason) ?? '',
    repository_url: safeLink(str(m.repository_url)),
    transport: m.transport === 'http' || m.transport === 'stdio' ? m.transport : null,
    url: str(m.url),
    command: str(m.command),
    args: strs(m.args),
    package: str(m.package),
    runs_on_your_computer: m.runs_on_your_computer === true,
    fixed_env: strs(m.fixed_env),
    fixed_headers: strs(m.fixed_headers),
    inputs: Array.isArray(m.inputs) ? m.inputs.map(parseInput).filter((x): x is SuggestionInput => !!x) : [],
    status: STATUSES.includes(m.status as SuggestionStatus) ? (m.status as SuggestionStatus) : 'pending',
    server_id: str(m.server_id),
    last_error: str(m.last_error),
  };
}

/** Registry text is untrusted: only http(s) links become clickable. */
export function safeLink(url: string | null): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' || u.protocol === 'http:' ? u.href : null;
  } catch {
    return null;
  }
}

/** Card state: the live record (newer card meta) wins over the message's meta. */
export function suggestionView(meta: SuggestionMeta, live?: Suggestion | null): SuggestionMeta {
  if (!live || live.id !== meta.suggestion_id) return meta;
  const card = parseSuggestionMeta(live.card);
  const base = card ?? meta;
  return { ...base, status: live.status, server_id: live.server_id, last_error: live.last_error };
}

const RANK: Record<SuggestionStatus, number> = { pending: 0, installing: 1, installed: 2, dismissed: 2 };

/** Keeps the latest record per suggestion; a finished one never goes back to pending. */
export function applySuggestion(map: Record<string, Suggestion>, s: Suggestion): Record<string, Suggestion> {
  const prev = map[s.id];
  if (prev && RANK[prev.status] === 2 && RANK[s.status] < 2) return map;
  return { ...map, [s.id]: s };
}

/** What installing runs, in one line: the address, or the exact command. */
export function runsLine(m: Pick<SuggestionMeta, 'url' | 'command' | 'args'>): string {
  if (m.url) return m.url;
  return m.command ? joinCommandLine([m.command, ...m.args]) : '';
}

/** The single `{name}` in a template like `Bearer {api_key}`, else null. */
export function templateSlot(template: string | null): string | null {
  if (!template) return null;
  const all = template.match(/\{[^{}]+\}/g);
  return all && all.length === 1 ? all[0].slice(1, -1) : null;
}

const WORDS: Record<string, string> = { api: 'API', id: 'ID', url: 'URL', pat: 'PAT', oauth: 'OAuth' };

/** `api_key` → `API key`, `accessToken` → `Access token`. */
export function humanizeSlot(slot: string): string {
  const words = slot
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .split(/[\s_-]+/)
    .filter(Boolean)
    .map((w) => WORDS[w.toLowerCase()] ?? w.toLowerCase());
  const s = words.join(' ');
  return s ? s.charAt(0).toUpperCase() + s.slice(1) : slot;
}

/** How a field is labelled: a template asks for its one slot (the key), else the variable's name. */
export function inputLabel(i: SuggestionInput): { label: string; code: boolean; note: string | null } {
  const slot = templateSlot(i.template);
  if (slot) {
    const label = humanizeSlot(slot);
    return { label, code: false, note: `Sent as the ${i.name} ${i.kind === 'header' ? 'header' : 'setting'}: ${i.template!.replace(`{${slot}}`, `<${label.toLowerCase()}>`)}` };
  }
  return { label: i.name, code: true, note: null };
}

export const inputKey = (i: Pick<SuggestionInput, 'kind' | 'name'>) => `${i.kind}:${i.name}`;

/** The install body: trimmed, blank optional fields left out. Missing required names → `missing`. */
export function installPayload(
  inputs: SuggestionInput[],
  values: Record<string, string>,
): { body: { env?: Record<string, string>; headers?: Record<string, string> }; missing: string[] } {
  const env: Record<string, string> = {};
  const headers: Record<string, string> = {};
  const missing: string[] = [];
  for (const i of inputs) {
    const v = (values[inputKey(i)] ?? '').trim();
    if (!v) {
      if (i.required) missing.push(inputLabel(i).label);
      continue;
    }
    (i.kind === 'env' ? env : headers)[i.name] = v;
  }
  const body: { env?: Record<string, string>; headers?: Record<string, string> } = {};
  if (Object.keys(env).length) body.env = env;
  if (Object.keys(headers).length) body.headers = headers;
  return { body, missing };
}

/** A failed install in plain words: a refused key says so instead of the server's transport error. */
export function failureText(raw: string, asksForKeys: boolean): string {
  if (asksForKeys && /\b(401|403)\b|unauthori[sz]ed|forbidden/i.test(raw)) return 'The server turned down the key. Check it and try again.';
  return `${raw.trim().replace(/\.?$/, '.')}${asksForKeys ? ' Check what you typed and try again.' : ' You can try again.'}`;
}

const enc = encodeURIComponent;

export const suggestionsApi = {
  list: (status?: SuggestionStatus) => request<Suggestion[]>('GET', `/api/mcp/suggestions${status ? `?status=${status}` : ''}`),
  install: (id: string, body: { env?: Record<string, string>; headers?: Record<string, string> }) =>
    request<InstallResult>('POST', `/api/mcp/suggestions/${enc(id)}/install`, body),
  dismiss: (id: string) => request<Suggestion>('POST', `/api/mcp/suggestions/${enc(id)}/dismiss`),
  /** Re-checks an installed server, e.g. after its sign-in window closes. */
  testServer: (serverId: string) => request<McpTestResult>('POST', `/api/mcp/servers/${enc(serverId)}/test`),
};
