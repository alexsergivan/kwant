// LLM profiles: one account per engine (a work and a personal Claude, say). Keys are write-only.

export type Engine = 'claude' | 'codex' | 'grok' | 'custom';
import { dateClock, timeText } from './clock';
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
  /** How much of its 5-hour and weekly limit a Claude or Codex sign-in has left, as last read. null or
   *  missing: never read (or a server before account-usage). */
  usage?: AccountUsage | null;
}

/** A subscription's limit: the rolling 5-hour one or the weekly one. */
export type UsageKind = 'five_hour' | 'weekly';

export interface UsageWindow {
  kind: UsageKind;
  /** 0–100, rounded. The UI shows 100 − this as "% left". */
  used_pct: number;
  resets_at: string | null;
}

/** GET /api/profiles `usage`, and what POST /api/profiles/:id/usage returns. */
export interface AccountUsage {
  checked_at: string;
  /** "turn": read off a bot's reply; "check": the person pressed Check now. */
  source: 'turn' | 'check';
  status: 'ok' | 'warning' | 'out' | null;
  windows: UsageWindow[];
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
  { id: 'anthropic-messages', label: 'Anthropic-compatible', hint: 'Bots can use tools' },
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
  {
    id: 'nvidia',
    label: 'NVIDIA',
    name: 'NVIDIA',
    urls: { 'openai-chat': 'https://integrate.api.nvidia.com/v1' },
    needsKey: true,
    note: 'Free models from build.nvidia.com. Needs an NVIDIA API key (starts with nvapi-).',
  },
  { id: 'lmstudio', label: 'LM Studio', name: 'LM Studio', urls: { 'openai-chat': 'http://localhost:1234/v1' }, needsKey: false, note: 'Start LM Studio’s local server first.' },
];

/** True when the address is a preset server that needs a key (NVIDIA, OpenRouter); the form drops "optional" then. */
export function customKeyRequired(baseUrl: string): boolean {
  const url = baseUrl.trim().replace(/\/+$/, '');
  return !!url && CUSTOM_PRESETS.some((p) => p.needsKey && Object.values(p.urls).some((u) => u === url));
}

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
  return dateClock(at) ?? '';
}

/** Below this much left, a limit's bar and the avatar badge turn amber (account-usage design §2.2). */
export const USAGE_LOW_LEFT = 20;
/** Numbers older than this are dimmed and say so (design §2.4). */
export const USAGE_STALE_MS = 60 * 60_000;
/** The server answers Check now from its copy for this long after a read, so the button waits too. */
export const USAGE_CHECK_COOLDOWN_MS = 60_000;

const USAGE_LABEL: Record<UsageKind, '5-hour' | 'Weekly'> = { five_hour: '5-hour', weekly: 'Weekly' };

/** One limit on an account row: "5-hour · 93% left · resets 22:29". */
export interface UsageRow {
  kind: UsageKind;
  label: '5-hour' | 'Weekly';
  /** 0–100; null once its reset has passed (the old number is wrong and the new one isn't known yet). */
  left: number | null;
  /** "93% left"; null once reset. */
  leftText: string | null;
  /** "resets 22:29", "resets Wed 00:59", "resets Nov 20"; "reset at 14:29" once passed; null when not known. */
  reset: string | null;
  /** low: under 20% left (amber); out: nothing left (empty amber track); reset: its reset has passed. */
  tone: 'ok' | 'low' | 'out' | 'reset';
}

/** Only sign-ins to Claude or Codex have these limits; API keys and Custom API accounts never show them. */
export const hasUsageLimits = (p: Pick<Profile, 'engine' | 'kind'>) => (p.engine === 'claude' || p.engine === 'codex') && (p.kind === 'system' || p.kind === 'cli_home');

/** A time on the person's own clock: "22:29" or "10:29 PM". */
const clockOf = (at: Date) => timeText(at) ?? '';
const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
const weekday = (at: Date) => at.toLocaleDateString(undefined, { weekday: 'short' });

/** When a limit resets: the time today, "Wed 00:59" within 6 days, else "Nov 20". */
export function usageResetTime(at: Date, now = Date.now()): string {
  if (sameDay(at, new Date(now))) return clockOf(at);
  if (at.getTime() - now < 6 * 86400_000) return `${weekday(at)} ${clockOf(at)}`;
  return at.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** An account's limits, 5-hour first; [] when it has none or they were never read. */
export function usageRows(p: Pick<Profile, 'engine' | 'kind' | 'usage'>, now = Date.now()): UsageRow[] {
  if (!hasUsageLimits(p) || !p.usage?.windows?.length) return [];
  const rows: UsageRow[] = [];
  for (const kind of ['five_hour', 'weekly'] as const) {
    const w = p.usage.windows.find((x) => x.kind === kind);
    if (!w || typeof w.used_pct !== 'number' || !Number.isFinite(w.used_pct)) continue;
    const parsed = w.resets_at ? new Date(w.resets_at) : null;
    const at = parsed && !Number.isNaN(parsed.getTime()) ? parsed : null;
    const label = USAGE_LABEL[kind];
    if (at && at.getTime() <= now) {
      const when = sameDay(at, new Date(now)) ? clockOf(at) : `${weekday(at)} ${clockOf(at)}`;
      rows.push({ kind, label, left: null, leftText: null, reset: `reset at ${when}`, tone: 'reset' });
      continue;
    }
    const left = Math.max(0, Math.min(100, Math.round(100 - w.used_pct)));
    rows.push({
      kind,
      label,
      left,
      leftText: `${left}% left`,
      reset: at ? `resets ${usageResetTime(at, now)}` : null,
      tone: left <= 0 ? 'out' : left < USAGE_LOW_LEFT ? 'low' : 'ok',
    });
  }
  return rows;
}

/** "checked just now", "… 4 min ago", "… 6 h ago", "… yesterday", then "checked Nov 11"; null when never read. */
export function usageCheckedText(usage: Pick<AccountUsage, 'checked_at'> | null | undefined, now = Date.now()): string | null {
  const t = usage?.checked_at ? Date.parse(usage.checked_at) : NaN;
  if (Number.isNaN(t)) return null;
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return 'checked just now';
  const m = Math.floor(s / 60);
  if (m < 60) return `checked ${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `checked ${h} h ago`;
  const at = new Date(t);
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  return sameDay(at, yesterday) ? 'checked yesterday' : `checked ${at.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`;
}

/** Read more than an hour ago: dimmed, with a labelled Check now. */
export const usageStale = (usage: Pick<AccountUsage, 'checked_at'> | null | undefined, now = Date.now()) => {
  const t = usage?.checked_at ? Date.parse(usage.checked_at) : NaN;
  return !Number.isNaN(t) && now - t > USAGE_STALE_MS;
};

/** Read under a minute ago: Check now would only get the server's copy back, so it waits. */
export const usageCoolingDown = (usage: Pick<AccountUsage, 'checked_at'> | null | undefined, now = Date.now()) => {
  const t = usage?.checked_at ? Date.parse(usage.checked_at) : NaN;
  return !Number.isNaN(t) && now - t >= 0 && now - t < USAGE_CHECK_COOLDOWN_MS;
};

/**
 * The model-badge hover's line (design §5): "5-hour 93% left · Weekly 44% left". A low window adds its reset
 * ("5-hour 12% left, resets 22:29"), a reset one is left out, stale numbers add " · checked 6 h ago", and `name`
 * (given when 2+ accounts of that engine could run) goes first. `low` draws the badge's amber dot. null when there's
 * nothing to say.
 */
export function usageSummary(p: Pick<Profile, 'engine' | 'kind' | 'usage'>, now = Date.now(), name?: string | null): { text: string; low: boolean } | null {
  const rows = usageRows(p, now).filter((r) => r.tone !== 'reset');
  if (!rows.length) return null;
  const parts = rows.map((r) => `${r.label} ${r.leftText}${r.tone !== 'ok' && r.reset ? `, ${r.reset}` : ''}`);
  if (name) parts.unshift(name);
  if (usageStale(p.usage, now)) parts.push(usageCheckedText(p.usage, now)!);
  return { text: parts.join(' · '), low: rows.some((r) => r.tone !== 'ok') };
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

/** The format and key note under a Custom API account's name; "key refused" once the server turned the key down. */
export function customKindText(p: Pick<Profile, 'has_key' | 'custom'>): string {
  const format = protocolLabel(p.custom?.protocol ?? '');
  if (!p.has_key) return format;
  return `${format} · ${p.custom?.status === 'key_rejected' ? 'key refused' : 'key saved'}`;
}

/** The "Model:" line for a Custom API account, or null when it would say nothing true (the server isn't answering). */
export function customModelText(c: CustomServer): string | null {
  if (c.model) return c.model;
  if (c.status !== 'ready') return null;
  return c.models.length ? 'each bot’s pick, else the server’s first' : 'none set';
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
    throw Object.assign(new Error(message), { status: res.status });
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
  /** Reads fresh 5-hour and weekly numbers (one tiny model call; the server answers from its copy within 60 s). */
  usage: (id: string) => request<AccountUsage>('POST', `/api/profiles/${enc(id)}/usage`),
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
