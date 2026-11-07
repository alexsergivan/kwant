// 0.7.5 O1: "Install Claude Code" in one click. The Setup API's states (can_install, installing,
// done → sign-in, failed, not installable → link), the 409 wait, and "Set up Claude" starting it.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = g.window.matchMedia || ((q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));
// ClaudeSignIn and the engines reads only; the Setup API is the mock below.
g.fetch = g.window.fetch = async () => new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/setupInstall';
  export { EngineInstall } from './src/components/EngineInstall';
  export { StartPicker } from './src/components/StartPicker';
`);
const { React, createRoot, flushSync } = m;
const h = React.createElement;
const settle = async (n = 5) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};

const items = (claude = {}) => [
  { id: 'claude', label: 'Claude Code', state: 'missing', can_install: true, ...claude },
  { id: 'codex', label: 'Codex', state: 'missing', can_install: false },
  { id: 'grok', label: 'Grok', state: 'missing', can_install: true },
];
/** A Setup API that installs in `polls` reads, or fails, logging what it would. */
function fakeApi({ list = items(), polls = 2, fail = false, busy = 0 } = {}) {
  const calls = [];
  let reads = 0;
  return {
    calls,
    async status() {
      calls.push('status');
      return list;
    },
    async install(item) {
      calls.push(`install:${item}`);
      if (busy-- > 0) return { status: 409, body: { error: 'another install is running', job_id: 'setup-0' } };
      return { status: 202, body: { job_id: 'setup-1', item } };
    },
    async job(id, after) {
      calls.push(`job:${id}:${after}`);
      reads++;
      const lines = [`step ${reads}`];
      if (reads < polls) return { id, state: 'running', error: null, lines, next: after + 1 };
      if (fail) return { id, state: 'failed', error: 'exit status 1', lines: ['npm ERR! code EACCES'], next: after + 1 };
      return { id, state: 'done', error: null, lines: ['added 3 packages'], next: after + 1 };
    },
  };
}
const noWait = async () => {};

const mounted = [];
async function mount(el) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  flushSync(() => root.render(el));
  mounted.push(root);
  await settle();
  return host;
}
test.afterEach(async () => {
  while (mounted.length) {
    const r = mounted.pop();
    r.unmount();
  }
  document.body.innerHTML = '';
});
const signIn = h('p', { 'data-test-signin': '' }, 'Sign in with Claude');

test('can_install: "Install Claude Code" instead of the Terminal link', async () => {
  const api = fakeApi();
  const host = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', url: 'https://docs.example.com/claude', needed: true, api, sleep: noWait }, signIn));
  assert.equal(host.querySelector('[data-engine-install]').getAttribute('data-engine-install'), 'can-install');
  const btn = host.querySelector('button');
  assert.equal(btn.textContent, 'Install Claude Code');
  assert.equal(host.querySelector('a'), null, 'no link to the Terminal steps');
  assert.equal(host.querySelector('[data-test-signin]'), null);
});

test('installing: one quiet progress line, never the raw log', async () => {
  let release;
  const gate = new Promise((r) => (release = r));
  const api = fakeApi({ polls: 3 });
  const host = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', needed: true, api, sleep: () => gate }, signIn));
  host.querySelector('button').click();
  await settle();
  const line = host.querySelector('[data-engine-install="installing"]');
  assert.ok(line);
  assert.equal(line.getAttribute('role'), 'status');
  assert.equal(line.textContent, 'Installing Claude Code… (about 15 seconds)');
  assert.doesNotMatch(host.textContent, /step \d/);
  assert.match(line.querySelector('svg').getAttribute('class'), /motion-safe:animate-spin/, 'no spin under reduced motion');
  release();
});

test('done: the engines are read again and the sign-in comes straight after, no Re-check', async () => {
  const api = fakeApi({ polls: 2 });
  let installed = 0;
  const host = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', needed: true, api, sleep: noWait, onInstalled: () => installed++ }, signIn));
  host.querySelector('button').click();
  await settle(10);
  assert.equal(installed, 1);
  assert.ok(host.querySelector('[data-test-signin]'), 'sign-in shown');
  assert.doesNotMatch(host.textContent, /Re-check/);
  // Followed the job by its line index
  assert.deepEqual(api.calls.filter((c) => c.startsWith('job')), ['job:setup-1:0', 'job:setup-1:1']);
});

test('failed: one sentence, the last log line under Details, the link to do it by hand', async () => {
  const api = fakeApi({ polls: 2, fail: true });
  const host = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', url: 'https://docs.example.com/claude', needed: true, api, sleep: noWait }, signIn));
  host.querySelector('button').click();
  await settle(10);
  const box = host.querySelector('[data-engine-install="failed"]');
  assert.ok(box);
  assert.equal(box.getAttribute('role'), 'alert');
  assert.equal(box.querySelector('p').textContent, 'Claude Code didn’t install.');
  const details = box.querySelector('details');
  assert.equal(details.querySelector('summary').textContent, 'Details');
  assert.equal(details.querySelector('code').textContent, 'npm ERR! code EACCES');
  const a = box.querySelector('a');
  assert.equal(a.getAttribute('href'), 'https://docs.example.com/claude');
  assert.equal(a.getAttribute('target'), '_blank');
  assert.ok([...box.querySelectorAll('button')].some((b) => b.textContent === 'Try again'));
  assert.equal(host.querySelector('[data-test-signin]'), null);
});

test('not installable here: the link and Re-check, as before', async () => {
  const api = fakeApi({ list: items({ can_install: false }) });
  const host = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', url: 'https://docs.example.com/claude', needed: true, api, sleep: noWait }, signIn));
  assert.equal(host.querySelector('[data-engine-install]').getAttribute('data-engine-install'), 'manual');
  assert.equal(host.querySelector('button'), null);
  assert.match(host.textContent, /Claude Code isn’t on this computer yet\. Install Claude Code ?, then press Re-check\./);
  // An older server without /api/setup/status: the link too
  const old = { ...fakeApi(), status: async () => { throw new Error('HTTP 404'); } };
  const host2 = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', url: 'https://docs.example.com/claude', needed: true, api: old, sleep: noWait }));
  assert.equal(host2.querySelector('[data-engine-install]').getAttribute('data-engine-install'), 'manual');
});

test('another install running (409): waits and tries again', async () => {
  const api = fakeApi({ busy: 2 });
  const r = await m.runInstall(api, 'grok', { sleep: noWait });
  assert.deepEqual(r, { ok: true });
  assert.deepEqual(api.calls.filter((c) => c.startsWith('install')), ['install:grok', 'install:grok', 'install:grok']);
  // Already there (200) is done; a manual step (422) fails with the server's sentence
  assert.deepEqual(await m.runInstall({ ...api, install: async () => ({ status: 200, body: { already_installed: true } }) }, 'claude'), { ok: true });
  assert.deepEqual(await m.runInstall({ ...api, install: async () => ({ status: 422, body: { error: 'this part needs a step only you can do' } }) }, 'codex'), {
    ok: false,
    lastLine: 'this part needs a step only you can do',
  });
});

test('canInstall reads the status row: missing and can_install; unknown until loaded', () => {
  assert.equal(m.canInstall(null, 'claude'), null);
  assert.equal(m.canInstall(items(), 'claude'), true);
  assert.equal(m.canInstall(items(), 'codex'), false);
  assert.equal(m.canInstall(items({ state: 'ready', can_install: false }), 'claude'), false);
  assert.equal(m.canInstall([], 'grok'), false);
});

test('"Set up Claude" (a focus request) installs without a click, once, then signs in', async () => {
  const api = fakeApi({ polls: 1 });
  const host = await mount(h(m.EngineInstall, { item: 'claude', name: 'Claude Code', needed: true, api, sleep: noWait, autoStart: 1 }, signIn));
  await settle(10);
  assert.equal(api.calls.filter((c) => c.startsWith('install')).length, 1);
  assert.ok(host.querySelector('[data-test-signin]'));
});

test('start picker: Sign in with Claude while Claude Code is missing offers the install', async () => {
  const engines = [{ id: 'claude', name: 'Claude Code CLI', kind: 'cli', status: 'not_installed', detail: null, version: null, models: [], checked_at: null, enabled: true, ready: false }];
  g.fetch = g.window.fetch = async (url) =>
    String(url).startsWith('/api/setup/status')
      ? new Response(JSON.stringify({ items: items() }), { status: 200, headers: { 'Content-Type': 'application/json' } })
      : new Response('{}', { status: 404, headers: { 'Content-Type': 'application/json' } });
  const host = await mount(h(m.StartPicker, { engines, onSignedIn() {}, onKeyAdded() {}, initial: 'claude' }));
  assert.ok([...host.querySelectorAll('button')].some((b) => b.textContent === 'Install Claude Code'));
  assert.doesNotMatch(host.textContent, /Re-check/);
});
