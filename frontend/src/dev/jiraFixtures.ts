// 0.7.6 "Work from Jira": example answers for the setup sheet (tests and the screenshot harness only).
// Shaped like Juno's routes (api.md §2.2, §2.3) and design.md §12. Example data only: Trailhead Outfitters,
// yourcompany.atlassian.net / jira.example.com / confluence.example.com, oleks@example.com. No token
// appears here: the transport below answers by what was asked, never by echoing a token back.

import { DEFAULT_LABEL } from './jiraSetup';

export const FX_EMAIL = 'oleks@example.com';
export const FX_CLOUD = 'https://yourcompany.atlassian.net';
export const FX_DC = 'https://jira.example.com';
export const FX_CONFLUENCE = 'https://confluence.example.com';

const PROJECTS = [
  { key: 'TRAIL', name: 'Storefront' },
  { key: 'SUP', name: 'Support' },
  { key: 'WEB', name: 'Website' },
  { key: 'OPS', name: 'Operations' },
  { key: 'MOB', name: 'Mobile app' },
  { key: 'DATA', name: 'Reporting' },
  { key: 'HR', name: 'People' },
];
const SPACES = [
  { id: '101', key: 'SHOP', name: 'Shop handbook' },
  { id: '102', key: 'ENG', name: 'Engineering' },
  { id: '103', key: 'HR', name: 'People' },
];
const SAMPLE = [
  {
    key: 'TRAIL-151',
    summary: 'Add a size guide link to product pages',
    url: `${FX_CLOUD}/browse/TRAIL-151`,
  },
  {
    key: 'TRAIL-148',
    summary: 'Gift cards can’t be combined with discount codes',
    url: `${FX_CLOUD}/browse/TRAIL-148`,
  },
  {
    key: 'TRAIL-146',
    summary: 'Low-stock email goes out twice',
    url: `${FX_CLOUD}/browse/TRAIL-146`,
  },
];

const blank = {
  message: null,
  confluence_url: null,
  auth: null,
  cloud_id: null,
  me: null,
  projects: [],
  confluence: null,
  jql: null,
  jql_error: null,
  count: null,
  sample: [],
  retry_at: null,
  reads: [],
  scope_note: null,
  suggested_project: null,
};

/** A passing check, counted for the picked projects and rule (TRAIL + label kwant = 3 open tickets). */
export function fxFound(
  kind: 'cloud' | 'dc',
  o: {
    projects?: string[];
    handover?: string;
    jql?: string | null;
    count?: number;
  } = {},
) {
  const projects = o.projects?.length ? o.projects : null;
  const count = o.count ?? (projects ? (projects.includes('TRAIL') ? (o.handover === 'assignee' ? 1 : 3) + (projects.length - 1) * 2 : projects.length) : null);
  return {
    ...blank,
    result: 'found',
    kind,
    jira_url: kind === 'cloud' ? FX_CLOUD : FX_DC,
    confluence_url: kind === 'cloud' ? `${FX_CLOUD}/wiki` : FX_CONFLUENCE,
    auth: kind === 'cloud' ? 'scoped' : 'pat',
    me: {
      id: kind === 'cloud' ? '5b10ac8d82e05b22cc7d4ef5' : 'oleks',
      display_name: 'Oleks Example',
    },
    projects: PROJECTS,
    confluence: { result: 'ok', spaces: SPACES },
    jql: projects ? `project in (${projects.join(', ')}) AND labels = ${JSON.stringify(DEFAULT_LABEL)} AND statusCategory != Done` : null,
    count,
    sample: count ? SAMPLE.slice(0, Math.min(3, count)) : [],
    reads: ['jira', 'confluence'],
    scope_note: kind === 'cloud' ? 'scoped: reads Jira and Confluence' : null,
    suggested_project: { key: 'TRAIL', branches: 7 },
  };
}

export const fxAuthFailed = (kind: 'cloud' | 'dc') => ({
  ...blank,
  result: 'auth_failed',
  kind,
  jira_url: kind === 'cloud' ? FX_CLOUD : FX_DC,
  message: 'Jira didn’t accept this token.',
});
export const fxNotJira = () => ({
  ...blank,
  result: 'not_jira',
  kind: null,
  jira_url: 'https://yourcompny.atlassian.net',
  message: 'No Jira answered at this address.',
});
export const fxNoProjects = (kind: 'cloud' | 'dc') => ({
  ...blank,
  result: 'no_projects',
  kind,
  jira_url: kind === 'cloud' ? FX_CLOUD : FX_DC,
  me: { id: 'x', display_name: 'Oleks Example' },
  reads: ['jira'],
});
export const fxRateLimited = (kind: 'cloud' | 'dc', retryAt: string) => ({
  ...blank,
  result: 'rate_limited',
  kind,
  jira_url: kind === 'cloud' ? FX_CLOUD : FX_DC,
  retry_at: retryAt,
});
/** `POST jira/check {jira_url, detect: true}`: only the type detection runs (api-decisions §8). */
export const fxDetected = (kind: 'cloud' | 'dc') => ({
  ...blank,
  result: 'detected',
  kind,
  jira_url: kind === 'cloud' ? FX_CLOUD : FX_DC,
});

/** The row as GET jira sends it today (JSON-string lists, `on_`, `either`). */
/** `[{key, name}]` → `{KEY: name}`, as the server stores names from a check. */
const fxNames = (list: unknown): Record<string, string> => Object.fromEntries((Array.isArray(list) ? (list as { key: string; name: string }[]) : []).map((p) => [p.key, p.name]));

export const fxSiteRow = (o: Record<string, unknown> = {}) => ({
  space_id: 'spc_demo',
  id: 'jira_0a1b2c',
  kind: 'cloud',
  jira_url: FX_CLOUD,
  confluence_url: `${FX_CLOUD}/wiki`,
  email: FX_EMAIL,
  auth: 'scoped',
  cloud_id: null,
  me_id: 'x',
  me_name: 'Oleks Example',
  projects: '["TRAIL"]',
  handover: 'label',
  assignee_id: null,
  assignee_key: null,
  assignee_name: null,
  label: DEFAULT_LABEL,
  jql: null,
  team_id: 'team_shop',
  spaces: '["SHOP","ENG"]',
  comment_progress: false,
  wait_for_ok: false,
  poll_secs: 300,
  on_: true,
  backfill_done: true,
  connected_at: '2026-11-07T09:55:00Z',
  updated_at: '2026-11-07T09:55:00Z',
  polled_at: '2026-11-07T10:04:00Z',
  poll_error: null,
  backoff_until: null,
  unreachable_since: null,
  token_rejected_at: null,
  scope_note: 'scoped: reads Jira and Confluence',
  project_names: { TRAIL: 'Storefront' },
  space_names: { SHOP: 'Shop handbook', ENG: 'Engineering' },
  ...o,
});

const t = (key: string, summary: string, state: string, extra: Record<string, unknown> = {}) => ({ key, summary, state, url: `${FX_CLOUD}/browse/${key}`, ...extra });
/** One of each kind that matters to the badge: 1 needs you, 1 being built, 2 backfill Waiting. */
export const fxTickets = () => ({
  tickets: [
    t('TRAIL-142', 'Export orders as a spreadsheet', 'needs_answer'),
    t('TRAIL-139', 'Show delivery dates at checkout', 'working'),
    t('TRAIL-137', 'Wishlist count in the header', 'waiting', {
      waiting_since_connect: true,
    }),
    t('TRAIL-138', 'Search ignores accents', 'waiting', {
      waiting_since_connect: true,
    }),
  ],
  slots: 3,
});

export const fxUsers = () => ({
  users: [
    { id: '5b10ac8d82e05b22cc7d4ef5', name: null, display_name: 'Shop bots' },
    { id: '5b10ac8d82e05b22cc7d4ef6', name: null, display_name: 'Dana Park' },
  ],
});

export type FxScenario = {
  /** What POST jira/check answers when a token is sent: found (default), auth_failed, no_projects, rate_limited. */
  check?: 'found' | 'auth_failed' | 'no_projects' | 'rate_limited';
  /** The site GET jira answers with; null = never connected; 'missing' = an older server (404). */
  site?: Record<string, unknown> | null | 'missing';
  tickets?: unknown;
  /** The kind the token-less detection answers (default: Cloud for atlassian.net, else Data Center). */
  detectKind?: 'cloud' | 'dc';
  /** Hold every check this long (the Checking state). */
  delayMs?: number;
  /** Rate-limit time for "Trying again at …". */
  retryAt?: string;
};

/** Requests the transport saw (path, method, body without the token values). Tests read it. */
export interface FxCall {
  path: string;
  method: string;
  body: Record<string, unknown> | null;
}

/** A fetch-shaped transport for `setJiraTransport` answering like Juno's routes. Tokens are redacted in `calls`. */
export function fxTransport(sc: FxScenario = {}, calls: FxCall[] = []) {
  let site: Record<string, unknown> | null | 'missing' = sc.site === undefined ? null : sc.site;
  const json = (b: unknown, status = 200) =>
    new Response(JSON.stringify(b), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
  return async (url: string, init: RequestInit = {}): Promise<Response> => {
    const path = url.split('?')[0];
    const method = init.method ?? 'GET';
    const raw = typeof init.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : null;
    const body = raw ? Object.fromEntries(Object.entries(raw).map(([k, v]) => [k, /token/.test(k) && v ? '[redacted]' : v])) : null;
    calls.push({ path, method, body });
    if (sc.delayMs && /\/jira(\/check)?$/.test(path) && method !== 'GET') await new Promise((r) => setTimeout(r, sc.delayMs));
    if (path.endsWith('/dev/jira') && method === 'GET') return site === 'missing' ? json(null, 404) : json({ site, tickets: site ? 4 : 0 });
    if (path.endsWith('/dev/tickets')) return json(site && site !== 'missing' ? (sc.tickets ?? fxTickets()) : { tickets: [], slots: 3 });
    if (path.endsWith('/dev/jira/users')) return json(fxUsers());
    if (path.endsWith('/dev/jira/check') || (path.endsWith('/dev/jira') && method === 'PUT')) {
      const address = String(raw?.jira_url ?? '');
      const kind: 'cloud' | 'dc' = raw?.kind === 'dc' || raw?.kind === 'cloud' ? (raw.kind as 'cloud' | 'dc') : /atlassian\.net/.test(address) ? 'cloud' : 'dc';
      let out: Record<string, unknown>;
      if (/yourcompny|example\.invalid/.test(address)) out = fxNotJira();
      else if (raw?.detect === true && method !== 'PUT') out = fxDetected(sc.detectKind ?? kind);
      else if (sc.check === 'auth_failed') out = fxAuthFailed(kind);
      else if (sc.check === 'no_projects') out = fxNoProjects(kind);
      else if (sc.check === 'rate_limited') out = fxRateLimited(kind, sc.retryAt ?? '2026-11-07T10:09:00Z');
      else
        out = fxFound(kind, {
          projects: raw?.projects as string[] | undefined,
          handover: raw?.handover as string | undefined,
        });
      if (method === 'PUT') {
        if (out.result !== 'found') return json({ error: 'check failed', code: 'check_failed', check: out }, 422);
        site = fxSiteRow({
          kind,
          jira_url: out.jira_url,
          projects: JSON.stringify(raw?.projects ?? []),
          spaces: JSON.stringify(raw?.spaces ?? []),
          handover: raw?.handover === 'jql' ? 'either' : (raw?.handover ?? 'label'),
          team_id: raw?.team_id ?? null,
          comment_progress: !!raw?.comment_progress,
          wait_for_ok: !!raw?.wait_for_ok,
          jql: raw?.jql ?? null,
          // Names come from this save's check, never from the body (api-decisions 23).
          project_names: fxNames((out as { projects?: unknown }).projects),
          space_names: fxNames((out as { confluence?: { spaces?: unknown } | null }).confluence?.spaces),
        });
        return json({ site, check: out });
      }
      return json(out);
    }
    if (path.endsWith('/dev/jira') && method === 'DELETE') {
      site = null;
      return json({ deleted: true });
    }
    if (path.endsWith('/dev/jira/on')) return json({ on: !!raw?.on });
    if (path.endsWith('/dev/jira/poll'))
      return json(
        site && site !== 'missing'
          ? { matched: 4, new: 0, waiting: 2, resumed: 0, taken_back: 0, closed: 0, failed: 0, started: 0, replies: 0, skipped: null, error: null }
          : { error: 'Jira isn’t set up in this space.', code: 'no_site' },
        site && site !== 'missing' ? 200 : 404,
      );
    return json({ error: 'not in the fixture' }, 404);
  };
}
