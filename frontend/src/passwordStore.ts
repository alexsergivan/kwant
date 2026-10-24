// Where saved passwords live: on this server (default) or in the person's Bitwarden. Bitwarden is
// locked after every restart; unlocking, revealing and disconnecting take the master password.
// The master password and an API key's client secret are read from their input at submit, sent in
// one request and never kept in React state, storage or logs. Every route is person-only.

import { isPersonOnlyBody, requestPersonToken } from './auth';

export interface BitwardenInfo {
  server_url: string;
  email: string;
  login: 'password' | 'api_key';
  client_id: string | null;
  unlocked: boolean;
  unlocked_at: string | null;
  item_count: number | null;
  last_sync: string | null;
}

/** GET /api/passwords/store */
export interface StoreStatus {
  store: 'local' | 'bitwarden';
  /** Bitwarden is the store and isn't unlocked: nothing is filled, the lists answer 423. */
  locked: boolean;
  /** Passwords still in the built-in store (what "Move to Bitwarden" would move). */
  local_count: number;
  bitwarden: BitwardenInfo | null;
  /** Some space or bot runs without asking. */
  approvals_off: boolean;
  /** Approvals off while passwords are still saved here (an install from before the block): any
   * bot that can run commands can read them. Nothing changes it on its own. */
  exposed: boolean;
}

const str = (v: unknown): string | null => (typeof v === 'string' ? v : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

export function parseStoreStatus(raw: unknown): StoreStatus {
  const o = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const b = o.bitwarden && typeof o.bitwarden === 'object' ? (o.bitwarden as Record<string, unknown>) : null;
  const bitwarden: BitwardenInfo | null = b
    ? {
        server_url: str(b.server_url) ?? '',
        email: str(b.email) ?? '',
        login: b.login === 'api_key' ? 'api_key' : 'password',
        client_id: str(b.client_id),
        unlocked: b.unlocked === true,
        unlocked_at: str(b.unlocked_at),
        item_count: num(b.item_count),
        last_sync: str(b.last_sync),
      }
    : null;
  const store = o.store === 'bitwarden' ? 'bitwarden' : 'local';
  const local_count = num(o.local_count) ?? 0;
  const approvals_off = o.approvals_off === true;
  return {
    store,
    locked: o.locked === true || (store === 'bitwarden' && !bitwarden?.unlocked),
    local_count,
    bitwarden,
    approvals_off,
    exposed: o.exposed === true,
  };
}

/** The Bitwarden cloud servers; anything else is self-hosted (Vaultwarden too). */
export const BITWARDEN_CLOUDS = [
  { value: 'https://vault.bitwarden.com', label: 'bitwarden.com' },
  { value: 'https://vault.bitwarden.eu', label: 'bitwarden.eu' },
] as const;
export const DEFAULT_BITWARDEN_SERVER = BITWARDEN_CLOUDS[0].value;

/** A short name for the account's server: "bitwarden.com", or the self-hosted host. */
export function serverLabel(serverUrl: string): string {
  const cloud = BITWARDEN_CLOUDS.find((c) => c.value === serverUrl.replace(/\/+$/, ''));
  if (cloud) return cloud.label;
  try {
    return new URL(serverUrl).host || serverUrl;
  } catch {
    return serverUrl;
  }
}

/** What the person typed as a self-hosted server → an address the server accepts, or why not. */
export function checkServerUrl(input: string): { url: string } | { error: string } {
  const v = input.trim().replace(/\/+$/, '');
  if (!v) return { error: 'Enter your Bitwarden server’s address.' };
  const withScheme = v.includes('://') ? v : `https://${v}`;
  let u: URL;
  try {
    u = new URL(withScheme);
  } catch {
    return { error: 'That isn’t a web address.' };
  }
  const host = u.hostname.replace(/^\[|\]$/g, '');
  const loopback = host === 'localhost' || /^127\./.test(host) || host === '::1';
  if (u.protocol !== 'https:' && !(u.protocol === 'http:' && loopback)) return { error: 'The address must start with https:// (http works only on this computer).' };
  if (u.search || u.hash || u.username || u.password) return { error: 'Leave out anything after the address, and any name or password in it.' };
  return { url: `${u.origin}${u.pathname.replace(/\/+$/, '')}` };
}

/** Bitwarden's two-step login methods. Only the ones with a code to type can be done here. */
export const TWO_FACTOR_NAMES: Record<number, string> = {
  0: 'Authenticator app',
  1: 'Email',
  2: 'Duo',
  3: 'YubiKey',
  4: 'FIDO U2F',
  6: 'Duo (organisation)',
  7: 'Passkey or security key',
};
const TYPED_CODE = new Set([0, 1, 3]);

/** The methods the account offers, usable ones (a code to type) first. */
export function twoFactorChoices(providers: unknown): Array<{ id: number; name: string; usable: boolean }> {
  const ids = (Array.isArray(providers) ? providers : [])
    .map((p) => (typeof p === 'number' ? p : typeof p === 'string' && /^\d+$/.test(p) ? Number(p) : NaN))
    .filter((n) => Number.isInteger(n) && n !== 5);
  const unique = [...new Set(ids)];
  return unique
    .map((id) => ({ id, name: TWO_FACTOR_NAMES[id] ?? `Method ${id}`, usable: TYPED_CODE.has(id) }))
    .sort((a, b) => Number(b.usable) - Number(a.usable));
}

/** Why an unlock (or a check of the master password) didn't go through. */
export type UnlockProblem =
  | { kind: 'needs-token' }
  | { kind: 'wrong-password' }
  | { kind: 'two-factor'; providers: Array<{ id: number; name: string; usable: boolean }> }
  | { kind: 'new-device-code' }
  | { kind: 'other-account' }
  | { kind: 'invalid'; message: string }
  | { kind: 'unreachable' }
  | { kind: 'locked' }
  | { kind: 'error'; message: string };

export function unlockProblemFor(status: number, body: unknown): UnlockProblem {
  const o = body && typeof body === 'object' ? (body as Record<string, unknown>) : {};
  const message = str(o.error) ?? '';
  if (status === 401 && isPersonOnlyBody(body)) return { kind: 'needs-token' };
  if (o.wrong_password === true) return { kind: 'wrong-password' };
  if (o.needs === 'two_factor') return { kind: 'two-factor', providers: twoFactorChoices(o.providers) };
  if (o.needs === 'new_device_otp') return { kind: 'new-device-code' };
  if (status === 409) return { kind: 'other-account' };
  if (status === 400) return { kind: 'invalid', message };
  if (status === 502) return { kind: 'unreachable' };
  if (status === 423 || o.locked === true) return { kind: 'locked' };
  if (status === 401) return { kind: 'invalid', message };
  return { kind: 'error', message: message || `The server answered ${status}.` };
}

export function unlockProblemText(p: UnlockProblem): string {
  switch (p.kind) {
    case 'needs-token':
      return 'This needs your access token first.';
    case 'wrong-password':
      return 'That master password isn’t right. After five wrong tries in a row Bitwarden stays locked.';
    case 'two-factor':
      return p.providers.some((x) => x.usable)
        ? 'Your account uses two-step login. Enter the code to continue.'
        : `Your account’s two-step login (${p.providers.map((x) => x.name).join(', ') || 'unknown'}) can’t be done here. Use an API key instead.`;
    case 'new-device-code':
      return 'Bitwarden emailed you a code to confirm this new device. Enter it to continue.';
    case 'other-account':
      return 'Another Bitwarden account is connected. Disconnect it first.';
    case 'invalid':
      return p.message ? capitalise(p.message) + '.' : 'Something in the form isn’t right.';
    case 'unreachable':
      return 'Bitwarden couldn’t be reached, or it refused. Check the server address and try again.';
    case 'locked':
      return 'Bitwarden is locked. Unlock it first.';
    case 'error':
      return p.message || 'Something went wrong.';
  }
}

const capitalise = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

export class StoreError extends Error {
  constructor(public problem: UnlockProblem, public status: number) {
    super(unlockProblemText(problem));
  }
}

/** One person-only call. Quiet: a 401 here is often a wrong master password, not a missing
 * token, so the app-wide gate must not open. The person-only 401 asks for the token itself. */
async function call(url: string, method = 'GET', body?: unknown, askToken = true): Promise<unknown> {
  const init = {
    method,
    cache: 'no-store',
    kwntQuiet: true,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  } as RequestInit;
  const res = await fetch(url, init);
  const answer = await res.json().catch(() => null);
  if (!res.ok) {
    const problem = unlockProblemFor(res.status, answer);
    if (problem.kind === 'needs-token' && askToken) requestPersonToken();
    throw new StoreError(problem, res.status);
  }
  return answer;
}

export interface UnlockFields {
  master_password: string;
  /** First connect only (later the saved ones are used). */
  server_url?: string;
  email?: string;
  /** API-key login: the id is saved; the secret is asked for at every unlock. */
  client_id?: string;
  client_secret?: string;
  two_factor_provider?: number;
  two_factor_token?: string;
  new_device_otp?: string;
}

/** Drops empty optional fields so the server falls back to the saved settings. */
export function unlockBody(f: UnlockFields): UnlockFields {
  const out: UnlockFields = { master_password: f.master_password };
  for (const k of ['server_url', 'email', 'client_id', 'client_secret', 'two_factor_token', 'new_device_otp'] as const) {
    const v = f[k]?.trim();
    if (v) out[k] = k === 'client_secret' ? f[k] : v;
  }
  if (f.two_factor_token?.trim() && typeof f.two_factor_provider === 'number') out.two_factor_provider = f.two_factor_provider;
  return out;
}

export interface MigrateResult {
  moved: number;
  failed: number;
}

export function migrateText(r: MigrateResult): string {
  const moved = `Moved ${r.moved} ${r.moved === 1 ? 'password' : 'passwords'} to Bitwarden.`;
  return r.failed ? `${moved} ${r.failed} couldn’t be moved and ${r.failed === 1 ? 'stays' : 'stay'} on this server; try again.` : moved;
}

export const storeApi = {
  status: async () => parseStoreStatus(await call('/api/passwords/store')),
  /** The same, without asking for the token: for screens where the store is a side note. */
  peek: async () => parseStoreStatus(await call('/api/passwords/store', 'GET', undefined, false)),
  unlock: async (f: UnlockFields) => parseStoreStatus(await call('/api/passwords/bitwarden/unlock', 'POST', unlockBody(f))),
  lock: async () => parseStoreStatus(await call('/api/passwords/bitwarden/lock', 'POST')),
  /** Back to the built-in store. The items stay in Bitwarden. */
  disconnect: async (master_password: string) => parseStoreStatus(await call('/api/passwords/bitwarden', 'DELETE', { master_password })),
  migrate: async (): Promise<MigrateResult> => {
    const o = (await call('/api/passwords/migrate', 'POST')) as Record<string, unknown> | null;
    return { moved: num(o?.moved) ?? 0, failed: num(o?.failed) ?? 0 };
  },
};

/** One sentence per store for wherever a space or bot is on "Ask only when risky" or "Don't ask".
 * Bitwarden: the accepted gap (2026-10-20). This server: either mode can't sit next to passwords
 * here (audit F-10), so it's blocked. */
export const AUTOFILL_GAP_NOTE = {
  bitwarden: 'While a password is being filled into a page, a bot that can run commands can read it.',
  local: 'Unless every space and bot is on “Ask before running commands”, any bot that can run commands could read passwords saved on this server, so they can’t be saved here: use Bitwarden.',
} as const;

/** The note for a store; unknown counts as this server, the stricter one. */
export const autofillGapNote = (store: StoreStatus | null | undefined): string =>
  store?.store === 'bitwarden' && store.local_count === 0 ? AUTOFILL_GAP_NOTE.bitwarden : AUTOFILL_GAP_NOTE.local;

const passwords = (n: number) => `${n} saved ${n === 1 ? 'password' : 'passwords'}`;

/** Approvals can't go off while this server holds passwords: what the confirm says instead. */
export const moveFirstText = (n: number) =>
  `${capitalise(passwords(n))} ${n === 1 ? 'is' : 'are'} kept on this server. Under “Ask only when risky” or “Don’t ask”, any bot that can run commands could read ${n === 1 ? 'it' : 'them'}, so move ${n === 1 ? 'it' : 'them'} to Bitwarden first.`;
export const moveFirstLabel = (n: number) => `Move ${passwords(n)} to Bitwarden first`;

/** An install already in that state (a space or bot stopped asking before the block). */
export const exposedText = (n: number) =>
  `A space or bot is on “Ask only when risky” or “Don’t ask” and ${passwords(n)} ${n === 1 ? 'is' : 'are'} still on this server, so any bot that can run commands can read ${n === 1 ? 'it' : 'them'}.`;

/** A 409 that says the server wants Bitwarden first: its sentence, and how many passwords are here. */
export function needsBitwardenFrom(status: number, body: unknown): { message: string; local_count: number | null } | null {
  const o = body && typeof body === 'object' ? (body as Record<string, unknown>) : {};
  if (status !== 409 || o.needs_bitwarden !== true) return null;
  const msg = str(o.error)?.trim();
  return {
    message: msg ? capitalise(msg.endsWith('.') ? msg : `${msg}.`) : 'Move the passwords on this server to Bitwarden first.',
    local_count: num(o.local_count),
  };
}
