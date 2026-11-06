// Dev mode (0.7): the switch's safety cover, the detected repos (0.7.1), the per-space store and the
// API client. Person-only on the server: a bot's token gets 403, so the switch shows disabled.
// Contract: dev-mode/plan/plan.md §3.2 and §3.6, the route JSON in dev-mode/impl/lin.md.

import { personFetch } from './auth';

// ---------------------------------------------------------------------------
// Server shapes

export type DeployHint = 'docker' | 'cloudflare_pages' | 'fly' | null;

/** GET/PUT /api/spaces/:id/dev_mode */
export interface DevModeState {
  on: boolean;
  repo: string | null;
  branch: string | null;
  deploy_hint: DeployHint;
  backup_command: string | null;
  last_seen_sha: string | null;
  polled_at: string | null;
  poll_error: string | null;
  calls_today: number | null;
  calls_cap: number | null;
  /** 0.7.1: which forge reads the repo ("github" in 0.7.1; gitlab and gitea from 0.7.2), or null: no Post button. */
  forge: Forge | null;
  /** 0.7.1: that forge connection's login, for "Posts as {login}." on review cards. */
  forge_login: string | null;
  /** 0.7.1 (any-git S4): the repos found in the space's teams, in team order. [] on an older server. */
  sources: DevSource[];
  /** 0.7.1: what Dev mode can read right now. null on an older server (then the desk shows the repo chip). */
  status: DevStatus | null;
  /** 0.7.1 (flow F6): review cards new since the person last opened the Dev desk. null on an older server. */
  new_reviews: number | null;
  /** 0.7.2 (team repos, T8): every repo Dev mode watches or found, in team order. On an older server it is
   * built from the deprecated `repo`/`branch` (and `sources`), so the desk reads one shape. */
  repos: DevRepo[];
}

/** 0.7.2: one repo of the space, from a team (its repo link or folder) or added by hand. */
export type RepoStatus = 'watching' | 'not_connected' | 'other_host' | 'error' | 'too_many';

export interface DevRepo {
  /** The dedupe key, `host/owner/name` lowercased. Cards carry it as `repo`; `?repo=` filters by it. */
  key: string;
  /** What the person reads: `owner/name` on GitHub, else the URL without userinfo. */
  repo: string;
  host: string | null;
  /** The team it came from; null for a hand-added repo. */
  team: string | null;
  /** That team's id (api.md `team_id`), for matching the space's teams; null on an older server. */
  team_id: string | null;
  /** The team's repo link, its folder's remote, a folder with no remote, or typed by the person. */
  from: SourceFrom | 'hand' | null;
  branch: string | null;
  status: RepoStatus;
  poll_error: string | null;
  polled_at: string | null;
  last_tag: string | null;
  /** Only `hand` rows can be removed. null: no row (other_host, too_many). */
  added_by: 'team' | 'hand' | null;
}

/** Where a space's repo was found: a team's repo link, a git folder with a remote, or a folder with no remote. */
export type SourceFrom = 'repo' | 'folder' | 'local';

/** One detected repo. Team name only: the server never sends folder paths. */
export interface DevSource {
  team: string;
  repo: string;
  host: string | null;
  from: SourceFrom;
}

export type DevStatus = 'watching' | 'no_repo' | 'not_connected' | 'other_host';

export type CheckState = 'found' | 'not_found' | 'not_connected';

/** POST /api/spaces/:id/dev_mode/check */
export interface RepoCheck {
  state: CheckState;
  repo: string | null;
  branch: string | null;
  deploy_hint: DeployHint;
}

const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v : null);
export type Forge = 'github' | 'gitlab' | 'gitea';
export const forgeOf = (v: unknown): Forge | null => (v === 'github' || v === 'gitlab' || v === 'gitea' ? v : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const STATUSES: readonly DevStatus[] = ['watching', 'no_repo', 'not_connected', 'other_host'];

/** Drops userinfo from a URL-looking repo (`https://user:token@host/…`), so a token never reaches the screen. */
export const stripUserinfo = (repo: string): string => repo.replace(/^([a-z][a-z0-9+.-]*:\/\/)[^/@\s]*@/i, '$1');

export function parseSources(v: unknown): DevSource[] {
  if (!Array.isArray(v)) return [];
  const out: DevSource[] = [];
  for (const x of v) {
    if (!x || typeof x !== 'object') continue;
    const o = x as Record<string, unknown>;
    const team = str(o.team);
    const repo = str(o.repo);
    const from = o.from === 'repo' || o.from === 'folder' || o.from === 'local' ? o.from : null;
    if (!team || !repo || !from) continue;
    out.push({ team, repo: stripUserinfo(repo), host: str(o.host), from });
  }
  return out;
}

const REPO_STATUSES: readonly RepoStatus[] = ['watching', 'not_connected', 'other_host', 'error', 'too_many'];

/** A key for a repo the server sent no key for (an older server): `host/owner/name`, lowercased. */
export const repoKey = (repo: string, host: string | null = 'github.com'): string => {
  const r = stripUserinfo(repo)
    .replace(/^[a-z][a-z0-9+.-]*:\/\//i, '')
    .replace(/^[\w.-]+@([\w.-]+):/, '$1/')
    .replace(/\.git$/i, '')
    .replace(/\/+$/, '');
  return (/^[^/]+\.[^/]+\//.test(r) ? r : `${host ?? 'local'}/${r}`).toLowerCase();
};

export function parseRepos(v: unknown): DevRepo[] {
  if (!Array.isArray(v)) return [];
  const out: DevRepo[] = [];
  const seen = new Set<string>();
  for (const x of v) {
    if (!x || typeof x !== 'object') continue;
    const o = x as Record<string, unknown>;
    const repoRaw = str(o.repo);
    const status = REPO_STATUSES.includes(o.status as RepoStatus) ? (o.status as RepoStatus) : null;
    if (!repoRaw || !status) continue;
    const repo = stripUserinfo(repoRaw);
    const host = str(o.host);
    const key = (str(o.key) ?? repoKey(repo, host)).toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    const from = o.from === 'repo' || o.from === 'folder' || o.from === 'local' || o.from === 'hand' ? o.from : null;
    const team = str(o.team);
    out.push({
      key,
      repo,
      host,
      team,
      team_id: str(o.team_id),
      from,
      branch: str(o.branch),
      status,
      poll_error: str(o.poll_error),
      polled_at: str(o.polled_at),
      last_tag: str(o.last_tag),
      // A 0.7.1 row migrated without a team reads as hand (api.md T2).
      added_by: o.added_by === 'hand' || o.added_by === 'migrated' || (from === 'hand' && o.added_by !== 'team') ? 'hand' : o.added_by === 'team' ? 'team' : null,
    });
  }
  return out;
}

/** An older server (0.7.1) has no `repos`: one row from the deprecated `repo`/`branch`, or the found source. */
function legacyRepos(o: Record<string, unknown>, sources: DevSource[], status: DevStatus | null): DevRepo[] {
  const r = str(o.repo);
  const repo = r ? stripUserinfo(r) : null;
  const src = repo ? sources.find((x) => x.repo.toLowerCase() === repo.toLowerCase()) ?? null : status === 'not_connected' || status === 'other_host' ? sources.find((x) => status !== 'other_host' || (x.host && x.host !== 'github.com')) ?? sources[0] ?? null : null;
  const name = repo ?? src?.repo ?? null;
  if (!name) return [];
  const st: RepoStatus = status === 'not_connected' || status === 'other_host' ? status : src?.from === 'local' ? 'other_host' : 'watching';
  return [
    {
      key: repoKey(name, src?.host ?? 'github.com'),
      repo: name,
      host: src?.host ?? null,
      team: src?.team ?? null,
      team_id: null,
      from: src?.from ?? null,
      branch: str(o.branch),
      status: st,
      poll_error: str(o.poll_error),
      polled_at: str(o.polled_at),
      last_tag: null,
      // An older server has no DELETE route: its typed repo has no Remove.
      added_by: src ? 'team' : null,
    },
  ];
}

const hint = (v: unknown): DeployHint => (v === 'docker' || v === 'cloudflare_pages' || v === 'fly' ? v : null);

export function parseDevMode(raw: unknown): DevModeState | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  if (typeof o.on !== 'boolean') return null;
  const sources = parseSources(o.sources);
  const status = STATUSES.includes(o.status as DevStatus) ? (o.status as DevStatus) : null;
  return {
    on: o.on,
    repo: (() => {
      const r = str(o.repo);
      return r ? stripUserinfo(r) : null;
    })(),
    branch: str(o.branch),
    deploy_hint: hint(o.deploy_hint),
    backup_command: str(o.backup_command),
    last_seen_sha: str(o.last_seen_sha),
    polled_at: str(o.polled_at),
    poll_error: str(o.poll_error),
    calls_today: num(o.calls_today),
    calls_cap: num(o.calls_cap),
    forge: forgeOf(o.forge),
    forge_login: str(o.forge_login),
    sources,
    status,
    new_reviews: (() => {
      const n = num(o.new_reviews);
      return n === null ? null : Math.max(0, Math.floor(n));
    })(),
    repos: Array.isArray(o.repos) ? parseRepos(o.repos) : legacyRepos(o, sources, status),
  };
}

/** Why a typed repo wasn't watched, from the PUT reply's `check`. Dev mode is on either way. */
export function putRefusal(c: RepoCheck | null, asked: string): string {
  const repo = stripUserinfo(c?.repo ?? asked);
  if (c?.state === 'not_connected') return `Connect GitHub so bots can read ${repo}. Dev mode is on, with the floor and guards.`;
  return `Couldn’t read ${repo}. Check the name, or that your GitHub connection can see it. Dev mode is on, with the floor and guards.`;
}

export function parseCheck(raw: unknown): RepoCheck | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  if (o.state !== 'found' && o.state !== 'not_found' && o.state !== 'not_connected') return null;
  return { state: o.state, repo: str(o.repo), branch: str(o.branch), deploy_hint: hint(o.deploy_hint) };
}

/** A team of the space, for "Link a repo" on a team that has none. */
export interface DevTeamRef {
  id: string;
  name: string;
}

export type RowAction = 'connect' | 'link' | 'remove';
export type Led = 'ok' | 'warn' | 'bad' | 'off';

/** One row of the Dev desk's repo list: a repo, or a team with no repo yet. */
export interface RepoRow {
  /** The repo key, or `team:{id}` for a team with none. */
  id: string;
  team: string | null;
  teamId: string | null;
  repo: DevRepo | null;
  /** The row's status words (Mira's spec §6). */
  state: string;
  /** The row's second line: "From {team}'s repo link", "From {team}'s folder" or "Added by you". */
  origin: string | null;
  /** A team's repo can't be removed here: the info note says where to change it. */
  teamNote: string | null;
  /** Tooltip for an error row: the poll error, URLs without their userinfo. */
  title: string | null;
  led: Led;
  /** The row's one action, or null. */
  action: RowAction | null;
}

/** "19:58" in the person's locale (24 h when the locale uses it). */
export const checkedTime = (iso: string | null): string | null => {
  const t = iso ? Date.parse(iso) : NaN;
  return Number.isFinite(t) ? new Date(t).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }) : null;
};

/** A poll error for a tooltip: never a URL with a token in it. */
export const safeError = (e: string | null): string | null => (e ? e.replace(/([a-z][a-z0-9+.-]*:\/\/)[^\s/@]+@/gi, '$1') : null);

export const TOO_MANY = 'Not watched: 10 repos is the limit';
export const NEEDS_GITHUB = 'Needs your GitHub connection';
export const CANT_READ = 'Can’t read it right now. Trying again.';
export const LOCAL_ONLY = 'Local folder, no remote';
export const hostNotRead = (host: string | null) => `${host ?? 'This host'} isn’t read yet`;
export const teamNoteText = (team: string, from: DevRepo['from']) =>
  from === 'folder' ? `Comes from ${team}’s folder. Change it in the team’s settings.` : `Comes from ${team}’s repo link. Change it in the team’s settings.`;
export const removedText = (repo: string) => `Stopped watching ${repo}. Its cards stay.`;
export const LINK_HELP = 'Saved as this team’s repo link. Bots only read it.';
export const LINK_HELP_HAND = 'Added by you. Bots only read it.';
export const LINK_ERROR = 'Couldn’t find that repo. Check the name, or connect GitHub first.';
export const LINK_PLACEHOLDER = 'owner/name or its URL';

const rowState = (r: DevRepo): { state: string; led: Led } => {
  if (r.from === 'local') return { state: LOCAL_ONLY, led: 'off' };
  switch (r.status) {
    case 'watching': {
      const t = checkedTime(r.polled_at);
      return { state: t ? `Watching · checked ${t}` : 'Watching', led: 'ok' };
    }
    case 'not_connected':
      return { state: NEEDS_GITHUB, led: 'warn' };
    case 'other_host':
      return { state: hostNotRead(r.host), led: 'off' };
    case 'error':
      return { state: CANT_READ, led: 'bad' };
    default:
      return { state: TOO_MANY, led: 'off' };
  }
};

const rowOrigin = (r: DevRepo): string | null =>
  r.added_by === 'hand' ? 'Added by you' : r.team && r.from === 'repo' ? `From ${r.team}’s repo link` : r.team && r.from === 'folder' ? `From ${r.team}’s folder` : null;

/** A row's one action: Connect GitHub when the repo needs it, else Remove for a hand-added repo. */
const rowAction = (r: DevRepo): RowAction | null => (r.status === 'not_connected' ? 'connect' : r.added_by === 'hand' ? 'remove' : null);

/** Does this team bring a repo (a watched one, or one found in its folder)? */
const teamHasRepo = (s: DevModeState, t: DevTeamRef) => s.repos.some((r) => r.team_id === t.id || r.team === t.name) || s.sources.some((x) => x.team === t.name);

/** The space's teams with no repo at all. */
export const teamsWithoutRepo = (s: DevModeState | null, teams: DevTeamRef[] = []): DevTeamRef[] => (s ? teams.filter((t) => !teamHasRepo(s, t)) : []);

const repoRow = (r: DevRepo, teams: DevTeamRef[]): RepoRow => ({
  id: r.key,
  team: r.team,
  teamId: r.team_id ?? teams.find((t) => t.name === r.team)?.id ?? null,
  repo: r,
  ...rowState(r),
  origin: rowOrigin(r),
  teamNote: r.added_by !== 'hand' && r.team && (r.from === 'repo' || r.from === 'folder') ? teamNoteText(r.team, r.from) : null,
  title: r.status === 'error' ? safeError(r.poll_error) : null,
  action: rowAction(r),
});

const noRepoRow = (t: DevTeamRef): RepoRow => ({ id: `team:${t.id}`, team: t.name, teamId: t.id, repo: null, state: 'No repo', origin: null, teamNote: null, title: null, led: 'off', action: 'link' });

/**
 * The Dev desk's repo list (0.7.2, spec §6 and §9b): one row per repo in team order. Teams with no repo
 * get their own "No repo" row only while no repo is watched at all; otherwise they fold into one quiet line.
 */
export function repoRows(s: DevModeState | null, teams: DevTeamRef[] = []): RepoRow[] {
  if (!s) return [];
  const rows = s.repos.map((r) => repoRow(r, teams));
  if (!s.repos.some((r) => r.status === 'watching')) rows.push(...teamsWithoutRepo(s, teams).map(noRepoRow));
  return rows;
}

/** Past this many rows the list folds behind "Watching N repos". */
export const LIST_OPEN_MAX = 3;

export interface StatusLine {
  status: DevStatus | RepoStatus;
  /** Plain text for the line; `{repo}` parts are marked so the view can set them in mono. */
  parts: Array<{ text: string; mono?: boolean; branch?: boolean }>;
  /** The line's one action (a single row): Connect GitHub, Link a repo, or Remove. */
  action: RowAction | null;
  led: Led;
  /** Two rows or more: the list under the line. */
  rows: RepoRow[];
  /** More than LIST_OPEN_MAX rows: the list starts folded behind the line. */
  collapsed: boolean;
  /** §9b: teams with no repo while some repo is watched, folded into one quiet last line. */
  noRepoTeams: DevTeamRef[];
}

const NO_REPO = 'No git repo in this space’s folders yet. The floor and guards are on.';

/** Repos that need the person: not connected or failing (spec §9b: teams with no repo never count). */
export const needsYouCount = (s: DevModeState) => s.repos.filter((r) => r.status === 'not_connected' || r.status === 'error').length;

/** The summary over several repos: "Watching 4 repos", or "Watching 3 of 4 repos · 1 needs you". */
export function reposSummary(s: DevModeState): string {
  const total = s.repos.length;
  const n = s.repos.filter((r) => r.status === 'watching' && r.from !== 'local').length;
  if (!total) return 'No repos yet';
  const need = needsYouCount(s);
  const head = n === total ? `Watching ${n} ${n === 1 ? 'repo' : 'repos'}` : `Watching ${n} of ${total} repos`;
  return need ? `${head} · ${need} ${need === 1 ? 'needs' : 'need'} you` : head;
}

/** "1 team has no repo", "3 teams have no repo". */
export const noRepoTeamsText = (n: number) => (n === 1 ? '1 team has no repo' : `${n} teams have no repo`);

/** One repo as a single line, as in 0.7.1: "Watching demo-org/app · main". */
function singleLine(r: DevRepo): Omit<StatusLine, 'rows' | 'collapsed' | 'noRepoTeams'> {
  const { state, led } = rowState(r);
  const action = rowAction(r);
  if (r.status === 'watching' && r.from !== 'local')
    return { status: 'watching', led, action, parts: [{ text: 'Watching ' }, { text: r.repo, mono: true }, { text: ' · ' }, { text: r.branch || 'main', mono: true, branch: true }] };
  return { status: r.status, led, action, parts: [{ text: r.repo, mono: true }, { text: ` · ${state}` }] };
}

/** The Dev desk's status: one line for one repo (as in 0.7.1), a list for several. null on a server older than 0.7.1. */
export function statusLine(s: DevModeState | null, teams: DevTeamRef[] = []): StatusLine | null {
  if (!s || !s.status) return null;
  const rows = repoRows(s, teams);
  const noRepoTeams = s.repos.some((r) => r.status === 'watching') ? teamsWithoutRepo(s, teams) : [];
  // GitHub isn't connected and no repo was read yet: Connect first, as in 0.7.1.
  if (rows.length === 0 && s.status === 'not_connected')
    return { status: 'not_connected', parts: [{ text: 'Reviews need your GitHub connection.' }], action: 'connect', led: 'warn', rows: [], collapsed: false, noRepoTeams };
  if (rows.length === 0) return { status: 'no_repo', parts: [{ text: NO_REPO }], action: 'link', led: 'off', rows: [], collapsed: false, noRepoTeams };
  if (rows.length === 1) {
    const r = rows[0];
    if (!r.repo) return { status: 'no_repo', parts: [{ text: NO_REPO }], action: 'link', led: 'off', rows: [], collapsed: false, noRepoTeams };
    return { ...singleLine(r.repo), rows: [], collapsed: false, noRepoTeams };
  }
  const led: Led = rows.some((r) => r.led === 'bad') ? 'bad' : rows.some((r) => r.led === 'warn') ? 'warn' : rows.some((r) => r.led === 'ok') ? 'ok' : 'off';
  return { status: s.status, parts: [{ text: reposSummary(s) }], action: null, led, rows, collapsed: rows.length > LIST_OPEN_MAX, noRepoTeams };
}

/** The toast after the switch turns Dev mode on (flow F3, Mira's flow copy §3). `action` is the
 * status's one fix; watching, other_host and "checking" offer Open Dev desk. Undo is always there. */
export interface OnToast {
  /** "Dev mode on", or "Dev mode on · guards are up" when reviews can't run yet. */
  title: string;
  /** Line 2; `mono` parts are a repo, branch or host. */
  body: Array<{ text: string; mono?: boolean }>;
  action: 'open' | 'connect' | 'type';
  /** The response came before detection finished (no status yet): the same toast updates in place. */
  checking: boolean;
}

const ON = 'Dev mode on';
const GUARDS = 'Dev mode on · guards are up';

export function onToast(s: DevModeState | null): OnToast {
  if (!s?.status) return { title: ON, body: [{ text: 'Looking for a git repo in your teams’ folders…' }], action: 'open', checking: true };
  switch (s.status) {
    case 'watching': {
      const watched = s.repos.filter((r) => r.status === 'watching');
      const first = watched[0] ?? s.repos.find((r) => r.from === 'local') ?? null;
      if (watched.length > 1) return { title: ON, body: [{ text: `${reposSummary(s)}.` }], action: 'open', checking: false };
      if (first?.from === 'local')
        return { title: GUARDS, body: [{ text: `${first.team ?? 'This team'}’s folder has a local repo with no remote. The floor works now.` }], action: 'open', checking: false };
      return {
        title: ON,
        body: [{ text: 'Watching ' }, { text: first?.repo ?? s.repo ?? '', mono: true }, { text: ' on ' }, { text: first?.branch || s.branch || 'main', mono: true }, { text: '.' }],
        action: 'open',
        checking: false,
      };
    }
    case 'not_connected': {
      const repo = s.repos.find((r) => r.status === 'not_connected')?.repo ?? s.repo ?? s.sources[0]?.repo;
      return {
        title: GUARDS,
        body: repo ? [{ text: 'Found ' }, { text: repo, mono: true }, { text: '. Connect GitHub to get reviews.' }] : [{ text: 'Connect GitHub to get reviews.' }],
        action: 'connect',
        checking: false,
      };
    }
    case 'other_host': {
      const host = s.repos.find((r) => r.status === 'other_host' && r.host)?.host ?? s.sources.find((x) => x.host && x.host !== 'github.com')?.host;
      return {
        title: GUARDS,
        body: host ? [{ text: host, mono: true }, { text: ' repos are read from the next update. The floor works now.' }] : [{ text: 'Repos on this host are read from the next update. The floor works now.' }],
        action: 'open',
        checking: false,
      };
    }
    default:
      return { title: GUARDS, body: [{ text: 'No git repo in this space’s folders yet. The floor works now.' }], action: 'type', checking: false };
  }
}

// ---------------------------------------------------------------------------
// Repo chips and filters on the cards (0.7.2)

/** The short name on a filter chip: the repo's last part ("app"), or owner/name when two share it. */
export function repoFilters(s: DevModeState | null): Array<{ key: string; label: string; repo: string }> {
  const list = (s?.repos ?? []).filter((r) => r.status !== 'too_many' && r.status !== 'other_host');
  if (list.length < 2) return [];
  const short = (r: DevRepo) => r.repo.replace(/\/+$/, '').split('/').pop() || r.repo;
  const counts = new Map<string, number>();
  for (const r of list) counts.set(short(r).toLowerCase(), (counts.get(short(r).toLowerCase()) ?? 0) + 1);
  return list.map((r) => ({ key: r.key, repo: r.repo, label: (counts.get(short(r).toLowerCase()) ?? 0) > 1 ? r.repo.replace(/^[a-z][a-z0-9+.-]*:\/\/[^/]+\//i, '') : short(r) }));
}

/** The chip on a card: its repo's `owner/name`, only when the space has more than one repo.
 *  The card's own `repo_name` wins (api.md: a removed repo keeps its label). */
export function cardRepoLabel(s: DevModeState | null, key: unknown, name?: unknown): string | null {
  if (!s || s.repos.length < 2 || typeof key !== 'string' || !key.trim()) return null;
  if (typeof name === 'string' && name.trim()) return stripUserinfo(name.trim());
  return cardRepoFull(s, key);
}

/** spec §6: the chip says the repo name only ("app"); its title is owner/name. */
export const shortRepo = (full: string) => full.replace(/\/+$/, '').split('/').pop() || full;

function cardRepoFull(s: DevModeState, key: string): string {
  const k = key.toLowerCase();
  const r = s.repos.find((x) => x.key === k);
  if (r) return r.repo;
  // A card from a repo no team points at any more keeps its label: the key without its host.
  const parts = stripUserinfo(key).split('/');
  return parts.length > 2 && parts[0].includes('.') ? parts.slice(1).join('/') : key;
}

/** Cards of one repo; an older server that ignores `?repo=` still shows only that repo's cards. */
export const cardsOfRepo = <T extends object>(cards: T[], key: string | null): T[] =>
  key
    ? cards.filter((c) => {
        const r = (c as { repo?: unknown }).repo;
        return typeof r !== 'string' || r.toLowerCase() === key;
      })
    : cards;

export const onToastText = (t: OnToast) => `${t.title}. ${t.body.map((p) => p.text).join('')}`;

/** A typed repo (Link a repo): a GitHub `owner/name` or URL, or any git URL with its userinfo dropped. */
export function typedRepo(input: string): string | null {
  const gh = normaliseRepo(input);
  if (gh) return gh;
  const s = stripUserinfo(input.trim());
  if (/^(?:https?|ssh|git):\/\/[^\s/]+\/\S+$/i.test(s) || /^[\w.-]+@[\w.-]+:\S+$/.test(s)) return s.replace(/\/+$/, '');
  return null;
}

// ---------------------------------------------------------------------------
// Repo input: a URL, `owner/name` or `git@github.com:owner/name.git`, normalised to `owner/name`

const PART = /^[A-Za-z0-9_.-]{1,100}$/;

export function normaliseRepo(input: string): string | null {
  let s = input.trim();
  if (!s) return null;
  s = s.replace(/^git@github\.com:/i, '');
  s = s.replace(/^(?:https?:\/\/|git:\/\/|ssh:\/\/git@)?(?:www\.)?github\.com\//i, '');
  s = s.replace(/[?#].*$/, '').replace(/\/+$/, '').replace(/\.git$/i, '');
  const [owner, name, ...rest] = s.split('/');
  // github.com/owner/name/tree/main and /pull/3 still name the repo
  if (!owner || !name || (rest.length && !['tree', 'blob', 'pull', 'pulls', 'issues', 'actions', 'commit', 'commits'].includes(rest[0]))) return null;
  if (!PART.test(owner) || !PART.test(name) || name === '.' || name === '..') return null;
  return `${owner}/${name}`;
}

// ---------------------------------------------------------------------------
// The switch: a safety cover over a launch switch (design §1)

/** How long a lifted cover waits before it drops back with nothing changed. */
export const RECOVER_MS = 5000;
/** After a flip the cover drops back at 450 ms (150 ms drop), and presses are ignored until then. */
export const SETTLE_MS = 450;
export const DROP_MS = 150;

export type SwitchPhase = 'covered' | 'lifted' | 'settling';

export interface SwitchState {
  /** The committed value, as the server last said (or as just pressed, while saving). */
  on: boolean;
  phase: SwitchPhase;
  /** When the cover was lifted, for the 5 s re-cover. */
  liftedAt: number | null;
  /** Presses are ignored until this time (the settle after a flip). */
  settleUntil: number;
  /** Not the person (403), or the server doesn't have Dev mode. */
  disabled: boolean;
  /** Under prefers-reduced-motion every change is instant, so there is nothing to settle. */
  reduced: boolean;
  /** What the polite live region says next; seq changes on every announcement. */
  announce: { text: string; seq: number } | null;
  /** A save is in flight (the press already flipped the switch). */
  saving: boolean;
  error: string | null;
}

export type SwitchAction =
  | { type: 'press'; now: number }
  | { type: 'recover'; now: number }
  | { type: 'escape' }
  | { type: 'blur' }
  | { type: 'settled'; now: number }
  | { type: 'saved'; on: boolean }
  | { type: 'save_failed'; error: string }
  | { type: 'server'; state: DevModeState | null; disabled?: boolean }
  | { type: 'reduced'; reduced: boolean };

export const initialSwitch = (over: Partial<SwitchState> = {}): SwitchState => ({
  on: false,
  phase: 'covered',
  liftedAt: null,
  settleUntil: 0,
  disabled: false,
  reduced: false,
  announce: null,
  saving: false,
  error: null,
  ...over,
});

const say = (s: SwitchState, text: string) => ({ text, seq: (s.announce?.seq ?? 0) + 1 });

export const onText = (space: string) => `Dev mode on for ${space}.`;
export const offText = (space: string) => `Dev mode off for ${space}. Reviews, the ship gate and the production floor stopped.`;
/** Flow copy §2: the armed line, announced politely when the cover lifts. Only "on" can arm (F2). */
export const ARMED_TEXT = 'Press again to turn Dev mode off, or Escape to keep it on.';
/** Flow copy §2: the 5 s run out, Esc, or focus left. */
export const STAYS_ON_TEXT = 'Dev mode stays on.';
/** @deprecated kept for callers; arming only happens while on. */
export const liftText = (_on: boolean) => ARMED_TEXT;

/**
 * The switch's state machine. Pure: time comes in on the actions, and the component runs the timers
 * (re-cover at liftedAt + 5 s, settled at settleUntil). `space` names the space in announcements.
 */
export function switchReducer(s: SwitchState, a: SwitchAction, space = 'this space'): SwitchState {
  switch (a.type) {
    case 'press': {
      if (s.disabled || s.saving) return s;
      // A fast double-click: the second press lands while the cover settles, and is ignored.
      if (s.phase === 'settling' || a.now < s.settleUntil) return s;
      // Flow F1: on only adds guards, so off has no cover and one press turns it on.
      if (!s.on) return flip(s, true, a.now, space);
      // F2: off removes the guards, so it takes two presses: arm, then confirm within 5 s.
      if (s.phase === 'covered') return { ...s, phase: 'lifted', liftedAt: a.now, error: null, announce: say(s, ARMED_TEXT) };
      return flip(s, false, a.now, space);
    }
    case 'recover':
      if (s.phase !== 'lifted' || s.liftedAt === null || a.now - s.liftedAt < RECOVER_MS) return s;
      return { ...s, phase: 'covered', liftedAt: null, announce: say(s, STAYS_ON_TEXT) };
    case 'escape':
    case 'blur':
      if (s.phase !== 'lifted') return s;
      return { ...s, phase: 'covered', liftedAt: null, announce: say(s, STAYS_ON_TEXT) };
    case 'settled':
      if (s.phase !== 'settling' || a.now < s.settleUntil) return s;
      return { ...s, phase: 'covered' };
    case 'saved':
      return { ...s, saving: false, on: a.on };
    case 'save_failed':
      // The press didn't take: back to what it was, covered, with the reason.
      return { ...s, saving: false, on: !s.on, phase: 'covered', liftedAt: null, settleUntil: 0, error: a.error, announce: say(s, a.error) };
    case 'server': {
      const disabled = a.disabled ?? s.disabled;
      if (!a.state) return { ...s, disabled };
      // A save in flight, or a cover the person is holding open, keeps what they see.
      if (s.saving) return { ...s, disabled };
      return { ...s, disabled, on: a.state.on, phase: s.phase === 'lifted' && a.state.on !== s.on ? 'covered' : s.phase };
    }
    case 'reduced':
      return { ...s, reduced: a.reduced };
  }
}

function flip(s: SwitchState, on: boolean, now: number, space: string): SwitchState {
  return {
    ...s,
    on,
    saving: true,
    liftedAt: null,
    // Reduced motion: no hinge to settle, it locks at once. Still two presses to the next flip.
    phase: s.reduced ? 'covered' : 'settling',
    settleUntil: s.reduced ? 0 : now + SETTLE_MS,
    error: null,
    announce: say(s, on ? onText(space) : offText(space)),
  };
}

/** The visual state the switch draws: off, armed (first press), on, disabled. 0.7.1: no pending
 * look, since there is no setup sheet. Styles key off this (data-look), so a new look is CSS plus the glyph map. */
export type SwitchLookName = 'off' | 'armed' | 'on' | 'disabled';
/** The icon in the knob: the code mark at rest, an open lock when armed, a closed lock when disabled. */
export type SwitchGlyph = 'code' | 'unlocked' | 'locked';

export interface SwitchLook {
  look: SwitchLookName;
  glyph: SwitchGlyph;
  coverOpen: boolean;
  knobRight: boolean;
  fill: number;
  ring: boolean;
  hint: string | null;
  /** aria-label: role="switch" carries on/off in aria-checked, so this is just the name. */
  label: string;
  /** aria-describedby text (flow copy §1). */
  describe: string;
  tooltip: string;
}

export const OFF_TOOLTIP = 'Dev mode: plain-words merges, code reviews, production floor';
export const ON_TOOLTIP = 'Dev mode is on. Press twice to turn it off.';
export const OWNER_TOOLTIP = 'Only the space owner can change Dev mode.';
export const OFF_DESCRIBE = 'Turns on reviews, the ship gate and the production floor.';
export const ON_DESCRIBE = 'Press twice to turn off.';
export const ARMED_HINT = 'Press again to turn Dev mode off';

export function switchLook(s: SwitchState): SwitchLook {
  const base = switchParts(s);
  const look: SwitchLookName = s.disabled ? 'disabled' : s.phase === 'lifted' ? 'armed' : s.on ? 'on' : 'off';
  // The lock: shut while on and covered (guards locked in) or not the owner, open while armed.
  const glyph: SwitchGlyph = s.disabled ? 'locked' : s.phase === 'lifted' ? 'unlocked' : s.on ? 'locked' : 'code';
  return { look, glyph, ...base };
}

function switchParts(s: SwitchState): Omit<SwitchLook, 'look' | 'glyph'> {
  const tooltip = s.disabled ? OWNER_TOOLTIP : s.on ? ON_TOOLTIP : OFF_TOOLTIP;
  const describe = s.disabled ? OWNER_TOOLTIP : s.on ? ON_DESCRIBE : OFF_DESCRIBE;
  const common = { label: 'Dev mode', describe, tooltip };
  switch (s.phase) {
    case 'lifted':
      return { coverOpen: true, knobRight: true, fill: 1, ring: true, hint: ARMED_HINT, ...common };
    case 'settling':
      return { coverOpen: !s.on, knobRight: s.on, fill: s.on ? 1 : 0, ring: false, hint: null, ...common };
    default:
      return { coverOpen: false, knobRight: s.on, fill: s.on ? 1 : 0, ring: s.on && !s.disabled, hint: null, ...common };
  }
}

// ---------------------------------------------------------------------------
// Setup sheet: the Repository field (design §3 table)

export type FieldState = 'empty' | 'invalid' | 'checking' | 'found' | 'not_found' | 'not_connected' | 'error';

export interface FieldView {
  chip: { tone: 'info' | 'ok' | 'bad' | 'warn'; text: string; spinner?: boolean } | null;
  /** Plain help text, or null for the "filled in from this team's repo" default. */
  help: string | null;
  /** The repo name shown bold inside the not-found help line. */
  helpRepo: string | null;
  connect: boolean;
  canTurnOn: boolean;
}

export const DEFAULT_HELP = 'Filled in from this team’s repo. Read with your GitHub connection from Loose ends; nothing is written.';
export const EMPTY_HELP = 'Paste a GitHub repo, like owner/name or its URL.';

export function fieldView(state: FieldState, repo: string | null, error: string | null = null): FieldView {
  switch (state) {
    case 'checking':
      return { chip: { tone: 'info', text: 'Checking', spinner: true }, help: null, helpRepo: null, connect: false, canTurnOn: false };
    case 'found':
      return { chip: { tone: 'ok', text: 'Found' }, help: DEFAULT_HELP, helpRepo: null, connect: false, canTurnOn: true };
    case 'not_found':
      return {
        chip: { tone: 'bad', text: 'Can’t see it' },
        help: 'Bots can’t see {repo} with your GitHub connection. Check the name, or give Loose ends access to it.',
        helpRepo: repo,
        connect: false,
        canTurnOn: false,
      };
    case 'not_connected':
      return { chip: { tone: 'warn', text: 'Not connected' }, help: 'Connect GitHub to read this repo. Bots only read it.', helpRepo: null, connect: true, canTurnOn: false };
    case 'invalid':
      return { chip: { tone: 'bad', text: 'Not a repo' }, help: EMPTY_HELP, helpRepo: null, connect: false, canTurnOn: false };
    case 'error':
      return { chip: { tone: 'warn', text: 'Can’t check' }, help: error ?? 'GitHub can’t be reached right now. Try again in a minute.', helpRepo: null, connect: false, canTurnOn: false };
    default:
      return { chip: null, help: EMPTY_HELP, helpRepo: null, connect: false, canTurnOn: false };
  }
}

export const CHECK_DEBOUNCE_MS = 400;

export interface CheckerTimers {
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (h: unknown) => void;
}

export interface CheckerUpdate {
  state: FieldState;
  check: RepoCheck | null;
  error: string | null;
}

/**
 * The Repository field's checker: each input waits 400 ms, then asks the server. Only the latest
 * input's answer counts, so a slow reply to an earlier name never overwrites a newer one.
 */
export function createRepoChecker(
  check: (repo: string) => Promise<RepoCheck>,
  onUpdate: (u: CheckerUpdate) => void,
  timers: CheckerTimers = { setTimeout: (f, ms) => setTimeout(f, ms), clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) },
  delay = CHECK_DEBOUNCE_MS,
) {
  let handle: unknown = null;
  let seq = 0;
  return {
    input(raw: string) {
      const mine = ++seq;
      if (handle !== null) timers.clearTimeout(handle);
      handle = null;
      if (!raw.trim()) return onUpdate({ state: 'empty', check: null, error: null });
      const repo = normaliseRepo(raw);
      if (!repo) return onUpdate({ state: 'invalid', check: null, error: null });
      onUpdate({ state: 'checking', check: null, error: null });
      handle = timers.setTimeout(() => {
        handle = null;
        check(repo).then(
          (c) => mine === seq && onUpdate({ state: c.state, check: c, error: null }),
          (e: unknown) => mine === seq && onUpdate({ state: 'error', check: null, error: e instanceof Error ? e.message : null }),
        );
      }, delay);
    },
    cancel() {
      seq++;
      if (handle !== null) timers.clearTimeout(handle);
      handle = null;
    },
  };
}

export const deployLabel = (h: DeployHint): { name: string; detail: string } | null =>
  h === 'docker' ? { name: 'Docker', detail: 'compose file' } : h === 'cloudflare_pages' ? { name: 'Cloudflare Pages', detail: 'static site' } : h === 'fly' ? { name: 'Fly.io', detail: 'fly.toml' } : null;

export const approvalsWord = (a: string | undefined) => (a === 'off' ? 'Off' : a === 'smart' ? 'Smart' : 'Ask');

// ---------------------------------------------------------------------------
// API

export class DevModeError extends Error {
  constructor(message: string, public status: number, public code: string | null = null) {
    super(message);
  }
}

async function call<T>(url: string, method = 'GET', body?: unknown, send: typeof fetch = (...a) => personFetch(a[0] as string, a[1])): Promise<{ status: number; data: T }> {
  let res: Response;
  try {
    res = await send(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new DevModeError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const data = await res.json().catch(() => null);
  if (res.ok) return { status: res.status, data: data as T };
  const err = data && typeof data === 'object' ? (data as Record<string, unknown>).error : null;
  if (res.status === 403) throw new DevModeError('Only the space owner can change Dev mode.', 403);
  if (res.status === 404 && !err) throw new DevModeError('This server doesn’t have Dev mode yet. Update the app.', 404);
  const code = data && typeof data === 'object' && typeof (data as Record<string, unknown>).code === 'string' ? ((data as Record<string, unknown>).code as string) : null;
  throw new DevModeError(typeof err === 'string' && err.trim() ? err : `Something went wrong (HTTP ${res.status}).`, res.status, code);
}

const enc = encodeURIComponent;

export const devApi = {
  async get(space: string): Promise<DevModeState> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode`);
    const s = parseDevMode(r.data);
    if (!s) throw new DevModeError('The server sent Dev mode in a shape this app doesn’t know.', 500);
    return s;
  },
  async put(space: string, body: { on: boolean; repo?: string | null; branch?: string | null; backup_command?: string | null }): Promise<DevModeState> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode`, 'PUT', body);
    const s = parseDevMode(r.data);
    if (!s) throw new DevModeError('The server sent Dev mode in a shape this app doesn’t know.', 500);
    return s;
  },
  /**
   * 0.7.2 (T8): `PUT {repo, team}` sets that team's repo link; `PUT {repo}` with no team adds a repo by
   * hand. A repo that can't be read still keeps Dev mode on, and the reply's `check` says why.
   */
  async linkRepo(space: string, body: { repo: string; team?: string | null }): Promise<{ state: DevModeState; refused: string | null }> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode`, 'PUT', { on: true, repo: body.repo, ...(body.team ? { team: body.team } : {}) });
    const s = parseDevMode(r.data);
    if (!s) throw new DevModeError('The server sent Dev mode in a shape this app doesn’t know.', 500);
    const c = parseCheck((r.data as Record<string, unknown>).check);
    return { state: s, refused: c && c.state !== 'found' ? putRefusal(c, body.repo) : null };
  },
  /** 0.7.2 (T8): removes a hand-added repo (409 for a team's repo: change the team instead). Returns the new state. */
  async removeRepo(space: string, key: string): Promise<DevModeState> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode/repos/${enc(key)}`, 'DELETE');
    const s = parseDevMode(r.data);
    if (s) return s;
    return devApi.get(space);
  },
  async check(space: string, repo: string): Promise<RepoCheck> {
    const r = await call<unknown>(`/api/spaces/${enc(space)}/dev_mode/check`, 'POST', { repo });
    const c = parseCheck(r.data);
    if (!c) throw new DevModeError('The server sent a repo check this app doesn’t know.', 500);
    return c;
  },
  landed: (space: string, before?: string, repo?: string | null) =>
    call<unknown>(`/api/spaces/${enc(space)}/dev/landed?limit=30${before ? `&before=${enc(before)}` : ''}${repo ? `&repo=${enc(repo)}` : ''}`).then((r) => r.data),
  seen: (space: string) => call<{ seen_at: string }>(`/api/spaces/${enc(space)}/dev/landed/seen`, 'POST').then((r) => r.data),
  /** Flow F6 (Juno): the person opened the Dev desk, so new_reviews goes back to 0 on every device. */
  deskSeen: (space: string) => call<unknown>(`/api/spaces/${enc(space)}/dev_mode/seen`, 'POST').then((r) => r.data),
  reviews: (space: string, repo?: string | null) => call<unknown>(`/api/spaces/${enc(space)}/dev/reviews${repo ? `?repo=${enc(repo)}` : ''}`).then((r) => r.data),
  finding: (card: string, fid: string, body: { action: string; reason?: string; note?: string }) =>
    call<unknown>(`/api/dev/cards/${enc(card)}/findings/${enc(fid)}`, 'POST', body).then((r) => r.data),
  /** 0.7.1 Post to {Forge}: one PR review from this card. 200 with the card, `post_error` set when the forge refused. */
  post: (card: string) => call<unknown>(`/api/dev/cards/${enc(card)}/post`, 'POST', {}).then((r) => r.data),
  /** Not in lin-dev-core 492019f yet (asked Lin): the ready release-notes card after its SSE. */
  card: (id: string) => call<unknown>(`/api/dev/cards/${enc(id)}`).then((r) => r.data),
  /** 0.7.2: `repo` drafts that repo's notes (since its newest tag); without it, the first watched repo's. */
  releaseNotes: (space: string, repo?: string | null) => call<unknown>(`/api/spaces/${enc(space)}/dev/release_notes${repo ? `?repo=${enc(repo)}` : ''}`, 'POST').then((r) => r.data),
  production: (connection: string, space: string, production: boolean) =>
    call<{ id: string; production: boolean }>(`/api/connections/${enc(connection)}/production?space=${enc(space)}`, 'PUT', { production }).then((r) => r.data),
};

export const _call = call;

/** One press from outside the switch (the toast's Undo, the off note's Turn back on): save, then the
 * store tells the switch, the desk and the badge. */
export async function turnDevMode(space: string, on: boolean, put: typeof devApi.put = devApi.put): Promise<DevModeState> {
  const s = await put(space, { on });
  devStore.set(space, s);
  return s;
}

/** What the desk should open with once it mounts: the Link a repo field, focused (the toast's "Type a repo",
 * the header's "Link a repo"), or the repo list expanded and in view (the header's "See repos"). */
export type DeskIntent = 'type' | 'repos';
const deskIntents = new Map<string, DeskIntent>();
export const deskIntent = {
  set: (space: string, what: DeskIntent) => void deskIntents.set(space, what),
  take(space: string): DeskIntent | null {
    const v = deskIntents.get(space) ?? null;
    deskIntents.delete(space);
    return v;
  },
};

/** Connect from the Dev desk or the toast: Loose ends opens on the GitHub form, and after a GitHub
 * source is added the app returns to this space's Dev desk (flow F5). */
let connectReturn: string | null = null;
export const devConnect = {
  start: (space: string) => void (connectReturn = space),
  pending: () => connectReturn,
  /** A source was added: the space to return to, once. */
  finish(): string | null {
    const v = connectReturn;
    connectReturn = null;
    return v;
  },
  cancel: () => void (connectReturn = null),
};

// First-visit marks, per space and per device (they only steer hints, never what is on).
const mark = (key: string): boolean => {
  try {
    if (localStorage.getItem(key)) return false;
    localStorage.setItem(key, '1');
    return true;
  } catch {
    return false;
  }
};
const marked = (key: string): boolean => {
  try {
    return !!localStorage.getItem(key);
  } catch {
    return true;
  }
};
export const firstRun = {
  /** True the first time the desk opens in this space; it stays shown until dismissed this visit. */
  claimIntro: (space: string) => mark(`kwnt.dev.intro.${space}`),
  introSeen: (space: string) => marked(`kwnt.dev.intro.${space}`),
  /** True the first time the Dev desk row shows in this space: one soft highlight. */
  claimRow: (space: string) => mark(`kwnt.dev.row.${space}`),
  rowSeen: (space: string) => marked(`kwnt.dev.row.${space}`),
};

// ---------------------------------------------------------------------------
// The per-space store: the header, rail badge, hard hats and grid all read it

type Listener = () => void;
const states = new Map<string, DevModeState>();
const listeners = new Set<Listener>();
let current: string | null = null;

export const devStore = {
  get: (space: string | null | undefined): DevModeState | null => (space ? states.get(space) ?? null : null),
  isOn: (space: string | null | undefined): boolean => !!(space && states.get(space)?.on),
  set(space: string, s: DevModeState) {
    states.set(space, s);
    listeners.forEach((l) => l());
  },
  /** The `dev_mode {space_id, on}` SSE event: flips the cached value at once, then the caller refetches. */
  setOn(space: string, on: boolean) {
    const s = states.get(space);
    if (s && s.on === on) return;
    states.set(space, { ...(s ?? emptyState()), on });
    listeners.forEach((l) => l());
  },
  setCurrent(space: string | null) {
    if (current === space) return;
    current = space;
    listeners.forEach((l) => l());
  },
  current: () => current,
  subscribe(l: Listener) {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
  /** Tests only. */
  _reset() {
    states.clear();
    current = null;
  },
};

export const emptyState = (): DevModeState => ({
  on: false,
  repo: null,
  branch: null,
  deploy_hint: null,
  backup_command: null,
  last_seen_sha: null,
  polled_at: null,
  poll_error: null,
  calls_today: null,
  calls_cap: null,
  forge: null,
  forge_login: null,
  sources: [],
  status: null,
  new_reviews: null,
  repos: [],
});

// ---------------------------------------------------------------------------
// Dev desk

/** "Today", "Yesterday", or "Tue 3 Nov", in the viewer's local time. */
export function dayLabel(iso: string, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(now) - day(d)) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  return d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });
}

/** The day separator goes on the first card of each day, newest first. */
export function withDayLabels<T extends { created_at?: string | null; merged_at?: string | null }>(cards: T[], now = new Date()): Array<{ card: T; dayLabel: string | null }> {
  let last = '';
  return cards.map((card) => {
    const label = dayLabel(card.merged_at ?? card.created_at ?? '', now);
    const show = label && label !== last ? label : null;
    if (label) last = label;
    return { card, dayLabel: show };
  });
}

/** The Dev desk row's status: "3 landed · 1 review open". */
export function deskStatus(unseenLanded: number, openReviews: number): string {
  const parts = [];
  parts.push(`${unseenLanded} landed`);
  parts.push(`${openReviews} ${openReviews === 1 ? 'review' : 'reviews'} open`);
  return parts.join(' · ');
}
