// API token handling: the `#kwnt_token=` link, the session cookie and 401s from any `/api` call.

import { getCurrentSpace, withSpace } from './spaces';

/** Fired on `window` when an `/api` call answers 401 (the token is missing or no longer valid). */
export const UNAUTHORIZED_EVENT = 'kwnt:unauthorized';
/** Fired when a person-only route (saved passwords, answering approvals, the approvals switch)
 * answers 401 `{person_only:true}`: the rest of the app works, only this action needs the token. */
export const PERSON_TOKEN_EVENT = 'kwnt:person-token';
/** Fired after the person signed in from that prompt, so open panels can load again. */
export const PERSON_SIGNED_IN_EVENT = 'kwnt:person-signed-in';
/** Fired when the person closed that prompt without signing in. */
export const PERSON_PROMPT_CLOSED_EVENT = 'kwnt:person-prompt-closed';

/** What a person-only action says when the person closed "Confirm it's you" instead of signing in. */
export const PERSON_CANCELLED_TEXT = 'Nothing was changed. Only you can do this: confirm it’s you, then try again.';

/** Thrown by `personFetch` when the person closed "Confirm it's you"; nothing was changed. */
export class PersonCancelledError extends Error {
  constructor() {
    super(PERSON_CANCELLED_TEXT);
  }
}

/** Resolves true once the person signs in from the prompt, false when they close it. */
export function waitForPersonSignIn(target: EventTarget = window): Promise<boolean> {
  return new Promise((resolve) => {
    const done = (ok: boolean) => () => {
      target.removeEventListener(PERSON_SIGNED_IN_EVENT, onIn);
      target.removeEventListener(PERSON_PROMPT_CLOSED_EVENT, onClosed);
      resolve(ok);
    };
    const onIn = done(true);
    const onClosed = done(false);
    target.addEventListener(PERSON_SIGNED_IN_EVENT, onIn);
    target.addEventListener(PERSON_PROMPT_CLOSED_EVENT, onClosed);
  });
}

/**
 * `fetch` for a person-only action (deleting a bot or team, undoing an action or import). On a
 * person-only 401 the fetch wrapper has already asked "Confirm it's you"; this waits for that and
 * sends the request once more, so the person doesn't see an error and press Delete again. Throws
 * `PersonCancelledError` when they close the prompt instead.
 */
export async function personFetch(input: string, init?: RequestInit, target?: EventTarget, send: typeof fetch = (...a) => fetch(...a)): Promise<Response> {
  const res = await send(input, init);
  if (res.status !== 401) return res;
  const body = await res.clone().json().catch(() => null);
  if (!isPersonOnlyBody(body)) return res;
  // Listen before anything can answer the prompt the wrapper just opened.
  if (!(await waitForPersonSignIn(target ?? window))) throw new PersonCancelledError();
  return send(input, init);
}

export interface AuthStatus {
  auth_required: boolean;
  authenticated: boolean;
  /** Person-only actions need the token even where the rest of the API doesn't (newer servers). */
  person_auth_required?: boolean;
  person_authenticated?: boolean;
}

/** Asks for the token for a person-only action, as if one had answered 401. */
export function requestPersonToken(): void {
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(PERSON_TOKEN_EVENT));
}

/** Whether a 401 body is the person-only answer, not "the whole API needs the token". */
export function isPersonOnlyBody(body: unknown): boolean {
  return !!body && typeof body === 'object' && (body as { person_only?: unknown }).person_only === true;
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
    // `kwntQuiet`: a background load that shows its own "needs the token" state instead.
    if (res.status === 401 && !(init as { kwntQuiet?: boolean } | undefined)?.kwntQuiet) {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (isGuardedApiUrl(url, window.location.origin)) {
        const body = await res.clone().json().catch(() => null);
        window.dispatchEvent(new Event(isPersonOnlyBody(body) ? PERSON_TOKEN_EVENT : UNAUTHORIZED_EVENT));
      }
    }
    return res;
  };
}

export async function fetchAuthStatus(): Promise<AuthStatus> {
  const res = await fetch('/api/auth/status', { cache: 'no-store' });
  if (res.status === 403) throw new Error("This page's address isn't allowed to use the API (403). Open kwant bots at its own address.");
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return res.json();
}

/** What the server said to a token or one-time link code. */
export type SessionResult = 'ok' | 'wrong_token' | 'link_used_or_expired';

/** One-time sign-in link codes start with this; the API token never does. */
export const LINK_CODE_PREFIX = 'kl1.';

/** Whether a pasted or linked value is a one-time sign-in link code rather than the API token. */
export function isLinkCode(value: string): boolean {
  return value.trim().startsWith(LINK_CODE_PREFIX);
}

/** Exchanges the token, or a one-time link code, for a session cookie. */
export async function redeemSignIn(token: string): Promise<SessionResult> {
  const res = await fetch('/api/auth/session', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token }),
  });
  if (res.status === 401) {
    const body = await res.json().catch(() => null);
    // A link this server forgot (it restarted) reads as a wrong token; for a link it's the same story
    if (body?.reason === 'link_used_or_expired' || isLinkCode(token)) return 'link_used_or_expired';
    return 'wrong_token';
  }
  if (!res.ok) throw new Error(`The server answered ${res.status}.`);
  return 'ok';
}

/** Exchanges the token for the session cookie; resolves to false when the token is wrong. */
export async function startSession(token: string): Promise<boolean> {
  return (await redeemSignIn(token)) === 'ok';
}

export async function endSession(): Promise<void> {
  await fetch('/api/auth/session', { method: 'DELETE' });
}

let linkSession: Promise<string | null> | null = null;

/** What the sign-in screen says when a one-time link was opened a second time or too late. */
export const LINK_SPENT =
  'This sign-in link was already used or has expired. Each link works once, for 15 minutes. Sign in another way below.';

/**
 * Signs in with a `#kwnt_token=` link (a one-time code or the API token) and removes it from the
 * address bar (and history) before any network call, so React's second effect run finds no token
 * and shares the first one's result. Resolves to an error message when it was refused.
 */
export function consumeTokenLink(): Promise<string | null> {
  const { token, rest } = extractHashToken(window.location.hash);
  if (token === null) return linkSession ?? Promise.resolve(null);
  window.history.replaceState(window.history.state, '', `${window.location.pathname}${window.location.search}${rest}`);
  linkSession = !token
    ? Promise.resolve('The link is incomplete. Copy the whole link, or sign in another way below.')
    : redeemSignIn(token).then(
        (result) =>
          result === 'ok'
            ? null
            : result === 'link_used_or_expired'
              ? LINK_SPENT
              : "The link's token wasn't accepted. It may have been changed; sign in another way below.",
        (e: unknown) => (e instanceof Error ? e.message : 'Could not reach the server.'),
      );
  return linkSession;
}
