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

// ---------------------------------------------------------------------------
// Notifications. The server pushes {title, body, kind, space_id, channel_id, kwant_id, url, tag}
// (src/server/push.rs) to each device that turned on "Notify me on this device"; the open page
// shows its own through showNotification with the same tag and data (src/push.ts).

/** The chat's address in the app: /s/<space>/c/<channel> or /s/<space>/b/<bot>. */
function notificationPath(p) {
  if (!p.space_id) return typeof p.url === 'string' && p.url.startsWith('/') && !p.url.startsWith('//') ? p.url : '/';
  const base = `/s/${encodeURIComponent(p.space_id)}`;
  if (p.channel_id) return `${base}/c/${encodeURIComponent(p.channel_id)}`;
  if (p.kwant_id) return `${base}/b/${encodeURIComponent(p.kwant_id)}`;
  return base;
}

function windows() {
  return self.clients.matchAll({ type: 'window', includeUncontrolled: true });
}

/** kwant is open and in front of the person: the app shows it itself (a toast, the chat). */
function inFront(client) {
  return client.focused && client.visibilityState === 'visible';
}

self.addEventListener('push', (event) => {
  let p = {};
  try {
    p = event.data ? event.data.json() : {};
  } catch {
    p = {};
  }
  const title = typeof p.title === 'string' && p.title ? p.title : 'kwant';
  const data = {
    path: notificationPath(p),
    kind: p.kind || null,
    space_id: p.space_id || null,
    channel_id: p.channel_id || null,
    kwant_id: p.kwant_id || null,
  };
  event.waitUntil(
    windows().then((list) => {
      if (list.some(inFront)) return undefined;
      return self.registration.showNotification(title, {
        body: typeof p.body === 'string' ? p.body : '',
        tag: typeof p.tag === 'string' && p.tag ? p.tag : undefined,
        icon: '/icons/icon-192.png',
        badge: '/icons/icon-192.png',
        data,
      });
    }),
  );
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const d = event.notification.data || {};
  const path = typeof d.path === 'string' && d.path.startsWith('/') && !d.path.startsWith('//') ? d.path : '/';
  event.waitUntil(
    windows().then((list) => {
      const client = list.find((c) => new URL(c.url).origin === self.location.origin);
      if (!client) return self.clients.openWindow(path);
      // The open app switches to the chat itself: no reload, and it can open a question or step.
      client.postMessage({
        type: 'kwnt-notification-open',
        kind: d.kind || '',
        space_id: d.space_id || null,
        channel_id: d.channel_id || null,
        kwant_id: d.kwant_id || null,
      });
      return client.focus().catch(() => self.clients.openWindow(path));
    }),
  );
});

// The browser replaced the subscription (expiry, key rotation): tell the server about the new one.
self.addEventListener('pushsubscriptionchange', (event) => {
  const old = event.oldSubscription;
  const options = (old && old.options) || (event.newSubscription && event.newSubscription.options);
  const post = (path, body) =>
    fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, credentials: 'same-origin', body: JSON.stringify(body) });
  event.waitUntil(
    Promise.resolve(event.newSubscription || (options ? self.registration.pushManager.subscribe(options) : null))
      .then((sub) => {
        if (!sub) return undefined;
        return post('/api/push/subscribe', sub.toJSON()).then(() =>
          old && old.endpoint !== sub.endpoint ? post('/api/push/unsubscribe', { endpoint: old.endpoint }) : undefined,
        );
      })
      .catch(() => undefined),
  );
});
