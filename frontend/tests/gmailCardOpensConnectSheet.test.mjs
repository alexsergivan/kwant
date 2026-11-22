// Pixel (2026-11-21), 32x gmail-card-opens-connect-sheet (autonomy/gmail-card-opens-connect-sheet/brief.md,
// Mira's UX pass 0.9.37 fix #2): the "suggests a connection · Gmail" card's button opened Settings →
// Apps & accounts → Advanced, with no Gmail on screen, and a card posted before Composio was added said
// "isn't added yet" forever. Now the button reads "Connect Gmail" and opens the Connect Gmail sheet with
// the bot ticked, and the card checks again on focus and when Settings closes.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia ??= () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
g.window.HTMLElement.prototype.scrollIntoView ??= () => {};
g.CSS ??= { escape: (x) => String(x).replace(/[^\w-]/g, (c) => '\\' + c) };

const m = await loadBundled(`
  export * from './src/connectionSuggestions';
  export { ConnectionSuggestionCard } from './src/components/ConnectionSuggestionCard';
  export { AppsAccounts } from './src/components/AppsAccounts';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
  export { flushSync } from 'react-dom';
`);
const h = m.createElement;
const src = (p) => readFileSync(new URL(`../src/${p}`, import.meta.url), 'utf8');

const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
// Settle on what's drawn, not on a fixed number of ticks.
const until = async (fn, what) => {
  const end = Date.now() + 3000;
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 5));
  }
};
const mount = (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const render = (p) => m.flushSync(() => root.render(h(C, p)));
  render(props);
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return { host, render };
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').replace(/’/g, "'").trim();
const buttons = (el) => [...el.querySelectorAll('button, a')].map(text);

const gmail = (over = {}) =>
  m.parseConnectionMeta({ kind: 'connection_suggestion', app: 'gmail', name: 'Gmail', reason: 'To read your landlord’s mail.', composio: 'not_added', kwant_id: 'k1', ...over });
// Composio answers the status call: an Error is "can't tell" (409 before Composio is added).
const statusApi = (answers) => {
  const calls = [];
  return {
    calls,
    connect: async () => ({ redirect_url: 'https://composio.dev/auth/x' }),
    status: async (slug) => {
      calls.push(slug);
      const a = answers.length > 1 ? answers.shift() : answers[0];
      if (a instanceof Error) throw a;
      return { connected: a };
    },
  };
};
const phase = (host) => host.querySelector('[data-connection-card]')?.getAttribute('data-connection-card');

test('1. the card’s button reads "Connect Gmail" and hands over its app and bot, not Advanced', async () => {
  for (const composio of ['not_added', 'needs_key', 'off']) {
    const opened = [];
    const meta = gmail({ composio });
    const { host } = mount(m.ConnectionSuggestionCard, {
      meta, messageId: 'm1', kwantName: 'Scout', kwantId: 'k1', api: statusApi([new Error('409')]), onOpenConnections: (x) => opened.push(x),
    });
    await new Promise((r) => setTimeout(r, 20));
    assert.equal(phase(host), 'missing', composio);
    assert.deepEqual(buttons(host), ['Connect Gmail'], composio);
    // Mira's fix 1: not_added / needs_key read as a next step (neutral, no icon); only `off` keeps the amber warning.
    const card = host.querySelector('[data-connection-card]');
    const amber = card.querySelector('.text-amber-200');
    const warnIcon = card.querySelector('svg.lucide-triangle-alert, svg.lucide-alert-triangle');
    if (composio === 'off') {
      assert.ok(amber && warnIcon, 'off keeps the amber AlertTriangle');
      assert.ok(card.querySelector('[class*="border-amber-500/30"]'), 'off keeps the amber border');
    } else {
      assert.equal(amber, null, `${composio}: no amber text`);
      assert.equal(warnIcon, null, `${composio}: no AlertTriangle`);
      assert.equal(card.querySelector('[class*="amber"]'), null, `${composio}: nothing amber`);
      assert.ok(card.querySelector('[data-next-step].text-zinc-300'), `${composio}: neutral next-step line`);
    }
    host.querySelector('[data-open-connections]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    assert.equal(opened.length, 1, composio);
    assert.deepEqual(m.connectSheetFor(opened[0]), { slug: 'gmail', initial: { picked: ['k1'] } }, composio);
    for (const u of [...mounted]) u();
  }
  assert.deepEqual(m.connectSheetFor(gmail({ kwant_id: null })), { slug: 'gmail', initial: { picked: [] } }, 'no bot: none ticked');
});

test('2. Apps & accounts given the card’s sheet opens Connect Gmail with that bot ticked', async () => {
  const composio = { id: 'composio', kind: 'mcp', name: 'Composio', description: 'Apps.', optional: true, added: false, connection_id: null, enabled: false, logins_stored: 'composio', warning: 'w', add: '/api/connections/composio' };
  const kwants = [{ id: 'k1', name: 'Scout', approvals_effective: 'ask' }, { id: 'k2', name: 'Ben', approvals_effective: 'ask' }];
  mount(m.AppsAccounts, {
    spaceName: 'Home', spaceMode: 'ask', kwants, data: { connections: [], counts: {}, vault: { status: 'ok' }, catalog: [composio] }, profiles: [], engines: [],
    manageAccounts: () => null, advanced: () => h('p', null, 'ADVANCED'), phone: false, initialSheet: m.connectSheetFor(gmail()),
  });
  const sheet = await until(() => document.querySelector('[data-connect-sheet="gmail"]'), 'the Gmail sheet');
  assert.match(text(sheet), /Connect Gmail/);
  const ticked = (id) => sheet.querySelector(`[data-sheet-bot="${id}"] input`).checked;
  assert.equal(ticked('k1'), true, 'the bot that asked is ticked');
  assert.equal(ticked('k2'), false);

  // Settings hands the card's sheet to Apps & accounts (and opens a fresh one on a second press).
  const ss = src('components/SpaceSettings.tsx');
  assert.match(ss, /initialSheet=\{p\.connectApp \? \{ slug: p\.connectApp\.slug, initial: p\.connectApp\.initial \} : undefined\}/);
  assert.match(ss, /key=\{`\$\{space\.id\}:\$\{p\.connectApp\?\.seq \?\? 0\}`\}/);
});

test('3. a card that said "isn’t added yet" turns Connected on focus and when Settings closes', async () => {
  // Focus: Composio added and Gmail signed in while the person was in another tab.
  const api = statusApi([new Error('409'), true]);
  const a = mount(m.ConnectionSuggestionCard, { meta: gmail(), messageId: 'm1', kwantName: 'Scout', api, onOpenConnections: () => {} });
  await until(() => api.calls.length >= 1, 'the first check');
  assert.equal(phase(a.host), 'missing');
  window.dispatchEvent(new window.Event('focus'));
  await until(() => phase(a.host) === 'connected', 'Connected after focus');
  assert.match(text(a.host), /Connected\. Scout can use Gmail from its next turn\./);
  for (const u of [...mounted]) u();

  // Settings closes (recheck goes up): connected now.
  const api2 = statusApi([new Error('409'), true]);
  const props = { meta: gmail(), messageId: 'm2', kwantName: 'Scout', api: api2, onOpenConnections: () => {}, recheck: 0 };
  const b = mount(m.ConnectionSuggestionCard, props);
  await until(() => api2.calls.length >= 1, 'the first check');
  assert.equal(phase(b.host), 'missing');
  b.render({ ...props, recheck: 1 });
  await until(() => phase(b.host) === 'connected', 'Connected after Settings closes');
  for (const u of [...mounted]) u();

  // Composio added but Gmail not signed in yet: the card offers its own Connect, not the old warning.
  const api3 = statusApi([new Error('409'), false]);
  const c = mount(m.ConnectionSuggestionCard, { meta: gmail(), messageId: 'm3', kwantName: 'Scout', api: api3, onOpenConnections: () => {} });
  await until(() => api3.calls.length >= 1, 'the first check');
  window.dispatchEvent(new window.Event('focus'));
  await until(() => phase(c.host) === 'ready', 'ready once Composio can start a sign-in');
  assert.deepEqual(buttons(c.host), ['Connect']);
  assert.doesNotMatch(text(c.host), /connects through Composio \(free\)/);
});

test('4. InboxNew and the tool notes still open Advanced; only the card opens the sheet', () => {
  const app = src('App.tsx');
  assert.match(app, /const openConnections = \(\) => openSpaceSettings\('apps', 'advanced'\);/);
  assert.equal((app.match(/onOpenConnections=\{openConnections\}/g) ?? []).length, 2, 'InboxNew and the chat view tool notes');
  assert.match(app, /<ConnectionSuggestionCard[\s\S]{0,400}onOpenConnections=\{openConnectSheet\}/);
  assert.match(app, /recheck=\{settingsClosedCount\}/);
  assert.match(app, /const openConnectSheet = \(meta: ConnectionSuggestionMeta\) => \{[\s\S]{0,200}openSpaceSettings\('apps', 'apps'\)/);
});

test('5. after the sheet is closed, leaving Apps & accounts and coming back doesn’t open it again (Rigel’s QA defect 1)', async () => {
  const composio = { id: 'composio', kind: 'mcp', name: 'Composio', description: 'Apps.', optional: true, added: false, connection_id: null, enabled: false, logins_stored: 'composio', warning: 'w', add: '/api/connections/composio' };
  const props = (initialSheet) => ({
    spaceName: 'Home', spaceMode: 'ask', kwants: [{ id: 'k1', name: 'Scout', approvals_effective: 'ask' }], data: { connections: [], counts: {}, vault: { status: 'ok' }, catalog: [composio] },
    profiles: [], engines: [], manageAccounts: () => null, advanced: () => h('p', null, 'ADVANCED'), phone: false, initialSheet,
  });
  // The mechanism: Apps & accounts opens whatever sheet it is handed each time it mounts (a tab switch remounts it).
  let a = mount(m.AppsAccounts, props(m.connectSheetFor(gmail())));
  await until(() => document.querySelector('[data-connect-sheet="gmail"]'), 'the Gmail sheet');
  for (const u of [...mounted]) u();
  // Back on Apps & accounts after General: App has cleared connectApp, so nothing is handed over and no sheet opens.
  a = mount(m.AppsAccounts, props(undefined));
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(document.querySelector('[data-connect-sheet]'), null, 'no sheet by itself');

  // App: every Settings tab change goes through changeSpaceSettingsTab, which clears connectApp first.
  const app = src('App.tsx');
  assert.match(app, /const changeSpaceSettingsTab = \(tab: SpaceSettingsTab\) => \{\s*setConnectApp\(null\);\s*setSpaceSettingsTab\(tab\);/);
  assert.match(app, /<SpaceSettings[\s\S]{0,200}onTab=\{changeSpaceSettingsTab\}/);
  assert.doesNotMatch(app, /onTab=\{setSpaceSettingsTab\}/);
});
