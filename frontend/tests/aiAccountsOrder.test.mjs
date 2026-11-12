// AI accounts order (autonomy/apps-accounts/ai-accounts-order.md, from Oleks's "this not very intuitive"):
// rows titled by the AI, grouped Used first / If it runs out / Not used, "Use first" instead of grip,
// number and arrows, Off accounts outside the order with Turn on, and a no-backup line. Display only:
// the order is still each account's priority, saved with PATCH /api/profiles/:id.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'CSS']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.CSS ??= { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };
g.window.CSS ??= g.CSS;

const m = await loadBundled(`
  export { AccountPriority } from './src/components/AccountPriority';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
  export { act } from 'react';
`);
const h = m.createElement;
const NOW = Date.parse('2026-11-12T20:00:00');

const profile = (over) => ({
  id: 'p', engine: 'claude', name: 'This machine', kind: 'system', enabled: true, is_default: false, priority: 0,
  home_dir: null, has_key: false, signed_in: true, account_label: null, limited_until: null, last_error: null, ...over,
});
const engine = (id, over = {}) => ({ id, name: id, kind: 'cli', status: 'ready', detail: null, version: '1', models: [], checked_at: null, enabled: true, ready: true, uses: null, ...over });

// Oleks's shot: 1 Personal/Claude Ready, 2 This machine/Grok Ready, 3 This machine/Claude Off, 4 This machine/Codex Off.
const oleks = () => [
  profile({ id: 'personal', name: 'Personal', kind: 'cli_home', priority: 10 }),
  profile({ id: 'sys-grok', engine: 'grok', priority: 20 }),
  profile({ id: 'sys-claude', priority: 30, enabled: false }),
  profile({ id: 'sys-codex', engine: 'codex', priority: 40 }),
];
const oleksEngines = [engine('claude'), engine('grok'), engine('codex', { enabled: false, status: 'disabled' })];

// ── A tiny server: GET is the list, PATCH changes one account ─────────────────────────────────
const server = (profiles) => {
  const calls = [];
  g.fetch = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const body = init.body ? JSON.parse(init.body) : undefined;
    calls.push([method, url, body]);
    const id = decodeURIComponent(url.split('/').pop());
    const p = profiles.find((x) => x.id === id);
    if (method === 'PATCH' && p) Object.assign(p, body);
    return { ok: true, status: 200, json: async () => p ?? profiles };
  };
  return calls;
};

const mounted = [];
afterEach(() => {
  for (const r of mounted.splice(0)) r.unmount();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
/** Mounts the list; onChanged re-renders it from the fake server, like the page's reload does. */
const mount = async (profiles, engines, extra = {}) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  mounted.push(root);
  const engineOn = [];
  const props = () => ({
    profiles: profiles.map((p) => ({ ...p })),
    engines,
    spaceName: 'Binan',
    now: NOW,
    onChanged: async () => root.render(h(m.AccountPriority, props())),
    onEngineOn: async (id) => {
      engineOn.push(id);
      const e = engines.find((x) => x.id === id);
      if (e) Object.assign(e, { enabled: true, status: 'ready' });
    },
    onAddAccount: () => {},
    ...extra,
  });
  root.render(h(m.AccountPriority, props()));
  await settle();
  return { host, engineOn };
};
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const titles = (el) => [...(el?.querySelectorAll('[data-account-title]') ?? [])].map(text);
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

test('no row is titled "This machine": titles are the AI, and the built-in account says where it is', () => {
  const html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: oleks(), engines: oleksEngines, onChanged: async () => {}, now: NOW, spaceName: 'Binan' }));
  assert.doesNotMatch(html, /This machine/);
  assert.match(html, /Your bots in Binan use the first account that’s ready\. If it runs out, they switch to the next one by themselves\./);
  assert.match(html, /Signed in on this computer/);
  assert.doesNotMatch(html, /Order to use|draggable|aria-label="Move /, 'no grip, numbers or arrows');
});

test('Oleks’s four accounts: Claude (Personal) first, Grok if it runs out, Claude and Codex on this computer not used, each with Turn on', async () => {
  server(oleks());
  const { host } = await mount(oleks(), oleksEngines.map((e) => ({ ...e })));
  const first = host.querySelector('[data-group="first"]');
  assert.deepEqual(titles(first), ['Claude']);
  assert.match(text(first), /Personal/);
  assert.deepEqual(titles(host.querySelector('[data-group="backup"]')), ['Grok']);
  const notUsed = host.querySelector('[data-group="not-used"]');
  assert.deepEqual(titles(notUsed), ['Claude', 'Codex']);
  assert.deepEqual([...notUsed.querySelectorAll('[data-not-used]')].map((li) => li.getAttribute('data-not-used')), ['off', 'off']);
  assert.equal(notUsed.querySelectorAll('[data-turn-on]').length, 2);
  assert.match(text(notUsed), /On this computer · Turned off/);
  assert.equal(host.querySelector('[data-no-backup]'), null, 'Grok is the backup');
  assert.ok(host.querySelector('[data-group="backup"] [data-use-first]'), 'the backup row has Use first');
  assert.equal(host.querySelector('[data-group="first"] [data-use-first]'), null);
});

test('Turn on saves enabled with a place at the end of "If it runs out", and the row moves there', async () => {
  const profs = oleks();
  const calls = server(profs);
  const engines = oleksEngines.map((e) => ({ ...e }));
  const { host, engineOn } = await mount(profs, engines);
  // Claude on this computer: the account itself is off.
  await click(host.querySelector('[data-account="sys-claude"] [data-turn-on]'));
  const patch = calls.find(([meth, url]) => meth === 'PATCH' && url.endsWith('/sys-claude'));
  assert.equal(patch[2].enabled, true);
  // Its old number (30) already sorts after Grok (20), so it keeps it; any new one must too.
  assert.ok((patch[2].priority ?? profs.find((p) => p.id === 'sys-claude').priority) > 20, `after Grok (${patch[2].priority})`);
  assert.deepEqual(titles(host.querySelector('[data-group="backup"]')), ['Grok', 'Claude']);
  assert.equal(host.querySelector('[data-group="backup"] li:last-child').getAttribute('data-account'), 'sys-claude');
  // Codex: the whole engine is off, so its own switch is turned on too.
  await click(host.querySelector('[data-account="sys-codex"] [data-turn-on]'));
  assert.deepEqual(engineOn, ['codex']);
  assert.deepEqual([...host.querySelectorAll('[data-group="backup"] li')].map((li) => li.getAttribute('data-account')), ['sys-grok', 'sys-claude', 'sys-codex']);
  assert.equal(host.querySelector('[data-group="not-used"]'), null);
});

test('Use first saves the new priorities and puts that account in "Used first"', async () => {
  const profs = oleks();
  const calls = server(profs);
  const { host } = await mount(profs, oleksEngines.map((e) => ({ ...e })));
  await click(host.querySelector('[data-account="sys-grok"] [data-use-first]'));
  const patches = Object.fromEntries(calls.filter(([meth]) => meth === 'PATCH').map(([, url, b]) => [url.split('/').pop(), b.priority]));
  assert.ok(patches['sys-grok'] < (patches.personal ?? 10), `Grok ranks above Personal: ${JSON.stringify(patches)}`);
  assert.deepEqual(titles(host.querySelector('[data-group="first"]')), ['Grok']);
  assert.deepEqual(titles(host.querySelector('[data-group="backup"]')), ['Claude']);
  assert.equal(document.activeElement?.getAttribute('data-focus-key'), 'sys-grok:more', 'focus stays with the row that moved');
});

test('one ready account: "No backup yet" with Turn on for an off one, or Add an account when none is off', async () => {
  const one = [profile({ id: 'personal', name: 'Personal', kind: 'cli_home', priority: 10 }), profile({ id: 'sys-codex', engine: 'codex', priority: 20, enabled: false })];
  server(one);
  const { host } = await mount(one, [engine('claude'), engine('codex')]);
  const line = host.querySelector('[data-no-backup]');
  assert.match(text(line), /^No backup yet\. If Claude runs out, your bots wait until it resets\./);
  assert.equal(text(line.querySelector('[data-no-backup-action]')), 'Turn on Codex');
  assert.equal(host.querySelector('[data-group="backup"]'), null);
  const alone = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: [one[0]], engines: [engine('claude')], onChanged: async () => {}, onAddAccount: () => {}, now: NOW }));
  assert.match(alone, /No backup yet\./);
  assert.match(alone, /data-no-backup-action="true">Add an account</);
});

test('an account that is out keeps its place and names the account bots use until then', () => {
  const profs = oleks();
  profs[0].limited_until = '2026-11-13T03:00:00';
  const html = m.renderToStaticMarkup(h(m.AccountPriority, { profiles: profs, engines: oleksEngines, onChanged: async () => {}, now: NOW }));
  const first = html.slice(html.indexOf('data-group="first"'), html.indexOf('data-group="backup"'));
  assert.match(first, /Claude/);
  assert.match(first, /Out until/);
  assert.match(first, /data-step-in="true">Bots use Grok until then\./);
});

test('the More menu holds Move up, Move down and Turn off; arrows move in it and Escape returns focus', async () => {
  server(oleks());
  const { host } = await mount(oleks(), oleksEngines.map((e) => ({ ...e })));
  const btn = host.querySelector('[data-account="sys-grok"] [data-row-menu]');
  assert.equal(btn.getAttribute('aria-haspopup'), 'menu');
  assert.equal(btn.getAttribute('aria-label'), 'More for Grok (on this computer)');
  await click(btn);
  const items = [...host.querySelectorAll('[data-row-menu-list] [role="menuitem"]')];
  assert.deepEqual(items.map(text), ['Move up', 'Move down', 'Turn off']);
  assert.ok(items[1].disabled, 'the last used row can’t go down');
  assert.equal(document.activeElement, items[0]);
  items[0].dispatchEvent(new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
  await settle();
  assert.equal(document.activeElement, items[2], 'skips the disabled item');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await settle();
  assert.equal(host.querySelector('[data-row-menu-list]'), null);
  assert.equal(document.activeElement, btn);
});
