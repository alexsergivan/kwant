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
}

/** What the API returns in place of every stored secret value. */
export const SECRET_MASK = '********';

export interface SecretRow {
  key: string;
  value: string;
  /** Already stored on the server: a blank value keeps it. */
  saved: boolean;
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

export interface McpPreset {
  id: string;
  label: string;
  blurb: string;
  form: McpFormState;
}

/** Ready-made examples offered on the empty screen and in the form. */
export const MCP_PRESETS: McpPreset[] = [
  {
    id: 'files',
    label: 'Files in a folder',
    blurb: 'Read and write files in one folder you choose. Needs Node.js.',
    form: {
      ...emptyMcpForm('stdio'),
      name: 'files',
      commandLine: 'npx -y @modelcontextprotocol/server-filesystem /home/me/projects',
    },
  },
  {
    id: 'fetch',
    label: 'Read web pages',
    blurb: 'Fetch a URL and turn the page into readable text. Needs uv.',
    form: { ...emptyMcpForm('stdio'), name: 'fetch', commandLine: 'uvx mcp-server-fetch' },
  },
  {
    id: 'github',
    label: 'GitHub',
    blurb: 'Issues, pull requests and code search. Needs a GitHub token.',
    form: {
      ...emptyMcpForm('stdio'),
      name: 'github',
      commandLine: 'npx -y @modelcontextprotocol/server-github',
      env: [{ key: 'GITHUB_PERSONAL_ACCESS_TOKEN', value: '', saved: false }],
    },
  },
];

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
  if (name.toLowerCase() === 'kwnt') return '"kwnt" is reserved, pick another name.';
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
  if (model === 'local_ollama') return { supported: true, note: null };
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
}

export const toStoredStatus = (result: Pick<McpTestResult, 'ok' | 'tools'>, at: string): StoredStatus => ({
  ok: result.ok === true,
  tools: (result.tools ?? []).map((t) => t.name).filter((n): n is string => typeof n === 'string'),
  at,
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
