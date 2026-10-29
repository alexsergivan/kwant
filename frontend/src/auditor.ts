// Swarm Auditor (Phase D2, docs/mission-control.md §18): a second bot challenges a task with
// evidence, the owner answers once, and the risks left unresolved stay listed on the task until the
// person closes them. An audit never marks work passed or failed; only checks do. Pure helpers,
// shared by the app and the node tests.
import type { AwayItem, AwayLink } from './away';

export type AuditStatus = 'queued' | 'challenge' | 'rebuttal' | 'done' | 'skipped' | 'failed' | 'interrupted';
export type RiskStatus = 'open' | 'disputed' | 'resolved' | 'dismissed';
export type RiskResolution = 'resolved' | 'dismissed';

export interface Risk {
  id: string;
  audit_id: string;
  task_id: string;
  seq: number;
  risk: string;
  evidence: string;
  evidence_ids?: string[];
  stance: string | null;
  answer: string | null;
  answer_evidence: string | null;
  status: RiskStatus;
  resolved_by: string | null;
  resolved_at: string | null;
  note: string | null;
  created_at: string;
  unresolved?: boolean;
}

export interface Audit {
  id: string;
  task_id: string;
  attempt: number;
  trigger: string;
  challenger_kwant_id: string;
  owner_kwant_id: string;
  status: AuditStatus;
  reason: string | null;
  challenge_tokens: number;
  rebuttal_tokens: number;
  created_at: string;
  finished_at: string | null;
  risks: Risk[];
}

export interface TaskRisks {
  audits: Audit[];
  open_risks: number;
}

/** Shown wherever audits show: what an audit is and isn't. */
export const NOT_A_PASS_TEXT = 'An audit never marks work passed or failed. Only checks do that, so “no risks raised” is not a pass.';
export const SETTING_HINT =
  'On important tasks, or ones nothing outside the bot will check, a second bot challenges the work with evidence and the owner answers once. An audit never marks work passed or failed, and each audit uses tokens.';

export const unresolved = (r: Pick<Risk, 'status'>) => r.status === 'open' || r.status === 'disputed';

export const RISK_STATE: Record<RiskStatus, { label: string; tone: 'amber' | 'rose' | 'emerald' | 'zinc' }> = {
  open: { label: 'Open', tone: 'amber' },
  disputed: { label: 'Disputed', tone: 'rose' },
  resolved: { label: 'Resolved', tone: 'emerald' },
  dismissed: { label: 'Dismissed', tone: 'zinc' },
};
export const riskState = (s: string) => RISK_STATE[s as RiskStatus] ?? { label: s, tone: 'zinc' as const };

/** Is the audit still going (so a card may be waiting on the person)? */
export const auditLive = (a: Pick<Audit, 'status'>) => a.status === 'queued' || a.status === 'challenge' || a.status === 'rebuttal';

/** The bot whose turn it is, or null when no turn is running. */
export function auditTurnBot(a: Pick<Audit, 'status' | 'challenger_kwant_id' | 'owner_kwant_id'>): string | null {
  if (a.status === 'challenge') return a.challenger_kwant_id;
  if (a.status === 'rebuttal') return a.owner_kwant_id;
  return null;
}

const name = (id: string, names: Record<string, string>) => names[id] || 'A bot';

/** One plain line for an audit's state. */
export function auditStatusText(a: Audit, names: Record<string, string> = {}): string {
  const challenger = name(a.challenger_kwant_id, names);
  const owner = name(a.owner_kwant_id, names);
  switch (a.status) {
    case 'queued':
      return `${challenger} will challenge it once the audit before it finishes.`;
    case 'challenge':
      return `${challenger} is challenging the work.`;
    case 'rebuttal':
      return `${owner} is answering the risks.`;
    case 'done':
      return a.risks.length === 0 ? `${challenger} raised no risks. That isn’t a pass.` : `${challenger} raised ${plural(a.risks.length, 'risk')}; ${owner} answered.`;
    case 'skipped':
      return `Skipped${a.reason ? `: ${a.reason}` : ''}.`;
    case 'failed':
      return `Didn’t finish${a.reason ? `: ${a.reason}` : ''}.`;
    case 'interrupted':
      return 'Stopped when the app restarted.';
    default:
      return a.status;
  }
}

export const TRIGGER_LABELS: Record<string, string> = {
  high_impact: 'high or urgent task',
  low_confidence: 'nothing outside the bot checks it',
  manual: 'started by you',
};
export const triggerLabel = (t: string) => TRIGGER_LABELS[t] ?? t.replace(/_/g, ' ');

const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;

export function auditTokens(a: Pick<Audit, 'challenge_tokens' | 'rebuttal_tokens'>): number {
  return Math.max(0, (a.challenge_tokens || 0) + (a.rebuttal_tokens || 0));
}

/** Board badge: unresolved risks per task, from `GET /api/risks`. */
export function openRiskCounts(risks: Array<Pick<Risk, 'task_id' | 'status'>>): Record<string, number> {
  const out: Record<string, number> = {};
  for (const r of risks) if (unresolved(r)) out[r.task_id] = (out[r.task_id] ?? 0) + 1;
  return out;
}

export const riskBadgeText = (n: number) => (n === 1 ? '1 open risk' : `${n} open risks`);

/** A live audit's turn waiting on one of the person's approval cards. */
export interface AuditWaiting {
  audit_id: string;
  kwant_id: string;
  approval_id: string;
  /** Where the card is: its own chat when the server says, else the bot's direct chat. */
  link: AwayLink;
}

/**
 * The approval card a live audit is waiting on: a waiting approval from the bot whose turn it is,
 * asked after the audit started. Audit turns always ask, so this is what stalls an audit.
 */
export function auditWaiting(audits: Audit[], waiting: AwayItem[]): AuditWaiting | null {
  for (const a of audits) {
    const bot = auditTurnBot(a);
    if (!bot) continue;
    const since = Date.parse(a.created_at);
    const card = waiting
      .filter((i) => i.kind === 'approval' && i.kwant_id === bot && !(Date.parse(i.timestamp) < since))
      .sort((x, y) => Date.parse(y.timestamp) - Date.parse(x.timestamp))[0];
    if (!card) continue;
    const l = card.link;
    const link = l && (l.thread_id || l.channel_id || l.direct_kwant_id) ? l : { thread_id: null, channel_id: null, direct_kwant_id: bot };
    return { audit_id: a.id, kwant_id: bot, approval_id: card.id, link };
  }
  return null;
}

/** Where "Answer in chat" goes for a stalled audit: the card's chat, else the bot's direct chat. */
export function auditCardPlace(w: AuditWaiting): { channelId: string | null; kwantId: string | null; threadId: string | null } {
  const l = w.link;
  if (l.channel_id) return { channelId: l.channel_id, kwantId: null, threadId: l.thread_id };
  return { channelId: null, kwantId: l.direct_kwant_id ?? w.kwant_id, threadId: l.thread_id };
}

export interface AuditorSettings {
  auditor: 'on' | 'off';
  auditor_daily_cap: number;
}
export const DAILY_CAP_MIN = 1;
export const DAILY_CAP_MAX = 50;
export const DAILY_CAPS = [1, 2, 3, 5, 10, 20, 50];

/** `auditor` and `auditor_daily_cap` from GET /api/settings; null on a server without the Auditor. */
export function parseAuditorSettings(data: unknown): AuditorSettings | null {
  if (!data || typeof data !== 'object') return null;
  const d = data as Record<string, unknown>;
  if (d.auditor !== 'on' && d.auditor !== 'off') return null;
  const cap = typeof d.auditor_daily_cap === 'number' ? Math.max(DAILY_CAP_MIN, Math.min(DAILY_CAP_MAX, Math.round(d.auditor_daily_cap))) : 5;
  return { auditor: d.auditor, auditor_daily_cap: cap };
}

/** What a resolve press says afterwards. */
export const riskResolvedText = (r: RiskResolution) => (r === 'resolved' ? 'Marked resolved. The task’s state doesn’t change.' : 'Dismissed. The task’s state doesn’t change.');
