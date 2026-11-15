/**
 * The place chip at the top of the Sidebar and the Settings "Perch" section (desktop-island/design.md 9.1, 8).
 *
 * Contracts with the desktop app (apps/mac, Juno; desktop-island/build-brief.md):
 * - `window.__KWANT_APP__` ('mac' | 'linux') marks the desktop app. Without it there is no chip.
 * - `window.__KWANT_ELSEWHERE__` (integer) is how many things wait in the person's OTHER places. Rust sets it
 *   only in the This Mac window and fires `kwant-elsewhere` on window after each change. A server window never
 *   gets it, so its chip shows no count.
 * - `window.__KWANT_ISLAND__` `{mode, shortcut, available}` exists only in the This Mac window. Without it
 *   there is no Perch section.
 * - The page reaches Rust only by navigating to `kwant-app://places?at=x,y` and
 *   `kwant-app://island?mode=…&shortcut=…`, which on_navigation catches and refuses. Each at most once a second.
 */
import { inDesktopApp } from './externalLinks';

type Win = {
  __KWANT_APP__?: unknown;
  __KWANT_ELSEWHERE__?: unknown;
  __KWANT_ISLAND__?: unknown;
  location?: { host?: string; hostname?: string; port?: string };
};

const win = (): unknown => (typeof window !== 'undefined' ? window : undefined);

/** Hostnames of the desktop app's in-process server on this computer. Its port is 8799 unless startup.rs had to
 *  move it (choose_port), so any non-zero port counts: the same rule as Rust's `switcher::is_local_page`. */
export const LOCAL_HOSTNAMES: readonly string[] = ['127.0.0.1', 'localhost'];

/** Whether `host` ("name:port") is this computer's kwant: a loopback name with an explicit, non-zero port. */
export function isLocalHost(host: string): boolean {
  const m = /^([^:]+):(\d+)$/.exec(host.toLowerCase());
  return !!m && LOCAL_HOSTNAMES.includes(m[1]) && Number(m[2]) > 0 && Number(m[2]) <= 65535;
}

/** Hosts longer than this are cut in the middle (design.md 9.1). */
export const HOST_MAX = 28;

/** Cuts the middle of `s` so it is at most `max` characters, keeping its start and end. */
export function middleCut(s: string, max = HOST_MAX): string {
  const chars = Array.from(s);
  if (chars.length <= max) return s;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  return `${chars.slice(0, head).join('')}…${chars.slice(chars.length - (keep - head)).join('')}`;
}

export type PlaceKind = 'local' | 'server';

/** Which place this page is, from its own origin only: nothing about other places reaches a page. */
export function placeOf(w: unknown = win()): { kind: PlaceKind; name: string; full: string } {
  const loc = (w as Win | undefined)?.location;
  const host = (loc?.host || loc?.hostname || '').toLowerCase();
  if (isLocalHost(host)) {
    const name = (w as Win).__KWANT_APP__ === 'mac' ? 'This Mac' : 'This computer';
    return { kind: 'local', name, full: name };
  }
  const full = host.replace(/:443$/, '') || 'this server';
  return { kind: 'server', name: middleCut(full), full };
}

/** Whether to show the chip: in the desktop app always, even with one place; never in the web build. */
export const showPlaceChip = (w: unknown = win()): boolean => inDesktopApp(w);

/** How many wait in other places, or 0 when Rust gave nothing (a server window, an older app). */
export function elsewhereCount(w: unknown = win()): number {
  const n = (w as Win | undefined)?.__KWANT_ELSEWHERE__;
  return typeof n === 'number' && Number.isInteger(n) && n > 0 ? Math.min(n, 999) : 0;
}

export const ELSEWHERE_EVENT = 'kwant-elsewhere';

/** "9+" past nine, like the rest of the app's small badges. */
export const elsewhereText = (n: number): string => (n > 9 ? '9+' : String(n));

/** Starting… and Didn't start come from the app; a page that answers is running, so no dot by default. */
export type PlaceDot = 'starting' | 'failed' | null;

export function chipAriaLabel(name: string, elsewhere: number): string {
  return elsewhere > 0
    ? `Place: ${name}, ${elsewhere} waiting in other places. Switch place`
    : `Place: ${name}. Switch place`;
}

/** The picker URL. x,y is the chip's bottom-left in window coordinates, whole pixels; Rust clamps it. */
export function placesUrl(x: number, y: number): string {
  const r = (v: number) => (Number.isFinite(v) ? Math.max(0, Math.round(v)) : 0);
  return `kwant-app://places?at=${r(x)},${r(y)}`;
}

/** Lets a call through at most once per `ms`; the rest are dropped, not queued. */
export function onceEvery(ms: number, now: () => number = () => Date.now()): () => boolean {
  let last = -Infinity;
  return () => {
    const t = now();
    if (t - last < ms) return false;
    last = t;
    return true;
  };
}

// ---------- Perch settings ----------

export const PERCH_MODES = ['busy', 'always', 'off'] as const;
export type PerchMode = (typeof PERCH_MODES)[number];
/** Shortcut tokens on the wire, the same on every OS: `k` = ⌥⌘K / Ctrl+Alt+K, `space` = ⌃⌥Space / Ctrl+Alt+Space. */
export const PERCH_SHORTCUTS = ['k', 'space', 'off'] as const;
export type PerchShortcut = (typeof PERCH_SHORTCUTS)[number];

export type PerchSettings = { mode: PerchMode; shortcut: PerchShortcut; available: boolean };

const isMode = (v: unknown): v is PerchMode => typeof v === 'string' && (PERCH_MODES as readonly string[]).includes(v);
const isShortcut = (v: unknown): v is PerchShortcut =>
  typeof v === 'string' && (PERCH_SHORTCUTS as readonly string[]).includes(v);

/** The perch settings Rust gave this window, or null when it gave none (any window but This Mac, the web). */
export function perchSettings(w: unknown = win()): PerchSettings | null {
  if (!inDesktopApp(w)) return null;
  const raw = (w as Win).__KWANT_ISLAND__;
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  return {
    mode: isMode(r.mode) ? r.mode : 'busy',
    shortcut: isShortcut(r.shortcut) ? r.shortcut : 'k',
    available: r.available !== false,
  };
}

/** The settings URL, or null for any value outside the whitelist. */
export function islandUrl(mode: unknown, shortcut: unknown): string | null {
  if (!isMode(mode) || !isShortcut(shortcut)) return null;
  return `kwant-app://island?mode=${mode}&shortcut=${shortcut}`;
}

export const PERCH_COPY = {
  title: 'Perch',
  about: 'A small bar at the top of your screen: who is working, what needs you, and quick answers without opening the app.',
  modes: [
    { value: 'busy', label: 'When something’s happening', line: 'Shows while bots work or need you, and hides when nothing is going on.' },
    { value: 'always', label: 'Always', line: 'Stays at the top of the screen, even when nothing is going on.' },
    { value: 'off', label: 'Off', line: 'No perch. You get the nudge pill, the menu bar mark and the app badge, as before.' },
  ] as { value: PerchMode; label: string; line: string }[],
  /** Linux and Windows have a tray mark, not a menu bar mark or a Dock badge. */
  offLineElsewhere: 'No perch. You get the nudge pill and the tray mark, as before.',
  shortcutLabel: 'Shortcut to open it',
  wayland: 'The perch isn’t available on Wayland yet. You still get the tray mark and a notification when a bot needs you.',
} as const;

/** The line under the mode buttons, in this OS's words. */
export function perchModeLine(mode: PerchMode, w: unknown = win()): string {
  if (mode === 'off' && (w as Win | undefined)?.__KWANT_APP__ !== 'mac') return PERCH_COPY.offLineElsewhere;
  return PERCH_COPY.modes.find((m) => m.value === mode)?.line ?? '';
}

/** The shortcut choices in this OS's words. */
export function shortcutOptions(w: unknown = win()): { value: PerchShortcut; label: string }[] {
  const mac = (w as Win | undefined)?.__KWANT_APP__ === 'mac';
  return [
    { value: 'k', label: mac ? '⌥⌘K' : 'Ctrl+Alt+K' },
    { value: 'space', label: mac ? '⌃⌥Space' : 'Ctrl+Alt+Space' },
    { value: 'off', label: 'Off' },
  ];
}
