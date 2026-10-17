// LLM profiles: one account per engine (a work and a personal Claude, say). Keys are write-only.

export type Engine = 'claude' | 'codex' | 'grok';
export type ProfileKind = 'system' | 'cli_home' | 'api_key';

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
];

export const KIND_LABELS: Record<ProfileKind, string> = {
  system: 'This machine',
  cli_home: 'Subscription',
  api_key: 'API key',
};

/** The engine a kwant's model runs on, like the server's `Engine::from_model`; null for auto and Ollama. */
export function engineOfModel(model: string | null | undefined): Engine | null {
  if (!model) return null;
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

/** Short status for a profile: what a person needs to know before relying on it. */
export function profileStatus(p: Profile, now = Date.now()): { tone: 'ok' | 'warn' | 'error' | 'muted'; text: string } {
  if (!p.enabled) return { tone: 'muted', text: 'Turned off' };
  const limit = activeLimit(p.limited_until, now);
  if (limit) return { tone: 'warn', text: `Out of usage · back ${formatUntil(limit, now)}` };
  if (p.last_error === 'auth') return { tone: 'error', text: 'Signed out · sign in again' };
  if (p.kind === 'api_key') return p.has_key ? { tone: 'ok', text: 'Key saved' } : { tone: 'error', text: 'No key yet' };
  if (p.signed_in === true) return { tone: 'ok', text: p.account_label ? `Signed in as ${p.account_label}` : 'Signed in' };
  if (p.signed_in === false) return { tone: 'error', text: 'Not signed in' };
  return { tone: 'muted', text: p.kind === 'system' ? 'Uses this machine’s login' : 'Sign-in unknown (CLI missing?)' };
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
    kind: 'cli_home' | 'api_key';
    home_dir?: string;
    priority?: number;
    enabled?: boolean;
    is_default?: boolean;
    api_key?: string;
  }) => request<Profile>('POST', '/api/profiles', body),
  update: (id: string, body: { name?: string; priority?: number; enabled?: boolean; is_default?: boolean }) =>
    request<Profile>('PATCH', `/api/profiles/${enc(id)}`, body),
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
  const who = kwantName || 'A kwant';
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

type EngineCardId = 'claude' | 'codex' | 'grok' | 'anthropic_api' | 'openai_api' | 'xai_api' | 'ollama';
const API_CARD: Record<Engine, EngineCardId> = { claude: 'anthropic_api', codex: 'openai_api', grok: 'xai_api' };

/** The engine card an account sits on: API-key accounts on the API engine, the rest on the CLI. */
export const cardOf = (p: Pick<Profile, 'engine' | 'kind'>): EngineCardId => (p.kind === 'api_key' ? API_CARD[p.engine] : p.engine);

/** The account family and kind a new account on this card gets; null where accounts can't be added. */
export function newAccountFor(card: string): { engine: Engine; kind: 'cli_home' | 'api_key' } | null {
  if (card === 'claude' || card === 'codex' || card === 'grok') return { engine: card, kind: 'cli_home' };
  const api = (Object.keys(API_CARD) as Engine[]).find((e) => API_CARD[e] === card);
  return api ? { engine: api, kind: 'api_key' } : null;
}

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

/** Could run a turn as last checked: on, signed in (or keyed), not out of usage. */
export const accountUsable = (p: Profile, now = Date.now()) =>
  p.enabled && !activeLimit(p.limited_until, now) && p.last_error !== 'auth' && (p.kind === 'api_key' ? p.has_key : p.signed_in === true);
