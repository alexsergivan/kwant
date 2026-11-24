// Entry for /p/<code>. Reads the optional v1 state the server put in the page, picks the screens by the pocket's kit
// (Secret Santa, gather and the Wall load their own chunks; everything else is the quiz, exactly as before), then joins the live pocket.
import './guest.css';
import { codeFrom, Env } from './api';
import { GuestApp } from './app';
import { remoteToken } from './remote/link';
import { screenToken } from './screen/link';

function boot(): unknown {
  const el = document.getElementById('pocket-boot');
  if (!el?.textContent) return null;
  try {
    return JSON.parse(el.textContent);
  } catch {
    return null;
  }
}

const kitOf = (json: unknown): string => {
  const p = json && typeof json === 'object' ? (json as { pocket?: { kit?: unknown } }).pocket : undefined;
  return typeof p?.kit === 'string' ? p.kit : '';
};

const root = document.getElementById('app')!;
const w = window;
const env: Env & { now: () => number; setInterval: (f: () => void, ms: number) => number; clearInterval: (id: number) => void } = {
  fetch: w.fetch.bind(w),
  WebSocket: w.WebSocket,
  setTimeout: (f, ms) => w.setTimeout(f, ms),
  clearTimeout: (id) => w.clearTimeout(id),
  setInterval: (f, ms) => w.setInterval(f, ms),
  clearInterval: (id) => w.clearInterval(id),
  now: () => Date.now(),
  location: w.location,
};
const code = codeFrom(w.location.pathname);

// Before anything else (pa5b): a #screen=<token> link is read and wiped from the address bar here.
const screen = screenToken(w, code);
// The same for the host remote (pa4b): #remote=<token>, read and wiped before any request.
const remote = screen === undefined ? remoteToken(w, code) : undefined;

/** The boot JSON names the kit. Without one (an old box), the public `info` does; if that fails too, the quiz. */
async function kit(b: unknown): Promise<string> {
  if (b) return kitOf(b);
  try {
    const r = await w.fetch(`/api/pocket/${encodeURIComponent(code)}/info`, { credentials: 'same-origin', cache: 'no-store' });
    return r.ok ? kitOf(await r.json()) : '';
  } catch {
    return '';
  }
}

function store() {
  try {
    return w.localStorage;
  } catch {
    return null;
  }
}

async function main(): Promise<void> {
  const b = boot();
  let app: { start(boot: unknown): Promise<void>; wake(): void };
  if (screen !== undefined) {
    const still = w.matchMedia?.('(prefers-reduced-motion: reduce)');
    const tvEnv = { ...env, origin: w.location.origin, still: () => !!still?.matches };
    // pa14: a Wall's big screen is the photo slideshow, its own chunk; every other kit keeps the quiz screen.
    const tv =
      (await kit(b)) === 'wall'
        ? new (await import('./wall/screen-wall')).WallTV(root, tvEnv, code, screen || undefined)
        : new (await import('./screen/screen')).ScreenApp(root, tvEnv, code, screen || undefined);
    const fit = () => tv.fit(w.innerWidth, w.innerHeight);
    fit();
    w.addEventListener('resize', fit);
    app = tv;
  } else if (remote !== undefined) {
    const { RemoteApp } = await import('./remote/remote');
    app = new RemoteApp(root, env, code, remote || undefined);
  } else {
    const k = await kit(b);
    if (k === 'santa') {
      const { SantaApp } = await import('./santa/app');
      app = new SantaApp(root, { ...env, store: store() }, code);
    } else if (k === 'wall') {
      const { WallApp } = await import('./wall/wall');
      const blobs = { create: (f: Blob) => URL.createObjectURL(f), revoke: (u: string) => URL.revokeObjectURL(u) };
      app = new WallApp(root, { ...env, XHR: w.XMLHttpRequest, blobs }, code);
    } else if (k === 'gather') {
      const { GatherApp } = await import('./gather/app');
      app = new GatherApp(root, env, code);
    } else app = new GuestApp(root, env, code);
  }
  void app.start(b);
  // A socket can die quietly while the phone sleeps or switches network: ask the box again on wake, online and restore.
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') app.wake();
  });
  w.addEventListener('online', () => app.wake());
  w.addEventListener('pageshow', (e) => {
    if (e.persisted) app.wake();
  });
}

void main();
