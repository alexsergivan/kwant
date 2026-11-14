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
  import { AppUpdateBannerView } from './src/components/AppUpdateBanner';
  import { VersionCardView } from './src/components/VersionCard';
  export const banner = (props) => renderToStaticMarkup(React.createElement(AppUpdateBannerView, props));
  const noop = () => {};
  export const card = (props) =>
    renderToStaticMarkup(React.createElement(VersionCardView, { now: Date.parse('2026-11-12T22:02:00Z'), onCheck: noop, onInstall: noop, onWhatsNew: noop, onReleaseNotes: noop, ...props }));
`);
const src = (path) => readFileSync(new URL(`../src/${path}`, import.meta.url), 'utf8');

const NOTES = '### Added\n- **In-app updates.**';
const CHECKED = '2026-11-12T22:00:00Z';
const status = (over = {}) => ({
  supported: true,
  can_install: true,
  checked_at: CHECKED,
  how: null,
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
  current: null,
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
  // The Mac app installs; elsewhere the server says how
  assert.equal(s.canInstall, true);
  assert.equal(s.lastChecked, Date.parse(CHECKED));
  assert.equal(s.how, null);
  const off = u.parseUpdateStatus(status({ can_install: false, checked_at: null, how: { kind: 'docker', command: 'docker compose pull kwnt && docker compose up -d' } }));
  assert.equal(off.canInstall, false);
  assert.equal(off.lastChecked, null);
  assert.deepEqual(off.how, { kind: 'docker', command: 'docker compose pull kwnt && docker compose up -d' });
  assert.deepEqual(u.parseUpdateStatus(status({ how: { kind: 'windows', command: null } })).how, { kind: 'windows', command: null });
  assert.equal(u.parseUpdateStatus(status({ how: { kind: 'rm -rf', command: 'x' } })).how, null);
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
  // Off the Mac app the update can't run from the app: no bar, the kwant card says it
  assert.equal(u.bannerView(model(status({ can_install: false })), null), null);
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

test('off (404 or supported=false) shows the version, Check again and the latest release, and is never final', async () => {
  for (const answer of [json({ error: 'not found' }, 404), json({ ...status(), supported: false })]) {
    const h = harness(() => answer);
    h.ctrl.start();
    await flush();
    assert.equal(h.ctrl.get().unsupported, true);
    assert.deepEqual(h.calls, ['GET /api/app/update']);
    assert.equal(u.bannerView(h.ctrl.get(), null), null);
    assert.equal(u.versionLine(h.ctrl.get()).kind, 'off');
    const html = ui.card({ model: h.ctrl.get() });
    assert.match(html, /data-version-card="off"/);
    assert.match(html, /Update checks are turned off here\./);
    assert.match(html, /data-version-recheck[^>]*>Check again<\/button>/);
    assert.match(html, /href="https:\/\/github\.com\/alexsergivan\/kwant\/releases\/latest"[^>]*>Get the latest version/);
    assert.doesNotMatch(html, /latest version<\/span>|Update<\/button>/);
    assert.doesNotMatch(html, /All releases on GitHub/, 'off has one GitHub link, not two');
    if (answer.status === 200) assert.match(html, /kwant bots <span[^>]*>0\.1\.2<\/span>/, 'the version from the answer');
    // Asked again after 10 s, 60 s, then every 30 minutes
    assert.deepEqual(h.live().map((t) => t.ms), [10_000]);
    await h.tick(10_000);
    assert.deepEqual(h.live().map((t) => t.ms), [60_000]);
    await h.tick(60_000);
    assert.deepEqual(h.live().map((t) => t.ms), [30 * 60 * 1000]);
    await h.tick();
    assert.deepEqual(h.live().map((t) => t.ms), [30 * 60 * 1000]);
    assert.equal(h.calls.length, 4);
    await h.ctrl.install();
    assert.ok(!h.calls.some((c) => c.startsWith('POST')), 'no install or check POST while off');
  }
});

test('off, then supported: the card and the banner come back (the Mac updater registers late)', async () => {
  for (const off of [json({ error: 'not found' }, 404), json({ supported: false, current: '0.1.2' })]) {
    let n = 0;
    const h = harness(() => (++n === 1 ? off : json(status())));
    h.ctrl.start();
    await flush();
    assert.equal(u.versionLine(h.ctrl.get()).kind, 'off');
    await h.tick(10_000);
    const m = h.ctrl.get();
    assert.equal(m.unsupported, false);
    assert.equal(u.versionLine(m).kind, 'available');
    assert.equal(u.bannerView(m, null)?.kind, 'available');
    assert.match(ui.card({ model: m }), /kwant bots 0\.1\.3 is available/);
    assert.deepEqual(h.live().map((t) => t.ms), [30 * 60 * 1000], 'back on the normal poll');
    // Off again later starts the short retries over
    n = 0;
    await h.tick();
    assert.equal(h.ctrl.get().unsupported, true);
    assert.deepEqual(h.live().map((t) => t.ms), [10_000]);
  }
});

test('Check again while off asks now (a GET, no POST) and says Checking… meanwhile', async () => {
  let answer = json({ supported: false });
  let release;
  const h = harness(() => answer);
  h.ctrl.start();
  await flush();
  const gate = new Promise((r) => (release = r));
  answer = gate.then(() => json(status({ state: 'idle', available: null })));
  const pressed = h.ctrl.check();
  assert.equal(h.ctrl.get().rechecking, true);
  assert.deepEqual(u.versionLine(h.ctrl.get()), { kind: 'off', rechecking: true });
  const busy = ui.card({ model: h.ctrl.get() });
  assert.match(busy, /aria-disabled="true"[^>]*data-version-recheck[^>]*>.*Checking…<\/button>/s);
  release();
  await pressed;
  assert.equal(h.ctrl.get().rechecking, false);
  assert.deepEqual(h.calls, ['GET /api/app/update', 'GET /api/app/update']);
  assert.equal(u.versionLine(h.ctrl.get()).kind, 'latest');
  assert.equal(h.live().length, 1, 'one poll scheduled, not two');
});

test('opening Settings asks again when the last answer was off', async () => {
  let n = 0;
  const h = harness(() => (++n === 1 ? json({ supported: false }) : json(status())));
  h.ctrl.start();
  await flush();
  await h.ctrl.checkIfStale();
  assert.deepEqual(h.calls, ['GET /api/app/update', 'GET /api/app/update']);
  assert.equal(u.versionLine(h.ctrl.get()).kind, 'available');
  assert.match(src('components/VersionCard.tsx'), /if \(known \|\| ctrl\.get\(\)\.unsupported\) void ctrl\.checkIfStale\(\);/);
});

test('opening Settings 3 s after an off answer keeps the 10 s retry (N1)', async () => {
  let answer = json({ supported: false, current: '0.1.2' });
  const h = harness(() => answer);
  h.ctrl.start();
  await flush();
  const [retry] = h.live();
  assert.equal(retry.ms, 10_000);
  h.advance(3_000);
  await h.ctrl.checkIfStale(); // Settings opened: asks now, still off
  assert.equal(h.calls.length, 2);
  assert.equal(h.ctrl.get().unsupported, true);
  assert.deepEqual(h.live(), [retry], 'the pending 10 s retry is kept, not replaced by a later one');
  // The Mac updater is up by now: the retry at 10 s brings the card back
  answer = json(status());
  await h.tick(7_000);
  assert.equal(h.ctrl.get().unsupported, false);
  assert.equal(u.versionLine(h.ctrl.get()).kind, 'available');
});

test('Check again three times while off leaves the next automatic retry where it was (N1)', async () => {
  const h = harness(() => json({ supported: false }));
  h.ctrl.start();
  await flush();
  await h.tick(10_000); // the scheduled retry, still off: next one in 60 s
  const [retry] = h.live();
  assert.equal(retry.ms, 60_000);
  for (let i = 0; i < 3; i++) {
    h.advance(1_000);
    await h.ctrl.check();
  }
  assert.equal(h.calls.length, 5);
  assert.ok(!h.calls.some((c) => c.startsWith('POST')));
  assert.deepEqual(h.live(), [retry], 'still the 60 s retry, not pushed to 30 min');
  // Only scheduled polls advance the waits: the 60 s retry, still off, goes to 30 min
  await h.tick(57_000);
  assert.deepEqual(h.live().map((t) => t.ms), [30 * 60 * 1000]);
});

test('an off answer while the app relaunches after an install keeps asking every second', () => {
  assert.equal(u.offRetryMs(1, 'restarting'), 1000);
  assert.equal(u.offRetryMs(5, 'installing'), 1000);
  assert.equal(u.offRetryMs(1), 10_000);
  assert.equal(u.offRetryMs(2), 60_000);
  assert.equal(u.offRetryMs(3), 30 * 60 * 1000);
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
  assert.deepEqual(u.versionLine(h.ctrl.get()), { kind: 'updated', version: '0.1.3' });
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
  assert.equal(u.versionLine(h.ctrl.get()).kind, 'latest');
  await h.ctrl.check();
  assert.equal(u.versionLine(h.ctrl.get()).kind, 'checking');
  assert.equal(h.live()[0].ms, 1000);
  await h.tick();
  await h.tick();
  assert.deepEqual(u.versionLine(h.ctrl.get()), { kind: 'latest', checkedAt: Date.parse(CHECKED) });
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

test('version line: one of latest, available, checking, couldn’t check — never a wrong latest', () => {
  const line = (raw, over) => u.versionLine(model(raw, over));
  assert.deepEqual(line(status({ state: 'idle', available: null })), { kind: 'latest', checkedAt: Date.parse(CHECKED) });
  assert.equal(line(status()).kind, 'available');
  assert.equal(line(status()).canInstall, true);
  assert.equal(line(status({ can_install: false })).canInstall, false);
  assert.equal(line(status({ state: 'checking', available: null })).kind, 'checking');
  // Never checked yet: checking, not "latest"
  assert.equal(line(status({ state: 'idle', available: null, checked_at: null })).kind, 'checking');
  assert.equal(line(null).kind, 'loading');
  const cant = line(status({ state: 'error', available: null, error: 'Couldn’t check for updates: GitHub couldn’t be reached.' }));
  assert.equal(cant.kind, 'check-error');
  // A refused Check again with nothing known
  assert.equal(line(status({ state: 'idle', available: null }), { actionError: 'busy' }).kind, 'check-error');
  // A version found earlier stays on offer after a failed re-check
  assert.equal(line(status({ state: 'error', error: 'Couldn’t check for updates: offline' })).kind, 'available');
  // A failed install is its own line, with Try again and Download manually
  const inst = line(status({ state: 'error', error: 'The signature check failed.' }));
  assert.equal(inst.kind, 'install-error');
  assert.equal(inst.error, 'The signature check failed.');
  assert.equal(line(status({ state: 'downloading', progress: 0.2 })).kind, 'downloading');
  assert.equal(line(status({ state: 'ready' })).kind, 'restarting');
});

test('checked ago, in plain words', () => {
  const t = Date.parse(CHECKED);
  assert.equal(u.checkedAgo(null, t), '');
  assert.equal(u.checkedAgo(t, t + 20_000), 'Checked just now');
  assert.equal(u.checkedAgo(t, t + 2 * 60_000), 'Checked 2 min ago');
  assert.equal(u.checkedAgo(t, t + 3 * 3600_000), 'Checked 3 h ago');
  assert.equal(u.checkedAgo(t, t + 26 * 3600_000), 'Checked 1 day ago');
  assert.equal(u.checkedAgo(t, t + 50 * 3600_000), 'Checked 2 days ago');
});

test('kwant card: the version, one status line, What’s new, All releases — no earlier versions', () => {
  const latest = ui.card({ model: model(status({ state: 'idle', available: null })) });
  assert.match(latest, /data-version-card="latest"/);
  assert.match(latest, /kwant bots <span[^>]*>0\.1\.2<\/span>/);
  assert.match(latest, /You’re on the latest version/);
  assert.match(latest, /Check again/);
  assert.match(latest, /Checked 2 min ago/);
  assert.match(latest, /All releases on GitHub/);
  assert.doesNotMatch(latest, /Earlier versions|Update<|is available/);

  const mac = ui.card({ model: model(status()) });
  assert.match(mac, /kwant bots 0\.1\.3 is available/);
  assert.match(mac, /data-version-update[^>]*>.*Update<\/button>/s);
  assert.doesNotMatch(mac, /aria-expanded/, 'on the Mac, Update installs');
  assert.match(mac, /What’s new in 0\.1\.3/, 'the newer version’s notes');
  assert.doesNotMatch(mac, /latest version/);

  const how = { kind: 'docker', command: 'docker compose pull kwnt && docker compose up -d' };
  const docker = ui.card({ model: model(status({ can_install: false, how })), howOpen: true });
  assert.match(docker, /aria-expanded="true"/);
  assert.match(docker, /How to update/);
  assert.match(docker, /docker compose pull kwnt &amp;&amp; docker compose up -d/);
  assert.match(docker, /Copy/);
  const closed = ui.card({ model: model(status({ can_install: false, how })) });
  assert.match(closed, /aria-expanded="false"/);
  assert.doesNotMatch(closed, /How to update/);
  const win = ui.card({ model: model(status({ can_install: false, how: { kind: 'windows', command: null } })), howOpen: true });
  assert.match(win, /Download kwant bots 0\.1\.3/);
  assert.match(win, /href="https:\/\/github\.com\/alexsergivan\/kwant\/releases\/tag\/mac-v0\.1\.3"/);

  const cant = ui.card({ model: model(status({ state: 'error', available: null, error: 'Couldn’t check for updates: GitHub couldn’t be reached.' })) });
  assert.match(cant, /data-version-card="check-error"/);
  assert.match(cant, /Couldn’t check for updates/);
  assert.match(cant, /Try again/);
  assert.match(cant, />GitHub couldn’t be reached\.</);
  assert.doesNotMatch(cant, /latest version/);

  const failed = ui.card({ model: model(status({ state: 'error', error: 'The signature check failed.' })) });
  assert.match(failed, /kwant bots 0\.1\.3 didn’t install\./);
  assert.match(failed, /Download manually/);

  const dl = ui.card({ model: model(status({ state: 'downloading', progress: 0.43 })) });
  assert.match(dl, /Downloading kwant bots 0\.1\.3… 43%/);
  assert.match(dl, /role="progressbar"[^>]*aria-valuenow="43"/);

  // The current version’s sheet, in the What’s new card’s look, when this app bundles one
  const entry = { version: '0.1.2', date: '2026-11-10', highlights: [{ id: 'a', title: 'A', text: 'a' }, { id: 'b', title: 'B', text: 'b' }], fixes: ['x'] };
  const withSheet = ui.card({ model: model(status({ state: 'idle', available: null })), entryFor: (v) => (v === '0.1.2' ? entry : null) });
  assert.match(withSheet, /data-whats-new-current="0\.1\.2"/);
  assert.match(withSheet, /What’s new in 0\.1\.2/);
  assert.match(withSheet, /Nov 10 · 2 new things, 1 fix/);
});

test('opening Settings checks again when the last answer is over an hour old, quietly', async () => {
  let checks = 0;
  const h = harness((url, init) => {
    if (init.method === 'POST') {
      checks++;
      return json({ error: 'nope' }, 409);
    }
    return json(status({ state: 'idle', available: null }));
  });
  h.ctrl.start();
  await flush();
  h.advance(Date.parse(CHECKED) + 10 * 60_000);
  await h.ctrl.checkIfStale();
  assert.equal(checks, 0, 'fresh: no check');
  h.advance(2 * 3600_000);
  await h.ctrl.checkIfStale();
  assert.equal(checks, 1);
  assert.equal(h.ctrl.get().actionError, null, 'a refused quiet check says nothing');
});

test('wired once: the banner in the shell, the kwant card in App settings, hidden on phones', () => {
  assert.match(src('Shell.tsx'), /<AppUpdateBanner \/>/);
  const settings = src('components/SettingsModal.tsx');
  assert.equal(settings.match(/<VersionCard \/>/g)?.length, 1);
  assert.doesNotMatch(settings, /AppUpdateAbout|WhatsNewAbout/);
  assert.match(src('components/AppUpdateBanner.tsx'), /if \(!view \|\| phone\) return null;/);
});

const notify = await loadBundled(`export * from './src/continuity';`);

test('App updates switch is listed only in the Mac app, after the other kinds', () => {
  const ids = (on) => notify.notifyKindsFor(on).map((k) => k.id);
  assert.ok(!ids(false).includes('app_update'));
  assert.deepEqual(ids(false), notify.NOTIFY_KINDS.map((k) => k.id));
  assert.equal(ids(true).at(-1), 'app_update');
  assert.equal(notify.notifyKindsFor(true).at(-1).label, 'App updates');
});

test('app_update is saved in the Mac app and never sent to a server without it', () => {
  const on = ['turn_finished', 'app_update'];
  assert.deepEqual(notify.notifyOnToSave(on, true), on);
  assert.deepEqual(notify.notifyOnToSave(on, false), ['turn_finished']);
  // Switched off in the Mac app: stays off
  assert.deepEqual(notify.notifyOnToSave(['turn_finished'], true), ['turn_finished']);
});

test('App settings shows the switch only where the app installs updates (the Mac app)', () => {
  const s = src('components/SettingsModal.tsx');
  assert.match(s, /const appUpdates = useAppUpdate\(\)\.model\.status\?\.canInstall === true;/);
  assert.match(s, /notifyKindsFor\(appUpdates, !!proactive, !!discovery(, [^)]+)?\)\.map/);
  assert.match(s, /notify_on: notifyOnToSave\(notifyOn, appUpdates, !!proactive, !!discovery(, [^)]+)?\)/);
  assert.doesNotMatch(s, /\{NOTIFY_KINDS\.map/);
});
