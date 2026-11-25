// 36b a11y-basics (ux-a11y-1124/a11y-audit.md): a keyboard or screen-reader user had no <main>
// landmark and no skip link to jump past the sidebar, and the sidebar's rows nested a second
// interactive element inside the row's own button (axe nested-interactive, 4-5 nodes per screen,
// 89 serious nodes total). Red on main 3f75d4e9 (before this branch's fixes): no [data-skip-link],
// no <main>, and axe nested-interactive fires on the sidebar.
import assert from 'node:assert/strict';
import test, { after, afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import axe from 'axe-core';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'DOMParser', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
class FakeEvents {
  addEventListener() {}
  removeEventListener() {}
  close() {}
}
g.EventSource = g.window.EventSource = FakeEvents;

const bot = (id, name, title = 'Role') => ({ id, name, title, persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('k1', 'Ada', 'Lead'), bot('k2', 'Lin', 'Backend'), bot('k3', 'Pixel', 'Frontend')];
const team = { id: 't1', name: 'Ship', icon: '🚢', lead_kwant_id: 'k1', member_kwants: ['k1', 'k2', 'k3'] };
const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 3, ready_accounts: 1, unread: 0 } };
const routes = {
  '/api/spaces': [space],
  '/api/kwants': kwants,
  '/api/channels': [{ id: 'ch1', name: 'general', kwant_ids: ['k1', 'k2', 'k3'], created_at: '' }],
  '/api/teams': [team],
  '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
};
const json = (b) => new Response(JSON.stringify(b), { status: 200, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init) => {
  const path = String(url).split('?')[0];
  return json(path in routes ? routes[path] : init?.method === 'PUT' ? {} : []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { App } from './src/Shell';
`);
const { React } = m;
const settle = async () => {
  for (let i = 0; i < 30; i++) await new Promise((r) => setTimeout(r, 0));
};

const mounted = new Set();
afterEach(() => {
  for (const unmount of mounted) unmount();
  document.body.replaceChildren();
});
// Mounting Shell's App lets react-dom's scheduler register an idle-priority window.setTimeout
// (days-long delay) that jsdom keeps tracked even after unmount; only dom.window.close() (jsdom's
// own stopAllTimers teardown) clears it, so the process can exit instead of hanging after the run.
after(() => {
  dom.window.close();
});
const mountApp = async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(React.createElement(m.App)));
  await settle();
  const unmount = () => {
    if (!mounted.delete(unmount)) return;
    m.flushSync(() => root.unmount());
    host.remove();
  };
  mounted.add(unmount);
  return host;
};
const q = (sel) => document.querySelector(sel);
const focusables = () =>
  [...document.querySelectorAll('a[href], button:not([disabled]), input, select, textarea, [tabindex]')].filter((el) => el.tabIndex >= 0);

test('<main data-main-pane> holds the content pane', async () => {
  await mountApp();
  const main = q('main[data-main-pane]');
  assert.ok(main, 'a <main> landmark wraps the content pane');
  assert.equal(main.tagName, 'MAIN');
  assert.equal(main.id, 'main-pane');
});

test('the skip link is the first tab stop and moves focus to <main>', async () => {
  await mountApp();
  const link = q('[data-skip-link]');
  assert.ok(link, 'a skip link is on the page');
  assert.equal(link.textContent.trim(), 'Skip to conversation');
  assert.equal(focusables()[0], link, 'nothing else on the page comes before it in tab order');
  link.focus();
  link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
  await settle();
  assert.equal(document.activeElement, q('[data-main-pane]'), 'activating it moves focus to the main pane (no card is waiting)');
});

test('the skip link prefers a waiting Needs-you card over the plain conversation pane', async () => {
  await mountApp();
  // Stand in for a waiting card the way NeedsYouStrip renders it: a focusable section with the hook
  // Shell.tsx's click handler looks for, placed inside <main> as it would be in the real tree.
  const card = document.createElement('section');
  card.tabIndex = -1;
  card.setAttribute('data-needs-you-strip', 'ntf_1');
  q('[data-main-pane]').prepend(card);
  const link = q('[data-skip-link]');
  link.focus();
  link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
  await settle();
  assert.equal(document.activeElement, card, 'a waiting card wins over the bare main pane');
});

test('no nested-interactive in the sidebar', async () => {
  await mountApp();
  const sidebar = q('aside');
  assert.ok(sidebar, 'the sidebar is on screen');
  assert.ok(sidebar.querySelectorAll('[data-bot-row], [data-pin-row], [data-team-row]').length > 0, 'rows are actually rendered');
  const results = await axe.run(sidebar, { runOnly: { type: 'rule', values: ['nested-interactive'] } });
  assert.deepEqual(
    results.violations.map((v) => v.nodes.map((n) => n.target)),
    [],
    'axe nested-interactive',
  );
});

test('a long code block is its own focusable region, named "Code, N lines"', async () => {
  const ui = await loadBundled(`
    import React from 'react';
    import { renderToStaticMarkup } from 'react-dom/server.browser';
    import { MarkdownRenderer } from './src/components/MarkdownRenderer';
    export const render = (value) => renderToStaticMarkup(React.createElement(MarkdownRenderer, { content: '\`\`\`js\\n' + value + '\\n\`\`\`' }));
  `);
  const lines = Array.from({ length: 40 }, (_, i) => `line ${i + 1}`).join('\n');
  const html = ui.render(lines);
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  const pre = doc.querySelector('pre');
  assert.ok(pre, 'the code block renders');
  assert.equal(pre.getAttribute('tabindex'), '0', 'focusable so a keyboard user can scroll it');
  assert.equal(pre.getAttribute('role'), 'region');
  assert.equal(pre.getAttribute('aria-label'), 'Code, 40 lines');
});
