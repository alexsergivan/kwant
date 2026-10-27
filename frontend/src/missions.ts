// Mission board (Mission Control + Verified Outcomes, Phase A). Contract:
// /home/maus/shared/ship-software-safely/mission-control/contract.md. Every unit of work (a mission
// task, a hand-off, a routine run, a Loose End taken on, a work-ahead run) is one row in `tasks`.
// Nothing here can mark a task verified on a bot's word: the board only shows what kwant checked,
// and every non-GET route is person-only on the server.

import { PersonCancelledError, personFetch } from './auth';
import { relativeTime } from './away';

export type TaskState = 'backlog' | 'ready' | 'working' | 'needs_you' | 'review' | 'verified' | 'blocked' | 'failed' | 'cancelled';
export type MissionStatus = 'draft' | 'active' | 'done' | 'cancelled';
export type SourceKind = 'mission' | 'person' | 'delegation' | 'routine' | 'zombie' | 'work_ahead' | 'chat';
export type CheckKind = 'command' | 'http' | 'file_exists' | 'file_contains' | 'file_sha256' | 'peer_review' | 'person';
export type CheckStatus = 'pending' | 'needs_approval' | 'running' | 'passed' | 'failed' | 'error';
export type ChecksStatus = 'not_checked' | 'pending' | 'passed' | 'failed';

export type StateCounts = Record<TaskState, number>;

export interface Mission {
  id: string;
  space_id: string;
  title: string;
  goal: string;
  status: MissionStatus;
  lead_kwant_id: string;
  lead_name: string | null;
  channel_id: string | null;
  direct_kwant_id: string | null;
  thread_id: string | null;
  budget_tokens: number;
  spent_tokens: number;
  reserved_tokens: number;
  over_budget: boolean;
  created_by: string;
  approved_at: string | null;
  approved_by: string | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  counts: StateCounts;
}

export interface NeedsYouItem {
  /** check_approval: a required check waits for the person's OK (id = the check's id); only on tasks in review.
   * Optional checks never appear here.
   * may_be_invalid: an open flag (id = the flag's id), in any state, verified included; resolve it with
   * `POST /api/tasks/:id/flags/:flag_id/resolve`. */
  kind: 'question' | 'approval' | 'human_step' | 'check_approval' | 'may_be_invalid';
  id: string;
  text: string;
}

export interface Claim {
  kwant_id: string;
  claimed_at: string;
  expires_at: string;
}

export interface ChecksSummary {
  status: ChecksStatus;
  passed: number;
  failed: number;
  /** Checks waiting for the person to allow them (older servers leave it out). */
  needs_approval?: number;
  /** How many of `needs_approval` are required, so hold the task (older servers leave it out). */
  required_needs_approval?: number;
  total: number;
}

export interface Task {
  id: string;
  space_id: string;
  mission_id: string | null;
  mission_title: string | null;
  title: string;
  acceptance: string;
  state: TaskState;
  priority: number;
  owner_kwant_id: string | null;
  owner_name: string | null;
  suggested_kwant_id: string | null;
  budget_tokens: number;
  spent_tokens: number;
  attempts: number;
  max_attempts: number;
  blocked_reason: string | null;
  source_kind: SourceKind;
  source_id: string | null;
  channel_id: string | null;
  direct_kwant_id: string | null;
  thread_id: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
  depends_on: string[];
  waiting_on: string[];
  claim: Claim | null;
  needs_you: NeedsYouItem[];
  checks_summary: ChecksSummary;
  /** The task this one redoes (a fork); older servers leave it out. */
  forked_from?: string | null;
  /** Open "may be invalid" flags; older servers leave it out. */
  flags?: TaskFlag[];
}

// Work graph (Phase B1, docs/mission-control.md §11)

export type FlagCause = 'evidence_changed' | 'upstream_evidence_changed' | 'upstream_forked';
export type FlagResolution = 'still_valid' | 'redo' | 'cancelled';

export interface TaskFlag {
  id: string;
  kind: 'may_be_invalid';
  cause: FlagCause;
  /** The upstream task, or the task itself for evidence_changed. */
  cause_task_id: string;
  cause_title: string | null;
  /** evidence_*: {path, old_sha256, new_sha256 (null when the file is gone)}; upstream_forked: {fork_id, reason}. */
  detail: Record<string, unknown>;
  created_at: string;
  resolved_at: string | null;
  resolution: FlagResolution | null;
}

export interface GraphNode {
  id: string;
  title: string;
  state: TaskState;
  owner_kwant_id: string | null;
  owner_name: string | null;
  attempts: number;
  max_attempts: number;
  forked_from: string | null;
  /** Tasks that depend on this one, directly or further down ("N more downstream"). */
  downstream: number;
  /** Open flags only. */
  flags: TaskFlag[];
  checks_summary: ChecksSummary;
}

/** `from` is the upstream task (depends_on_id), `to` the task that depends on it. */
export interface GraphEdge {
  from: string;
  to: string;
}

export interface ScanSkipped {
  task_id: string;
  path: string;
  why: string;
}

/** `GET /api/missions/:id/graph`, or `GET /api/graph` (tasks without a mission; mission_id null). */
export interface WorkGraph {
  mission_id: string | null;
  nodes: GraphNode[];
  edges: GraphEdge[];
  scanned_at: string | null;
  scan_skipped: ScanSkipped[];
}

/** `POST /api/missions/:id/graph/scan`. */
export interface GraphScanResult {
  scanned: number;
  flagged: string[];
  skipped: ScanSkipped[];
}

/** `POST /api/tasks/:id/flags/:flag_id/resolve`: the closed flag, the task, and the fork when `redo` forked a verified task. */
export interface ResolveFlagResult {
  flag: TaskFlag;
  task: Task;
  fork: Task | null;
}

export interface Check {
  id: string;
  task_id: string;
  kind: CheckKind;
  spec: Record<string, unknown>;
  required: boolean;
  approved: boolean;
  /** Who let it run and count; null while it waits for approval. */
  approved_by?: 'person' | 'mission' | 'auto' | null;
  /** Whether it can count toward Verified (the owner's own check can't until you allow it). */
  counts?: boolean;
  status: CheckStatus;
  output_excerpt: string | null;
  checked_by: string | null;
  checked_by_name: string | null;
  requested_by: string;
  evidence_id: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
}

export type EvidenceKind = 'file' | 'url' | 'note' | 'screenshot' | 'check_output';

export interface Evidence {
  id: string;
  task_id: string;
  kind: EvidenceKind;
  label: string;
  uri: string | null;
  path: string | null;
  sha256: string | null;
  bytes: number | null;
  excerpt: string | null;
  captured_by: string;
  run_id: string | null;
  captured_at: string;
}

export interface TaskEvent {
  id: number;
  task_id: string;
  mission_id: string | null;
  kind: string;
  actor: string;
  actor_name: string | null;
  from_state: TaskState | null;
  to_state: TaskState | null;
  detail: Record<string, unknown>;
  created_at: string;
}

export interface TaskDetail {
  task: Task;
  checks: Check[];
  evidence: Evidence[];
  events: TaskEvent[];
}

export interface MissionDetail {
  mission: Mission;
  tasks: Task[];
}

export interface Insights {
  verified_7d: number;
  false_success_7d: number;
  needs_you_now: number;
  blocked_now: number;
  /** Minutes tasks spent blocked over the last 7 days (older servers leave it out). */
  blocked_minutes_7d?: number;
  interventions_7d: number;
  tokens_7d: number;
  tokens_per_verified_7d: number | null;
}

// ---------------------------------------------------------------------------
// Board columns

export type Column = 'ready' | 'working' | 'needs_you' | 'review' | 'verified';

export const COLUMNS: Array<{ key: Column; label: string; hint: string; empty: string }> = [
  { key: 'ready', label: 'Ready', hint: 'Can start, or waiting on another task', empty: 'Nothing waiting to start.' },
  { key: 'working', label: 'Working', hint: 'A bot is on it now', empty: 'No bot is working on a task right now.' },
  { key: 'needs_you', label: 'Needs you', hint: 'A question, approval or step only you can do', empty: 'Nothing needs you.' },
  { key: 'review', label: 'Review', hint: 'Bot says done; waiting for a check', empty: 'Nothing waiting for a check.' },
  { key: 'verified', label: 'Verified', hint: 'A check passed', empty: 'Nothing verified yet. A task gets here only when a check passes.' },
];

/**
 * Which column a task sits in. Blocked and Failed are badges, not columns: the Task shape doesn't
 * carry the state before it, so a blocked task waits in Ready (it can't start) and a failed one in
 * Review (its checks failed; you decide whether to retry). Cancelled tasks are hidden (null).
 */
export function columnOf(state: TaskState): Column | null {
  switch (state) {
    case 'backlog':
    case 'ready':
    case 'blocked':
      return 'ready';
    case 'working':
      return 'working';
    case 'needs_you':
      return 'needs_you';
    case 'review':
    case 'failed':
      return 'review';
    case 'verified':
      return 'verified';
    default:
      return null;
  }
}

/** Urgent first, then high; inside a priority, the most recently changed first. */
export function sortTasks(tasks: Task[]): Task[] {
  return [...tasks].sort((a, b) => b.priority - a.priority || Date.parse(b.updated_at) - Date.parse(a.updated_at));
}

export function groupByColumn(tasks: Task[]): Record<Column, Task[]> {
  const out: Record<Column, Task[]> = { ready: [], working: [], needs_you: [], review: [], verified: [] };
  for (const t of sortTasks(tasks)) {
    const c = columnOf(t.state);
    if (c) out[c].push(t);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Labels

export type Tone = 'emerald' | 'amber' | 'rose' | 'sky' | 'violet' | 'zinc';

export const STATE_LABELS: Record<TaskState, { label: string; tone: Tone }> = {
  backlog: { label: 'Waiting', tone: 'zinc' },
  ready: { label: 'Ready', tone: 'sky' },
  working: { label: 'Working', tone: 'violet' },
  needs_you: { label: 'Needs you', tone: 'amber' },
  review: { label: 'Review', tone: 'sky' },
  verified: { label: 'Verified', tone: 'emerald' },
  blocked: { label: 'Blocked', tone: 'rose' },
  failed: { label: 'Failed', tone: 'rose' },
  cancelled: { label: 'Cancelled', tone: 'zinc' },
};

export const stateLabel = (s: string): { label: string; tone: Tone } => STATE_LABELS[s as TaskState] ?? { label: humanize(s), tone: 'zinc' };

export const PRIORITY_LABELS = ['Low', 'Normal', 'High', 'Urgent'];
export const priorityLabel = (p: number): string => PRIORITY_LABELS[Math.max(0, Math.min(3, Math.round(p)))] ?? 'Normal';

const SOURCES: Record<string, string> = {
  mission: 'Mission plan',
  person: 'Added by you',
  delegation: 'Hand-off',
  routine: 'Routine run',
  zombie: 'Loose end',
  work_ahead: 'Worked ahead',
  chat: 'Chat',
};
export const sourceLabel = (k: string): string => SOURCES[k] ?? humanize(k);

export const MISSION_STATUS: Record<MissionStatus, { label: string; tone: Tone }> = {
  draft: { label: 'Waiting for your OK', tone: 'amber' },
  active: { label: 'Active', tone: 'sky' },
  done: { label: 'Done', tone: 'emerald' },
  cancelled: { label: 'Cancelled', tone: 'zinc' },
};
export const missionStatusLabel = (s: string) => MISSION_STATUS[s as MissionStatus] ?? { label: humanize(s), tone: 'zinc' as Tone };

export function humanize(s: string): string {
  const t = s.replace(/[_-]+/g, ' ').trim();
  return t ? t[0].toUpperCase() + t.slice(1) : 'Other';
}

/** 950 → "950", 12 400 → "12.4k", 2 000 000 → "2M". */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n)) return '0';
  const a = Math.abs(n);
  if (a < 1000) return String(Math.round(n));
  if (a < 1_000_000) return `${trim(n / 1000)}k`;
  return `${trim(n / 1_000_000)}M`;
}
const trim = (x: number) => (Math.abs(x) >= 100 ? String(Math.round(x)) : x.toFixed(1).replace(/\.0$/, ''));

/** "12.4k of 50k tokens"; just the spend when there's no budget. */
export function costText(spent: number, budget: number): string {
  return budget > 0 ? `${formatTokens(spent)} of ${formatTokens(budget)} tokens` : `${formatTokens(spent)} tokens`;
}

/** 0..1 for a spend bar. */
export const costRatio = (spent: number, budget: number): number => (budget > 0 ? Math.max(0, Math.min(1, spent / budget)) : 0);

/** "45 min", "3 h", "1.5 h", "2 d": a plain length of time from minutes. */
export function durationText(minutes: number): string {
  const m = Math.max(0, Math.round(minutes));
  if (m < 60) return `${m} min`;
  const h = m / 60;
  if (h < 48) return `${h < 10 ? trim(Math.round(h * 2) / 2) : Math.round(h)} h`;
  const d = h / 24;
  return `${d < 10 ? trim(Math.round(d * 2) / 2) : Math.round(d)} d`;
}

/** "Waiting for you to allow 1 check" when checks wait for the person's OK, else null. */
export function waitingApprovalText(n: number | null | undefined): string | null {
  if (!n || n <= 0) return null;
  return n === 1 ? 'Waiting for you to allow 1 check' : `Waiting for you to allow ${n} checks`;
}

/** "Optional check waiting for your OK": it waits for the person but doesn't hold the task. */
export function optionalWaitingText(n: number | null | undefined): string | null {
  if (!n || n <= 0) return null;
  return n === 1 ? 'Optional check waiting for your OK' : `${n} optional checks waiting for your OK`;
}

/**
 * Whether checks waiting for the person hold the task, from the summary alone (cards only have the
 * summary). Only a required one does: `checks_summary.required_needs_approval`. A "pending" status
 * with none of those means a required check is still running, not that it waits for you. Older
 * servers leave the field out; that reads as 0, so they never show "Needs you" on a card.
 */
export function approvalHolds(s: ChecksSummary): boolean {
  return (s.required_needs_approval ?? 0) > 0;
}

/** Optional checks waiting for the person: they never hold the task. */
function optionalWaitingCount(s: ChecksSummary): number {
  return Math.max(0, (s.needs_approval ?? 0) - (s.required_needs_approval ?? 0));
}

/**
 * The one-line check chip on a card. It counts only checks that count (`passed`/`failed`), never
 * `total`: an optional check waiting for you isn't a check that didn't pass. A task that has never
 * been checked says so, plainly.
 */
export function checksText(s: ChecksSummary): { text: string; tone: Tone } {
  const n = (k: number, one: string, many: string) => (k === 1 ? one : many.replace('#', String(k)));
  if (approvalHolds(s)) return { text: n(s.required_needs_approval ?? 0, '1 check needs your OK', '# checks need your OK'), tone: 'amber' };
  if (s.status === 'not_checked') return { text: 'Not checked', tone: 'zinc' };
  if (s.status === 'failed') return { text: n(s.failed, '1 check failed', '# checks failed'), tone: 'rose' };
  if (s.status === 'pending') return { text: s.passed > 0 ? n(s.passed, '1 check passed, still checking', '# checks passed, still checking') : 'Checking', tone: 'zinc' };
  return { text: s.passed > 1 ? `${s.passed} checks passed` : 'Check passed', tone: 'emerald' };
}

/**
 * The grey note next to the chip for optional checks waiting for you ("1 optional check waiting"),
 * else null. Shown while a required check is still running too, since the optional one never holds
 * the task.
 */
export function optionalChecksNote(s: ChecksSummary): string | null {
  const k = optionalWaitingCount(s);
  if (k <= 0) return null;
  return k === 1 ? '1 optional check waiting' : `${k} optional checks waiting`;
}

/** "in 12 min", "in 1 h", or "now" once it has passed. */
export function untilText(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const m = Math.round((t - now) / 60000);
  if (m <= 0) return 'now';
  if (m < 60) return `in ${m} min`;
  const h = Math.round(m / 60);
  return `in ${h} h`;
}

export const ago = relativeTime;

/** Who did something: "you", "kwant", or the bot's name. */
export function actorName(actor: string, name: string | null | undefined, names: Record<string, string> = {}): string {
  if (actor === 'person') return 'you';
  if (actor === 'kwant') return 'kwant';
  return name || names[actor] || 'a bot';
}

/** The short status line under a card title. */
export function cardStatus(t: Task, names: Record<string, string> = {}, now: number = Date.now()): string | null {
  switch (t.state) {
    case 'blocked':
      return t.blocked_reason === 'budget' ? 'Out of budget' : t.blocked_reason ? `Blocked: ${t.blocked_reason}` : 'Blocked';
    case 'failed':
      return `Checks failed ${t.attempts} of ${t.max_attempts} times`;
    case 'backlog':
      if (t.waiting_on.length) return t.waiting_on.length === 1 ? 'Waiting on 1 task' : `Waiting on ${t.waiting_on.length} tasks`;
      return 'Waiting for the mission to be approved';
    case 'working':
      return t.claim ? `${names[t.claim.kwant_id] || t.owner_name || 'A bot'} started ${ago(t.claim.claimed_at, now)}` : 'Starting';
    case 'needs_you':
      return t.needs_you[0]?.text ?? 'A bot is waiting for you';
    case 'review':
      if (approvalHolds(t.checks_summary)) return waitingApprovalText(t.checks_summary.required_needs_approval);
      if (t.checks_summary.status === 'not_checked') return 'Bot says done. Not checked yet';
      // While a required check runs the card reads as checking; the grey note covers optional ones.
      if (t.checks_summary.status === 'pending') return 'Bot says done. Checking';
      return optionalWaitingText(optionalWaitingCount(t.checks_summary)) ?? 'Bot says done. Checking';
    case 'verified':
      return t.finished_at ? `Verified ${ago(t.finished_at, now)}` : 'Verified';
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Evidence receipt

export const CHECK_KINDS: Record<string, string> = {
  command: 'Command',
  http: 'Web address',
  file_exists: 'File exists',
  file_contains: 'File contains',
  file_sha256: 'File fingerprint',
  peer_review: 'Teammate review',
  person: 'You verified',
};
export const checkKindLabel = (k: string): string => CHECK_KINDS[k] ?? humanize(k);

const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);

/** What a check looks at, in one line: the command, the address, or the file. */
export function checkTarget(c: Pick<Check, 'kind' | 'spec'>): string {
  const s = c.spec ?? {};
  switch (c.kind) {
    case 'command':
      return str(s.command) ?? 'a command';
    case 'http': {
      const url = str(s.url) ?? 'an address';
      const status = typeof s.expect_status === 'number' ? ` answers ${s.expect_status}` : ' answers';
      const has = str(s.contains);
      return `${url}${status}${has ? ` and shows “${has}”` : ''}`;
    }
    case 'file_exists':
      return str(s.path) ?? 'a file';
    case 'file_contains':
      return `${str(s.path) ?? 'a file'} has “${str(s.text) ?? ''}”`;
    case 'file_sha256': {
      const h = str(s.sha256);
      return `${str(s.path) ?? 'a file'} matches ${h ? h.slice(0, 12) + '…' : 'its fingerprint'}`;
    }
    case 'peer_review':
    case 'person':
      return str(s.note) ?? '';
    default:
      return '';
  }
}

export const CHECK_STATUS: Record<CheckStatus, { label: string; tone: Tone }> = {
  passed: { label: 'Passed', tone: 'emerald' },
  failed: { label: 'Failed', tone: 'rose' },
  error: { label: 'Couldn’t run', tone: 'rose' },
  running: { label: 'Running', tone: 'violet' },
  pending: { label: 'Not run yet', tone: 'zinc' },
  needs_approval: { label: 'Needs your OK', tone: 'amber' },
};
export const checkStatusLabel = (s: string) => CHECK_STATUS[s as CheckStatus] ?? { label: humanize(s), tone: 'zinc' as Tone };

export interface ReceiptLine {
  check: Check;
  status: { label: string; tone: Tone };
  target: string;
  by: string | null;
  output: Evidence | null;
  /** The owner's own pass: shown, but it doesn't count toward Verified. */
  ownClaim: boolean;
  /** Why this check doesn't count toward Verified yet, or null when it does. */
  notCounting: string | null;
}

export interface Receipt {
  lines: ReceiptLine[];
  /** Evidence attached by bots (files, links, notes, screenshots); check output sits on its line. */
  attachments: Evidence[];
  /** No check has run (or none exists): the receipt says "Not checked", never hides it. */
  notChecked: boolean;
  /** The passing check that made it Verified, when there is one. */
  verifiedBy: ReceiptLine | null;
  /** Why a task in Review isn't Verified yet because required checks wait for the person ("Waiting for you to allow 1 check"). */
  waitingOnYou: string | null;
  /** Optional checks waiting for the person, in Review or Verified; they don't hold the task ("Optional check waiting for your OK"). */
  optionalWaiting: string | null;
  /** The task is Verified: a waiting optional check is offered quietly ("Run this optional check"). */
  settledVerified: boolean;
}

/** Builds the evidence receipt: every check with its result and output, newest result first per check. */
export function buildReceipt(d: Pick<TaskDetail, 'task' | 'checks' | 'evidence'>, names: Record<string, string> = {}): Receipt {
  const byId = new Map(d.evidence.map((e) => [e.id, e]));
  const owner = d.task.owner_kwant_id;
  const lines = [...d.checks]
    .sort((a, b) => Date.parse(b.finished_at ?? b.created_at) - Date.parse(a.finished_at ?? a.created_at))
    .map<ReceiptLine>((c) => ({
      check: c,
      status: checkStatusLabel(c.status),
      target: checkTarget(c),
      by: c.checked_by ? actorName(c.checked_by, c.checked_by_name, names) : null,
      output: c.evidence_id ? byId.get(c.evidence_id) ?? null : null,
      ownClaim: !!owner && c.checked_by === owner,
      notCounting:
        c.counts !== false
          ? null
          : !!owner && c.requested_by === owner
            ? 'Doesn’t count yet: the task’s own bot added it. It counts once you allow it.'
            : 'Doesn’t count toward Verified yet.',
    }));
  const ran = lines.some((l) => l.check.status === 'passed' || l.check.status === 'failed' || l.check.status === 'error');
  const verifiedBy = d.task.state === 'verified' ? lines.find((l) => l.check.status === 'passed' && !l.ownClaim && !l.notCounting) ?? null : null;
  const waitingChecks = d.checks.filter((c) => c.status === 'needs_approval');
  const requiredWaiting = waitingChecks.filter((c) => c.required).length;
  const review = d.task.state === 'review';
  const settled = review || d.task.state === 'verified';
  return {
    lines,
    attachments: d.evidence.filter((e) => e.kind !== 'check_output').sort((a, b) => Date.parse(b.captured_at) - Date.parse(a.captured_at)),
    notChecked: !ran,
    verifiedBy,
    // From the checks, not the summary: "pending" can mean a required check is still running.
    waitingOnYou: review ? waitingApprovalText(requiredWaiting) : null,
    optionalWaiting: settled ? optionalWaitingText(waitingChecks.length - requiredWaiting) : null,
    settledVerified: d.task.state === 'verified',
  };
}

/** 2 048 → "2 KB". */
export function formatBytes(n: number | null | undefined): string | null {
  if (typeof n !== 'number' || !Number.isFinite(n)) return null;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1).replace(/\.0$/, '')} MB`;
}

/** Only web links open from the board; file paths and anything else are shown, not linked. */
export const safeHttps = (url: string | null | undefined): string | null => {
  if (!url) return null;
  try {
    const u = new URL(url);
    return u.protocol === 'https:' || u.protocol === 'http:' ? u.href : null;
  } catch {
    return null;
  }
};

// ---------------------------------------------------------------------------
// Activity

/** One plain line per event for the activity list. */
export function eventText(e: TaskEvent, names: Record<string, string> = {}): string {
  const who = actorName(e.actor, e.actor_name, names);
  const Who = who[0].toUpperCase() + who.slice(1);
  const d = e.detail ?? {};
  const note = str(d.note) ?? str(d.reason);
  switch (e.kind) {
    case 'created':
      return `${Who} added this task`;
    case 'state':
      return `${Who} moved it to ${stateLabel(e.to_state ?? '').label}${note ? `: ${note}` : ''}`;
    case 'claimed':
      return `${Who} took it on`;
    case 'released':
      return `${Who} let it go${note ? `: ${note}` : ''}`;
    case 'lease_expired':
      return 'The claim ran out with no activity, so it went back to Ready';
    case 'note':
      return `${Who}: ${note ?? 'added a note'}`;
    case 'evidence':
      return `${Who} attached evidence${str(d.label) ? `: ${str(d.label)}` : ''}`;
    case 'check_added':
      return `${Who} added a check${str(d.kind) ? ` (${checkKindLabel(String(d.kind)).toLowerCase()})` : ''}`;
    case 'check_started':
      return 'A check started';
    case 'check_result':
      return `A check ${str(d.status) === 'passed' ? 'passed' : str(d.status) === 'error' ? 'couldn’t run' : 'failed'}`;
    case 'review':
      return `${Who} reviewed it: ${str(d.verdict) === 'pass' ? 'pass' : 'fail'}${note ? `. ${note}` : ''}`;
    case 'budget':
      return 'It reached its token budget';
    case 'edited':
      return `${Who} edited it`;
    default:
      return `${Who}: ${humanize(e.kind)}`;
  }
}

// ---------------------------------------------------------------------------
// Person actions

export type TaskAction = 'verify' | 'reject' | 'block' | 'unblock' | 'retry' | 'cancel' | 'run_checks';

/** The buttons the person gets on a task in its state. Verify only appears in Review. */
export function actionsFor(t: Pick<Task, 'state'>): TaskAction[] {
  switch (t.state) {
    case 'review':
      return ['verify', 'reject', 'run_checks', 'cancel'];
    case 'failed':
      return ['retry', 'cancel'];
    case 'blocked':
      return ['unblock', 'cancel'];
    case 'backlog':
    case 'ready':
    case 'working':
    case 'needs_you':
      return ['block', 'cancel'];
    default:
      return [];
  }
}

export const ACTION_LABELS: Record<TaskAction, string> = {
  verify: 'Mark verified',
  reject: 'Send back',
  block: 'Block',
  unblock: 'Unblock',
  retry: 'Try again',
  cancel: 'Cancel task',
  run_checks: 'Run checks',
};

/** Actions that need a reason typed first. */
export const NEEDS_REASON: Partial<Record<TaskAction, string>> = {
  reject: 'What still needs doing?',
  block: 'Why is it blocked?',
};

export const ACTION_DONE: Record<TaskAction, string> = {
  verify: 'Marked verified by you.',
  reject: 'Sent back to Ready.',
  block: 'Blocked.',
  unblock: 'Unblocked.',
  retry: 'Back in Ready for another try.',
  cancel: 'Cancelled.',
  run_checks: 'Checks ran.',
};

// ---------------------------------------------------------------------------
// Live updates: App's EventSource re-broadcasts `task_updated` and `mission_updated` here.

export type MissionEvent = { type: 'task_updated'; data: { task: Task } } | { type: 'mission_updated'; data: { mission: Mission } };

const bus = typeof EventTarget !== 'undefined' ? new EventTarget() : null;

export function emitMission(e: MissionEvent) {
  bus?.dispatchEvent(new CustomEvent(e.type, { detail: e.data }));
}

export function onMission<T extends MissionEvent['type']>(type: T, fn: (data: Extract<MissionEvent, { type: T }>['data']) => void): () => void {
  const listener = (ev: Event) => fn((ev as CustomEvent).detail);
  bus?.addEventListener(type, listener);
  return () => bus?.removeEventListener(type, listener);
}

/** Puts a changed task into the list (or drops it when it's cancelled and cancelled are hidden). */
export function upsertTask(list: Task[], t: Task, showCancelled = false): Task[] {
  const rest = list.filter((x) => x.id !== t.id);
  if (t.state === 'cancelled' && !showCancelled) return rest;
  return [t, ...rest];
}

export function upsertMission(list: Mission[], m: Mission): Mission[] {
  return list.some((x) => x.id === m.id) ? list.map((x) => (x.id === m.id ? m : x)) : [m, ...list];
}

/** Fired by App settings after saving, so the sidebar shows or hides the board at once. */
export const MISSION_BOARD_SETTING_EVENT = 'kwnt:mission-board-setting';

/** `mission_board` from GET /api/settings; null on a server without Mission Control. */
export function parseMissionBoardSetting(data: unknown): boolean | null {
  if (!data || typeof data !== 'object') return null;
  const v = (data as Record<string, unknown>).mission_board;
  return typeof v === 'boolean' ? v : null;
}

// ---------------------------------------------------------------------------
// API

export class MissionError extends Error {
  constructor(
    message: string,
    public status: number,
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

export function errorFrom(status: number, answer: unknown): MissionError {
  const a = answer && typeof answer === 'object' ? (answer as Record<string, unknown>) : {};
  const err = typeof a.error === 'string' && a.error.trim() ? sentence(a.error) : null;
  if (status === 403) return new MissionError('Only you can change tasks on the board.', 403);
  if (status === 404 && !err) return new MissionError('This server doesn’t have the Mission board yet. Update the app and try again.', 404);
  return new MissionError(err ?? `Something went wrong (HTTP ${status}).`, status);
}

async function call<T>(url: string, method = 'GET', body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new MissionError(e.message, 401);
    throw new MissionError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const answer = await res.json().catch(() => null);
  if (res.ok) return answer as T;
  throw errorFrom(res.status, answer);
}

const enc = encodeURIComponent;

export const missionsApi = {
  missions: () => call<{ missions: Mission[] }>('/api/missions').then((r) => r.missions),
  mission: (id: string) => call<MissionDetail>(`/api/missions/${enc(id)}`),
  insights: () => call<Insights>('/api/missions/insights'),
  approveMission: (id: string) => call<Mission>(`/api/missions/${enc(id)}/approve`, 'POST', {}),
  cancelMission: (id: string) => call<Mission>(`/api/missions/${enc(id)}/cancel`, 'POST', {}),
  setBudget: (id: string, budget_tokens: number) => call<Mission>(`/api/missions/${enc(id)}`, 'PATCH', { budget_tokens }),
  tasks: () => call<{ tasks: Task[] }>('/api/tasks?limit=500').then((r) => r.tasks),
  task: (id: string) => call<TaskDetail>(`/api/tasks/${enc(id)}`),
  approveCheck: (id: string) => call<Check>(`/api/checks/${enc(id)}/approve`, 'POST', {}),
  /** One person action on a task. `run_checks` answers `{checks, task}`, the others the Task. */
  act: async (id: string, action: TaskAction, reason?: string): Promise<Task> => {
    const base = `/api/tasks/${enc(id)}`;
    switch (action) {
      case 'verify':
        return call<Task>(`${base}/verify`, 'POST', reason ? { note: reason } : {});
      case 'reject':
        return call<Task>(`${base}/reject`, 'POST', { reason: reason ?? '' });
      case 'block':
        return call<Task>(`${base}/block`, 'POST', { reason: reason ?? '' });
      case 'run_checks':
        return call<{ task: Task }>(`${base}/checks/run`, 'POST', {}).then((r) => r.task);
      default:
        return call<Task>(`${base}/${action}`, 'POST', {});
    }
  },
};

/** Where a task came from in the app: a channel, a bot's thread, or a bot's direct chat. */
export function sourceChat(t: Pick<Task, 'channel_id' | 'direct_kwant_id' | 'thread_id'>): { channelId: string | null; kwantId: string | null; threadId: string | null } | null {
  if (t.channel_id) return { channelId: t.channel_id, kwantId: null, threadId: t.thread_id };
  if (t.direct_kwant_id) return { channelId: null, kwantId: t.direct_kwant_id, threadId: t.thread_id };
  return null;
}
