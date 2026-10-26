import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadBundled } from './bundle.mjs';

// The markdown renderer pulls in the UI sounds, which read localStorage when loaded.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const u = await loadBundled(`export * from './src/appUpdate';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { AppUpdateBannerView, AppUpdateAboutView } from './src/components/AppUpdateBanner';
  export const banner = (props) => renderToStaticMarkup(React.createElement(AppUpdateBannerView, props));
  export const about = (props) => renderToStaticMarkup(React.createElement(AppUpdateAboutView, props));
`);
const src = (path) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8');

const NOTES = '### Added\n- **In-app updates.**';
const status = (over = {}) => ({
  supported: true,
  current: '0.1.2',
  state: 'available',
  available: { version: '0.1.3', notes: NOTES, date: '2026-10-27T10:00:00Z', url: 'https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.3' },
  progress: null,
  error: null,
  ...over,
});
const model = (raw, over = {}) => ({
  status: u.parseUpdateStatus(raw),
  unsupported: false,
  phase: 'idle',
  target: null,
  actionError: null,
  installError: null,
  checkedAt: 1,
  ...over,
});
const memStore = (init = {}) => {
  const m = new Map(Object.entries(init));
  return { m, getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, String(v)), removeItem: (k) => m.delete(k) };
};
const json = (body, code = 200) => ({ ok: code >= 200 && code < 300, status: code, json: async () => body });

/** A fake server and clock: records every request and every scheduled poll. */
function harness(answers, store = memStore()) {
  const calls = [];
  const timers = [];
  let now = 0;
  const ctrl = u.createUpdateController({
    fetch: async (url, init = {}) => {
      calls.push(`${init.method ?? 'GET'} ${url}`);
      const a = answers(url, init, calls.length);
      if (a instanceof Error) throw a;
      return a;
    },
    setTimeout: (fn, ms) => {
      const t = { fn, ms, live: true };
      timers.push(t);
      return t;
    },
    clearTimeout: (t) => t && (t.live = false),
    now: () => now,
    store,
  });
  const live = () => timers.filter((t) => t.live);
  const tick = async (advance = 0) => {
    now += advance;
    const t = live()[0];
    assert.ok(t, 'a poll is scheduled');
    t.live = false;
    t.fn();
    await flush();
  };
  return { ctrl, calls, live, tick, store, advance: (ms) => (now += ms) };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

test('parses the contract; supported=false and junk mean not the Mac app', () => {
  const s = u.parseUpdateStatus(status({ progress: 1.7 }));
  assert.equal(s.current, '0.1.2');
  assert.equal(s.available.version, '0.1.3');
  assert.equal(s.progress, 1);
  assert.equal(u.parseUpdateStatus(status({ state: 'weird' })).state, 'idle');
  assert.equal(u.parseUpdateStatus({ ...status(), supported: false }), null);
  assert.equal(u.parseUpdateStatus(null), null);
  assert.equal(u.parseUpdateStatus('nope'), null);
  // A non-https link never becomes the Download manually href.
  const odd = u.parseUpdateStatus(status({ available: { version: '0.1.3', notes: '', url: 'javascript:alert(1)' } }));
  assert.equal(odd.available.url, 'https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.3');
});

test('banner for each state', () => {
  assert.equal(u.bannerView(model(status({ state: 'idle', available: null })), null), null);
  assert.equal(u.bannerView(model(status({ state: 'checking', available: null })), null), null);
  assert.deepEqual(u.bannerView(model(status()), null), { kind: 'available', version: '0.1.3', notes: NOTES, url: status().available.url });
  assert.deepEqual(u.bannerView(model(status({ state: 'downloading', progress: 0.43 })), null), { kind: 'downloading', version: '0.1.3', progress: 0.43 });
  assert.equal(u.bannerView(model(status({ state: 'ready' })), null).kind, 'restarting');
  const err = u.bannerView(model(status({ state: 'error', error: 'The signature check failed.' })), null);
  assert.equal(err.kind, 'error');
  assert.equal(err.error, 'The signature check failed.');
  assert.equal(err.url, status().available.url);
  // A failed check (nothing on offer) stays in Settings, not in the banner.
  assert.equal(u.bannerView(model(status({ state: 'error', available: null, error: 'offline' })), null), null);
  assert.equal(u.bannerView(model(status(), { phase: 'restarting', target: '0.1.3' }), null).kind, 'restarting');
  assert.deepEqual(u.bannerView(model(status({ state: 'idle', available: null }), { phase: 'updated', target: '0.1.3' }), null), { kind: 'updated', version: '0.1.3' });
  assert.equal(u.bannerView(model(status(), { phase: 'stalled', target: '0.1.3' }), null).kind, 'error');
});

test('dismiss hides that version only, and storage that throws is fine', () => {
  const store = memStore();
  u.dismissVersion('0.1.3', store);
  assert.equal(u.dismissedVersion(store), '0.1.3');
  assert.equal(u.bannerView(model(status()), u.dismissedVersion(store)), null);
  const next = status({ available: { ...status().available, version: '0.1.4' } });
  assert.equal(u.bannerView(model(next), u.dismissedVersion(store)).version, '0.1.4');
  // A dismissed version that then fails to install still shows the error.
  assert.equal(u.bannerView(model(status({ state: 'error', error: 'x' })), '0.1.3').kind, 'error');
  const blocked = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); }, removeItem: () => { throw new Error('denied'); } };
  assert.equal(u.dismissedVersion(blocked), null);
  assert.doesNotThrow(() => u.dismissVersion('0.1.3', blocked));
});

test('unsupported (404 or supported=false) renders nothing and never polls again', async () => {
  for (const answer of [json({ error: 'not found' }, 404), json({ ...status(), supported: false })]) {
    const h = harness(() => answer);
    h.ctrl.start();
    await flush();
    assert.equal(h.ctrl.get().unsupported, true);
    assert.equal(h.live().length, 0, 'no timer left');
    assert.deepEqual(h.calls, ['GET /api/app/update']);
    assert.equal(u.bannerView(h.ctrl.get(), null), null);
    assert.equal(ui.about({ model: h.ctrl.get(), onCheck() {}, onUpdate() {} }), '');
    await h.ctrl.check();
    await h.ctrl.install();
    assert.deepEqual(h.calls, ['GET /api/app/update'], 'check/install do nothing off the Mac app');
  }
});

test('polls on load and every 30 minutes; 1 s while something moves', async () => {
  const h = harness(() => json(status({ state: 'idle', available: null })));
  h.ctrl.start();
  h.ctrl.start();
  await flush();
  assert.deepEqual(h.calls, ['GET /api/app/update'], 'start twice = one poller');
  assert.equal(h.live().length, 1);
  assert.equal(h.live()[0].ms, 30 * 60 * 1000);
  assert.equal(u.nextPollMs(model(status({ state: 'downloading' }))), 1000);
  assert.equal(u.nextPollMs(model(status({ state: 'checking' }))), 1000);
  assert.equal(u.nextPollMs(model(status(), { phase: 'restarting' })), 1000);
  h.ctrl.stop();
  assert.equal(h.live().length, 0);
});

test('install: downloading → restarting while the server is gone → updated when it is back', async () => {
  let phase = 'available';
  const store = memStore();
  const h = harness((url, init) => {
    if (init.method === 'POST') {
      phase = 'downloading';
      return json(status({ state: 'downloading', progress: 0 }), 202);
    }
    if (phase === 'downloading') {
      phase = 'ready';
      return json(status({ state: 'downloading', progress: 0.5 }));
    }
    if (phase === 'ready') {
      phase = 'gone';
      return json(status({ state: 'ready', progress: 1 }));
    }
    if (phase === 'gone') {
      phase = 'back';
      return new TypeError('Failed to fetch');
    }
    return json(phase === 'back' ? status({ current: '0.1.3', state: 'idle', available: null }) : status());
  }, store);
  h.ctrl.start();
  await flush();
  assert.equal(u.bannerView(h.ctrl.get(), null).kind, 'available');
  await h.ctrl.install();
  assert.ok(h.calls.includes('POST /api/app/update/install'));
  assert.equal(store.m.get(u.UPDATE_PENDING_KEY), '0.1.3');
  assert.equal(u.bannerView(h.ctrl.get(), null).kind, 'downloading');
  assert.equal(h.live().length, 1);
  assert.equal(h.live()[0].ms, 1000);
  await h.tick();
  assert.deepEqual(u.bannerView(h.ctrl.get(), null), { kind: 'downloading', version: '0.1.3', progress: 0.5 });
  await h.tick();
  assert.equal(u.bannerView(h.ctrl.get(), null).kind, 'restarting');
  await h.tick();
  assert.equal(h.ctrl.get().phase, 'restarting');
  assert.equal(u.bannerView(h.ctrl.get(), null).kind, 'restarting');
  await h.tick();
  assert.deepEqual(u.bannerView(h.ctrl.get(), null), { kind: 'updated', version: '0.1.3' });
  assert.equal(store.m.has(u.UPDATE_PENDING_KEY), false);
  assert.equal(u.aboutLine(h.ctrl.get()), 'Updated to 0.1.3.');
  h.ctrl.clearUpdated();
  assert.equal(u.bannerView(h.ctrl.get(), null), null);
});

test('a fresh window after the relaunch says "Updated to"; a stale pending install is forgotten', async () => {
  const fresh = harness(() => json(status({ current: '0.1.3', state: 'idle', available: null })), memStore({ [u.UPDATE_PENDING_KEY]: '0.1.3' }));
  fresh.ctrl.start();
  await flush();
  assert.equal(u.bannerView(fresh.ctrl.get(), null).kind, 'updated');
  const stale = harness(() => json(status()), memStore({ [u.UPDATE_PENDING_KEY]: '0.1.3' }));
  stale.ctrl.start();
  await flush();
  assert.equal(stale.store.m.has(u.UPDATE_PENDING_KEY), false);
  assert.equal(u.bannerView(stale.ctrl.get(), null).kind, 'available');
});

test('a failed download shows the error with Try again and Download manually; the app keeps working', async () => {
  let n = 0;
  const h = harness((url, init) => {
    if (init.method === 'POST') return json(status({ state: 'downloading', progress: 0 }), 202);
    n += 1;
    return json(n === 1 ? status() : status({ state: 'error', error: 'The signature check failed.' }));
  });
  h.ctrl.start();
  await flush();
  await h.ctrl.install();
  await h.tick();
  const v = u.bannerView(h.ctrl.get(), null);
  assert.equal(v.kind, 'error');
  assert.equal(h.ctrl.get().phase, 'idle');
  assert.equal(h.store.m.has(u.UPDATE_PENDING_KEY), false);
  const html = ui.banner({ view: v, onUpdate() {}, onDismiss() {} });
  assert.match(html, /kwant bots 0\.1\.3 didn’t install\./);
  assert.match(html, /The signature check failed\./);
  assert.match(html, /This version keeps working/);
  assert.match(html, /Try again/);
  assert.match(html, /href="https:\/\/github\.com\/alexsergivan\/kwant\/releases\/tag\/mac-v0\.1\.3"[^>]*>.*Download manually/);
});

test('a refused install (409, 403) shows in the banner; a refused check stays in Settings', async () => {
  const h = harness((url, init) => {
    if (url.endsWith('/install')) return json({ error: 'An update is already downloading.' }, 409);
    if (url.endsWith('/check')) return json({ error: 'busy' }, 403);
    return json(status());
  });
  h.ctrl.start();
  await flush();
  await h.ctrl.check();
  assert.equal(h.ctrl.get().actionError, 'Only you can update the app, from the Mac it runs on.');
  assert.equal(u.bannerView(h.ctrl.get(), null).kind, 'available');
  await h.ctrl.install();
  const v = u.bannerView(h.ctrl.get(), null);
  assert.equal(v.kind, 'error');
  assert.equal(v.error, 'An update is already downloading.');
  assert.equal(h.store.m.has(u.UPDATE_PENDING_KEY), false);
});

test('Check for updates polls fast while checking, then shows the result', async () => {
  let checking = false;
  const h = harness((url, init) => {
    if (init.method === 'POST') {
      checking = true;
      return json(status({ state: 'checking', available: null }), 202);
    }
    if (checking) {
      checking = false;
      return json(status({ state: 'checking', available: null }));
    }
    return json(status({ state: 'idle', available: null }));
  });
  h.ctrl.start();
  await flush();
  assert.equal(u.aboutLine(h.ctrl.get()), 'You’re up to date.');
  await h.ctrl.check();
  assert.equal(u.aboutLine(h.ctrl.get()), 'Checking for updates…');
  assert.equal(h.live()[0].ms, 1000);
  await h.tick();
  await h.tick();
  assert.equal(u.aboutLine(h.ctrl.get()), 'You’re up to date.');
  assert.equal(h.live()[0].ms, 30 * 60 * 1000);
});

test('the relaunch gives up after 3 minutes and says so', async () => {
  let gone = false;
  const h = harness((url, init) => {
    if (init.method === 'POST') {
      gone = true;
      return json(status({ state: 'ready' }), 202);
    }
    return gone ? new TypeError('Failed to fetch') : json(status());
  });
  h.ctrl.start();
  await flush();
  await h.ctrl.install();
  await h.tick();
  assert.equal(h.ctrl.get().phase, 'restarting');
  await h.tick(4 * 60 * 1000);
  const v = u.bannerView(h.ctrl.get(), null);
  assert.equal(v.kind, 'error');
  assert.match(v.error, /didn’t come back/);
});

test('banner markup: buttons, notes, progress and dismiss', () => {
  const avail = ui.banner({ view: u.bannerView(model(status()), null), onUpdate() {}, onDismiss() {} });
  assert.match(avail, /kwant bots 0\.1\.3 is ready\./);
  assert.match(avail, /Update and restart/);
  assert.match(avail, /aria-expanded="false"[^>]*>What’s new/);
  assert.match(avail, /hidden=""[^>]*>.*In-app updates/s, 'notes are collapsed until What’s new');
  assert.match(avail, /aria-label="Hide until the next version"/);
  const noNotes = ui.banner({ view: { ...u.bannerView(model(status()), null), notes: '' }, onUpdate() {}, onDismiss() {} });
  assert.doesNotMatch(noNotes, /What’s new/);
  const dl = ui.banner({ view: { kind: 'downloading', version: '0.1.3', progress: 0.43 }, onUpdate() {}, onDismiss() {} });
  assert.match(dl, /Downloading kwant bots 0\.1\.3… 43%/);
  assert.match(dl, /role="progressbar"[^>]*aria-valuenow="43"/);
  assert.doesNotMatch(dl, /Update and restart|aria-label="Hide/);
  assert.match(ui.banner({ view: { kind: 'restarting', version: '0.1.3' }, onUpdate() {}, onDismiss() {} }), /Restarting the app to finish the update…/);
  assert.match(ui.banner({ view: { kind: 'updated', version: '0.1.3' }, onUpdate() {}, onDismiss() {} }), /Updated to kwant bots 0\.1\.3\./);
});

test('About row: version, Check for updates, and the offer', () => {
  const up = ui.about({ model: model(status({ state: 'idle', available: null })), onCheck() {}, onUpdate() {} });
  assert.match(up, /kwant bots for Mac <span[^>]*>0\.1\.2<\/span>/);
  assert.match(up, /Check for updates/);
  assert.match(up, /You’re up to date\./);
  assert.doesNotMatch(up, /disabled=""/);
  const offer = ui.about({ model: model(status()), onCheck() {}, onUpdate() {} });
  assert.match(offer, /kwant bots 0\.1\.3 is ready\./);
  assert.match(offer, /Update to 0\.1\.3 and restart/);
  const busy = ui.about({ model: model(status({ state: 'downloading', progress: 0.2 })), onCheck() {}, onUpdate() {} });
  assert.match(busy, /disabled=""/);
  assert.match(busy, /Downloading the update… 20%/);
  const failed = ui.about({ model: model(status({ state: 'error', available: null, error: 'GitHub didn’t answer.' })), onCheck() {}, onUpdate() {} });
  assert.match(failed, /text-amber-300[^>]*>GitHub didn’t answer\./);
  assert.equal(ui.about({ model: model(null), onCheck() {}, onUpdate() {} }), '');
});

test('wired once: the banner in the shell, the About row in App settings, hidden on phones', () => {
  assert.match(src('Shell.tsx'), /<AppUpdateBanner \/>/);
  assert.match(src('components/SettingsModal.tsx'), /<AppUpdateAbout \/>/);
  assert.match(src('components/AppUpdateBanner.tsx'), /if \(!view \|\| phone\) return null;/);
});
