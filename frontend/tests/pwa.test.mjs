// Installing kwant bots on a phone: the service worker's cache rules, the manifest and icons,
// the install hint and the offline message.
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { loadBundled } from './bundle.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const ORIGIN = 'https://kwant.example';
const SHELL = ['/', '/assets/index-abc123.js', '/assets/index-def456.css', '/icons/icon-192.png', '/index.html', '/manifest.webmanifest'];

/** Runs sw.js against fake caches and network; returns what it did. */
function loadWorker({ source, version = 'v2', shell = SHELL, oldCaches = [] } = {}) {
  const code = (source ?? readFileSync(`${root}/sw.js`, 'utf8'))
    .replace('__KWNT_SW_VERSION__', version)
    .replace('__KWNT_SW_SHELL__', JSON.stringify(shell));
  const stores = new Map(oldCaches.map((k) => [k, new Map()]));
  const log = { put: [], fetched: [], deleted: [], skipWaiting: 0, claimed: 0 };
  const store = (name) => {
    if (!stores.has(name)) stores.set(name, new Map());
    const m = stores.get(name);
    const key = (r) => new URL(typeof r === 'string' ? r : r.url, ORIGIN).pathname;
    return {
      addAll: async (paths) => paths.forEach((p) => { log.put.push(p); m.set(key(p), `cached ${key(p)}`); }),
      put: async (r, res) => { log.put.push(key(r)); m.set(key(r), res); },
      match: async (r) => m.get(key(r)),
    };
  };
  const handlers = {};
  let online = true;
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type, fn) => { handlers[type] = fn; },
    skipWaiting: async () => { log.skipWaiting++; },
    clients: { claim: async () => { log.claimed++; } },
  };
  const context = {
    self,
    URL,
    Response: { error: () => 'network error' },
    caches: {
      open: async (name) => store(name),
      keys: async () => [...stores.keys()],
      delete: async (name) => { log.deleted.push(name); return stores.delete(name); },
    },
    fetch: async (r) => {
      log.fetched.push(new URL(r.url).pathname);
      if (!online) throw new TypeError('Failed to fetch');
      return `network ${new URL(r.url).pathname}`;
    },
  };
  vm.runInNewContext(code, context);
  const run = async (type, extra = {}) => {
    let waited;
    let responded;
    handlers[type]({ ...extra, waitUntil: (p) => { waited = p; }, respondWith: (p) => { responded = p; } });
    await waited;
    return responded === undefined ? undefined : await responded;
  };
  const request = (path, { method = 'GET', mode = 'cors', headers = {}, origin = ORIGIN } = {}) => ({
    url: new URL(path, origin).href,
    method,
    mode,
    headers: new Headers(headers),
  });
  return {
    log,
    stores,
    request,
    install: () => run('install'),
    activate: () => run('activate'),
    fetch: (req) => run('fetch', { request: req }),
    setOnline: (v) => { online = v; },
  };
}

test('the worker never answers or stores the API, the event stream, uploads or other sites', async () => {
  const sw = loadWorker();
  await sw.install();
  const putBefore = sw.log.put.length;
  const passThrough = [
    sw.request('/api'),
    sw.request('/api/spaces'),
    sw.request('/api/auth/status'),
    sw.request('/api/events', { headers: { accept: 'text/event-stream' } }),
    sw.request('/api/browser/ws'),
    sw.request('/api/kwants', { method: 'POST' }),
    sw.request('/api/uploads/x', { method: 'PUT' }),
    // Even a page load under /api is the server's, never index.html
    sw.request('/api/export', { mode: 'navigate' }),
    sw.request('/events', { headers: { accept: 'text/event-stream' } }),
    sw.request('/assets/index-abc123.js', { headers: { range: 'bytes=0-10' } }),
    sw.request('/assets/index-abc123.js', { origin: 'https://cdn.example' }),
    sw.request('/', { origin: 'https://other.example', mode: 'navigate' }),
    sw.request('/', { method: 'POST', mode: 'navigate' }),
    sw.request('/files/report.pdf'),
  ];
  for (const r of passThrough) {
    assert.equal(await sw.fetch(r), undefined, `${r.method} ${r.url} must go straight to the network`);
  }
  assert.equal(sw.log.put.length, putBefore, 'nothing new is stored');
  assert.deepEqual(sw.log.fetched, [], 'the worker made no requests of its own');
  for (const [, m] of sw.stores) {
    assert.ok(![...m.keys()].some((k) => k === '/api' || k.startsWith('/api/')), 'no /api entry in any cache');
  }
});

test('install stores exactly the app shell, under a cache named for this build', async () => {
  const sw = loadWorker({ version: 'abc' });
  await sw.install();
  assert.deepEqual(sw.log.put, SHELL);
  assert.deepEqual([...sw.stores.keys()], ['kwnt-shell-abc']);
  assert.equal(sw.log.skipWaiting, 1, 'the new build takes over without waiting for every tab to close');
});

test('activate drops caches from older builds but leaves other caches alone', async () => {
  const sw = loadWorker({ version: 'new', oldCaches: ['kwnt-shell-old', 'kwnt-shell-older', 'someone-elses-cache'] });
  await sw.install();
  await sw.activate();
  assert.deepEqual(sw.log.deleted.sort(), ['kwnt-shell-old', 'kwnt-shell-older']);
  assert.deepEqual([...sw.stores.keys()].sort(), ['kwnt-shell-new', 'someone-elses-cache']);
  assert.equal(sw.log.claimed, 1);
});

test('pages come from the server first and from the stored index.html only when it is unreachable', async () => {
  const sw = loadWorker();
  await sw.install();
  assert.equal(await sw.fetch(sw.request('/channels/abc', { mode: 'navigate' })), 'network /channels/abc');
  sw.setOnline(false);
  assert.equal(await sw.fetch(sw.request('/channels/abc', { mode: 'navigate' })), 'cached /index.html');
});

test('with nothing stored yet, an offline page load is a plain network error, not a blank page', async () => {
  const sw = loadWorker();
  sw.setOnline(false);
  assert.equal(await sw.fetch(sw.request('/', { mode: 'navigate' })), 'network error');
});

test('built files come from the store first, so the installed app opens without the server', async () => {
  const sw = loadWorker();
  await sw.install();
  sw.setOnline(false);
  assert.equal(await sw.fetch(sw.request('/assets/index-abc123.js')), 'cached /assets/index-abc123.js');
  assert.deepEqual(sw.log.fetched, []);
});

const built = `${root}/dist/sw.js`;
test('the build fills in the version and the shell list', { skip: !existsSync(built) && 'run npm run build first' }, async () => {
  const source = readFileSync(built, 'utf8');
  assert.ok(!source.includes('__KWNT_SW_'), 'no placeholders left');
  const version = source.match(/const VERSION = '([0-9a-f]{12})'/);
  assert.ok(version, 'a 12-character build version');
  const shell = JSON.parse(source.match(/const SHELL = (\[.*\]);/)[1]);
  assert.ok(shell.includes('/index.html'));
  assert.ok(shell.includes('/manifest.webmanifest'));
  assert.ok(shell.some((p) => /^\/assets\/index-.*\.js$/.test(p)));
  assert.ok(!shell.some((p) => p.startsWith('/api')), 'the API is never part of the shell');
  assert.ok(!shell.some((p) => p.endsWith('.map') || p === '/sw.js'));
  for (const p of shell) assert.ok(existsSync(`${root}/dist${p}`), `${p} is in dist`);
  // The built worker keeps the same rules as the template
  const sw = loadWorker({ source: source.replace(version[1], '__KWNT_SW_VERSION__').replace(/const SHELL = \[.*\];/, 'const SHELL = __KWNT_SW_SHELL__;') });
  await sw.install();
  assert.equal(await sw.fetch(sw.request('/api/spaces')), undefined);
});

test('the manifest makes kwant bots installable, and every icon it names exists', () => {
  const m = JSON.parse(readFileSync(`${root}/public/manifest.webmanifest`, 'utf8'));
  assert.equal(m.name, 'kwant bots');
  assert.equal(m.short_name, 'kwant bots');
  assert.equal(m.start_url, '/');
  assert.equal(m.scope, '/');
  assert.equal(m.display, 'standalone');
  assert.match(m.theme_color, /^#[0-9a-f]{6}$/i);
  assert.match(m.background_color, /^#[0-9a-f]{6}$/i);
  const sizes = (purpose) => m.icons.filter((i) => (i.purpose ?? 'any').split(' ').includes(purpose)).map((i) => i.sizes);
  for (const s of ['192x192', '512x512']) {
    assert.ok(sizes('any').includes(s), `${s} icon`);
    assert.ok(sizes('maskable').includes(s), `${s} maskable icon`);
  }
  for (const i of m.icons) assert.ok(existsSync(`${root}/public${i.src}`), `${i.src} exists`);

  const html = readFileSync(`${root}/index.html`, 'utf8');
  assert.match(html, /<link rel="manifest" href="\/manifest\.webmanifest"/);
  assert.match(html, /<link rel="apple-touch-icon" href="\/icons\/apple-touch-icon\.png"/);
  assert.match(html, /name="apple-mobile-web-app-capable" content="yes"/);
  assert.match(html, new RegExp(`name="theme-color" content="${m.theme_color}"`));
  assert.match(html, /viewport-fit=cover/);
  assert.ok(existsSync(`${root}/public/icons/apple-touch-icon.png`));
});

const pwa = await loadBundled(`export * from './src/pwa.ts'; export { offlineMessage } from './src/components/AuthGate.tsx';`);

test('the install hint: install button on Android, Share steps on iPhone, nothing otherwise', () => {
  const base = { standalone: false, dismissed: false, phone: true, secure: true, ios: false, canPrompt: false };
  assert.equal(pwa.installHintKind({ ...base, canPrompt: true }), 'prompt');
  assert.equal(pwa.installHintKind({ ...base, ios: true }), 'ios');
  assert.equal(pwa.installHintKind(base), null, 'no install route to offer');
  assert.equal(pwa.installHintKind({ ...base, canPrompt: true, standalone: true }), null, 'already installed');
  assert.equal(pwa.installHintKind({ ...base, ios: true, dismissed: true }), null, 'dismissed');
  assert.equal(pwa.installHintKind({ ...base, canPrompt: true, phone: false }), null, 'not on a phone');
  assert.equal(pwa.installHintKind({ ...base, ios: true, secure: false }), null, 'plain http cannot install');
});

test('iPhone and iPad are recognised, including iPadOS calling itself a Mac', () => {
  assert.ok(pwa.isIos({ userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)', maxTouchPoints: 5 }));
  assert.ok(pwa.isIos({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari', maxTouchPoints: 5 }));
  assert.ok(!pwa.isIos({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari', maxTouchPoints: 0 }));
  assert.ok(!pwa.isIos({ userAgent: 'Mozilla/5.0 (Linux; Android 15; Pixel 9) Chrome/154', maxTouchPoints: 5 }));
});

const UA = {
  macSafari: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Safari/605.1.15',
  macChrome: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36',
  macEdge: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0',
  macFirefox: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:144.0) Gecko/20100101 Firefox/144.0',
  macApp: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)',
  winChrome: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36',
};

test('Safari on a Mac and the Mac app web view are told apart from Chrome, Edge and Firefox', () => {
  const nav = (userAgent, maxTouchPoints = 0) => ({ userAgent, maxTouchPoints });
  assert.ok(pwa.isMacSafari(nav(UA.macSafari)));
  for (const k of ['macChrome', 'macEdge', 'macFirefox', 'macApp', 'winChrome']) assert.ok(!pwa.isMacSafari(nav(UA[k])), k);
  assert.ok(!pwa.isMacSafari(nav(UA.macSafari, 5)), 'an iPad says it is a Mac');
  assert.ok(pwa.isEmbeddedWebView(nav(UA.macApp)));
  for (const k of ['macSafari', 'macChrome', 'macEdge', 'macFirefox', 'winChrome']) assert.ok(!pwa.isEmbeddedWebView(nav(UA[k])), k);
});

test('the desktop Install app entry: one click, Safari steps, https note or browser menu', () => {
  const base = { standalone: false, embedded: false, secure: true, canPrompt: false, macSafari: false, ios: false };
  assert.equal(pwa.desktopInstallKind({ ...base, canPrompt: true }), 'prompt');
  assert.equal(pwa.desktopInstallKind({ ...base, macSafari: true }), 'safari');
  assert.equal(pwa.desktopInstallKind({ ...base, ios: true }), 'ios');
  assert.equal(pwa.desktopInstallKind(base), 'manual', 'Firefox, or Chrome before it offers');
  assert.equal(pwa.desktopInstallKind({ ...base, secure: false, canPrompt: true }), 'insecure', 'plain http says why');
  assert.equal(pwa.desktopInstallKind({ ...base, canPrompt: true, standalone: true }), null, 'already installed');
  assert.equal(pwa.desktopInstallKind({ ...base, embedded: true, macSafari: true }), null, 'the Mac app');
});

test('the install offer is kept from page load and cleared once used or installed', async () => {
  const handlers = {};
  const win = {
    navigator: {},
    matchMedia: () => ({ matches: false }),
    addEventListener: (type, fn) => { handlers[type] = fn; },
  };
  pwa.captureInstallPrompt(win);
  let changes = 0;
  const off = pwa.subscribeInstallState(() => changes++);
  assert.deepEqual(pwa.getInstallState(), { prompt: null, installed: false });

  let prevented = false;
  let shown = 0;
  const event = {
    preventDefault: () => { prevented = true; },
    prompt: async () => { shown++; },
    userChoice: Promise.resolve({ outcome: 'dismissed' }),
  };
  handlers.beforeinstallprompt(event);
  assert.ok(prevented, "Chrome's own mini bar stays away");
  assert.equal(pwa.getInstallState().prompt, event);

  await pwa.runInstallPrompt();
  assert.equal(shown, 1);
  assert.deepEqual(pwa.getInstallState(), { prompt: null, installed: false }, 'a prompt works once');

  handlers.beforeinstallprompt({ ...event, userChoice: Promise.resolve({ outcome: 'accepted' }) });
  await pwa.runInstallPrompt();
  assert.equal(pwa.getInstallState().installed, true);

  handlers.appinstalled();
  assert.deepEqual(pwa.getInstallState(), { prompt: null, installed: true });
  assert.ok(changes >= 4);
  off();
});

test('the Mac download goes to the newest release', () => {
  assert.equal(pwa.MAC_DOWNLOAD_URL, 'https://github.com/alexsergivan/kwant/releases/latest');
});

test('standalone means the installed app, on iOS or Android', () => {
  const win = (standalone, matches) => ({ navigator: { standalone }, matchMedia: () => ({ matches }) });
  assert.ok(pwa.isStandalone(win(true, false)));
  assert.ok(pwa.isStandalone(win(undefined, true)));
  assert.ok(!pwa.isStandalone(win(undefined, false)));
});

test('the service worker registers only in a build over HTTPS or localhost', () => {
  const win = (secure) => {
    const calls = [];
    return {
      calls,
      isSecureContext: secure,
      document: { readyState: 'complete' },
      addEventListener: () => {},
      navigator: { serviceWorker: { register: (...a) => { calls.push(a); return Promise.resolve(); } } },
    };
  };
  const ok = win(true);
  assert.equal(pwa.registerServiceWorker(ok, true), true);
  assert.deepEqual(ok.calls, [['/sw.js', { scope: '/' }]]);
  const dev = win(true);
  assert.equal(pwa.registerServiceWorker(dev, false), false);
  assert.deepEqual(dev.calls, []);
  const http = win(false);
  assert.equal(pwa.registerServiceWorker(http, true), false);
  assert.deepEqual(http.calls, []);
});

test('the offline screen says offline, or that the server cannot be reached, never a raw fetch error', () => {
  assert.match(pwa.offlineMessage(new TypeError('Failed to fetch'), false), /You're offline/);
  assert.match(pwa.offlineMessage(new TypeError('Failed to fetch'), true), /Can't reach the kwant bots server/);
  assert.match(pwa.offlineMessage(new TypeError('Load failed'), true), /Can't reach the kwant bots server/);
  assert.equal(pwa.offlineMessage(new Error('Server returned 502'), true), 'Server returned 502');
});
