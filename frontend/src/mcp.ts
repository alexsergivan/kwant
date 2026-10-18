// MCP server helpers shared by the servers screen, the per-kwant picker and chat.
// Contract: docs/mcp-api.md

export type McpTransport = 'stdio' | 'http';

export interface McpServer {
  id: string;
  name: string;
  transport: McpTransport;
  command: string | null;
  args: string[];
  url: string | null;
  env: Record<string, string>;
  headers: Record<string, string>;
  enabled: boolean;
  timeout_secs: number;
  created_at: string | null;
  /** Sign-in state for URL servers; never carries tokens. */
  oauth?: { connected: boolean; expires_at: string | null };
  /** Kwants that switched this server off, sorted by name. Empty: every kwant gets it. */
  disabled_for_kwants?: string[];
}

export interface McpTool {
  name: string;
  description?: string;
  input_schema?: unknown;
}

export interface McpTestResult {
  ok: boolean;
  server_info?: { name?: string; version?: string };
  duration_ms?: number;
  tools?: McpTool[];
  error?: string;
  stderr_tail?: string;
  /** The server answered 401 and kwnt has no usable sign-in for it. */
  auth_required?: boolean;
}

/** What the API returns in place of every stored secret value. */
export const SECRET_MASK = '********';

export interface SecretRow {
  key: string;
  value: string;
  /** Already stored on the server: a blank value keeps it. */
  saved: boolean;
  /** Placeholder for an empty value, like the format a preset expects. */
  hint?: string;
  /** Set by presets: the server won't work until this has a value. */
  required?: boolean;
}

export interface McpFormState {
  name: string;
  transport: McpTransport;
  /** Command and arguments as one line, the way READMEs show them. */
  commandLine: string;
  url: string;
  env: SecretRow[];
  headers: SecretRow[];
  enabled: boolean;
  timeout_secs: number;
}

export const emptyMcpForm = (transport: McpTransport = 'stdio'): McpFormState => ({
  name: '',
  transport,
  commandLine: '',
  url: '',
  env: [],
  headers: [],
  enabled: true,
  timeout_secs: 30,
});

export type McpPresetCategory = 'files' | 'web' | 'data' | 'productivity' | 'devtools';

/** Catalog groups, in display order. */
export const MCP_PRESET_CATEGORIES: Array<{ id: McpPresetCategory; label: string }> = [
  { id: 'files', label: 'Files & code' },
  { id: 'web', label: 'Web' },
  { id: 'data', label: 'Data' },
  { id: 'productivity', label: 'Productivity' },
  { id: 'devtools', label: 'Dev tools' },
];

/** What a preset needs before it works, shown as small tags on its card. */
export type McpPresetNeed = 'Node.js' | 'uv' | 'API key' | 'Token' | 'Sign-in';

export interface McpPreset {
  id: string;
  label: string;
  blurb: string;
  category: McpPresetCategory;
  needs: McpPresetNeed[];
  /** Where the setup comes from: the vendor's docs or the package page. */
  source: string;
  form: McpFormState;
}

/** A preset's secret: name, placeholder, and whether the server needs it to work. */
type PresetSecret = [key: string, hint: string, required?: boolean];

const presetRows = (secrets: PresetSecret[]): SecretRow[] =>
  secrets.map(([key, hint, required]) => ({ key, value: '', saved: false, hint, ...(required ? { required: true } : {}) }));

const stdioPreset = (name: string, commandLine: string, env: PresetSecret[] = []): McpFormState => ({
  ...emptyMcpForm('stdio'),
  name,
  commandLine,
  env: presetRows(env),
});

const httpPreset = (name: string, url: string, headers: PresetSecret[] = []): McpFormState => ({
  ...emptyMcpForm('http'),
  name,
  url,
  headers: presetRows(headers),
});

/**
 * Well-known servers offered as one-click starting points. Built in rather than fetched from a
 * registry, so the list works offline and on a VPS. Each entry was checked against its package
 * or the vendor's docs on 2026-10-17; reference servers that were deprecated were swapped for
 * the vendor's own server. URL servers marked Sign-in support automatic client registration,
 * which kwnt's Connect flow needs.
 */
export const MCP_PRESETS: McpPreset[] = [
  {
    // npm @modelcontextprotocol/server-filesystem 1.0.0
    id: 'files',
    label: 'Files in a folder',
    blurb: 'Read and write files in one folder you choose.',
    category: 'files',
    needs: ['Node.js'],
    source: 'https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem',
    form: stdioPreset('files', 'npx -y @modelcontextprotocol/server-filesystem /home/me/projects'),
  },
  {
    // PyPI mcp-server-git 2026.10.10
    id: 'git',
    label: 'Git',
    blurb: 'Status, diffs, history and commits in one local repository.',
    category: 'files',
    needs: ['uv'],
    source: 'https://github.com/modelcontextprotocol/servers/tree/main/src/git',
    form: stdioPreset('git', 'uvx mcp-server-git --repository /home/me/projects/my-repo'),
  },
  {
    // GitHub's official remote server. Its sign-in has no automatic client registration, so it
    // takes a personal access token. Replaces the deprecated @modelcontextprotocol/server-github.
    id: 'github',
    label: 'GitHub',
    blurb: 'Issues, pull requests, code search and Actions. Paste “Bearer” and your token.',
    category: 'files',
    needs: ['Token'],
    source: 'https://github.com/github/github-mcp-server',
    form: httpPreset('github', 'https://api.githubcopilot.com/mcp/', [['Authorization', 'Bearer ghp_…', true]]),
  },
  {
    // PyPI mcp-server-fetch 2026.10.10
    id: 'fetch',
    label: 'Read web pages',
    blurb: 'Fetch a URL and turn the page into readable text.',
    category: 'web',
    needs: ['uv'],
    source: 'https://github.com/modelcontextprotocol/servers/tree/main/src/fetch',
    form: stdioPreset('fetch', 'uvx mcp-server-fetch'),
  },
  {
    // npm @playwright/mcp 0.0.83. Headless so it also runs on a server without a screen.
    id: 'playwright',
    label: 'Playwright browser',
    blurb: 'Drive a real browser: open pages, click, fill forms, take screenshots.',
    category: 'web',
    needs: ['Node.js'],
    source: 'https://github.com/microsoft/playwright-mcp',
    form: stdioPreset('playwright', 'npx -y @playwright/mcp@latest --headless'),
  },
  {
    // npm @brave/brave-search-mcp-server 2.1.4. Replaces the deprecated @modelcontextprotocol/server-brave-search.
    id: 'brave-search',
    label: 'Brave Search',
    blurb: 'Web, news and image search through the Brave Search API.',
    category: 'web',
    needs: ['Node.js', 'API key'],
    source: 'https://github.com/brave/brave-search-mcp-server',
    form: stdioPreset('brave-search', 'npx -y @brave/brave-search-mcp-server --transport stdio', [['BRAVE_API_KEY', 'From api-dashboard.search.brave.com', true]]),
  },
  {
    // Upstash's hosted server; works without a key, a key (Authorization: Bearer …) raises limits.
    id: 'context7',
    label: 'Context7 docs',
    blurb: 'Up-to-date docs and code examples for libraries and frameworks.',
    category: 'web',
    needs: [],
    source: 'https://github.com/upstash/context7',
    form: httpPreset('context7', 'https://mcp.context7.com/mcp'),
  },
  {
    // PyPI postgres-mcp 0.3.0 (Postgres MCP Pro). Replaces the deprecated @modelcontextprotocol/server-postgres.
    // Restricted mode keeps it read-only.
    id: 'postgres',
    label: 'PostgreSQL',
    blurb: 'Read-only queries, schema browsing and index advice for a Postgres database.',
    category: 'data',
    needs: ['uv'],
    source: 'https://github.com/crystaldba/postgres-mcp',
    form: stdioPreset('postgres', 'uvx postgres-mcp --access-mode=restricted', [['DATABASE_URI', 'postgresql://user:password@host:5432/db', true]]),
  },
  {
    // Supabase's hosted server with sign-in.
    id: 'supabase',
    label: 'Supabase',
    blurb: 'Projects, tables, SQL, logs and edge functions. Sign in after adding.',
    category: 'data',
    needs: ['Sign-in'],
    source: 'https://supabase.com/docs/guides/getting-started/mcp',
    form: httpPreset('supabase', 'https://mcp.supabase.com/mcp'),
  },
  {
    // npm @modelcontextprotocol/server-memory 1.0.0
    id: 'memory',
    label: 'Memory',
    blurb: 'A small knowledge graph bots can save facts to and recall later.',
    category: 'data',
    needs: ['Node.js'],
    source: 'https://github.com/modelcontextprotocol/servers/tree/main/src/memory',
    form: stdioPreset('memory', 'npx -y @modelcontextprotocol/server-memory'),
  },
  {
    // Notion's hosted server with sign-in.
    id: 'notion',
    label: 'Notion',
    blurb: 'Search, read and edit pages and databases. Sign in after adding.',
    category: 'productivity',
    needs: ['Sign-in'],
    source: 'https://developers.notion.com/docs/mcp',
    form: httpPreset('notion', 'https://mcp.notion.com/mcp'),
  },
  {
    // Linear's hosted server with sign-in.
    id: 'linear',
    label: 'Linear',
    blurb: 'Find, create and update issues, projects and comments. Sign in after adding.',
    category: 'productivity',
    needs: ['Sign-in'],
    source: 'https://linear.app/docs/mcp',
    form: httpPreset('linear', 'https://mcp.linear.app/mcp'),
  },
  {
    // Atlassian's hosted Rovo MCP server with sign-in.
    id: 'atlassian',
    label: 'Jira & Confluence',
    blurb: 'Search and update Jira issues and Confluence pages. Sign in after adding.',
    category: 'productivity',
    needs: ['Sign-in'],
    source: 'https://github.com/atlassian/atlassian-mcp-server',
    form: httpPreset('atlassian', 'https://mcp.atlassian.com/v1/mcp'),
  },
  {
    // Sentry's hosted server with sign-in.
    id: 'sentry',
    label: 'Sentry',
    blurb: 'Look into errors, issues and releases. Sign in after adding.',
    category: 'devtools',
    needs: ['Sign-in'],
    source: 'https://github.com/getsentry/sentry-mcp',
    form: httpPreset('sentry', 'https://mcp.sentry.dev/mcp'),
  },
  {
    // npm @modelcontextprotocol/server-sequential-thinking 1.0.0
    id: 'sequential-thinking',
    label: 'Sequential thinking',
    blurb: 'Helps a bot plan step by step and revise its plan on hard problems.',
    category: 'devtools',
    needs: ['Node.js'],
    source: 'https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking',
    form: stdioPreset('sequential-thinking', 'npx -y @modelcontextprotocol/server-sequential-thinking'),
  },
];

/** Presets matching a search, by name, blurb, group or what they need. A blank query keeps all. */
export const searchPresets = (query: string, presets: McpPreset[] = MCP_PRESETS): McpPreset[] => {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return presets;
  const label = (c: McpPresetCategory) => MCP_PRESET_CATEGORIES.find((x) => x.id === c)?.label ?? c;
  return presets.filter((p) => {
    const text = [p.id, p.label, p.blurb, label(p.category), ...p.needs].join(' ').toLowerCase();
    return words.every((w) => text.includes(w));
  });
};

/** Presets grouped for display, in category order, leaving out empty groups. */
export const groupPresets = (presets: McpPreset[]) =>
  MCP_PRESET_CATEGORIES.map((c) => ({ ...c, presets: presets.filter((p) => p.category === c.id) })).filter((g) => g.presets.length > 0);

/** Splits a command line into words, honouring '…' and "…" quotes and backslash escapes. */
export const splitCommandLine = (line: string): string[] => {
  const words: string[] = [];
  let current = '';
  let inWord = false;
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (quote) {
      if (ch === quote) quote = null;
      else if (ch === '\\' && quote === '"' && i + 1 < line.length) current += line[++i];
      else current += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
      inWord = true;
    } else if (ch === '\\' && i + 1 < line.length) {
      current += line[++i];
      inWord = true;
    } else if (/\s/.test(ch)) {
      if (inWord) words.push(current);
      current = '';
      inWord = false;
    } else {
      current += ch;
      inWord = true;
    }
  }
  if (inWord) words.push(current);
  return words;
};

const quoteWord = (word: string) =>
  word === '' || /[\s'"\\]/.test(word) ? `"${word.replace(/(["\\])/g, '\\$1')}"` : word;

/** Inverse of splitCommandLine. */
export const joinCommandLine = (words: string[]): string => words.map(quoteWord).join(' ');

const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,47}$/;
const ENV_KEY_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Turns any label into a valid server name ("My Files!" → "my-files"). */
export const toServerName = (label: string): string =>
  label
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/_{2,}/g, '_')
    .replace(/^[-_]+|[-_]+$/g, '')
    .slice(0, 48);

/** Returns a friendly message for the first problem in the form, or null when it can be saved. */
export const validateMcpForm = (form: McpFormState): string | null => {
  const name = form.name.trim();
  if (!name) return 'Give the server a short name.';
  if (!NAME_RE.test(name) || name.includes('__'))
    return 'Use letters, numbers, - or _ for the name (no spaces, up to 48 characters).';
  if (name.toLowerCase() === 'kwnt') return 'That name is reserved for the built-in tools, pick another.';
  if (form.transport === 'stdio') {
    if (splitCommandLine(form.commandLine).length === 0) return 'Add the command that starts the server.';
    const bad = form.env.find((r) => r.key.trim() && !ENV_KEY_RE.test(r.key.trim()));
    if (bad) return `"${bad.key}" isn't a valid variable name (letters, numbers and _ only).`;
  } else {
    if (!/^https?:\/\/\S+$/.test(form.url.trim())) return 'The URL should start with http:// or https://.';
  }
  if (!Number.isInteger(form.timeout_secs) || form.timeout_secs < 1 || form.timeout_secs > 600)
    return 'Timeout should be between 1 and 600 seconds.';
  return null;
};

/**
 * Whether a row the server needs is still empty. A saved row is never blank: leaving it empty
 * keeps the stored value. "Bearer" or "Basic" with nothing after it counts as empty.
 */
export const isBlankRequired = (row: SecretRow): boolean =>
  !!row.required && !row.saved && /^((bearer|basic)\b)?$/i.test(row.value.trim());

/** Names of required keys in the form that are still empty, for the "needs a key" warning. Saving is still allowed. */
export const blankRequiredKeys = (form: McpFormState): string[] =>
  (form.transport === 'stdio' ? form.env : form.headers).filter(isBlankRequired).map((r) => r.key.trim() || 'key');

const rowsToMap = (rows: SecretRow[]): Record<string, string> => {
  const out: Record<string, string> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (!key) continue;
    // A saved secret left blank is sent back as the mask, which keeps the stored value.
    out[key] = row.saved && row.value === '' ? SECRET_MASK : row.value;
  }
  return out;
};

/** Request body for POST/PUT /api/mcp/servers. Rows that were removed are left out, which deletes them. */
export const formToPayload = (form: McpFormState) => {
  const base = { name: form.name.trim(), transport: form.transport, enabled: form.enabled, timeout_secs: form.timeout_secs };
  if (form.transport === 'stdio') {
    const [command, ...args] = splitCommandLine(form.commandLine);
    return { ...base, command, args, url: null, env: rowsToMap(form.env), headers: {} };
  }
  return { ...base, command: null, args: [], url: form.url.trim(), env: {}, headers: rowsToMap(form.headers) };
};

const mapToRows = (map: Record<string, string> | null | undefined): SecretRow[] =>
  Object.keys(map ?? {}).map((key) => ({ key, value: '', saved: true }));

export const serverToForm = (server: McpServer): McpFormState => ({
  name: server.name,
  transport: server.transport,
  commandLine: server.command ? joinCommandLine([server.command, ...(server.args ?? [])]) : '',
  url: server.url ?? '',
  env: mapToRows(server.env),
  headers: mapToRows(server.headers),
  enabled: server.enabled,
  timeout_secs: server.timeout_secs || 30,
});

/** One-line summary of where a server lives, for cards. */
export const serverTarget = (server: McpServer): string =>
  server.transport === 'stdio'
    ? joinCommandLine([server.command ?? '', ...(server.args ?? [])])
    : server.url ?? '';

/** Splits `mcp__<server>__<tool>`; null for anything else. */
export const parseMcpToolName = (name: string): { server: string; tool: string } | null => {
  const m = /^mcp__(.+?)__(.+)$/.exec(name ?? '');
  return m ? { server: m[1], tool: m[2] } : null;
};

/** `read_file` → `read file`, for display. */
export const humanizeToolName = (tool: string): string => tool.replace(/[_-]+/g, ' ').trim();

export interface RecordedToolCall {
  id?: string;
  name: string;
  server?: string;
  tool?: string;
  arguments?: unknown;
  status?: 'ok' | 'error' | 'denied' | 'allowed' | string;
  result?: string;
  duration_ms?: number;
}

/** Reads a message's `tool_calls` (a JSON string from the API, or already parsed). */
export const parseToolCalls = (raw: unknown): RecordedToolCall[] => {
  let value = raw;
  if (typeof raw === 'string') {
    if (!raw.trim()) return [];
    try {
      value = JSON.parse(raw);
    } catch {
      return [];
    }
  }
  if (!Array.isArray(value)) return [];
  return value
    .filter((c): c is RecordedToolCall => !!c && typeof c === 'object' && typeof (c as RecordedToolCall).name === 'string')
    .map((c) => {
      const parsed = parseMcpToolName(c.name);
      return { ...c, server: c.server ?? parsed?.server, tool: c.tool ?? parsed?.tool ?? c.name };
    });
};

/** Pretty-prints tool arguments; strings that hold JSON are unpacked first. */
export const formatArgs = (args: unknown): string => {
  if (args === undefined || args === null) return '';
  let value = args;
  if (typeof args === 'string') {
    try {
      value = JSON.parse(args);
    } catch {
      return args;
    }
  }
  if (value && typeof value === 'object' && Object.keys(value).length === 0) return '';
  return JSON.stringify(value, null, 2);
};

export interface EngineKeys {
  anthropic: boolean;
  openai: boolean;
  xai: boolean;
}

/**
 * Whether a kwant with this engine can use MCP tools, mirroring how kwnt picks a driver
 * (an API key in Settings wins over the local CLI). `note` is shown instead of pretending.
 */
export const mcpSupportFor = (preferredModel: string, keys: EngineKeys): { supported: boolean; note: string | null } => {
  const model = (preferredModel || 'auto').toLowerCase();
  if (model === 'local_ollama' || model.startsWith('ollama/')) return { supported: true, note: null };
  // A Custom API pin, before the name checks: its model ids often say gpt or claude.
  if (model === 'custom' || /^custom[:/]/.test(model)) {
    return { supported: true, note: 'Tools work through an OpenAI-compatible server. An Anthropic-compatible one runs without them.' };
  }
  if (model.includes('claude')) {
    return keys.anthropic
      ? { supported: false, note: 'With an Anthropic API key set, Claude runs through the Anthropic API, which can’t use MCP tools yet. Remove the key to use the Claude Code CLI instead.' }
      : { supported: true, note: null };
  }
  if (model.includes('grok')) {
    return keys.xai
      ? { supported: true, note: null }
      : { supported: false, note: 'The Grok CLI can’t use MCP tools yet. Add an xAI API key in Settings to use them with Grok.' };
  }
  if (model.includes('codex') || model.includes('gpt') || model.includes('openai')) {
    return keys.openai
      ? { supported: true, note: null }
      : { supported: false, note: 'The Codex CLI can’t use MCP tools yet. Add an OpenAI API key in Settings to use them with this engine.' };
  }
  return {
    supported: true,
    note: 'Laya picks the engine for each message. Tools work with Claude Code, Ollama and API keys, but not through the Codex or Grok CLIs.',
  };
};

/**
 * Last test result per server, kept in the browser so cards can show a status.
 * Only ok, tool names and time are kept: a server's error text can echo back
 * secrets, so the full error lives in memory for the open screen only.
 */
const STATUS_KEY = 'kwnt.mcp.lastTest';

export interface StoredStatus {
  ok: boolean;
  tools: string[];
  at: string;
  /** Only present (and true) when the last test asked for sign-in. */
  auth_required?: boolean;
}

export const toStoredStatus = (result: Pick<McpTestResult, 'ok' | 'tools' | 'auth_required'>, at: string): StoredStatus => ({
  ok: result.ok === true,
  tools: (result.tools ?? []).map((t) => t.name).filter((n): n is string => typeof n === 'string'),
  at,
  ...(result.ok !== true && result.auth_required === true ? { auth_required: true } : {}),
});

/** Keeps only the safe fields of each entry; drops anything malformed (and old `error` text). */
export const sanitizeStatuses = (raw: unknown): Record<string, StoredStatus> => {
  const out: Record<string, StoredStatus> = {};
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return out;
  for (const [id, entry] of Object.entries(raw as Record<string, unknown>)) {
    if (!entry || typeof entry !== 'object') continue;
    const e = entry as Record<string, unknown>;
    if (typeof e.ok !== 'boolean') continue;
    out[id] = {
      ok: e.ok,
      tools: Array.isArray(e.tools) ? e.tools.filter((n): n is string => typeof n === 'string') : [],
      at: typeof e.at === 'string' ? e.at : '',
      ...(e.ok === false && e.auth_required === true ? { auth_required: true } : {}),
    };
  }
  return out;
};

export const saveStatuses = (statuses: Record<string, StoredStatus>) => {
  try {
    localStorage.setItem(STATUS_KEY, JSON.stringify(sanitizeStatuses(statuses)));
  } catch {
    // storage full or blocked: status is a nicety
  }
};

export const loadStatuses = (): Record<string, StoredStatus> => {
  try {
    const stored = localStorage.getItem(STATUS_KEY);
    if (!stored) return {};
    const clean = sanitizeStatuses(JSON.parse(stored));
    // Rewrite older entries that still carry error text.
    if (JSON.stringify(clean) !== stored) saveStatuses(clean);
    return clean;
  } catch {
    try {
      localStorage.removeItem(STATUS_KEY);
    } catch {
      // blocked
    }
    return {};
  }
};

export type ServerStatusKind = 'testing' | 'paused' | 'ok' | 'needs_sign_in' | 'error' | 'untested';

/**
 * What a server card shows. The live result of a test on this screen wins over the stored one.
 * "Needs sign-in" comes only from kwnt saying so (a 401 with no usable token): a URL server
 * that is simply down still reads "Couldn't connect".
 */
export const serverStatusKind = (
  server: Pick<McpServer, 'enabled' | 'transport'>,
  status: Pick<StoredStatus, 'ok' | 'auth_required'> | undefined,
  testing = false,
): ServerStatusKind => {
  if (testing) return 'testing';
  if (!server.enabled) return 'paused';
  if (!status) return 'untested';
  if (status.ok) return 'ok';
  if (status.auth_required && server.transport === 'http') return 'needs_sign_in';
  return 'error';
};

/** Whether kwnt holds a sign-in for this URL server. */
export const isSignedIn = (server: Pick<McpServer, 'transport' | 'oauth'>): boolean =>
  server.transport === 'http' && server.oauth?.connected === true;

export interface OAuthResultMessage {
  server_id: string | null;
  ok: boolean;
  error?: string;
}

/**
 * Reads the message kwnt's sign-in callback page posts to this window. Anything from another
 * origin, of another type or malformed is ignored (null): only kwnt's own page may report a result.
 */
export const readOAuthMessage = (
  event: { origin: string; data: unknown },
  expectedOrigin: string,
): OAuthResultMessage | null => {
  if (event.origin !== expectedOrigin) return null;
  const d = event.data as Record<string, unknown> | null;
  if (!d || typeof d !== 'object' || d.type !== 'kwnt-mcp-oauth' || typeof d.ok !== 'boolean') return null;
  if (d.server_id !== null && typeof d.server_id !== 'string') return null;
  return {
    server_id: d.server_id as string | null,
    ok: d.ok,
    ...(typeof d.error === 'string' && d.error ? { error: d.error.slice(0, 300) } : {}),
  };
};

/**
 * Which kwants a server reaches, for its card. Servers are on for every kwant unless a kwant
 * switches one off. Ids of kwants this screen doesn't know about are left out of the names.
 */
export const kwantUsage = (
  server: Pick<McpServer, 'disabled_for_kwants'>,
  kwants: Array<{ id: string; name: string }>,
): { allKwants: boolean; offFor: string[] } => {
  const off = server.disabled_for_kwants ?? [];
  const names = new Map(kwants.map((k) => [k.id, k.name]));
  const offFor = off.map((id) => names.get(id)).filter((n): n is string => !!n);
  return { allKwants: offFor.length === 0, offFor };
};

/** A new kwant starts with every server on, the same as the backend default. */
export const defaultKwantServerIds = (servers: Array<Pick<McpServer, 'id'>>): string[] => servers.map((s) => s.id);
