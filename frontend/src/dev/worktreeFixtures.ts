// 0.9.0 H1: sample worktrees for the tests and the 390/1280 shots (the states in Mira's h1-states-board).
// Names and paths are made up; the shape is Lin's contract v2 (GET /api/spaces/:id/dev/worktrees).

export const NOW = Date.parse('2026-11-10T14:20:00Z');
export const ROOT = '/home/ann/.kwant/spaces/sp1/worktrees';
export const BOTS = [
  { id: 'k-pixel', name: 'Pixel', role: 'frontend' },
  { id: 'k-lin', name: 'Lin', role: 'backend' },
  { id: 'k-rigel', name: 'Rigel', role: 'qa' },
];

const base = {
  team_id: 't-shop',
  team: 'Shop',
  repo: 'shop',
  base: 'main',
  made_branch: true,
  head: '3f2c9a1b7e',
  changes: [],
  ahead: 0,
  behind: 0,
  state: 'live',
  ours: true,
  working: false,
  thread_id: null,
  task_id: null,
  kwant_id: null,
  task_title: null,
  created_at: '2026-11-10T09:12:00Z',
  done_at: null,
  clean_at: null,
  keep_build: false,
  cleaned_at: null,
  cleaned: [],
  cleaned_bytes: null,
  kept: [],
  removed_at: null,
};

export const wt = (id: string, branch: string, extra: Record<string, unknown> = {}) => ({
  ...base,
  id,
  branch,
  folder: `shop/${branch.toLowerCase().replace(/[^a-z0-9_]+/g, '-')}`,
  path: `${ROOT}/shop/${branch.toLowerCase().replace(/[^a-z0-9_]+/g, '-')}`,
  ...extra,
});

export const WORKING = wt('w1', 'pixel/inbox-empty-state', {
  kwant_id: 'k-pixel',
  thread_id: 'th-pixel',
  working: true,
  task_title: 'Inbox: empty state and keyboard order',
  dirty: 3,
  changes: [
    { status: 'M', path: 'src/components/Inbox.tsx' },
    { status: 'M', path: 'src/styles/modern.css' },
    { status: '?', path: 'src/components/InboxEmpty.tsx' },
  ],
  ahead: 2,
  behind: 0,
});
export const FINISHED_DUE = wt('w2', 'lin/worktree-routes', {
  kwant_id: 'k-lin',
  thread_id: 'th-lin',
  task_title: 'Worktree routes and the cleanup job',
  dirty: 0,
  ahead: 4,
  behind: 1,
  done_at: '2026-11-10T13:42:00Z',
  clean_at: '2026-11-10T14:42:00Z',
});
export const CLEANED = wt('w3', 'rigel/f6-notification', {
  kwant_id: 'k-rigel',
  task_title: 'QA: notification opens the origin chat',
  dirty: 0,
  ahead: 1,
  behind: 3,
  done_at: '2026-11-09T17:40:00Z',
  cleaned_at: '2026-11-09T18:40:00Z',
  cleaned: ['node_modules/.vite', 'target'],
  cleaned_bytes: 1_240_000_000,
  kept: [{ dir: 'dist', why: 'tracked' }],
});
export const MINE = wt('w4', 'try-vite-6', { dirty: 1, changes: [{ status: 'M', path: 'package.json' }], ahead: 0, behind: 0 });
export const MISSING = wt('w5', 'old/spike', { state: 'missing', dirty: null, ahead: null, behind: null });

export const LIST = {
  repos: [
    {
      repo: 'shop',
      team_id: 't-shop',
      team: 'Shop',
      checkout: '/home/ann/code/shop',
      default_branch: 'main',
      error: false,
      worktrees: [WORKING, FINISHED_DUE, CLEANED, MINE],
      other: [{ folder_name: 'shop-hotfix', path: '/home/ann/code/shop-hotfix', branch: 'hotfix/1.4.2', head: 'aa11bb2', missing: false }],
    },
    { repo: 'site', team_id: 't-site', team: 'Site', checkout: '/home/ann/code/site', default_branch: 'main', error: false, worktrees: [], other: [] },
  ],
  clean_after_minutes: 60,
  build_dirs: ['target', 'node_modules/.vite', 'dist', '.next'],
};

export const BRANCHES = {
  branches: [
    { name: 'main', remote: false, checked_out_at: '/home/ann/code/shop' },
    { name: 'origin/main', remote: true, checked_out_at: null },
    { name: 'release/0.9', remote: false, checked_out_at: null },
    { name: 'pixel/inbox-empty-state', remote: false, checked_out_at: `${ROOT}/shop/pixel-inbox-empty-state` },
    { name: 'origin/feature/search', remote: true, checked_out_at: null },
  ],
};
