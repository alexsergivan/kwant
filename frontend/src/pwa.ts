// Installing kwant bots on a phone: the service worker, the install hint and the app height.

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
