// 0.7.2 (Rigel qa-357.md N1, qa-072.md N1): opening the desk straight from a link to /s/<space>/dev
// clears the sidebar's new-reviews badge without a reload. The store is empty when the desk mounts;
// the sidebar's Dev mode state (fetched before the server saw the desk) arrives afterwards with the
// old count. Demo data only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo/dev', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const base = { on: true, repo: 'demo-org/demo-app', branch: 'main', deploy_hint: null, backup_command: null, last_seen_sha: null, polled_at: null, poll_error: null, calls_today: null, calls_cap: null, sources: [], status: 'watching', repos: [] };
let calls = [];
let releaseSeen = () => {};
g.fetch = g.window.fetch = async (url, init = {}) => {
  const u = String(url);
  const method = init.method ?? 'GET';
  calls.push([method, u]);
  const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
  if (method === 'POST' && u.endsWith('/dev_mode/seen')) {
    await new Promise((r) => (releaseSeen = r));
    return json({ ok: true });
  }
  if (u.includes('/dev/landed')) return json({ cards: [], rollup: { count: 0, areas: [] }, last_tag: null, last_merge: null });
  if (u.includes('/dev/reviews')) return json({ cards: [] });
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * from './src/devMode';
  export { DevDeskView, deskStore, rowBadge } from './src/components/DevDesk';
  export * as fx from './src/dev/fixtures';
`);
const h = m.React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const reset = () => {
  calls = [];
  m.devStore._reset();
  m.deskStore._reset();
  localStorage.clear();
  localStorage.setItem('kwnt.dev.intro.spc_demo', '1');
  document.body.innerHTML = '';
};
const mount = async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(m.DevDeskView, { spaceId: 'spc_demo', spaceName: 'Demo', now: m.fx.FIXTURE_NOW }));
  await settle();
  return root;
};
const badge = () => m.rowBadge(m.devStore.get('spc_demo'), 0);

test('deep link, state arrives before the server answers: the badge reads 0', async () => {
  reset();
  const root = await mount();
  assert.equal(m.devStore.get('spc_demo'), null, 'deep link: nothing in the store yet');
  assert.ok(calls.some(([meth, u]) => meth === 'POST' && u.endsWith('/api/spaces/spc_demo/dev_mode/seen')));
  m.devStore.set('spc_demo', m.parseDevMode({ ...base, new_reviews: 2 }));
  await settle();
  assert.equal(badge(), 0);
  releaseSeen();
  await settle();
  assert.equal(badge(), 0);
  root.unmount();
});

test('deep link, state arrives after the server answers: the badge reads 0', async () => {
  reset();
  const root = await mount();
  releaseSeen();
  await settle();
  m.devStore.set('spc_demo', m.parseDevMode({ ...base, new_reviews: 3 }));
  await settle();
  assert.equal(badge(), 0);
  root.unmount();
});

test('a review that comes in while the desk is open still counts', async () => {
  reset();
  const root = await mount();
  m.devStore.set('spc_demo', m.parseDevMode({ ...base, new_reviews: 2 }));
  releaseSeen();
  await settle();
  assert.equal(badge(), 0);
  m.devStore.set('spc_demo', { ...m.devStore.get('spc_demo'), new_reviews: 1 });
  await settle();
  assert.equal(badge(), 1);
  root.unmount();
});

test('after leaving the desk, a later count is left alone', async () => {
  reset();
  const root = await mount();
  root.unmount();
  m.devStore.set('spc_demo', m.parseDevMode({ ...base, new_reviews: 2 }));
  releaseSeen();
  await settle();
  assert.equal(badge(), 2);
});
