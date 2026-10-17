// "Sign in with Claude" for the Claude engine: the server runs Claude Code's own `auth login`,
// we open the claude.ai link it prints and send back the code claude.ai shows. The UI never sees a
// token; the code only ever goes in a POST body.

import type { EngineInfo } from './engines';

export type ClaudeLoginState = 'starting' | 'waiting_for_code' | 'verifying' | 'done' | 'failed' | 'expired' | 'cancelled';

/** Shaped like SessionView in src/server/engine_login.rs. */
export interface ClaudeLoginSession {
  session_id: string;
  state: ClaudeLoginState;
  detail: string | null;
  /** The claude.ai link; null once the sign-in has finished. */
  url: string | null;
  expires_at: string;
}

export class ClaudeLoginError extends Error {
  constructor(
    message: string,
    readonly status: number,
    /** On 409: the sign-in that's already running. */
    readonly sessionId: string | null = null,
  ) {
    super(message);
  }
}

const BASE = '/api/engines/claude';

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let r: Response;
  try {
    r = await fetch(`${BASE}${path}`, {
      method,
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ClaudeLoginError('Couldn’t reach the server.', 0);
  }
  if (r.status === 204) return undefined as T;
  const d = await r.json().catch(() => null);
  if (!r.ok) {
    const msg = typeof d?.error === 'string' && d.error ? d.error : `HTTP ${r.status}`;
    throw new ClaudeLoginError(msg, r.status, typeof d?.session_id === 'string' ? d.session_id : null);
  }
  return d as T;
}

export const claudeLoginApi = {
  start: () => request<ClaudeLoginSession>('POST', '/login'),
  get: (sid: string) => request<ClaudeLoginSession>('GET', `/login/${encodeURIComponent(sid)}`),
  submitCode: (sid: string, code: string) => request<ClaudeLoginSession>('POST', `/login/${encodeURIComponent(sid)}/code`, { code }),
  cancel: (sid: string) => request<void>('DELETE', `/login/${encodeURIComponent(sid)}`),
  logout: () => request<{ engines: EngineInfo[] }>('POST', '/logout'),
};

/** Still going: the CLI is waiting for a code or checking one. */
export const isLoginActive = (s: ClaudeLoginState) => s === 'starting' || s === 'waiting_for_code' || s === 'verifying';

/** What to tell the user when a sign-in ended without signing in; null when there's nothing to say. */
export function loginFailure(s: Pick<ClaudeLoginSession, 'state' | 'detail'>): string | null {
  switch (s.state) {
    case 'failed':
      return s.detail || 'Sign-in failed. Start it again.';
    case 'expired':
      return s.detail || 'Sign-in timed out. Start it again.';
    default:
      return null;
  }
}

/** Only https links to Anthropic's own sign-in pages are opened. */
export function isClaudeSignInUrl(url: string | null | undefined): url is string {
  if (!url) return false;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' && /(^|\.)(claude\.ai|claude\.com|anthropic\.com)$/.test(u.hostname);
  } catch {
    return false;
  }
}

/** The Claude Code CLI is on but signed out and no other Claude account is ready, so "Sign in with Claude" fixes it. */
export const claudeNeedsSignIn = (engines: EngineInfo[] | null | undefined) =>
  !!engines?.some((e) => e.id === 'claude' && e.enabled !== false && e.status === 'signed_out' && e.ready !== true);
