// 34e phone-board-link-opens-board (autonomy/phone-board-link-opens-board/brief.md): on a phone, a link straight to
// /s/<space>/board opens the Mission board, not the sidebar list (Mira's ux-pass-0940-goals Minor j). Parallel
// Universe and the Dev desk open the same way, as a tap does (showPage). The jar stays a tab of the list, the Inbox
// still opens over it, Back from the board returns to the list, and a computer is unchanged.
// Real SpaceApp in jsdom at 390 px with a faked server. Made-up words only.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default/board', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
/** The viewport width; the app's phone query is (max-width: 767px). */
let width = 390;
g.window.matchMedia = (q) => {
  const max = /max-width:\s*(\d+)px/.exec(q);
  return { matches: max ? width <= Number(max[1]) : false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} };
};
class FakeEvents {
  addEventListener() {}
  removeEventListener() {}
  close() {}
}
g.EventSource = g.window.EventSource = FakeEvents;
g.CSS ??= { escape: (v) => String(v) };

const bot = (id, name) => ({ id, name, title: 'Analyst', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
const routes = {
  '/api/kwants': [bot('k1', 'Ada'), bot('k2', 'Lin')],
  '/api/channels': [{ id: 'ch1', name: 'general', kwant_ids: ['k1', 'k2'], created_at: '' }],
  '/api/teams': [],
  '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
  '/api/settings': { mission_board: true },
  // The board's own lists, empty (missions.ts wraps each in an object)
  '/api/missions': { missions: [] },
  '/api/tasks': { tasks: [] },
  '/api/risks': { risks: [] },
  '/api/automation-discovery': { setting: 'off', daily_cap: 3, blocked_by: [], last_scan_at: null, suggestions: [], stats: {} },
};
g.fetch = g.window.fetch = async (url, init) => {
  const u = new URL(String(url), 'http://localhost');
  return json(u.pathname in routes ? routes[u.pathname] : init?.method === 'PUT' ? {} : []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { SpaceApp } from './src/App';
`);
const { React } = m;
const noop = () => {};
const settle = async (n = 40) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 2, ready_accounts: 1, unread: 0 } };

const mounted = new Set();
afterEach(() => {
  for (const unmount of mounted) unmount();
  document.body.replaceChildren();
  width = 390;
});
/** SpaceApp's first load on `initialView` (what /s/<space>/<view> parses to). */
const open = async (initialView, initialTarget) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(m.SpaceApp, {
    space, spaces: [space], initialView, initialTarget, onSwitchSpace: noop, onCreateSpace: noop, onOpenArchived: noop,
    onSpacesChanged: noop, onSpaceArchived: noop, onChatChange: noop,
  }));
  await settle();
  const unmount = () => {
    if (!mounted.delete(unmount)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(unmount);
  return host;
};
/** The phone's sidebar list shows its tab bar ("Default sections"); a page covers the list and has none. */
// Booleans only: a failed assert on a jsdom node makes node:test print the whole document (gigabytes).
const tabBarEl = (host) => host.querySelector('[aria-label="Default sections"]');
const tabBar = (host) => !!tabBarEl(host);
const paneEl = (host) => host.querySelector('[data-main-pane]');
const pane = (host) => !!paneEl(host);
const boardEl = (host) => [...host.querySelectorAll('h2')].find((h) => h.textContent.trim() === 'Mission board');
const boardTitle = (host) => !!boardEl(host);
const backButton = (host) => host.querySelector('button[aria-label="Back to chats"]');
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('34e: on a phone, a link to /s/<space>/board opens the Mission board, not the list', async () => {
  const host = await open({ kind: 'board' });
  assert.equal(tabBar(host), false, 'the sidebar list (with its tab bar) is not what shows');
  assert.equal(boardTitle(host), true, 'the Mission board is on screen');
  assert.equal(!!paneEl(host)?.contains(boardEl(host) ?? null), true, 'in the main pane');
});

test('34e: on a phone, Back from a board opened by link returns to the list', async () => {
  const host = await open({ kind: 'board' });
  const back = backButton(host);
  assert.equal(!!back, true, 'the board has its Back button on a phone');
  await click(back);
  assert.equal(tabBar(host), true, 'the list with its tab bar');
  assert.equal(boardTitle(host), false, 'the board is gone');
});

test('34e: on a phone, a link to Parallel Universe or the Dev desk opens that page over the list', async () => {
  for (const kind of ['parallel', 'dev']) {
    const host = await open({ kind });
    assert.equal(tabBar(host), false, `${kind}: not the list`);
    assert.equal(pane(host), true, `${kind}: the main pane shows`);
    for (const unmount of [...mounted]) unmount();
  }
});

test('34e: on a phone, a link to the jar stays on the list, with the Jar tab picked', async () => {
  const host = await open({ kind: 'jar' });
  assert.equal(tabBar(host), true, 'the list with its tab bar');
  assert.equal(tabBarEl(host).querySelector('[data-tab="jar"]')?.getAttribute('aria-current'), 'page', 'the Jar tab is the picked one');
});

test('34e: on a phone, the Inbox and a chat link still open over the list; a plain space link stays on the list', async () => {
  let host = await open({ kind: 'inbox', tab: 'new' });
  assert.equal(tabBar(host), false, 'inbox: not the list');
  for (const unmount of [...mounted]) unmount();
  host = await open(null, { botId: 'k1' });
  assert.equal(tabBar(host), false, 'a chat link: not the list');
  assert.equal(!!host.querySelector('[role="log"]'), true, 'the chat is open');
  for (const unmount of [...mounted]) unmount();
  host = await open(null);
  assert.equal(tabBar(host), true, 'a plain space link: the list');
});

test('34e: on a computer, a board link is unchanged: the sidebar and the board side by side', async () => {
  width = 1440;
  const host = await open({ kind: 'board' });
  assert.equal(tabBar(host), false, 'no phone tab bar');
  assert.equal(boardTitle(host), true, 'the Mission board');
  assert.equal(!!backButton(host), false, 'no phone Back button');
});
