// Automation Discovery (Mission Control Phase D4, docs/mission-control.md §20): kwant notices a
// board task the person asks for on a regular rhythm and suggests a routine for it. Nothing is
// created until the person accepts; every route here is the person's only.

import { PersonCancelledError, personFetch } from './auth';
import type { Routine, RoutineInput, Schedule } from './continuity';
import { type Check, type CheckKind, checkKindLabel, checkTarget } from './missions';

export type DiscoveryBlock = 'off' | 'board_off' | 'no_timezone';
export type SuggestionStatus = 'offered' | 'accepted' | 'dismissed' | 'expired' | 'superseded';
export type DiscoveryPattern = 'monthly' | 'weekly' | 'weekdays' | 'daily';

export interface DiscoveryCheck {
  kind: string;
  /** Secrets are already masked; never sent back. */
  spec: unknown;
  required: boolean;
}

export interface LeftOutCheck {
  kind: string;
  spec: unknown;
  reason: string;
}

export interface ChainStep {
  key: string;
  title: string;
  kwant_id: string | null;
  /** Same bot: a step of the routine. Another bot: only mentioned. */
  in_instructions: boolean;
  after: number;
}

export interface DiscoverySource {
  task_id: string;
  title: string;
  /** The person's local day and time of the ask. */
  local_date: string;
  local_time: string;
  state: string;
}

export interface DiscoveryEvidence {
  pattern: DiscoveryPattern;
  matched: number;
  expected: number;
  coverage: number;
  precision: number;
  verified: number;
  settled: number;
  total: number;
  time_spread: string;
  time_varies: boolean;
  last_ask_at: string;
}

export interface DiscoveryPermissions {
  flight_school: boolean;
  tier: 'observe' | 'sandbox' | 'act';
  tier_from: string;
  tier_label: string;
  approval_mode: 'ask' | 'smart' | 'off';
  payments_ask: boolean;
  past: { external: number; workspace: number; needed_ok: number };
}

export interface Suggestion {
  id: string;
  space_id: string;
  cluster_key: string;
  kwant_id: string;
  name: string;
  instructions: string;
  schedule: Schedule;
  schedule_text: string;
  first_run_at: string;
  first_run_text: string;
  delivery: { channel_id?: string; thread_id?: string; direct_kwant_id?: string };
  task_class: string;
  acceptance: string;
  checks: DiscoveryCheck[];
  left_out_checks: LeftOutCheck[];
  chain: ChainStep[];
  source_task_ids: string[];
  sources: DiscoverySource[];
  time_spread: string;
  evidence: DiscoveryEvidence;
  permissions: DiscoveryPermissions;
  status: SuggestionStatus;
  routine_id: string | null;
  cooldown_until: string | null;
  offered_at: string;
  decided_at: string | null;
}

export interface DiscoveryStats {
  offered_30d: number;
  accepted_30d: number;
  dismissed_30d: number;
  retained: number;
  dropped: number;
}

export interface DiscoveryList {
  setting: 'on' | 'off';
  daily_cap: number;
  blocked_by: DiscoveryBlock[];
  last_scan_at: string | null;
  suggestions: Suggestion[];
  stats: DiscoveryStats;
}

export interface SkippedHabit {
  cluster_key: string;
  title: string;
  asks: number;
  reason: string;
}

/** A `?preview=1` offer: what would be suggested, never stored. */
export interface PreviewOffer {
  cluster_key: string;
  kwant_id: string;
  name: string;
  schedule: Schedule;
  first_run_text: string;
  evidence: DiscoveryEvidence;
}

export interface ScanResult {
  blocked_by: DiscoveryBlock | null;
  preview: boolean;
  timezone?: string;
  offered: Array<Suggestion | PreviewOffer>;
  skipped: SkippedHabit[];
  expired?: string[];
  superseded?: string[];
}

/** What Accept may change; anything left out stays as suggested. Checks can only be dropped. */
export interface AcceptEdits {
  name?: string;
  instructions?: string;
  schedule?: Schedule;
  kwant_id?: string;
  /** `null`: the bot's direct chat. */
  channel_id?: string | null;
  acceptance?: string | null;
  /** Indexes into the suggestion's checks to keep. */
  keep_checks?: number[];
}

export interface Accepted {
  suggestion: Suggestion;
  routine: Routine;
  first_run_text: string | null;
}

export class DiscoveryError extends Error {
  constructor(
    message: string,
    public status: number,
    /** On a 409: what already happened to it, and its routine when it was accepted. */
    public current: { status: string; routine_id: string | null } | null = null,
  ) {
    super(message);
  }
}

const sentence = (s: string) => {
  const t = s.trim();
  if (!t) return t;
  const c = t[0].toUpperCase() + t.slice(1);
  return /[.!?]$/.test(c) ? c : `${c}.`;
};

export function discoveryErrorFrom(status: number, answer: unknown): DiscoveryError {
  const a = answer && typeof answer === 'object' ? (answer as Record<string, unknown>) : {};
  const err = typeof a.error === 'string' && a.error.trim() ? sentence(a.error) : null;
  if (status === 403) return new DiscoveryError('Only you can act on suggested routines.', 403);
  if (status === 409) {
    const current = { status: typeof a.status === 'string' ? a.status : '', routine_id: typeof a.routine_id === 'string' ? a.routine_id : null };
    return new DiscoveryError(alreadyHandledText(current.status), 409, current);
  }
  if (status === 404) return new DiscoveryError(err && !/not found/i.test(err) ? err : 'That suggestion no longer exists.', 404);
  return new DiscoveryError(err ?? `Something went wrong (HTTP ${status}).`, status);
}

/** The 409 in the person's words: someone (often another tab) already decided it. */
export function alreadyHandledText(status: string): string {
  switch (status) {
    case 'accepted':
      return 'Already added. It’s in your routines below.';
    case 'dismissed':
      return 'Already dismissed.';
    case 'expired':
      return 'This suggestion has expired: the habit no longer fits a schedule.';
    case 'superseded':
      return 'This suggestion was updated. The list now shows the new one.';
    default:
      return 'This suggestion was already handled.';
  }
}

async function call<T>(url: string, method = 'GET', body?: unknown, extra?: Record<string, string>): Promise<T> {
  let res: Response;
  const headers = { ...(body === undefined ? {} : { 'Content-Type': 'application/json' }), ...extra };
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: Object.keys(headers).length ? headers : undefined,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new DiscoveryError(e.message, 401);
    throw new DiscoveryError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const answer = await res.json().catch(() => null);
  if (res.ok) return answer as T;
  throw discoveryErrorFrom(res.status, answer);
}

const enc = encodeURIComponent;
const BASE = '/api/automation-discovery';

export const discoveryApi = {
  /** The space comes from the fetch wrapper's `?space=`. */
  list: (status: SuggestionStatus | 'all' = 'offered') => call<DiscoveryList>(`${BASE}?status=${enc(status)}`),
  scan: (preview = false) => call<ScanResult>(`${BASE}/scan${preview ? '?preview=1' : ''}`, 'POST'),
  /** `key` is one per Accept intent: a retry or a double press sends the same one. */
  accept: (id: string, key: string, edits?: AcceptEdits) =>
    call<Accepted>(`${BASE}/${enc(id)}/accept`, 'POST', edits && Object.keys(edits).length ? edits : undefined, { 'Idempotency-Key': key }),
  dismiss: (id: string) => call<{ suggestion: Suggestion }>(`${BASE}/${enc(id)}/dismiss`, 'POST'),
};

/** Keys sorted, so the same schedule written in another order compares equal. */
const canonical = (v: unknown): string =>
  JSON.stringify(v, (_k, x) => (x && typeof x === 'object' && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort(([a], [b]) => a.localeCompare(b))) : x));

/**
 * The Accept body for what the person changed in the form: only the fields that differ from the
 * suggestion (an empty object accepts it as it is), and `keep_checks` only when one was unticked.
 */
export function acceptEdits(s: Suggestion, body: RoutineInput, placeChanged: boolean, keep: boolean[]): AcceptEdits {
  const out: AcceptEdits = {};
  if (body.name.trim() !== s.name) out.name = body.name.trim();
  if (body.instructions.trim() !== s.instructions.trim()) out.instructions = body.instructions.trim();
  if (canonical(body.schedule) !== canonical(s.schedule)) out.schedule = body.schedule;
  if (body.kwant_id !== s.kwant_id) out.kwant_id = body.kwant_id;
  if (placeChanged) out.channel_id = body.channel_id;
  if (keep.length === s.checks.length && keep.some((k) => !k)) out.keep_checks = keep.flatMap((k, i) => (k ? [i] : []));
  return out;
}

/**
 * One Idempotency-Key per Accept intent: the same key until the server answers for good (a 2xx or a
 * refusal it will repeat), so a retry after a dropped connection or a double press can't add a second
 * routine. A new intent (after a refusal the person fixes, or another card) gets a new key.
 */
export class AcceptIntent {
  private key: string | null = null;
  constructor(private make: () => string) {}
  current(): string {
    this.key ??= this.make();
    return this.key;
  }
  /** After an answer that isn't worth retrying with the same key. */
  settle(err: unknown) {
    // Network errors (0) and server errors (5xx) may have gone through: keep the key so the retry replays
    const status = err instanceof DiscoveryError ? err.status : 0;
    if (status !== 0 && status < 500) this.key = null;
  }
}

// ---------------------------------------------------------------------------
// In the person's words

export const BLOCK_TEXT: Record<DiscoveryBlock, { title: string; fix: string }> = {
  off: {
    title: 'Suggestions are off.',
    fix: 'Turn on “Suggest routines from repeated tasks” in Settings, under Mission board.',
  },
  board_off: {
    title: 'Suggestions learn from tasks on the Mission board, and the board is off.',
    fix: 'Turn on “Show the Mission board” in Settings.',
  },
  no_timezone: {
    title: 'Your time zone isn’t set, so there’s no telling what time you usually ask.',
    fix: 'Set your time zone in Settings, under Where you are.',
  },
};

export const blockText = (b: string) => BLOCK_TEXT[b as DiscoveryBlock] ?? { title: 'Suggestions can’t run right now.', fix: 'Check Settings.' };

const SKIP_TEXT: Record<string, string> = {
  too_short: 'Not enough asks yet to see a rhythm',
  not_regular: 'No regular rhythm',
  stopped: 'You stopped asking for it',
  not_verified: 'Too few of these tasks passed their checks',
  mixed_owners: 'Different bots did it, so there’s no one bot to suggest',
  bot_unavailable: 'The bot that did it isn’t in this space any more',
  covered: 'A routine already does this',
  cooling_off: 'You dismissed it, or deleted its routine, in the last 30 days',
  already_offered: 'Already suggested',
  in_chain: 'It’s a follow-up step of another suggestion',
  capped: 'Waiting: today’s limit of new suggestions is reached',
};

export const skipText = (reason: string) => SKIP_TEXT[reason] ?? reason.replace(/_/g, ' ');

const PATTERN_TEXT: Record<DiscoveryPattern, string> = {
  monthly: 'once a month',
  weekly: 'every week',
  weekdays: 'on weekdays',
  daily: 'every day',
};

/** "You asked for this 5 times, every week, usually around 09:10." */
export function habitLine(e: DiscoveryEvidence | undefined | null): string {
  if (!e) return '';
  const times = e.matched === 1 ? 'once' : `${e.matched} times`;
  const rhythm = PATTERN_TEXT[e.pattern] ? `, ${PATTERN_TEXT[e.pattern]}` : '';
  const when = e.time_spread ? (e.time_spread.includes('–') ? `, usually between ${e.time_spread.replace('–', ' and ')}` : `, usually around ${e.time_spread}`) : '';
  return `You asked for this ${times}${rhythm}${when}${e.time_varies ? ' (the time varies)' : ''}.`;
}

const APPROVAL_TEXT: Record<string, string> = {
  ask: 'Asks you before actions that need approval',
  smart: 'Asks you only when an action is risky',
  off: 'Doesn’t ask before actions (approvals are off for this bot)',
};
export const approvalText = (m: string) => APPROVAL_TEXT[m] ?? APPROVAL_TEXT.ask;

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** "In past runs: 2 outside actions, 1 workspace change; needed your OK twice." */
export function pastText(p: DiscoveryPermissions['past'] | undefined): string {
  if (!p) return '';
  const did = [p.external ? plural(p.external, 'outside action') : '', p.workspace ? plural(p.workspace, 'workspace change') : ''].filter(Boolean);
  const ok = p.needed_ok ? `needed your OK ${p.needed_ok === 1 ? 'once' : p.needed_ok === 2 ? 'twice' : `${p.needed_ok} times`}` : 'never needed your OK';
  return did.length ? `In past runs: ${did.join(', ')}; ${ok}.` : `In past runs it changed nothing outside its chat, and ${ok}.`;
}

/** A kept or left-out check on one line, from its masked spec: shown only, never editable. */
export function checkLine(c: { kind: string; spec: unknown }): string {
  const spec = c.spec && typeof c.spec === 'object' && !Array.isArray(c.spec) ? (c.spec as Record<string, unknown>) : {};
  const target = checkTarget({ kind: c.kind as CheckKind, spec } as Pick<Check, 'kind' | 'spec'>);
  return target ? `${checkKindLabel(c.kind)}: ${target}` : checkKindLabel(c.kind);
}

const LEFT_OUT_TEXT: Record<string, string> = {
  'needs a person or a reviewer': 'Needs you or a teammate to look, so it can’t run on a schedule',
  'changes every time': 'It was different every time',
  'passed only once': 'It passed only once',
  'more than 10 checks': 'A routine keeps at most 10 checks',
};
export const leftOutText = (reason: string) => LEFT_OUT_TEXT[reason] ?? sentence(reason).replace(/\.$/, '');

/** "Last 30 days: 3 suggested, 1 added, 1 dismissed. Still on after 14 days: 1 of 2." */
export function statsLine(s: DiscoveryStats | undefined | null): string {
  if (!s) return '';
  const parts = `Last 30 days: ${s.offered_30d} suggested, ${s.accepted_30d} added, ${s.dismissed_30d} dismissed.`;
  const judged = s.retained + s.dropped;
  return judged ? `${parts} Still on 14 days after adding: ${s.retained} of ${judged}.` : parts;
}

/** A source ask's day: "Mon 12 Oct". Local dates come as YYYY-MM-DD in the person's zone. */
export function sourceDay(localDate: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(localDate);
  if (!m) return localDate;
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  return d.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short', timeZone: 'UTC' });
}

/** The automation_discovery settings, or null on a server without them. */
export function parseDiscoverySettings(data: unknown): { automation_discovery: 'on' | 'off'; automation_discovery_daily_cap: number } | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  if (d.automation_discovery !== 'on' && d.automation_discovery !== 'off') return null;
  const cap = typeof d.automation_discovery_daily_cap === 'number' && Number.isInteger(d.automation_discovery_daily_cap) ? d.automation_discovery_daily_cap : 2;
  return { automation_discovery: d.automation_discovery, automation_discovery_daily_cap: Math.min(10, Math.max(0, cap)) };
}

export const DISCOVERY_CAPS = Array.from({ length: 11 }, (_, i) => i);

/** Fired after Settings saves the discovery setting, so an open Routines list checks again. */
export const DISCOVERY_SETTING_EVENT = 'kwnt:automation-discovery-setting';
