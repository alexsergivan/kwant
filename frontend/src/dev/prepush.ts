// 0.8.0 B3: the pre-push card (Mira's design b3-fresh-eyes.md §2, Juno's contract
// fresh-eyes-audit.md §4). The server decides every state and every number; this module only
// reads the card, maps it to Mira's P0–P7, and calls the four routes.

import { personFetch } from '../auth';
import { DevModeError } from '../devMode';
import { parseFreshEyes, type FreshEyes } from './freshEyes';

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, max = 2000): string | null => (typeof v === 'string' && v.trim() !== '' ? v.slice(0, max) : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const oneOf = <T extends string>(v: unknown, all: readonly T[], fallback: T): T => (all.includes(v as T) ? (v as T) : fallback);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

/** The event the app re-dispatches when the stream sends `dev_prepush`. */
export const PREPUSH_EVENT = 'kwant:dev-prepush';
export const MIN_REASON = 10;
export const MAX_REASON = 2000;
/** Server rule: needs_you once a blocked or failed card reaches this try. */
export const NEEDS_YOU_TRIES = 2;

export const PREPUSH_STATUSES = ['checking', 'clear', 'blocked', 'failed', 'overridden', 'retried'] as const;
/** `unknown`: a status this build can't read yet (e.g. §4.4b `pushed`). Shown as a read-only grey line,
 * never dropped, so a card doesn't vanish from the thread when the server learns a new state. */
export type PrepushStatus = (typeof PREPUSH_STATUSES)[number] | 'unknown';
export const CHECK_KEYS = ['secrets', 'pkgs', 'tests', 'risk', 'review', 'semgrep'] as const;
export type CheckKey = (typeof CHECK_KEYS)[number];
export const CHECK_STATES = ['running', 'pass', 'hit', 'error', 'not_installed'] as const;
export type CheckState = (typeof CHECK_STATES)[number];
export type FindingSource = 'review' | 'gitleaks' | 'packages' | 'tamper' | 'semgrep' | 'kwant';

export interface PrepushCheck {
  key: CheckKey;
  state: CheckState;
  text: string;
  required: boolean;
}

export interface PrepushFinding {
  id: string | null;
  source: FindingSource;
  severity: 'critical' | 'high' | 'medium' | 'low';
  title: string;
  file: string | null;
  line: number | null;
  detail: string | null;
  open: boolean;
}

export interface PrepushCard {
  id: string;
  space_id: string | null;
  kwant_id: string;
  branch: string;
  head_sha: string | null;
  status: PrepushStatus;
  tries: number;
  needs_you: boolean;
  no_change: boolean;
  diffstat: { files: number; add: number; del: number } | null;
  checks: PrepushCheck[];
  blockers: PrepushFinding[];
  override: { by: string | null; reason: string; at: number | null } | null;
  findings: PrepushFinding[];
  reviewer: { name: string; engine: string | null; model: string | null } | null;
  fresh_eyes: FreshEyes | null;
  error: string | null;
  created_at: number | null;
  updated_at: number | null;
}

const SOURCES = ['review', 'gitleaks', 'packages', 'tamper', 'semgrep', 'kwant'] as const;

function pfinding(v: unknown): PrepushFinding | null {
  if (!isObj(v)) return null;
  const title = str(v.title, 300);
  if (!title) return null;
  const line = num(v.line);
  return {
    id: str(v.id, 80),
    source: oneOf(v.source, SOURCES, 'kwant'),
    severity: oneOf(v.severity, ['critical', 'high', 'medium', 'low'] as const, 'high'),
    title,
    file: str(v.file, 400),
    line: line !== null && line > 0 ? Math.floor(line) : null,
    detail: str(v.detail, 1200),
    open: (str(v.state, 20) ?? 'open') === 'open',
  };
}

function check(v: unknown): PrepushCheck | null {
  if (!isObj(v)) return null;
  const key = CHECK_KEYS.find((k) => k === v.key);
  const state = CHECK_STATES.find((s) => s === v.state);
  const text = str(v.text, 400);
  if (!key || !state || !text) return null;
  return { key, state, text, required: v.required !== false && key !== 'semgrep' };
}

function diffstat(v: unknown): PrepushCard['diffstat'] {
  if (!isObj(v)) return null;
  const files = num(v.files);
  const add = num(v.add);
  const del = num(v.del);
  if (files === null || add === null || del === null || files < 0 || add < 0 || del < 0) return null;
  return { files, add, del };
}

/** One card from the server, or null when it isn't one we can show honestly. */
export function parsePrepush(raw: unknown): PrepushCard | null {
  if (!isObj(raw)) return null;
  const id = str(raw.id, 120);
  const kwant = str(raw.kwant_id, 120);
  const branch = str(raw.branch, 200);
  const known = PREPUSH_STATUSES.find((s) => s === raw.status);
  // A status string we don't know yet still names a real card: keep it, read-only (Ada Later 2).
  const status: PrepushStatus | null = known ?? (str(raw.status, 40) ? 'unknown' : null);
  if (!id || !kwant || !branch || !status) return null;
  const rawChecks = arr(raw.checks);
  const checks = rawChecks.map(check).filter((c): c is PrepushCheck => !!c);
  // A row we can't read would be a check we'd silently drop: refuse the card instead.
  if (checks.length !== rawChecks.length) return null;
  const inner = isObj(raw.card) ? raw.card : null;
  const ov = isObj(raw.override) ? raw.override : null;
  const rv = inner && isObj(inner.reviewer) ? inner.reviewer : null;
  const tries = num(raw.tries);
  return {
    id,
    space_id: str(raw.space_id, 120),
    kwant_id: kwant,
    branch,
    head_sha: str(raw.head_sha, 64),
    status,
    tries: tries !== null && tries >= 1 ? Math.floor(tries) : 1,
    needs_you: raw.needs_you === true && (status === 'blocked' || status === 'failed'),
    no_change: raw.no_change === true,
    diffstat: diffstat(raw.diffstat),
    checks,
    blockers: arr(raw.blockers).map(pfinding).filter((f): f is PrepushFinding => !!f),
    override: ov ? { by: str(ov.by, 80), reason: str(ov.reason, MAX_REASON) ?? '', at: num(ov.at) } : null,
    findings: inner ? arr(inner.findings).map(pfinding).filter((f): f is PrepushFinding => !!f) : [],
    reviewer: rv && str(rv.name, 80) ? { name: str(rv.name, 80)!, engine: str(rv.engine, 40), model: str(rv.model, 80) } : null,
    fresh_eyes: inner ? parseFreshEyes(inner.fresh_eyes) : null,
    error: inner ? str(inner.error, 400) : null,
    created_at: num(raw.created_at),
    updated_at: num(raw.updated_at),
  };
}

export function parsePrepushList(raw: unknown): PrepushCard[] {
  const list = isObj(raw) ? arr(raw.cards) : arr(raw);
  return list.map(parsePrepush).filter((c): c is PrepushCard => !!c);
}

// ---------------------------------------------------------------------------------------------
// Mira's phases

export type Phase =
  | 'P0' // nothing to push
  | 'P1' // running
  | 'P2' // all clear
  | 'P3' // blocked, the bot is fixing
  | 'P4' // needs you
  | 'P6' // pushed anyway (overridden)
  | 'P7' // a check couldn't run (failed, before needs_you)
  | 'RT' // the person asked for a re-check (retried)
  | 'UN'; // a status this build can't read yet: a read-only line

export function phaseOf(c: PrepushCard): Phase {
  switch (c.status) {
    case 'checking':
      return 'P1';
    case 'clear':
      return c.no_change ? 'P0' : 'P2';
    case 'blocked':
      return c.needs_you ? 'P4' : 'P3';
    case 'failed':
      return c.needs_you ? 'P4' : 'P7';
    case 'overridden':
      return 'P6';
    case 'retried':
      return 'RT';
    case 'unknown':
      return 'UN';
  }
}

/** Phases where "Push anyway…" is offered: the server only takes an override on blocked or failed. */
export const canOverride = (c: PrepushCard): boolean => c.status === 'blocked' || c.status === 'failed';
export const canRetry = canOverride;

/** Open findings that hold the push. The server's `blockers` list; counted, never computed further. */
export const openCount = (c: PrepushCard): number => {
  // "kwant couldn't check" and "the review didn't finish" hold the push too, but they aren't findings.
  const reviewFailed = c.checks.some((k) => k.key === 'review' && k.state === 'error');
  return c.blockers.filter((b) => b.source !== 'kwant' && !(reviewFailed && b.source === 'review')).length;
};

export type ChipTone = 'plain' | 'ok' | 'bad' | 'warn';
export interface StatusChip {
  tone: ChipTone;
  text: string;
  led: 'run' | null;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export function statusChip(c: PrepushCard): StatusChip | null {
  switch (phaseOf(c)) {
    case 'P0':
      return null;
    case 'P1':
      // The server marks every row running until the check settles, so there's no partial count.
      return { tone: 'plain', text: 'Checking', led: 'run' };
    case 'P2':
      return { tone: 'ok', text: 'Clear', led: null };
    case 'P3':
      return { tone: 'bad', text: openCount(c) > 0 ? `${openCount(c)} to fix` : 'Blocked', led: null };
    case 'P4':
      return { tone: 'warn', text: 'Needs you', led: null };
    case 'P6':
      return { tone: 'plain', text: 'Pushed anyway', led: null };
    case 'P7':
      return { tone: 'plain', text: "Couldn't check", led: null };
    case 'RT':
      return { tone: 'plain', text: 'Checking again', led: null };
    case 'UN':
      return null;
  }
}

// ---------------------------------------------------------------------------------------------
// Rows

export const CHECK_NAME: Record<CheckKey, string> = {
  secrets: 'Secrets',
  pkgs: 'New packages',
  tests: 'Tests',
  risk: 'Risk rules',
  review: 'Fresh-eyes review',
  semgrep: 'Code patterns',
};

/** Mira's tool line. The review row names the reviewer when the card has one. */
export function toolLine(key: CheckKey, c: PrepushCard): string {
  switch (key) {
    case 'secrets':
      return 'gitleaks';
    case 'pkgs':
      return 'package registry';
    case 'tests':
      return 'tamper check';
    case 'risk':
      return 'built-in rules';
    case 'review':
      return c.reviewer ? (c.reviewer.engine ? `${c.reviewer.name} on ${c.reviewer.engine}` : c.reviewer.name) : 'another bot';
    case 'semgrep':
      return 'semgrep · optional';
  }
}

/** Short tool names for the folded "N checks passed · …" row on phone. */
export const FOLD_TOOL: Record<CheckKey, string> = {
  secrets: 'gitleaks',
  pkgs: 'registry',
  tests: 'tamper',
  risk: 'rules',
  review: 'review',
  semgrep: 'semgrep',
};

/** The visually hidden word next to each LED (colour is never the only signal). */
export const STATE_WORD: Record<CheckState, string> = {
  pass: 'passed',
  hit: 'found a problem',
  running: 'running',
  error: 'could not run',
  not_installed: 'not installed',
};

export const STATE_LED: Record<CheckState, 'ok' | 'bad' | 'run' | 'off'> = {
  pass: 'ok',
  hit: 'bad',
  running: 'run',
  error: 'off',
  not_installed: 'off',
};

const ROW_SOURCES: Record<CheckKey, FindingSource[]> = {
  secrets: ['gitleaks'],
  pkgs: ['packages'],
  tests: ['tamper'],
  risk: [],
  review: ['review'],
  semgrep: ['semgrep'],
};

/**
 * Whether a row that found something holds the push. The server's `blockers` list decides (contract
 * §4.3 "blocks?"): risk never blocks on its own, semgrep never blocks, and a tamper or review hit
 * blocks only when a high or critical finding behind it is in `blockers`.
 */
export function rowBlocks(c: PrepushCard, key: CheckKey): boolean {
  const src = ROW_SOURCES[key];
  if (src.length === 0 || key === 'semgrep') return false;
  return c.blockers.some((b) => src.includes(b.source));
}

/** What "Show" opens on a row that found something: the open findings behind it, from the card. */
export function rowFindings(c: PrepushCard, key: CheckKey): PrepushFinding[] {
  const src = ROW_SOURCES[key];
  if (src.length === 0) return [];
  const fromFindings = c.findings.filter((f) => f.open && src.includes(f.source));
  if (fromFindings.length > 0) return fromFindings;
  return c.blockers.filter((f) => src.includes(f.source));
}

/** Rows folded into one on phone (P3–P6): passing rows only; findings always show. */
export function foldable(c: PrepushCard): boolean {
  const p = phaseOf(c);
  return p === 'P3' || p === 'P4' || p === 'P6' || p === 'P7';
}

export function passedRows(c: PrepushCard): PrepushCheck[] {
  return c.checks.filter((k) => k.state === 'pass');
}

export function foldLine(c: PrepushCard): string {
  const passed = passedRows(c);
  return [`${plural(passed.length, 'check')} passed`, ...passed.map((k) => FOLD_TOOL[k.key])].join(' · ');
}

/** Required rows that passed: what P2b's "after N checks passed" counts. Server states only. */
export const requiredPassed = (c: PrepushCard): number => c.checks.filter((k) => k.required && k.state === 'pass').length;

// ---------------------------------------------------------------------------------------------
// Copy

export function hhmm(ms: number | null): string | null {
  if (ms === null) return null;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return null;
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

export function footer(c: PrepushCard, bot: string): string {
  switch (phaseOf(c)) {
    case 'P0':
      return `Nothing to check: ${c.branch} has no new commits. Nothing was pushed.`;
    case 'P1':
      return 'The push waits until these finish. Usually under a minute; the review takes longest.';
    case 'P2':
      return `All clear. ${bot} can push now.`;
    case 'P3':
      return `The push waits, then every check runs again. Try ${c.tries} of ${NEEDS_YOU_TRIES}.`;
    case 'P4':
      return `The push is waiting for you. It's also in your Inbox.`;
    case 'P6':
      return '';
    case 'P7':
      return "A check couldn't run, so this push can't count as checked. Run the checks again, or push anyway with a reason.";
    case 'RT':
      return `You asked for a new check. ${bot}'s next push of this commit is checked again from try 1.`;
    case 'UN':
      return `This version of the app can't show where the check of ${c.branch} is yet. Update the app to see it.`;
  }
}

export function needsYouLead(c: PrepushCard, bot: string): string {
  const n = openCount(c);
  if (n === 0)
    return `${bot}'s push couldn't be checked after ${plural(c.tries, 'try', 'tries')}.`;
  return `${bot} couldn't clear ${plural(Math.max(n, 1), 'finding')} after ${plural(c.tries, 'try', 'tries')}.`;
}

/** P2b's line around the branch, which the card renders as code (Mira Later 6). */
export function clearedParts(c: PrepushCard): { before: string; branch: string; after: string } {
  const at = hhmm(c.updated_at);
  return { before: 'Cleared to push ', branch: c.branch, after: ` after ${plural(requiredPassed(c), 'check')} passed${at ? ` · ${at}` : ''}` };
}

export function clearedLine(c: PrepushCard): string {
  const p = clearedParts(c);
  return `${p.before}${p.branch}${p.after}`;
}

export function overriddenLine(c: PrepushCard): string {
  const at = hhmm(c.override?.at ?? null);
  const n = openCount(c);
  return `You let this push go${at ? ` at ${at}` : ''} with ${plural(n, 'finding')} open. It covers this commit only.`;
}

export function inboxLine(c: PrepushCard, bot: string): string {
  const n = openCount(c);
  if (n === 0) return `${bot}'s push is waiting for you: the checks couldn't run`;
  return `${bot}'s push is waiting for you: ${plural(n, 'finding')} open`;
}

/** What "Ask {bot} to try again" sends through the normal chat send after /retry. */
export const retryMessage = (c: PrepushCard): string =>
  `I asked for a new check of your push of ${c.branch}. Fix what the pre-push card found, commit, and run the same push again.`;

export const overrideMessage = (c: PrepushCard): string =>
  `I let your push of ${c.branch} go anyway, for this commit only. Run the same push again.`;

/** Characters as the server counts them (code points, not UTF-16 units: Ada Later 6). */
export const reasonLength = (s: string): number => [...s.trim()].length;
export const reasonOk = (s: string): boolean => reasonLength(s) >= MIN_REASON && reasonLength(s) <= MAX_REASON;

/** The card changed but the chat message to the bot didn't go (Ada Later 7). */
export const messageNotSent = (bot: string): string => `Done, but the message to ${bot} didn't send. Tell ${bot} in the chat.`;

// ---------------------------------------------------------------------------------------------
// Needs you (Inbox)

/** One Inbox item per card that needs the person, newest first. Derived from the cards only. */
export function needsYou(cards: PrepushCard[]): PrepushCard[] {
  return cards.filter((c) => c.needs_you).sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
}

/**
 * Merge a list response into what the page already holds (Ada Later 5): a card an event refreshed
 * after the list was read keeps its newer copy, by the server's `updated_at`.
 */
export function mergeList(current: PrepushCard[], list: PrepushCard[]): PrepushCard[] {
  const have = new Map(current.map((c) => [c.id, c]));
  const merged = list.map((c) => {
    const h = have.get(c.id);
    return h && (h.updated_at ?? 0) > (c.updated_at ?? 0) ? h : c;
  });
  const ids = new Set(merged.map((c) => c.id));
  // A card an event brought in that the (older) list didn't have yet stays too.
  for (const c of current) if (!ids.has(c.id)) merged.push(c);
  return merged.sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0));
}

/** Merge a fetched card into a list (newest first), replacing the same id. */
export function upsert(list: PrepushCard[], c: PrepushCard): PrepushCard[] {
  const rest = list.filter((x) => x.id !== c.id);
  return [c, ...rest].sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0));
}

// ---------------------------------------------------------------------------------------------
// API

async function call(url: string, method = 'GET', body?: unknown): Promise<unknown> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new DevModeError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const data: unknown = await res.json().catch(() => null);
  if (res.ok) return data;
  const o = isObj(data) ? data : {};
  const code = typeof o.code === 'string' ? o.code : null;
  const msg =
    code === 'reason_needed'
      ? `Give a reason of at least ${MIN_REASON} characters.`
      : res.status === 409
        ? 'This card moved on in the meantime. It shows where it is now.'
        : `Something went wrong (HTTP ${res.status}).`;
  throw new DevModeError(msg, res.status, code);
}

function one(data: unknown): PrepushCard {
  const c = parsePrepush(data);
  if (!c) throw new DevModeError('The server sent a card this page can’t read.', 200, 'bad_card');
  return c;
}

export const prepushApi = {
  list: async (spaceId: string): Promise<PrepushCard[]> => parsePrepushList(await call(`/api/spaces/${encodeURIComponent(spaceId)}/dev/prepush`)),
  get: async (id: string): Promise<PrepushCard> => one(await call(`/api/dev/prepush/${encodeURIComponent(id)}`)),
  override: async (id: string, reason: string): Promise<PrepushCard> =>
    one(await call(`/api/dev/prepush/${encodeURIComponent(id)}/override`, 'POST', { reason: reason.trim() })),
  retry: async (id: string): Promise<PrepushCard> => one(await call(`/api/dev/prepush/${encodeURIComponent(id)}/retry`, 'POST')),
};
