// 0.7 Dev mode, the frontend slice (Pixel): the switch's state machine (two presses, the 5 s re-cover,
// Esc/blur, the settle that eats a double-click, reduced motion), the setup sheet's Repository field
// (400 ms debounce, stale replies, every chip), the switch and the sheet in a DOM against the real
// /api/spaces/:id/dev_mode client, Settings' Dev mode card, the Production mark and the Dev desk shell.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;

// The server, as Lin's lin-dev-core routes answer (demo data only).
const off = { on: false, repo: null, branch: null, deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null };
let server = { state: { ...off }, check: { state: 'found', repo: 'demo-org/demo-app', branch: 'main', deploy_hint: null }, fail: null, calls: [] };
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  const body = init.body ? JSON.parse(init.body) : undefined;
  server.calls.push([method, u.split('?')[0], body]);
  const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
  if (server.fail && server.fail.method === method && u.includes(server.fail.path)) return json({ error: server.fail.error }, server.fail.status);
  if (u.endsWith('/dev_mode/check')) return json(server.check);
  if (u.includes('/dev_mode')) {
    if (method === 'PUT') server.state = { ...server.state, ...body, polled_at: body.on ? '2026-11-05T09:00:00Z' : server.state.polled_at };
    return json(server.state);
  }
  if (u.includes('/production')) return json({ id: decodeURIComponent(u.split('/')[3]), production: body.production });
  if (u.includes('/dev/landed')) return json({ cards: [], rollup: { count: 0, areas: [] }, last_tag: null, last_merge: null });
  if (u.includes('/dev/reviews')) return json({ cards: [] });
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * from './src/devMode';
  export { DevModeSwitch, DevOffNote, DevBadge, HardHat } from './src/components/DevModeSwitch';
  export { DevModeSetup } from './src/components/DevModeSetup';
  export { DevModeSettings } from './src/components/DevModeSettings';
  export { ConnectionRow } from './src/components/ConnectionsView';
  export { canMarkProduction } from './src/connections';
  export { DevDeskRow, DevDeskView, deskStore } from './src/components/DevDesk';
  export { parseView, viewSegment } from './src/nav';
`);
const { React, createRoot } = m;
const h = React.createElement;

const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

const reset = (state = off) => {
  server = { state: { ...state }, check: { state: 'found', repo: 'demo-org/demo-app', branch: 'main', deploy_hint: null }, fail: null, calls: [] };
  m.devStore._reset();
  document.body.innerHTML = '';
};

const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, root };
};

// ---------------------------------------------------------------------------
// The state machine

const R = (s, a) => m.switchReducer(s, a, 'Demo');

test('two presses: the first lifts the cover, the second flips (set up already)', () => {
  let s = m.initialSwitch({ setup: false });
  s = R(s, { type: 'press', now: 1000 });
  assert.equal(s.phase, 'lifted');
  assert.equal(s.on, false);
  assert.equal(s.announce.text, 'Unlocked. Press again to turn Dev mode on.');
  assert.equal(m.switchLook(s).hint, 'Press again to turn Dev mode on');
  s = R(s, { type: 'press', now: 1200 });
  assert.equal(s.on, true);
  assert.equal(s.saving, true);
  assert.equal(s.phase, 'settling');
  assert.equal(s.settleUntil, 1200 + m.SETTLE_MS);
  assert.equal(s.announce.text, 'Dev mode on for Demo.');
  assert.equal(m.switchLook(s).label, 'Dev mode, on. Press to unlock');
});

test('a double-click while it settles is ignored, and so is a press while saving', () => {
  let s = m.initialSwitch({ setup: false });
  s = R(R(s, { type: 'press', now: 0 }), { type: 'press', now: 100 });
  const settling = s;
  assert.equal(R(settling, { type: 'press', now: 150 }), settling, 'third press inside the settle');
  s = R({ ...settling, saving: false }, { type: 'press', now: 300 });
  assert.equal(s.phase, 'settling', 'still settling at 300 ms');
  s = R({ ...settling, saving: false }, { type: 'settled', now: 100 + m.SETTLE_MS });
  assert.equal(s.phase, 'covered');
  assert.equal(R({ ...s, saving: true }, { type: 'press', now: 9999 }).phase, 'covered', 'a save in flight eats the press');
  assert.equal(R({ ...s, saving: false }, { type: 'press', now: 9999 }).phase, 'lifted');
});

test('a lifted cover drops back after 5 s with nothing changed, not before', () => {
  let s = R(m.initialSwitch({ on: true, setup: false }), { type: 'press', now: 1000 });
  assert.equal(R(s, { type: 'recover', now: 1000 + m.RECOVER_MS - 1 }).phase, 'lifted');
  s = R(s, { type: 'recover', now: 1000 + m.RECOVER_MS });
  assert.equal(s.phase, 'covered');
  assert.equal(s.on, true);
  assert.equal(s.announce.text, 'Locked.');
});

test('Esc and blur drop a lifted cover; they do nothing when covered', () => {
  const lifted = R(m.initialSwitch({ setup: false }), { type: 'press', now: 0 });
  for (const type of ['escape', 'blur']) {
    const s = R(lifted, { type });
    assert.equal(s.phase, 'covered', type);
    assert.equal(s.on, false, type);
  }
  const covered = m.initialSwitch();
  assert.equal(R(covered, { type: 'escape' }), covered);
  assert.equal(R(covered, { type: 'blur' }), covered);
});

test('reduced motion: still two presses, but nothing to settle, so the next press lifts at once', () => {
  let s = m.initialSwitch({ setup: false, reduced: true });
  s = R(s, { type: 'press', now: 0 });
  assert.equal(s.phase, 'lifted');
  s = R(s, { type: 'press', now: 10 });
  assert.equal(s.on, true);
  assert.equal(s.phase, 'covered');
  assert.equal(s.settleUntil, 0);
  s = R({ ...s, saving: false }, { type: 'press', now: 20 });
  assert.equal(s.phase, 'lifted', 'one press lifts, never flips');
  assert.equal(s.on, true);
});

test('first time: the second press opens the setup (pending, half-flipped); cancel covers, done flips on', () => {
  let s = R(R(m.initialSwitch(), { type: 'press', now: 0 }), { type: 'press', now: 100 });
  assert.equal(s.phase, 'pending');
  assert.equal(s.on, false);
  const look = m.switchLook(s);
  assert.equal(look.fill, 0.45);
  assert.equal(look.knobRight, true);
  assert.equal(look.coverOpen, true);
  assert.equal(R(s, { type: 'press', now: 200 }), s, 'presses wait for the sheet');
  assert.equal(R(s, { type: 'setup_cancel' }).phase, 'covered');
  assert.equal(R(s, { type: 'setup_cancel' }).on, false);
  const done = R(s, { type: 'setup_done', now: 300 });
  assert.equal(done.on, true);
  assert.equal(done.setup, false);
  assert.equal(done.phase, 'settling');
});

test('a failed save puts the switch back, covered, with the reason announced', () => {
  let s = R(R(m.initialSwitch({ on: true, setup: false }), { type: 'press', now: 0 }), { type: 'press', now: 100 });
  assert.equal(s.on, false);
  s = R(s, { type: 'save_failed', error: 'Only the space owner can change Dev mode.' });
  assert.equal(s.on, true);
  assert.equal(s.phase, 'covered');
  assert.equal(s.saving, false);
  assert.equal(s.error, 'Only the space owner can change Dev mode.');
  assert.equal(s.announce.text, 'Only the space owner can change Dev mode.');
});

test('the server changing Dev mode closes a held-open cover; a disabled switch says so', () => {
  const lifted = R(m.initialSwitch({ setup: false }), { type: 'press', now: 0 });
  const s = R(lifted, { type: 'server', state: { ...off, on: true, repo: 'demo-org/demo-app', polled_at: 'x' } });
  assert.equal(s.on, true);
  assert.equal(s.phase, 'covered');
  const d = R(m.initialSwitch(), { type: 'server', state: null, disabled: true });
  assert.equal(R(d, { type: 'press', now: 0 }), d);
  assert.match(m.switchLook(d).label, /unavailable$/);
  assert.equal(m.switchLook(d).tooltip, 'Only the space owner can change Dev mode');
});

// ---------------------------------------------------------------------------
// Repository field

test('normaliseRepo takes owner/name, URLs, SSH and repo sub-pages; refuses the rest', () => {
  for (const [input, want] of [
    ['demo-org/demo-app', 'demo-org/demo-app'],
    ['  https://github.com/demo-org/demo-app.git  ', 'demo-org/demo-app'],
    ['github.com/demo-org/demo-app/', 'demo-org/demo-app'],
    ['git@github.com:demo-org/demo-app.git', 'demo-org/demo-app'],
    ['https://www.github.com/demo-org/demo-app/pull/12', 'demo-org/demo-app'],
    ['https://github.com/demo-org/demo-app/tree/main?x=1', 'demo-org/demo-app'],
    ['demo-org', null],
    ['', null],
    ['demo org/app', null],
    ['https://github.com/demo-org/demo-app/settings', null],
    ['demo-org/..', null],
  ])
    assert.equal(m.normaliseRepo(input), want, input);
});

test('fieldView: each state has its chip, help and whether Turn on is allowed', () => {
  const v = (s) => m.fieldView(s, 'demo-org/demo-app', 'GitHub is slow');
  assert.equal(v('empty').chip, null);
  assert.equal(v('empty').help, m.EMPTY_HELP);
  assert.deepEqual(v('checking').chip, { tone: 'info', text: 'Checking', spinner: true });
  assert.deepEqual(v('found').chip, { tone: 'ok', text: 'Found' });
  assert.equal(v('found').canTurnOn, true);
  assert.equal(v('not_found').chip.text, 'Can’t see it');
  assert.equal(v('not_found').helpRepo, 'demo-org/demo-app');
  assert.match(v('not_found').help, /\{repo\}/);
  assert.equal(v('not_connected').connect, true);
  assert.equal(v('invalid').chip.text, 'Not a repo');
  assert.equal(v('error').help, 'GitHub is slow');
  for (const s of ['empty', 'checking', 'not_found', 'not_connected', 'invalid', 'error']) assert.equal(v(s).canTurnOn, false, s);
});

const fakeTimers = () => {
  let t = 0;
  let id = 0;
  const q = new Map();
  return {
    timers: { setTimeout: (f, ms) => (q.set(++id, { f, at: t + ms }), id), clearTimeout: (h) => q.delete(h) },
    advance(ms) {
      t += ms;
      for (const [k, v] of [...q]) if (v.at <= t) (q.delete(k), v.f());
    },
    pending: () => q.size,
  };
};

test('the checker waits 400 ms after the last keystroke, then asks once', async () => {
  const asked = [];
  const updates = [];
  const ft = fakeTimers();
  const c = m.createRepoChecker(async (r) => (asked.push(r), { state: 'found', repo: r, branch: 'main', deploy_hint: null }), (u) => updates.push(u.state), ft.timers);
  c.input('demo-org/d');
  ft.advance(200);
  c.input('demo-org/de');
  ft.advance(399);
  c.input('demo-org/demo-app');
  assert.deepEqual(asked, [], 'nothing asked while typing');
  assert.equal(ft.pending(), 1);
  ft.advance(m.CHECK_DEBOUNCE_MS - 1);
  assert.deepEqual(asked, []);
  ft.advance(1);
  assert.deepEqual(asked, ['demo-org/demo-app']);
  await settle();
  assert.deepEqual(updates, ['checking', 'checking', 'checking', 'found']);
});

test('the checker: empty and invalid answer at once without asking; a stale reply is dropped', async () => {
  const replies = [];
  const updates = [];
  const ft = fakeTimers();
  const c = m.createRepoChecker((r) => new Promise((res) => replies.push(() => res({ state: r.endsWith('old') ? 'not_found' : 'found', repo: r, branch: null, deploy_hint: null }))), (u) => updates.push(u.state), ft.timers);
  c.input('nope');
  c.input('');
  assert.deepEqual(updates, ['invalid', 'empty']);
  assert.equal(ft.pending(), 0);
  c.input('demo-org/old');
  ft.advance(400);
  c.input('demo-org/new');
  ft.advance(400);
  replies[1]();
  await settle();
  replies[0]();
  await settle();
  assert.equal(updates.at(-1), 'found', 'the old name’s late answer is ignored');
  assert.equal(updates.filter((u) => u === 'not_found').length, 0);
});

// ---------------------------------------------------------------------------
// In a DOM

const switchEl = (host) => host.querySelector('[data-dev-switch]');
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('DOM: on → off with two presses saves PUT {on:false}, announces, and calls onTurnedOff', async () => {
  reset({ ...off, on: true, repo: 'demo-org/demo-app', branch: 'main', polled_at: '2026-11-05T08:00:00Z' });
  let turnedOff = 0;
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' }, onTurnedOff: () => turnedOff++ }));
  const b = switchEl(host);
  assert.equal(b.tagName, 'BUTTON');
  assert.equal(b.getAttribute('role'), null, 'a button, not role=switch');
  assert.equal(b.dataset.on, 'true');
  assert.equal(b.getAttribute('aria-label'), 'Dev mode, on. Press to unlock');
  await click(b);
  assert.equal(b.dataset.devSwitch, 'lifted');
  assert.equal(b.dataset.cover, 'open');
  assert.equal(b.dataset.ring, 'true');
  assert.match(host.querySelector('[data-dv-live]').textContent, /Unlocked\. Press again to turn Dev mode off\./);
  assert.ok(host.querySelector('[data-dv-hint]'));
  await click(b);
  await click(b); // the double-click's third press: ignored while settling
  assert.deepEqual(
    server.calls.filter(([meth]) => meth === 'PUT').map(([, , body]) => body),
    [{ on: false }],
  );
  assert.match(host.querySelector('[data-dv-live]').textContent, /Dev mode off for Demo/);
  assert.equal(turnedOff, 1);
  await wait(m.SETTLE_MS + 30);
  assert.equal(b.dataset.devSwitch, 'covered');
  assert.equal(b.dataset.on, undefined);
  await click(b); // a render mid-save once left the switch stuck; the next press still lifts
  assert.equal(b.dataset.devSwitch, 'lifted');
  root.unmount();
});

test('DOM: Escape and blur drop a lifted cover without saving', async () => {
  reset({ ...off, on: true, repo: 'demo-org/demo-app', polled_at: 'x' });
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b = switchEl(host);
  await click(b);
  b.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  await settle();
  assert.equal(b.dataset.devSwitch, 'covered');
  await click(b);
  b.focus();
  b.blur();
  await settle();
  assert.equal(b.dataset.devSwitch, 'covered');
  assert.equal(server.calls.filter(([meth]) => meth === 'PUT').length, 0);
  root.unmount();
});

test('DOM: a 403 on load shows the switch disabled', async () => {
  reset();
  server.fail = { method: 'GET', path: '/dev_mode', status: 403, error: 'only the person' };
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  assert.equal(switchEl(host).disabled, true);
  root.unmount();
});

test('DOM: reduced motion shows the Dev label and the lock glyph, and opens with no settle', async () => {
  reset({ ...off, on: false, repo: 'demo-org/demo-app', polled_at: 'x' });
  window.matchMedia = (q) => ({ matches: q.includes('reduce'), media: q, addEventListener() {}, removeEventListener() {} });
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b = switchEl(host);
  assert.ok(b.classList.contains('kw-dv-rm'));
  assert.ok(b.querySelector('[data-dv-glyph="locked"]'));
  await click(b);
  assert.ok(b.querySelector('[data-dv-glyph="unlocked"]'));
  await click(b);
  assert.equal(b.dataset.devSwitch, 'covered');
  assert.equal(b.dataset.on, 'true');
  await wait(30); // the save comes back; presses wait for it, motion or not
  await settle();
  await click(b);
  assert.equal(b.dataset.devSwitch, 'lifted', 'the next press lifts at once: nothing to settle');
  root.unmount();
  delete window.matchMedia;
});

test('DOM: first switch-on opens the setup sheet; typing checks after 400 ms; Turn on sends repo and branch', async () => {
  reset();
  server.check = { state: 'found', repo: 'demo-org/demo-app', branch: 'trunk', deploy_hint: null };
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b = switchEl(host);
  await click(b);
  await click(b);
  assert.equal(b.dataset.devSwitch, 'pending');
  const dialog = document.querySelector('[role="dialog"]');
  assert.equal(dialog.getAttribute('aria-modal'), 'true');
  assert.match(dialog.textContent, /Which repo is this space about\?/);
  assert.doesNotMatch(dialog.textContent, /Ready to ship/);
  const turnOn = dialog.querySelector('[data-dv-turn-on]');
  assert.equal(turnOn.disabled, true);
  const input = dialog.querySelector('#dv-repo');
  const type = async (v) => {
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    set.call(input, v);
    input.dispatchEvent(new window.Event('input', { bubbles: true }));
    await settle();
  };
  await type('demo-org');
  assert.equal(dialog.querySelector('[data-dv-chip]').dataset.dvChip, 'invalid');
  await type('https://github.com/demo-org/demo-app');
  assert.equal(dialog.querySelector('[data-dv-chip]').dataset.dvChip, 'checking');
  await wait(200);
  assert.equal(server.calls.filter(([, u]) => u.endsWith('/check')).length, 0, 'not before 400 ms');
  await wait(260);
  await settle();
  assert.deepEqual(server.calls.filter(([, u]) => u.endsWith('/check')).map(([, , body]) => body), [{ repo: 'demo-org/demo-app' }]);
  assert.equal(dialog.querySelector('[data-dv-chip]').dataset.dvChip, 'found');
  assert.match(dialog.querySelector('[data-dv-tiles]').textContent, /trunk/);
  assert.equal(turnOn.disabled, false);
  await click(turnOn);
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , body]) => body), [{ on: true, repo: 'demo-org/demo-app', branch: 'trunk' }]);
  assert.equal(document.querySelector('[role="dialog"]'), null);
  assert.equal(b.dataset.on, 'true');
  root.unmount();
});

test('DOM: the setup sheet’s chips for not found and not connected, and Esc cancels', async () => {
  reset();
  let cancelled = 0;
  for (const [state, chip, text] of [
    ['not_found', 'not_found', /Bots can’t see demo-org\/demo-app with your GitHub connection/],
    ['not_connected', 'not_connected', /Connect GitHub to read this repo/],
  ]) {
    const { root } = await mount(
      h(m.DevModeSetup, {
        space: { id: 'spc_demo', name: 'Demo' },
        initial: { ...off, repo: 'demo-org/demo-app' },
        onCancel: () => cancelled++,
        onDone: () => {},
        onOpenConnections: () => {},
        check: async (r) => ({ state, repo: r, branch: null, deploy_hint: null }),
      }),
    );
    await wait(450);
    await settle();
    const d = document.querySelector('[role="dialog"]');
    assert.equal(d.querySelector('[data-dv-chip]').dataset.dvChip, chip);
    assert.match(d.querySelector('#dv-repo-help').textContent, text);
    assert.equal(d.querySelector('[data-dv-turn-on]').disabled, true);
    if (state === 'not_connected') assert.match(d.textContent, /Connect GitHub/);
    d.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    root.unmount();
    document.body.innerHTML = '';
  }
  assert.equal(cancelled, 2);
});

test('DOM: a failed Turn on keeps the sheet open with the server’s reason', async () => {
  reset();
  const { root } = await mount(
    h(m.DevModeSetup, {
      space: { id: 'spc_demo', name: 'Demo' },
      initial: { ...off, repo: 'demo-org/demo-app' },
      onCancel: () => {},
      onDone: () => assert.fail('not done'),
      check: async (r) => ({ state: 'found', repo: r, branch: 'main', deploy_hint: null }),
      save: async () => {
        throw new Error('GitHub didn’t answer. Try again in a minute.');
      },
    }),
  );
  await wait(450);
  await settle();
  await click(document.querySelector('[data-dv-turn-on]'));
  assert.match(document.querySelector('[role="alert"]').textContent, /GitHub didn’t answer/);
  assert.ok(document.querySelector('[role="dialog"]'));
  root.unmount();
});

// ---------------------------------------------------------------------------
// Settings, Connections, the desk, the looks

test('Settings › Dev mode: hidden before the first switch-on; saves repo, branch, backup command; Turn off', async () => {
  reset();
  let v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  assert.equal(v.host.querySelector('[data-dev-settings]'), null);
  v.root.unmount();

  reset({ ...off, on: true, repo: 'demo-org/demo-app', branch: 'main', polled_at: 'x' });
  v = await mount(h(m.DevModeSettings, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  const form = v.host.querySelector('[data-dev-settings]');
  assert.ok(form);
  assert.equal(form.querySelector('#dv-set-repo').value, 'demo-org/demo-app');
  const save = form.querySelector('button[type="submit"]');
  assert.equal(save.disabled, true, 'nothing changed yet');
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  const input = form.querySelector('#dv-set-backup');
  set.call(input, './scripts/backup.sh');
  input.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  assert.equal(save.disabled, false);
  form.dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , b]) => b), [{ on: true, repo: 'demo-org/demo-app', branch: 'main', backup_command: './scripts/backup.sh' }]);
  assert.match(form.textContent, /Saved/);
  await click(form.querySelector('[data-dev-turn-off]'));
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').at(-1)[2], { on: false });
  assert.equal(m.devStore.isOn('spc_demo'), false);
  assert.match(form.textContent, /Off\. Reviews, the ship gate and the production floor are stopped/);
  v.root.unmount();
});

test('Production mark: only MCP servers and Composio apps, and only when asked for', () => {
  assert.equal(m.canMarkProduction({ id: 'mcp:demo-db' }), true);
  assert.equal(m.canMarkProduction({ id: 'composio:github' }), true);
  assert.equal(m.canMarkProduction({ id: 'composio:Bad Slug' }), false);
  assert.equal(m.canMarkProduction({ id: 'engine:claude' }), false);
  const c = { id: 'mcp:demo-db', kind: 'mcp', name: 'Demo DB', detail: null, status: 'ok', status_detail: null, last_check: null, used_by: [], logins_stored: 'vault', manage: '', production: true };
  const without = m.renderToStaticMarkup(h(m.ConnectionRow, { c, now: 0 }));
  assert.doesNotMatch(without, /data-production-toggle/);
  assert.match(without, /data-production-badge/);
  const withMark = m.renderToStaticMarkup(h(m.ConnectionRow, { c, now: 0, onProduction: () => {} }));
  assert.match(withMark, /data-production-toggle="mcp:demo-db"/);
  assert.match(withMark, /aria-label="Demo DB is production"/);
  assert.match(withMark, /checked=""/);
});

test('looks: the hard hat, the </> badge, and the turned-off note', () => {
  const hat = m.renderToStaticMarkup(h(m.HardHat));
  assert.match(hat, /title="Working under Dev mode rules"/);
  assert.match(hat, /<svg width="10" height="8"/);
  assert.match(m.renderToStaticMarkup(h(m.DevBadge, { behind: 'var(--kw-surface-side)' })), /&lt;\/&gt;.*kw-dev-caret/);
  const note = m.renderToStaticMarkup(h(m.DevOffNote, { approvals: 'smart', onDismiss: () => {} }));
  assert.match(note, /role="status"/);
  assert.match(note, /Dev mode is off\./);
});

test('nav: /dev is the Dev desk', () => {
  assert.deepEqual(m.parseView('dev')?.view, { kind: 'dev' });
  assert.equal(m.viewSegment({ kind: 'dev' }), 'dev');
});

test('Dev desk: empty What landed, then the Reviews tab by keyboard', async () => {
  reset({ ...off, on: true, repo: 'demo-org/demo-app', polled_at: 'x' });
  m.deskStore._reset();
  const { host, root } = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await settle(20);
  const tabs = [...host.querySelectorAll('[role="tab"]')];
  assert.deepEqual(tabs.map((t) => t.textContent.replace(/\d+/g, '').trim()), ['What landed', 'Reviews']);
  assert.equal(tabs[0].getAttribute('aria-selected'), 'true');
  assert.doesNotMatch(host.textContent, /Loading/);
  tabs[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
  await settle();
  const now = [...host.querySelectorAll('[role="tab"]')];
  assert.equal(now[1].getAttribute('aria-selected'), 'true');
  assert.equal(document.activeElement, now[1]);
  root.unmount();
});

// Round 2 (Mira's review of #315): B1 toast, S2/S3 setup footer, S1 desk header, S4 tab targets.
test('round 2: on phone the hint is a viewport toast above the tab bar with the short copy', async () => {
  reset({ ...off, on: true, repo: 'demo-org/demo-app', branch: 'main', polled_at: '2026-11-05T08:00:00Z' });
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' }, phone: true }));
  await click(switchEl(host));
  const hint = host.querySelector('[data-dv-hint]');
  assert.equal(hint.textContent, 'Press again to turn Dev mode off');
  for (const c of ['fixed', 'inset-x-4', 'mx-auto', 'w-fit', 'max-w-[calc(100vw-32px)]', 'text-center', 'bottom-[calc(56px+env(safe-area-inset-bottom)+12px)]']) {
    assert.ok(hint.classList.contains(c), c);
  }
  assert.ok(!hint.classList.contains('whitespace-nowrap'), 'may wrap to 2 lines rather than run off');
  root.unmount();
});

test('round 2: the setup footer is sticky, says nothing is posted, and its buttons are 44 px on phone', async () => {
  reset();
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b = switchEl(host);
  await click(b);
  await click(b);
  const footer = document.querySelector('[data-dv-setup-footer]');
  assert.ok(footer.classList.contains('sticky') && footer.classList.contains('bottom-0'));
  assert.ok(footer.classList.contains('pb-[max(16px,env(safe-area-inset-bottom))]'));
  assert.match(footer.textContent, /Only you can turn this on or off\. Nothing is posted to GitHub\./);
  assert.doesNotMatch(document.body.textContent, /without asking/);
  for (const btn of footer.querySelectorAll('button')) assert.ok(btn.classList.contains('min-h-[44px]'), btn.textContent);
  root.unmount();
});

test('round 2: the Dev desk header shows the repo chip and when it was checked; tabs are 44 px on phone', async () => {
  reset();
  m.devStore.set('spc_demo', { ...off, on: true, repo: 'demo-org/demo-app', branch: 'trunk' });
  m.deskStore._reset();
  const { host, root } = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await settle(20);
  const chip = host.querySelector('[data-dev-repo-chip]');
  assert.equal(chip.textContent, 'demo-org/demo-app · trunk');
  assert.ok(chip.classList.contains('hidden') && chip.classList.contains('sm:inline-flex'), 'chip hides at 390');
  assert.match(host.querySelector('[data-dev-checked]').textContent, /^Checked \d\d:\d\d$/);
  for (const t of host.querySelectorAll('[role="tab"]')) assert.ok(t.classList.contains('min-h-[44px]'));
  root.unmount();
  m.devStore.set('spc_demo', { ...off, on: true, repo: null });
  m.deskStore._reset();
  const again = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await settle(20);
  assert.equal(again.host.querySelector('[data-dev-repo-chip]'), null, 'no repo known: no chip');
  again.root.unmount();
});
