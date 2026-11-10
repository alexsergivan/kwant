// 0.9.0 H1 + P3: the Dev desk's Worktrees tab. Types and client for Lin's routes (release-090/h1-contract.md
// v2, PR #486), the pure copy helpers the tab and the tests share (Mira's designs/h1-worktrees.md), and one
// cache per space. Until #486 merges the routes may 404: the tab then says the server doesn't have them yet.

import { personFetch } from '../auth';

export type WorktreeState = 'live' | 'missing' | 'leftover' | 'removed';

export interface WorktreeChange {
  status: string;
  path: string;
}

export interface Worktree {
  id: string;
  team_id: string | null;
  team: string | null;
  repo: string;
  folder: string;
  path: string;
  branch: string;
  base: string | null;
  made_branch: boolean;
  head: string | null;
  dirty: number | null;
  changes: WorktreeChange[];
  ahead: number | null;
  behind: number | null;
  state: WorktreeState;
  ours: boolean;
  working: boolean;
  thread_id: string | null;
  task_id: string | null;
  kwant_id: string | null;
  task_title: string | null;
  /** The chat the worktree is bound to (contract v3); null without a thread. */
  thread_title: string | null;
  created_at: string | null;
  done_at: string | null;
  clean_at: string | null;
  keep_build: boolean;
  cleaned_at: string | null;
  cleaned: string[];
  cleaned_bytes: number | null;
  kept: { dir: string; why: string }[];
  removed_at: string | null;
}

/** A linked worktree git lists that kwant didn't make: read-only on the desk. */
export interface OtherWorktree {
  folder_name: string;
  path: string;
  branch: string | null;
  head: string | null;
  missing: boolean;
}

export interface WorktreeRepo {
  repo: string;
  team_id: string | null;
  team: string | null;
  checkout: string | null;
  default_branch: string | null;
  error: boolean;
  worktrees: Worktree[];
  other: OtherWorktree[];
}

export interface WorktreeList {
  repos: WorktreeRepo[];
  clean_after_minutes: number;
  build_dirs: string[];
}

/** The create dialog's branch pickers (Lin, contract v3; optional until the route exists).
 *  A remote name carries its remote: `origin/foo`. */
export interface BranchInfo {
  name: string;
  remote: boolean;
  checked_out_at: string | null;
}

// ---------------------------------------------------------------------------
// Parsing: the server's shape, defensively (an older or newer server never crashes the tab).

const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const rec = (v: unknown): Record<string, unknown> | null => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

export function parseWorktree(v: unknown): Worktree | null {
  const o = rec(v);
  const id = str(o?.id);
  const path = str(o?.path);
  const branch = str(o?.branch);
  if (!o || !id || !path || !branch) return null;
  const state = (['live', 'missing', 'leftover', 'removed'] as const).find((s) => s === o.state) ?? 'live';
  return {
    id,
    team_id: str(o.team_id),
    team: str(o.team),
    repo: str(o.repo) ?? '',
    folder: str(o.folder) ?? '',
    path,
    branch,
    base: str(o.base),
    made_branch: o.made_branch === true,
    head: str(o.head),
    dirty: num(o.dirty),
    changes: arr(o.changes).flatMap((c) => {
      const r = rec(c);
      const p = str(r?.path);
      return p ? [{ status: str(r?.status) ?? '?', path: p }] : [];
    }),
    ahead: num(o.ahead),
    behind: num(o.behind),
    state,
    ours: o.ours !== false,
    working: o.working === true,
    thread_id: str(o.thread_id),
    task_id: str(o.task_id),
    kwant_id: str(o.kwant_id),
    task_title: str(o.task_title),
    thread_title: str(o.thread_title),
    created_at: str(o.created_at),
    done_at: str(o.done_at),
    clean_at: str(o.clean_at),
    keep_build: o.keep_build === true,
    cleaned_at: str(o.cleaned_at),
    cleaned: arr(o.cleaned).filter((d): d is string => typeof d === 'string'),
    cleaned_bytes: num(o.cleaned_bytes),
    kept: arr(o.kept).flatMap((k) => {
      const r = rec(k);
      const dir = str(r?.dir);
      return dir ? [{ dir, why: str(r?.why) ?? '' }] : [];
    }),
    removed_at: str(o.removed_at),
  };
}

export function parseList(v: unknown): WorktreeList | null {
  const o = rec(v);
  if (!o || !Array.isArray(o.repos)) return null;
  return {
    repos: o.repos.flatMap((r) => {
      const g = rec(r);
      const repo = str(g?.repo);
      if (!g || !repo) return [];
      return [
        {
          repo,
          team_id: str(g.team_id),
          team: str(g.team),
          checkout: str(g.checkout),
          default_branch: str(g.default_branch),
          error: g.error === true,
          worktrees: arr(g.worktrees)
            .map(parseWorktree)
            .filter((w): w is Worktree => !!w && w.state !== 'removed'),
          other: arr(g.other).flatMap((x) => {
            const q = rec(x);
            const p = str(q?.path);
            return p ? [{ folder_name: str(q?.folder_name) ?? p.split('/').pop() ?? p, path: p, branch: str(q?.branch), head: str(q?.head), missing: q?.missing === true }] : [];
          }),
        },
      ];
    }),
    clean_after_minutes: num(o.clean_after_minutes) ?? 60,
    build_dirs: arr(o.build_dirs).filter((d): d is string => typeof d === 'string'),
  };
}

// ---------------------------------------------------------------------------
// Client

export class WorktreeError extends Error {
  constructor(
    message: string,
    public status: number,
    public code: string | null = null,
    public body: Record<string, unknown> | null = null,
  ) {
    super(message);
  }
}

async function call(url: string, method = 'GET', body?: unknown): Promise<unknown> {
  let res: Response;
  try {
    res = await personFetch(url, {
      method,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new WorktreeError('Couldn’t reach the server. Check your connection and try again.', 0);
  }
  const data = await res.json().catch(() => null);
  if (res.ok) return data;
  const o = rec(data);
  const code = str(o?.code);
  const err = str(o?.error);
  if (res.status === 404 && !code) throw new WorktreeError('This server doesn’t have worktrees yet. Update the app.', 404, 'no_route', o);
  throw new WorktreeError(err ?? `Something went wrong (HTTP ${res.status}).`, res.status, code, o);
}

const enc = encodeURIComponent;

export interface CreateBody {
  team_id: string;
  branch: string;
  base?: string;
  thread_id?: string;
  kwant_id?: string;
  task_id?: string;
}

export const worktreeApi = {
  list: async (space: string): Promise<WorktreeList> => {
    const l = parseList(await call(`/api/spaces/${enc(space)}/dev/worktrees`));
    if (!l) throw new WorktreeError('The server sent worktrees in a shape this app doesn’t know.', 500);
    return l;
  },
  /** Contract v3 (Lin, coming to #486). A 404 means "type the branch name". */
  branches: async (space: string, team: string): Promise<BranchInfo[]> => {
    const o = rec(await call(`/api/spaces/${enc(space)}/dev/worktrees/branches?team_id=${enc(team)}`));
    return arr(o?.branches).flatMap((b) => {
      const r = rec(b);
      const name = str(r?.name);
      return name ? [{ name, remote: r?.remote === true, checked_out_at: str(r?.checked_out_at) }] : [];
    });
  },
  create: async (space: string, body: CreateBody): Promise<Worktree> => {
    const w = parseWorktree(await call(`/api/spaces/${enc(space)}/dev/worktrees`, 'POST', body));
    if (!w) throw new WorktreeError('The server sent the worktree in a shape this app doesn’t know.', 500);
    return w;
  },
  remove: (id: string, force: boolean) => call(`/api/dev/worktrees/${enc(id)}?force=${force ? 'true' : 'false'}`, 'DELETE'),
  keep: async (id: string, keep: boolean) => parseWorktree(await call(`/api/dev/worktrees/${enc(id)}/keep`, 'POST', { keep })),
  clean: async (id: string) => parseWorktree(await call(`/api/dev/worktrees/${enc(id)}/clean`, 'POST', {})),
};

/** A remote-only `origin/foo` has no "existing branch" form in create: it goes in as a new local
 *  branch `foo` made from `origin/foo` (Lin, #486). Returns the local name. */
export const localNameOf = (remote: string): string => remote.replace(/^[^/]+\//, '');

/** The Existing picker: local branches, plus remote ones with no local branch of the same name. */
export function pickableBranches(all: BranchInfo[]): BranchInfo[] {
  const local = new Set(all.filter((b) => !b.remote).map((b) => b.name));
  return all.filter((b) => !b.remote || !local.has(localNameOf(b.name)));
}

/** The create request for what the dialog holds. */
export function createBodyFor(o: { team: string; mode: 'new' | 'existing'; name: string; base: string; pickedRemote: string | null; bot: string; thread: string | null }): CreateBody {
  const b: CreateBody =
    o.mode === 'new' ? { team_id: o.team, branch: o.name, base: o.base } : o.pickedRemote ? { team_id: o.team, branch: o.name, base: o.pickedRemote } : { team_id: o.team, branch: o.name };
  if (o.bot) b.kwant_id = o.bot;
  if (o.thread) b.thread_id = o.thread;
  return b;
}

// ---------------------------------------------------------------------------
// One cache per space; the tab count and the tab read it.

export interface WorktreesData {
  list: WorktreeList | null;
  loading: boolean;
  error: string | null;
  /** The server has no worktree routes yet (#486 not merged there): the tab stays hidden. */
  missing: boolean;
}

const EMPTY: WorktreesData = { list: null, loading: false, error: null, missing: false };
const cache = new Map<string, WorktreesData>();
const listeners = new Set<() => void>();
const inflight = new Map<string, Promise<void>>();

function put(space: string, patch: Partial<WorktreesData>) {
  cache.set(space, { ...(cache.get(space) ?? EMPTY), ...patch });
  listeners.forEach((l) => l());
}

export const worktreeStore = {
  get: (space: string): WorktreesData => cache.get(space) ?? EMPTY,
  subscribe(l: () => void) {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
  load(space: string): Promise<void> {
    const running = inflight.get(space);
    if (running) return running;
    put(space, { loading: true });
    const p = worktreeApi
      .list(space)
      .then((list) => put(space, { list, loading: false, error: null, missing: false }))
      .catch((e: unknown) => {
        const gone = e instanceof WorktreeError && (e.code === 'no_route' || e.code === 'off');
        put(space, { loading: false, missing: gone, error: gone ? null : e instanceof Error ? e.message : 'Couldn’t read the worktrees.' });
      })
      .finally(() => inflight.delete(space));
    inflight.set(space, p);
    return p;
  },
  /** One worktree changed (keep, clean): swap it in place. */
  patch(space: string, w: Worktree) {
    const l = cache.get(space)?.list;
    if (!l) return;
    put(space, { list: { ...l, repos: l.repos.map((r) => ({ ...r, worktrees: r.worktrees.map((x) => (x.id === w.id ? w : x)) })) } });
  },
  _set: (space: string, patch: Partial<WorktreesData>) => put(space, patch),
  _reset() {
    cache.clear();
    inflight.clear();
  },
};

/** The tab count: linked worktrees (kwant's and git's others), never the main checkout. */
export const worktreeCount = (l: WorktreeList | null): number => (l ? l.repos.reduce((n, r) => n + r.worktrees.length + r.other.length, 0) : 0);

// ---------------------------------------------------------------------------
// Copy (designs/h1-worktrees.md "Copy (all)")

const pad = (n: number) => String(n).padStart(2, '0');
export const hhmm = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;
const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const dayStart = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** "14:05" today, "yesterday", "Mon" within a week, else "3 Nov". */
export function dayLabel(iso: string, now: number): { day: string; time: string; today: boolean } {
  const d = new Date(iso);
  const days = Math.round((dayStart(new Date(now)) - dayStart(d)) / 86_400_000);
  const time = hhmm(d);
  if (days <= 0) return { day: '', time, today: true };
  if (days === 1) return { day: 'yesterday', time, today: false };
  if (days < 7) return { day: DAYS[d.getDay()], time, today: false };
  return { day: `${d.getDate()} ${d.toLocaleString('en', { month: 'short' })}`, time, today: false };
}

export type Led = 'run' | 'ok' | 'off' | 'warn';

export interface RowStatus {
  led: Led;
  /** The words after the name (or the name's replacement for missing and leftover). */
  text: string;
  kind: 'working' | 'finished' | 'mine' | 'outside' | 'missing' | 'leftover' | 'leftover_foreign';
}

export function rowStatus(w: Worktree, now: number): RowStatus {
  if (w.state === 'missing') return { led: 'warn', text: 'Folder missing', kind: 'missing' };
  if (w.state === 'leftover') return { led: 'off', text: 'Leftover folder', kind: w.ours ? 'leftover' : 'leftover_foreign' };
  if (w.working) return { led: 'run', text: 'is working here', kind: 'working' };
  if (w.done_at) {
    const f = dayLabel(w.done_at, now);
    const fresh = now - new Date(w.done_at).getTime() < 86_400_000;
    return { led: fresh ? 'ok' : 'off', text: `finished ${f.today ? f.time : f.day}`, kind: 'finished' };
  }
  return { led: 'off', text: 'made by you', kind: 'mine' };
}

/** Line 3's time: "active now", "finished Mon 17:40", or when it was made. */
export function rowTime(w: Worktree, now: number): string | null {
  if (w.working) return 'active now';
  if (w.done_at) {
    const f = dayLabel(w.done_at, now);
    return `finished ${f.today ? f.time : `${f.day} ${f.time}`}`;
  }
  if (!w.created_at) return null;
  const c = dayLabel(w.created_at, now);
  return c.today ? c.time : `${c.day} ${c.time}`;
}

/** Line 2: the task, or how it was made. */
export function rowTask(w: Worktree, now: number): string {
  if (w.task_title) return w.task_title;
  if (w.thread_title) return `Chat: ${w.thread_title}`;
  if (!w.created_at) return 'No task.';
  const c = dayLabel(w.created_at, now);
  return `No task. Made from the Dev desk ${c.today ? c.time : `${c.day} ${c.time}`}`;
}

export const changesLabel = (dirty: number | null): string | null =>
  dirty === null ? null : dirty === 0 ? 'No uncommitted changes' : dirty === 1 ? '1 uncommitted file' : `${dirty} uncommitted files`;

export const upToDate = (w: { ahead: number | null; behind: number | null }) => w.ahead === 0 && w.behind === 0;

/** "1.2 GB", "412 MB", "88 KB": decimal units, one decimal under 10. */
export function sizeLabel(bytes: number): string {
  const units = ['bytes', 'KB', 'MB', 'GB', 'TB'];
  let v = bytes;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  if (i === 0) return `${Math.round(v)} bytes`;
  return `${v < 10 ? v.toFixed(1).replace(/\.0$/, '') : Math.round(v)} ${units[i]}`;
}

export type P3 =
  | { kind: 'due'; at: string; dirs: string[] }
  | { kind: 'kept' }
  | { kind: 'cleaning' }
  | { kind: 'cleaned'; size: string; at: string; dirs: string[] }
  | { kind: 'failed'; reason: string };

/** The P3 line under a finished worktree, or null (nothing to clean, or not finished). */
export function p3State(w: Worktree, opts: { cleaning?: boolean; failed?: string | null } = {}): P3 | null {
  if (w.state !== 'live' || !w.ours) return null;
  if (opts.cleaning) return { kind: 'cleaning' };
  if (opts.failed) return { kind: 'failed', reason: opts.failed };
  if (w.clean_at && !w.keep_build) return { kind: 'due', at: hhmm(new Date(w.clean_at)), dirs: [] };
  if (w.keep_build && w.done_at) return { kind: 'kept' };
  if (w.cleaned_at && w.cleaned.length && (w.cleaned_bytes ?? 0) > 0) {
    return { kind: 'cleaned', size: sizeLabel(w.cleaned_bytes ?? 0), at: hhmm(new Date(w.cleaned_at)), dirs: w.cleaned.map((d) => d.split('/').pop() ?? d).filter((d, i, a) => a.indexOf(d) === i) };
  }
  return null;
}

/** "· Kept dist: Git tracks a file in it." for each skipped folder, after a clean. */
export const keptNotes = (w: Worktree): string[] =>
  w.kept.map((k) => (k.why === 'tracked' ? `Kept ${k.dir}: Git tracks a file in it.` : k.why === 'not_ignored' ? `Kept ${k.dir}: Git doesn’t ignore it.` : `Kept ${k.dir}.`));

// ---------------------------------------------------------------------------
// Branch names (create dialog)

/** The folder slug, as Lin's server makes it (contract v2): lowercase, runs outside [a-z0-9_] → "-", 60 max. */
export function branchSlug(branch: string): string {
  const s = branch
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 60)
    .replace(/-+$/g, '');
  return s || 'worktree';
}

/** `git check-ref-format --branch`, close enough for the field; the server has the last word. */
export function branchProblem(name: string): { kind: 'spaces'; suggestion: string } | { kind: 'invalid' } | null {
  const n = name.trim();
  if (!n) return null;
  if (/\s/.test(n)) {
    const suggestion = n.replace(/\s+/g, '-');
    return { kind: 'spaces', suggestion: branchProblem(suggestion) ? branchSlug(suggestion) : suggestion };
  }
  if (
    n === '@' ||
    n.startsWith('-') ||
    n.startsWith('/') ||
    n.endsWith('/') ||
    n.endsWith('.') ||
    n.endsWith('.lock') ||
    n.includes('..') ||
    n.includes('//') ||
    n.includes('@{') ||
    /[~^:?*[\\\x00-\x1f\x7f]/.test(n) ||
    n.split('/').some((c) => c.startsWith('.') || c.endsWith('.lock'))
  )
    return { kind: 'invalid' };
  return null;
}

/** A branch name suggested from the bot and the task: "pixel/inbox-empty-state". */
export function suggestBranch(bot: string | null, task: string | null): string {
  const words = (task ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
    .split(' ')
    .filter(Boolean)
    .slice(0, 4)
    .join('-');
  const who = bot ? bot.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') : '';
  return [who, words].filter(Boolean).join('/');
}

/** The create dialog's inline error for a server refusal; `dialog` errors go above the buttons. */
export function createErrorCopy(e: { code: string | null; message: string }, ctx: { branch: string; path: string; checkedOutAt?: string | null }): { field: boolean; text: string } {
  switch (e.code) {
    case 'bad_branch':
      return { field: true, text: 'That isn’t a valid branch name.' };
    case 'branch_exists':
      return { field: true, text: `${ctx.branch} already exists. Pick Existing branch to use it, or choose another name.` };
    case 'branch_checked_out':
      return { field: true, text: ctx.checkedOutAt ? `${ctx.branch} already has a worktree: ${ctx.checkedOutAt}.` : `${ctx.branch} is already checked out in another folder.` };
    case 'folder_taken':
      return { field: true, text: `${ctx.path} already exists and isn’t empty. kwant won’t write into it.` };
    case 'unknown_branch':
      return { field: true, text: `There’s no branch called ${ctx.branch}.` };
    case 'bad_base':
    case 'unknown_base':
      return { field: true, text: 'That base branch doesn’t exist here.' };
    case 'thread_has_worktree':
      return { field: false, text: 'Couldn’t create it: that chat already has a worktree. Nothing was changed.' };
    case 'git_failed':
      return { field: false, text: 'Couldn’t create it: Git refused. Nothing was changed.' };
    case 'no_git':
      return { field: false, text: 'Couldn’t create it: Git isn’t installed on this server. Nothing was changed.' };
    default:
      return { field: false, text: `Couldn’t create it: ${e.message.replace(/\.$/, '')}. Nothing was changed.` };
  }
}

export function removeErrorCopy(e: { code: string | null; message: string }, bot: string | null): string {
  switch (e.code) {
    case 'working':
      return `${bot ?? 'A bot'} is working here. Stop ${bot ?? 'it'} first.`;
    case 'not_ours':
      return 'Couldn’t remove it: the folder isn’t a worktree made here. Nothing was deleted.';
    case 'git_failed':
      return 'Couldn’t remove it: Git refused. Nothing was deleted.';
    default:
      return `Couldn’t remove it: ${e.message.replace(/\.$/, '')}. Nothing was deleted.`;
  }
}

/** "~/kwant-wt/shop/x" from an absolute path under the person's home, for display only. */
export function shortPath(p: string, home: string | null = null): string {
  if (home && p.startsWith(home + '/')) return `~${p.slice(home.length)}`;
  const m = p.match(/^\/(?:home|Users)\/[^/]+(\/.*)$/);
  return m ? `~${m[1]}` : p;
}
