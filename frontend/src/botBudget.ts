// Per-bot monthly budget (0.9.5, backend G2 since 0.9.0): fresh tokens and/or dollars per calendar
// month in the person's time zone. At 80% the bot warns once; at 100% it finishes its turn and starts
// nothing new until the person raises the limit, continues this month, or the next month starts.
// Every number on screen is the server's (`src/server/bot_budget.rs`); nothing here counts spend.
import { usdText } from './missions';
import { compactTokens } from './modelUsage';
import { stopTokensFrom, stopUsdFrom } from './spendStop';

export type BudgetState = 'off' | 'under' | 'warn' | 'paused' | 'continued';

/** `GET /api/kwants/:id/budget`, one bot's month against its budget. */
export interface BotBudget {
  kwant_id: string;
  kwant_name: string;
  /** `YYYY-MM` in `timezone`. */
  month: string;
  timezone: string;
  used_tokens: number;
  used_usd: number;
  /** 0 when off. */
  budget_tokens: number;
  /** 0 when off. */
  budget_usd: number;
  /** The higher of the set limits' percentages; 0 when off. */
  percent: number;
  over: 'tokens' | 'usd' | null;
  state: BudgetState;
  paused_since: string | null;
  /** Local 00:00 on the 1st of next month. */
  resets_at: string;
  /** Some turn this month reported no usage: the numbers are partly the server's estimate. */
  estimated: boolean;
  /** No turn this month reported a cost: dollars can't be counted for this bot's engine. */
  cost_unreported: boolean;
}

const STATES: readonly BudgetState[] = ['off', 'under', 'warn', 'paused', 'continued'];
const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : 0);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

export function parseBudget(x: unknown): BotBudget | null {
  if (!x || typeof x !== 'object') return null;
  const o = x as Record<string, unknown>;
  if (typeof o.kwant_id !== 'string' || !STATES.includes(o.state as BudgetState)) return null;
  return {
    kwant_id: o.kwant_id,
    kwant_name: str(o.kwant_name),
    month: str(o.month),
    timezone: str(o.timezone),
    used_tokens: num(o.used_tokens),
    used_usd: num(o.used_usd),
    budget_tokens: num(o.budget_tokens),
    budget_usd: num(o.budget_usd),
    percent: num(o.percent),
    over: o.over === 'tokens' || o.over === 'usd' ? o.over : null,
    state: o.state as BudgetState,
    paused_since: typeof o.paused_since === 'string' ? o.paused_since : null,
    resets_at: str(o.resets_at),
    estimated: o.estimated === true,
    cost_unreported: o.cost_reported === false,
  };
}

export const hasBudget = (b: Pick<BotBudget, 'state'> | null | undefined): boolean => !!b && b.state !== 'off';
/** The bot is paused at its budget: no new work until the person acts or the month ends. */
export const isPaused = (b: Pick<BotBudget, 'state'> | null | undefined): boolean => b?.state === 'paused';
/** 80% or more and not yet paused (and not continued). */
export const isWarn = (b: Pick<BotBudget, 'state'> | null | undefined): boolean => b?.state === 'warn';

/** The meter's fill, 0–100. */
export const meterPercent = (b: Pick<BotBudget, 'percent'>): number => Math.max(0, Math.min(100, Math.round(b.percent)));

export const PAUSED_LINE = 'Paused: monthly budget reached';

function monthName(month: string): string {
  const m = /^(\d{4})-(\d{2})$/.exec(month);
  if (!m) return 'this month';
  return new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, 1)).toLocaleDateString('en-US', { month: 'long', timeZone: 'UTC' });
}

/** "December 1", the day the budget starts over, in the budget's own time zone. */
export function resetDay(b: Pick<BotBudget, 'resets_at' | 'timezone'>): string | null {
  const d = new Date(b.resets_at);
  if (!b.resets_at || Number.isNaN(d.getTime())) return null;
  const opts: Intl.DateTimeFormatOptions = { month: 'long', day: 'numeric' };
  try {
    return d.toLocaleDateString('en-US', { ...opts, timeZone: b.timezone || 'UTC' });
  } catch {
    return d.toLocaleDateString('en-US', { ...opts, timeZone: 'UTC' });
  }
}

/** Which limit the line talks about: the one hit, else the one with the higher share. */
function leadIsUsd(b: Pick<BotBudget, 'over' | 'used_tokens' | 'budget_tokens' | 'used_usd' | 'budget_usd'>): boolean {
  if (b.over) return b.over === 'usd';
  const pt = b.budget_tokens > 0 ? b.used_tokens / b.budget_tokens : -1;
  const pu = b.budget_usd > 0 ? b.used_usd / b.budget_usd : -1;
  return pu > pt;
}

/** "1.6M of 2M fresh tokens in November" or "$16.40 of $20.00 in November" (with "about" when estimated). */
export function usedLine(b: Pick<BotBudget, 'over' | 'used_tokens' | 'budget_tokens' | 'used_usd' | 'budget_usd' | 'month' | 'estimated'>): string {
  const about = b.estimated ? 'about ' : '';
  const month = monthName(b.month);
  if (b.budget_tokens <= 0 && b.budget_usd <= 0)
    return `${about}${compactTokens(b.used_tokens)} fresh tokens${b.used_usd > 0 ? ` · ${usdText(b.used_usd)}` : ''} in ${month}`;
  if (leadIsUsd(b)) return `${about}${usdText(b.used_usd) ?? '$0.00'} of ${usdText(b.budget_usd)} in ${month}`;
  return `${about}${compactTokens(b.used_tokens)} of ${compactTokens(b.budget_tokens)} fresh tokens in ${month}`;
}

/** The line under the meter: what the bot does now, in plain words. */
export function stateLine(b: BotBudget): string {
  const day = resetDay(b);
  const on = day ? ` on ${day}` : ' next month';
  const until = day ? ` until ${day}` : ' until next month';
  switch (b.state) {
    case 'off':
      return 'No budget. This bot can use as much as it needs.';
    case 'under':
      return `Starts over${on}.`;
    case 'warn':
      return `${meterPercent(b)}% used. At 100% it finishes what it’s doing and starts nothing new${until}.`;
    case 'paused':
      return `${PAUSED_LINE}. It starts nothing new until you raise the limit or continue this month. It starts over${on}.`;
    case 'continued':
      return `Continued for ${monthName(b.month)}: the limit is ignored${until}.`;
  }
}

// ---------------------------------------------------------------- the settings fields

export interface BudgetFields {
  tokens: string;
  usd: string;
}

export const fieldsFromBudget = (b: Pick<BotBudget, 'budget_tokens' | 'budget_usd'> | null): BudgetFields => ({
  tokens: b && b.budget_tokens > 0 ? String(b.budget_tokens) : '',
  usd: b && b.budget_usd > 0 ? String(b.budget_usd) : '',
});

export function budgetTokensError(raw: string): string | null {
  if (stopTokensFrom(raw) !== null) return null;
  const s = raw.trim().toLowerCase().replace(/[\s_,]/g, '').replace(/tokens?$/, '');
  if (/^(\d+(?:\.\d+)?)([km]?)$/.test(s)) return 'At most 10 billion tokens a month.';
  return 'Enter a number of tokens, like 500k or 2M, or leave it empty.';
}

export function budgetUsdError(raw: string): string | null {
  if (stopUsdFrom(raw) !== null) return null;
  const s = raw.trim().replace(/^\$/, '').replace(/[\s,]/g, '');
  if (/^\d+(?:\.\d{1,2})?$/.test(s)) return 'At most $100,000 a month.';
  return 'Enter dollars, like 20 or 2.50, or leave it empty.';
}

/** The POST body (0 or null turns a limit off), or null while a field can't be saved. */
export function budgetBody(f: BudgetFields): { budget_tokens: number | null; budget_usd: number | null } | null {
  const t = stopTokensFrom(f.tokens);
  const u = stopUsdFrom(f.usd);
  if (t === null || u === null) return null;
  return { budget_tokens: t > 0 ? t : null, budget_usd: u > 0 ? u : null };
}

/** True when the fields say something other than what is saved. */
export function fieldsChanged(f: BudgetFields, b: Pick<BotBudget, 'budget_tokens' | 'budget_usd'> | null): boolean {
  const body = budgetBody(f);
  if (!body) return true;
  return (body.budget_tokens ?? 0) !== (b?.budget_tokens ?? 0) || (body.budget_usd ?? 0) !== (b?.budget_usd ?? 0);
}

// ---------------------------------------------------------------- the API

export class BudgetError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

type Fetch = typeof fetch;

async function call<T>(send: Fetch, url: string, read: (b: unknown) => T | null, init: RequestInit | undefined, failed: string): Promise<T> {
  let r: Response;
  try {
    r = await send(url, { cache: 'no-store', ...(init?.body ? { headers: { 'Content-Type': 'application/json' } } : {}), ...init });
  } catch {
    throw new BudgetError('Couldn’t reach the server. Try again.', 0);
  }
  let body: unknown = null;
  try {
    body = await r.json();
  } catch {
    /* not JSON */
  }
  if (!r.ok) {
    const err = body && typeof (body as { error?: unknown }).error === 'string' ? (body as { error: string }).error : '';
    if (r.status === 403) throw new BudgetError('Only you can change a bot’s budget, signed in on this device.', 403);
    throw new BudgetError(err && (r.status === 400 || r.status === 409) ? err : failed, r.status);
  }
  const v = read(body);
  if (v === null) throw new BudgetError('Couldn’t read the budget.', r.status);
  return v;
}

export interface BudgetList {
  month: string;
  resets_at: string;
  bots: BotBudget[];
}

export function parseBudgetList(x: unknown): BudgetList | null {
  if (!x || typeof x !== 'object') return null;
  const o = x as Record<string, unknown>;
  if (!Array.isArray(o.bots)) return null;
  return { month: str(o.month), resets_at: str(o.resets_at), bots: o.bots.map(parseBudget).filter((b): b is BotBudget => !!b) };
}

const enc = encodeURIComponent;

export const botBudgetApi = (send: Fetch = (...a) => fetch(...a)) => ({
  get: (id: string) => call(send, `/api/kwants/${enc(id)}/budget`, parseBudget, undefined, 'Couldn’t load the budget.'),
  set: (id: string, body: { budget_tokens: number | null; budget_usd: number | null }) =>
    call(send, `/api/kwants/${enc(id)}/budget`, parseBudget, { method: 'POST', body: JSON.stringify(body) }, 'Couldn’t save the budget. Try again.'),
  /** Ignores the budget until the month ends (person only). */
  continueMonth: (id: string) => call(send, `/api/kwants/${enc(id)}/budget/continue`, parseBudget, { method: 'POST' }, 'Couldn’t continue. Try again.'),
  /** Undoes Continue: the bot pauses at its budget again. */
  pauseAgain: (id: string) => call(send, `/api/kwants/${enc(id)}/budget/continue`, parseBudget, { method: 'DELETE' }, 'Couldn’t save. Try again.'),
  list: () => call(send, '/api/usage/budgets', parseBudgetList, undefined, 'Couldn’t load the budgets.'),
});

// ---------------------------------------------------------------- the space's budgets, shared

// One list for the sidebar rows, the chat banner, Needs you and Settings › Usage. It is re-read on
// a budget notice (warn, stop, resumed, continued), after any change made here, and every minute.
export interface BudgetSnapshot {
  byBot: Readonly<Record<string, BotBudget>>;
  list: BudgetList | null;
  error: string | null;
  loaded: boolean;
}

const EMPTY: BudgetSnapshot = { byBot: {}, list: null, error: null, loaded: false };
let snapshot: BudgetSnapshot = EMPTY;
const listeners = new Set<() => void>();
let inflight: Promise<void> | null = null;
let fetcher: Fetch = (...a) => fetch(...a);

export const budgetStore = {
  get: (): BudgetSnapshot => snapshot,
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  /** Puts one bot's fresh record in (the answer to a save, continue or pause-again). */
  put(b: BotBudget) {
    const bots = snapshot.list ? snapshot.list.bots.map((x) => (x.kwant_id === b.kwant_id ? b : x)) : [b];
    if (snapshot.list && !bots.some((x) => x.kwant_id === b.kwant_id)) bots.push(b);
    set({ ...snapshot, byBot: { ...snapshot.byBot, [b.kwant_id]: b }, list: snapshot.list ? { ...snapshot.list, bots } : snapshot.list });
  },
  refresh(): Promise<void> {
    if (inflight) return inflight;
    inflight = botBudgetApi(fetcher)
      .list()
      .then(
        (list) => set({ byBot: Object.fromEntries(list.bots.map((b) => [b.kwant_id, b])), list, error: null, loaded: true }),
        (e: unknown) => set({ ...snapshot, error: e instanceof Error ? e.message : 'Couldn’t load the budgets.', loaded: true }),
      )
      .finally(() => {
        inflight = null;
      });
    return inflight;
  },
  /** Tests and a space switch: forget everything (and use `send` from now on). */
  reset(send?: Fetch) {
    if (send) fetcher = send;
    inflight = null;
    set(EMPTY);
  },
};

function set(next: BudgetSnapshot) {
  snapshot = next;
  for (const fn of [...listeners]) fn();
}

/** Bots that need the person: paused first, then at 80% or more, highest first. */
export function budgetsNeedingYou(list: readonly BotBudget[]): BotBudget[] {
  return list.filter((b) => b.state === 'paused' || b.state === 'warn').sort((a, b) => (a.state === b.state ? b.percent - a.percent : a.state === 'paused' ? -1 : 1));
}

// ---------------------------------------------------------------- chat lines

export const BUDGET_KINDS = ['bot_budget_stopped', 'bot_budget_warn', 'bot_budget_resumed', 'bot_budget_continued'] as const;
export type BudgetKind = (typeof BUDGET_KINDS)[number];

export interface BudgetNoticeMeta {
  kind: BudgetKind;
  kwant_id: string;
  kwant_name: string;
  month: string;
  percent: number;
  detail: string;
}

function metaObject(meta: unknown): Record<string, unknown> | null {
  if (!meta) return null;
  if (typeof meta === 'object') return meta as Record<string, unknown>;
  if (typeof meta !== 'string') return null;
  try {
    const m = JSON.parse(meta);
    return m && typeof m === 'object' ? m : null;
  } catch {
    return null;
  }
}

/** A budget line's meta (a string from the message list, or an object from a live event). */
export function parseBudgetNotice(meta: unknown): BudgetNoticeMeta | null {
  const o = metaObject(meta);
  if (!o || !BUDGET_KINDS.includes(o.kind as BudgetKind) || typeof o.kwant_id !== 'string') return null;
  return { kind: o.kind as BudgetKind, kwant_id: o.kwant_id, kwant_name: str(o.kwant_name), month: str(o.month), percent: num(o.percent), detail: str(o.detail) };
}

/** True for any message that changes a bot's budget state, so the shared list is re-read. */
export const isBudgetMessage = (m: { meta?: unknown } | null | undefined): boolean => !!m && parseBudgetNotice(m.meta) !== null;

// ---------------------------------------------------------------- "Raise the limit" focus

// Set by Raise the limit (a notice, Needs you, the chat banner): the bot's settings open on Usage
// and the budget section puts focus on its first field once. In memory only.
let focusPending: string | null = null;
export const requestBudgetFocus = (kwantId: string): void => {
  focusPending = kwantId;
};
export const takeBudgetFocus = (kwantId: string): boolean => {
  if (focusPending !== kwantId) return false;
  focusPending = null;
  return true;
};
