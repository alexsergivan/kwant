// 0.9.0 V1 Review inbox: the GET /api/inbox shape (Mira's ENG in release-090/designs/v1-review-inbox.md,
// with Ada's decisions D1-D5), its fetch, a few pure helpers, and a fixture that covers every state on
// v1-states-board. Iris's ReviewItemView imports the types and the fixture; the Inbox store and queue use
// the rest. Nothing here decides what is safe to approve: `one_tap` and `batchable` come from the server.

import { PersonCancelledError, personFetch } from '../auth';
import type { AwayItem, AwayKind, AwayResponse } from '../away';

export type InboxKind = AwayKind | 'review';
export type InboxGroup = 'waiting_on_you' | 'failed_or_stuck' | 'to_review' | 'finished';

export type VerdictResult = 'pass' | 'fail' | 'needs_you' | 'none';
export type Confidence = 'high' | 'medium' | 'low';
export type RiskLevel = 'low' | 'medium' | 'high';
export type CriterionStatus = 'met' | 'not_met' | 'not_checked' | 'claim_only';
export type CriterionSource = 'task' | 'person_note' | 'must_not_change';

export interface InboxVerdict {
  /**
   * The verdict record's id (task_verdicts.id), sent back as `verdict_id` on verify so a stale approve
   * gets 409 `stale` (D2). Null when there is no verdict yet ('none').
   */
  id: string | null;
  result: VerdictResult;
  confidence: Confidence | null;
  /** One sentence on why. A Not met criterion's text is in it (Mira §3.6). */
  line: string;
  checker: { engine: string; model: string; tier_note: 'different_tier' | null } | null;
  checked_at: string | null;
  steps_run: number | null;
  steps_total: number | null;
}

export interface InboxCriterion {
  index: number;
  text: string;
  status: CriterionStatus;
  source: CriterionSource;
  evidence: { kind: string; ref: string; label: string } | null;
  note: string | null;
  round_added: number;
}

export interface InboxDecision {
  index: number;
  question: string;
  picked: string;
  why: string;
  /** Always null in 0.9 (D3). */
  cost: 'costly' | 'cheap' | null;
}

export interface InboxCheck {
  name: string;
  command: string;
  result: 'pass' | 'fail';
  summary: string;
  ran_by: 'checker' | 'author';
  output_tail: string;
}

export interface InboxPr {
  repo: string;
  number: number;
  url: string;
  add: number;
  del: number;
  files: number;
  ci: string;
}

export interface HeldAction {
  id: string;
  tool_name: string;
  verb: string;
  summary: string;
  /** The tool and its arguments as they will run, redacted like the approval card. */
  args_display: string;
  gate: 'merge' | 'payment' | 'approval';
}

export interface PreviousRound {
  sent_back_at: string;
  note: string;
  flips: { index: number; from: CriterionStatus; to: CriterionStatus }[];
  diff: { files: number; add: number; del: number };
  bot_summary: string;
}

/** Review-only fields; present when `kind === 'review'`. */
export interface ReviewFields {
  task_id: string;
  mission_id?: string | null;
  project?: string | null;
  in_review_since: string;
  round: { n: number; max: number };
  verdict: InboxVerdict;
  risk: { level: RiskLevel | null; reasons: string[] };
  criteria: InboxCriterion[];
  criteria_summary: { met: number; total: number; not_met: number; not_checked: number };
  couldnt_verify: { what: string; why: string }[];
  decisions: { total: number; top: InboxDecision[] };
  evidence: { checks: InboxCheck[]; screenshots: { url: string; caption: string }[]; pr: InboxPr | null };
  /**
   * In 0.9 these are the task's pending approvals, shown exactly as they will run with their own
   * Allow / Deny; Approve stays disabled until they are resolved (D3).
   */
  held_actions: HeldAction[];
  previous_round: PreviousRound | null;
  /** Set when an approve failed its freshness check; the item is then in failed_or_stuck. */
  stale: { paths: string[]; at: string } | null;
  approve_error: { code: string; message: string } | null;
  one_tap: boolean;
  batchable: boolean;
  rollback_ok: boolean;
}

export type InboxItem = Omit<AwayItem, 'kind'> & {
  kind: InboxKind;
  group: InboxGroup;
  sort_key: (string | number)[];
} & Partial<ReviewFields>;

/** A review item, with every review field present. */
export type ReviewItem = Omit<InboxItem, keyof ReviewFields> & ReviewFields & { kind: 'review' };

export interface InboxCounts {
  waiting_on_you: number;
  failed_or_stuck: number;
  to_review: number;
  finished: number;
  /** Today's unseen.total + open review items; the Low-risk card counts 1. Absent on older servers. */
  badge?: number;
}

export interface InboxResponse {
  now: string;
  since: string;
  last_seen: string | null;
  counts: InboxCounts;
  /** Already sorted by sort_key. Items in `batch.ids` are left out; the client draws the card. */
  items: InboxItem[];
  /** The Low-risk card; null when fewer than 2 qualify. */
  batch: { ids: string[]; count: number } | null;
  /** The batch's items, so the card can list them. Optional: older shapes send only ids. */
  batch_items?: InboxItem[];
}

export const INBOX_GROUPS: { key: InboxGroup; label: string; hint: string; empty: string }[] = [
  { key: 'waiting_on_you', label: 'Waiting on you', hint: 'oldest first', empty: 'No questions or approvals waiting.' },
  { key: 'failed_or_stuck', label: 'Failed or stuck', hint: 'oldest first', empty: 'Nothing failed or stuck.' },
  { key: 'to_review', label: 'To review', hint: 'highest risk first', empty: 'Nothing to review.' },
  { key: 'finished', label: 'Finished', hint: 'not read first, then newest', empty: 'Nothing finished since you last looked.' },
];

export const isReview = (i: InboxItem | null | undefined): i is ReviewItem => !!i && i.kind === 'review' && !!i.task_id;

/** Items per group, in the server's order. */
export function groupItems(r: Pick<InboxResponse, 'items'>): Record<InboxGroup, InboxItem[]> {
  const out: Record<InboxGroup, InboxItem[]> = { waiting_on_you: [], failed_or_stuck: [], to_review: [], finished: [] };
  for (const i of r.items) (out[i.group] ?? out.finished).push(i);
  return out;
}

/**
 * The one Inbox badge: `counts.badge` when the server sends it, else today's sum
 * (`unseen.total + prepushWaiting`).
 */
export function inboxBadge(inbox: Pick<InboxResponse, 'counts'> | null | undefined, away: Pick<AwayResponse, 'unseen'> | null | undefined, prepushWaiting = 0): number {
  const b = inbox?.counts?.badge;
  if (typeof b === 'number' && Number.isFinite(b)) return Math.max(0, b);
  return (away?.unseen.total ?? 0) + prepushWaiting;
}

/**
 * Approve's label. In 0.9 Approve never runs a held action (D3), so it is always "Approve"; with
 * anything held it is disabled and `blocked` says why.
 */
export function approveState(item: Pick<ReviewFields, 'held_actions'>): { label: string; blocked: string | null } {
  return item.held_actions.length ? { label: 'Approve', blocked: 'Allow or deny the held action first' } : { label: 'Approve', blocked: null };
}

/** The verdict word Mira's spec shows. */
export const VERDICT_WORD: Record<VerdictResult, string> = { pass: 'Pass', fail: 'Fail', needs_you: 'Needs you', none: 'Not checked yet' };

/** "Pass · high" for the board card, or null with no verdict. */
export function verdictShort(v: Pick<InboxVerdict, 'result' | 'confidence'> | null | undefined): string | null {
  if (!v || v.result === 'none') return null;
  return v.confidence ? `${VERDICT_WORD[v.result]} · ${v.confidence}` : VERDICT_WORD[v.result];
}

// ---- API ----

export class InboxError extends Error {
  constructor(public status: number, message: string, public code: string | null = null, public paths: string[] = []) {
    super(message);
  }
  /** A 409 from verify: the work changed after its check, nothing was approved. */
  get stale(): boolean {
    return this.status === 409 && this.code === 'stale';
  }
}

async function call<T>(method: string, url: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new InboxError(401, e.message);
    throw new InboxError(0, 'the server didn’t answer');
  }
  const data = await res.json().catch(() => null);
  if (res.ok) return data as T;
  const d = data && typeof data === 'object' ? (data as Record<string, unknown>) : {};
  const msg = typeof d.error === 'string' && d.error ? d.error : `HTTP ${res.status}`;
  const code = typeof d.code === 'string' ? d.code : null;
  const paths = Array.isArray(d.paths) ? d.paths.filter((p): p is string => typeof p === 'string') : [];
  throw new InboxError(res.status, msg, code, paths);
}

const enc = encodeURIComponent;

export const inboxApi = {
  get: (since?: string) => call<InboxResponse>('GET', `/api/inbox${since ? `?since=${enc(since)}` : ''}`),
  /** Approve. Approve all always sends verdict_id; a 409 `stale` throws InboxError with `.stale` and `.paths`. */
  verify: (taskId: string, verdictId?: string | null) =>
    call<unknown>('POST', `/api/tasks/${enc(taskId)}/verify`, verdictId ? { verdict_id: verdictId } : {}),
  reject: (taskId: string, reason: string) => call<unknown>('POST', `/api/tasks/${enc(taskId)}/reject`, { reason }),
  /** "Check again" on a stale item. */
  runChecks: (taskId: string) => call<unknown>('POST', `/api/tasks/${enc(taskId)}/checks/run`, {}),
};

// ---- Fixture: every state on v1-states-board ----

const T0 = '2026-11-10T15:20:00Z';
const ago = (min: number) => new Date(Date.parse(T0) - min * 60_000).toISOString();
const link = (kwant: string) => ({ thread_id: null, channel_id: null, direct_kwant_id: kwant });

function review(over: Partial<ReviewItem> & Pick<ReviewItem, 'id' | 'kwant_id' | 'kwant_name' | 'title' | 'group'>): ReviewItem {
  const base: ReviewItem = {
    kind: 'review',
    summary: '',
    status: 'review',
    timestamp: ago(30),
    link: link(over.kwant_id),
    sort_key: [],
    task_id: `t-${over.id}`,
    mission_id: 'm-auth',
    project: 'shop',
    in_review_since: ago(30),
    round: { n: 1, max: 2 },
    verdict: {
      id: `v-${over.id}`,
      result: 'pass',
      confidence: 'high',
      line: 'All 4 criteria are met and every check re-ran clean.',
      checker: { engine: 'Codex', model: 'gpt-5.5-codex', tier_note: null },
      checked_at: ago(25),
      steps_run: 2,
      steps_total: 2,
    },
    risk: { level: 'low', reasons: [] },
    criteria: [
      { index: 1, text: 'Typo fixed in README', status: 'met', source: 'task', evidence: { kind: 'check', ref: 'test.py', label: 'test.py · 4 passed' }, note: null, round_added: 1 },
    ],
    criteria_summary: { met: 1, total: 1, not_met: 0, not_checked: 0 },
    couldnt_verify: [],
    decisions: { total: 0, top: [] },
    evidence: { checks: [], screenshots: [], pr: null },
    held_actions: [],
    previous_round: null,
    stale: null,
    approve_error: null,
    one_tap: false,
    batchable: false,
    rollback_ok: true,
    ...over,
  } as ReviewItem;
  return base;
}

const CRITERIA_ALL_STATES: InboxCriterion[] = [
  { index: 1, text: 'Met, with evidence', status: 'met', source: 'task', evidence: { kind: 'check', ref: 'test.py', label: 'test.py · 4 passed' }, note: null, round_added: 1 },
  { index: 2, text: 'The table keeps its last row', status: 'not_met', source: 'task', evidence: null, note: 'Checker: “drops the last row at 101 rows”', round_added: 1 },
  { index: 3, text: 'Works on Safari', status: 'not_checked', source: 'task', evidence: null, note: 'No evidence. See “Couldn’t check”.', round_added: 1 },
  { index: 4, text: 'Rate limit is per IP', status: 'claim_only', source: 'task', evidence: null, note: 'Lin says it works; nothing backs it.', round_added: 1 },
  { index: 5, text: 'Show the error under the field, not in a toast', status: 'met', source: 'person_note', evidence: { kind: 'screenshot', ref: 'shot-1', label: 'login-error.png' }, note: null, round_added: 2 },
  { index: 6, text: 'Don’t change the public login URL', status: 'met', source: 'must_not_change', evidence: null, note: null, round_added: 1 },
];

/** Needs you · medium, round 1, high risk: Waiting on you. */
const needsYou = review({
  id: 'r-needs', kwant_id: 'k-lin', kwant_name: 'Lin', title: 'Limit sign-in attempts', group: 'waiting_on_you', sort_key: [1, 0, 1, 'Lin', 'r-needs'],
  timestamp: ago(48), in_review_since: ago(48),
  verdict: { id: 'v-needs', result: 'needs_you', confidence: 'medium', line: 'The checks pass, but Lin assumed per-IP counting.', checker: { engine: 'Codex', model: 'gpt-5.5-codex', tier_note: null }, checked_at: ago(40), steps_run: 6, steps_total: 6 },
  risk: { level: 'high', reasons: ['touches auth', 'priority high'] },
  criteria: CRITERIA_ALL_STATES.slice(0, 4),
  criteria_summary: { met: 1, total: 4, not_met: 1, not_checked: 2 },
  couldnt_verify: [{ what: 'Safari login', why: 'no Safari in the clean copy' }],
  decisions: { total: 4, top: [
    { index: 1, question: 'Count failed sign-ins per IP or per account?', picked: 'Per IP', why: '“Per account lets an attacker lock anyone out.”', cost: null },
    { index: 2, question: 'Lockout length?', picked: '15 minutes', why: '“Matches the old session timeout.”', cost: null },
  ] },
  evidence: {
    checks: [{ name: 'unit', command: 'cargo test auth', result: 'pass', summary: '42 passed', ran_by: 'checker', output_tail: 'test auth::limit::per_ip ... ok\ntest result: ok. 42 passed; 0 failed' }],
    screenshots: [{ url: '/api/files/shot-1', caption: 'login-error.png' }],
    pr: { repo: 'shop', number: 512, url: 'https://github.com/example/shop/pull/512', add: 84, del: 12, files: 3, ci: 'passed' },
  },
  held_actions: [{ id: 'a-merge', tool_name: 'github', verb: 'merge', summary: 'merge pull request', args_display: 'github · merge pull request\nrepo shop · #512 → main · squash', gate: 'merge' }],
});

/** Fail at the round limit (round 2 of 2), on a different tier: Waiting on you, with round 2 data. */
const failRoundLimit = review({
  id: 'r-fail2', kwant_id: 'k-iris', kwant_name: 'Iris', title: 'Legend under the plot', group: 'waiting_on_you', sort_key: [1, 0, 2, 'Iris', 'r-fail2'],
  timestamp: ago(35), in_review_since: ago(35), round: { n: 2, max: 2 },
  verdict: { id: 'v-fail2', result: 'fail', confidence: 'high', line: 'Criterion 2 is still not met: the table drops the last row. Repro: rows=101.', checker: { engine: 'Claude', model: 'claude-sonnet-5-5', tier_note: 'different_tier' }, checked_at: ago(30), steps_run: 2, steps_total: 2 },
  risk: { level: 'medium', reasons: [] },
  criteria: CRITERIA_ALL_STATES,
  criteria_summary: { met: 3, total: 6, not_met: 1, not_checked: 2 },
  previous_round: {
    sent_back_at: ago(90), note: 'Show the error under the field, not in a toast',
    flips: [{ index: 2, from: 'not_met', to: 'met' }],
    diff: { files: 2, add: 14, del: 6 },
    bot_summary: '“Moved the error under the field and fixed the legend overlap.”',
  },
});

/** Not checked yet (no checker): To review, medium (null) risk; never one-tap, never batched. */
const notChecked = review({
  id: 'r-none', kwant_id: 'k-juno', kwant_name: 'Juno', title: 'Nightly export to CSV', group: 'to_review', sort_key: [3, 1, 3, 'Juno', 'r-none'],
  verdict: { id: null, result: 'none', confidence: null, line: '3 of 3 required checks passed. The checker hasn’t looked at this one.', checker: null, checked_at: null, steps_run: null, steps_total: null },
  risk: { level: null, reasons: [] },
  criteria: [{ index: 1, text: 'Export runs at 02:00', status: 'not_checked', source: 'task', evidence: null, note: null, round_added: 1 }],
  criteria_summary: { met: 0, total: 1, not_met: 0, not_checked: 1 },
  round: { n: 1, max: 2 },
});

/** Pass · high, high risk, held refund: To review, not one-tap (held + high risk). */
const passHeld = review({
  id: 'r-refund', kwant_id: 'k-vega', kwant_name: 'Vega', title: 'Refund the double charge', group: 'to_review', sort_key: [3, 0, 1, 'Vega', 'r-refund'],
  risk: { level: 'high', reasons: ['payment'] },
  held_actions: [{ id: 'a-refund', tool_name: 'stripe', verb: 'refund', summary: 'create refund', args_display: 'stripe · create refund\ncustomer cus_4f2… · 49.00 EUR', gate: 'payment' }],
});

/** Pass · high, medium risk, nothing held: one-tap, but alone (not in the batch). */
const oneTap = review({
  id: 'r-onetap', kwant_id: 'k-mira', kwant_name: 'Mira', title: 'Empty state copy', group: 'to_review', sort_key: [3, 1, 2, 'Mira', 'r-onetap'],
  risk: { level: 'medium', reasons: [] }, one_tap: true,
});

/** Freshness fail: Failed or stuck, with Check again. */
const stale = review({
  id: 'r-stale', kwant_id: 'k-rigel', kwant_name: 'Rigel', title: 'Fix typo in README', group: 'failed_or_stuck', sort_key: [2, 0, 1, 'Rigel', 'r-stale'],
  stale: { paths: ['README.md'], at: '2026-11-10T15:09:00Z' },
});

/** Action error notice (states board "Couldn't approve …"). */
const approveError = review({
  id: 'r-err', kwant_id: 'k-lin', kwant_name: 'Lin', title: 'Rotate the API key', group: 'failed_or_stuck', sort_key: [2, 0, 2, 'Lin', 'r-err'],
  approve_error: { code: 'refused', message: 'The merge was refused: main moved on. Nothing was approved.' },
});

/** Roll back not possible (verified elsewhere, no checkpoint). */
const noRollback = review({
  id: 'r-norb', kwant_id: 'k-ada', kwant_name: 'Ada', title: 'Release notes for 0.9', group: 'to_review', sort_key: [3, 1, 4, 'Ada', 'r-norb'],
  risk: { level: 'medium', reasons: [] }, rollback_ok: false,
});

const BATCH_NAMES: [string, string, string][] = [
  ['k-iris', 'Iris', 'Tooltip delay'],
  ['k-mira', 'Mira', 'Icon spacing in the drawer'],
  ['k-juno', 'Juno', 'Log line for retries'],
  ['k-lin', 'Lin', 'Bump serde'],
  ['k-vega', 'Vega', 'Fix a flaky test name'],
];
/** The Low-risk card's 5 items: pass · high · low risk · nothing held · nothing couldn't-check. */
export const FIXTURE_BATCH: ReviewItem[] = BATCH_NAMES.map(([kid, name, title], i) =>
  review({ id: `r-b${i + 1}`, kwant_id: kid, kwant_name: name, title, group: 'to_review', sort_key: [3, 2, i, name, `r-b${i + 1}`], one_tap: true, batchable: true }),
);

const question: InboxItem = {
  kind: 'question', id: 'q1', kwant_id: 'k-scout', kwant_name: 'Scout', title: 'Ship tonight?', summary: 'Choices: Yes · No', status: 'open',
  timestamp: ago(60), link: { thread_id: null, channel_id: 'c1', direct_kwant_id: null }, choices: ['Yes', 'No'], group: 'waiting_on_you', sort_key: [1, 0, 0, 'Scout', 'q1'],
};
const finished: InboxItem = {
  kind: 'delegation', id: 'd1', kwant_id: 'k-dev', kwant_name: 'Dev', title: 'Write the brief', summary: 'Brief is in the shared folder.', status: 'completed',
  timestamp: ago(10), link: { thread_id: 't1', channel_id: null, direct_kwant_id: null }, group: 'finished', sort_key: [4, 0, 0, 'Dev', 'd1'],
};

/** Every review state, one item each (for ReviewItemView stories and tests). */
export const FIXTURE_REVIEW_ITEMS: Record<string, ReviewItem> = {
  needsYou, failRoundLimit, notChecked, passHeld, oneTap, stale, approveError, noRollback, batchFirst: FIXTURE_BATCH[0],
};

/** A full GET /api/inbox answer in H5 + V1 order, with the Low-risk card. */
export const INBOX_FIXTURE: InboxResponse = {
  now: T0,
  since: ago(7 * 24 * 60),
  last_seen: ago(120),
  counts: { waiting_on_you: 3, failed_or_stuck: 2, to_review: 9, finished: 1, badge: 3 + 2 + 4 + 1 + 1 },
  items: [question, needsYou, failRoundLimit, stale, approveError, passHeld, notChecked, oneTap, noRollback, finished].sort(cmpSortKey),
  batch: { ids: FIXTURE_BATCH.map((i) => i.id), count: FIXTURE_BATCH.length },
  batch_items: FIXTURE_BATCH,
};

/** All clear. */
export const INBOX_FIXTURE_EMPTY: InboxResponse = {
  now: T0, since: ago(7 * 24 * 60), last_seen: ago(5),
  counts: { waiting_on_you: 0, failed_or_stuck: 0, to_review: 0, finished: 0, badge: 0 },
  items: [], batch: null,
};

/** Compares two sort keys element by element (numbers numerically, strings A→Z). */
export function cmpSortKey(a: Pick<InboxItem, 'sort_key'>, b: Pick<InboxItem, 'sort_key'>): number {
  const n = Math.max(a.sort_key.length, b.sort_key.length);
  for (let i = 0; i < n; i++) {
    const x = a.sort_key[i];
    const y = b.sort_key[i];
    if (x === y) continue;
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    if (typeof x === 'number' && typeof y === 'number') return x - y;
    return String(x).localeCompare(String(y));
  }
  return 0;
}
