// "While you were away" (batch 3): what agents need from you, finished and failed since you last
// looked. Pure helpers, so the page and the node tests share them; the API calls are at the bottom.

export type AwayKind = 'question' | 'approval' | 'delegation' | 'routine_run';
export type AwaySection = 'waiting' | 'finished' | 'failed';
export type ApprovalDecision = 'allow_once' | 'always_allow' | 'deny';

export interface AwayLink {
  thread_id: string | null;
  channel_id: string | null;
  direct_kwant_id: string | null;
}

export interface AwayItem {
  kind: AwayKind;
  id: string;
  kwant_id: string;
  kwant_name: string;
  title: string;
  summary: string;
  status: string;
  timestamp: string;
  link: AwayLink;
  /** Questions only. */
  choices?: string[];
  /** Approvals only, from servers that send them: the tool and the card's details. */
  tool_name?: string;
  details?: Record<string, unknown>;
}

export interface AwayCounts {
  waiting: number;
  finished: number;
  failed: number;
  total: number;
}

export interface AwayResponse {
  since: string;
  last_seen: string | null;
  now: string;
  stuck_after_minutes: number;
  waiting: AwayItem[];
  finished: AwayItem[];
  failed: AwayItem[];
  unseen: AwayCounts;
}

export const AWAY_SECTIONS: { key: AwaySection; label: string; empty: string }[] = [
  { key: 'waiting', label: 'Waiting on you', empty: 'No questions or approvals waiting.' },
  { key: 'finished', label: 'Finished', empty: 'Nothing finished since you last looked.' },
  { key: 'failed', label: 'Failed or stuck', empty: 'Nothing failed or stuck.' },
];

/** "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", else the date. */
export function relativeTime(ts: string, now: number = Date.now()): string {
  const t = Date.parse(ts);
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return 'just now';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.floor(h / 24);
  if (d === 1) return 'yesterday';
  if (d < 7) return `${d} days ago`;
  return new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** Page subtitle: the 7-day default window, or since the last visit. */
export function windowLabel(since: string, now: number = Date.now()): string {
  const t = Date.parse(since);
  if (Number.isNaN(t)) return '';
  if (now - t >= 7 * 86400_000 - 60_000) return 'From the last 7 days';
  return `Since you last looked, ${relativeTime(since, now)}`;
}

export type Tone = 'amber' | 'emerald' | 'rose' | 'zinc';

/** Short label and colour for an item's kind and status. */
export function itemLabel(item: Pick<AwayItem, 'kind' | 'status'>): { label: string; tone: Tone } {
  switch (item.kind) {
    case 'question':
      return { label: 'Question', tone: 'amber' };
    case 'approval':
      return { label: 'Needs approval', tone: 'amber' };
    case 'routine_run':
      return item.status === 'ok' ? { label: 'Routine ran', tone: 'emerald' } : { label: 'Routine failed', tone: 'rose' };
    case 'delegation':
      switch (item.status) {
        case 'completed':
          return { label: 'Handoff done', tone: 'emerald' };
        case 'stuck':
          return { label: 'Stuck', tone: 'amber' };
        case 'interrupted':
          return { label: 'Interrupted', tone: 'rose' };
        default:
          return { label: 'Handoff failed', tone: 'rose' };
      }
  }
  return { label: item.status, tone: 'zinc' };
}

/** Sidebar badge text: nothing at 0, "99+" past 99. */
export const badgeText = (n: number): string => (n <= 0 ? '' : n > 99 ? '99+' : String(n));

export const isAwayEmpty = (r: Pick<AwayResponse, AwaySection> | null): boolean =>
  !!r && r.waiting.length === 0 && r.finished.length === 0 && r.failed.length === 0;

/** Where clicking an item goes: exactly one of thread, channel or direct chat. */
export type AwayTarget = { thread: string; kwant: string } | { channel: string } | { direct: string };

export function awayTarget(item: Pick<AwayItem, 'kwant_id' | 'link'>): AwayTarget {
  const l = item.link;
  if (l.thread_id) return { thread: l.thread_id, kwant: item.kwant_id };
  if (l.channel_id) return { channel: l.channel_id };
  return { direct: l.direct_kwant_id || item.kwant_id };
}

/** Drops an acted-on item right away, before the refresh confirms it. */
export function withoutItem(r: AwayResponse, section: AwaySection, id: string): AwayResponse {
  const list = r[section].filter((i) => i.id !== id);
  if (list.length === r[section].length) return r;
  const dropped = section === 'waiting' ? 1 : 0;
  return {
    ...r,
    [section]: list,
    unseen: { ...r.unseen, waiting: Math.max(0, r.unseen.waiting - dropped), total: Math.max(0, r.unseen.total - dropped) },
  };
}

/** The message for a failed one-tap action; a 409 means the request lapsed meanwhile. */
export const actionErrorText = (status: number, fallback: string): string =>
  status === 409 || status === 404 ? 'This request expired' : fallback;

export class AwayError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

async function call<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let message = `Request failed (HTTP ${res.status}).`;
    try {
      const data = await res.json();
      if (data && typeof data.error === 'string' && data.error) message = data.error;
    } catch {
      // no JSON body
    }
    throw new AwayError(res.status, message);
  }
  return res.json();
}

const enc = encodeURIComponent;

export const awayApi = {
  get: (since?: string) => call<AwayResponse>('GET', `/api/away${since ? `?since=${enc(since)}` : ''}`),
  seen: () => call<{ last_seen: string }>('POST', '/api/away/seen'),
  answer: (id: string, choice: number) => call<unknown>('POST', `/api/questions/${enc(id)}/answer`, { choice }),
  resolve: (id: string, decision: ApprovalDecision) => call<unknown>('POST', `/api/approvals/${enc(id)}/resolve`, { decision }),
};

/** SSE events that can change what the page shows, so the badge re-counts. */
const AWAY_EVENTS = new Set([
  'question_updated', 'QuestionUpdated',
  'delegation_updated', 'DelegationUpdated',
  'routine_run', 'RoutineRun',
  'ApprovalRequired', 'ApprovalWithdrawn',
]);
export const affectsAway = (type: unknown): boolean => typeof type === 'string' && AWAY_EVENTS.has(type);
