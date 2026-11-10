// 0.9.0 H1 (Pixel): the Dev desk's third tab. Worktrees shows with its count when the server has
// Lin's routes, Refresh on it reloads the worktrees, and a server without them (404) shows no tab.
process.env.TZ = 'UTC';

import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/sp1/dev', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export * as fx from './src/dev/worktreeFixtures';
  export { DevDeskView, deskStore } from './src/components/DevDesk';
  export { worktreeStore } from './src/dev/worktrees';
  export { devStore, parseDevMode } from './src/devMode';
`);
const { React, createRoot, fx } = m;
const h = React.createElement;

let hasRoutes = true;
let listCalls = 0;
g.fetch = g.window.fetch = async (url) => {
  const u = String(url);
  const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
  if (u.includes('/dev/worktrees/branches')) return json({}, 404);
  if (u.endsWith('/dev/worktrees')) return (listCalls++, hasRoutes ? json(fx.LIST) : json({}, 404));
  if (u.includes('/dev/landed')) return json({ cards: [] });
  if (u.includes('/dev/reviews')) return json({ cards: [] });
  if (u.includes('/dev_mode')) return json({ on: true, repo: 'ann/shop', branch: 'main' });
  return json({});
};

const settle = async (n = 25) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const mounted = [];
afterEach(() => {
  while (mounted.length) mounted.pop()();
  document.body.replaceChildren();
});
const desk = async () => {
  m.deskStore._reset();
  m.worktreeStore._reset();
  localStorage.setItem('kwnt.dev.intro.sp1', '1');
  m.devStore.set('sp1', m.parseDevMode({ on: true, repo: 'ann/shop', branch: 'main' }));
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(h(m.DevDeskView, { spaceId: 'sp1', spaceName: 'Shop', now: fx.NOW, bots: fx.BOTS }));
  await settle();
  mounted.push(() => root.unmount());
  return host;
};
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('Worktrees is a tab with its count; it lists the repos; Refresh reloads it', async () => {
  hasRoutes = true;
  const host = await desk();
  const tab = host.querySelector('[data-dev-tab="worktrees"]');
  assert.ok(tab, 'the tab exists');
  assert.equal(tab.textContent.replace(/\s+/g, ' ').trim(), 'Worktrees5');
  assert.deepEqual([...host.querySelectorAll('[data-dev-tab]')].map((t) => t.dataset.devTab), ['landed', 'reviews', 'worktrees']);
  await click(tab);
  assert.equal(tab.getAttribute('aria-selected'), 'true');
  assert.ok(host.querySelector('[role="tabpanel"] [data-wt-panel] [data-wt-repo="shop"]'));
  assert.equal(host.querySelector('[data-dev-repo-filters]'), null);
  const before = listCalls;
  await click(host.querySelector('[data-dev-refresh]'));
  assert.equal(listCalls, before + 1);
});

test('a server without the worktree routes shows no Worktrees tab', async () => {
  hasRoutes = false;
  const host = await desk();
  assert.equal(host.querySelector('[data-dev-tab="worktrees"]'), null);
  assert.deepEqual([...host.querySelectorAll('[data-dev-tab]')].map((t) => t.dataset.devTab), ['landed', 'reviews']);
});
