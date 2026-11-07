// 0.7.6 "Work from Jira": the setup sheet's data, its API calls and the badge rule (Pixel).
//
// All Jira calls the UI makes are here, in one small module, so the sheet, the settings row and the
// Dev desk badge never build a URL themselves. Shapes follow Juno's api.md §2 and the server as
// written on 2026-11-07 (src/db/dev_jira.rs JiraSiteRow, src/server/dev/atlassian/check.rs
// CheckOutcome); where they differ from design.md §12 the PR lists it.
//
// The token goes from the field straight into the request body. It is never stored here, never put in
// the store, a URL, a log or an error message.
//
// "Off means unchanged": with no Jira site (or a server without the routes) every helper below returns
// what 0.7.5 had: no ticket count, the violet badge, the old row text.

import { personFetch } from '../auth';
import { type Ticket, parseTicketList, strList, ticketsNeedCount } from './tickets';

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const str = (v: unknown, max = 500): string | null => (typeof v === 'string' && v.trim() !== '' ? v.slice(0, max) : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const bool = (v: unknown, fallback = false): boolean => (typeof v === 'boolean' ? v : fallback);

/** The Dev desk hint, once hidden, stays hidden in this space (design §1 "hasn't been dismissed"). Settings is the way back. */
export const hintHiddenKey = (space: string) => `kwnt:jira-hint-hidden:${space}`;
export const jiraHint = {
  hidden(space: string): boolean {
    try {
      return localStorage.getItem(hintHiddenKey(space)) === '1';
    } catch {
      return false;
    }
  },
  hide(space: string): void {
    try {
      localStorage.setItem(hintHiddenKey(space), '1');
    } catch {
      // storage off: it hides for this view only
    }
  },
};

/** The hand-over label when none is set (Juno's backend default). */
export const DEFAULT_LABEL = 'kwant';

// ---------------------------------------------------------------------------------------------------
// Copy (design.md §1, §2, §7). Kept here so tests and the components read the same words.

export const JIRA_COPY = {
  title: 'Take work from Jira',
  sub: 'Hand a ticket to your bots in Jira. They read it, ask what’s unclear, and build it. You stay in charge: Jira only changes when you say so.',
  hint: 'Use Jira? Your bots can take tickets you hand them.',
  hintButton: 'Take work from Jira',
  hintHide: 'Hide',
  hintHideLabel: 'Hide the Jira hint in this space',
  hintHidden: 'Hidden. Set it up any time in Settings › Dev mode.',
  /** The 0.7.4 "Before you save the token" card, with the one Jira sentence (design §2, spec change 8). */
  saveFirstWhy: 'Some bots here run commands without asking, so one of them could read a token saved on this server. A scoped token that can only read Jira and Confluence keeps that risk small.',
  cloudHelp: 'Make one in your Atlassian account, under Security › API tokens. Your bots see what you can see in Jira.',
  dcHelp: 'In Jira, open your profile › Personal Access Tokens. Your bots see what you can see in Jira.',
  nothingSaved: 'Nothing is saved until the check passes.',
  checking: 'Checking with Jira…',
  notJira: 'No Jira answered at this address. Check the spelling, or ask your Jira admin for it.',
  handoverHelp: 'The label keeps the real owner as assignee. Anyone who can edit the ticket can hand it over; removing the label takes it back.',
  waitNote: 'They’ll wait for your Start. Tickets handed over from now on start on their own.',
  confluence: 'Pages the ticket links to or mentions. Every line in the brief names the page and its version.',
  confluenceCloud: 'Uses the same sign-in as Jira.',
  confluenceDc: 'Data Center keeps Confluence separate, so it needs its own token.',
  comment: 'Off: Jira only changes when you post a question there yourself. Your bots never move or close tickets.',
  advancedHelp: 'Built from your choices above. Edit it and the choices above show “Set by your search”.',
  laterNote: 'Change any of this later in Dev mode settings.',
  settingsBlurb: 'Hand Jira tickets to your bots. They read the ticket and Confluence, ask what’s unclear, then build.',
} as const;

/** Step 4's "Wait for my OK" help, naming the lead when the team has one (shots-review 19). */
export const waitForOkText = (lead?: string | null): string => `Off: ${lead || 'the lead'} writes the brief, asks if something is unclear, then builds. On: you OK every brief first.`;

/** Settings' Token row: where it lives, plus Juno's verified scope note when there is one (api-decisions 6). */
export const tokenLine = (store: 'local' | 'bitwarden' | null, scopeNote: string | null): string =>
  `${store === 'bitwarden' ? 'Saved in Bitwarden' : 'Saved on this server'}${scopeNote ? ` · ${scopeNote}` : ''}`;

export const tokenRejected = (kind: JiraKind, email: string | null): string =>
  `Jira didn’t accept this token. Check it was copied whole${kind === 'cloud' && email ? ` and belongs to ${email}` : ''}. Nothing was saved.`;
export const noProjectsText = (email: string | null): string => `This token can’t see any projects. Ask a Jira admin to give ${email || 'you'} access to the project, then tap Check now.`;
export const rateLimitedText = (retryAt: string | null, fmt: (iso: string) => string = hhmm): string =>
  retryAt ? `Jira asked us to slow down. Trying again at ${fmt(retryAt)}.` : 'Jira asked us to slow down. Trying again in a minute.';
export const matchLine = (n: number): string => (n === 0 ? 'No open tickets match yet.' : `${n} open ${n === 1 ? 'ticket matches' : 'tickets match'} right now`);
export const zeroMatchHelp = (label: string): string => `Add the label ${label} to one in Jira to try it.`;

export const hhmm = (iso: string): string => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
};

// ---------------------------------------------------------------------------------------------------
// The address: Cloud is known from the host; anything else is asked of the server (a check with no token
// runs only the type detection: `detect: true`, api-decisions.md §8).

export type JiraKind = 'cloud' | 'dc';
export type Handover = 'label' | 'assignee' | 'jql';

/** `https://host[:port][/path]` without a trailing slash, or null when it can't be a Jira address. */
export function normaliseJiraUrl(raw: string): string | null {
  let s = raw.trim();
  if (!s || /\s/.test(s)) return null;
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(s)) s = `https://${s}`;
  let u: URL;
  try {
    u = new URL(s);
  } catch {
    return null;
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') return null;
  if (!u.hostname.includes('.') && u.hostname !== 'localhost') return null;
  if (u.username || u.password) return null;
  // Keep a Data Center context path (/jira) but drop a pasted page (/browse/KEY-1, /secure/Dashboard.jspa, …).
  const path = u.pathname.replace(/\/(browse|secure|projects|issues|jira\/(software|servicedesk|core)|plugins|rest)(\/.*)?$/i, '').replace(/\/+$/, '');
  return `${u.protocol}//${u.host}${path}`;
}

/** The host shown in chips and the collapsed step ("yourcompany.atlassian.net"). */
export const jiraHost = (url: string | null): string => {
  if (!url) return '';
  try {
    const u = new URL(url);
    return `${u.host}${u.pathname.replace(/\/+$/, '')}`;
  } catch {
    return url;
  }
};

/** Cloud by host alone (`*.atlassian.net`, `*.jira.com`); null = ask the server. */
export function kindFromHost(url: string | null): JiraKind | null {
  if (!url) return null;
  try {
    const h = new URL(url).hostname.toLowerCase();
    return h.endsWith('.atlassian.net') || h.endsWith('.jira.com') ? 'cloud' : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------------------------------
// The check (POST jira/check) and its answer

export type CheckResult = 'found' | 'detected' | 'auth_failed' | 'not_jira' | 'unreachable' | 'rate_limited' | 'no_projects' | 'invalid';
const CHECK_RESULTS: readonly CheckResult[] = ['found', 'detected', 'auth_failed', 'not_jira', 'unreachable', 'rate_limited', 'no_projects', 'invalid'];

export interface JiraProject {
  key: string;
  name: string;
}
export interface ConfluenceSpace {
  id: string | null;
  key: string;
  name: string;
}
export type ConfluenceResult = 'ok' | 'auth_failed' | 'unreachable' | 'needs_token' | 'off';

export interface CheckOutcome {
  result: CheckResult;
  message: string | null;
  kind: JiraKind | null;
  jira_url: string | null;
  confluence_url: string | null;
  auth: 'basic' | 'scoped' | 'pat' | null;
  me: { id: string | null; display_name: string | null } | null;
  projects: JiraProject[];
  confluence: { result: ConfluenceResult; spaces: ConfluenceSpace[] } | null;
  jql: string | null;
  jql_error: string | null;
  count: number | null;
  sample: { key: string; summary: string; url: string | null }[];
  retry_at: string | null;
  reads: string[];
  scope_note: string | null;
  suggested_project: { key: string; branches: number } | null;
}

const KEY = /^[A-Z][A-Z0-9_]{0,29}$/;

export function parseCheck(v: unknown): CheckOutcome {
  const o = isObj(v) ? v : {};
  const kind = o.kind === 'cloud' || o.kind === 'dc' ? o.kind : null;
  const auth = o.auth === 'basic' || o.auth === 'scoped' || o.auth === 'pat' ? o.auth : null;
  const projects = (Array.isArray(o.projects) ? o.projects : [])
    .map((p) => (isObj(p) && str(p.key, 40) && KEY.test(String(p.key)) ? { key: String(p.key), name: str(p.name, 120) ?? String(p.key) } : null))
    .filter((p): p is JiraProject => !!p)
    .slice(0, 200);
  const c = isObj(o.confluence) ? o.confluence : null;
  const confluence = c
    ? {
        result: (['ok', 'auth_failed', 'unreachable', 'needs_token', 'off'] as const).includes(c.result as ConfluenceResult) ? (c.result as ConfluenceResult) : 'off',
        spaces: (Array.isArray(c.spaces) ? c.spaces : [])
          .map((s) =>
            isObj(s) && str(s.key, 60)
              ? {
                  id: str(s.id, 60),
                  key: String(s.key),
                  name: str(s.name, 120) ?? String(s.key),
                }
              : null,
          )
          .filter((s): s is ConfluenceSpace => !!s)
          .slice(0, 200),
      }
    : null;
  const me = isObj(o.me) ? { id: str(o.me.id, 200), display_name: str(o.me.display_name, 120) } : null;
  const sp =
    isObj(o.suggested_project) && str(o.suggested_project.key, 40)
      ? {
          key: String(o.suggested_project.key),
          branches: num(o.suggested_project.branches) ?? 0,
        }
      : null;
  return {
    result: CHECK_RESULTS.includes(o.result as CheckResult) ? (o.result as CheckResult) : 'invalid',
    message: str(o.message, 400),
    kind,
    jira_url: str(o.jira_url, 300),
    confluence_url: str(o.confluence_url, 300),
    auth,
    me,
    projects,
    confluence,
    jql: str(o.jql, 2000),
    jql_error: str(o.jql_error, 400),
    count: num(o.count),
    sample: (Array.isArray(o.sample) ? o.sample : [])
      .map((s) =>
        isObj(s) && str(s.key, 40)
          ? {
              key: String(s.key),
              summary: str(s.summary, 200) ?? '',
              url: str(s.url, 400),
            }
          : null,
      )
      .filter((s): s is { key: string; summary: string; url: string | null } => !!s)
      .slice(0, 3),
    retry_at: str(o.retry_at, 40),
    reads: strList(o.reads, 4),
    scope_note: str(o.scope_note, 120),
    suggested_project: sp && projects.some((p) => p.key === sp.key) ? sp : null,
  };
}

/** Which step-1 error to show, or null. The server's message is used for `invalid` only. */
export type Step1Error = {
  field: 'address' | 'token' | 'email' | 'confluence' | null;
  text: string;
};
export function step1Error(c: CheckOutcome, kind: JiraKind, email: string | null): Step1Error | null {
  switch (c.result) {
    case 'found':
    case 'detected':
    case 'no_projects':
    case 'rate_limited':
      return null;
    case 'auth_failed':
      return { field: 'token', text: tokenRejected(kind, email) };
    case 'not_jira':
    case 'unreachable':
      return { field: 'address', text: JIRA_COPY.notJira };
    case 'invalid': {
      const m = c.message ?? 'Check the fields and try again.';
      const field = /email/i.test(m) ? 'email' : /token/i.test(m) ? 'token' : /confluence/i.test(m) ? 'confluence' : /address|url/i.test(m) ? 'address' : null;
      return {
        field,
        text: m.charAt(0).toUpperCase() + m.slice(1) + (/[.!?]$/.test(m) ? '' : '.'),
      };
    }
  }
}

// ---------------------------------------------------------------------------------------------------
// The JQL the sheet shows under Advanced (the same rule as api.md §2.4; the server builds its own).

const q = (s: string) => `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
const bare = (s: string) => (/^[A-Za-z0-9_]+$/.test(s) ? s : q(s));

export function builtJql(c: { projects: string[]; handover: Handover; label: string; assignee_id: string | null }): string {
  const ps = c.projects.length === 1 ? `project = ${c.projects[0]}` : `project in (${c.projects.join(', ')})`;
  const who = c.handover === 'assignee' ? (c.assignee_id ? `assignee = ${q(c.assignee_id)}` : 'assignee = …') : `labels = ${bare(c.label || DEFAULT_LABEL)}`;
  return `${ps} AND ${who} AND statusCategory != Done ORDER BY created ASC`;
}

// ---------------------------------------------------------------------------------------------------
// The saved site (GET jira → {site, tickets}). The row is serialized as stored: `projects` and `spaces`
// are JSON strings, `handover` is `either` for the Advanced rule, and the on flag is `on_`.

export interface JiraSite {
  kind: JiraKind;
  jira_url: string;
  confluence_url: string | null;
  email: string | null;
  auth: 'basic' | 'scoped' | 'pat';
  me_name: string | null;
  projects: string[];
  handover: Handover;
  assignee_id: string | null;
  assignee_name: string | null;
  label: string;
  jql: string | null;
  team_id: string | null;
  spaces: string[];
  comment_progress: boolean;
  wait_for_ok: boolean;
  poll_secs: number;
  on: boolean;
  polled_at: string | null;
  poll_error: string | null;
  backoff_until: string | null;
  token_rejected_at: string | null;
  /** "scoped: reads Jira and Confluence" when Juno's check verified a scoped token; null otherwise. */
  scope_note: string | null;
  /** Names next to keys, from the server (api-decisions 23): `{}` until listed. A key without a name shows alone. */
  project_names: Record<string, string>;
  space_names: Record<string, string>;
}

/** `{KEY: name}` from the server: untrusted text, so strings only, capped. */
const nameMap = (v: unknown): Record<string, string> => {
  const out: Record<string, string> = {};
  if (!isObj(v)) return out;
  for (const [k, n] of Object.entries(v).slice(0, 100)) {
    const name = str(n, 120);
    if (/^[A-Za-z0-9_~-]{1,100}$/.test(k) && name) out[k] = name;
  }
  return out;
};

export function parseSite(v: unknown): JiraSite | null {
  if (!isObj(v)) return null;
  const jira_url = str(v.jira_url, 300);
  if (!jira_url) return null;
  const h = v.handover === 'assignee' ? 'assignee' : v.handover === 'either' || v.handover === 'jql' ? 'jql' : 'label';
  return {
    kind: v.kind === 'dc' ? 'dc' : 'cloud',
    jira_url,
    confluence_url: str(v.confluence_url, 300),
    email: str(v.email, 254),
    auth: v.auth === 'scoped' ? 'scoped' : v.auth === 'pat' ? 'pat' : 'basic',
    me_name: str(v.me_name, 120),
    projects: strList(v.projects, 20),
    handover: h,
    assignee_id: str(v.assignee_id, 200),
    assignee_name: str(v.assignee_name, 120),
    label: str(v.label, 100) ?? DEFAULT_LABEL,
    jql: str(v.jql, 2000),
    team_id: str(v.team_id, 120),
    spaces: strList(v.spaces, 20),
    comment_progress: bool(v.comment_progress),
    wait_for_ok: bool(v.wait_for_ok),
    poll_secs: num(v.poll_secs) ?? 300,
    on: bool(v.on ?? v.on_, true),
    polled_at: str(v.polled_at, 40),
    poll_error: str(v.poll_error, 400),
    backoff_until: str(v.backoff_until, 40),
    token_rejected_at: str(v.token_rejected_at, 40),
    scope_note: str(v.scope_note, 120),
    project_names: nameMap(v.project_names),
    space_names: nameMap(v.space_names),
  };
}

// ---------------------------------------------------------------------------------------------------
// API (all person-only on the server)

export class JiraError extends Error {
  constructor(
    message: string,
    public status: number,
    public code: string | null = null,
    public check: CheckOutcome | null = null,
  ) {
    super(message);
  }
}

type Send = (url: string, init?: RequestInit) => Promise<Response>;
let send: Send = (url, init) => personFetch(url, init);
/** Tests and the screenshot harness swap the transport; the app never calls this. */
export const setJiraTransport = (s: Send | null): void => {
  send = s ?? ((url, init) => personFetch(url, init));
};

async function call(path: string, method = 'GET', body?: unknown): Promise<{ status: number; data: unknown }> {
  let res: Response;
  try {
    res = await send(path, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new JiraError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const data: unknown = await res.json().catch(() => null);
  if (res.ok) return { status: res.status, data };
  const o = isObj(data) ? data : {};
  const code = str(o.code, 60);
  if (res.status === 423 || o.locked === true) throw new JiraError('Bitwarden is locked. Unlock it to save the token there.', 423, 'locked');
  if (res.status === 422 && isObj(o.check)) throw new JiraError('The check didn’t pass.', 422, code ?? 'check_failed', parseCheck(o.check));
  if (res.status === 403) throw new JiraError('Only the person can connect Jira.', 403, code);
  const err = str(o.error, 400);
  throw new JiraError(err ?? `Something went wrong (HTTP ${res.status}).`, res.status, code);
}

const base = (space: string) => `/api/spaces/${encodeURIComponent(space)}/dev`;

/** What the sheet sends to check or save. `token` is left out to reuse the saved one (same address only). */
export interface JiraForm {
  jira_url: string;
  kind?: JiraKind;
  email?: string;
  token?: string;
  confluence_url?: string;
  confluence_token?: string;
  projects?: string[];
  handover?: Handover;
  assignee_id?: string | null;
  label?: string;
  jql?: string | null;
}
export interface JiraSettings {
  spaces: string[];
  team_id: string | null;
  assignee_key?: string | null;
  assignee_name?: string | null;
  comment_progress: boolean;
  wait_for_ok: boolean;
  poll_secs: number;
  accept_risk?: boolean;
}

/** Drops empty strings and undefined so a blank field never overwrites a saved value. */
const clean = (o: Record<string, unknown>) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== ''));

export interface JiraUser {
  id: string;
  name: string | null;
  display_name: string;
}

export const jiraApi = {
  /** null site = never connected. A server without the routes answers 404, which throws: the store then
   *  leaves the space unknown, so no hint and no settings row show there (off means unchanged). */
  async get(space: string): Promise<{ site: JiraSite | null; count: number }> {
    const r = await call(`${base(space)}/jira`);
    const o = isObj(r.data) ? r.data : {};
    return { site: parseSite(o.site), count: num(o.tickets) ?? 0 };
  },
  async tickets(space: string): Promise<Ticket[]> {
    const r = await call(`${base(space)}/tickets`);
    return parseTicketList(r.data).tickets;
  },
  async check(space: string, form: JiraForm): Promise<CheckOutcome> {
    const r = await call(`${base(space)}/jira/check`, 'POST', clean({ ...form }));
    return parseCheck(r.data);
  },
  /** The token-less type detection (api-decisions §8): `{result: "detected", kind}` or not_jira / unreachable / rate_limited. */
  async detect(space: string, jira_url: string): Promise<CheckOutcome> {
    const r = await call(`${base(space)}/jira/check`, 'POST', { jira_url, detect: true });
    return parseCheck(r.data);
  },
  async save(space: string, form: JiraForm, settings: JiraSettings): Promise<{ site: JiraSite | null; check: CheckOutcome }> {
    // The server owns the names (api-decisions 23): never send them back.
    const { project_names: _p, space_names: _s, ...body } = { ...form, ...settings } as JiraForm & JiraSettings & { project_names?: unknown; space_names?: unknown };
    const r = await call(`${base(space)}/jira`, 'PUT', clean(body));
    const o = isObj(r.data) ? r.data : {};
    return { site: parseSite(o.site), check: parseCheck(o.check) };
  },
  async remove(space: string, forgetToken = true): Promise<boolean> {
    const r = await call(`${base(space)}/jira?forget_token=${forgetToken}`, 'DELETE');
    return isObj(r.data) && r.data.deleted === true;
  },
  /** Check now (J4): one pickup pass. Answers the PollReport's `error` or `skipped`, or null when it ran. */
  async poll(space: string): Promise<string | null> {
    const r = await call(`${base(space)}/jira/poll`, 'POST');
    const o = isObj(r.data) ? r.data : {};
    return str(o.error, 400) ?? (str(o.skipped, 40) ? `Skipped: ${String(o.skipped).replace(/_/g, ' ')}.` : null);
  },
  async setOn(space: string, on: boolean): Promise<boolean> {
    const r = await call(`${base(space)}/jira/on`, 'POST', { on });
    return isObj(r.data) ? bool(r.data.on, on) : on;
  },
  async users(space: string, query: string): Promise<JiraUser[]> {
    const r = await call(`${base(space)}/jira/users?q=${encodeURIComponent(query)}`);
    const o = isObj(r.data) ? r.data : {};
    return (Array.isArray(o.users) ? o.users : [])
      .map((u) =>
        isObj(u) && str(u.id, 200)
          ? {
              id: String(u.id),
              name: str(u.name, 120),
              display_name: str(u.display_name, 120) ?? String(u.id),
            }
          : null,
      )
      .filter((u): u is JiraUser => !!u)
      .slice(0, 20);
  },
};

// ---------------------------------------------------------------------------------------------------
// The store: one GET jira per space, tickets only when a site exists.

export interface JiraState {
  site: JiraSite | null;
  tickets: Ticket[];
}
type Listener = () => void;
const states = new Map<string, JiraState>();
const loading = new Set<string>();
const listeners = new Set<Listener>();

export const jiraStore = {
  get: (space: string | null | undefined): JiraState | null => (space ? (states.get(space) ?? null) : null),
  set(space: string, s: JiraState) {
    states.set(space, s);
    listeners.forEach((l) => l());
  },
  /** Loads once (or again with `force`). Errors leave the space unknown, which reads as "off". */
  async load(space: string, force = false): Promise<void> {
    if (!force && (states.has(space) || loading.has(space))) return;
    loading.add(space);
    try {
      const { site } = await jiraApi.get(space);
      const tickets = site ? await jiraApi.tickets(space).catch(() => states.get(space)?.tickets ?? []) : [];
      jiraStore.set(space, { site, tickets });
    } catch {
      /* unknown = off: nothing changes on screen */
    } finally {
      loading.delete(space);
    }
  },
  subscribe(l: Listener) {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
  /** Tests only. */
  _reset() {
    states.clear();
    loading.clear();
  },
};

/** Sent after the sheet saved (or settings turned it off): `detail.space`. */
export const JIRA_CHANGED_EVENT = 'kwnt:jira-changed';

// ---------------------------------------------------------------------------------------------------
// The badge rule (design §7): it counts needs, not news.

export interface DeskBadge {
  count: number;
  /** Tickets waiting on the person (questions, Brief ready). Waiting backfill cards never count. */
  tickets: number;
  reviews: number;
  /** Amber when a ticket waits on you, else the existing violet. */
  tone: 'needs' | 'accent';
  /** The row's aria-label, or undefined at 0 (the 0.7.5 behaviour). */
  label: string | undefined;
  /** The row subtitle's lead ("1 ticket needs you"), or null with no ticket needing you. */
  lead: string | null;
}

export function deskBadge(reviews: number, tickets: readonly Pick<Ticket, 'state'>[] | null): DeskBadge {
  // Iris's rule: needs_answer and brief_ready only. Waiting (backfill), picked up, reading and working don't count.
  const t = tickets ? ticketsNeedCount([...tickets]) : 0;
  const count = t + reviews;
  const lead = t > 0 ? `${t} ${t === 1 ? 'ticket needs' : 'tickets need'} you` : null;
  const rv = reviews > 0 ? `${reviews} ${reviews === 1 ? 'review' : 'reviews'} open` : null;
  const label = count === 0 ? undefined : t === 0 ? `Dev desk, ${reviews} new ${reviews === 1 ? 'review' : 'reviews'}` : ['Dev desk', lead, rv].filter(Boolean).join(', ');
  return {
    count,
    tickets: t,
    reviews,
    tone: t > 0 ? 'needs' : 'accent',
    label,
    lead,
  };
}

/** Settings' status line ("1 ticket needs you, 1 being built, 2 waiting for your Start"), from GET tickets. */
export function ticketSummary(tickets: readonly Pick<Ticket, 'state'>[]): string {
  const n = (f: (s: string) => boolean) => tickets.filter((t) => f(t.state)).length;
  const needs = n((s) => s === 'needs_answer' || s === 'brief_ready');
  const built = n((s) => s === 'picked_up' || s === 'reading' || s === 'working' || s === 'in_review');
  const waiting = n((s) => s === 'waiting');
  const parts = [needs ? `${needs} ${needs === 1 ? 'ticket needs' : 'tickets need'} you` : null, built ? `${built} being built` : null, waiting ? `${waiting} waiting for your Start` : null].filter(
    Boolean,
  );
  return parts.length ? parts.join(', ') : 'No tickets handed over yet';
}
