// 0.7.6 O2 "Get started": the one stepped card in #general (Choose your AI → Sign in → Pick a team).
// Design: next-076/onboarding/design.md §3–§13, Ada's C1–C6 in review.md. This file is the logic only
// (which step is open, the words for each state, the sign-in API for three engines); the card is
// components/GetStarted.tsx. Installed and signed-in always come from the server (/api/engines,
// /api/setup/status), so a reopened app resumes where it was; only the chosen AI and "dismissed" are
// kept in this browser.

import type { EngineId, EngineInfo } from './engines';
import { isUsable } from './engines';
import type { SetupItem } from './setupInstall';

/** The four tiles (design §5.1). */
export type AiChoice = 'claude' | 'codex' | 'grok' | 'key';
/** Engines that sign in inside the app (Lin's O3 routes). */
export type LoginEngine = 'claude' | 'codex' | 'grok';

export const CHOICES: Array<{ id: AiChoice; name: string; line: string }> = [
  { id: 'claude', name: 'Claude', line: 'Claude Pro or Max plan' },
  { id: 'codex', name: 'ChatGPT', line: 'Your ChatGPT plan, through Codex' },
  { id: 'grok', name: 'Grok', line: 'Your Grok account' },
  { id: 'key', name: 'API key', line: 'Anthropic, OpenAI or xAI' },
];

export const choiceName = (c: AiChoice) => CHOICES.find((x) => x.id === c)?.name ?? c;
/** What the CLI is called while it is being got ready ("Getting Claude ready…"). */
export const cliName = (c: LoginEngine) => (c === 'codex' ? 'Codex' : c === 'grok' ? 'Grok' : 'Claude');

const KEY_ENGINES: EngineId[] = ['anthropic_api', 'openai_api', 'xai_api'];
/** Today's engine order, for "already signed in" when several are (design §5.1). */
const ORDER: EngineId[] = ['claude', 'codex', 'grok', 'anthropic_api', 'openai_api', 'xai_api'];

/** The tile an engine belongs to. Ollama and custom APIs aren't tiles; they still count as "can run". */
export const choiceOf = (id: EngineId): AiChoice | null =>
  id === 'claude' || id === 'codex' || id === 'grok' ? id : KEY_ENGINES.includes(id) ? 'key' : null;

/** The first tile that can already run, in today's engine order; null when none. */
export function readyChoice(engines: EngineInfo[] | null | undefined): AiChoice | null {
  if (!engines) return null;
  for (const id of ORDER) {
    const e = engines.find((x) => x.id === id);
    if (e && isUsable(e)) return choiceOf(id);
  }
  return null;
}

/** What a tile says under its line (design §5.1, T3): signed in, installed but signed out, or nothing. */
export type TileStatus = 'signed_in' | 'installed' | null;
export function tileStatus(engines: EngineInfo[] | null | undefined, c: AiChoice): TileStatus {
  if (!engines) return null;
  if (c === 'key') return engines.some((e) => KEY_ENGINES.includes(e.id) && isUsable(e)) ? 'signed_in' : null;
  const e = engines.find((x) => x.id === c);
  if (!e || e.enabled === false) return null;
  if (isUsable(e)) return 'signed_in';
  return e.status === 'signed_out' ? 'installed' : null;
}

/** Whether the chosen AI's CLI still has to be installed before it can sign in. */
export function needsInstall(engines: EngineInfo[] | null | undefined, c: AiChoice): boolean {
  if (c === 'key' || !engines) return false;
  const e = engines.find((x) => x.id === c);
  return !e || e.status === 'not_installed';
}

/** Whether the server can install it here (setup status); null while unknown. */
export function installable(items: SetupItem[] | null, c: AiChoice): boolean | null {
  if (c === 'key') return false;
  if (!items) return null;
  const row = items.find((i) => i.id === c);
  return !!row && (row.state !== 'missing' || row.can_install);
}

export type StepNo = 1 | 2 | 3;

export interface StepsInput {
  engines: EngineInfo[] | null;
  /** The tile the person picked (stored); null before. */
  chosen: AiChoice | null;
  /** Step 3 was opened early while an install is slow (F8). */
  teamEarly?: boolean;
  /** The person pressed Change on step 1. */
  changing?: boolean;
}

export interface StepsView {
  open: StepNo;
  /** Step 1 shows as done, with this value ("Claude, installed"). */
  done1: boolean;
  done2: boolean;
  /** The AI the card is about: the stored choice, or the one found signed in. */
  choice: AiChoice | null;
  /** D2: nothing was chosen here and an account already worked on first look. */
  foundReady: boolean;
}

/**
 * Which step is open (design §3). Exactly one is. Steps never go backwards by themselves: once the
 * chosen AI can run, steps 1–2 are done and 3 is open; Change reopens 1.
 */
export function stepsView({ engines, chosen, teamEarly = false, changing = false }: StepsInput): StepsView {
  const ready = readyChoice(engines);
  const choice = chosen ?? ready;
  const foundReady = !chosen && !!ready;
  const chosenReady = !!choice && tileStatus(engines, choice) === 'signed_in';
  if (changing) return { open: 1, done1: false, done2: false, choice, foundReady };
  if (chosenReady) return { open: 3, done1: true, done2: true, choice, foundReady };
  if (!choice) return { open: 1, done1: false, done2: false, choice, foundReady };
  return { open: teamEarly ? 3 : 2, done1: true, done2: false, choice, foundReady };
}

/** Step 1's value when done ("Claude, installed" / "Claude, already signed in on this Mac"). */
export function step1Value(v: StepsView, engines: EngineInfo[] | null, where: string): string {
  if (!v.choice) return '';
  const name = choiceName(v.choice);
  if (v.foundReady) return `${name}, already signed in ${where}`;
  if (v.choice === 'key') return name;
  return needsInstall(engines, v.choice) ? name : `${name}, installed`;
}

/** "on this Mac", "on this computer", or "on {host}" for a server opened on a phone (design §10). */
export function whereLabel(opts: { host?: string | null; mac?: boolean }): string {
  if (opts.host) return `on ${opts.host}`;
  return opts.mac ? 'on this Mac' : 'on this computer';
}

/** The hero's title and subtitle (design §4). */
export function heroText(v: StepsView, opts: { host?: string | null; mac?: boolean }): { title: string; sub: string } {
  const name = v.choice ? choiceName(v.choice) : '';
  if (v.open === 3 && v.done2 && v.foundReady)
    return {
      title: `${name} is ready ${opts.host ? `on ${opts.host}` : opts.mac ? 'on this Mac' : 'on this computer'}`,
      sub: 'You were already signed in. Pick a team and say hello.',
    };
  if (v.open === 3 && v.done2) return { title: `${name} is ready`, sub: 'Last step: who should your bots be?' };
  return {
    title: 'Welcome to kwant bots',
    sub: opts.host
      ? 'Choose your AI and sign in. It installs on your server, not on this phone.'
      : 'Choose your AI and sign in. The rest installs by itself.',
  };
}

// ------------------------------------------------------------------ install failures (design §9)

export type InstallFailKind = 'offline' | 'network' | 'disk' | 'permission' | 'other';

/** The job's error and last lines mapped to one of four kinds; the raw text only goes to Copy details. */
export function installFailKind(text: string | null | undefined, online = true): InstallFailKind {
  if (!online) return 'offline';
  const t = (text || '').toLowerCase();
  if (/no space|enospc|disk full|not enough (free )?space/.test(t)) return 'disk';
  if (/sudo|permission denied|eacces|operation not permitted|needs? root|as root/.test(t)) return 'permission';
  if (/could not resolve|network|timed out|timeout|connection (reset|refused)|econn|enotfound|curl: \(\d+\)|tls|ssl|download/.test(t)) return 'network';
  return 'other';
}

export interface FailView {
  tone: 'warn' | 'bad';
  title: string;
  reason: string;
}

export function installFailView(kind: InstallFailKind, name: string, where: string, host?: string | null): FailView {
  const onHost = host ? ` on ${host}` : '';
  switch (kind) {
    case 'offline':
      return { tone: 'warn', title: `You’re offline, so ${name} can’t download yet.`, reason: 'It starts again by itself when you’re back online.' };
    case 'network':
      return { tone: 'bad', title: `Couldn’t get ${name} ready${onHost}.`, reason: 'The download stopped partway. Your connection may have dropped.' };
    case 'disk':
      return { tone: 'bad', title: `Couldn’t get ${name} ready${onHost}.`, reason: 'There isn’t enough free space. Free some up, then try again.' };
    case 'permission':
      return { tone: 'bad', title: `Couldn’t get ${name} ready${onHost}.`, reason: `It needs an admin step ${where}. Copy details shows what went wrong.` };
    default:
      return { tone: 'bad', title: `Something went wrong while getting ${name} ready.`, reason: 'Try again, or copy the details for help.' };
  }
}

// ------------------------------------------------------------------ sign-in for three engines (O3)

export type LoginState =
  | 'starting'
  | 'waiting_for_code'
  | 'waiting_for_approval'
  | 'verifying'
  | 'done'
  | 'failed'
  | 'expired'
  | 'cancelled';

/** SessionView from o3-api.md. Claude on an older server has no `kind`, `engine` or `user_code`. */
export interface LoginSession {
  session_id: string;
  engine?: LoginEngine;
  kind?: 'device_code' | 'paste_code';
  state: LoginState;
  detail: string | null;
  url: string | null;
  user_code?: string | null;
  expires_at: string | null;
}

export class LoginError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly sessionId: string | null = null,
  ) {
    super(message);
  }
}

export const isLive = (s: LoginState) => s === 'starting' || s === 'waiting_for_code' || s === 'waiting_for_approval' || s === 'verifying';

export interface LoginApi {
  start: (engine: LoginEngine) => Promise<LoginSession>;
  get: (engine: LoginEngine, sid: string) => Promise<LoginSession>;
  cancel: (engine: LoginEngine, sid: string) => Promise<void>;
  submitCode: (engine: LoginEngine, sid: string, code: string) => Promise<LoginSession>;
}

async function call<T>(method: string, path: string, body?: unknown, send: typeof fetch = (...a) => fetch(...a)): Promise<T> {
  let r: Response;
  try {
    r = await send(path, {
      method,
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new LoginError('Couldn’t reach the server.', 0);
  }
  if (r.status === 204) return undefined as T;
  const d = await r.json().catch(() => null);
  if (!r.ok) {
    throw new LoginError(typeof d?.error === 'string' && d.error ? d.error : `HTTP ${r.status}`, r.status, typeof d?.session_id === 'string' ? d.session_id : null);
  }
  return d as T;
}

export const makeLoginApi = (send?: typeof fetch): LoginApi => {
  const base = (e: LoginEngine) => `/api/engines/${e}/login`;
  return {
    start: (e) => call('POST', base(e), undefined, send),
    get: (e, sid) => call('GET', `${base(e)}/${encodeURIComponent(sid)}`, undefined, send),
    cancel: (e, sid) => call('DELETE', `${base(e)}/${encodeURIComponent(sid)}`, undefined, send),
    submitCode: (e, sid, code) => call('POST', `${base(e)}/${encodeURIComponent(sid)}/code`, { code }, send),
  };
};
export const loginApi = makeLoginApi();

/** Starts a sign-in, or picks up the one already running (409 with its session id; F7). */
export async function startOrResume(api: LoginApi, engine: LoginEngine): Promise<LoginSession> {
  try {
    return await api.start(engine);
  } catch (e) {
    if (e instanceof LoginError && e.status === 409 && e.sessionId) return api.get(engine, e.sessionId);
    throw e;
  }
}

/** Whether this server has the in-app device sign-in (O3): false on a 404 for the engine's route. */
export const noDeviceRoute = (e: unknown) => e instanceof LoginError && e.status === 404;

/** The page to open, shown without https:// and without the query ("chatgpt.com/device"). */
export function deviceLabel(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    if (u.protocol !== 'https:') return null;
    const path = u.pathname.replace(/\/$/, '');
    return `${u.hostname.replace(/^www\./, '')}${path}`;
  } catch {
    return null;
  }
}

/** Only https links are opened; for device codes, only the engines' own sign-in hosts. */
export function isSignInUrl(engine: LoginEngine, url: string | null | undefined): url is string {
  if (!url) return false;
  try {
    const u = new URL(url);
    if (u.protocol !== 'https:') return false;
    const hosts =
      engine === 'claude'
        ? /(^|\.)(claude\.ai|claude\.com|anthropic\.com)$/
        : engine === 'codex'
          ? /(^|\.)(openai\.com|chatgpt\.com)$/
          : /(^|\.)(x\.ai|grok\.com|x\.com)$/;
    return hosts.test(u.hostname);
  } catch {
    return false;
  }
}

/** "Code K 7 Q F, 2 M X D": read out one character at a time, groups split by a pause (design §13). */
export function codeAriaLabel(code: string): string {
  const groups = code
    .trim()
    .split(/[-\s]+/)
    .filter(Boolean)
    .map((g) => g.split('').join(' '));
  return `Code ${groups.join(', ')}`;
}

/** Whole minutes left on a code, for "The code works for 15 minutes." null without an expiry. */
export function minutesLeft(expiresAt: string | null | undefined, now: number): number | null {
  if (!expiresAt) return null;
  const t = Date.parse(expiresAt);
  if (!Number.isFinite(t)) return null;
  return Math.max(0, Math.ceil((t - now) / 60000));
}

/** A pasted Claude code with spaces or a newline, trimmed to one token (design §5.3). */
export const cleanPastedCode = (raw: string) => raw.replace(/\s+/g, '');

/** What a finished sign-in that didn't sign in shows (design §9 F3, F4, F6). */
export interface SignInEnd {
  kind: 'cancelled' | 'expired' | 'failed';
  tone: 'neutral' | 'bad';
  title: string;
  reason: string | null;
  action: string;
}
export function signInEnd(s: Pick<LoginSession, 'state' | 'detail'>, engine: LoginEngine): SignInEnd | null {
  const name = cliName(engine);
  switch (s.state) {
    case 'cancelled':
      return { kind: 'cancelled', tone: 'neutral', title: 'Sign-in stopped. Nothing was saved.', reason: 'You can start again whenever you like.', action: 'Start again' };
    case 'expired':
      return engine === 'claude'
        ? { kind: 'expired', tone: 'neutral', title: 'That sign-in timed out.', reason: 'Start it again to get a fresh link.', action: 'Start again' }
        : { kind: 'expired', tone: 'neutral', title: 'That code expired.', reason: s.detail || 'Get a new code to try again.', action: 'Get a new code' };
    case 'failed':
      return {
        kind: 'failed',
        tone: 'bad',
        title: /can’t use this account|can't use this account/i.test(s.detail || '') ? `Signed in, but ${name} can’t use this account.` : 'Sign-in didn’t finish.',
        reason: s.detail,
        action: 'Try again',
      };
    default:
      return null;
  }
}

// ------------------------------------------------------------------ the banner outside #general (§7.1)

export type Phase =
  | { kind: 'choose' }
  | { kind: 'installing'; seconds: number }
  | { kind: 'signin' }
  | { kind: 'failed' }
  | { kind: 'done' };

export interface BannerView {
  title: string;
  /** Phones show only this. */
  short: string;
  sub: string | null;
  button: 'Open' | 'Continue';
  spinner: boolean;
}

export function bannerView(phase: Phase, choice: AiChoice | null): BannerView | null {
  const name = choice && choice !== 'key' ? cliName(choice) : choice ? 'your key' : 'your AI';
  switch (phase.kind) {
    case 'choose':
      return { title: 'Getting started · Choose your AI', short: 'Getting started · Choose your AI', sub: 'One tap installs it.', button: 'Open', spinner: false };
    case 'installing':
      return {
        title: `Getting started · Getting ${name} ready… ${phase.seconds} s`,
        short: `Getting ${name} ready… ${phase.seconds} s`,
        sub: 'Then sign in, and your bots can answer.',
        button: 'Open',
        spinner: true,
      };
    case 'signin':
      return choice === 'key'
        ? { title: 'Getting started · Add your key', short: 'Getting started · Add your key', sub: 'One key and your bots can answer.', button: 'Continue', spinner: false }
        : {
            title: `Getting started · Sign in to ${name}`,
            short: `Sign in to ${name}`,
            sub: `${name} is installed. One sign-in and your bots can answer.`,
            button: 'Continue',
            spinner: false,
          };
    case 'failed':
      return { title: `Getting started · Couldn’t get ${name} ready`, short: `Couldn’t get ${name} ready`, sub: null, button: 'Open', spinner: false };
    default:
      return null;
  }
}

// ------------------------------------------------------------------ what this browser keeps (§11)

const startListeners = new Set<() => void>();
let startVersion = 0;
/** Something this browser keeps about getting started changed (choice, kept, dismissed). */
export const onStartChange = (l: () => void) => {
  startListeners.add(l);
  return () => void startListeners.delete(l);
};
export const getStartVersion = () => startVersion;
const changed = () => {
  startVersion++;
  startListeners.forEach((l) => l());
};

const dismissKey = (space: string) => `kwnt:getting-started:${space}`;
const startKey = (space: string) => `kwnt:start:${space}`;

const store = (): Storage | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

export const isDismissed = (space: string) => store()?.getItem(dismissKey(space)) === 'dismissed';
/** The card was shown here before (so it stays until dismissed, C4, even once an account works). */
export const wasStarted = (space: string) => {
  const v = store()?.getItem(dismissKey(space));
  return v === 'open' || v === 'kept';
};
/** "Keep the starter bots" pressed before sign-in finished: step 3 is done, the card waits for step 2. */
export const isKept = (space: string) => store()?.getItem(dismissKey(space)) === 'kept';
export function markStarted(space: string, value: 'open' | 'kept' = 'open') {
  try {
    const cur = store()?.getItem(dismissKey(space));
    if (cur === 'dismissed' || (cur === 'kept' && value === 'open')) return;
    store()?.setItem(dismissKey(space), value);
  } catch {
    /* nothing to keep */
  }
  changed();
}
export function dismiss(space: string) {
  try {
    store()?.setItem(dismissKey(space), 'dismissed');
    store()?.removeItem(startKey(space));
  } catch {
    /* private mode: the card goes on the next message anyway */
  }
  changed();
}
export function storedChoice(space: string): AiChoice | null {
  const v = store()?.getItem(startKey(space));
  return v === 'claude' || v === 'codex' || v === 'grok' || v === 'key' ? v : null;
}
export function storeChoice(space: string, c: AiChoice | null) {
  try {
    if (c) store()?.setItem(startKey(space), c);
    else store()?.removeItem(startKey(space));
  } catch {
    /* nothing to keep */
  }
  changed();
}

/**
 * Whether #general shows the card (C4): not dismissed here, and either nothing can run yet or the
 * card was started here, or #general is still empty in a space with only the starter team (a fresh install that
 * found an account already signed in, D2). An older install with any history never sees it.
 */
export function showCard(opts: { dismissed: boolean; canRun: boolean | null; generalMessages: number; teams: number; started: boolean }): boolean {
  if (opts.dismissed) return false;
  if (opts.canRun === false || opts.started) return true;
  if (opts.canRun === null) return false;
  // A fresh install comes with one starter team (core-engineering), so one team still counts as fresh.
  return opts.generalMessages === 0 && opts.teams <= 1;
}

/**
 * Whether What's new may treat this person as someone who has used kwant (true), as brand new (false: the
 * version is recorded silently), or can't tell yet (null: wait). Get started wins (F-1): a fresh install that
 * found an AI already signed in gets the card, never the sheet on top of it. Until the space and the open
 * chat's messages have loaded, an upgrading user could look new, so the answer waits.
 */
export function whatsNewEstablished(opts: {
  loaded: boolean;
  messagesLoaded: boolean;
  canRun: boolean | null;
  wroteHere: boolean;
  getStartedShowing: boolean;
}): boolean | null {
  if (opts.wroteHere) return true;
  if (!opts.loaded || opts.canRun == null || !opts.messagesLoaded) return null;
  return opts.canRun === true && !opts.getStartedShowing;
}
