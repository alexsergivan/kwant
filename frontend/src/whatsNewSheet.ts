// "What's new in kwant X.Y.Z": the sheet shown once per device after an update (0.7.6 on).
// The content is frontend/src/whatsnew/<version>.json, written in each release commit; the "All changes"
// view is that version's CHANGELOG section. Both are bundled at build time (see vite.config.ts), so the
// version shown is the newest bundled file: the web app on a server shows it after the server updates too.
// This file is the pure part (rules, limits, storage); components/WhatsNewSheet.tsx draws it.

import { parseView, type SpaceView } from './nav';
import type { KeyValueStore } from './whatsNew';

export interface WhatsNewHighlight {
  id: string;
  title: string;
  text: string;
  /** A webp in frontend/public/whatsnew/<version>/, or null. */
  image: string | null;
  /** An in-app place under /s/<space>/ (a view segment such as "settings/teams"), or null. */
  open: string | null;
}

export interface WhatsNewEntry {
  version: string;
  /** YYYY-MM-DD */
  date: string;
  highlights: WhatsNewHighlight[];
  fixes: string[];
}

/** The limits a whatsnew file must keep. scripts/site_release_gate.py checks the same ones at release. */
export const LIMITS = { highlightsMin: 2, highlightsMax: 6, title: 60, text: 200, fixesMax: 15, fix: 140 } as const;

export const SEEN_KEY = 'kwant:whatsnew:seen';
/** More fixes than this fold behind "Show N more"; folded, the first FIXES_SHOWN stay visible. */
export const FIXES_FOLD_OVER = 4;
export const FIXES_SHOWN = 3;
export const RELEASES_URL = 'https://github.com/alexsergivan/kwant/releases';
export const releaseUrl = (version: string) => `${RELEASES_URL}/tag/mac-v${version}`;

const VERSION = /^\d+\.\d+\.\d+$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;
const IMAGE = /^[a-z0-9][a-z0-9._-]*\.webp$/;

const isStr = (v: unknown): v is string => typeof v === 'string';

/** Every way `raw` breaks the format or the limits; empty when it's good. */
export function validateEntry(raw: unknown, fileVersion?: string): string[] {
  const out: string[] = [];
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return ['not a JSON object'];
  const e = raw as Record<string, unknown>;
  if (!isStr(e.version) || !VERSION.test(e.version)) out.push('version must be X.Y.Z');
  else if (fileVersion && e.version !== fileVersion) out.push(`version ${e.version} doesn't match the file name ${fileVersion}.json`);
  if (!isStr(e.date) || !DATE.test(e.date)) out.push('date must be YYYY-MM-DD');
  const hs = e.highlights;
  if (!Array.isArray(hs)) out.push('highlights must be a list');
  else {
    if (hs.length < LIMITS.highlightsMin || hs.length > LIMITS.highlightsMax) {
      out.push(`highlights: ${hs.length}, needs ${LIMITS.highlightsMin}–${LIMITS.highlightsMax}`);
    }
    const ids = new Set<string>();
    hs.forEach((h: unknown, i) => {
      const at = `highlights[${i}]`;
      if (!h || typeof h !== 'object') return void out.push(`${at} must be an object`);
      const x = h as Record<string, unknown>;
      if (!isStr(x.id) || !/^[a-z0-9-]+$/.test(x.id)) out.push(`${at}.id must be lowercase letters, digits and dashes`);
      else if (ids.has(x.id)) out.push(`${at}.id "${x.id}" is used twice`);
      else ids.add(x.id);
      if (!isStr(x.title) || !x.title.trim()) out.push(`${at}.title is empty`);
      else if ([...x.title].length > LIMITS.title) out.push(`${at}.title is ${[...x.title].length} characters, max ${LIMITS.title}`);
      if (!isStr(x.text) || !x.text.trim()) out.push(`${at}.text is empty`);
      else if ([...x.text].length > LIMITS.text) out.push(`${at}.text is ${[...x.text].length} characters, max ${LIMITS.text}`);
      if (x.image !== null && (!isStr(x.image) || !IMAGE.test(x.image))) out.push(`${at}.image must be null or a .webp file name`);
      if (x.open !== null && (!isStr(x.open) || !parseView(x.open) || parseView(x.open)!.legacy)) {
        out.push(`${at}.open must be null or an in-app place such as "settings/teams"`);
      }
    });
  }
  const fx = e.fixes;
  if (!Array.isArray(fx)) out.push('fixes must be a list');
  else {
    if (fx.length > LIMITS.fixesMax) out.push(`fixes: ${fx.length}, max ${LIMITS.fixesMax}`);
    fx.forEach((f: unknown, i) => {
      if (!isStr(f) || !f.trim()) out.push(`fixes[${i}] is empty`);
      else if ([...f].length > LIMITS.fix) out.push(`fixes[${i}] is ${[...f].length} characters, max ${LIMITS.fix}`);
    });
  }
  return out;
}

/** -1, 0 or 1. Anything that isn't X.Y.Z sorts below every version. */
export function compareVersions(a: string | null | undefined, b: string | null | undefined): number {
  const p = (v: string | null | undefined) => (v && VERSION.test(v) ? v.split('.').map(Number) : null);
  const x = p(a);
  const y = p(b);
  if (!x || !y) return x ? 1 : y ? -1 : 0;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
  return 0;
}

/** The good entries, newest first. A broken file is left out (the release gate stops it before it ships). */
export function sortEntries(raw: unknown[]): WhatsNewEntry[] {
  return raw
    .filter((r) => validateEntry(r).length === 0)
    .map((r) => r as WhatsNewEntry)
    .sort((a, b) => compareVersions(b.version, a.version));
}

/** A version's CHANGELOG section ("## [Mac X.Y.Z] - date" up to the next "## "), without its heading. */
export function changelogSection(changelog: string, version: string): string | null {
  const lines = changelog.split('\n');
  const head = lines.findIndex((l) => l.trimEnd().startsWith(`## [Mac ${version}]`));
  if (head < 0) return null;
  let end = lines.findIndex((l, i) => i > head && l.startsWith('## '));
  if (end < 0) end = lines.length;
  const body = lines.slice(head + 1, end).join('\n').trim();
  return body || null;
}

export function shortDate(iso: string): string {
  const [y, m, d] = iso.split('-').map(Number);
  if (!y || !m || !d) return iso;
  return `${['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][m - 1]} ${d}`;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
/** "6 new things, 4 fixes" */
export const entrySummary = (e: WhatsNewEntry) =>
  `${plural(e.highlights.length, 'new thing')}${e.fixes.length ? `, ${plural(e.fixes.length, 'fix', 'fixes')}` : ''}`;

/** The place a highlight's "Show me" opens, or null. */
export function highlightView(h: WhatsNewHighlight): SpaceView | null {
  const p = h.open ? parseView(h.open) : null;
  return p && !p.legacy ? p.view : null;
}

// ---------- show once ----------

/**
 * What to do on a load, given the stored version (`seen`), the newest bundled one and whether this is an
 * existing person: `established` is true when they have used kwant before (an account works, or they have
 * written in a chat), false for a brand-new person, null while that isn't known yet.
 * - show: open the sheet (when no dialog is open), then store `latest`.
 * - record: store `latest` without showing anything (a new person gets Get started instead).
 * - wait: decide again once `established` is known.
 * - none: nothing to do.
 */
export type OnLoad = 'show' | 'record' | 'wait' | 'none';

export function decideOnLoad(seen: string | null, latest: string | null, established: boolean | null): OnLoad {
  if (!latest) return 'none';
  if (seen !== null) return compareVersions(latest, seen) > 0 ? 'show' : 'none';
  // No key: everyone upgrading from 0.7.5 or earlier. Only a person who has used kwant sees it.
  if (established === null) return 'wait';
  return established ? 'show' : 'record';
}

const defaultStore = (): KeyValueStore | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};

export function seenVersion(store: KeyValueStore | null = defaultStore()): string | null {
  try {
    return store?.getItem(SEEN_KEY) ?? null;
  } catch {
    return null;
  }
}

/** Never moves back: an older tab can't make a newer version show again. */
export function markSeen(version: string, store: KeyValueStore | null = defaultStore()): void {
  if (!store) return;
  try {
    if (compareVersions(version, store.getItem(SEEN_KEY)) > 0) store.setItem(SEEN_KEY, version);
  } catch {
    return;
  }
}

/** A modal or an alert is open (any dialog but the sheet itself), so the sheet waits. */
export function otherDialogOpen(doc: Document | null = typeof document === 'undefined' ? null : document): boolean {
  if (!doc) return false;
  return [...doc.querySelectorAll('[aria-modal="true"], [role="alertdialog"], dialog[open]')].some((d) => !d.closest('[data-whats-new]'));
}

// ---------- opening it from elsewhere (the update banner, Settings › About) ----------

export interface OpenRequest {
  version: string;
  seq: number;
  /** A version this app has no sheet for (one not installed yet): its release notes, markdown. */
  notes?: string;
  /** That release's date, YYYY-MM-DD or RFC 3339. */
  date?: string | null;
}
let request: OpenRequest | null = null;
let seq = 0;
const listeners = new Set<() => void>();
export const subscribeOpen = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};
export const openRequest = () => request;
/** Ask the mounted sheet host to open `version`'s sheet. */
export function openWhatsNew(version: string, notes?: { notes: string; date?: string | null }): void {
  request = { version, seq: ++seq, ...(notes ? { notes: notes.notes, date: notes.date ?? null } : {}) };
  listeners.forEach((l) => l());
}
