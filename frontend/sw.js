// kwant bots service worker: keeps the app shell (index.html, the built JS/CSS, icons) on the
// device so the installed app opens fast and shows "can't reach the server" instead of a blank
// page when the server is down. It never stores anything else: the API, the event stream, the
// browser stream, uploads and other sites always go straight to the network, untouched.
//
// vite.config.ts fills in the version and the file list for each build (see pwaServiceWorker),
// so a new build installs a new cache and drops the old one.

const VERSION = '__KWNT_SW_VERSION__';
const SHELL = __KWNT_SW_SHELL__;
const CACHE = `kwnt-shell-${VERSION}`;
const INDEX = '/index.html';

/** Whether a request is one the worker answers. Anything else is left to the network. */
function handles(request, origin) {
  if (request.method !== 'GET') return false;
  const url = new URL(request.url);
  if (url.origin !== origin) return false;
  if (url.pathname === '/api' || url.pathname.startsWith('/api/')) return false;
  if (request.headers.get('range')) return false;
  if ((request.headers.get('accept') || '').includes('text/event-stream')) return false;
  if (request.mode === 'navigate') return true;
  return SHELL.includes(url.pathname);
}

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(SHELL))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith('kwnt-shell-') && k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const { request } = event;
  if (!handles(request, self.location.origin)) return;
  if (request.mode === 'navigate') {
    // The page itself: the server first, so an update shows on the next load; the stored copy
    // only when the server can't be reached. The app then says so itself.
    event.respondWith(
      fetch(request).catch(() =>
        caches.open(CACHE).then((cache) => cache.match(INDEX)).then((hit) => hit || Response.error()),
      ),
    );
    return;
  }
  // Built files have a hash in their name and never change: the stored copy first.
  event.respondWith(
    caches.open(CACHE).then((cache) => cache.match(request).then((hit) => hit || fetch(request))),
  );
});
