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
  export { DevModeSwitch, DevOffNote, DevFlipNote, DevBadge, HardHat } from './src/components/DevModeSwitch';
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

test('flow F1: off → on is one press (set up already); no cover while off', () => {
  let s = m.initialSwitch({ setup: false });
  assert.equal(m.switchLook(s).ring, false, 'off: no cover');
  assert.equal(m.switchLook(s).glyph, 'code');
  s = R(s, { type: 'press', now: 1000 });
  assert.equal(s.on, true);
  assert.equal(s.saving, true);
  assert.equal(s.phase, 'settling');
  assert.equal(s.settleUntil, 1000 + m.SETTLE_MS);
  assert.equal(s.announce.text, 'Dev mode on for Demo.');
  s = R({ ...s, saving: false }, { type: 'settled', now: 1000 + m.SETTLE_MS });
  const look = m.switchLook(s);
  assert.equal(look.look, 'on');
  assert.equal(look.ring, true, 'on: the 1 px cover');
  assert.equal(look.glyph, 'locked', 'on: lock shut');
  assert.equal(look.label, 'Dev mode');
  assert.equal(look.describe, 'Press twice to turn off.');
  assert.equal(look.tooltip, 'Dev mode is on. Press twice to turn it off.');
  assert.equal(m.switchLook(m.initialSwitch()).describe, 'Turns on reviews, the ship gate and the production floor.');
  assert.equal(m.switchLook(m.initialSwitch()).tooltip, 'Dev mode: plain-words merges, code reviews, production floor');
});

test('flow F2: on → off is two presses: arm (amber, open lock, the armed line), then off', () => {
  let s = m.initialSwitch({ on: true, setup: false });
  s = R(s, { type: 'press', now: 1000 });
  assert.equal(s.phase, 'lifted');
  assert.equal(s.on, true);
  assert.equal(s.announce.text, 'Press again to turn Dev mode off, or Escape to keep it on.');
  const look = m.switchLook(s);
  assert.equal(look.look, 'armed');
  assert.equal(look.glyph, 'unlocked');
  assert.equal(look.hint, 'Press again to turn Dev mode off');
  s = R(s, { type: 'press', now: 1200 });
  assert.equal(s.on, false);
  assert.equal(s.phase, 'settling');
  assert.match(s.announce.text, /^Dev mode off for Demo\./);
});

test('a double-click on Off turns it on once and never arms it; a press while saving is eaten', () => {
  let s = R(m.initialSwitch({ setup: false }), { type: 'press', now: 0 });
  const settling = { ...s, saving: false };
  assert.equal(R(settling, { type: 'press', now: 100 }), settling, 'second click inside the settle');
  assert.equal(R(settling, { type: 'press', now: m.SETTLE_MS - 1 }).phase, 'settling');
  s = R(settling, { type: 'settled', now: m.SETTLE_MS });
  assert.equal(s.phase, 'covered');
  assert.equal(R({ ...s, saving: true }, { type: 'press', now: 9999 }).phase, 'covered', 'a save in flight eats the press');
  assert.equal(R(s, { type: 'press', now: 9999 }).phase, 'lifted', 'after the settle, a press arms');
});

test('armed drops back after 5 s with nothing changed, not before: "Dev mode stays on."', () => {
  let s = R(m.initialSwitch({ on: true, setup: false }), { type: 'press', now: 1000 });
  assert.equal(R(s, { type: 'recover', now: 1000 + m.RECOVER_MS - 1 }).phase, 'lifted');
  s = R(s, { type: 'recover', now: 1000 + m.RECOVER_MS });
  assert.equal(s.phase, 'covered');
  assert.equal(s.on, true);
  assert.equal(s.announce.text, 'Dev mode stays on.');
});

test('Esc and blur re-cover an armed switch; they do nothing when covered', () => {
  const lifted = R(m.initialSwitch({ on: true, setup: false }), { type: 'press', now: 0 });
  for (const type of ['escape', 'blur']) {
    const s = R(lifted, { type });
    assert.equal(s.phase, 'covered', type);
    assert.equal(s.on, true, type);
    assert.equal(s.announce.text, 'Dev mode stays on.', type);
  }
  const covered = m.initialSwitch({ on: true });
  assert.equal(R(covered, { type: 'escape' }), covered);
  assert.equal(R(covered, { type: 'blur' }), covered);
});

test('reduced motion: nothing to settle; on is one press, off still two', () => {
  let s = m.initialSwitch({ setup: false, reduced: true });
  s = R(s, { type: 'press', now: 0 });
  assert.equal(s.on, true);
  assert.equal(s.phase, 'covered');
  assert.equal(s.settleUntil, 0);
  s = R({ ...s, saving: false }, { type: 'press', now: 20 });
  assert.equal(s.phase, 'lifted', 'one press arms, never flips');
  assert.equal(s.on, true);
});

test('a failed save puts the switch back, covered, with the reason announced', () => {
  let s = R(R(m.initialSwitch({ on: true }), { type: 'press', now: 0 }), { type: 'press', now: 100 });
  assert.equal(s.on, false);
  s = R(s, { type: 'save_failed', error: 'Only the space owner can change Dev mode.' });
  assert.equal(s.on, true);
  assert.equal(s.phase, 'covered');
  assert.equal(s.saving, false);
  assert.equal(s.error, 'Only the space owner can change Dev mode.');
  assert.equal(s.announce.text, 'Only the space owner can change Dev mode.');
  const f = R(R(m.initialSwitch(), { type: 'press', now: 0 }), { type: 'save_failed', error: 'Offline.' });
  assert.equal(f.on, false, 'a failed one-press on goes back to off');
});

test('the server changing Dev mode closes a held-open cover; a disabled switch says so', () => {
  const lifted = R(m.initialSwitch({ on: true, setup: false }), { type: 'press', now: 0 });
  const s = R(lifted, { type: 'server', state: { ...off, on: false, repo: 'demo-org/demo-app', polled_at: 'x' } });
  assert.equal(s.on, false);
  assert.equal(s.phase, 'covered');
  const d = R(m.initialSwitch({ on: true }), { type: 'server', state: null, disabled: true });
  assert.equal(R(d, { type: 'press', now: 0 }), d);
  const look = m.switchLook(d);
  assert.equal(look.tooltip, 'Only the space owner can change Dev mode.');
  assert.equal(look.ring, false, 'not the owner: no ring');
  assert.equal(look.glyph, 'locked');
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
const desc = (host, b) => {
  const d = host.querySelector('[data-dv-describe]');
  assert.equal(d.id, b.getAttribute('aria-describedby'));
  return d.textContent;
};
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
  assert.equal(b.getAttribute('role'), 'switch');
  assert.equal(b.getAttribute('aria-checked'), 'true');
  assert.equal(b.dataset.on, 'true');
  assert.equal(b.dataset.covered, 'true', 'on: the cover is closed');
  assert.equal(b.getAttribute('aria-label'), 'Dev mode');
  assert.equal(desc(host, b), 'Press twice to turn off.');
  assert.equal(b.title, 'Dev mode is on. Press twice to turn it off.');
  assert.ok(b.querySelector('[data-dv-glyph="locked"]'), 'on: lock shut');
  await click(b);
  assert.equal(b.dataset.devSwitch, 'lifted');
  assert.equal(b.dataset.armed, 'true');
  assert.equal(b.dataset.covered, undefined);
  assert.ok(b.querySelector('[data-dv-glyph="unlocked"]'), 'armed: lock open');
  assert.match(host.querySelector('[data-dv-live]').textContent, /Press again to turn Dev mode off, or Escape to keep it on\./);
  assert.ok(host.querySelector('[data-dv-hint]'));
  await click(b);
  await click(b); // the double-click's third press: ignored while settling
  assert.deepEqual(
    server.calls.filter(([meth]) => meth === 'PUT').map(([, , body]) => body),
    [{ on: false }],
  );
  assert.match(host.querySelector('[data-dv-live]').textContent, /Dev mode off for Demo/);
  assert.equal(turnedOff, 1);
  // The settle timer fires after SETTLE_MS; a busy CI runner can run it late, so poll for it
  await wait(m.SETTLE_MS);
  for (let i = 0; i < 100 && b.dataset.devSwitch !== 'covered'; i++) await wait(20);
  assert.equal(b.dataset.devSwitch, 'covered');
  assert.equal(b.dataset.on, undefined);
  assert.equal(b.getAttribute('aria-checked'), 'false');
  assert.equal(desc(host, b), 'Turns on reviews, the ship gate and the production floor.');
  await click(b); // a render mid-save once left the switch stuck; the next press still turns it on (one press)
  assert.equal(b.dataset.on, 'true');
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , body]) => body), [{ on: false }, { on: true }]);
  root.unmount();
});

test('DOM: off → on is one press: PUT {on:true}, onTurnedOn, and a double-click never arms it', async () => {
  reset({ ...off, on: false, repo: 'demo-org/demo-app', polled_at: 'x' });
  let turnedOn = 0;
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' }, onTurnedOn: () => turnedOn++ }));
  const b = switchEl(host);
  assert.equal(b.dataset.covered, undefined, 'off: no cover');
  assert.equal(b.querySelector('[data-dv-glyph]'), null, 'off: no lock');
  assert.equal(b.title, 'Dev mode: plain-words merges, code reviews, production floor');
  await click(b);
  await click(b);
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , body]) => body), [{ on: true }]);
  assert.equal(b.dataset.on, 'true');
  assert.equal(b.dataset.armed, undefined, 'the double-click’s second press lands in the settle');
  assert.equal(turnedOn, 1);
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
  assert.match(host.querySelector('[data-dv-live]').textContent, /Dev mode stays on\./);
  assert.equal(server.calls.filter(([meth]) => meth === 'PUT').length, 0);
  root.unmount();
});

test('DOM: a 403 on load shows the switch disabled', async () => {
  reset();
  server.fail = { method: 'GET', path: '/dev_mode', status: 403, error: 'only the person' };
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  assert.equal(switchEl(host).disabled, true);
  assert.ok(switchEl(host).querySelector('[data-dv-glyph="locked"]'), 'disabled shows a closed lock');
  root.unmount();
});

test('DOM: reduced motion shows the same switch, turns on in one press and arms with no settle', async () => {
  reset({ ...off, on: false, repo: 'demo-org/demo-app', polled_at: 'x' });
  window.matchMedia = (q) => ({ matches: q.includes('reduce'), media: q, addEventListener() {}, removeEventListener() {} });
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b = switchEl(host);
  assert.ok(b.classList.contains('kw-dv-rm'));
  assert.ok(b.querySelector('[data-dv-graph] .kw-dv-node'), 'the same branch graph');
  assert.equal(b.textContent.trim(), 'dev', 'the reduced-motion variant keeps its dev label');
  await click(b);
  assert.equal(b.dataset.devSwitch, 'covered');
  assert.equal(b.dataset.on, 'true');
  await wait(30); // the save comes back; presses wait for it, motion or not
  await settle();
  await click(b);
  assert.equal(b.dataset.devSwitch, 'lifted', 'the next press arms at once: nothing to settle');
  assert.equal(b.querySelector('[data-dv-word]').textContent, 'off?');
  root.unmount();
  delete window.matchMedia;
});

test('switchLook names the visual state and the knob glyph: off, armed, on, disabled (0.7.1: no pending look)', () => {
  const st = (o) => ({ ...m.initialSwitch({ reduced: false }), ...o });
  const pick = (o) => { const l = m.switchLook(st(o)); return [l.look, l.glyph]; };
  assert.deepEqual(pick({ on: false }), ['off', 'code']);
  assert.deepEqual(pick({ on: true }), ['on', 'locked']);
  assert.deepEqual(pick({ on: true, phase: 'lifted', liftedAt: 0 }), ['armed', 'unlocked']);
  assert.deepEqual(pick({ disabled: true, on: true }), ['disabled', 'locked']);
});

test('0.7.1 look (style v2 §10): the Branch switch; label dev → (one press) dev → off?; 44 px tall on phone; no knob or track', async () => {
  reset({ ...off, on: false, repo: 'demo-org/demo-app', polled_at: 'x' });
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' }, phone: true }));
  const b = switchEl(host);
  const word = () => b.querySelector('[data-dv-word]').textContent;
  assert.ok(b.classList.contains('kw-dv') && !b.classList.contains('kw-dv-rm'));
  assert.ok(b.classList.contains('h-11'), 'the phone target is 44 px tall');
  assert.ok(host.querySelector('[data-dev-switch-wrap]').className.includes('min-w-[44px]'));
  for (const gone of ['.kw-dv-cover', '.kw-dv-hinge', '.kw-dv-ring', '.kw-dv-knob', '.kw-dv-track']) assert.equal(b.querySelector(gone), null, gone);
  const svg = b.querySelector('svg[data-dv-graph]');
  assert.equal(svg.getAttribute('viewBox'), '0 0 44 24');
  assert.equal(svg.querySelector('.kw-dv-base').getAttribute('d'), 'M2 17H42');
  assert.equal(svg.querySelector('.kw-dv-fork').getAttribute('d'), 'M12 17C19 17 18 7 26 7H42');
  assert.equal(svg.querySelector('.kw-dv-dot').getAttribute('r'), '5');
  assert.equal(svg.querySelector('.kw-dv-pip').getAttribute('r'), '1.6');
  assert.equal(word(), 'dev');
  assert.equal(b.dataset.armed, undefined);
  await click(b);
  assert.equal(b.dataset.on, 'true', 'one press: on');
  assert.equal(b.dataset.armed, undefined, 'never armed on the way on');
  assert.equal(word(), 'dev');
  root.unmount();
  reset({ ...off, on: true, repo: 'demo-org/demo-app', polled_at: 'x' });
  const on = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b2 = switchEl(on.host);
  assert.equal(b2.dataset.on, 'true');
  assert.equal(b2.querySelector('[data-dv-word]').textContent, 'dev');
  await click(b2);
  assert.equal(b2.querySelector('[data-dv-word]').textContent, 'off?', 'armed to turn off asks off?');
  assert.equal(b2.dataset.on, 'true', 'the node stays on the fork while armed to turn off');
  on.root.unmount();
});

test('flip notes: HEAD → dev with one caret, and HEAD → main (flow copy §7)', () => {
  const onNote = m.renderToStaticMarkup(h(m.DevFlipNote, { on: true, space: 'Trailhead', onDismiss: () => {} }));
  assert.match(onNote, /role="status"/);
  assert.match(onNote, /HEAD → dev/);
  assert.match(onNote, /class="dv-caret"/);
  assert.match(onNote, /Dev mode is on for Trailhead\./);
  const offNote = m.renderToStaticMarkup(h(m.DevFlipNote, { on: false, approvals: 'smart', onDismiss: () => {} }));
  assert.match(offNote, /HEAD → main/);
  assert.match(offNote, /Dev mode is off\.<\/strong> Reviews, the ship gate and the production floor stopped\. Approvals stay on Smart\. What\s+landed keeps its history\./);
  assert.doesNotMatch(offNote, /dv-caret/);
});

test('DOM 0.7.1: the first switch-on turns Dev mode on with no dialog and sends only {on: true}', async () => {
  reset();
  const { host, root } = await mount(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' } }));
  const b = switchEl(host);
  let turnedOn = null;
  root.render(h(m.DevModeSwitch, { space: { id: 'spc_demo', name: 'Demo' }, onTurnedOn: (s) => (turnedOn = s) }));
  await settle();
  assert.equal(b.getAttribute('aria-checked'), 'false');
  await click(b); // flow F1: one press
  await settle();
  assert.equal(turnedOn?.on, true, 'onTurnedOn gets the saved state, for the toast');
  assert.equal(document.querySelector('[role="dialog"]'), null, 'no setup sheet');
  assert.equal(document.querySelector('[data-dev-setup]'), null);
  assert.deepEqual(server.calls.filter(([meth]) => meth === 'PUT').map(([, , body]) => body), [{ on: true }]);
  assert.equal(server.calls.filter(([, u]) => u.endsWith('/check')).length, 0, 'no repo check');
  assert.equal(b.dataset.on, 'true');
  assert.equal(m.devStore.isOn('spc_demo'), true);
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
  // 0.7.1: the switch no longer opens the sheet; the component is checked on its own.
  const { root } = await mount(h(m.DevModeSetup, { space: { id: 'spc_demo', name: 'Demo' }, initial: null, onCancel() {}, onDone() {} }));
  const footer = document.querySelector('[data-dv-setup-footer]');
  assert.ok(footer.classList.contains('sticky') && footer.classList.contains('bottom-0'));
  assert.ok(footer.classList.contains('pb-[max(16px,env(safe-area-inset-bottom))]'));
  assert.match(footer.textContent, /Only you can turn this on or off\. Nothing is posted to GitHub unless you press Post\./);
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
  // style-v2 §3: tabs are .kw-dv-tab (44 px under 640 px in devStatus.css), active = 2 px underline.
  for (const t of host.querySelectorAll('[role="tab"]')) assert.ok(t.classList.contains('kw-dv-tab'));
  root.unmount();
  m.devStore.set('spc_demo', { ...off, on: true, repo: null });
  m.deskStore._reset();
  const again = await mount(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo' }));
  await settle(20);
  assert.equal(again.host.querySelector('[data-dev-repo-chip]'), null, 'no repo known: no chip');
  again.root.unmount();
});
