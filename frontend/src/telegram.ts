// Telegram (0.7.8, design §G): talk to your bots from a Telegram bot the person makes.
// Status, setup, pairing and the settings live under /api/connections/telegram (person-only; the
// fetch wrapper adds `?space=`). The token is never returned by any route, so nothing here holds it
// after the PUT that saves it.
import { personFetch } from './auth';

export type TelegramState = 'off' | 'token_saved' | 'pairing' | 'paired' | 'rejected';
export type TelegramPoll = 'listening' | 'offline' | 'throttled';
export type KindKey = 'question' | 'approval' | 'human_step' | 'stuck' | 'done';
export type Kinds = Record<KindKey, boolean>;

export interface TelegramStatus {
  state: TelegramState;
  bot: { username: string; name: string } | null;
  paired: { name: string; username: string | null; since: string } | null;
  kinds: Kinds;
  allow_approvals: boolean;
  allow_since: string | null;
  status: TelegramPoll;
  queued: number;
  last_message_at: string | null;
  pair_expires_at: string | null;
  pair_locked: boolean;
  undo_until: string | null;
  error: 'token_rejected' | 'conflict' | null;
  app_url: string | null;
  strangers: number;
}

export interface PairLink {
  link: string;
  expires_at: string;
  qr_svg: string;
}

/** The "needs you" kinds in the order the design lists them (G5), with their defaults. */
export const KINDS: { key: KindKey; title: string; help: string }[] = [
  { key: 'question', title: 'Questions with choices', help: 'Answer with buttons in Telegram' },
  { key: 'approval', title: 'Approvals', help: 'What is asked, with an Open button' },
  { key: 'human_step', title: 'Human steps', help: 'Sign-ins, codes and CAPTCHAs: with an Open button' },
  { key: 'stuck', title: 'A bot is stuck or stopped', help: 'With what it was doing' },
  { key: 'done', title: 'A task you started from Telegram is done', help: 'Answers to your messages always come back' },
];
export const DEFAULT_KINDS: Kinds = { question: true, approval: true, human_step: true, stuck: true, done: false };

// Copy (design §G). One place, so tests and screens agree.
export const COPY = {
  privacyLead: 'Telegram can read what you and your bots send in this chat.',
  privacyRest: "Bot chats there aren't end-to-end encrypted, so keep passwords and codes out of it.",
  description: 'Message your bots from Telegram, and get “needs you” alerts there. It uses a Telegram bot you make, and needs no public address.',
  rejected: "Telegram didn't accept this token. Copy it again from BotFather: it's a number, a colon, then about 35 letters and digits. Nothing was saved.",
  unreachable: "Couldn't reach Telegram from this computer. Check its connection and try again. Nothing was saved.",
  expired: 'This link expired after 10 minutes. Nothing was paired.',
  copied: 'Link copied. It works once, for 10 minutes.',
  testSent: 'Test sent. Check Telegram.',
  allowHelpOff: "Off: approvals show what's asked, and you approve in the app. Telegram can't grant anything.",
  never: 'Never from Telegram: production connections, payment floors, human steps and Take over. Those always open the app.',
};

const s = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);
const STATES: TelegramState[] = ['off', 'token_saved', 'pairing', 'paired', 'rejected'];
const POLLS: TelegramPoll[] = ['listening', 'offline', 'throttled'];

/** Reads a status body defensively: unknown fields drop, missing ones take the "off" defaults. */
export function parseStatus(raw: unknown): TelegramStatus {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Record<string, any>;
  const state = STATES.includes(r.state) ? (r.state as TelegramState) : 'off';
  const bot = r.bot && s(r.bot.username) ? { username: String(r.bot.username).replace(/^@/, ''), name: s(r.bot.name) ?? '' } : null;
  const paired = r.paired && s(r.paired.name) ? { name: r.paired.name as string, username: s(r.paired.username)?.replace(/^@/, '') ?? null, since: s(r.paired.since) ?? '' } : null;
  const kinds = { ...DEFAULT_KINDS };
  if (r.kinds && typeof r.kinds === 'object') for (const k of KINDS) if (typeof r.kinds[k.key] === 'boolean') kinds[k.key] = r.kinds[k.key];
  const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? Math.floor(v) : 0);
  return {
    state,
    bot,
    paired: state === 'paired' ? paired : null,
    kinds,
    allow_approvals: r.allow_approvals === true && state === 'paired',
    allow_since: s(r.allow_since),
    status: POLLS.includes(r.status) ? r.status : 'offline',
    queued: n(r.queued),
    last_message_at: s(r.last_message_at),
    pair_expires_at: s(r.pair_expires_at),
    pair_locked: r.pair_locked === true,
    undo_until: s(r.undo_until),
    error: r.error === 'token_rejected' || r.error === 'conflict' ? r.error : null,
    app_url: s(r.app_url),
    strangers: n(r.strangers),
  };
}

export class TelegramError extends Error {
  constructor(message: string, readonly status: number, readonly code: string) {
    super(message);
  }
}

const BASE = '/api/connections/telegram';

async function call<T>(method: string, path: string, body?: unknown, read: (j: unknown) => T = (j) => j as T): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(BASE + path, {
      method,
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new TelegramError("Couldn't reach the app's server. Check this computer's connection and try again.", 0, 'network');
  }
  if (res.status === 204) return read(null);
  const j = await res.json().catch(() => null);
  if (!res.ok) {
    const e = j && typeof j === 'object' ? (j as any).error : null;
    const code = typeof e?.code === 'string' ? e.code : typeof e === 'string' ? e : 'failed';
    const message = typeof e?.message === 'string' && e.message.trim() ? e.message : `Telegram setup failed (${res.status}).`;
    throw new TelegramError(message, res.status, code);
  }
  return read(j);
}

export const telegramApi = {
  status: () => call('GET', '', undefined, parseStatus),
  saveToken: (token: string, app_url?: string) => call('PUT', '/token', app_url ? { token, app_url } : { token }, parseStatus),
  pair: () =>
    call('POST', '/pair', undefined, (j) => {
      const r = (j ?? {}) as Record<string, unknown>;
      return { link: String(r.link ?? ''), expires_at: String(r.expires_at ?? ''), qr_svg: String(r.qr_svg ?? '') } as PairLink;
    }),
  unpair: () => call('POST', '/unpair', undefined, parseStatus),
  undoUnpair: () => call('POST', '/unpair/undo', undefined, parseStatus),
  patch: (body: { kinds?: Partial<Kinds>; allow_approvals?: boolean; app_url?: string | null }) => call('PATCH', '', body, parseStatus),
  test: () => call('POST', '/test', undefined, () => undefined),
  disconnect: () => call('DELETE', '', undefined, () => undefined),
};

/** The setup error line for a failed token check: our own words for the two the design draws, else the server's. */
export function tokenErrorText(e: unknown): { text: string; net: boolean } {
  if (e instanceof TelegramError) {
    if (e.code === 'token_rejected' || e.code === 'token_shape') return { text: COPY.rejected, net: false };
    if (e.code === 'telegram_unreachable' || e.code === 'network') return { text: COPY.unreachable, net: true };
    return { text: e.message, net: false };
  }
  return { text: 'Something went wrong. Nothing was saved.', net: false };
}

/** `m:ss` left until `until`, never below 0:00. */
export function countdown(until: string | null, now: number): { secs: number; text: string } {
  const t = until ? Date.parse(until) : NaN;
  const secs = Number.isFinite(t) ? Math.max(0, Math.ceil((t - now) / 1000)) : 0;
  return { secs, text: `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}` };
}

/** The countdown is spoken only at 5:00 and 1:00 (and the expiry line, which replaces it). */
export const announceAt = (secs: number) => secs === 300 || secs === 60;

/** `t.me/<bot>?start=…`: the code is never printed; Copy link copies the whole link. */
export function shortLink(link: string): string {
  try {
    const u = new URL(link);
    return `${u.host}${u.pathname}${u.search ? '?start=…' : ''}`;
  } catch {
    return 't.me/…';
  }
}

/** Only a real https t.me link is opened or copied; anything else from the server is ignored. */
export const safeLink = (link: string): string | null => {
  try {
    const u = new URL(link);
    return u.protocol === 'https:' && (u.host === 't.me' || u.host === 'telegram.me') ? u.href : null;
  } catch {
    return null;
  }
};

/**
 * The server's QR as an image source. It's drawn into an <img>, never into the page, so nothing in
 * it runs or reaches the DOM. Anything that isn't an SVG gives null (then the link is the way).
 */
export function qrSource(svg: string): string | null {
  const t = svg.trim();
  if (!/^<svg[\s>]/i.test(t) || t.length > 200_000) return null;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(t)}`;
}

/** A clock time, "10:04", in the person's locale. */
export const hhmm = (iso: string | null): string | null => {
  const t = iso ? Date.parse(iso) : NaN;
  return Number.isFinite(t) ? new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;
};
/** "Nov 7". */
export const shortDate = (iso: string | null): string | null => {
  const t = iso ? Date.parse(iso) : NaN;
  return Number.isFinite(t) ? new Date(t).toLocaleDateString([], { month: 'short', day: 'numeric' }) : null;
};

/** The card's status line (G8): tone plus words; "Fix it" is added by the card for `rejected`. */
export function statusLine(st: TelegramStatus): { tone: 'ok' | 'warn' | 'muted' | 'bad'; text: string } {
  if (st.state === 'rejected' || st.error === 'token_rejected') return { tone: 'bad', text: 'Telegram rejected the token.' };
  if (st.error === 'conflict') return { tone: 'warn', text: 'Paused: another program is reading this Telegram bot. Stop it, and your bots pick up again.' };
  const wait = (n: number) => (n === 1 ? '1 message waits and goes' : `${n} messages wait and go`);
  if (st.status === 'offline') return { tone: 'warn', text: st.queued ? `Paused: this computer is offline. ${wait(st.queued)} out when it's back.` : 'Paused: this computer is offline.' };
  if (st.status === 'throttled') return { tone: 'muted', text: `Sending slower: Telegram limits how fast bots can send.${st.queued ? ` ${st.queued} waiting.` : ''}` };
  const last = hhmm(st.last_message_at);
  return { tone: 'ok', text: last ? `Listening · last message ${last}` : 'Listening' };
}

/** "Sam @sam_example", or the name alone when the account has no @username. */
export const pairedName = (p: { name: string; username: string | null }) => (p.username ? `${p.name} @${p.username}` : p.name);

/** Window events App re-dispatches from the event stream (`telegram.status|paired|expired`). */
export const TG_EVENT = 'kwant:telegram';
export type TelegramEventDetail = { type: 'telegram.status' | 'telegram.paired' | 'telegram.expired'; data: Record<string, unknown> };

/** An event for this space (or one that names none). With no current space known, every one counts. */
export const forThisSpace = (eventSpace: unknown, current: string | null): boolean =>
  typeof eventSpace !== 'string' || !eventSpace || !current || eventSpace === current;

/** Whether a message came in from Telegram (`meta.source`). */
export function fromTelegram(meta: Record<string, unknown> | null): boolean {
  return meta?.source === 'telegram';
}
/** Whether a bot reply also went out to Telegram (`meta.sent_to_telegram`). */
export function sentToTelegram(meta: Record<string, unknown> | null): boolean {
  return meta?.sent_to_telegram === true;
}
