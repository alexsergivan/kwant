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

export const PREPUSH_STATUSES = ['checking', 'clear', 'blocked', 'failed', 'overridden', 'pushed', 'retried', 'superseded'] as const;
/** `unknown`: a status this build can't read yet. Shown as a read-only grey line,
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

/**
 * 0.9.7 prepush-grant: the person's press is the approval. "Ask {bot} to try again" (retry) covers the
 * bot's next push of the branch once its card comes back clear; "Push anyway" (override) covers the one
 * push of that commit. Both the card the person pressed and the card the covered push makes carry it.
 */
export interface PrepushGrant {
  kind: 'retry' | 'override';
  at: number | null;
  expires_at: number | null;
  /** When the push it covered went; null while it's still waiting. */
  used_at: number | null;
  /** The card the person pressed, when the server names it. */
  card_id: string | null;
  /** The newest card a push of the branch made while the grant waited, then the card of the push it let go. */
  next_card_id: string | null;
}

export interface PrepushCard {
  id: string;
  space_id: string | null;
  kwant_id: string;
  /** The folder the push runs in, when the server sends it (Needs you is one item per bot, folder, branch). */
  folder: string | null;
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
  grant: PrepushGrant | null;
  /** On a superseded card: the newer card that replaced it. */
  superseded_by: string | null;
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

function grant(v: unknown): PrepushGrant | null {
  if (!isObj(v)) return null;
  const kind = v.kind === 'retry' || v.kind === 'override' ? v.kind : null;
  if (!kind) return null;
  return { kind, at: num(v.at), expires_at: num(v.expires_at), used_at: num(v.used_at), card_id: str(v.card_id, 120), next_card_id: str(v.next_card_id, 120) };
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
    folder: str(raw.folder, 400),
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
    grant: grant(raw.grant),
    superseded_by: status === 'superseded' ? str(raw.superseded_by, 120) : null,
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
  | 'P6' // pushed anyway (overridden, or pushed: its one push went)
  | 'P7' // a check couldn't run (failed, before needs_you)
  | 'RT' // the person asked for a re-check (retried)
  | 'SS' // superseded: a newer push of the branch replaced it
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
    case 'pushed':
      return 'P6';
    case 'retried':
      return 'RT';
    case 'superseded':
      return 'SS';
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
      return { tone: 'plain', text: c.status === 'pushed' ? 'Pushed' : 'Pushed anyway', led: null };
    case 'P7':
      return { tone: 'plain', text: "Couldn't check", led: null };
    case 'RT':
      return { tone: 'plain', text: 'Checking again', led: null };
    case 'SS':
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
      return `You asked ${bot} to fix this and push again.`;
    case 'SS':
      return `Replaced by a newer push of ${c.branch}.`;
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

// The override's first sentence on its own: what the person did, with no promise about the next push.
export function overriddenHead(c: PrepushCard): string {
  const at = hhmm(c.override?.at ?? null);
  return `You let this push go${at ? ` at ${at}` : ''} with ${plural(openCount(c), 'finding')} open.`;
}

export function overriddenLine(c: PrepushCard, bot: string): string {
  const head = overriddenHead(c);
  // Mira FIX 2: with a grant, the press was the OK, and the card says so.
  if (c.grant) return `${head} That was your OK: ${bot}'s push of this commit goes without asking you.`;
  return `${head} It covers this commit only.`;
}

export function inboxLine(c: PrepushCard, bot: string): string {
  const n = openCount(c);
  if (n === 0) return `${bot}'s push is waiting for you: the checks couldn't run`;
  return `${bot}'s push is waiting for you: ${plural(n, 'finding')} open`;
}

/** What "Ask {bot} to try again" sends through the normal chat send after /retry. */
export const retryMessage = (c: PrepushCard): string =>
  `Fix what the pre-push check found on ${c.branch}, commit, and push again. I've given my OK for that push, so once the checks pass it goes without asking me.`;

export const overrideMessage = (c: PrepushCard): string =>
  `I let your push of ${c.branch} go anyway, for this commit only. That was my OK, so run the same push again without asking me.`;

/** Characters as the server counts them (code points, not UTF-16 units: Ada Later 6). */
export const reasonLength = (s: string): number => [...s.trim()].length;
export const reasonOk = (s: string): boolean => reasonLength(s) >= MIN_REASON && reasonLength(s) <= MAX_REASON;

/** The card changed but the chat message to the bot didn't go (Ada Later 7). */
export const messageNotSent = (bot: string): string => `Done, but the message to ${bot} didn't send. Tell ${bot} in the chat.`;

// ---------------------------------------------------------------------------------------------
// After the press (0.9.7 prepush-grant): one story from the person's press to the push

/** The cards of one push target: the same bot, folder (when the server sends it) and branch. */
export const branchKey = (c: PrepushCard): string => JSON.stringify([c.space_id ?? '', c.kwant_id, c.folder ?? '', c.branch]);

/** Whether the person pressed this card (retry, override) and the press still tells a story. */
export const pressed = (c: PrepushCard): boolean => c.status === 'retried' || c.status === 'overridden' || c.status === 'pushed';

/**
 * The card the bot's next push made after the person pressed `c`: named by the grant's `card_id` when
 * the server sends it, otherwise the newest later card of the same push target. Null while the bot is
 * still fixing. An override's push goes on the same card, so it has none.
 */
export function followUp(c: PrepushCard, cards: PrepushCard[]): PrepushCard | null {
  if (c.status !== 'retried') return null;
  const named = c.grant?.next_card_id ? cards.find((x) => x.id === c.grant!.next_card_id && x.id !== c.id) : undefined;
  if (named) return named;
  const key = branchKey(c);
  const after = c.grant?.at ?? c.updated_at ?? c.created_at ?? 0;
  const later = cards
    .filter((x) => x.id !== c.id && branchKey(x) === key)
    .filter((x) => (x.grant?.card_id ? x.grant.card_id === c.id : (x.created_at ?? 0) >= after))
    .sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0));
  return later[0] ?? null;
}

/** The card a follow-up card came from, for its "after your retry" line. */
export function pressedBefore(c: PrepushCard, cards: PrepushCard[]): PrepushCard | null {
  if (pressed(c)) return null;
  return cards.find((x) => x.status === 'retried' && followUp(x, cards)?.id === c.id) ?? null;
}

/** An approval still waiting for this push (production floor, payment): the card links to it. */
export interface PushApproval {
  id: string;
  /** Why this kind of push always asks (the floor's why, or the payment reason); null when neither says. */
  reason: string | null;
}

export type StoryStep =
  | 'fixing' // retry: the bot hasn't pushed again yet
  | 'checking' // its new push is being checked
  | 'pushing' // clear (or overridden): the push goes now, no card asks
  | 'asks' // a floor or payment approval still has to be answered
  | 'pushed' // the push went
  | 'blocked' // the new push was blocked again
  | 'expired' // the grant's window ran out before the push it covered
  | 'replaced'; // a newer push replaced this one before it went

export interface Story {
  step: StoryStep;
  /** The first line: what the person did, and what it lets happen. */
  lead: string;
  /** Where it is now. */
  now: string;
  /** The new card, to link to (retry only). */
  next: PrepushCard | null;
  /** When `step` is 'pushed'. */
  at: number | null;
  approval: PushApproval | null;
}

const usedAt = (...cs: (PrepushCard | null)[]): number | null => {
  for (const c of cs) if (c?.grant?.used_at) return c.grant.used_at;
  return null;
};

/**
 * What the card says after the person pressed it, and how it follows the bot's new push live.
 * Retry: fixing → checking the new push → Pushed · HH:MM, or blocked again. Override: pushing → Pushed.
 * `now` is the clock (ms), so a retry window that ran out says so instead of "fixing" for ever.
 */
export function storyOf(c: PrepushCard, cards: PrepushCard[], bot: string, approval: PushApproval | null = null, now = Date.now()): Story | null {
  if (!pressed(c)) return null;
  const covered = !!c.grant;
  if (c.status === 'overridden' || c.status === 'pushed') {
    const at = c.status === 'pushed' ? (usedAt(c) ?? c.updated_at) : null;
    // One source for the override's first line: the card renders it as the lead (Mira FIX 2).
    const lead = overriddenLine(c, bot);
    if (at !== null || c.status === 'pushed') return { step: 'pushed', lead, now: pushedLine(at), next: null, at, approval: null };
    const exp = c.grant?.expires_at ?? null;
    if (c.grant && c.grant.used_at === null && exp !== null && now >= exp)
      // Ada (Nov 13), Mira's FIX 7 rule: in an expired step the lead drops its promise.
      return { step: 'expired', lead: overriddenHead(c), now: `${bot} didn't push within 2 hours, so your OK ran out. Its push asks you as usual.`, next: null, at: null, approval: null };
    if (approval) return { step: 'asks', lead, now: asksLine(bot, approval), next: null, at: null, approval };
    return { step: 'pushing', lead, now: `${bot} can push it now.`, next: null, at: null, approval: null };
  }
  // retried
  const lead = covered
    ? `You asked ${bot} to fix this and push again. That was your OK: once the checks pass, its next push of ${c.branch} goes without asking you.`
    : `You asked ${bot} to fix this and push again. Its next push of ${c.branch} is checked from try 1.`;
  const next = followUp(c, cards);
  const at = usedAt(next, c);
  const base = { lead, next, approval: null, at: null };
  if (at !== null) return { ...base, step: 'pushed', now: pushedLine(at), at };
  if (!next) {
    const exp = c.grant?.expires_at ?? null;
    if (covered && exp !== null && now >= exp)
      return {
        ...base,
        // Mira FIX 7: the lead stops promising once the OK has run out.
        lead: `You asked ${bot} to fix this and push again.`,
        step: 'expired',
        now: `No fixed push came within 2 hours, so your OK ran out. ${bot}'s next push asks you as usual.`,
      };
    return { ...base, step: 'fixing', now: `${bot} is fixing it…` };
  }
  switch (next.status) {
    case 'checking':
      return { ...base, step: 'checking', now: `Checking ${bot}'s new push…` };
    case 'clear':
      if (approval) return { ...base, step: 'asks', now: asksLine(bot, approval), approval };
      return { ...base, step: 'pushing', now: `The checks passed. ${bot} is pushing…` };
    case 'blocked':
    case 'failed': {
      const n = openCount(next);
      const what = next.status === 'failed' && n === 0 ? "a check couldn't run" : plural(Math.max(n, 1), 'finding');
      return { ...base, step: 'blocked', now: `Blocked again: ${what}. Nothing was pushed.` };
    }
    case 'superseded':
      return { ...base, step: 'replaced', now: `${bot} pushed again, so a newer check replaced this one.` };
    default:
      return { ...base, step: 'checking', now: `Checking ${bot}'s new push…` };
  }
}

export const pushedLine = (at: number | null): string => (hhmm(at) ? `Pushed · ${hhmm(at)}` : 'Pushed');
/** Mira FIX 3: "still" and "always ask" say the press covered the usual OK, not this kind. */
export const asksLine = (bot: string, a: PushApproval): string =>
  a.reason ? `${bot}'s push still needs your OK. ${a.reason}. Pushes like this always ask, every time.` : `${bot}'s push still needs your OK. Pushes like this always ask, every time.`;

/** A follow-up card's own lead line, so the two cards read as one story. */
export const afterRetryLine = (c: PrepushCard): string => {
  const at = hhmm(c.grant?.at ?? null);
  return `After you asked for a fix${at ? ` at ${at}` : ''}.`;
};

/**
 * The approval a push the person already let go still has to get (floor or payment), from the chat's
 * pending approvals: the same bot, asked after the press, and one the floor raised. Never the ordinary
 * approval: the grant replaces that one, so if one shows up it isn't this push's.
 */
export function pushApproval(
  c: PrepushCard,
  approvals: readonly { id: string; tool_name: string; details: Record<string, unknown>; asked_at?: string; kwant_id?: string | null }[],
): PushApproval | null {
  if (!pressed(c) || (c.status === 'pushed')) return null;
  const after = c.grant?.at ?? c.updated_at ?? 0;
  for (const a of approvals) {
    const kw = a.kwant_id ?? (typeof a.details.kwant_id === 'string' ? a.details.kwant_id : null);
    if (kw !== c.kwant_id) continue;
    const asked = a.asked_at ? Date.parse(a.asked_at) : NaN;
    if (Number.isFinite(asked) && asked + 5_000 < after) continue;
    const floor = isObj(a.details.prod_floor) ? a.details.prod_floor : null;
    const pay = str(a.details.payment_reason, 300);
    if (!floor && !pay) continue;
    const reason = ((floor && str(floor.why, 300)) || pay || '').replace(/[.\s]+$/, '');
    return { id: a.id, reason: reason || null };
  }
  return null;
}

// ---------------------------------------------------------------------------------------------
// Needs you (Inbox)

/**
 * The Inbox's pushes waiting for the person, newest first: at most one per push target (bot, folder,
 * branch), and only when the newest card there is the one waiting. An older blocked card on a branch the
 * bot has pushed again since (or that the server marked superseded) never shows. `hidden`: cards the
 * person just retried or overrode, which leave at once, before the server answers. A superseded card never
 * counts as the newest: the server can revive an older commit the bot pushes again and supersede the newer
 * card instead (impl.md, server deviation 5), and the revived one is then the one waiting.
 */
export function needsYou(cards: PrepushCard[], hidden: ReadonlySet<string> = new Set()): PrepushCard[] {
  const newest = new Map<string, PrepushCard>();
  for (const c of cards) {
    if (c.status === 'superseded') continue;
    const k = branchKey(c);
    const h = newest.get(k);
    if (!h || (c.created_at ?? 0) > (h.created_at ?? 0) || ((c.created_at ?? 0) === (h.created_at ?? 0) && (c.updated_at ?? 0) > (h.updated_at ?? 0))) newest.set(k, c);
  }
  return [...newest.values()].filter((c) => c.needs_you && !hidden.has(c.id)).sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
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
