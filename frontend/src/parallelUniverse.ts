// Parallel Universe: try up to three ways to deploy one app in throwaway containers on this server
// and compare them. Every route is person-only. Nothing here touches a cloud account or spends money:
// costs come from a dated price table checked into kwant, never from a live call.
//
// Every number the server returns is a `Figure` with a `tag`. The UI never shows a number without
// saying which kind it is, so an estimate can't pass for a measurement.

import { PersonCancelledError, personFetch } from './auth';

export const MAX_OPTIONS = 3;
export const MAX_TOTAL_MEMORY_MB = 3072;
export const LIMIT_RANGES = {
  cpus: { min: 0.1, max: 2, label: 'CPUs' },
  memory_mb: { min: 64, max: 2048, label: 'Memory' },
  pids: { min: 16, max: 512, label: 'Processes' },
  timeout_secs: { min: 30, max: 900, label: 'Time limit' },
} as const;
export type LimitKey = keyof typeof LIMIT_RANGES;

export interface Limits {
  cpus: number;
  memory_mb: number;
  pids: number;
  timeout_secs: number;
}

export interface AppSpec {
  image?: string | null;
  context?: string | null;
  dockerfile?: string | null;
  port: number;
  health_path: string;
}

export interface DeployOption {
  key: string;
  label: string;
  plan?: string | null;
  app: AppSpec;
  limits: Limits;
  ops_notes: string;
  rollback_notes: string;
}

export type CriterionKey = 'cost' | 'p95_latency' | 'ops_burden' | 'rollback';

export interface Scenario {
  id: string;
  title: string;
  question: string;
  requirements: { notes: string; min_memory_mb: number | null; needs_persistent_disk: boolean; regions: string[] };
  traffic: { monthly_requests: number; peak_rps: number; avg_response_kb: number; avg_duration_ms: number };
  load: { requests: number; concurrency: number; path: string };
  criteria: Array<{ key: CriterionKey; weight: number }>;
  options: DeployOption[];
  build_network: boolean;
  created_at: string;
  updated_at: string;
}

export interface Price {
  id: string;
  provider: string;
  plan: string;
  kind: string;
  currency: string;
  monthly: number;
  source_url: string;
  as_of: string;
  notes: string;
}

export type Figure = {
  metric: string;
  value: number;
  unit: string;
} & (
  | { tag: 'verified'; command: string; raw_output: string; measured_at: string }
  | { tag: 'estimate'; source_url: string; as_of: string; stale: boolean; assumptions: string[] }
  | { tag: 'speculation'; note: string }
);
export type FigureTag = Figure['tag'];

export type RunStatus = 'running' | 'finished' | 'interrupted';
export type BranchStatus = 'pending' | 'running' | 'ok' | 'failed';

export interface Branch {
  key: string;
  label: string;
  status: BranchStatus;
  error_code: string | null;
  error: string | null;
  figures: Figure[];
  cleanup_errors: string[];
  build_network_used: boolean;
  started_at: string | null;
  finished_at: string | null;
}

export interface Run {
  id: string;
  scenario_id: string;
  status: RunStatus;
  scenario: Scenario;
  branches: Branch[];
  cleanup_errors: string[];
  started_at: string;
  finished_at: string | null;
  swept_at: string | null;
}

export interface Note {
  criterion: CriterionKey;
  text: string;
  tag: 'speculation';
  note: string;
}

export interface OptionReport {
  key: string;
  label: string;
  plan: string | null;
  status: BranchStatus;
  error_code: string | null;
  error: string | null;
  measured: Figure[];
  cost: Figure;
  notes: Note[];
  build_network_used: boolean;
}

export interface Leader {
  criterion: CriterionKey;
  option: string;
  value: number;
  unit: string;
  basis: string;
}

export interface Report {
  run_id: string;
  scenario_id: string;
  title: string;
  question: string;
  status: RunStatus;
  started_at: string;
  finished_at: string | null;
  build_network_used: boolean;
  options: OptionReport[];
  leaders: Leader[];
  caveats: string[];
}

export type Tone = 'emerald' | 'amber' | 'violet' | 'rose' | 'sky' | 'zinc';

/** What a number is, in the words the person sees. Unknown tags read as a guess, never as measured. */
export function figureKind(tag: string): { label: 'Measured' | 'Estimate' | 'Guess'; tone: Tone; hint: string } {
  if (tag === 'verified') return { label: 'Measured', tone: 'emerald', hint: 'Measured on this server by a load test.' };
  if (tag === 'estimate') return { label: 'Estimate', tone: 'amber', hint: 'Worked out from a dated price list, not measured.' };
  return { label: 'Guess', tone: 'violet', hint: 'Neither measured nor priced.' };
}

const METRICS: Record<string, string> = {
  p50_latency: 'Median response',
  p95_latency: '95% answered within',
  p99_latency: '99% answered within',
  requests_per_sec: 'Requests per second',
  ok_share: 'Requests answered OK',
  success_rate: 'Requests completed',
  memory_after_load: 'Memory after load',
  monthly_cost: 'Monthly cost',
};
const humanize = (s: string) => {
  const t = s.replace(/[_-]+/g, ' ').trim();
  return t ? t[0].toUpperCase() + t.slice(1) : 'Other';
};
export const metricLabel = (metric: string): string => METRICS[metric] ?? humanize(metric);

const CRITERIA: Record<string, string> = { cost: 'Cheapest', p95_latency: 'Fastest', ops_burden: 'Easiest to run', rollback: 'Easiest to roll back' };
export const leaderLabel = (c: string): string => CRITERIA[c] ?? humanize(c);
const NOTE_LABELS: Record<string, string> = { ops_burden: 'Running it', rollback: 'Rolling back' };
export const noteLabel = (c: string): string => NOTE_LABELS[c] ?? humanize(c);

/** `57.5 ms`, `591 /s`, `99.5 %`, `6.00 USD`. */
export function formatValue(value: number, unit: string): string {
  if (!Number.isFinite(value)) return '—';
  if (unit === 'ratio') return `${(value * 100).toFixed(value === 1 ? 0 : 1)} %`;
  if (unit === '1/s') return `${value >= 100 ? Math.round(value) : value.toFixed(1)} /s`;
  if (/^[A-Z]{3}$/.test(unit)) return `${value.toFixed(2)} ${unit}`;
  const n = value >= 100 ? Math.round(value).toString() : value >= 10 ? value.toFixed(1) : value.toFixed(value === 0 ? 0 : 2);
  return `${n} ${unit}`;
}

/** The host of a price source, for a short link text. */
export function sourceHost(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}
export const safeHttps = (url: string | null | undefined): string | null => (url && /^https:\/\//i.test(url) ? url : null);

export const RUN_STATUS: Record<RunStatus, { label: string; tone: Tone }> = {
  running: { label: 'Running', tone: 'sky' },
  finished: { label: 'Finished', tone: 'emerald' },
  interrupted: { label: 'Interrupted', tone: 'rose' },
};
export const runStatus = (s: string): { label: string; tone: Tone } => RUN_STATUS[s as RunStatus] ?? { label: humanize(s), tone: 'zinc' };

export const BRANCH_STATUS: Record<BranchStatus, { label: string; tone: Tone }> = {
  pending: { label: 'Waiting', tone: 'zinc' },
  running: { label: 'Testing', tone: 'sky' },
  ok: { label: 'Done', tone: 'emerald' },
  failed: { label: 'Failed', tone: 'rose' },
};
export const branchStatus = (s: string): { label: string; tone: Tone } => BRANCH_STATUS[s as BranchStatus] ?? { label: humanize(s), tone: 'zinc' };

/** "2 of 3 options tried". */
export function runProgress(run: Pick<Run, 'branches'>): { done: number; total: number; text: string } {
  const total = run.branches.length;
  const done = run.branches.filter((b) => b.status === 'ok' || b.status === 'failed').length;
  return { done, total, text: `${done} of ${total} option${total === 1 ? '' : 's'} tried` };
}

/** What an interrupted run means, said plainly. */
export const INTERRUPTED_TEXT =
  'The server stopped before this run finished. Options that hadn’t finished have no numbers, and their containers were removed when the server started again.';

// ---------- The form ----------

/** The form keeps what the person typed, so a half-typed number isn't lost. */
export interface OptionDraft {
  key: string;
  label: string;
  plan: string;
  source: 'image' | 'context';
  image: string;
  context: string;
  dockerfile: string;
  port: string;
  health_path: string;
  cpus: string;
  memory_mb: string;
  pids: string;
  timeout_secs: string;
  ops_notes: string;
  rollback_notes: string;
}

export interface ScenarioDraft {
  title: string;
  question: string;
  monthly_requests: string;
  peak_rps: string;
  avg_response_kb: string;
  avg_duration_ms: string;
  load_requests: string;
  load_concurrency: string;
  load_path: string;
  build_network: boolean;
  options: OptionDraft[];
}

export const emptyOption = (): OptionDraft => ({
  key: '',
  label: '',
  plan: '',
  source: 'image',
  image: '',
  context: '',
  dockerfile: '',
  port: '8080',
  health_path: '/',
  cpus: '1',
  memory_mb: '512',
  pids: '128',
  timeout_secs: '300',
  ops_notes: '',
  rollback_notes: '',
});

export const emptyScenario = (): ScenarioDraft => ({
  title: '',
  question: '',
  monthly_requests: '100000',
  peak_rps: '5',
  avg_response_kb: '20',
  avg_duration_ms: '50',
  load_requests: '2000',
  load_concurrency: '10',
  load_path: '/',
  build_network: false,
  options: [emptyOption()],
});

export function draftFrom(s: Scenario): ScenarioDraft {
  return {
    title: s.title,
    question: s.question,
    monthly_requests: String(s.traffic.monthly_requests),
    peak_rps: String(s.traffic.peak_rps),
    avg_response_kb: String(s.traffic.avg_response_kb),
    avg_duration_ms: String(s.traffic.avg_duration_ms),
    load_requests: String(s.load.requests),
    load_concurrency: String(s.load.concurrency),
    load_path: s.load.path,
    build_network: s.build_network,
    options: s.options.map((o) => ({
      key: o.key,
      label: o.label,
      plan: o.plan ?? '',
      source: o.app.context ? 'context' : 'image',
      image: o.app.image ?? '',
      context: o.app.context ?? '',
      dockerfile: o.app.dockerfile ?? '',
      port: String(o.app.port),
      health_path: o.app.health_path,
      cpus: String(o.limits.cpus),
      memory_mb: String(o.limits.memory_mb),
      pids: String(o.limits.pids),
      timeout_secs: String(o.limits.timeout_secs),
      ops_notes: o.ops_notes,
      rollback_notes: o.rollback_notes,
    })),
  };
}

/** `DigitalOcean droplet` → `digitalocean-droplet`: the option key the server wants. */
export function slugify(s: string): string {
  return s
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32)
    .replace(/-+$/, '');
}

const num = (s: string): number | null => {
  const t = s.trim().replace(',', '.');
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
};
const isInt = (n: number | null): n is number => n !== null && Number.isInteger(n);
const isImageRef = (s: string) => s.length > 0 && s.length <= 256 && !s.startsWith('-') && /^[A-Za-z0-9./:_@-]+$/.test(s);
const isUrlPath = (s: string) => s.startsWith('/') && s.length <= 512 && /^[\x21-\x7e]+$/.test(s);

/** Total memory the options reserve, counting only what parses. */
export const reservedMemory = (d: ScenarioDraft): number => d.options.reduce((sum, o) => sum + (num(o.memory_mb) ?? 0), 0);

/** Problems by field name (`title`, `options.1.memory_mb`, `memory_total`, …), in the order they appear. */
export type FormErrors = Record<string, string>;

/** Checks the draft the way the server will and builds the body. Messages say what to change. */
export function validateScenario(d: ScenarioDraft, planIds?: Set<string>): { errors: FormErrors; body: Omit<Scenario, 'id' | 'created_at' | 'updated_at' | 'requirements' | 'criteria'> | null } {
  const errors: FormErrors = {};
  const title = d.title.trim();
  if (!title) errors.title = 'Give the scenario a name.';
  else if ([...title].length > 120) errors.title = 'Keep the name to 120 characters.';
  if ([...d.question].length > 2000) errors.question = 'Keep the question to 2000 characters.';

  const traffic = {
    monthly_requests: num(d.monthly_requests),
    peak_rps: num(d.peak_rps),
    avg_response_kb: num(d.avg_response_kb),
    avg_duration_ms: num(d.avg_duration_ms),
  };
  if (!isInt(traffic.monthly_requests) || traffic.monthly_requests < 0 || traffic.monthly_requests > 100_000_000_000)
    errors.monthly_requests = 'A whole number from 0.';
  for (const k of ['peak_rps', 'avg_response_kb', 'avg_duration_ms'] as const) {
    const v = traffic[k];
    if (v === null || v < 0 || v > 1e7) errors[k] = 'A number from 0.';
  }
  const load = { requests: num(d.load_requests), concurrency: num(d.load_concurrency), path: d.load_path.trim() };
  if (!isInt(load.requests) || load.requests < 1 || load.requests > 100_000) errors.load_requests = 'From 1 to 100,000.';
  if (!isInt(load.concurrency) || load.concurrency < 1 || load.concurrency > 64) errors.load_concurrency = 'From 1 to 64.';
  if (!isUrlPath(load.path)) errors.load_path = 'Starts with /, like /health.';

  if (d.options.length === 0) errors.options = 'Add at least one option.';
  if (d.options.length > MAX_OPTIONS) errors.options = `At most ${MAX_OPTIONS} options.`;

  const keys = new Set<string>();
  const options: DeployOption[] = d.options.map((o, i) => {
    const at = (f: string) => `options.${i}.${f}`;
    const label = o.label.trim();
    if (!label) errors[at('label')] = 'Name this option.';
    else if ([...label].length > 80) errors[at('label')] = 'Keep it to 80 characters.';
    // A new option gets its key from its name; an existing one keeps its key so past runs still match.
    let key = o.key || slugify(label) || `option-${i + 1}`;
    if (keys.has(key)) {
      if (o.key) errors[at('label')] = 'Two options have the same key. Rename one.';
      else {
        let n = 2;
        while (keys.has(`${key.slice(0, 29)}-${n}`)) n++;
        key = `${key.slice(0, 29)}-${n}`;
      }
    }
    keys.add(key);

    if (o.plan && planIds && !planIds.has(o.plan)) errors[at('plan')] = 'That plan isn’t in the price table.';

    const app: AppSpec = { port: 0, health_path: o.health_path.trim() || '/' };
    if (o.source === 'image') {
      const image = o.image.trim();
      if (!isImageRef(image)) errors[at('image')] = 'A Docker image on this server, like myapp:latest.';
      app.image = image;
    } else {
      const context = o.context.trim();
      if (!context.startsWith('/') || context.length > 1024 || context.split('/').includes('..'))
        errors[at('context')] = 'A full folder path on this server, like /home/me/app, without “..”.';
      app.context = context;
      const df = o.dockerfile.trim();
      if (df) {
        if (df.startsWith('/') || df.split('/').some((p) => p === '..' || p === '.' || p === '')) errors[at('dockerfile')] = 'A path inside the folder, like Dockerfile.';
        app.dockerfile = df;
      }
    }
    const port = num(o.port);
    if (!isInt(port) || port < 1 || port > 65535) errors[at('port')] = 'From 1 to 65535.';
    else app.port = port;
    if (!isUrlPath(app.health_path)) errors[at('health_path')] = 'Starts with /.';

    const limits = {} as Limits;
    for (const k of Object.keys(LIMIT_RANGES) as LimitKey[]) {
      const r = LIMIT_RANGES[k];
      const v = num(o[k]);
      const ok = v !== null && v >= r.min && v <= r.max && (k === 'cpus' || Number.isInteger(v));
      if (!ok) errors[at(k)] = `From ${r.min} to ${r.max}.`;
      limits[k] = v ?? 0;
    }
    if ([...o.ops_notes].length > 2000 || [...o.rollback_notes].length > 2000) errors[at('ops_notes')] = 'Keep notes to 2000 characters each.';
    return { key, label, plan: o.plan || null, app, limits, ops_notes: o.ops_notes.trim(), rollback_notes: o.rollback_notes.trim() };
  });

  const total = reservedMemory(d);
  if (total > MAX_TOTAL_MEMORY_MB) errors.memory_total = `The options reserve ${total} MB together. The limit is ${MAX_TOTAL_MEMORY_MB} MB, so lower the memory of one or more options.`;

  if (Object.keys(errors).length) return { errors, body: null };
  return {
    errors,
    body: {
      title,
      question: d.question.trim(),
      traffic: traffic as Scenario['traffic'],
      load: load as Scenario['load'],
      options,
      build_network: d.build_network,
    },
  };
}

// ---------- API ----------

/** Why a call failed, as the panel shows it. */
export class ParallelError extends Error {
  constructor(
    message: string,
    public status: number,
    /** 409 `code: "busy"`: another run is going. Not an error the person caused. */
    public busy = false,
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

/** Turns a response the server refused into a `ParallelError`. */
export function errorFrom(status: number, answer: unknown): ParallelError {
  const a = answer && typeof answer === 'object' ? (answer as Record<string, unknown>) : {};
  if (status === 409 && a.code === 'busy') return new ParallelError('Another run is going. It can start once that one finishes.', 409, true);
  const err = typeof a.error === 'string' && a.error.trim() ? sentence(a.error) : null;
  if (status === 403) return new ParallelError('Only you can see and run Parallel Universe.', 403);
  if (status === 404 && !err) return new ParallelError('This server doesn’t have Parallel Universe yet. Update the app and try again.', 404);
  return new ParallelError(err ?? `Something went wrong (HTTP ${status}).`, status);
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
    if (e instanceof PersonCancelledError) throw new ParallelError(e.message, 401);
    throw new ParallelError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const answer = await res.json().catch(() => null);
  if (res.ok) return answer as T;
  throw errorFrom(res.status, answer);
}

const enc = encodeURIComponent;
export type ScenarioBody = NonNullable<ReturnType<typeof validateScenario>['body']>;

export const parallelApi = {
  scenarios: () => call<{ scenarios: Scenario[] }>('/api/parallel/scenarios').then((r) => r.scenarios),
  create: (s: ScenarioBody) => call<Scenario>('/api/parallel/scenarios', 'POST', s),
  /** PUT replaces the whole scenario, so the fields the form doesn't show come from `old`. */
  update: (old: Scenario, s: ScenarioBody) =>
    call<Scenario>(`/api/parallel/scenarios/${enc(old.id)}`, 'PUT', { ...s, requirements: old.requirements, criteria: old.criteria }),
  remove: (id: string) => call<unknown>(`/api/parallel/scenarios/${enc(id)}`, 'DELETE'),
  prices: () => call<{ prices: Price[]; stale_after_days: number }>('/api/parallel/prices'),
  start: (id: string) => call<{ run: Run }>(`/api/parallel/scenarios/${enc(id)}/run`, 'POST', {}),
  runs: (scenarioId: string) => call<{ runs: Run[]; running: boolean }>(`/api/parallel/scenarios/${enc(scenarioId)}/runs`),
  run: (id: string) => call<{ run: Run; report: Report }>(`/api/parallel/runs/${enc(id)}`),
};
