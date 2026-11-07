// Today's spending stop (0.7.4): two optional daily limits, fresh tokens and dollars, in the person's
// time zone. Past either, kwnt holds every new turn until the person presses Continue today, raises a
// limit, or the day ends. The numbers on screen are the server's; nothing here adds or estimates spend.
import { usdText } from './missions';
import { compactTokens } from './modelUsage';

export const STOP_TOKENS_MAX = 10_000_000_000;
export const STOP_USD_MAX = 100_000;
export const SPEND_STOPPED = 'spend_stopped';

// ---------------------------------------------------------------- Settings fields

/** What GET /api/settings says, as field text ("" is off); null on a server without the stop. */
export function stopFieldsFromSettings(d: unknown): { tokens: string; usd: string } | null {
  if (!d || typeof d !== 'object') return null;
  const o = d as Record<string, unknown>;
  if (typeof o.daily_spend_stop_tokens !== 'number' && typeof o.daily_spend_stop_usd !== 'number') return null;
  const t = typeof o.daily_spend_stop_tokens === 'number' && o.daily_spend_stop_tokens > 0 ? String(o.daily_spend_stop_tokens) : '';
  const u = typeof o.daily_spend_stop_usd === 'number' && o.daily_spend_stop_usd > 0 ? String(o.daily_spend_stop_usd) : '';
  return { tokens: t, usd: u };
}

/** "200k", "1.5M", "200,000" → tokens; "" → 0 (off); null when it isn't a whole amount. */
export function stopTokensFrom(raw: string): number | null {
  const s = raw.trim().toLowerCase().replace(/[\s_,]/g, '').replace(/tokens?$/, '');
  if (!s) return 0;
  const m = /^(\d+(?:\.\d+)?)([km]?)$/.exec(s);
  if (!m) return null;
  const n = Math.round(Number(m[1]) * (m[2] === 'k' ? 1_000 : m[2] === 'm' ? 1_000_000 : 1));
  return Number.isFinite(n) && n <= STOP_TOKENS_MAX ? n : null;
}

export function stopTokensError(raw: string): string | null {
  if (stopTokensFrom(raw) !== null) return null;
  const s = raw.trim().toLowerCase().replace(/[\s_,]/g, '').replace(/tokens?$/, '');
  if (/^(\d+(?:\.\d+)?)([km]?)$/.test(s)) return 'At most 10 billion tokens a day.';
  return 'Enter a number of tokens, like 500k or 2M, or leave it empty.';
}

/** "5", "$5", "2.50" → dollars; "" → 0 (off); null when it isn't an amount. */
export function stopUsdFrom(raw: string): number | null {
  const s = raw.trim().replace(/^\$/, '').replace(/[\s,]/g, '');
  if (!s) return 0;
  if (!/^\d+(?:\.\d{1,2})?$/.test(s)) return null;
  const n = Number(s);
  return Number.isFinite(n) && n <= STOP_USD_MAX ? n : null;
}

export function stopUsdError(raw: string): string | null {
  if (stopUsdFrom(raw) !== null) return null;
  const s = raw.trim().replace(/^\$/, '').replace(/[\s,]/g, '');
  if (/^\d+(?:\.\d{1,2})?$/.test(s)) return 'At most $100,000 a day.';
  return 'Enter dollars, like 5 or 2.50, or leave it empty.';
}

/** The POST /api/settings part, or null while a field can't be saved. */
export function stopBody(f: { tokens: string; usd: string }): { daily_spend_stop_tokens: number; daily_spend_stop_usd: number } | null {
  const t = stopTokensFrom(f.tokens);
  const u = stopUsdFrom(f.usd);
  if (t === null || u === null) return null;
  return { daily_spend_stop_tokens: t, daily_spend_stop_usd: u };
}

// ---------------------------------------------------------------- GET /api/usage/today

export interface UsageToday {
  day: string;
  timezone: string;
  today_tokens: number;
  today_usd: number;
  stop_tokens: number;
  stop_usd: number;
  continued: boolean;
  stopped: boolean;
  over: 'tokens' | 'usd' | null;
  day_ends_at: string;
}

const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : 0);
const text = (v: unknown): string => (typeof v === 'string' ? v : '');

export function parseUsageToday(x: unknown): UsageToday | null {
  if (!x || typeof x !== 'object') return null;
  const o = x as Record<string, unknown>;
  if (typeof o.today_tokens !== 'number' && typeof o.today_usd !== 'number') return null;
  return {
    day: text(o.day),
    timezone: text(o.timezone),
    today_tokens: num(o.today_tokens),
    today_usd: num(o.today_usd),
    stop_tokens: num(o.stop_tokens),
    stop_usd: num(o.stop_usd),
    continued: o.continued === true,
    stopped: o.stopped === true,
    over: o.over === 'tokens' || o.over === 'usd' ? o.over : null,
    day_ends_at: text(o.day_ends_at),
  };
}

/** "Today: 17,420 tokens · $0.84", the server's own figures. */
export function todayLine(u: Pick<UsageToday, 'today_tokens' | 'today_usd'>): string {
  const n = Math.round(u.today_tokens);
  return `Today: ${n.toLocaleString('en-US')} ${n === 1 ? 'token' : 'tokens'} · ${usdText(u.today_usd) ?? '$0.00'}`;
}

/** Under the line: whether the stop holds new work, or Continue still applies. */
export function todayState(u: UsageToday): string | null {
  if (u.stopped) return 'The stop is on: bots won’t start new work today unless you continue.';
  if (u.continued && u.over) return 'Continued for today. The limit applies again tomorrow.';
  return null;
}

type Fetch = typeof fetch;

export class SpendStopError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function call(send: Fetch, url: string, init?: RequestInit): Promise<UsageToday> {
  let r: Response;
  try {
    r = await send(url, { cache: 'no-store', ...init });
  } catch {
    throw new SpendStopError('Couldn’t reach the server. Try again.', 0);
  }
  let body: unknown = null;
  try {
    body = await r.json();
  } catch {
    /* not JSON */
  }
  if (!r.ok) {
    const err = body && typeof (body as { error?: unknown }).error === 'string' ? (body as { error: string }).error : '';
    if (r.status === 403) throw new SpendStopError('Only you can continue today, signed in on this device.', 403);
    throw new SpendStopError(err && r.status === 400 ? err : 'Couldn’t continue. Try again.', r.status);
  }
  const u = parseUsageToday(body);
  if (!u) throw new SpendStopError('Couldn’t read today’s use.', r.status);
  return u;
}

export const spendStopApi = (send: Fetch = (...a) => fetch(...a)) => ({
  today: () => call(send, '/api/usage/today'),
  /** Lifts the stop until the day ends (person only). */
  continueToday: () => call(send, '/api/usage/today/continue', { method: 'POST' }),
});

// ---------------------------------------------------------------- the chat notice and Needs you

export interface SpendStoppedMeta {
  kind: 'spend_stopped';
  day: string;
  over: 'tokens' | 'usd' | null;
  used_tokens: number;
  stop_tokens: number;
  used_usd: number;
  stop_usd: number;
  day_ends_at: string;
}

function readStopped(o: Record<string, unknown>): SpendStoppedMeta {
  return {
    kind: 'spend_stopped',
    day: text(o.day),
    over: o.over === 'tokens' || o.over === 'usd' ? o.over : null,
    used_tokens: num(o.used_tokens),
    stop_tokens: num(o.stop_tokens),
    used_usd: num(o.used_usd),
    stop_usd: num(o.stop_usd),
    day_ends_at: text(o.day_ends_at),
  };
}

export function parseSpendStopped(meta: string | null | undefined): SpendStoppedMeta | null {
  if (!meta) return null;
  try {
    const m = JSON.parse(meta);
    return m && typeof m === 'object' && m.kind === SPEND_STOPPED ? readStopped(m) : null;
  } catch {
    return null;
  }
}

/** "17.4k of 10k tokens" or "$0.84 of $0.50": the limit it reached, from the server's numbers. */
export function stoppedDetail(m: Pick<SpendStoppedMeta, 'over' | 'used_tokens' | 'stop_tokens' | 'used_usd' | 'stop_usd'>): string | null {
  if (m.over === 'usd' && m.stop_usd > 0) return `${usdText(m.used_usd) ?? '$0.00'} of ${usdText(m.stop_usd)} today`;
  if (m.stop_tokens > 0) return `${compactTokens(m.used_tokens)} of ${compactTokens(m.stop_tokens)} fresh tokens today`;
  return null;
}

/** The Needs-you item from GET /api/waiting, when the stop holds work. */
export interface SpendStoppedItem extends SpendStoppedMeta {
  id: string;
  text: string;
}

export function spendStoppedItem(body: unknown): SpendStoppedItem | null {
  const items = body && typeof body === 'object' ? (body as { items?: unknown }).items : null;
  if (!Array.isArray(items)) return null;
  const it = items.find((i) => i && typeof i === 'object' && (i as { kind?: unknown }).kind === SPEND_STOPPED) as Record<string, unknown> | undefined;
  if (!it) return null;
  return { ...readStopped(it), id: text(it.id), text: text(it.text) };
}

export async function fetchSpendStopped(send: Fetch = (...a) => fetch(...a)): Promise<SpendStoppedItem | null> {
  try {
    const r = await send('/api/waiting', { cache: 'no-store' });
    return r.ok ? spendStoppedItem(await r.json()) : null;
  } catch {
    return null;
  }
}

/** A tap on the `spend_stopped` notification: the app was opened at `/?notify=spend_stopped`. */
export const notifyFromQuery = (search: string): string | null => new URLSearchParams(search).get('notify');

// ---------------------------------------------------------------- skips in each feature's history

/** True when a routine run, task event or AutoFix hold was the stop's doing. */
export const isSpendStopReason = (reason: unknown): boolean =>
  typeof reason === 'string' && (reason.startsWith(SPEND_STOPPED) || reason === `skipped: ${SPEND_STOPPED}` || reason.startsWith('skipped: today’s spending stop') || reason.startsWith("skipped: today's spending stop"));

export const ROUTINE_SKIPPED_TEXT = 'Skipped: today’s spending stop was on.';

/** A routine run's note: the stop's skip reads plainly, not as an error. */
export function routineRunNote(r: { status: string; error: string | null }): { text: string; quiet: boolean } | null {
  if (!r.error) return null;
  if (r.status === 'skipped' && isSpendStopReason(r.error)) return { text: ROUTINE_SKIPPED_TEXT, quiet: true };
  return { text: r.error, quiet: false };
}

/** Mission task lines for the stop, or null when the event isn't the stop's. */
export function spendStopEventText(e: { kind: string; to_state?: string | null; detail?: Record<string, unknown> | null }): string | null {
  const reason = e.detail?.reason;
  if (!isSpendStopReason(reason)) return null;
  switch (e.kind) {
    case 'skipped':
      return 'Skipped for now: today’s spending stop is on';
    case 'released':
      return 'Back to Ready: today’s spending stop is on, so it didn’t start';
    case 'autofix_held':
      return 'AutoFix waits: today’s spending stop is on. It retries once the stop lifts';
    case 'state':
      return e.to_state === 'cancelled' ? 'Cancelled: today’s spending stop skipped this routine run' : null;
    default:
      return null;
  }
}

// ---------------------------------------------------------------- a notification tap

// Set by a tap on the spending-stop notification (cold open or with the app running); the Needs-you
// item takes it once and moves focus to its Continue button. In memory only.
let focusPending = false;
export const requestSpendStopFocus = (): void => {
  focusPending = true;
};
export const takeSpendStopFocus = (): boolean => {
  const was = focusPending;
  focusPending = false;
  return was;
};
