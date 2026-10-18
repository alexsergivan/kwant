// API token handling: the `#kwnt_token=` link, the session cookie and 401s from any `/api` call.

import { getCurrentSpace, withSpace } from './spaces';

/** Fired on `window` when an `/api` call answers 401 (the token is missing or no longer valid). */
export const UNAUTHORIZED_EVENT = 'kwnt:unauthorized';

export interface AuthStatus {
  auth_required: boolean;
  authenticated: boolean;
}

/** Splits `#kwnt_token=<t>` off a location hash, keeping any other fragment parameters. */
export function extractHashToken(hash: string): { token: string | null; rest: string } {
  const raw = hash.startsWith('#') ? hash.slice(1) : hash;
  if (!raw) return { token: null, rest: '' };
  const params = new URLSearchParams(raw);
  const token = params.get('kwnt_token');
  if (token === null) return { token: null, rest: hash };
  params.delete('kwnt_token');
  const rest = params.toString();
  return { token: token.trim(), rest: rest ? `#${rest}` : '' };
}

/** Whether a 401 from this URL means the API token is missing. The auth endpoints answer for themselves. */
export function isGuardedApiUrl(url: string, origin: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url, origin);
  } catch {
    return false;
  }
  return parsed.origin === origin && parsed.pathname.startsWith('/api/') && !parsed.pathname.startsWith('/api/auth/');
}

let installed = false;

/** Wraps `window.fetch` so every component's `/api` 401 reaches the auth gate, and space-scoped
 * `/api` calls carry the open space as `?space=`. */
export function installUnauthorizedListener(): void {
  if (installed || typeof window === 'undefined') return;
  installed = true;
  const original = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    if (typeof input === 'string') input = withSpace(input, window.location.origin, getCurrentSpace());
    const res = await original(input, init);
    if (res.status === 401) {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (isGuardedApiUrl(url, window.location.origin)) window.dispatchEvent(new Event(UNAUTHORIZED_EVENT));
    }
    return res;
  };
}

export async function fetchAuthStatus(): Promise<AuthStatus> {
  const res = await fetch('/api/auth/status', { cache: 'no-store' });
  if (res.status === 403) throw new Error("This page's address isn't allowed to use the API (403). Open kwnt at its own address.");
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return res.json();
}

/** Exchanges the token for the session cookie; resolves to false when the token is wrong. */
export async function startSession(token: string): Promise<boolean> {
  const res = await fetch('/api/auth/session', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token }),
  });
  if (res.status === 401) return false;
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return true;
}

export async function endSession(): Promise<void> {
  await fetch('/api/auth/session', { method: 'DELETE' });
}

let linkSession: Promise<string | null> | null = null;

/**
 * Signs in with a `#kwnt_token=` link and removes the token from the address bar (and history)
 * before any network call, so React's second effect run finds no token and shares the first one's
 * result. Resolves to an error message when the token was refused.
 */
export function consumeTokenLink(): Promise<string | null> {
  const { token, rest } = extractHashToken(window.location.hash);
  if (token === null) return linkSession ?? Promise.resolve(null);
  window.history.replaceState(window.history.state, '', `${window.location.pathname}${window.location.search}${rest}`);
  linkSession = !token
    ? Promise.resolve('The link has an empty token.')
    : startSession(token).then(
        (ok) => (ok ? null : "The link's token wasn't accepted. It may have been changed; paste the current one."),
        (e: unknown) => (e instanceof Error ? e.message : 'Could not reach the server.'),
      );
  return linkSession;
}
