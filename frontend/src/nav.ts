// Where you are inside a space besides a chat: Inbox, Mission board, Parallel Universe or
// Settings. Each has its own address under /s/<space>/, so a reload, a bookmark or a link
// lands on the same place. Older names (While you were away, Loose ends, Connections,
// Routines) still resolve and are rewritten to the new address.

export type InboxTab = 'new' | 'loose';

/** Settings › This space tabs. Kept here, not in the component, so routing stays pure. */
export const SPACE_SETTINGS_TABS = ['general', 'accounts', 'bots', 'teams', 'connections', 'tools', 'skills', 'routines', 'import'] as const;
export type SpaceSettingsTab = (typeof SPACE_SETTINGS_TABS)[number];

export type SettingsScope = 'space' | 'app';

export type SpaceView =
  | { kind: 'inbox'; tab: InboxTab }
  | { kind: 'board' }
  | { kind: 'parallel' }
  | { kind: 'settings'; scope: 'space'; tab: SpaceSettingsTab }
  | { kind: 'settings'; scope: 'app' };

/** The address segment after /s/<space>/ for a view. */
export function viewSegment(v: SpaceView): string {
  switch (v.kind) {
    case 'inbox':
      return v.tab === 'loose' ? 'inbox/loose-ends' : 'inbox';
    case 'board':
      return 'board';
    case 'parallel':
      return 'parallel';
    case 'settings':
      return v.scope === 'app' ? 'settings/app' : `settings/${v.tab}`;
  }
}

/** Old names that still open the right place. */
const LEGACY: Record<string, SpaceView> = {
  away: { kind: 'inbox', tab: 'new' },
  'while-you-were-away': { kind: 'inbox', tab: 'new' },
  'loose-ends': { kind: 'inbox', tab: 'loose' },
  connections: { kind: 'settings', scope: 'space', tab: 'connections' },
  routines: { kind: 'settings', scope: 'space', tab: 'routines' },
  'mission-board': { kind: 'board' },
  missions: { kind: 'board' },
  'parallel-universe': { kind: 'parallel' },
  'app-settings': { kind: 'settings', scope: 'app' },
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
  if (rest.length) return null;
  if (head === 'inbox') {
    if (!sub || sub === 'new') return { view: { kind: 'inbox', tab: 'new' }, legacy: !!sub };
    if (sub === 'loose-ends' || sub === 'loose') return { view: { kind: 'inbox', tab: 'loose' }, legacy: sub !== 'loose-ends' };
    return null;
  }
  if (head === 'board' && !sub) return { view: { kind: 'board' }, legacy: false };
  if (head === 'parallel' && !sub) return { view: { kind: 'parallel' }, legacy: false };
  if (head === 'settings') {
    if (!sub) return { view: { kind: 'settings', scope: 'space', tab: 'general' }, legacy: true };
    if (sub === 'app' || sub === 'all') return { view: { kind: 'settings', scope: 'app' }, legacy: sub !== 'app' };
    if ((SPACE_SETTINGS_TABS as readonly string[]).includes(sub)) {
      return { view: { kind: 'settings', scope: 'space', tab: sub as SpaceSettingsTab }, legacy: false };
    }
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

const INSTALL_TIP_KEY = 'kwnt.installTip.dismissed';

/** The desktop "Install app" tip shows until it is dismissed or the app is installed. */
export function installTipDismissed(): boolean {
  try {
    return localStorage.getItem(INSTALL_TIP_KEY) === '1';
  } catch {
    return false;
  }
}
export function dismissInstallTip() {
  try {
    localStorage.setItem(INSTALL_TIP_KEY, '1');
  } catch {
    // private mode: it comes back next load
  }
}
