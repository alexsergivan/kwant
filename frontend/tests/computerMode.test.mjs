import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

// The sound effects module reads its mute flag on load.
globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  export * from './src/computerMode';
  export { ComputerModeControl, ComputerModeChip, LocalBrowserNote } from './src/components/ComputerModeSwitch';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createElement } from 'react';
`);

const settings = (mode, local = true, vm = true, reasons = {}) => ({
  mode,
  available: { local, vm },
  reasons: { local: null, vm: null, ...reasons },
});
const api = (state) => ({ state, select: () => {}, dismissError: () => {} });
const render = (C, state) => m.renderToStaticMarkup(m.createElement(C, { api: api(state) }));
const ready = (s) => ({ ...m.initialSwitchState, settings: s });

test('parseComputerSettings accepts the contract and rejects anything else', () => {
  const ok = m.parseComputerSettings({ mode: 'vm', available: { local: false, vm: true }, reasons: { local: 'No Chrome found', vm: null } });
  assert.deepEqual(ok, settings('vm', false, true, { local: 'No Chrome found' }));
  assert.equal(m.parseComputerSettings({ mode: 'VM', available: {} }), null);
  assert.equal(m.parseComputerSettings({ mode: 'local' }), null);
  assert.equal(m.parseComputerSettings(null), null);
  // Missing reasons and non-boolean availability are tolerated, read as false/null.
  assert.deepEqual(m.parseComputerSettings({ mode: 'local', available: { local: 1, vm: true } }), settings('local', false, true));
});

test('unavailableReason uses the server reason, with a fallback', () => {
  assert.equal(m.unavailableReason(settings('local', true, false, { vm: 'Docker is not installed' }), 'vm'), 'Docker is not installed');
  assert.match(m.unavailableReason(settings('local', true, false), 'vm'), /VM \(isolated\) isn't available/);
  assert.equal(m.unavailableReason(settings('local'), 'vm'), null);
});

test('a failed PUT reverts the control and keeps the error', () => {
  let s = m.switchReducer(m.initialSwitchState, { type: 'loaded', settings: settings('local') });
  s = m.switchReducer(s, { type: 'start', mode: 'vm' });
  assert.equal(m.shownMode(s), 'vm');
  s = m.switchReducer(s, { type: 'failed', error: 'Docker is not running' });
  assert.equal(m.shownMode(s), 'local');
  assert.equal(s.error, 'Docker is not running');
  s = m.switchReducer(s, { type: 'start', mode: 'vm' });
  assert.equal(s.error, null, 'a new attempt clears the old error');
  s = m.switchReducer(s, { type: 'saved', settings: settings('vm') });
  assert.equal(m.shownMode(s), 'vm');
  assert.equal(s.pending, null);
});

test('a refetch while saving does not override the pending choice', () => {
  let s = m.switchReducer(ready(settings('local')), { type: 'start', mode: 'vm' });
  s = m.switchReducer(s, { type: 'loaded', settings: settings('local') });
  assert.equal(m.shownMode(s), 'vm');
});

test('saveComputerMode surfaces the 400 error body and network failures', async () => {
  const realFetch = globalThis.fetch;
  try {
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'Docker is not installed' }), { status: 400 });
    assert.deepEqual(await m.saveComputerMode('vm'), { ok: false, error: 'Docker is not installed' });
    globalThis.fetch = async () => new Response('oops', { status: 500 });
    assert.deepEqual(await m.saveComputerMode('vm'), { ok: false, error: 'Switching failed (HTTP 500).' });
    globalThis.fetch = async () => { throw new TypeError('offline'); };
    assert.deepEqual(await m.saveComputerMode('vm'), { ok: false, error: 'Could not reach the server.' });
    let body;
    globalThis.fetch = async (_u, init) => { body = init.body; return new Response(JSON.stringify(settings('vm')), { status: 200 }); };
    assert.deepEqual(await m.saveComputerMode('vm'), { ok: true, settings: settings('vm') });
    assert.equal(body, '{"mode":"vm"}');
    globalThis.fetch = async () => new Response('', { status: 404 });
    assert.equal(await m.fetchComputerSettings(), null, 'older servers without the route hide the switch');
  } finally {
    globalThis.fetch = realFetch;
  }
});

test('control: both available', () => {
  const html = render(m.ComputerModeControl, ready(settings('local')));
  assert.match(html, /role="radiogroup"/);
  assert.match(html, /aria-checked="true"[^>]*>.*Local computer/);
  assert.match(html, /Uses your Chrome and your files\.</);
  assert.doesNotMatch(html, /Commands ask/);
  assert.match(html, /Runs in Docker containers, isolated from your files\./);
  assert.doesNotMatch(html, /disabled=""/);
  assert.doesNotMatch(html, /Unavailable/);
});

test('control: one unavailable is disabled and says why', () => {
  const html = render(m.ComputerModeControl, ready(settings('local', true, false, { vm: 'Docker is not installed' })));
  assert.match(html, /aria-disabled="true"[^>]*disabled=""[^>]*>.*VM \(isolated\)/);
  assert.match(html, /Unavailable: Docker is not installed/);
});

test('control: PUT failed shows the error inline on the saved mode', () => {
  const html = render(m.ComputerModeControl, { settings: settings('local'), pending: null, error: 'Docker is not running' });
  assert.match(html, /role="alert"[^>]*>.*Docker is not running/);
  assert.match(html, /aria-checked="true"[^>]*>.*Local computer/);
});

test('control and chip render nothing before the setting loads', () => {
  assert.equal(render(m.ComputerModeControl, m.initialSwitchState), '');
  assert.equal(render(m.ComputerModeChip, m.initialSwitchState), '');
});

test('chip names the current mode and offers the other one', () => {
  const vm = render(m.ComputerModeChip, ready(settings('vm')));
  assert.match(vm, />VM</);
  assert.match(vm, /Click to switch to Local computer/);
  const blocked = render(m.ComputerModeChip, ready(settings('local', true, false, { vm: 'Docker is not installed' })));
  assert.match(blocked, /disabled=""/);
  assert.match(blocked, /VM \(isolated\) is unavailable: Docker is not installed/);
});

test('local note text', () => {
  assert.match(m.renderToStaticMarkup(m.createElement(m.LocalBrowserNote)), /Your bots are using a Chrome window on your computer\./);
});
