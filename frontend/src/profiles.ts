// LLM profiles: one account per engine (a work and a personal Claude, say). Keys are write-only.

export type Engine = 'claude' | 'codex' | 'grok' | 'custom';
export type ProfileKind = 'system' | 'cli_home' | 'api_key' | 'custom_api';

/** How a Custom API account's server talks: OpenAI chat completions or Anthropic Messages. */
export type CustomProtocol = 'openai-chat' | 'anthropic-messages';
export type CustomStatus = 'ready' | 'unreachable' | 'key_rejected' | 'error';

/** A Custom API account's server, as the server last checked it (GET {base}/models). */
export interface CustomServer {
  protocol: CustomProtocol;
  base_url: string;
  /** The model it runs when the bot doesn't pin one; null lets the server's list decide. */
  model: string | null;
  /** null: not checked yet. */
  status: CustomStatus | null;
  /** "Connected · 12 models", or the server's own words for what went wrong. */
  detail: string | null;
  models: Array<{ id: string; label: string; tier: 'fast' | 'balanced' | 'deep' | null; default: boolean }>;
  checked_at: string | null;
  /** MCP tools reach bots on it (OpenAI format only). */
  supports_tools: boolean;
}

export interface Profile {
  id: string;
  engine: Engine;
  name: string;
  kind: ProfileKind;
  enabled: boolean;
  is_default: boolean;
  priority: number;
  home_dir: string | null;
  has_key: boolean;
  /** null: couldn't tell (CLI missing). */
  signed_in: boolean | null;
  account_label: string | null;
  limited_until: string | null;
  last_error: 'quota' | 'auth' | null;
  /** Custom API accounts only. */
  custom?: CustomServer | null;
}

export type LoginStatus = 'starting' | 'waiting' | 'succeeded' | 'failed' | 'expired' | 'cancelled';

export interface Login {
  flow_id: string;
  profile_id: string;
  status: LoginStatus;
  url: string | null;
  user_code: string | null;
  needs_code: boolean;
  error: string | null;
  expires_at: string;
}

export const ENGINES: Array<{ id: Engine; label: string }> = [
  { id: 'claude', label: 'Claude' },
  { id: 'codex', label: 'Codex (OpenAI)' },
  { id: 'grok', label: 'Grok (xAI)' },
  { id: 'custom', label: 'Custom API' },
];

export const KIND_LABELS: Record<ProfileKind, string> = {
  system: 'This machine',
  cli_home: 'Subscription',
  api_key: 'API key',
  custom_api: 'Custom API',
};

export const PROTOCOLS: Array<{ id: CustomProtocol; label: string; hint: string }> = [
  { id: 'openai-chat', label: 'OpenAI-compatible', hint: 'Bots can use tools' },
  { id: 'anthropic-messages', label: 'Anthropic-compatible', hint: 'No tools' },
];
export const protocolLabel = (p: CustomProtocol | string) => PROTOCOLS.find((x) => x.id === p)?.label ?? p;

/** Well-known servers, to fill the form. Agent Maestro serves GitHub Copilot's models from VS Code on the Mac. */
export const CUSTOM_PRESETS: Array<{ id: string; label: string; name: string; urls: Partial<Record<CustomProtocol, string>>; needsKey: boolean; note: string }> = [
  {
    id: 'maestro',
    label: 'Agent Maestro',
    name: 'Copilot',
    urls: { 'openai-chat': 'http://localhost:23333/api/openai/v1', 'anthropic-messages': 'http://localhost:23333/api/anthropic' },
    needsKey: false,
    note: 'GitHub Copilot models from VS Code. Keep VS Code open with Agent Maestro running.',
  },
  { id: 'openrouter', label: 'OpenRouter', name: 'OpenRouter', urls: { 'openai-chat': 'https://openrouter.ai/api/v1' }, needsKey: true, note: 'Needs an OpenRouter API key.' },
  { id: 'lmstudio', label: 'LM Studio', name: 'LM Studio', urls: { 'openai-chat': 'http://localhost:1234/v1' }, needsKey: false, note: 'Start LM Studio’s local server first.' },
];

/** The engine a kwant's model runs on, like the server's `Engine::from_model`; null for auto and Ollama. */
export function engineOfModel(model: string | null | undefined): Engine | null {
  if (!model) return null;
  if (model === 'custom' || model.startsWith('custom:') || model.startsWith('custom/')) return 'custom';
  if (model.includes('claude')) return 'claude';
  if (model.includes('grok')) return 'grok';
  if (model.includes('codex') || model.includes('gpt') || model.includes('openai')) return 'codex';
  return null;
}

/** An engine's profiles in the order turns try them: default first, then by priority. */
export function profilesForEngine(profiles: Profile[], engine: Engine | null): Profile[] {
  if (!engine) return [];
  return profiles
    .filter((p) => p.engine === engine)
    .sort((a, b) => Number(b.is_default) - Number(a.is_default) || a.priority - b.priority || a.name.localeCompare(b.name));
}

export const isLoginDone = (s: LoginStatus) => s !== 'starting' && s !== 'waiting';

/** Only https links are opened, so a bad value can't run script in this page. */
export function safeExternalUrl(url: string | null): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' ? u.href : null;
  } catch {
    return null;
  }
}

/** A limit still in force, or null once it has passed. */
export function activeLimit(limitedUntil: string | null, now = Date.now()): Date | null {
  if (!limitedUntil) return null;
  const at = new Date(limitedUntil);
  return Number.isNaN(at.getTime()) || at.getTime() <= now ? null : at;
}

/** "in 45 min", "in 3 h", or a date when it's further away. */
export function formatUntil(at: Date, now = Date.now()): string {
  const mins = Math.max(1, Math.round((at.getTime() - now) / 60000));
  if (mins < 60) return `in ${mins} min`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `in ${hours} h`;
  return at.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/** What the server's account label says when Claude Code is set up for another provider, not a Claude login. */
const THIRD_PARTY_LABELS = new Set(['Amazon Bedrock', 'Google Vertex AI', 'Microsoft Foundry', 'a third-party provider', 'third_party']);
export const isThirdPartyLabel = (label: string | null | undefined) => !!label && THIRD_PARTY_LABELS.has(label);
const providerName = (label: string) => (label === 'third_party' ? 'a third-party provider' : label);

export type AccountTone = 'ok' | 'warn' | 'error' | 'muted' | 'info';

/** An account's state in the words every screen uses: a one-word chip, and a line saying who or why. */
export interface AccountState {
  label: 'Ready' | 'Needs sign-in' | 'Off' | 'Out of usage' | 'Other provider' | 'No key' | 'Unknown' | 'Can’t connect' | 'Key rejected' | 'Error' | 'Not checked';
  tone: AccountTone;
  /** The email or provider when it's ready, else what to do; null when there's nothing to add. */
  detail: string | null;
}

/** The state of an added account (This machine's comes from its engine's check; see accountStatus). */
export function accountState(p: Profile, now = Date.now()): AccountState {
  if (!p.enabled) return { label: 'Off', tone: 'muted', detail: 'Bots skip this account' };
  const limit = activeLimit(p.limited_until, now);
  if (limit) return { label: 'Out of usage', tone: 'warn', detail: `Back ${formatUntil(limit, now)}` };
  if (p.last_error === 'auth') return { label: 'Needs sign-in', tone: 'error', detail: 'Its login stopped working. Sign in again.' };
  if (p.kind === 'custom_api') return customState(p.custom ?? null);
  if (p.kind === 'api_key') return p.has_key ? { label: 'Ready', tone: 'ok', detail: 'Key saved' } : { label: 'No key', tone: 'warn', detail: 'Add a key to use it' };
  if (p.signed_in === true) {
    if (isThirdPartyLabel(p.account_label)) {
      const via = providerName(p.account_label!);
      return p.kind === 'system'
        ? { label: 'Other provider', tone: 'info', detail: `Runs through ${via}, not a Claude login` }
        : { label: 'Other provider', tone: 'warn', detail: `Runs through ${via} instead of its own Claude login` };
    }
    return { label: 'Ready', tone: 'ok', detail: p.account_label };
  }
  if (p.signed_in === false) return { label: 'Needs sign-in', tone: 'warn', detail: null };
  return { label: 'Unknown', tone: 'muted', detail: p.kind === 'system' ? 'Uses this computer’s login' : 'Couldn’t check the sign-in (is the CLI installed?)' };
}

/** A Custom API account's check, with the server's own words when it failed. */
function customState(c: CustomServer | null): AccountState {
  switch (c?.status) {
    case 'ready':
      return { label: 'Ready', tone: 'ok', detail: c.detail ?? 'Connected' };
    case 'unreachable':
      return { label: 'Can’t connect', tone: 'error', detail: c.detail ?? 'Nothing is answering at its address.' };
    case 'key_rejected':
      return { label: 'Key rejected', tone: 'error', detail: c.detail ?? 'The server refused the key. Replace it.' };
    case 'error':
      return { label: 'Error', tone: 'error', detail: c.detail ?? 'The check failed.' };
    default:
      return { label: 'Not checked', tone: 'muted', detail: 'Press Check again' };
  }
}

/** Short status for a profile, e.g. "Ready · me@x.io" or "Out of usage · Back in 45 min", for pickers. */
export function profileStatus(p: Profile, now = Date.now()): { tone: AccountTone; text: string } {
  const s = accountState(p, now);
  // The email tells two ready accounts apart; Off and Needs sign-in say enough on their own.
  const withDetail = s.label === 'Out of usage' || s.label === 'Other provider' || (s.label === 'Ready' && p.kind !== 'api_key' && p.kind !== 'custom_api');
  return { tone: s.tone, text: withDetail && s.detail ? `${s.label} · ${s.detail}` : s.label };
}

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let message = `Request failed (HTTP ${res.status}).`;
    try {
      const data = await res.json();
      if (data && typeof data.error === 'string' && data.error) message = data.error;
    } catch {
      // no JSON body
    }
    if (res.status === 409 && message.startsWith('Request failed')) message = 'A profile with that name or folder already exists.';
    throw new Error(message);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

const enc = encodeURIComponent;

export const profilesApi = {
  list: () => request<Profile[]>('GET', '/api/profiles'),
  create: (body: {
    engine: Engine;
    name: string;
    kind: 'cli_home' | 'api_key' | 'custom_api';
    home_dir?: string;
    priority?: number;
    enabled?: boolean;
    is_default?: boolean;
    api_key?: string;
    protocol?: CustomProtocol;
    base_url?: string;
    model?: string;
  }) => request<Profile>('POST', '/api/profiles', body),
  /** `protocol`, `base_url` and `model` ("" clears it) are for Custom API accounts, and re-check the server. */
  update: (
    id: string,
    body: { name?: string; priority?: number; enabled?: boolean; is_default?: boolean; protocol?: CustomProtocol; base_url?: string; model?: string },
  ) => request<Profile>('PATCH', `/api/profiles/${enc(id)}`, body),
  /** Checks a Custom API account's server again now. */
  check: (id: string) => request<Profile>('POST', `/api/profiles/${enc(id)}/check`),
  remove: (id: string) => request<void>('DELETE', `/api/profiles/${enc(id)}`),
  setKey: (id: string, apiKey: string) => request<void>('PUT', `/api/profiles/${enc(id)}/key`, { api_key: apiKey }),
  removeKey: (id: string) => request<void>('DELETE', `/api/profiles/${enc(id)}/key`),
  startLogin: (id: string) => request<Login>('POST', `/api/profiles/${enc(id)}/login`),
  getLogin: (id: string, flow: string) => request<Login>('GET', `/api/profiles/${enc(id)}/login/${enc(flow)}`),
  submitCode: (id: string, flow: string, code: string) =>
    request<Login>('POST', `/api/profiles/${enc(id)}/login/${enc(flow)}/code`, { code }),
  cancelLogin: (id: string, flow: string) => request<void>('DELETE', `/api/profiles/${enc(id)}/login/${enc(flow)}`),
  logout: (id: string) => request<void>('POST', `/api/profiles/${enc(id)}/logout`),
};

export interface ProfileSwitchedEvent {
  kwant_id: string;
  from_profile_id: string;
  from_name: string;
  to_profile_id: string;
  to_name: string;
  class: 'quota' | 'auth' | string;
  retry_at?: string;
}

/** Toast text for a `profile_switched` event. */
export function describeProfileSwitch(e: ProfileSwitchedEvent, kwantName: string | undefined, now = Date.now()) {
  const who = kwantName || 'A bot';
  const limit = activeLimit(e.retry_at ?? null, now);
  const why =
    e.class === 'auth'
      ? `${e.from_name} is signed out.`
      : limit
        ? `${e.from_name} is out of usage until it resets ${formatUntil(limit, now)}.`
        : `${e.from_name} is out of usage.`;
  return { title: `${who} switched to ${e.to_name}`, detail: why };
}

// ---- Accounts on the "Engines & accounts" cards ----

type EngineCardId = 'claude' | 'codex' | 'grok' | 'anthropic_api' | 'openai_api' | 'xai_api' | 'ollama' | 'custom_api';
type KeyedEngine = Exclude<Engine, 'custom'>;
const API_CARD: Record<KeyedEngine, EngineCardId> = { claude: 'anthropic_api', codex: 'openai_api', grok: 'xai_api' };

/** The engine card an account sits on: API-key accounts on the API engine, Custom API on its own, the rest on the CLI. */
export const cardOf = (p: Pick<Profile, 'engine' | 'kind'>): EngineCardId =>
  p.engine === 'custom' ? 'custom_api' : p.kind === 'api_key' ? API_CARD[p.engine] : p.engine;

export type NewAccountSpec = { engine: Engine; kind: 'cli_home' | 'api_key' | 'custom_api' };

/** The account family and kind a new account on this card gets; null where accounts can't be added. */
export function newAccountFor(card: string): NewAccountSpec | null {
  if (card === 'claude' || card === 'codex' || card === 'grok') return { engine: card, kind: 'cli_home' };
  if (card === 'custom_api') return { engine: 'custom', kind: 'custom_api' };
  const api = (Object.keys(API_CARD) as KeyedEngine[]).find((e) => API_CARD[e] === card);
  return api ? { engine: api, kind: 'api_key' } : null;
}

/** The space's Custom API accounts that answered their last check and are on, default first. */
export const readyCustomAccounts = (profiles: Profile[] | null | undefined): Profile[] =>
  profilesForEngine(profiles ?? [], 'custom').filter((p) => p.enabled && p.custom?.status === 'ready');

/** A card's accounts: "This machine" first, then the order turns try them (default, then priority). */
export function accountsForCard(profiles: Profile[], card: string): Profile[] {
  return profiles
    .filter((p) => cardOf(p) === card)
    .sort(
      (a, b) =>
        Number(b.kind === 'system') - Number(a.kind === 'system') ||
        Number(b.is_default) - Number(a.is_default) ||
        a.priority - b.priority ||
        a.name.localeCompare(b.name),
    );
}

/** What turn routing asks of an account (Availability::usable on the server): not out of usage, not an
 * API-key account without its key, and not signed out on its last check. Not checked yet still counts,
 * so the server tries it; a refused turn's "auth" error doesn't matter until a check says signed out. */
export const accountRoutable = (p: Profile, now = Date.now()) =>
  !activeLimit(p.limited_until, now) && !(p.kind === 'api_key' && !p.has_key) && p.signed_in !== false && (p.kind !== 'custom_api' || !p.custom?.status || p.custom.status === 'ready');

/** Could run a turn as last checked: on, signed in (or keyed), not out of usage. */
export const accountUsable = (p: Profile, now = Date.now()) =>
  p.enabled &&
  !activeLimit(p.limited_until, now) &&
  p.last_error !== 'auth' &&
  (p.kind === 'custom_api' ? p.custom?.status === 'ready' : p.kind === 'api_key' ? p.has_key : p.signed_in === true);
