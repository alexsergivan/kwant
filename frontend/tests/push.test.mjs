// Notifications on this device: the push helpers (src/push.ts) and the service worker's push,
// notificationclick and pushsubscriptionchange handlers (sw.js).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { loadBundled } from './bundle.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const ORIGIN = 'https://kwant.example';
const p = await loadBundled(`export * from './src/push.ts';`);

test('the page uses the same tag as the server, so a push replaces the page’s notification', () => {
  // Same cases as payload_shape_ttl_and_urgency in src/server/push.rs
  assert.equal(p.notificationTag({ space_id: 'sp 1', kind: 'approval_needed', channel_id: 'c1', kwant_id: 'k1' }), 'kwnt-sp 1-approval_needed-c1');
  assert.equal(p.notificationTag({ kind: 'turn_finished', channel_id: null, kwant_id: 'k1' }), 'kwnt-none-turn_finished-k1');
  assert.equal(p.notificationTag({ space_id: null, kind: 'turn_finished', channel_id: null, kwant_id: 'k1' }), 'kwnt-none-turn_finished-k1');
});

test('a notification opens the chat’s own address', () => {
  assert.equal(p.notificationPath({ space_id: 'sp 1', channel_id: 'c/1', kwant_id: 'k1' }), '/s/sp%201/c/c%2F1');
  assert.equal(p.notificationPath({ space_id: 's', channel_id: null, kwant_id: 'k1' }), '/s/s/b/k1');
  assert.equal(p.notificationPath({ space_id: null, channel_id: 'c1', kwant_id: 'k1' }), '/');
});

test('the push url’s query works as a link too', () => {
  assert.deepEqual(p.targetFromQuery('?space=sp%201&channel=c1'), { spaceId: 'sp 1', channelId: 'c1', botId: null });
  assert.deepEqual(p.targetFromQuery('?space=s&kwant=k1'), { spaceId: 's', channelId: null, botId: 'k1' });
  assert.deepEqual(p.targetFromQuery('?kwant=k1'), { spaceId: null, channelId: null, botId: 'k1' });
  assert.equal(p.targetFromQuery(''), null);
  assert.equal(p.targetFromQuery('?utm=x'), null);
});

test('what this device can do: push, page only, or why not', () => {
  const env = { secure: true, ios: false, standalone: false, notification: true, serviceWorker: true, pushManager: true };
  assert.equal(p.deviceNotifySupport(env), 'push', 'Android Chrome, desktop Chrome/Firefox/Safari');
  assert.equal(p.deviceNotifySupport({ ...env, secure: false }), 'insecure');
  assert.equal(p.deviceNotifySupport({ ...env, ios: true, notification: false, pushManager: false }), 'ios-install', 'Safari tab on an iPhone');
  assert.equal(p.deviceNotifySupport({ ...env, ios: true, standalone: true }), 'push', 'Home Screen app on iOS 16.4+');
  assert.equal(p.deviceNotifySupport({ ...env, ios: true, standalone: true, pushManager: false, notification: false }), 'unsupported', 'older iOS');
  assert.equal(p.deviceNotifySupport({ ...env, pushManager: false }), 'page');
  assert.equal(p.deviceNotifySupport({ ...env, serviceWorker: false, pushManager: false, notification: false }), 'unsupported');
});

test('the line under the switch says what it does or how to fix it', () => {
  const base = { support: 'push', permission: 'default', on: false, android: false, standalone: false };
  assert.match(p.deviceNotifyHint(base).text, /even with the app closed/);
  assert.equal(p.deviceNotifyHint(base).warn, false);
  assert.match(p.deviceNotifyHint({ ...base, on: true, permission: 'granted' }).text, /even when the app is closed/);
  assert.match(p.deviceNotifyHint({ ...base, support: 'page' }).text, /Only while this tab is open/);
  const insecure = p.deviceNotifyHint({ ...base, support: 'insecure' });
  assert.ok(insecure.warn && /https/.test(insecure.text));
  const ios = p.deviceNotifyHint({ ...base, support: 'ios-install', permission: 'unsupported' });
  assert.ok(ios.warn && /Add to Home Screen/.test(ios.text));
  const deniedAndroidApp = p.deviceNotifyHint({ ...base, permission: 'denied', android: true, standalone: true });
  assert.ok(deniedAndroidApp.warn && /App info/.test(deniedAndroidApp.text));
  assert.match(p.deviceNotifyHint({ ...base, permission: 'denied' }).text, /site settings/);
  assert.match(p.deviceNotifyHint({ ...base, support: 'unsupported', permission: 'unsupported' }).text, /can't show/);
});

test('the server key decodes from base64url, with or without padding', () => {
  const bytes = Uint8Array.from({ length: 65 }, (_, i) => (i * 37 + 4) & 255);
  const b64url = Buffer.from(bytes).toString('base64url');
  assert.deepEqual([...p.base64UrlToBytes(b64url)], [...bytes]);
  assert.deepEqual([...p.base64UrlToBytes(b64url + '=')], [...bytes]);
  assert.ok(p.sameKey(bytes.buffer, bytes));
  assert.ok(!p.sameKey(bytes.slice(1).buffer, bytes));
  assert.ok(!p.sameKey(null, bytes));
});

/** A fake server and pushManager. */
function fakes({ key = 'AQID', existing = null, subscribeStatus = 204, keyStatus = 200 } = {}) {
  const calls = [];
  const fetchFn = async (url, init = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body ? JSON.parse(init.body) : undefined });
    if (url === '/api/push/key') return new Response(JSON.stringify({ public_key: key }), { status: keyStatus });
    if (url === '/api/push/subscribe') {
      return subscribeStatus === 204 ? new Response(null, { status: 204 }) : new Response(JSON.stringify({ error: 'endpoint must be https' }), { status: subscribeStatus });
    }
    return new Response(null, { status: 204 });
  };
  const made = [];
  const sub = (endpoint, keyBytes) => ({
    endpoint,
    options: { applicationServerKey: keyBytes.buffer },
    unsubscribed: false,
    async unsubscribe() { this.unsubscribed = true; current = null; return true; },
    toJSON() { return { endpoint, expirationTime: null, keys: { p256dh: 'pk', auth: 'au' } }; },
  });
  let current = existing ? sub(existing.endpoint, existing.key) : null;
  const old = current;
  const reg = {
    pushManager: {
      getSubscription: async () => current,
      subscribe: async (opts) => {
        made.push(opts);
        current = sub(`https://fcm.googleapis.com/fcm/send/${made.length}`, opts.applicationServerKey);
        return current;
      },
    },
  };
  return { calls, fetchFn, reg, made, old, current: () => current };
}

test('turning it on subscribes with the server key and tells the server', async () => {
  const f = fakes();
  const sub = await p.subscribePush(f.reg, f.fetchFn);
  assert.equal(f.made.length, 1);
  assert.equal(f.made[0].userVisibleOnly, true);
  assert.deepEqual([...f.made[0].applicationServerKey], [1, 2, 3]);
  assert.deepEqual(f.calls.map((c) => `${c.method} ${c.url}`), ['GET /api/push/key', 'POST /api/push/subscribe']);
  assert.deepEqual(f.calls[1].body, sub.toJSON());
});

test('an existing subscription for the same key is reused; one for an old key is replaced', async () => {
  const same = fakes({ existing: { endpoint: 'https://push.example/a', key: new Uint8Array([1, 2, 3]) } });
  await p.subscribePush(same.reg, same.fetchFn);
  assert.equal(same.made.length, 0);
  assert.equal(same.calls[1].body.endpoint, 'https://push.example/a');

  const rotated = fakes({ existing: { endpoint: 'https://push.example/old', key: new Uint8Array([9, 9, 9]) } });
  await p.subscribePush(rotated.reg, rotated.fetchFn);
  assert.ok(rotated.old.unsubscribed, 'the old-key subscription is dropped');
  assert.equal(rotated.made.length, 1);
  assert.notEqual(rotated.calls[1].body.endpoint, 'https://push.example/old');
});

test('a server refusal surfaces its message', async () => {
  const f = fakes({ subscribeStatus: 400 });
  await assert.rejects(p.subscribePush(f.reg, f.fetchFn), /endpoint must be https/);
  const noKey = fakes({ keyStatus: 401 });
  await assert.rejects(p.subscribePush(noKey.reg, noKey.fetchFn), /HTTP 401/);
});

test('turning it off unsubscribes on the server and in the browser', async () => {
  const f = fakes({ existing: { endpoint: 'https://push.example/a', key: new Uint8Array([1, 2, 3]) } });
  await p.unsubscribePush(f.reg, f.fetchFn);
  assert.deepEqual(f.calls, [{ url: '/api/push/unsubscribe', method: 'POST', body: { endpoint: 'https://push.example/a' } }]);
  assert.ok(f.old.unsubscribed);
  const none = fakes();
  await p.unsubscribePush(none.reg, none.fetchFn);
  assert.deepEqual(none.calls, [], 'nothing to drop, no request');
});

test('on load, an enabled device re-syncs its subscription; anything else is left alone', async () => {
  const f = fakes({ existing: { endpoint: 'https://push.example/a', key: new Uint8Array([1, 2, 3]) } });
  const opts = { enabled: true, permission: 'granted', support: 'push', registration: async () => f.reg, fetchFn: f.fetchFn };
  assert.equal(await p.resyncPush(opts), 'synced');
  assert.equal(f.calls.at(-1).url, '/api/push/subscribe');
  for (const off of [{ enabled: false }, { permission: 'default' }, { permission: 'denied' }, { support: 'page' }, { support: 'ios-install' }]) {
    const g = fakes();
    assert.equal(await p.resyncPush({ ...opts, ...off, registration: async () => g.reg, fetchFn: g.fetchFn }), 'skipped', JSON.stringify(off));
    assert.equal(g.calls.length, 0);
  }
  assert.equal(await p.resyncPush({ ...opts, registration: async () => null }), 'skipped', 'no service worker (dev server)');
  const bad = fakes({ subscribeStatus: 400 });
  assert.equal(await p.resyncPush({ ...opts, registration: async () => bad.reg, fetchFn: bad.fetchFn }), 'failed');
});

test('the page shows its notification through the service worker, with the push’s tag and target', async () => {
  const shown = [];
  const reg = { showNotification: async (title, opts) => shown.push({ title, opts }) };
  const nav = { serviceWorker: { getRegistration: async () => reg } };
  let clicked = 0;
  await p.showDeviceNotification(
    { kind: 'approval_needed', kwant_id: 'k1', kwant_name: 'Ada', channel_id: 'c1', title: 'Ada needs approval', body: 'Run tests?', space_id: 's1' },
    () => clicked++,
    nav,
  );
  assert.equal(shown.length, 1);
  assert.equal(shown[0].title, 'Ada needs approval');
  assert.equal(shown[0].opts.tag, 'kwnt-s1-approval_needed-c1');
  assert.deepEqual(shown[0].opts.data, { path: '/s/s1/c/c1', kind: 'approval_needed', space_id: 's1', channel_id: 'c1', kwant_id: 'k1' });
  assert.equal(clicked, 0, 'the tap goes through sw.js');
  assert.ok(p.isNotificationOpen({ type: 'kwnt-notification-open' }));
  assert.ok(!p.isNotificationOpen({ type: 'other' }) && !p.isNotificationOpen(null));
});

// ---------------------------------------------------------------------------
// sw.js

/** Objects made inside the worker's vm context, compared as plain data. */
const plain = (v) => JSON.parse(JSON.stringify(v));

function loadWorker({ clients = [] } = {}) {
  const code = readFileSync(`${root}/sw.js`, 'utf8').replace('__KWNT_SW_VERSION__', 'v').replace('__KWNT_SW_SHELL__', '[]');
  const handlers = {};
  const log = { shown: [], opened: [], posted: [], focused: [], fetched: [], subscribed: [] };
  const win = (url, { focused = false, visible = false } = {}) => ({
    url: new URL(url, ORIGIN).href,
    focused,
    visibilityState: visible ? 'visible' : 'hidden',
    postMessage: (m) => log.posted.push({ url, m }),
    focus: async function () { log.focused.push(url); return this; },
  });
  const list = clients.map((c) => win(c.url, c));
  const self = {
    location: { origin: ORIGIN },
    addEventListener: (type, fn) => { handlers[type] = fn; },
    registration: {
      showNotification: async (title, opts) => { log.shown.push({ title, ...opts }); },
      pushManager: { subscribe: async (opts) => { log.subscribed.push(opts); return { endpoint: 'https://push.example/new', toJSON: () => ({ endpoint: 'https://push.example/new' }) }; } },
    },
    clients: {
      matchAll: async () => list,
      openWindow: async (url) => { log.opened.push(url); },
      claim: async () => undefined,
    },
  };
  vm.runInNewContext(code, {
    self,
    URL,
    caches: {},
    fetch: async (url, init) => { log.fetched.push({ url, body: JSON.parse(init.body) }); return { ok: true }; },
  });
  const run = async (type, extra) => {
    let waited;
    handlers[type]({ ...extra, waitUntil: (pr) => { waited = pr; } });
    await waited;
  };
  return {
    log,
    push: (payload) => run('push', { data: payload === undefined ? null : { json: () => (typeof payload === 'string' ? JSON.parse(payload) : payload) } }),
    click: (data) => {
      const n = { data, closed: false, close() { this.closed = true; } };
      return run('notificationclick', { notification: n }).then(() => n);
    },
    change: (extra) => run('pushsubscriptionchange', extra),
  };
}

const PAYLOAD = {
  title: 'Ada needs approval',
  body: 'Run the tests?',
  kind: 'approval_needed',
  space_id: 's1',
  channel_id: 'c1',
  kwant_id: 'k1',
  url: '/?space=s1&channel=c1',
  tag: 'kwnt-s1-approval_needed-c1',
};

test('a push with kwant closed shows a notification with the server’s tag and the chat to open', async () => {
  const sw = loadWorker();
  await sw.push(PAYLOAD);
  assert.equal(sw.log.shown.length, 1);
  const n = sw.log.shown[0];
  assert.equal(n.title, 'Ada needs approval');
  assert.equal(n.body, 'Run the tests?');
  assert.equal(n.tag, 'kwnt-s1-approval_needed-c1');
  assert.equal(n.icon, '/icons/icon-192.png');
  assert.deepEqual(plain(n.data), { path: '/s/s1/c/c1', kind: 'approval_needed', space_id: 's1', channel_id: 'c1', kwant_id: 'k1' });
});

test('a push while kwant is in front is left to the app; a background tab still gets it', async () => {
  const front = loadWorker({ clients: [{ url: '/s/s1/c/other', focused: true, visible: true }] });
  await front.push(PAYLOAD);
  assert.equal(front.log.shown.length, 0);
  const behind = loadWorker({ clients: [{ url: '/s/s1/c/c1', focused: false, visible: false }] });
  await behind.push(PAYLOAD);
  assert.equal(behind.log.shown.length, 1);
});

test('odd pushes still show something safe', async () => {
  const sw = loadWorker();
  await sw.push(undefined);
  await sw.push({ title: '', url: '//evil.example/x' });
  await sw.push({ title: 'x', kwant_id: 'k1', url: '/?kwant=k1' });
  assert.deepEqual(sw.log.shown.map((n) => [n.title, n.data.path]), [['kwant', '/'], ['kwant', '/'], ['x', '/?kwant=k1']]);
});

test('a tap focuses the open app and asks it to open the chat', async () => {
  const sw = loadWorker({ clients: [{ url: '/s/s1/c/zz' }] });
  const n = await sw.click({ path: '/s/s1/c/c1', kind: 'question_asked', space_id: 's1', channel_id: 'c1', kwant_id: 'k1' });
  assert.ok(n.closed);
  assert.deepEqual(plain(sw.log.posted.map((x) => x.m)), [{ type: 'kwnt-notification-open', kind: 'question_asked', space_id: 's1', channel_id: 'c1', kwant_id: 'k1' }]);
  assert.deepEqual(sw.log.focused, ['/s/s1/c/zz']);
  assert.deepEqual(sw.log.opened, []);
});

test('a tap with kwant closed opens it on the chat, and never off-site', async () => {
  const sw = loadWorker();
  await sw.click({ path: '/s/s1/b/k1', kind: 'turn_finished', space_id: 's1', kwant_id: 'k1' });
  await sw.click({ path: '//evil.example/' });
  await sw.click(undefined);
  assert.deepEqual(sw.log.opened, ['/s/s1/b/k1', '/', '/']);
});

test('a replaced subscription is sent to the server and the old one dropped', async () => {
  const sw = loadWorker();
  await sw.change({ oldSubscription: { endpoint: 'https://push.example/old', options: { userVisibleOnly: true, applicationServerKey: 'k' } } });
  assert.equal(sw.log.subscribed.length, 1);
  assert.deepEqual(sw.log.fetched, [
    { url: '/api/push/subscribe', body: { endpoint: 'https://push.example/new' } },
    { url: '/api/push/unsubscribe', body: { endpoint: 'https://push.example/old' } },
  ]);
});
