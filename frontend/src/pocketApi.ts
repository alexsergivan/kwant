// Pocket apps (pa4): the host's calls from the Pocket card to the main port. Written against Lin's pa1
// host API ("Pocket API v1", autonomy/pocket-apps/pa1/impl.md; until it merges, pa1/brief.md item 7 and
// the routes on lin-pa1-pocket-core): list, read one, end, delete. All four are person-only on the main
// port; a guest or host key is never sent from here. Nothing in the card polls: it reads on demand.

/** A pocket as the person's list shows it: public facts and counts, never names or guest text. */
export interface PocketCard {
  code: string;
  kit: string;
  title: string;
  state: string;
  opens_at: string | null;
  closes_at: string | null;
  ended_at: string | null;
  expires_at: string | null;
  players: number;
  max_players: number | null;
  needs_pin: boolean;
  retention: string | null;
  created_at?: string;
  owner_kwant_id?: string | null;
  /** `/p/<code>` on the guest port. */
  path?: string;
}

export interface PocketList {
  enabled: boolean;
  pocket_port: number | null;
  kits: string[];
  pockets: PocketCard[];
}

export interface PocketRead {
  pocket: PocketCard;
  /** The kit's host summary (counts only), or null for a kit this box doesn't know. */
  summary: unknown;
  removed: number;
}

export class PocketApiError extends Error {
  constructor(message: string, readonly status: number, readonly code: string | null) {
    super(message);
    this.name = 'PocketApiError';
  }
}

/** The same rule as the server's `keys::valid_code`, loosely: letters and digits, 3-32. */
export const validPocketCode = (code: string) => /^[A-Za-z0-9_-]{3,32}$/.test(code);

const FALLBACK: Record<string, string> = {
  pocket_apps_off: 'Pocket apps are off. Turn them on in Settings first.',
  not_found: 'This pocket app no longer exists.',
};

async function call<T>(method: 'GET' | 'POST' | 'DELETE', path: string): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, { method, headers: { Accept: 'application/json' }, credentials: 'same-origin' });
  } catch {
    throw new PocketApiError('Your box didn’t answer. Check the connection and try again.', 0, null);
  }
  if (!res.ok) {
    let data: Record<string, unknown> | null = null;
    try {
      data = (await res.json()) as Record<string, unknown>;
    } catch {
      // no JSON body
    }
    const code = typeof data?.error === 'string' ? data.error : null;
    const message =
      (typeof data?.message === 'string' && data.message) ||
      (code && FALLBACK[code]) ||
      (res.status === 404 ? FALLBACK.not_found : `That didn’t work (HTTP ${res.status}).`);
    throw new PocketApiError(message, res.status, code);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

const one = (code: string) => {
  if (!validPocketCode(code)) throw new PocketApiError(FALLBACK.not_found, 404, 'not_found');
  return `/api/pockets/${encodeURIComponent(code)}`;
};

export const pocketApi = {
  list: () => call<PocketList>('GET', '/api/pockets'),
  read: async (code: string) => call<PocketRead>('GET', one(code)),
  /** Stop (before the game) and End quiz (during it) both end the pocket: the link closes. */
  end: async (code: string) => call<{ pocket: PocketCard }>('POST', `${one(code)}/end`),
  /** Final: players, answers and the pocket go now. */
  remove: async (code: string) => call<void>('DELETE', one(code)),
};
