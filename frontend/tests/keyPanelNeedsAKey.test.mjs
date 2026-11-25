// 32y key-panel-needs-a-key (Mira's UX pass 0.9.37, fix #3 + minor a). In Get started → Use an API key,
// Add account waits for a key, the name starts as the provider and the account is the default; the
// Settings form keeps today's behaviour. When getting a CLI ready stalls for 20 s, a FailRow offers
// Try again and Choose another AI.
import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));

/** Fake server: records every call; profiles and setup routes answer, the rest are 404. */
let calls = [];
let setupItems = [{ id: 'claude', label: 'Claude Code', state: 'missing', can_install: true }];
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url).replace(/^https?:\/\/[^/]+/, '');
  const method = (init.method || 'GET').toUpperCase();
  calls.push({ method, u, body: init.body ? JSON.parse(init.body) : null });
  if (u === '/api/profiles' && method === 'POST') return json({ id: 'p1', ...JSON.parse(init.body) });
  if (u === '/api/setup/status') return json({ items: setupItems });
  if (u === '/api/setup/install') return json({ job_id: 'j1', item: 'claude' }, 202);
  if (u.startsWith('/api/setup/jobs/')) return json({ id: 'j1', state: 'running', error: null, lines: [], next: 0 });
  return json({}, 404);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/gettingStarted';
  export * from './src/gettingStartedInstall';
  export { GetStarted } from './src/components/GetStarted';
  export { NewKeyAccountForm } from './src/components/EngineAccounts';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const tick = () => new Promise((r) => setImmediate(r));
const settle = async (n = 8) => {
  for (let i = 0; i < n; i++) await tick();
};

const engine = (id, over = {}) => ({
  id, name: id, kind: id.endsWith('_api') ? 'api' : 'cli', status: 'not_installed', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: false, uses: null, ...over,
});

function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() => root.render(el));
  return { host, root, done: () => (root.unmount(), host.remove()) };
}
function typeInto(el, value) {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
}
const creates = () => calls.filter((c) => c.method === 'POST' && c.u === '/api/profiles');
const installs = () => calls.filter((c) => c.method === 'POST' && c.u === '/api/setup/install');

async function keyPanel(space) {
  localStorage.clear();
  calls = [];
  m.storeChoice(space, 'key');
  let added = 0;
  const r = mount(h(m.GetStarted, { space, bots: [], engines: [engine('claude')], reloadEngines: () => added++, onDismissed() {}, hostOverride: null }));
  await settle();
  const panel = r.host.querySelector('[data-key-panel]');
  assert.ok(panel, 'the key panel is open');
  const form = () => panel.querySelector('form[aria-label="New account"]');
  const q = {
    form,
    name: () => form().querySelector('input[id$="-name"]'),
    key: () => form().querySelector('input[type="password"]'),
    def: () => form().querySelector('input[type="checkbox"]'),
    add: () => [...form().querySelectorAll('button')].find((b) => /Add account/.test(b.textContent)),
    cancel: () => [...form().querySelectorAll('button')].find((b) => b.textContent.trim() === 'Cancel'),
  };
  return { r, panel, q, added: () => added };
}

// ------------------------------------------------------------------ the Get started key panel

test('32y: Get started key panel: Add account is disabled with an empty key, says to paste it, and Enter adds nothing', async () => {
  const { r, panel, q } = await keyPanel('k1');
  assert.equal(q.add().disabled, true, 'disabled with no key');
  assert.match(panel.textContent, /Paste your key to continue/);
  const hint = [...panel.querySelectorAll('p')].find((p) => /Paste your key to continue/.test(p.textContent));
  assert.ok(hint.id && q.key().getAttribute('aria-describedby')?.split(' ').includes(hint.id), 'the hint describes the key field');
  // Enter in Name (a form submit) with no key creates nothing.
  flushSync(() => q.form().dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true })));
  await settle();
  assert.equal(creates().length, 0, 'no keyless account');
  flushSync(() => typeInto(q.key(), 'sk-ant-test'));
  assert.equal(q.add().disabled, false, 'enabled once a key is typed');
  // The hint keeps its line (no jump) but is hidden and no longer describes the field.
  assert.match(hint.className, /\binvisible\b/);
  assert.equal(q.key().getAttribute('aria-describedby'), null);
  flushSync(() => typeInto(q.key(), '   '));
  assert.equal(q.add().disabled, true, 'spaces are not a key');
  r.done();
});

test('32y: Get started key panel: name starts as "Anthropic", default ticked, no Cancel, key label drops "can be added later"', async () => {
  const { r, panel, q } = await keyPanel('k2');
  assert.equal(q.name().value, 'Anthropic');
  assert.equal(q.def().checked, true);
  assert.equal(q.cancel(), undefined, 'no Cancel in Get started');
  assert.doesNotMatch(panel.textContent, /can be added later/);
  assert.match(panel.textContent, /never shown again/);
  r.done();
});

test('32y: with a key, one press creates one "Anthropic" default account with the key', async () => {
  const { r, q, added } = await keyPanel('k3');
  flushSync(() => typeInto(q.key(), '  sk-ant-test  '));
  flushSync(() => q.add().click());
  await settle();
  assert.equal(creates().length, 1);
  assert.deepEqual(creates()[0].body, { engine: 'claude', name: 'Anthropic', kind: 'api_key', is_default: true, api_key: 'sk-ant-test' });
  assert.equal(added(), 1, 'step 2 reloads the engines once');
  r.done();
});

test('32y: switching to OpenAI starts the name as "OpenAI" (xAI likewise)', async () => {
  const { r, panel, q } = await keyPanel('k4');
  const radio = (label) => [...panel.querySelectorAll('[role="radio"]')].find((b) => b.textContent === label);
  flushSync(() => radio('OpenAI').click());
  assert.equal(q.name().value, 'OpenAI');
  assert.equal(q.def().checked, true);
  flushSync(() => radio('xAI').click());
  assert.equal(q.name().value, 'xAI');
  r.done();
});

test('35a: the Mistral card names the account "Mistral", says how it is paid, and adds a Mistral key account', async () => {
  const { r, panel, q } = await keyPanel('k5');
  const radio = (label) => [...panel.querySelectorAll('[role="radio"]')].find((b) => b.textContent === label);
  assert.ok(radio('Mistral'), 'a Mistral card next to Anthropic, OpenAI and xAI');
  flushSync(() => radio('Mistral').click());
  assert.equal(q.name().value, 'Mistral');
  assert.match(panel.textContent, /Paid per use on your Mistral account\. Mistral's free plan is too limited for bots\. The key stays on this computer\./);
  flushSync(() => typeInto(q.key(), 'mistral-test-key'));
  flushSync(() => q.add().click());
  await settle();
  assert.deepEqual(creates()[0].body, { engine: 'mistral', name: 'Mistral', kind: 'api_key', is_default: true, api_key: 'mistral-test-key' });
  r.done();
});

// ------------------------------------------------------------------ the Settings form, unchanged

test('32y (guard): the form without the Get started mode keeps today: empty name, default unticked, Cancel, keyless add', async () => {
  calls = [];
  let created = 0;
  let cancelled = 0;
  const r = mount(h(m.NewKeyAccountForm, { card: 'anthropic_api', onCancel: () => cancelled++, onCreated: () => created++ }));
  const form = r.host.querySelector('form');
  const name = form.querySelector('input[id$="-name"]');
  assert.equal(name.value, '');
  assert.equal(form.querySelector('input[type="checkbox"]').checked, false);
  assert.match(form.textContent, /can be added later, never shown again/);
  assert.doesNotMatch(form.textContent, /Paste your key to continue/);
  const cancel = [...form.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Cancel');
  assert.ok(cancel, 'Cancel shown');
  flushSync(() => cancel.click());
  assert.equal(cancelled, 1);
  flushSync(() => typeInto(name, 'Work'));
  const add = [...form.querySelectorAll('button')].find((b) => /Add account/.test(b.textContent));
  assert.equal(add.disabled, false, 'a name is enough here');
  flushSync(() => add.click());
  await settle();
  assert.equal(creates().length, 1);
  assert.deepEqual(creates()[0].body, { engine: 'claude', name: 'Work', kind: 'api_key', is_default: false });
  assert.equal(created, 1);
  r.done();
});

// ------------------------------------------------------------------ minor a: "Getting {name} ready…" stalls

test('32y: track done but the CLI still missing: the spinner first, after 20 s a FailRow with Try again and Choose another AI', async () => {
  localStorage.clear();
  calls = [];
  m.storeChoice('s1', 'claude');
  const fakeApi = {
    async status() { return setupItems; },
    async install() { return { status: 200, body: { already_installed: true } }; },
    async job() { return { state: 'done' }; },
  };
  const t = await m.startInstall('s1', 'claude', { api: fakeApi, sleep: async () => {}, online: () => true });
  assert.equal(t.state, 'done');
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const r = mount(h(m.GetStarted, { space: 's1', bots: [], engines: [engine('claude')], reloadEngines() {}, onDismissed() {}, hostOverride: null }));
    await settle();
    assert.match(r.host.textContent, /Getting Claude ready…/);
    assert.equal(r.host.querySelector('[data-fail-row]'), null, 'no failure before 20 s');
    flushSync(() => mock.timers.tick(19_000));
    await settle();
    assert.equal(r.host.querySelector('[data-fail-row]'), null, 'still waiting at 19 s');
    flushSync(() => mock.timers.tick(1_000));
    await settle();
    const row = r.host.querySelector('[data-fail-row]');
    assert.ok(row, 'the FailRow after 20 s');
    assert.equal(row.getAttribute('data-fail-row'), 'bad');
    assert.equal(row.getAttribute('role'), 'alert');
    assert.match(row.textContent, /Getting Claude ready is taking too long\./);
    assert.match(row.textContent, /It usually takes a few seconds, and kwant still can’t find it\./);
    assert.doesNotMatch(r.host.textContent, /Getting Claude ready…/, 'the spinner line is gone');
    const labels = [...row.querySelectorAll('button')].map((b) => b.textContent.trim());
    assert.deepEqual(labels, ['Try again', 'Choose another AI']);
    calls = [];
    flushSync(() => [...row.querySelectorAll('button')][0].click());
    await settle();
    assert.equal(installs().length, 1, 'Try again runs the install once');
    // Choose another AI goes back to step 1.
    m.forgetInstall('s1');
    r.done();
  } finally {
    mock.timers.reset();
  }
});

test('32y: Choose another AI on the stall row opens step 1', async () => {
  localStorage.clear();
  m.storeChoice('s2', 'claude');
  const fakeApi = { async status() { return setupItems; }, async install() { return { status: 200, body: { already_installed: true } }; }, async job() { return { state: 'done' }; } };
  await m.startInstall('s2', 'claude', { api: fakeApi, sleep: async () => {}, online: () => true });
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const r = mount(h(m.GetStarted, { space: 's2', bots: [], engines: [engine('claude')], reloadEngines() {}, onDismissed() {}, hostOverride: null }));
    await settle();
    flushSync(() => mock.timers.tick(20_000));
    await settle();
    const row = r.host.querySelector('[data-fail-row]');
    assert.ok(row);
    flushSync(() => [...row.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Choose another AI').click());
    await settle();
    assert.equal(r.host.querySelector('[data-get-started]').getAttribute('data-open-step'), '1');
    r.done();
  } finally {
    mock.timers.reset();
  }
});
