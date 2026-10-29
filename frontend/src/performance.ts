// Performance Market (Phase D1, docs/mission-control.md §17): how each bot does per kind of work.
// Report only: nothing here (or on the server) changes who gets work. Person-only on the server.
import { personFetch, PersonCancelledError } from './auth';
import { classLabel } from './flightSchool';
import { formatTokens } from './missions';

export type PerfBy = 'bot' | 'model';
export type PerfSince = '30d' | '90d' | 'all';

export interface PerfGroup {
  task_class: string;
  kwant_id: string;
  kwant_name: string | null;
  engine: string | null;
  /** Only when grouped by model. */
  model?: string;
  n: number;
  verified: number;
  failed: number;
  unverified: number;
  verified_rate: number;
  verified_ci95: [number, number];
  median_latency_ms: number | null;
  median_fresh_tokens: number | null;
  cost_usd: number | null;
  cost_per_task_usd: number | null;
  interventions_per_task: number;
  low_confidence: boolean;
}

export interface PerfReport {
  since: string;
  report_only: boolean;
  min_n: number;
  observations?: number;
  groups: PerfGroup[];
}

export interface PerfQuery {
  by: PerfBy;
  since: PerfSince;
  class?: string;
  bot?: string;
}

export const SINCE_OPTIONS: Array<{ value: PerfSince; label: string }> = [
  { value: '30d', label: 'Last 30 days' },
  { value: '90d', label: 'Last 90 days' },
  { value: 'all', label: 'All time' },
];

/** "All time" is a date before any kwant data; the server caps `Nd` at 365. */
const ALL_SINCE = '2000-01-01';

export function perfUrl(q: PerfQuery): string {
  const p = new URLSearchParams({ by: q.by, since: q.since === 'all' ? ALL_SINCE : q.since });
  if (q.class) p.set('class', q.class);
  if (q.bot) p.set('bot', q.bot);
  return `/api/performance?${p}`;
}

export class PerfError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

export async function loadPerformance(q: PerfQuery): Promise<PerfReport> {
  let res: Response;
  try {
    res = await personFetch(perfUrl(q), { cache: 'no-store' });
  } catch (e) {
    if (e instanceof PersonCancelledError) throw new PerfError(e.message, 401);
    throw new PerfError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const body = await res.json().catch(() => null);
  if (res.ok && body && Array.isArray(body.groups)) return body as PerfReport;
  if (res.ok) throw new PerfError('The server sent a report this app can’t read.', res.status);
  if (res.status === 404) throw new PerfError('This server doesn’t have performance numbers yet. Update the app.', 404);
  const err = body && typeof body.error === 'string' && body.error.trim() ? body.error.trim() : `Something went wrong (HTTP ${res.status}).`;
  throw new PerfError(err, res.status);
}

/** What the section shows. 403: the server says this isn't the person, so there's no section at all. */
export type PerfView = 'hidden' | 'loading' | 'error' | 'empty' | 'rows';
export function perfView(state: { report: PerfReport | null; error: PerfError | null; loading: boolean }): PerfView {
  if (state.error?.status === 403) return 'hidden';
  if (state.error) return 'error';
  if (!state.report) return 'loading';
  return state.report.groups.length ? 'rows' : 'empty';
}

const pct = (x: number) => Math.round(Math.max(0, Math.min(1, x)) * 100);

/** "56% (27–81%)": the verified rate with its 95% interval. */
export function rateText(rate: number, ci: [number, number] | null | undefined): string {
  const r = `${pct(rate)}%`;
  if (!ci || ci.length !== 2) return r;
  return `${r} (${pct(ci[0])}–${pct(ci[1])}%)`;
}

/** Screen-reader form: "56% verified, likely between 27% and 81%". */
export function rateLabel(rate: number, ci: [number, number] | null | undefined): string {
  const r = `${pct(rate)}% verified`;
  if (!ci || ci.length !== 2) return r;
  return `${r}, likely between ${pct(ci[0])}% and ${pct(ci[1])}%`;
}

/** "45 s", "3.5 min", "1.2 h"; "—" when unknown. */
export function latencyText(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return '—';
  const s = ms / 1000;
  if (s < 60) return `${Math.max(1, Math.round(s))} s`;
  const m = s / 60;
  if (m < 60) return `${m < 10 ? m.toFixed(1).replace(/\.0$/, '') : Math.round(m)} min`;
  const h = m / 60;
  return `${h < 10 ? h.toFixed(1).replace(/\.0$/, '') : Math.round(h)} h`;
}

export function tokensText(n: number | null | undefined): string {
  return n === null || n === undefined || !Number.isFinite(n) ? '—' : formatTokens(n);
}

/** "$0.15", "<$0.01", "—" when no run reported a cost. */
export function costText(usd: number | null | undefined): string {
  if (usd === null || usd === undefined || !Number.isFinite(usd)) return '—';
  if (usd === 0) return '$0';
  if (usd < 0.01) return '<$0.01';
  return `$${usd < 10 ? usd.toFixed(2) : Math.round(usd)}`;
}

/** Interventions per task: "0", "0.4", "2". */
export function perTaskText(x: number | null | undefined): string {
  if (x === null || x === undefined || !Number.isFinite(x)) return '—';
  return x === 0 ? '0' : x < 10 ? x.toFixed(1).replace(/\.0$/, '') : String(Math.round(x));
}

export const FEW_TASKS = 'few tasks';

export function botName(g: Pick<PerfGroup, 'kwant_name' | 'kwant_id'>, names: Record<string, string> = {}): string {
  return g.kwant_name || names[g.kwant_id] || 'Removed bot';
}

/** "Claude Opus" style labels aren't known here; show the model id, or say no usage matched. */
export function modelText(m: string | undefined): string {
  if (!m || m === 'unknown' || m === 'auto') return 'Not reported';
  return m;
}

export const classText = classLabel;

/** Options seen so far, kept across filter changes so a filter never hides its own choices. */
export function mergeOptions(prev: Record<string, string>, groups: PerfGroup[], pick: (g: PerfGroup) => [string, string]): Record<string, string> {
  const next = { ...prev };
  for (const g of groups) {
    const [k, v] = pick(g);
    next[k] = v;
  }
  return next;
}
