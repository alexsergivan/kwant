// Mission board (Mission Control + Verified Outcomes, Phase A). Contract:
// /home/maus/shared/ship-software-safely/mission-control/contract.md. Every unit of work (a mission
// task, a hand-off, a routine run, a Loose End taken on, a work-ahead run) is one row in `tasks`.
// Nothing here can mark a task verified on a bot's word: the board only shows what kwant checked,
// and every non-GET route is person-only on the server.

import { PersonCancelledError, personFetch } from './auth';
import { relativeTime } from './away';
import { Capabilities, CapabilityChange, CapabilityChangedEvent, ENGINE_CANT_SANDBOX, ProposalDecision, ProposalStatus, Scorecard, TaskTier, Tier, TierProposal } from './flightSchool';
import type { Risk, RiskResolution, TaskRisks } from './auditor';
import { Dispute, DisputeStatus, Fact, FactChangedEvent, FactDetail, FactDisputeEvent, FactStatus, ledgerSetting } from './truthLedger';

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
   * `POST /api/tasks/:id/flags/:flag_id/resolve`.
   * autofix_suggested: a retry waits for the person (id = the attempt's id); run it with `POST /api/tasks/:id/autofix`.
   * fix_needed: a failure AutoFix never retries (id = the failure's id). workflow_change: a proposed change
   * (id = the change's id); approve or reject it under /api/workflow-changes.
   * engine_cant_sandbox: Flight School blocked the task because its bot is below Act on an engine kwant
   * can't hold to a tier (id = the task's id).
   * fact_changed: an open task cites a fact that is now disputed or was replaced (cleared with
   * `POST /api/facts/:id/uncite`). rests_on_disputed_fact: a verified task's fact is disputed. */
  kind:
    | 'question'
    | 'approval'
    | 'human_step'
    | 'check_approval'
    | 'may_be_invalid'
    | 'autofix_suggested'
    | 'fix_needed'
    | 'workflow_change'
    | 'engine_cant_sandbox'
    | 'fact_changed'
    | 'rests_on_disputed_fact';
  id: string;
  text: string;
  /** fact_changed, rests_on_disputed_fact: the fact (id = fact_id), its status now and what replaced it. */
  fact_id?: string;
  status?: string;
  superseded_by?: string | null;
  /** question, approval, human_step: the chat the card is in (the owner's hand-off thread, a
   * channel or a direct chat), which may not be the chat the task came from. */
  chat?: { channel_id: string | null; kwant_id: string | null; thread_id: string | null };
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

export interface Task extends TaskTier {
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
  /** Who reviews a hand-off's result: the bot that handed it out. Older servers leave it out. */
  reviewer?: { kwant_id: string; name: string | null } | null;
  /** Set when a one-way hand-off's card was closed without review ("Notice, no review needed"):
   *  it reads as Closed, not Cancelled. Older servers leave it out. */
  closed_note?: string | null;
  /** Set when a restore held the task: nothing claims it until `POST /api/tasks/:id/run`
   *  (the person's Run). Older servers leave it out. */
  held_at?: string | null;
  /** Open "may be invalid" flags; older servers leave it out. */
  flags?: TaskFlag[];
  /** Time Machine checkpoints of this task; older servers leave these out. */
  checkpoint_count?: number;
  last_checkpoint_at?: string | null;
  /** AutoFix (Phase B3); older servers leave these out. Recovery spend is part of `spent_tokens`. */
  recovery_spent_tokens?: number;
  recovery_budget_tokens?: number;
  last_failure?: { class: FailureClass; summary: string; created_at: string } | null;
  autofix?: AutofixAttempt | null;
  /** Retries AutoFix started for this task (running, succeeded or failed). */
  autofix_attempts?: number;
}

// AutoFix (Phase B3, docs/mission-control.md §13)

export type AutofixMode = 'off' | 'suggest' | 'on';
export type FailureClass =
  | 'transient' | 'stalled' | 'check_failed' | 'bad_spec' | 'auth' | 'quota' | 'budget' | 'denied' | 'needs_input' | 'unknown';
export type FailureSource = 'check' | 'turn' | 'lease' | 'budget' | 'routine' | 'handoff';
export type AutofixStrategy =
  | 'wait_and_retry' | 'retry_from_checkpoint' | 'retry_with_diagnostics' | 'retry_same' | 'propose_change' | 'none';
export type AutofixStatus = 'suggested' | 'scheduled' | 'running' | 'succeeded' | 'failed' | 'skipped';

export interface AutofixAttempt {
  id: string;
  strategy: AutofixStrategy;
  status: AutofixStatus;
  /** When a scheduled retry may start (backoff); null to start as soon as possible. */
  not_before: string | null;
  /** Why; for a skipped attempt it starts with `no_attempts_left`, `over_recovery_budget` or `not_retryable`. */
  reason: string;
  tokens_spent: number;
  /** A routine retry's new task. */
  retry_task_id: string | null;
}

/** GET /api/tasks/:id/failures, oldest first. */
export interface TaskFailure {
  id: string;
  attempt: number;
  source: FailureSource;
  class: FailureClass;
  summary: string;
  /** Redacted output, at most 2,000 characters. */
  excerpt: string | null;
  checkpoint_id: string | null;
  created_at: string;
  autofix: AutofixAttempt | null;
}

/** 409 from POST /api/tasks/:id/autofix. */
export interface AutofixError {
  error: string;
  code: 'not_retryable' | 'no_attempts_left' | 'over_recovery_budget';
}

export type WorkflowTarget = 'check_spec' | 'task_acceptance' | 'max_attempts' | 'routine_instructions' | 'routine_schedule';
export type WorkflowChangeStatus = 'proposed' | 'approved' | 'rejected' | 'applied' | 'stale';

/** GET /api/workflow-changes?status=…&task_id=… (person only; both filters optional). */
export interface WorkflowChange {
  id: string;
  task_id: string | null;
  routine_id: string | null;
  target: WorkflowTarget;
  before: unknown;
  after: unknown;
  reason: string;
  proposed_by: string;
  proposed_by_name: string | null;
  status: WorkflowChangeStatus;
  decided_at: string | null;
  created_at: string;
}

// Time Machine (Phase B2, docs/mission-control.md §12)

export type CheckpointKind = 'before_run' | 'after_run' | 'manual' | 'before_restore';

/** A tool call the task's runs made that kwant can't undo. */
export interface ExternalCall {
  tool_name: string;
  /** Tool name plus a redacted outline of the arguments (≤ 300 chars, never a body). */
  summary: string;
  at: string;
}

/** `GET /api/tasks/:id/checkpoints` item (oldest first); `POST` (person-only) returns one. */
export interface Checkpoint {
  id: string;
  kind: CheckpointKind;
  attempt: number;
  created_at: string;
  created_by: 'kwant' | 'person';
  files: number;
  bytes: number;
  skipped: number;
  git_head: string | null;
  /** Why the snapshot failed; such a checkpoint isn't restorable. */
  error: string | null;
  restorable: boolean;
  /** Tool calls of this task between this checkpoint and the next one. */
  effects_after: { workspace: number; external: ExternalCall[] };
}

export type RestoreAction = 'overwrite' | 'create' | 'delete';
export type RestoreRefusal = 'fork_instead' | 'running';

/** `GET /api/tasks/:id/checkpoints/:cid/restore-preview`. Only paths this task's runs changed are listed. */
export interface RestorePreview {
  checkpoint_id: string;
  /** Send back as `preview_sha256`; a changed folder or task makes the restore 409 `stale_preview`. */
  preview_sha256: string;
  changes: { path: string; action: RestoreAction }[];
  /** Changed since the checkpoint by someone else: skipped unless named in `force_paths`.
   *  `changed_by_task` names a task only when that task's own file calls wrote the path;
   *  otherwise it is null and `why` is "changed outside this task". */
  conflicts: { path: string; changed_by_task: string | null; why?: string }[];
  skipped: { path: string; why: string }[];
  external_not_undone: ExternalCall[];
  /** Always "kwant can't undo these. Check them yourself." */
  not_undone_note: string;
  /** False when a run changed files but logged no call that could have: show "side effects unknown", never "none". */
  effects_known: boolean;
  task_state_after: 'ready' | 'backlog' | null;
  refused: RestoreRefusal | null;
}

/** `POST /api/tasks/:id/checkpoints/:cid/restore` (person-only; `Idempotency-Key` header required). */
export interface RestoreResult {
  restored: string[];
  deleted: string[];
  /** Paths that couldn't be written (e.g. a symlink appeared). */
  failed: { path: string; why: string }[];
  /** Restore this one to undo the restore. */
  before_restore_checkpoint_id: string;
  external_not_undone: ExternalCall[];
  not_undone_note: string;
  effects_known: boolean;
  task: Task;
}

/** 409 bodies carry `code`: `fork_instead`, `running`, `stale_preview` or `not_restorable`. */
export interface RestoreError {
  error: string;
  code?: RestoreRefusal | 'stale_preview' | 'not_restorable' | 'checkpoint_failed';
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
  /** AutoFix over the last 7 days (older servers leave these out). */
  autofix_7d?: { attempts: number; succeeded: number; tokens: number };
  failures_by_class_7d?: Partial<Record<FailureClass, number>>;
  /** Measures (older servers leave these out): 8 rolling weeks, oldest first; the last is `verified_7d`. */
  verified_weekly?: Array<{ start: string; end: string; verified: number; active: boolean }>;
  active_weeks?: number;
  verified_per_active_week?: number | null;
  /** Reported model cost of tasks settled in the last 7 days; null when none reported one. */
  cost_7d_usd?: number | null;
  /** `cost_7d_usd` over `verified_priced_7d`; null when no verified task reported a cost. */
  cost_per_verified_7d_usd?: number | null;
  verified_observed_7d?: number;
  /** Verified tasks that reported a cost: the divisor of `cost_per_verified_7d_usd`. */
  verified_priced_7d?: number;
  settled_7d?: number;
  /** Settled tasks with no reported cost: above 0, the cost undercounts. */
  unpriced_7d?: number;
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

/**
 * Which column a task card sits in: its state's column, except that an open "may be invalid" flag, an
 * AutoFix item (a retry to OK, something to fix, a proposed change) or a required check waiting for the
 * person's OK puts it in Needs you: they wait for the person in any state.
 */
export function columnOfTask(t: Pick<Task, 'state' | 'flags' | 'needs_you'>): Column | null {
  const c = columnOf(t.state);
  return c && (openFlags(t).length || (t.needs_you ?? []).some((n) => PERSON_ITEMS.has(n.kind))) ? 'needs_you' : c;
}

/** Items that wait for the person whatever the task's state, so its card sits in Needs you. */
const PERSON_ITEMS = new Set<NeedsYouItem['kind']>(['autofix_suggested', 'fix_needed', 'workflow_change', 'check_approval', ENGINE_CANT_SANDBOX]);

/**
 * The "Needs you now" count: the cards the Needs you column holds across all work, so the tile and the
 * column never disagree. The server's `needs_you_now` counts the same tasks.
 */
export function needsYouCount(tasks: Task[]): number {
  return tasks.filter((t) => columnOfTask(t) === 'needs_you').length;
}

export function groupByColumn(tasks: Task[]): Record<Column, Task[]> {
  const out: Record<Column, Task[]> = { ready: [], working: [], needs_you: [], review: [], verified: [] };
  for (const t of sortTasks(tasks)) {
    const c = columnOfTask(t);
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

/** The note a one-way hand-off's card is closed with: nothing to review, so not Cancelled but Closed. */
export const NOTICE_CLOSED_NOTE = 'Notice, no review needed';
const CLOSED = { label: 'Closed', tone: 'zinc' as Tone };

/** A task's state label: a closed notice reads "Closed", everything else as its state. */
export const taskStateLabel = (t: Pick<Task, 'state' | 'closed_note'>): { label: string; tone: Tone } =>
  t.state === 'cancelled' && t.closed_note ? CLOSED : stateLabel(t.state);

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

/** A dollar amount as people read it: "$0.33", "$12", "<$0.01". Null stays null, never "$0". */
export function usdText(n: number | null | undefined): string | null {
  if (typeof n !== 'number' || !Number.isFinite(n) || n < 0) return null;
  if (n === 0) return '$0.00';
  if (n < 0.01) return '<$0.01';
  if (n < 100) return `$${n.toFixed(2)}`;
  return `$${Math.round(n).toLocaleString('en-US')}`;
}

export interface WeekBar {
  /** "Oct 25" for the week's last day. */
  label: string;
  verified: number;
  active: boolean;
  /** 0–1 against the busiest week. */
  height: number;
  /** One line for screen readers and the tooltip. */
  text: string;
}

export interface MeasuresView {
  /** Verified per active week, one decimal, or "—" with no active week. */
  headline: string;
  headlineSub: string;
  weeks: WeekBar[];
  /** "$0.33", "Not reported" (cost unknown) or "—" (nothing settled). */
  cost: string;
  costSub: string;
  /** Set when some settled runs reported no cost: what the figure is based on instead. */
  costWarning: string | null;
  /** The bars for one summary line, e.g. "0, 0, 1, 3, 0, 2, 4, 2". */
  trendText: string;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The day a rolling week ends on, as "Oct 25". The last week ends now, so it reads "This week". */
export function weekLabel(endIso: string, last: boolean, timeZone?: string): string {
  if (last) return 'This week';
  const d = new Date(endIso);
  if (Number.isNaN(d.getTime())) return '';
  // The week runs up to `end`, exclusive: label it with its last whole day.
  const day = new Date(d.getTime() - 1);
  return `Week to ${day.toLocaleDateString('en-US', { month: 'short', day: 'numeric', ...(timeZone ? { timeZone } : {}) })}`;
}

/**
 * The Phase C measures (verified per active week and cost per verified result), or null on a server
 * that doesn't send them, so the board looks as before.
 */
export function measuresView(i: Insights | null | undefined, timeZone?: string): MeasuresView | null {
  if (!i || !Array.isArray(i.verified_weekly) || i.verified_weekly.length === 0) return null;
  const weeks = i.verified_weekly;
  const max = Math.max(1, ...weeks.map((w) => w.verified));
  const bars: WeekBar[] = weeks.map((w, n) => {
    const label = weekLabel(w.end, n === weeks.length - 1, timeZone);
    const text = `${label}: ${w.verified} verified${w.active ? '' : ', no activity'}`;
    return { label, verified: w.verified, active: w.active, height: w.verified / max, text };
  });
  const active = i.active_weeks ?? weeks.filter((w) => w.active).length;
  const per = i.verified_per_active_week;
  const headline = typeof per === 'number' && active > 0 ? per.toFixed(1).replace(/\.0$/, '') : '—';
  const headlineSub =
    active > 0
      ? `Average over ${plural(active, 'active week')} of the last ${weeks.length}`
      : `No task activity in the last ${weeks.length} weeks yet`;

  const settled = i.settled_7d ?? 0;
  const unpriced = i.unpriced_7d ?? 0;
  const verified = i.verified_observed_7d ?? i.verified_7d;
  // The divisor of the per-result cost: verified tasks that reported one.
  const priced = i.verified_priced_7d ?? verified;
  const perCost = usdText(i.cost_per_verified_7d_usd);
  const total = usdText(i.cost_7d_usd);
  let cost: string;
  let costSub: string;
  if (perCost) {
    cost = perCost;
    costSub = `${total ?? '$0.00'} over ${plural(settled, 'settled task')} this week, failed runs included`;
  } else if (settled === 0) {
    cost = '—';
    costSub = 'No tasks settled this week';
  } else if (total && verified === 0) {
    // Money went out but nothing was verified: there's no "per result" to show, so show what it cost.
    cost = '—';
    costSub = `${total} spent on ${plural(settled, 'settled task')} with nothing verified yet`;
  } else if (total) {
    // Some runs reported a cost, but none of the verified ones did: no honest per-result figure.
    cost = 'Not reported';
    costSub = `None of the ${plural(verified, 'verified result')} reported a cost; ${total} came from other runs`;
  } else {
    cost = 'Not reported';
    costSub = `None of the ${plural(settled, 'settled task')} this week reported a cost`;
  }
  // With runs missing a cost, the figure is an estimate from the priced ones: say what it rests on.
  const costWarning =
    unpriced > 0 && perCost
      ? `Based on ${plural(priced, 'verified result')} that reported a cost; ${plural(unpriced, 'run')} reported none.`
      : null;
  return { headline, headlineSub, weeks: bars, cost, costSub, costWarning, trendText: weeks.map((w) => w.verified).join(', ') };
}

/** "12.4k of 50k tokens"; "64.9k tokens used" when there's no limit (budget 0). */
export function costText(spent: number, budget: number): string {
  return budget > 0 ? `${formatTokens(spent)} of ${formatTokens(budget)} tokens` : `${formatTokens(spent)} tokens used`;
}

// ---------------------------------------------------------------------------
// Token limits (docs/mission-control.md §15): budget_tokens 0 is "no limit", the default.

export const NO_LIMIT = 0;
/** Quick picks in the limit editor and Settings. */
export const LIMIT_PICKS = [50_000, 200_000, 1_000_000] as const;
/** What the server takes: a task 1–5M, a mission 1000–5M, one request's hand-offs 1–100M (0 is no limit for all). */
export const LIMIT_RANGE = { task: [1, 5_000_000], mission: [1_000, 5_000_000], request: [1, 100_000_000] } as const;
export type LimitScope = keyof typeof LIMIT_RANGE;

/** "50k", "1.5M", "200 000", "200,000" → tokens; null when it isn't a number. */
export function parseLimit(raw: string): number | null {
  const s = raw.trim().toLowerCase().replace(/[\s_,]/g, '').replace(/tokens?$/, '');
  const m = /^(\d+(?:\.\d+)?)([km]?)$/.exec(s);
  if (!m) return null;
  const n = Math.round(Number(m[1]) * (m[2] === 'k' ? 1_000 : m[2] === 'm' ? 1_000_000 : 1));
  return Number.isFinite(n) ? n : null;
}

/** Why a typed limit can't be used, or null. `above` is what's already spent: a lower limit would stop it at once. */
export function limitError(raw: string, scope: LimitScope, above = 0): string | null {
  const n = parseLimit(raw);
  const [lo, hi] = LIMIT_RANGE[scope];
  if (n === null) return 'Enter a number of tokens, like 300k or 2M.';
  if (n < lo) return `At least ${formatTokens(lo)} tokens.`;
  if (n > hi) return `At most ${formatTokens(hi)} tokens.`;
  if (above > 0 && n <= above) return `It has already used ${formatTokens(above)}, so pick more than that.`;
  return null;
}

/** "No limit" or "50k tokens". */
export const limitLabel = (budget: number): string => (budget > 0 ? `${formatTokens(budget)} tokens` : 'No limit');

/** Settings' default limits for new tasks and missions (0 = none); null on a server without them. */
export interface DefaultLimits {
  task: number;
  mission: number;
}
export function parseDefaultLimits(d: { mission_default_task_budget_tokens?: unknown; mission_default_budget_tokens?: unknown }): DefaultLimits | null {
  const n = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? Math.round(v) : null);
  const task = n(d.mission_default_task_budget_tokens);
  if (task === null) return null;
  return { task, mission: n(d.mission_default_budget_tokens) ?? NO_LIMIT };
}
/** Settings' hand-off token limit per request (`chain_token_budget`) as typed: 0 is no limit, the default; null while it can't be saved. */
export function requestLimitFrom(raw: string): number | null {
  const n = parseLimit(raw);
  if (n === null) return null;
  return n === NO_LIMIT || !limitError(raw, 'request') ? n : null;
}
export const defaultLimitsBody = (l: DefaultLimits) => ({ mission_default_task_budget_tokens: l.task, mission_default_budget_tokens: l.mission });

/** The task stopped because it used up its own token limit. */
export const outOfBudget = (t: Pick<Task, 'state' | 'blocked_reason'>): boolean => t.state === 'blocked' && t.blocked_reason === 'budget';

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
      if (t.needs_you.some((n) => n.kind === ENGINE_CANT_SANDBOX)) return 'Blocked: its engine can’t be held to its tier';
      return t.blocked_reason === 'budget' ? 'Reached its token limit' : t.blocked_reason ? `Blocked: ${t.blocked_reason}` : 'Blocked';
    case 'failed': {
      const auto = autofixCardText(t, now);
      if (auto) return auto;
      if (t.last_failure) return `${failureClassLabel(t.last_failure.class)} (try ${t.attempts} of ${t.max_attempts})`;
      return `Checks failed ${t.attempts} of ${t.max_attempts} times`;
    }
    case 'backlog':
      if (t.held_at) return 'On hold after restore. Press Run to start it';
      if (t.waiting_on.length) return t.waiting_on.length === 1 ? 'Waiting on 1 task' : `Waiting on ${t.waiting_on.length} tasks`;
      return 'Waiting for the mission to be approved';
    case 'working':
      return t.claim ? `${names[t.claim.kwant_id] || t.owner_name || 'A bot'} started ${ago(t.claim.claimed_at, now)}` : 'Starting';
    case 'needs_you':
      return t.needs_you[0]?.text ?? 'A bot is waiting for you';
    case 'review':
      if (approvalHolds(t.checks_summary)) return waitingApprovalText(t.checks_summary.required_needs_approval);
      if (t.checks_summary.status === 'not_checked') {
        return t.reviewer?.name ? `Waiting for ${t.reviewer.name} to review` : 'Bot says done. Not checked yet';
      }
      // While a required check runs the card reads as checking; the grey note covers optional ones.
      if (t.checks_summary.status === 'pending') return 'Bot says done. Checking';
      return optionalWaitingText(optionalWaitingCount(t.checks_summary)) ?? 'Bot says done. Checking';
    case 'verified':
      return t.finished_at ? `Verified ${ago(t.finished_at, now)}` : 'Verified';
    case 'ready':
      return t.held_at ? 'On hold after restore. Press Run to start it' : null;
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// "May be invalid" flags and forks (Phase B1, docs/mission-control.md §11)

/** An open flag as the board shows it; `cause` is null when only the needs_you item arrived. */
export interface OpenFlag {
  id: string;
  cause: FlagCause | null;
  causeTaskId: string | null;
  text: string;
  createdAt: string | null;
}

const quote = (s: string | null | undefined) => (s && s.trim() ? `“${s.trim()}”` : 'a task it builds on');

/** One plain sentence for why a task may no longer hold. */
export function flagText(f: Pick<TaskFlag, 'cause' | 'cause_title' | 'detail'>): string {
  const d = f.detail ?? {};
  const path = str(d.path);
  switch (f.cause) {
    case 'evidence_changed':
      if (path && d.new_sha256 === null) return `Its file ${path} is gone since it was verified`;
      return path ? `Its file ${path} changed since it was verified` : 'A file it was verified with changed';
    case 'upstream_evidence_changed':
      return `A file from ${quote(f.cause_title)}, which this builds on, changed`;
    case 'upstream_forked':
      return `${quote(f.cause_title)}, which this builds on, is being redone`;
    default:
      return 'Something it depends on changed';
  }
}

/**
 * The task's open flags: from `flags` when the server sends them, plus any `may_be_invalid` needs_you
 * item without one (so a flag is never hidden). Older servers send neither: no flags.
 */
export function openFlags(t: Pick<Task, 'flags' | 'needs_you'>): OpenFlag[] {
  const out: OpenFlag[] = (t.flags ?? [])
    .filter((f) => !f.resolved_at)
    .map((f) => ({ id: f.id, cause: f.cause, causeTaskId: f.cause_task_id, text: flagText(f), createdAt: f.created_at }));
  for (const n of t.needs_you ?? []) {
    if (n.kind === 'may_be_invalid' && !out.some((f) => f.id === n.id)) {
      out.push({ id: n.id, cause: null, causeTaskId: null, text: n.text.replace(/^Check this still holds:\s*/i, '') || 'Something it depends on changed', createdAt: null });
    }
  }
  return out;
}

export type ResolveChoice = FlagResolution;

export interface ResolveOption {
  choice: ResolveChoice;
  label: string;
  /** What it does, in one line, for this task's state. */
  hint: string;
  /** Why it's off (it would get a 409), or null when it can be pressed. */
  disabled: string | null;
}

/** The three ways the person can settle a flag, worded and enabled for the task's state (§11 table). */
export function resolveOptions(state: TaskState): ResolveOption[] {
  const running = state === 'working' || state === 'needs_you';
  const redoHint: Partial<Record<TaskState, string>> = {
    verified: 'Makes a copy of this task that starts running on its own. This one stays as it is.',
    review: 'Sends it back to Ready for another try, with its tries reset.',
    failed: 'Sends it back to Ready for another try, with its tries reset.',
    blocked: 'Moves it back to Waiting so it can run again.',
    backlog: 'It hasn’t run yet, so it will run again anyway. This just closes the flag.',
    ready: 'It hasn’t run yet, so it will run again anyway. This just closes the flag.',
  };
  return [
    { choice: 'still_valid', label: 'Still valid', hint: 'You checked: it still holds. Nothing else changes.', disabled: null },
    {
      choice: 'redo',
      label: 'Redo',
      hint: redoHint[state] ?? '',
      disabled: running ? 'A bot is working on it. Decide once it stops.' : state === 'cancelled' ? 'It’s cancelled, so there’s nothing to redo.' : null,
    },
    {
      choice: 'cancelled',
      label: 'Cancel',
      hint: state === 'cancelled' ? 'It’s already cancelled. This just closes the flag.' : 'Cancels the task.',
      disabled: state === 'verified' ? 'A verified task can’t be cancelled. Choose Still valid or Redo.' : null,
    },
  ];
}

export const RESOLVE_DONE: Record<ResolveChoice, string> = {
  still_valid: 'Marked still valid.',
  redo: 'Sent back for another try.',
  cancelled: 'Cancelled.',
};

/** What happened after a resolve, in one sentence. A fork says plainly that it runs on its own. */
export function resolvedText(choice: ResolveChoice, r: Pick<ResolveFlagResult, 'fork' | 'task'>): string {
  if (choice === 'redo' && r.fork) return `Made a copy to redo it: “${r.fork.title}”. It starts running on its own; this task stays as it was.`;
  if (choice === 'redo' && (r.task.state === 'backlog' || r.task.state === 'ready')) return 'Flag closed. It will run again.';
  if (choice === 'cancelled' && r.task.state === 'cancelled') return 'Cancelled.';
  return RESOLVE_DONE[choice];
}

/** A plain message for a refused resolve or fork; 409 says what changed instead of the raw reason. */
export function flagErrorText(e: unknown): string {
  if (e instanceof MissionError && e.status === 409) {
    const msg = e.message.toLowerCase();
    if (msg.includes('already resolved')) return 'This flag was already settled, maybe in another tab. The board now shows the latest.';
    if (msg.includes('running')) return 'A bot is working on it now. Decide once it stops.';
    if (msg.includes('verified task can')) return 'A verified task can’t be cancelled. Choose Still valid or Redo.';
    if (msg.includes('mission is cancelled')) return 'Its mission is cancelled, so it can’t be redone.';
    if (msg.includes('cancelled')) return 'The task is cancelled, so there’s nothing to redo.';
    return 'The task changed while you were deciding. The board now shows the latest; try again.';
  }
  return e instanceof Error ? e.message : 'Something went wrong.';
}

/** A fresh Idempotency-Key for one press, so a repeat of the same press can't make a second copy. */
export function newIdempotencyKey(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  if (c?.randomUUID) return c.randomUUID();
  return `k-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

// ---------------------------------------------------------------------------
// AutoFix and restore holds (Phase B3, docs/mission-control.md §12–§13)

export const FAILURE_LABELS: Record<FailureClass, { label: string; tone: Tone }> = {
  transient: { label: 'A temporary error', tone: 'amber' },
  stalled: { label: 'Stopped before finishing', tone: 'amber' },
  check_failed: { label: 'A check failed', tone: 'rose' },
  bad_spec: { label: 'A check can’t run as written', tone: 'amber' },
  auth: { label: 'Signed out, or the key was rejected', tone: 'rose' },
  quota: { label: 'Usage limit reached', tone: 'amber' },
  budget: { label: 'Reached its token limit', tone: 'rose' },
  denied: { label: 'An action was denied', tone: 'rose' },
  needs_input: { label: 'Waiting for you', tone: 'amber' },
  unknown: { label: 'An unexpected error', tone: 'rose' },
};
export const failureClassLabel = (c: string): string => FAILURE_LABELS[c as FailureClass]?.label ?? humanize(c);
export const failureTone = (c: string): Tone => FAILURE_LABELS[c as FailureClass]?.tone ?? 'rose';

const STRATEGY_LABELS: Record<AutofixStrategy, string> = {
  wait_and_retry: 'Wait, then try again',
  retry_from_checkpoint: 'Carry on from where it stopped',
  retry_with_diagnostics: 'Try again with the check’s output',
  retry_same: 'Try again',
  propose_change: 'Propose a change for you to approve',
  none: 'No retry',
};
export const strategyLabel = (s: string): string => STRATEGY_LABELS[s as AutofixStrategy] ?? (s ? humanize(s) : 'Try again');

/** Why AutoFix didn't retry, in words: the server's reason starts with a code. */
export function skipText(reason: string): string {
  const r = reason.trim();
  if (r.startsWith('no_attempts_left')) return 'no tries left';
  if (r.startsWith('over_recovery_budget')) return 'another try would go over its retry budget';
  if (r.startsWith('not_retryable')) return 'this kind of failure isn’t retried';
  return r || 'it wasn’t retried';
}

/** One line for an AutoFix attempt: what it does, and when. */
export function autofixStatusText(a: Pick<AutofixAttempt, 'status' | 'strategy' | 'not_before' | 'reason'>, now: number = Date.now()): string {
  switch (a.status) {
    case 'suggested':
      return a.strategy === 'propose_change' ? 'AutoFix proposed a change. It waits for your OK.' : `AutoFix suggests: ${strategyLabel(a.strategy).toLowerCase()}. It waits for your OK.`;
    case 'scheduled': {
      const when = a.not_before ? untilText(a.not_before, now) : 'now';
      return `AutoFix will ${strategyLabel(a.strategy).toLowerCase()} ${when === 'now' ? 'shortly' : when}.`;
    }
    case 'running':
      return 'AutoFix is retrying it now.';
    case 'succeeded':
      return 'AutoFix’s retry worked.';
    case 'failed':
      return 'AutoFix’s retry failed too.';
    default:
      return `AutoFix didn’t retry: ${skipText(a.reason)}.`;
  }
}

/**
 * The "To fix" text without the failure summary an older server appends after it ("…then retry: <summary>",
 * often cut short with "…"): the panel shows the summary once, on the failure line above. A leading
 * `heading` (the failure's badge, e.g. "Usage limit reached: wait…") is dropped too.
 */
export function fixTextShown(fix: string, summary?: string | null, heading?: string | null): string {
  const s = (summary ?? '').trim();
  const at = s ? fix.indexOf(s.slice(0, Math.min(40, s.length))) : -1;
  const head = at > 0 ? fix.slice(0, at).replace(/[\s:;,.\u2013\u2014-]+$/, '') : '';
  return dropHeading(head ? `${head}.` : fix, heading);
}

/** `text` without a leading "<heading>:" the failure's badge already shows; kept whole when nothing follows. */
function dropHeading(text: string, heading?: string | null): string {
  const h = (heading ?? '').trim();
  if (!h || text.slice(0, h.length).toLowerCase() !== h.toLowerCase()) return text;
  const rest = text.slice(h.length).replace(/^[\s:;,.\u2013\u2014-]+/, '');
  return rest ? rest.charAt(0).toUpperCase() + rest.slice(1) : text;
}

/**
 * The AutoFix line under a failure the person must fix, else null to leave it out. "To fix" already says
 * what to do, so this only adds that AutoFix won't retry, in one short line, and says nothing when it
 * repeats "To fix" (a used-up retry budget). Works with the older server's reason, which is the fix text.
 */
export function skippedWithFixText(a: Pick<AutofixAttempt, 'status' | 'reason'>): string | null | undefined {
  if (a.status !== 'skipped') return undefined;
  const r = a.reason.trim();
  if (r.startsWith('over_recovery_budget')) return null;
  if (r.startsWith('no_attempts_left')) return 'AutoFix didn’t retry: no tries left.';
  return 'AutoFix doesn’t retry this kind of failure.';
}

/** The card line for a failed task AutoFix is handling, else null. */
export function autofixCardText(t: Pick<Task, 'autofix'>, now: number = Date.now()): string | null {
  const a = t.autofix;
  if (!a) return null;
  if (a.status === 'running') return 'AutoFix is retrying';
  if (a.status === 'scheduled') {
    const when = a.not_before ? untilText(a.not_before, now) : 'now';
    return when === 'now' ? 'AutoFix retries shortly' : `AutoFix retries ${when}`;
  }
  if (a.status === 'suggested' && a.strategy !== 'propose_change') return 'AutoFix suggests a retry';
  return null;
}

/** "3.2k of 12.5k tokens for retries"; null on a server without AutoFix. */
export function recoveryText(t: Pick<Task, 'recovery_spent_tokens' | 'recovery_budget_tokens'>): string | null {
  if (typeof t.recovery_spent_tokens !== 'number') return null;
  const spent = formatTokens(t.recovery_spent_tokens);
  return typeof t.recovery_budget_tokens === 'number' && t.recovery_budget_tokens > 0
    ? `${spent} of ${formatTokens(t.recovery_budget_tokens)} tokens`
    : `${spent} tokens`;
}

/** A plain message for a refused AutoFix retry. */
export function autofixErrorText(e: unknown): string {
  if (e instanceof MissionError && e.status === 409) {
    if (e.code === 'no_attempts_left') return 'No tries left. You can still try it again yourself.';
    if (e.code === 'over_recovery_budget') return 'Another retry would go over this task’s retry budget. You can still try it again yourself.';
    if (e.code === 'not_retryable') return 'AutoFix doesn’t retry this kind of failure, such as a sign-in problem or a usage limit. See what to fix above, then try it again yourself.';
    return 'The retry changed while you were deciding. The task now shows the latest.';
  }
  return e instanceof Error ? e.message : 'Something went wrong.';
}

export const AUTOFIX_MODES: { value: AutofixMode; label: string; hint: string }[] = [
  { value: 'off', label: 'Off', hint: 'Failures are recorded. Nothing is retried on its own.' },
  { value: 'suggest', label: 'Suggest', hint: 'AutoFix plans a retry and waits for your OK in Needs you.' },
  { value: 'on', label: 'On', hint: 'AutoFix retries within each task’s tries and retry budget.' },
];

export interface AutofixSettings {
  autofix: AutofixMode;
  recovery_budget_pct: number;
}

/** `autofix` and `recovery_budget_pct` from GET /api/settings; null on a server without AutoFix. */
export function parseAutofixSettings(data: unknown): AutofixSettings | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  if (d.autofix !== 'off' && d.autofix !== 'suggest' && d.autofix !== 'on') return null;
  const pct = typeof d.recovery_budget_pct === 'number' ? Math.max(0, Math.min(100, Math.round(d.recovery_budget_pct))) : 25;
  return { autofix: d.autofix, recovery_budget_pct: pct };
}

export const WORKFLOW_TARGET_LABELS: Record<WorkflowTarget, string> = {
  check_spec: 'A check',
  task_acceptance: 'What done looks like',
  max_attempts: 'Max tries',
  routine_instructions: 'The routine’s instructions',
  routine_schedule: 'The routine’s schedule',
};
export const workflowTargetLabel = (t: string): string => WORKFLOW_TARGET_LABELS[t as WorkflowTarget] ?? humanize(t);

export interface ChangeRow {
  key: string;
  label: string;
  before: string;
  after: string;
  changed: boolean;
}

const FIELD_LABELS: Record<string, string> = {
  kind: 'Kind',
  spec: 'Check',
  required: 'Required',
  instructions: 'Instructions',
  schedule: 'Schedule',
  acceptance: 'What done looks like',
  max_attempts: 'Max tries',
};

const showValue = (v: unknown): string => {
  if (v === undefined) return '—';
  if (v === null) return 'none';
  if (typeof v === 'boolean') return v ? 'Yes' : 'No';
  if (typeof v === 'string') return v;
  if (typeof v === 'number') return String(v);
  return JSON.stringify(v, null, 2);
};

/**
 * A proposed change as rows of before and after, changed rows first. Ids (`check_id`) aren't shown;
 * a bare value (a number, some text) is one row.
 */
export function changeRows(before: unknown, after: unknown): ChangeRow[] {
  const obj = (v: unknown) => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
  const b = obj(before);
  const a = obj(after);
  if (!b || !a) {
    const x = showValue(before);
    const y = showValue(after);
    return [{ key: 'value', label: 'Value', before: x, after: y, changed: x !== y }];
  }
  const keys = [...new Set([...Object.keys(b), ...Object.keys(a)])].filter((k) => !k.endsWith('_id'));
  const rows = keys.map((k) => {
    const x = showValue(b[k]);
    const y = showValue(a[k]);
    return { key: k, label: FIELD_LABELS[k] ?? humanize(k), before: x, after: y, changed: x !== y };
  });
  return [...rows.filter((r) => r.changed), ...rows.filter((r) => !r.changed)];
}

export const CHECKPOINT_LABELS: Record<CheckpointKind, string> = {
  before_run: 'Before a run',
  after_run: 'After a run',
  manual: 'Saved by you',
  before_restore: 'Before a restore',
};
export const checkpointLabel = (k: string): string => CHECKPOINT_LABELS[k as CheckpointKind] ?? humanize(k);

/** Who changed a conflicting file: a task only on proof (D11), else "outside this task". */
export function conflictText(c: RestorePreview['conflicts'][number], titleOf: (id: string) => string | undefined = () => undefined): string {
  if (!c.changed_by_task) return 'Changed outside this task';
  const t = titleOf(c.changed_by_task);
  return t ? `Changed by “${t}”` : 'Changed by another task';
}

export const RESTORE_ACTION_LABELS: Record<RestoreAction, string> = { overwrite: 'Put back', create: 'Bring back', delete: 'Remove' };

/** A plain message for a refused restore or Run. */
export function restoreErrorText(e: unknown): string {
  if (e instanceof MissionError) {
    switch (e.code) {
      case 'stale_preview':
        return 'Files changed since this preview. Look at the new preview, then restore.';
      case 'running':
        return 'A bot is working on it now. Restore once it stops.';
      case 'fork_instead':
        return 'A verified task isn’t restored. Use “Redo as a copy” instead.';
      case 'not_restorable':
        return 'This checkpoint can’t be restored: its snapshot failed or was cleared out.';
      case 'unknown_force_path':
        return 'A file you picked isn’t in the preview any more. Look at the new preview.';
      case 'checkpoint_failed':
        return 'The safety checkpoint couldn’t be taken, so nothing was restored.';
      case 'not_held':
        return 'It isn’t on hold any more, maybe Run was pressed in another tab.';
      default:
        break;
    }
  }
  return e instanceof Error ? e.message : 'Something went wrong.';
}

/** What a restore did, in one sentence. */
export function restoredText(r: Pick<RestoreResult, 'restored' | 'deleted' | 'failed' | 'task'>, rerun: boolean): string {
  const n = (k: number, one: string, many: string) => (k === 1 ? one : many.replace('#', String(k)));
  const parts = [r.restored.length ? n(r.restored.length, '1 file put back', '# files put back') : null, r.deleted.length ? n(r.deleted.length, '1 file removed', '# files removed') : null].filter(Boolean);
  const what = parts.length ? `Restored: ${parts.join(', ')}.` : 'Restored. No files needed changing.';
  const failed = r.failed.length ? ` ${n(r.failed.length, '1 file couldn’t be written', '# files couldn’t be written')}.` : '';
  const next = r.task.held_at ? ' It’s on hold until you press Run.' : rerun && (r.task.state === 'ready' || r.task.state === 'backlog') ? ' It will run again.' : '';
  return `${what}${failed}${next}`;
}

// ---------------------------------------------------------------------------
// Work graph view

export interface GraphRow {
  node: GraphNode;
  /** Tasks this one builds on (edge `from`), in graph order. */
  after: GraphNode[];
  /** How many tasks were flagged because of this one (open flags whose cause is this task, itself excluded). */
  flaggedBecause: number;
  /** "2 more tasks further down weren't flagged" when this task caused flags, else null. */
  notFlaggedText: string | null;
}

export interface GraphLevel {
  /** 1 for tasks that build on nothing, 2 for what follows them, and so on. */
  step: number;
  rows: GraphRow[];
}

/**
 * Lays the graph out as steps: each task one step after the latest task it builds on. A list reads at
 * 390 px where a drawn graph wouldn't. A loop (it shouldn't happen) lands in a last step, never lost.
 */
export function graphLevels(g: Pick<WorkGraph, 'nodes' | 'edges'>): GraphLevel[] {
  const byId = new Map(g.nodes.map((n) => [n.id, n]));
  const parents = new Map<string, string[]>(g.nodes.map((n) => [n.id, []]));
  for (const e of g.edges) if (byId.has(e.from) && byId.has(e.to)) parents.get(e.to)!.push(e.from);
  const level = new Map<string, number>();
  const visiting = new Set<string>();
  const depth = (id: string): number => {
    const known = level.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return Number.POSITIVE_INFINITY;
    visiting.add(id);
    const ps = parents.get(id) ?? [];
    const d = ps.length ? Math.max(...ps.map(depth)) + 1 : 1;
    visiting.delete(id);
    level.set(id, d);
    return d;
  };
  const flaggedBy = new Map<string, number>();
  for (const n of g.nodes) for (const f of n.flags ?? []) if (!f.resolved_at && f.cause_task_id !== n.id) flaggedBy.set(f.cause_task_id, (flaggedBy.get(f.cause_task_id) ?? 0) + 1);
  const causes = new Set<string>();
  for (const n of g.nodes) for (const f of n.flags ?? []) if (!f.resolved_at) causes.add(f.cause_task_id);
  const steps = new Map<number, GraphRow[]>();
  const finite = g.nodes.map((n) => depth(n.id)).filter(Number.isFinite);
  const last = (finite.length ? Math.max(...finite) : 0) + 1;
  for (const n of g.nodes) {
    const d = Number.isFinite(depth(n.id)) ? depth(n.id) : last;
    const flagged = flaggedBy.get(n.id) ?? 0;
    const rest = Math.max(0, (n.downstream ?? 0) - flagged);
    const row: GraphRow = {
      node: n,
      after: (parents.get(n.id) ?? []).map((id) => byId.get(id)!),
      flaggedBecause: flagged,
      notFlaggedText: causes.has(n.id) && rest > 0 ? (rest === 1 ? '1 more task further down wasn’t flagged' : `${rest} more tasks further down weren’t flagged`) : null,
    };
    if (!steps.has(d)) steps.set(d, []);
    steps.get(d)!.push(row);
  }
  return [...steps.entries()]
    .sort(([a], [b]) => a - b)
    .map(([, rows], i) => ({ step: i + 1, rows: rows.sort((a, b) => a.node.title.localeCompare(b.node.title)) }));
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
  /** Whether a check's row is the quiet optional one: grey "Not run" and "Run this optional check". It
   * can't hold the task in any state, so it never gets the amber hand or "Allow this check". */
  quietOptional: (c: Pick<Check, 'status' | 'required'>) => boolean;
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
    quietOptional: isQuietOptional,
  };
}

/** An optional check waiting for the person: it can run, but it never holds the task. */
export const isQuietOptional = (c: Pick<Check, 'status' | 'required'>): boolean => c.status === 'needs_approval' && !c.required;

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
      if (e.to_state === 'cancelled' && note === NOTICE_CLOSED_NOTE) return `${Who} closed it: ${note}`;
      return `${Who} moved it to ${stateLabel(e.to_state ?? '').label}${note ? `: ${note}` : ''}`;
    case 'claimed':
      return `${Who} took it on`;
    case 'released':
      if (e.actor === 'person' && !note) return 'You let it run again';
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
      return 'It reached its token limit';
    case 'edited':
      return `${Who} edited it`;
    case 'flagged':
      return 'Flagged: it may no longer hold';
    case 'flag_resolved': {
      const r = str(d.resolution);
      return `${Who} settled a flag: ${r === 'still_valid' ? 'still valid' : r === 'redo' ? 'redo' : r === 'cancelled' ? 'cancel' : 'decided'}`;
    }
    case 'forked':
      return `${Who} made a copy to redo it${note ? `: ${note}` : ''}`;
    case 'held':
      return 'On hold after the restore, until you press Run';
    case 'restored': {
      const n = (k: unknown, one: string, many: string) => (typeof k === 'number' && k > 0 ? (k === 1 ? one : many.replace('#', String(k))) : null);
      const parts = [n(d.restored, '1 file put back', '# files put back'), n(d.deleted, '1 removed', '# removed')].filter(Boolean);
      return `${Who} restored it from a checkpoint${parts.length ? `: ${parts.join(', ')}` : ''}`;
    }
    case 'failure':
      return `It failed: ${failureClassLabel(str(d.class) ?? 'unknown').toLowerCase()}${str(d.summary) ? `. ${str(d.summary)}` : ''}`;
    case 'autofix':
      return `AutoFix sent it back to Ready: ${strategyLabel(str(d.strategy) ?? '').toLowerCase()}`;
    case 'autofix_scheduled':
      return str(d.status) === 'suggested' ? 'AutoFix suggested a retry. It waits for your OK' : `AutoFix planned a retry: ${strategyLabel(str(d.strategy) ?? '').toLowerCase()}`;
    case 'autofix_started':
      return 'AutoFix started a retry';
    case 'autofix_finished': {
      const s = str(d.status);
      if (s === 'succeeded') return 'AutoFix’s retry worked';
      if (s === 'skipped') return `AutoFix didn’t retry: ${skipText(str(d.reason) ?? '')}`;
      return 'AutoFix’s retry failed';
    }
    case 'workflow_change_proposed':
      return `${Who} proposed a change for you to approve`;
    case 'workflow_change_decided':
      return str(d.status) === 'stale' ? 'A proposed change went out of date, so nothing changed' : `${Who} ${str(d.status) === 'rejected' ? 'rejected' : 'approved'} a proposed change`;
    case 'workflow_change_applied':
      return 'The approved change was applied';
    case 'audit_started':
      return `An audit started: ${actorName(str(d.challenger) ?? '', null, names)} challenges the work`;
    case 'audit_challenge':
      return `${Who} raised ${typeof d.risks === 'number' ? (d.risks === 1 ? '1 risk' : `${d.risks} risks`) : 'risks'} in the audit`;
    case 'audit_rebuttal':
      return `${Who} answered the audit`;
    case 'audit_done':
      return str(d.status) === 'done' ? 'The audit finished. It doesn’t change whether the task passed' : `The audit ended: ${str(d.status) ?? 'stopped'}`;
    case 'audit_skipped':
      return `No audit: ${str(d.reason) === 'daily_cap' ? 'today’s limit was reached' : str(d.reason) === 'no_challenger' ? 'no other bot could challenge it' : str(d.reason) === 'task_moved' ? 'the task moved on' : 'skipped'}`;
    case 'risk_resolved':
      return `${Who} ${str(d.resolution) === 'dismissed' ? 'dismissed' : 'resolved'} a risk`;
    case 'class_changed':
      return `${Who} changed its kind of work${str(d.to) ? ` to ${str(d.to)}` : ''}`;
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

export type MissionEvent =
  | { type: 'task_updated'; data: { task: Task } }
  | { type: 'mission_updated'; data: { mission: Mission } }
  /** Flight School C4: a bot asked for a tier, or a proposal was granted, declined or went stale. */
  | { type: 'capability_changed'; data: { capability: CapabilityChangedEvent } }
  | { type: 'fact_changed'; data: { fact: FactChangedEvent } }
  | { type: 'fact_dispute'; data: { dispute: FactDisputeEvent } };

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
    /** The server's `code` on a refusal (e.g. `stale_preview`, `no_attempts_left`), when it sends one. */
    public code: string | null = null,
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
  // A 409 without a `code` may name the state it found instead (a Flight School proposal, or a dispute or fact already decided).
  const code = typeof a.code === 'string' && a.code ? a.code : status === 409 && typeof a.status === 'string' && a.status ? a.status : null;
  if (status === 403) return new MissionError('Only you can change tasks on the board.', 403);
  if (status === 404 && !err) return new MissionError('This server doesn’t have the Mission board yet. Update the app and try again.', 404);
  return new MissionError(err ?? `Something went wrong (HTTP ${status}).`, status, code);
}

async function call<T>(url: string, method = 'GET', body?: unknown, extra?: Record<string, string>, keepalive = false): Promise<T> {
  let res: Response;
  const headers = { ...(body === undefined ? {} : { 'Content-Type': 'application/json' }), ...extra };
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: Object.keys(headers).length ? headers : undefined,
      body: body === undefined ? undefined : JSON.stringify(body),
      ...(keepalive ? { keepalive: true } : {}),
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
  /** Flight School: the person sets the task's kind of work (a slug). */
  setTaskClass: (id: string, task_class: string) => call<Task>(`/api/tasks/${enc(id)}`, 'PATCH', { task_class }),
  /** A mission's token limit; 0 removes it. */
  setBudget: (id: string, budget_tokens: number) => call<Mission>(`/api/missions/${enc(id)}`, 'PATCH', { budget_tokens }),
  /** A task's token limit; 0 removes it. */
  setTaskBudget: (id: string, budget_tokens: number) => call<Task>(`/api/tasks/${enc(id)}`, 'PATCH', { budget_tokens }),
  /** A task blocked by its limit: set the new one (0 = none), then unblock it, so it goes back to Ready. */
  continueWithLimit: async (id: string, budget_tokens: number): Promise<Task> => {
    await call<Task>(`/api/tasks/${enc(id)}`, 'PATCH', { budget_tokens });
    return call<Task>(`/api/tasks/${enc(id)}/unblock`, 'POST', {});
  },
  tasks: () => call<{ tasks: Task[] }>('/api/tasks?limit=500').then((r) => r.tasks),
  task: (id: string) => call<TaskDetail>(`/api/tasks/${enc(id)}`),
  approveCheck: (id: string) => call<Check>(`/api/checks/${enc(id)}/approve`, 'POST', {}),
  graph: (missionId: string) => call<WorkGraph>(`/api/missions/${enc(missionId)}/graph`),
  scan: (missionId: string) => call<GraphScanResult>(`/api/missions/${enc(missionId)}/graph/scan`, 'POST', {}),
  /** Settle a flag. The key rides along on `redo` too, since a verified task's redo makes a copy. */
  resolveFlag: (taskId: string, flagId: string, resolution: FlagResolution, key: string, note?: string) =>
    call<ResolveFlagResult>(`/api/tasks/${enc(taskId)}/flags/${enc(flagId)}/resolve`, 'POST', note ? { resolution, note } : { resolution }, { 'Idempotency-Key': key }),
  /** Copy a task to redo it; the same key returns the same copy. */
  fork: (taskId: string, reason: string, key: string) => call<Task>(`/api/tasks/${enc(taskId)}/fork`, 'POST', { reason }, { 'Idempotency-Key': key }),
  /** Let a task a restore held run again (person only). */
  /** Take a held task off hold. The same key replays the first 200, so a double press is a success. */
  runHeld: (id: string, key: string) => call<{ task: Task }>(`/api/tasks/${enc(id)}/run`, 'POST', {}, { 'Idempotency-Key': key }).then((r) => r.task),
  checkpoints: (id: string) => call<Checkpoint[]>(`/api/tasks/${enc(id)}/checkpoints`),
  restorePreview: (id: string, cid: string) => call<RestorePreview>(`/api/tasks/${enc(id)}/checkpoints/${enc(cid)}/restore-preview`),
  restore: (id: string, cid: string, body: { preview_sha256: string; force_paths?: string[]; rerun?: boolean }, key: string) =>
    call<RestoreResult>(`/api/tasks/${enc(id)}/checkpoints/${enc(cid)}/restore`, 'POST', body, { 'Idempotency-Key': key }),
  failures: (id: string) => call<TaskFailure[]>(`/api/tasks/${enc(id)}/failures`),
  /** Run the suggested retry now (or plan one). The same key returns the same attempt. */
  runAutofix: (id: string, key: string, failureId?: string) =>
    call<AutofixAttempt>(`/api/tasks/${enc(id)}/autofix`, 'POST', failureId ? { failure_id: failureId } : {}, { 'Idempotency-Key': key }),
  cancelAutofix: (id: string, aid: string) => call<AutofixAttempt>(`/api/tasks/${enc(id)}/autofix/${enc(aid)}/cancel`, 'POST', {}),
  workflowChanges: (status: WorkflowChangeStatus = 'proposed', taskId?: string) =>
    call<WorkflowChange[]>(`/api/workflow-changes?status=${status}${taskId ? `&task_id=${enc(taskId)}` : ''}`),
  /** Swarm Auditor: the task's audits and their risks. */
  risks: (id: string) => call<TaskRisks>(`/api/tasks/${enc(id)}/risks`),
  /** The space's unresolved risks, for the board's badges. */
  spaceRisks: () => call<{ risks: Risk[] }>('/api/risks').then((r) => r.risks),
  /** Close a risk (person only). The task's state doesn't change. */
  resolveRisk: (id: string, riskId: string, resolution: RiskResolution, note?: string) =>
    call<{ risk: Risk; unresolved: boolean }>(`/api/tasks/${enc(id)}/risks/${enc(riskId)}/resolve`, 'POST', note ? { resolution, note } : { resolution }),
  decideChange: (id: string, approve: boolean) => call<WorkflowChange>(`/api/workflow-changes/${enc(id)}/${approve ? 'approve' : 'reject'}`, 'POST', {}),
  /**
   * One person action on a task. `run_checks` answers `{checks, task}`, the others the Task.
   * `keepalive` lets the request finish after the page closes.
   */
  act: async (id: string, action: TaskAction, reason?: string, keepalive = false): Promise<Task> => {
    const base = `/api/tasks/${enc(id)}`;
    switch (action) {
      case 'verify':
        return call<Task>(`${base}/verify`, 'POST', reason ? { note: reason } : {}, undefined, keepalive);
      case 'reject':
        return call<Task>(`${base}/reject`, 'POST', { reason: reason ?? '' }, undefined, keepalive);
      case 'block':
        return call<Task>(`${base}/block`, 'POST', { reason: reason ?? '' }, undefined, keepalive);
      case 'run_checks':
        return call<{ task: Task }>(`${base}/checks/run`, 'POST', {}, undefined, keepalive).then((r) => r.task);
      default:
        return call<Task>(`${base}/${action}`, 'POST', {}, undefined, keepalive);
    }
  },
};

/** Flight School tiers (Phase C3). Reads are open; both writes are person-only on the server. */
export const capabilitiesApi = {
  get: (kwantId: string) => call<Capabilities>(`/api/capabilities?kwant_id=${enc(kwantId)}`),
  /** `task_class` '*' sets the bot's tier for all work without its own. */
  set: (kwantId: string, task_class: string, tier: Tier) => call<CapabilityChange>('/api/capabilities', 'POST', { kwant_id: kwantId, task_class, tier }),
  revoke: (kwantId: string, task_class: string, to_tier: Tier) =>
    call<CapabilityChange>(`/api/capabilities/${enc(kwantId)}/revoke`, 'POST', { task_class, to_tier }),
  /** C4: one bot's live scorecard for a kind of work (the server defaults to general). */
  scorecard: (kwantId: string, taskClass: string) => call<Scorecard>(`/api/capabilities/${enc(kwantId)}/scorecard?class=${enc(taskClass)}`),
  /** C4: the space's proposals, newest first; stale ones are marked on read. */
  proposals: (status?: ProposalStatus, kwantId?: string) => {
    const q = [status ? `status=${status}` : '', kwantId ? `kwant_id=${enc(kwantId)}` : ''].filter(Boolean).join('&');
    return call<{ proposals: TierProposal[] }>(`/api/capabilities/proposals${q ? `?${q}` : ''}`).then((r) => r.proposals);
  },
  /** Person only. The same key replays the first answer; a 409 carries the state found as `code`. */
  grant: (id: string, key: string) => call<ProposalDecision>(`/api/capabilities/proposals/${enc(id)}/grant`, 'POST', {}, { 'Idempotency-Key': key }),
  decline: (id: string, key: string, note?: string) =>
    call<ProposalDecision>(`/api/capabilities/proposals/${enc(id)}/decline`, 'POST', note ? { note } : {}, { 'Idempotency-Key': key }),
};

/** The Truth Ledger (C2, person only; writes 409 unless the ledger is `on`). */
export const factsApi = {
  /** `setting` tells whether to show anything: only `on` does. */
  list: (status?: FactStatus) =>
    call<{ setting?: string; facts?: Fact[] }>(`/api/facts${status ? `?status=${enc(status)}` : ''}`).then((r) => ({
      setting: ledgerSetting(r?.setting),
      facts: Array.isArray(r?.facts) ? r.facts : [],
    })),
  disputes: (status: DisputeStatus | 'all' = 'open') =>
    call<{ setting?: string; disputes?: Dispute[] }>(`/api/facts/disputes?status=${enc(status)}`).then((r) => ({
      setting: ledgerSetting(r?.setting),
      disputes: Array.isArray(r?.disputes) ? r.disputes : [],
    })),
  get: (id: string) => call<FactDetail>(`/api/facts/${enc(id)}`),
  resolve: (id: string, winner: string, key: string) => call<Dispute>(`/api/facts/disputes/${enc(id)}/resolve`, 'POST', { winner }, { 'Idempotency-Key': key }),
  dismiss: (id: string, key: string) => call<Dispute>(`/api/facts/disputes/${enc(id)}/dismiss`, 'POST', {}, { 'Idempotency-Key': key }),
  accept: (id: string, key: string) => call<FactDetail>(`/api/facts/${enc(id)}/accept`, 'POST', {}, { 'Idempotency-Key': key }),
  reject: (id: string, key: string) => call<FactDetail>(`/api/facts/${enc(id)}/reject`, 'POST', {}, { 'Idempotency-Key': key }),
  uncite: (id: string, taskId: string) => call<{ ok: boolean }>(`/api/facts/${enc(id)}/uncite`, 'POST', { task_id: taskId }),
};
export type FactsApi = typeof factsApi;

/** Where a task came from in the app: a channel, a bot's thread, or a bot's direct chat. */
/** Where to answer a Needs-you card: the chat the card is in, else the chat the task came from. */
export function answerChat(t: Pick<Task, 'channel_id' | 'direct_kwant_id' | 'thread_id'>, n: Pick<NeedsYouItem, 'chat'>): ChatPlace | null {
  const c = n.chat;
  if (c && (c.channel_id || c.kwant_id)) return { channelId: c.channel_id, kwantId: c.channel_id ? null : c.kwant_id, threadId: c.thread_id };
  return sourceChat(t);
}

export type ChatPlace = { channelId: string | null; kwantId: string | null; threadId: string | null };

export function sourceChat(t: Pick<Task, 'channel_id' | 'direct_kwant_id' | 'thread_id'>): { channelId: string | null; kwantId: string | null; threadId: string | null } | null {
  if (t.channel_id) return { channelId: t.channel_id, kwantId: null, threadId: t.thread_id };
  if (t.direct_kwant_id) return { channelId: null, kwantId: t.direct_kwant_id, threadId: t.thread_id };
  return null;
}

// ---------------------------------------------------------------------------
// Live board: who is busy right now, lanes by team or bot, repeated runs folded, Verified fading out

/** A running or queued turn, as GET /api/turns lists it. */
export interface BoardTurn {
  turn_id: string;
  kwant_id: string;
  context_id: string;
  thread_id: string | null;
  state: 'queued' | 'running';
  since?: string;
  purpose?: string;
}

export interface BoardTeam {
  id: string;
  name: string;
  lead_kwant_id: string;
  member_kwants: string[];
}

export type GroupBy = 'none' | 'team' | 'bot';
export const GROUP_BY_KEY = 'kwnt.missionBoard.groupBy';
export const NO_TEAM = '__no_team__';
export const NO_OWNER = '__no_owner__';

export function readGroupBy(): GroupBy {
  try {
    const v = localStorage.getItem(GROUP_BY_KEY);
    return v === 'team' || v === 'bot' ? v : 'none';
  } catch {
    return 'none';
  }
}

export function saveGroupBy(g: GroupBy): void {
  try {
    localStorage.setItem(GROUP_BY_KEY, g);
  } catch {
    // Private mode: the choice lasts until the page closes.
  }
}

/** Bot id → its team. A bot in several teams goes to the first one it leads, else the first it's in. */
export function teamOfBots(teams: BoardTeam[]): Record<string, BoardTeam> {
  const out: Record<string, BoardTeam> = {};
  for (const t of teams) if (t.lead_kwant_id) out[t.lead_kwant_id] ??= t;
  for (const t of teams) for (const k of t.member_kwants ?? []) out[k] ??= t;
  return out;
}

/** The bot a card belongs to: its owner, else the bot holding it. */
export const taskBot = (t: Pick<Task, 'owner_kwant_id' | 'claim'>): string | null => t.owner_kwant_id ?? t.claim?.kwant_id ?? null;

/** Sources the person didn't ask for one by one: they pile up, so they can be cleared in bulk. */
const AUTO_SOURCES = new Set<SourceKind>(['routine', 'work_ahead', 'zombie']);
export const isAutoTask = (t: Pick<Task, 'source_kind'>): boolean => AUTO_SOURCES.has(t.source_kind);

/** One card on the board: the newest task, and older runs of the same routine folded under it. */
export interface BoardItem {
  task: Task;
  older: Task[];
}

/**
 * What makes two cards "the same run again": a routine run by the same bot with the same title. A
 * routine card's source is its run, not the routine, so its title ("Routine: Morning overview")
 * stands in. Worked-ahead and loose-end cards never fold: the same title can be different work.
 */
export function repeatKey(t: Task): string | null {
  if (t.source_kind !== 'routine') return null;
  return `routine:${taskBot(t) ?? ''}:${t.title.trim().toLowerCase()}`;
}

/**
 * Folds repeated routine runs into one card, newest first, keeping the input's order otherwise.
 * Only cards in the same column fold together, so callers pass one column's tasks.
 */
export function collapseRepeats(tasks: Task[]): BoardItem[] {
  const out: BoardItem[] = [];
  const at = new Map<string, number>();
  const newest = (t: Task) => Date.parse(t.finished_at ?? t.updated_at) || 0;
  for (const t of tasks) {
    const k = repeatKey(t);
    const i = k === null ? undefined : at.get(k);
    if (i === undefined) {
      if (k !== null) at.set(k, out.length);
      out.push({ task: t, older: [] });
      continue;
    }
    const item = out[i];
    if (newest(t) > newest(item.task)) out[i] = { task: t, older: [item.task, ...item.older] };
    else item.older.push(t);
  }
  for (const item of out) item.older.sort((a, b) => newest(b) - newest(a));
  return out;
}

export const VERIFIED_WINDOW_HOURS = 24;
export const OLD_REVIEW_DAYS = 7;

/** Verified cards from the last day show; older ones wait behind "Show older". Nothing is deleted. */
export function splitVerified(tasks: Task[], now: number = Date.now(), hours = VERIFIED_WINDOW_HOURS): { recent: Task[]; older: Task[] } {
  const cutoff = now - hours * 3_600_000;
  const recent: Task[] = [];
  const older: Task[] = [];
  for (const t of tasks) {
    const at = Date.parse(t.finished_at ?? t.updated_at);
    (Number.isNaN(at) || at >= cutoff ? recent : older).push(t);
  }
  return { recent, older };
}

/** Automatic cards (routine runs, worked-ahead, loose ends) sitting in Review for more than a week. */
export function oldAutoReviews(tasks: Task[], now: number = Date.now(), days = OLD_REVIEW_DAYS): Task[] {
  const cutoff = now - days * 86_400_000;
  return tasks.filter((t) => columnOfTask(t) === 'review' && isAutoTask(t) && Date.parse(t.updated_at) < cutoff);
}

export interface Lane {
  key: string;
  label: string;
  columns: Record<Column, Task[]>;
  total: number;
  needsYou: number;
}

/**
 * Splits the board into rows (by team or bot), each with the five columns. Teams keep the order the
 * person made them in; bots go alphabetically; "No team" / "No owner" come last. Rows without a card
 * are left out.
 */
export function groupIntoLanes(tasks: Task[], by: Exclude<GroupBy, 'none'>, teams: BoardTeam[], names: Record<string, string>): Lane[] {
  const teamOf = teamOfBots(teams);
  const lanes = new Map<string, Lane>();
  const laneOf = (t: Task): { key: string; label: string } => {
    const bot = taskBot(t);
    if (by === 'team') {
      const team = bot ? teamOf[bot] : undefined;
      return team ? { key: team.id, label: team.name } : { key: NO_TEAM, label: 'No team' };
    }
    return bot ? { key: bot, label: names[bot] ?? t.owner_name ?? 'Unknown bot' } : { key: NO_OWNER, label: 'No owner yet' };
  };
  for (const t of sortTasks(tasks)) {
    const c = columnOfTask(t);
    if (!c) continue;
    const { key, label } = laneOf(t);
    let lane = lanes.get(key);
    if (!lane) {
      lane = { key, label, columns: { ready: [], working: [], needs_you: [], review: [], verified: [] }, total: 0, needsYou: 0 };
      lanes.set(key, lane);
    }
    lane.columns[c].push(t);
    lane.total++;
    if (c === 'needs_you') lane.needsYou++;
  }
  const order = by === 'team' ? new Map(teams.map((t, i) => [t.id, i])) : null;
  const last = (k: string) => k === NO_TEAM || k === NO_OWNER;
  return [...lanes.values()].sort((a, b) => {
    if (last(a.key) !== last(b.key)) return last(a.key) ? 1 : -1;
    if (order) return (order.get(a.key) ?? 1e9) - (order.get(b.key) ?? 1e9);
    return a.label.localeCompare(b.label);
  });
}

/** Keeps the cards of the picked teams (none picked = all). `NO_TEAM` picks cards without one. */
export function filterByTeams(tasks: Task[], picked: string[], teams: BoardTeam[]): Task[] {
  if (!picked.length) return tasks;
  const teamOf = teamOfBots(teams);
  const want = new Set(picked);
  return tasks.filter((t) => {
    const bot = taskBot(t);
    return want.has((bot && teamOf[bot]?.id) || NO_TEAM);
  });
}

/** "45s", "12m", "1h 05m", "2d 3h": how long a turn has been running. */
export function elapsedText(since: string | undefined, now: number = Date.now()): string {
  const t = since ? Date.parse(since) : NaN;
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.floor((now - t) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

export interface LiveBot {
  kwantId: string;
  name: string;
  teamId: string;
  teamName: string;
  /** One line: what it is doing. */
  what: string;
  kind: 'task' | 'room' | 'thread' | 'chat' | 'deciding';
  since?: string;
  /** Where clicking it goes. */
  place: ChatPlace;
  /** Its other running turns. */
  more: number;
  /** The card it's working on, when the turn is a task's. */
  taskId: string | null;
}

export interface LiveContext {
  names: Record<string, string>;
  teams: BoardTeam[];
  /** Room id → name. */
  rooms?: Record<string, string>;
  /** Thread id → title. */
  threadTitles?: Record<string, string>;
  tasks?: Task[];
}

/** What one running turn is about, and where it happens. */
export function describeTurn(turn: BoardTurn, ctx: LiveContext): Pick<LiveBot, 'what' | 'kind' | 'place' | 'taskId'> {
  const room = ctx.rooms?.[turn.context_id];
  const place: ChatPlace = room !== undefined
    ? { channelId: turn.context_id, kwantId: null, threadId: turn.thread_id }
    : { channelId: null, kwantId: turn.kwant_id, threadId: turn.thread_id };
  // A task's chat is the turn's: its thread, else its room, else its bot's direct chat.
  const task = (ctx.tasks ?? []).find(
    (t) => t.state === 'working' && taskBot(t) === turn.kwant_id && (turn.thread_id ? t.thread_id === turn.thread_id : !t.thread_id && (t.channel_id ?? t.direct_kwant_id) === turn.context_id),
  );
  if (task) return { what: task.title, kind: 'task', place, taskId: task.id };
  if (turn.purpose === 'decide') return { what: 'Deciding how to answer', kind: 'deciding', place, taskId: null };
  if (room !== undefined) return { what: `In ${room || 'a room'}`, kind: 'room', place, taskId: null };
  if (turn.thread_id) {
    const title = ctx.threadTitles?.[turn.thread_id];
    return { what: title ? title : 'In a side conversation', kind: 'thread', place, taskId: null };
  }
  return { what: 'In your chat', kind: 'chat', place, taskId: null };
}

/**
 * Every bot running something right now, one row per bot (its longest-running turn, "+N more" for
 * the rest), longest first. Queued turns aren't running, so they're left out.
 */
export function liveBots(turns: BoardTurn[], ctx: LiveContext): LiveBot[] {
  const teamOf = teamOfBots(ctx.teams);
  const byBot = new Map<string, BoardTurn[]>();
  for (const t of turns) {
    if (t.state !== 'running' || !t.kwant_id) continue;
    byBot.set(t.kwant_id, [...(byBot.get(t.kwant_id) ?? []), t]);
  }
  const started = (t: BoardTurn) => (t.since ? Date.parse(t.since) || Infinity : Infinity);
  const out: LiveBot[] = [];
  for (const [kwantId, list] of byBot) {
    // Prefer a task's turn (it says the most), then the longest-running.
    const described = list.map((t) => ({ t, d: describeTurn(t, ctx) }));
    described.sort((a, b) => Number(b.d.kind === 'task') - Number(a.d.kind === 'task') || started(a.t) - started(b.t));
    const { t, d } = described[0];
    const team = teamOf[kwantId];
    out.push({
      kwantId,
      name: ctx.names[kwantId] ?? 'A bot',
      teamId: team?.id ?? NO_TEAM,
      teamName: team?.name ?? 'No team',
      ...d,
      since: list.map((x) => x.since).filter(Boolean).sort()[0] ?? t.since,
      more: list.length - 1,
    });
  }
  return out.sort((a, b) => (a.since ?? '').localeCompare(b.since ?? '') || a.name.localeCompare(b.name));
}

/** Live bots by team, in the order the person made their teams; "No team" last. */
export function liveByTeam(bots: LiveBot[], teams: BoardTeam[]): Array<{ teamId: string; teamName: string; bots: LiveBot[] }> {
  const order = new Map(teams.map((t, i) => [t.id, i]));
  const groups = new Map<string, { teamId: string; teamName: string; bots: LiveBot[] }>();
  for (const b of bots) {
    const g = groups.get(b.teamId) ?? { teamId: b.teamId, teamName: b.teamName, bots: [] };
    g.bots.push(b);
    groups.set(b.teamId, g);
  }
  return [...groups.values()].sort((a, b) => (order.get(a.teamId) ?? 1e9) - (order.get(b.teamId) ?? 1e9));
}

/** The Live strip's summary, so it and the Working column never seem to disagree. */
export function liveSummary(bots: LiveBot[], workingCards: number): string {
  if (!bots.length) return workingCards ? `All bots idle · ${workingCards === 1 ? '1 task waits' : `${workingCards} tasks wait`} in Working for its bot’s next turn` : 'All bots idle';
  const onTasks = bots.filter((b) => b.kind === 'task').length;
  const head = bots.length === 1 ? '1 bot busy' : `${bots.length} bots busy`;
  const rest = bots.length - onTasks;
  if (!rest) return `${head}, all on board tasks`;
  return `${head} · ${onTasks} on board tasks, ${rest} in chats, rooms or routines`;
}

/** The Verified column's header count: "5 · 37 all" once older cards are hidden. */
export function verifiedCountText(recent: number, all: number): string {
  return recent === all ? String(all) : `${recent} · ${all} all`;
}

/** The one-tap actions a Review card carries. Dismiss is the server's cancel. */
export type QuickAction = 'verify' | 'reject' | 'cancel' | 'retry';

export function quickActionsFor(t: Pick<Task, 'state'>): QuickAction[] {
  if (t.state === 'review') return ['verify', 'reject', 'cancel'];
  if (t.state === 'failed') return ['retry', 'cancel'];
  return [];
}

export const QUICK_LABELS: Record<QuickAction, string> = { verify: 'Mark verified', reject: 'Send back', cancel: 'Dismiss', retry: 'Try again' };

/** Sending back needs a reason on the server; an empty note sends this one. */
export const DEFAULT_SEND_BACK = 'Take another look: it isn’t done yet.';

/** The undo toast's line for an action on `n` cards. */
export function quickDoneText(a: QuickAction, n: number, title?: string): string {
  const what = n === 1 && title ? `“${title}”` : n === 1 ? '1 card' : `${n} cards`;
  switch (a) {
    case 'verify':
      return `Marked ${what} verified.`;
    case 'reject':
      return `Sent ${what} back to its bot.`;
    case 'retry':
      return `${what} goes back to Ready for another try.`;
    default:
      return `Dismissed ${what}.`;
  }
}

/** How long Undo stays on screen before the action goes to the server. */
export const UNDO_MS = 6000;

/** A card action waiting out its Undo window. `send(leaving)` is told when the page is going away. */
export interface HeldAction {
  id: number;
  tasks: Task[];
  send: (leaving: boolean) => Promise<void>;
  timer: number;
}

/**
 * Holds at most one card action. Taking it out is the only way to send it, so an action goes once:
 * the timer, the next action, Undo, leaving the board and the page closing all race for the same one.
 */
export class HeldActionSlot {
  private held: HeldAction | null = null;
  constructor(private clearTimer: (t: number) => void = (t) => clearTimeout(t)) {}

  get current(): HeldAction | null {
    return this.held;
  }

  hold(a: HeldAction): void {
    this.held = a;
  }

  /** Takes the action out and stops its timer, without sending it. */
  take(): HeldAction | null {
    const a = this.held;
    if (!a) return null;
    this.held = null;
    this.clearTimer(a.timer);
    return a;
  }

  /** Sends the held action now, if there is one. */
  commit(leaving = false): { action: HeldAction; sent: Promise<void> } | null {
    const a = this.take();
    return a ? { action: a, sent: a.send(leaving) } : null;
  }
}

/**
 * Sends a held action when the page is closed, reloaded or put in the background (a phone's app
 * switcher often never fires pagehide). Returns the cleanup.
 */
export function flushOnPageLeave(
  flush: () => void,
  win: Pick<EventTarget, 'addEventListener' | 'removeEventListener'> = window,
  doc: Pick<EventTarget, 'addEventListener' | 'removeEventListener'> & { visibilityState: string } = document,
): () => void {
  const onHide = () => {
    if (doc.visibilityState === 'hidden') flush();
  };
  win.addEventListener('pagehide', flush);
  doc.addEventListener('visibilitychange', onHide);
  return () => {
    win.removeEventListener('pagehide', flush);
    doc.removeEventListener('visibilitychange', onHide);
  };
}
