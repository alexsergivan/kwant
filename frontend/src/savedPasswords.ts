// Saved passwords for a team's, channel's or bot's browser. The list carries no passwords: one is
// fetched only when the person presses Reveal, shown for a short while and never kept. A login the
// browser caught on a form waits as `pending` until the person saves or discards it. Every route
// is person-only: without the API token the server answers 401 `{person_only:true}`, and the auth
// layer then asks for the token (see auth.ts).

/** GET /api/sandbox/:id/passwords → passwords[] */
export interface SavedLogin {
  id: string;
  origin: string;
  domain: string;
  username: string;
  created_at: string | null;
  last_used: string | null;
}

/** A login caught on a page, waiting for the person's yes or no. */
export interface PendingLogin {
  origin: string;
  username: string;
  captured_at: string | null;
}

export interface PasswordsList {
  passwords: SavedLogin[];
  pending: PendingLogin | null;
}

/** Why the list can't be shown. `locked` is for a store that must be unlocked first (Bitwarden). */
export type PasswordsProblem = 'needs-token' | 'locked' | 'unavailable' | 'error';

/** How long a revealed password stays on screen. */
export const REVEAL_MS = 30_000;

const str = (v: unknown): string | null => (typeof v === 'string' ? v : null);

function parseLogin(raw: unknown): SavedLogin | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const id = str(o.id);
  const origin = str(o.origin) ?? '';
  const domain = str(o.domain) ?? hostOf(origin);
  if (!id || (!origin && !domain)) return null;
  // Whatever else the server sends (an older one sent `password`) is dropped here.
  return { id, origin, domain, username: str(o.username) ?? '', created_at: str(o.created_at), last_used: str(o.last_used) };
}

function parsePending(raw: unknown): PendingLogin | null {
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const origin = str(o.origin);
  if (!origin) return null;
  return { origin, username: str(o.username) ?? '', captured_at: str(o.captured_at) };
}

export function parsePasswordsList(raw: unknown): PasswordsList {
  const o = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const list = Array.isArray(o.passwords) ? o.passwords : [];
  return {
    passwords: list.map(parseLogin).filter((l): l is SavedLogin => l !== null),
    pending: parsePending(o.pending),
  };
}

export function hostOf(origin: string): string {
  try {
    return new URL(origin).hostname.toLowerCase();
  } catch {
    return '';
  }
}

/** The origin (scheme, host and port) of a page, or '' for a blank or unparsable one. */
export function pageOrigin(url: string | null | undefined): string {
  if (!url || url === 'about:blank') return '';
  try {
    const u = new URL(url);
    return u.protocol === 'http:' || u.protocol === 'https:' ? u.origin : '';
  } catch {
    return '';
  }
}

/** Autofill only fills on the exact origin it was saved for, so "for this site" means the same. */
export function loginMatchesPage(login: Pick<SavedLogin, 'origin'>, pageUrl: string | null | undefined): boolean {
  const page = pageOrigin(pageUrl);
  return !!page && pageOrigin(login.origin) === page;
}

/** What the person typed in "Site" → the origin to save under. A bare host means https. */
export function originFromInput(input: string): string | null {
  const v = input.trim();
  if (!v) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(v) ? v : `https://${v}`;
  try {
    const u = new URL(withScheme);
    if ((u.protocol !== 'https:' && u.protocol !== 'http:') || !u.hostname) return null;
    return u.origin;
  } catch {
    return null;
  }
}

/** A short label for an origin: the host, plus the scheme when it isn't https and any port. */
export function originLabel(origin: string): string {
  try {
    const u = new URL(origin);
    return u.protocol === 'https:' ? u.host : `${u.protocol}//${u.host}`;
  } catch {
    return origin;
  }
}

export function filterLogins(list: SavedLogin[], query: string): SavedLogin[] {
  const q = query.trim().toLowerCase();
  if (!q) return list;
  return list.filter((l) => l.origin.toLowerCase().includes(q) || l.domain.toLowerCase().includes(q) || l.username.toLowerCase().includes(q));
}

export function problemFor(status: number, body: unknown): PasswordsProblem {
  const o = body && typeof body === 'object' ? (body as Record<string, unknown>) : {};
  if (status === 401) return 'needs-token';
  if (status === 423 || o.locked === true) return 'locked';
  if (status === 503) return 'unavailable';
  return 'error';
}

export const PROBLEM_TEXT: Record<PasswordsProblem, string> = {
  'needs-token': 'Saved passwords need your access token, even on this computer, so a bot can’t reach them.',
  locked: 'The password store is locked. Unlock it to see or use saved passwords.',
  unavailable: 'The password store isn’t available right now. Try again in a moment.',
  error: 'Couldn’t load saved passwords.',
};

export class PasswordsError extends Error {
  constructor(public problem: PasswordsProblem, public status: number, message: string) {
    super(message);
  }
}

async function call(url: string, init?: RequestInit): Promise<unknown> {
  const res = await fetch(url, { cache: 'no-store', ...init });
  const body = await res.json().catch(() => null);
  if (!res.ok) {
    const problem = problemFor(res.status, body);
    const msg = body && typeof body === 'object' && typeof (body as { error?: unknown }).error === 'string' ? (body as { error: string }).error : null;
    throw new PasswordsError(problem, res.status, problem === 'error' && msg ? msg : PROBLEM_TEXT[problem]);
  }
  return body;
}

const json = (method: string, body?: unknown): RequestInit => ({
  method,
  headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
  body: body === undefined ? undefined : JSON.stringify(body),
});

const base = (sandboxId: string) => `/api/sandbox/${encodeURIComponent(sandboxId)}/passwords`;

export const passwordsApi = {
  /** Quiet: a missing token shows as the panel's own state, not the token dialog. */
  list: async (sandboxId: string) => parsePasswordsList(await call(base(sandboxId), { kwntQuiet: true } as RequestInit)),
  /** The one call that returns a password. Callers show it briefly and drop it. */
  reveal: async (sandboxId: string, id: string): Promise<string> => {
    const body = (await call(`${base(sandboxId)}/${encodeURIComponent(id)}/reveal`, json('POST'))) as { password?: unknown } | null;
    if (typeof body?.password !== 'string') throw new PasswordsError('error', 200, 'The server sent no password.');
    return body.password;
  },
  add: (sandboxId: string, origin: string, username: string, password: string) => call(base(sandboxId), json('POST', { origin, username, password })),
  remove: (sandboxId: string, id: string) => call(`${base(sandboxId)}/${encodeURIComponent(id)}`, json('DELETE')),
  savePending: (sandboxId: string) => call(`${base(sandboxId)}/pending`, json('POST')),
  discardPending: (sandboxId: string) => call(`${base(sandboxId)}/pending`, json('DELETE')),
};
