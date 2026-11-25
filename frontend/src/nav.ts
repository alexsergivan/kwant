// Where you are inside a space besides a chat: Inbox, Mission board, Parallel Universe, the Dev
// desk or Settings. Each has its own address under /s/<space>/, so a reload, a bookmark or a link
// lands on the same place. Older names (While you were away, Loose ends, Connections,
// Routines, and Settings' old Accounts, Connections and Tools tabs) still resolve and are
// rewritten to the new address.

export type InboxTab = 'new' | 'loose';

/** Settings › This space tabs. Kept here, not in the component, so routing stays pure. */
export const SPACE_SETTINGS_TABS = ['general', 'apps', 'bots', 'teams', 'docs', 'skills', 'routines', 'import'] as const;
export type SpaceSettingsTab = (typeof SPACE_SETTINGS_TABS)[number];

export type SettingsScope = 'space' | 'app';

/** Settings › All spaces pages (36e). `settings/app` is General; the others are `settings/app/<page>`. */
export const APP_SETTINGS_PAGES = ['general', 'bots', 'usage', 'notifications'] as const;
export type AppSettingsPage = (typeof APP_SETTINGS_PAGES)[number];

/** The All spaces page that holds a focusRow target, so a deep link opens that page first. */
const ROW_PAGES: Record<string, AppSettingsPage> = {
  'idea-jar': 'bots',
  'decide-first': 'bots',
  'work-ahead': 'bots',
  'mission-board': 'bots',
  autofix: 'bots',
  lessons: 'bots',
  'flight-school': 'bots',
  auditor: 'bots',
  'automation-discovery': 'bots',
  'spend-stop': 'usage',
  'claude-isolation': 'general',
};
export function appSettingsPageForRow(row: string | null | undefined): AppSettingsPage {
  return (row && ROW_PAGES[row]) || 'general';
}

/** Where Apps & accounts opens; the address stays settings/apps. */
export type AppsSection = 'apps' | 'accounts' | 'advanced';

/** Settings' old tabs (0.9.4 and before), now sections of Apps & accounts. */
const OLD_APPS_TABS: Record<string, AppsSection> = { accounts: 'accounts', connections: 'advanced', tools: 'advanced' };

export type SpaceView =
  | { kind: 'inbox'; tab: InboxTab }
  | { kind: 'board' }
  | { kind: 'parallel' }
  | { kind: 'dev' }
  | { kind: 'jar' }
  | { kind: 'settings'; scope: 'space'; tab: SpaceSettingsTab; section?: AppsSection }
  | { kind: 'settings'; scope: 'app'; page?: AppSettingsPage };

/** The address segment after /s/<space>/ for a view. */
export function viewSegment(v: SpaceView): string {
  switch (v.kind) {
    case 'inbox':
      return v.tab === 'loose' ? 'inbox/loose-ends' : 'inbox';
    case 'board':
      return 'board';
    case 'parallel':
      return 'parallel';
    case 'dev':
      return 'dev';
    case 'jar':
      return 'jar';
    case 'settings':
      if (v.scope === 'space') return `settings/${v.tab}`;
      return v.page && v.page !== 'general' ? `settings/app/${v.page}` : 'settings/app';
  }
}

/** Old names that still open the right place. */
const LEGACY: Record<string, SpaceView> = {
  away: { kind: 'inbox', tab: 'new' },
  'while-you-were-away': { kind: 'inbox', tab: 'new' },
  'loose-ends': { kind: 'inbox', tab: 'loose' },
  connections: { kind: 'settings', scope: 'space', tab: 'apps', section: 'advanced' },
  routines: { kind: 'settings', scope: 'space', tab: 'routines' },
  'mission-board': { kind: 'board' },
  missions: { kind: 'board' },
  'parallel-universe': { kind: 'parallel' },
  'idea-jar': { kind: 'jar' },
  ideas: { kind: 'jar' },
  'app-settings': { kind: 'settings', scope: 'app', page: 'general' },
};

/**
 * The view an address segment names, or null when it names none. `legacy` is true when the
 * segment is an old name, so the caller rewrites the address to `viewSegment(view)`.
 */
export function parseView(segment: string): { view: SpaceView; legacy: boolean } | null {
  const s = segment.replace(/^\/+|\/+$/g, '').toLowerCase();
  if (!s) return null;
  if (LEGACY[s]) return { view: LEGACY[s], legacy: true };
  const [head, sub, ...rest] = s.split('/');
  // settings/app/<page>: an All spaces page; settings/all/<page> and settings/app/general are rewritten.
  if (head === 'settings' && (sub === 'app' || sub === 'all') && rest.length === 1) {
    const page = rest[0] as AppSettingsPage;
    if (!(APP_SETTINGS_PAGES as readonly string[]).includes(page)) return null;
    return { view: { kind: 'settings', scope: 'app', page }, legacy: sub !== 'app' || page === 'general' };
  }
  if (rest.length) return null;
  if (head === 'inbox') {
    if (!sub || sub === 'new') return { view: { kind: 'inbox', tab: 'new' }, legacy: !!sub };
    if (sub === 'loose-ends' || sub === 'loose') return { view: { kind: 'inbox', tab: 'loose' }, legacy: sub !== 'loose-ends' };
    return null;
  }
  if (head === 'board' && !sub) return { view: { kind: 'board' }, legacy: false };
  if (head === 'parallel' && !sub) return { view: { kind: 'parallel' }, legacy: false };
  if (head === 'dev' && !sub) return { view: { kind: 'dev' }, legacy: false };
  if (head === 'jar' && !sub) return { view: { kind: 'jar' }, legacy: false };
  if (head === 'settings') {
    if (!sub) return { view: { kind: 'settings', scope: 'space', tab: 'general' }, legacy: true };
    if (sub === 'app' || sub === 'all') return { view: { kind: 'settings', scope: 'app', page: 'general' }, legacy: sub !== 'app' };
    if ((SPACE_SETTINGS_TABS as readonly string[]).includes(sub)) {
      return { view: { kind: 'settings', scope: 'space', tab: sub as SpaceSettingsTab }, legacy: false };
    }
    if (OLD_APPS_TABS[sub]) return { view: { kind: 'settings', scope: 'space', tab: 'apps', section: OLD_APPS_TABS[sub] }, legacy: true };
    return null;
  }
  return null;
}

/** Whether two views are the same place. */
export function sameView(a: SpaceView | null | undefined, b: SpaceView | null | undefined): boolean {
  if (!a || !b) return !a && !b;
  return viewSegment(a) === viewSegment(b);
}

/**
 * The view to open in a space picked from the rail or with Ctrl/⌘ 1–9. Settings stays open
 * (This space then shows the new space, All spaces stays as is); anything else opens a chat.
 */
export function viewAfterSpaceSwitch(view: SpaceView | null | undefined): SpaceView | null {
  return view?.kind === 'settings' ? view : null;
}

const PHONE_TIP_KEY = 'kwnt.phoneTip.dismissed';

/** The desktop sidebar's one-time "Tip: use kwant bots on your phone" shows until it is dismissed
 *  or clicked. A new key on purpose: people who dismissed the old install tip see this once. */
export function phoneTipDismissed(): boolean {
  try {
    return localStorage.getItem(PHONE_TIP_KEY) === '1';
  } catch {
    return false;
  }
}
export function dismissPhoneTip() {
  try {
    localStorage.setItem(PHONE_TIP_KEY, '1');
  } catch {
    // private mode: it comes back next load
  }
}
