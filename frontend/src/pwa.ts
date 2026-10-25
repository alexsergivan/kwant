// Installing kwant bots: the service worker, the install hint and button, and the app height.

/** Registers /sw.js (built by vite.config.ts). Only in a build, and only where browsers allow
 * one: HTTPS or localhost. Plain http on a home network still works, just not installed. */
export function registerServiceWorker(win: Window = window, prod: boolean = import.meta.env.PROD): boolean {
  if (!prod || !win.isSecureContext || !('serviceWorker' in win.navigator)) return false;
  const register = () => {
    win.navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch(() => undefined);
  };
  if (win.document.readyState === 'complete') register();
  else win.addEventListener('load', register, { once: true });
  return true;
}

/** Whether this is the installed app (home screen), not a browser tab. */
export function isStandalone(win: Pick<Window, 'matchMedia' | 'navigator'>): boolean {
  if ((win.navigator as Navigator & { standalone?: boolean }).standalone === true) return true;
  return !!win.matchMedia?.('(display-mode: standalone)').matches;
}

/** iPhone and iPad, where installing is Share, then Add to Home Screen. iPadOS says it's a Mac. */
export function isIos(nav: Pick<Navigator, 'userAgent' | 'maxTouchPoints'>): boolean {
  return /iPhone|iPad|iPod/.test(nav.userAgent) || (/Macintosh/.test(nav.userAgent) && nav.maxTouchPoints > 1);
}

export type InstallHintKind = 'prompt' | 'ios' | null;

/** Which install hint to show: the browser's own install button (Android, Chrome), the Share
 * steps (iOS), or none: already installed, dismissed, not a phone, or not served over HTTPS. */
export function installHintKind(s: {
  standalone: boolean;
  dismissed: boolean;
  phone: boolean;
  secure: boolean;
  ios: boolean;
  canPrompt: boolean;
}): InstallHintKind {
  if (s.standalone || s.dismissed || !s.phone || !s.secure) return null;
  if (s.canPrompt) return 'prompt';
  return s.ios ? 'ios' : null;
}

export const INSTALL_HINT_DISMISSED_KEY = 'kwnt.installHint.dismissed';

/** The Mac app's .dmg in the newest release (published by the mac workflow on a mac-v* tag). */
export const MAC_DOWNLOAD_URL = 'https://github.com/alexsergivan/kwant/releases/latest/download/kwant_universal.dmg';

/** Safari on a Mac, where installing is File, then Add to Dock. Not iPad, not Chrome or Firefox. */
export function isMacSafari(nav: Pick<Navigator, 'userAgent' | 'maxTouchPoints'>): boolean {
  const ua = nav.userAgent;
  return /Macintosh/.test(ua) && !isIos(nav) && /Version\/[\d.]+.*Safari\//.test(ua) && !/Chrome|Chromium|CriOS|FxiOS|Edg\//.test(ua);
}

/** A web view inside an app (the kwant Mac app's WKWebView): a Mac WebKit page that isn't Safari. */
export function isEmbeddedWebView(nav: Pick<Navigator, 'userAgent' | 'maxTouchPoints'>): boolean {
  const ua = nav.userAgent;
  return /Macintosh/.test(ua) && !isIos(nav) && /AppleWebKit/.test(ua) && !/Safari\//.test(ua);
}

export type DesktopInstallKind = 'prompt' | 'safari' | 'ios' | 'insecure' | 'manual' | null;

/** What the desktop "Install app" entry offers: the browser's one-click install (Chrome, Edge),
 * the File > Add to Dock steps (Safari on a Mac), the Share steps (iPad), a note that installing
 * needs https, or the browser's own menu. None when it's already the installed app or the Mac app. */
export function desktopInstallKind(s: {
  standalone: boolean;
  embedded: boolean;
  secure: boolean;
  canPrompt: boolean;
  macSafari: boolean;
  ios: boolean;
}): DesktopInstallKind {
  if (s.standalone || s.embedded) return null;
  if (!s.secure) return 'insecure';
  if (s.canPrompt) return 'prompt';
  if (s.macSafari) return 'safari';
  if (s.ios) return 'ios';
  return 'manual';
}

/** Chrome's install event: not in the DOM types yet. */
export interface InstallPromptEvent extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

/** The browser's install offer, kept from page load, so it's there whenever an install button
 * mounts. Chrome sends it once, often before the app has rendered. */
export interface InstallState {
  prompt: InstallPromptEvent | null;
  installed: boolean;
}

let installState: InstallState = { prompt: null, installed: false };
const installListeners = new Set<() => void>();
const setInstallState = (next: InstallState) => {
  installState = next;
  installListeners.forEach((fn) => fn());
};

export function captureInstallPrompt(win: Window = window): void {
  installState = { prompt: null, installed: isStandalone(win) };
  win.addEventListener('beforeinstallprompt', (e) => {
    // Keep Chrome's own mini bar away; kwant offers the same install.
    e.preventDefault();
    setInstallState({ ...installState, prompt: e as InstallPromptEvent });
  });
  win.addEventListener('appinstalled', () => setInstallState({ prompt: null, installed: true }));
}

export const getInstallState = (): InstallState => installState;

export function subscribeInstallState(fn: () => void): () => void {
  installListeners.add(fn);
  return () => {
    installListeners.delete(fn);
  };
}

/** Shows the browser's install dialog. A prompt works once; Chrome sends a new event if it can be
 * offered again. */
export async function runInstallPrompt(): Promise<void> {
  const { prompt } = installState;
  if (!prompt) return;
  await prompt.prompt();
  const { outcome } = await prompt.userChoice.catch(() => ({ outcome: 'dismissed' as const }));
  setInstallState({ prompt: null, installed: installState.installed || outcome === 'accepted' });
}

/** Keeps --kwnt-app-height at the visible height, so the iOS keyboard doesn't cover the message
 * box (Android resizes the page itself: interactive-widget=resizes-content in index.html). */
export function trackAppHeight(win: Window = window): () => void {
  const vv = win.visualViewport;
  if (!vv) return () => undefined;
  const root = win.document.documentElement;
  const update = () => {
    // Pinch zoom shrinks the visible area too; times the zoom it's the page height again.
    root.style.setProperty('--kwnt-app-height', `${Math.round(vv.height * vv.scale)}px`);
    // iOS scrolls the whole page up to show the field; the app already fits, so undo that.
    if (vv.scale === 1 && (vv.offsetTop > 0 || win.scrollY > 0)) win.scrollTo(0, 0);
  };
  update();
  vv.addEventListener('resize', update);
  vv.addEventListener('scroll', update);
  return () => {
    vv.removeEventListener('resize', update);
    vv.removeEventListener('scroll', update);
  };
}
