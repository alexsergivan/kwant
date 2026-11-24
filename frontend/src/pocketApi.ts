// Pocket apps (pa4): the host's calls from the Pocket card to the main port. Written against Lin's pa1
// host API ("Pocket API v1", autonomy/pocket-apps/pa1/impl.md; until it merges, pa1/brief.md item 7 and
// the routes on lin-pa1-pocket-core): list, read one, end, delete; pa4b adds mintLink (pa1b "Pocket API v1.1:
// links"). All are person-only on the main port; a guest or host key is never sent from here. A minted link's
// token only ever goes into a URL fragment (linkUrl), never a query string. Nothing in the card polls.
// pa4c adds the quiz draft edit routes (Juno's pa4c-server/impl.md "Draft edit API"): read, add, edit and delete a
// question. Every edit sends `base` (the draft's `updated_at` the person saw); the app's fetch wrapper adds `?space=`.

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

/** How guests reach the pocket (pa10a): each url ends in /p/<code> on the guest port. */
export interface PocketJoin {
  mode: 'wifi' | 'anywhere';
  anywhere_url: string | null;
  wifi_url: string | null;
}

export interface PocketRead {
  pocket: PocketCard;
  /** The kit's host summary (counts only), or null for a kit this box doesn't know. */
  summary: unknown;
  removed: number;
  join?: PocketJoin | null;
  /** pa10b gather: the host's view (names, drafts with `next_copy_at`). */
  host_view?: unknown;
  /** pa13 desk, `?view=guest:<id>` only: one guest's conversation (host only). */
  conversation?: unknown;
}

export type PocketLinkRole = 'host' | 'screen';

/** pa1b: a single-use link token, good for 10 minutes. */
export interface PocketLink {
  token: string;
  role: PocketLinkRole;
  expires_at: string;
}

export class PocketApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code: string | null,
    /** 422 `invalid`: the server's numbered problems ("1. round 1, question 1: …"). */
    readonly problems: string[] = [],
  ) {
    super(message);
    this.name = 'PocketApiError';
  }
}

// ---- pa4c: quiz drafts ----

export type DraftQuestionKind = 'choice' | 'text' | 'number';
export type DraftStatus = 'checked' | 'yours' | 'not_checked';

export interface DraftQuestion {
  /** 1-based across all rounds; the `:n` of the routes. */
  n: number;
  round: number;
  /** "round 1, question 1" */
  at?: string;
  kind: DraftQuestionKind;
  question: string;
  answer: string | number;
  accepted: string[];
  options?: string[];
  status: DraftStatus;
  source_url?: string;
  /** not_checked only: pa3's failure words. */
  why?: string;
  picture?: unknown;
}

export interface QuizDraft {
  id: string;
  title: string;
  /** writing | checking | dry_run_stopped | ready | open | closed */
  state: string;
  chip: string;
  kwant_id?: string;
  pocket_code: string | null;
  /** The `base` every edit sends back. */
  updated_at: number;
  counts: { questions: number; checked: number; yours: number; not_checked: number };
  /** "40 questions · 39 checked · 1 yours" */
  header: string;
  dry_run: { ok: boolean; summary?: string; problems?: string[] } | null;
  problems: string[];
  rounds: { n: number; title: string | null }[];
  questions: DraftQuestion[];
}

/** The card the bot's pocket_create/pocket_update returns (pa3 `Card`); the pocket card swaps it in after an edit. */
export interface DraftCard {
  id: string;
  state: string;
  chip: string;
  checked: number;
  total: number;
  yours: number;
  failed: { q: number; at: string; url: string; why: string }[];
  problems: string[];
  dry_run?: { ok: boolean; summary: string; problems?: string[] } | null;
}

export interface DraftReply {
  draft: QuizDraft;
  card: DraftCard;
}

export interface QuestionFields {
  question?: string;
  answer?: string;
  accepted?: string[];
  options?: string[];
}

/** The same rule as the server's `keys::valid_code`, loosely: letters and digits, 3-32. */
export const validPocketCode = (code: string) => /^[A-Za-z0-9_-]{3,32}$/.test(code);

const FALLBACK: Record<string, string> = {
  pocket_apps_off: 'Pocket apps are off. Turn them on in Settings first.',
  not_found: 'This pocket app no longer exists.',
};

async function call<T>(method: 'GET' | 'POST' | 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      headers: body === undefined ? { Accept: 'application/json' } : { Accept: 'application/json', 'Content-Type': 'application/json' },
      credentials: 'same-origin',
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
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
    const problems = Array.isArray(data?.problems) ? data.problems.filter((p): p is string => typeof p === 'string') : [];
    throw new PocketApiError(message, res.status, code, problems);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

const one = (code: string) => {
  if (!validPocketCode(code)) throw new PocketApiError(FALLBACK.not_found, 404, 'not_found');
  return `/api/pockets/${encodeURIComponent(code)}`;
};

/** A quiz_drafts id as the card carries it. */
export const validDraftId = (id: string) => /^[A-Za-z0-9_-]{1,64}$/.test(id);
const draftPath = (id: string) => {
  if (!validDraftId(id)) throw new PocketApiError("There's no quiz draft here.", 404, 'not_found');
  return `/api/quiz-drafts/${encodeURIComponent(id)}`;
};
const qPath = (id: string, n: number) => {
  if (!Number.isInteger(n) || n < 1) throw new PocketApiError(`This quiz has no question ${n}.`, 404, 'no_question');
  return `${draftPath(id)}/questions/${n}`;
};

export const pocketApi = {
  list: () => call<PocketList>('GET', '/api/pockets'),
  read: async (code: string) => call<PocketRead>('GET', one(code)),
  /** Stop (before the game) and End quiz (during it) both end the pocket: the link closes. */
  end: async (code: string) => call<{ pocket: PocketCard }>('POST', `${one(code)}/end`),
  /** Final: players, answers and the pocket go now. */
  remove: async (code: string) => call<void>('DELETE', one(code)),
  // pa10a: the host's own calls, person-only on the main port (Lin's src/pocket/host_api.rs, #1074). No /host
  // segment: /host/* lives only on the guest port (/api/pocket/, singular), which refuses another origin.
  /** A named kit action (santa: draw_now, reveal_now, skip_reveal, add_pair, ...). Returns the host view. */
  hostAction: async (code: string, name: string, body?: Record<string, unknown>) => call<unknown>('POST', `${one(code)}/action`, body === undefined ? { name } : { name, body }),
  // pa10b gather: host actions through the same route, named as Lin's src/pocket/gather.rs (pa10b/server-shape.md).
  /** The host's own paid tick on a Yes row. Never shown to members. */
  setPaid: async (code: string, playerId: string, paid: boolean) => pocketApi.hostAction(code, 'set_paid', { player_id: playerId, paid }),
  /** The host copied or shared the draft to `to` (a name). Inside the 48 h window it's a 409 `not_now`: re-read `next_copy_at`. */
  copied: async (code: string, to: string) => pocketApi.hostAction(code, 'copied', { to }),
  /** Break a tie: the host picks a ballot option by its index. */
  pick: async (code: string, option: number) => pocketApi.hostAction(code, 'pick', { option }),
  /** Close a vote now and count it (`/end` is Stop). */
  closeNow: async (code: string) => pocketApi.hostAction(code, 'close_now'),
  /** Break a tie: voting opens again for 24 hours. */
  revote24h: async (code: string) => pocketApi.hostAction(code, 'revote_24h'),
  /** The player's keys stop working; before the draw only for santa. Returns the host view. */
  removePlayer: async (code: string, playerId: string) => call<unknown>('POST', `${one(code)}/remove`, { player_id: playerId }),
  /** A one-time code, exactly once. Nothing else comes back: never a match. */
  resetPin: async (code: string, playerId: string): Promise<ResetCode> => {
    const r = await call<unknown>('POST', `${one(code)}/reset-pin`, { player_id: playerId });
    const ok = r && typeof r === 'object' && typeof (r as ResetCode).code === 'string' && /^[A-Z0-9]{4,8}$/.test((r as ResetCode).code) && typeof (r as ResetCode).expires_at === 'string';
    if (!ok) throw new PocketApiError('The box sent something unexpected. Try again.', 200, 'bad_answer');
    // Only the two fields, whatever else the answer carried.
    return { code: (r as ResetCode).code, expires_at: (r as ResetCode).expires_at };
  },
  /** A fresh single-use link for the host remote or the big screen (pa1b). Every call mints a new one. */
  mintLink: async (code: string, role: PocketLinkRole) => call<PocketLink>('POST', `${one(code)}/link`, { role }),
  /** pa4c: the draft as the editor shows it. Works in every state (open and closed read only). */
  getDraft: async (id: string) => call<DraftReply>('GET', draftPath(id)),
  /** pa4c: a new question at the end of `round` (1-based). The server infers `kind` when it's left out. */
  addQuestion: async (id: string, body: QuestionFields & { round: number; kind?: DraftQuestionKind; base: number }) =>
    call<DraftReply>('POST', `${draftPath(id)}/questions`, body),
  /** pa4c: only the fields sent change; `accepted: []` clears the also-accept list. */
  editQuestion: async (id: string, n: number, body: QuestionFields & { base: number }) => call<DraftReply>('PATCH', qPath(id, n), body),
  deleteQuestion: async (id: string, n: number, base: number) => call<DraftReply>('DELETE', `${qPath(id, n)}?base=${encodeURIComponent(String(base))}`),
  // pa13 front desk (Lin's src/pocket/desk.rs, #1150): the host's two actions; both answer with the same read as GET.
  /** The host's own answer to one guest, marked "host". Clears `waiting`. */
  deskReply: async (code: string, guest: string, text: string) => call<PocketRead>('POST', `${one(code)}/action`, { name: 'reply', body: { guest, text } }),
  /** The host's daily reply limit (10..200, whole numbers; #1150 round 4). Answers with the read; the new cap is summary.counts.daily_cap. */
  deskSetCap: async (code: string, cap: number) => call<PocketRead>('POST', `${one(code)}/action`, { name: 'set_cap', body: { cap } }),
  /** "No reply needed": clears `waiting`, sends nothing to the guest. */
  deskResolve: async (code: string, guest: string) => call<PocketRead>('POST', `${one(code)}/action`, { name: 'resolve', body: { guest } }),
  /** One guest's conversation, host only: the read plus `conversation`. */
  deskGuest: async (code: string, guest: string) => call<PocketRead>('GET', `${one(code)}?view=${encodeURIComponent(`guest:${guest}`)}`),
  /** The Needs-you list (`GET /api/waiting`); the desk keeps only its own `pocket_desk` items (validDeskItems). */
  waiting: async () => call<unknown>('GET', '/api/waiting'),
};

/**
 * The page a minted link opens: the join address (`…/p/<code>`) with the token in the fragment, which browsers never
 * send to a server or put in a Referer. `#remote=` opens the host remote, `#screen=` the big screen.
 */
export function linkUrl(joinUrl: string, role: PocketLinkRole, token: string): string {
  const base = joinUrl.split('#')[0].split('?')[0];
  return `${base}#${role === 'host' ? 'remote' : 'screen'}=${encodeURIComponent(token)}`;
}

/** host/reset-pin's answer: the code and when it stops working (RFC 3339). */
export interface ResetCode {
  code: string;
  expires_at: string;
}
