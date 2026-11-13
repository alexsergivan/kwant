// Find a tool (composio-simple/design.md §1, §2, "Autocomplete"): suggestions from the first letter,
// three groups, the ghost and the keys, Connect before Composio is set up opening the key field under
// the row, the not-listed line, the public directory hiding on 404, and no plumbing words.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
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

const m = await loadBundled(`
  export { FindTool, buildGroups, directoryNeeds, presetPublisher } from './src/components/FindTool';
  export { ConnectionsError } from './src/connections';
  export { MCP_PRESETS } from './src/mcp';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const h = m.createElement;

const PLUMBING = /\b(MCP|header|user_id|toolkit|server URL|connector)\b/i;

const composio = (over = {}) => ({
  id: 'composio', kind: 'mcp', name: 'Composio', description: '', optional: true, added: false, connection_id: null, enabled: false,
  logins_stored: 'composio', warning: '', add: '/api/connections/composio', ...over,
});
const listOf = (entry = composio(), connections = []) => ({ catalog: [entry], connections });
const APPS = [
  { slug: 'gmail', name: 'Gmail', tools_count: 61 },
  { slug: 'googlecalendar', name: 'Google Calendar', tools_count: 46 },
  { slug: 'googlesheets', name: 'Google Sheets', tools_count: 40 },
  { slug: 'slack', name: 'Slack', tools_count: 130 },
  { slug: 'notion', name: 'Notion', tools_count: 44 },
];
const DIR = [
  { registry_name: 'io.example/gmail-tools', title: 'Gmail tools', description: 'Mail from a maker.', publisher: { label: 'example.io' }, kind: 'local', inputs: [{ name: 'API_KEY', secret: true }] },
];

/** A stub api that records calls. `over` replaces any of its functions. */
const stubApi = (over = {}) => {
  const calls = [];
  const api = {
    list: async () => (calls.push(['list']), listOf()),
    apps: async () => (calls.push(['apps']), { apps: APPS, source: 'popular' }),
    addKey: async (k) => (calls.push(['addKey', k]), {}),
    replaceKey: async (k) => (calls.push(['replaceKey', k]), {}),
    connect: async (slug) => (calls.push(['connect', slug]), { redirect_url: `https://connect.example/${slug}` }),
    directory: async (q) => (calls.push(['directory', q]), null),
    ...over,
  };
  return { api, calls };
};

// ── DOM helpers ──
const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async (ms = 0) => {
  if (ms) await new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(m.FindTool, { browseWhenEmpty: false, onAddPreset() {}, onChooseBots() {}, ...props }));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  return host;
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const input = (host) => host.querySelector('[data-find-input]');
const type = async (host, value, ms = 0) => {
  const el = input(host);
  el.focus();
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.setSelectionRange?.(value.length, value.length);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle(ms);
};
const key = async (el, k, opts = {}) => {
  const ev = new window.KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...opts });
  el.dispatchEvent(ev);
  await settle();
  return ev;
};
const rowNames = (host, group) => [...host.querySelectorAll(`[data-group="${group}"] [data-row-name]`)].map(text);
const activeRow = (host) => host.querySelector('[data-active]');

test('one letter already suggests, best match first, with the typed letter bold', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  await type(host, 'g');
  assert.deepEqual(rowNames(host, 'composio').slice(0, 3), ['Gmail', 'Google Calendar', 'Google Sheets']);
  assert.ok(host.querySelector('[data-group="builtin"]'), 'built-in tools starting with g (GitHub, Git) are listed too');
  const first = host.querySelector('[data-group="builtin"] [data-row-name] b');
  assert.equal(text(first).toLowerCase(), 'g');
  assert.equal(host.querySelector('[data-group="directory"]'), null, 'the directory waits for two letters');
});

test('the three groups come in order and say where the tool comes from', async () => {
  const { api } = stubApi({ directory: async () => ({ entries: DIR, unavailable: false }) });
  const host = await mount({ api });
  await type(host, 'gm', 200);
  const groups = [...host.querySelectorAll('[data-group]')].map((s) => s.dataset.group);
  assert.deepEqual(groups, ['composio', 'directory']);
  assert.match(text(host.querySelector('[data-group="composio"]')), /Through Composio.*Sign-ins stay on Composio’s servers/);
  assert.match(text(host.querySelector('[data-group="directory"]')), /From the public directory.*Listed by their makers, not checked by us/);
  assert.match(text(host.querySelector('[data-group="directory"] [data-row]')), /by example\.io.*Runs on this computer.*API key/);
  assert.doesNotMatch(text(host), PLUMBING);
});

test('typos and other names find the app', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  for (const [q, name] of [['gmial', 'Gmail'], ['slak', 'Slack'], ['notoin', 'Notion'], ['sheets', 'Google Sheets']]) {
    await type(host, q);
    assert.ok(rowNames(host, 'composio').includes(name), `${q} → ${name}`);
  }
});

test('"eleven" is not in the list: the not-listed line says bots find other apps themselves', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  await type(host, 'eleven');
  const line = host.querySelector('[data-not-listed]');
  assert.ok(line, 'the not-listed line shows');
  assert.equal(text(line), 'Not listed? Bots with Composio find its other apps themselves.');
  assert.equal(host.querySelectorAll('[data-group="composio"] [data-row]').length, 0);
  assert.equal(host.querySelector('[data-find-nothing]'), null, 'not "nothing found": the line is the answer');
});

test('Connect before Composio is set up opens the key field under that row, focused', async () => {
  const { api, calls } = stubApi();
  const opened = [];
  window.open = (url) => (opened.push(url), { closed: false, location: {}, close() {} });
  const host = await mount({ api });
  await type(host, 'gm');
  const row = host.querySelector('[data-row-key="composio-c-gmail"]');
  row.querySelector('[data-row-action]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  const setup = host.querySelector('[data-composio-setup="gmail"]');
  assert.ok(setup, 'the key field opens under Gmail');
  assert.ok(row.contains(setup), 'inside the row, not somewhere else');
  assert.equal(row.querySelector('[data-row-action]'), null, 'the row’s Connect button gives way to the field');
  const field = setup.querySelector('input[type="password"]');
  assert.equal(document.activeElement, field, 'the key field has focus');
  assert.equal(text(setup.querySelector(`label[for="${field.id}"]`)), 'Your Composio API key');
  assert.equal(text(setup.querySelector('[data-key-submit]')), 'Connect Gmail');
  assert.match(text(setup), /Free plan: 10,000 tool calls a month/);
  assert.doesNotMatch(text(setup), PLUMBING);
  assert.equal(opened.length, 0, 'nothing opens before the key is in');
  assert.equal(calls.filter((c) => c[0] === 'connect').length, 0);
});

test('Enter on Gmail before setup opens the same field; Esc in it closes it and goes back to the search', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  await type(host, 'gm');
  assert.equal(activeRow(host)?.dataset.rowKey, 'composio-c-gmail');
  await key(input(host), 'Enter');
  const field = host.querySelector('[data-composio-setup="gmail"] input');
  assert.equal(document.activeElement, field);
  await key(field, 'Escape');
  assert.equal(host.querySelector('[data-composio-setup]'), null);
  assert.equal(document.activeElement, input(host));
});

test('a good key adds Composio, then the sign-in opens in the tab opened by the press', async () => {
  let added = false;
  const { api, calls } = stubApi({ list: async () => listOf(composio({ added, can_connect: added })) });
  api.addKey = async (k) => ((added = true), calls.push(['addKey', k]), {});
  const tab = { closed: false, location: {}, close() {} };
  const opened = [];
  window.open = (url) => (opened.push(url), tab);
  const host = await mount({ api });
  await type(host, 'gm');
  await key(input(host), 'Enter');
  const field = host.querySelector('[data-composio-setup] input');
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(field, 'ck_test');
  field.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  host.querySelector('[data-key-submit]').closest('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  assert.deepEqual(calls.find((c) => c[0] === 'addKey'), ['addKey', 'ck_test']);
  assert.deepEqual(opened, ['about:blank'], 'the tab opens synchronously in the press');
  assert.equal(tab.location.href, 'https://connect.example/gmail');
  assert.match(text(host.querySelector('[data-find-notice]')), /Composio is added\. Finish signing in to Gmail in the new tab\./);
  assert.match(text(host.querySelector('[data-row-key="composio-c-gmail"]')), /Waiting for you to finish signing in/);
});

test('a refused key stays in the field with the error and its fix next to it', async () => {
  const { api } = stubApi({ addKey: async () => { throw new m.ConnectionsError('Composio didn’t accept this key.', 409, 'replace_key'); } });
  window.open = () => ({ closed: false, location: {}, close() {} });
  const host = await mount({ api });
  await type(host, 'gm');
  await key(input(host), 'Enter');
  const field = host.querySelector('[data-composio-setup] input');
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(field, 'nope');
  field.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
  field.closest('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
  const err = host.querySelector('[data-key-error="refused"]');
  assert.ok(err);
  assert.equal(err.getAttribute('role'), 'alert');
  assert.match(text(err), /Composio didn’t accept this key\. Open your API keys/);
  assert.equal(host.querySelector('[data-composio-setup] input').getAttribute('aria-invalid'), 'true');
  assert.ok(host.querySelector('[data-composio-setup] [data-key-error]'), 'the error sits in the field, under the row');
});

test('the ghost fills in the rest of the name; Tab and → accept it, Esc drops it, then clears', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  await type(host, 'gm');
  const ghost = host.querySelector('[data-ghost]');
  assert.ok(ghost);
  assert.equal(ghost.getAttribute('aria-hidden'), 'true');
  assert.equal(text(ghost.lastElementChild), 'ail');
  assert.ok(host.querySelector('[data-tab-keycap]'));
  const tab = await key(input(host), 'Tab');
  assert.equal(tab.defaultPrevented, true);
  assert.equal(input(host).value, 'Gmail');
  await type(host, 'gm');
  await key(input(host), 'Escape');
  assert.equal(host.querySelector('[data-ghost]'), null, 'first Esc drops the ghost');
  assert.equal(input(host).value, 'gm');
  await key(input(host), 'Escape');
  assert.equal(input(host).value, '', 'second Esc clears the field');
  const third = await key(input(host), 'Escape');
  assert.equal(third.defaultPrevented, false, 'with nothing to clear, Esc goes on to the dialog');
});

test('no ghost for a word or typo match', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  await type(host, 'sheets');
  assert.equal(host.querySelector('[data-ghost]'), null);
  await type(host, 'gmial');
  assert.equal(host.querySelector('[data-ghost]'), null);
});

test('arrows move one highlight across groups with no wrap; ↑ from the first row clears it', async () => {
  const { api } = stubApi({ directory: async () => ({ entries: DIR, unavailable: false }) });
  const host = await mount({ api });
  await type(host, 'gm', 200);
  const el = input(host);
  assert.equal(el.getAttribute('role'), 'combobox');
  assert.equal(el.getAttribute('aria-autocomplete'), 'both');
  assert.equal(el.getAttribute('aria-expanded'), 'true');
  const first = activeRow(host);
  assert.equal(el.getAttribute('aria-activedescendant'), first.id);
  assert.equal(first.getAttribute('aria-selected'), 'true');
  assert.ok(first.querySelector('[data-enter-keycap]'));
  await key(el, 'ArrowDown');
  assert.equal(activeRow(host).dataset.rowKey, 'directory-io.example/gmail-tools', 'down crosses into the next group');
  await key(el, 'ArrowDown');
  assert.equal(activeRow(host).dataset.rowKey, 'directory-io.example/gmail-tools', 'no wrap at the end');
  await key(el, 'ArrowUp');
  await key(el, 'ArrowUp');
  assert.equal(activeRow(host), null);
  assert.equal(el.getAttribute('aria-activedescendant'), null);
  for (const lb of host.querySelectorAll('[role="listbox"]')) assert.ok(document.getElementById(lb.getAttribute('aria-labelledby')));
});

test('Enter on a built-in tool adds it; on a directory row opens its form', async () => {
  const picked = [];
  const { api } = stubApi({ directory: async () => ({ entries: DIR, unavailable: false }) });
  const host = await mount({ api, onAddPreset: (p) => picked.push(['preset', p.id]), onAddDirectory: (e) => picked.push(['dir', e.registry_name]) });
  await type(host, 'githu');
  assert.equal(activeRow(host).dataset.row, 'builtin');
  await key(input(host), 'Enter');
  await type(host, 'gmail tools', 200);
  const dirRow = host.querySelector('[data-row-key="directory-io.example/gmail-tools"]');
  dirRow.querySelector('[data-row-action]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
  assert.deepEqual(picked, [['preset', 'github'], ['dir', 'io.example/gmail-tools']]);
});

test('the public directory hides on 404 and says so when it can’t be reached', async () => {
  const missing = stubApi();
  let host = await mount({ api: missing.api });
  await type(host, 'gm', 200);
  assert.equal(host.querySelector('[data-group="directory"]'), null, 'a server without the directory shows no group');
  assert.ok(missing.calls.some((c) => c[0] === 'directory'));
  for (const u of mounted) u();

  const down = stubApi({ directory: async () => ({ entries: [], unavailable: true }) });
  host = await mount({ api: down.api });
  await type(host, 'gm', 200);
  const line = host.querySelector('[data-directory-state="down"]');
  assert.ok(line);
  assert.match(text(line), /Couldn’t reach the public directory\. The tools above still work\. Try again/);
});

test('a directory entry that is a built-in tool is listed once, as built in', () => {
  const notion = m.MCP_PRESETS.find((p) => p.id === 'notion');
  const groups = m.buildGroups({
    query: 'notion',
    composioApps: [],
    composioAdded: false,
    directory: [{ registry_name: 'com.notion/mcp', title: 'Notion', description: '', publisher: { label: 'notion.com' }, kind: 'remote', url: notion.form.url + '/', inputs: [] }],
    connections: [],
    expanded: new Set(),
  });
  assert.deepEqual(groups.map((g) => [g.key, g.rows.length]), [['builtin', 1], ['composio', 0], ['directory', 0]]);
});

test('more than five in a group: a Show more row that is itself an option', () => {
  const composioApps = Array.from({ length: 8 }, (_, i) => ({ slug: `g${i}`, name: `Gapp ${i}`, description: '', tools: null, connected: false }));
  // No built-in tool matches "gapp", so Built in is left out and the group is found by its key.
  const c = m.buildGroups({ query: 'gapp', composioApps, composioAdded: true, directory: null, connections: [], expanded: new Set() }).find((g) => g.key === 'composio');
  assert.ok(c, 'a Through Composio group');
  assert.equal(c.rows.length, 6);
  assert.equal(c.rows[5].name, 'Show 3 more through Composio');
  const all = m.buildGroups({ query: 'gapp', composioApps, composioAdded: true, directory: null, connections: [], expanded: new Set(['composio']) }).find((g) => g.key === 'composio');
  assert.equal(all.rows.length, 8);
});

test('nothing anywhere: the empty state offers Add by address', async () => {
  const { api } = stubApi();
  let asked = 0;
  const host = await mount({ api, onAddByAddress: () => asked++ });
  await type(host, 'q');
  const none = host.querySelector('[data-find-nothing]');
  assert.ok(none);
  assert.match(text(none), /Nothing found for “q”\./);
  none.querySelector('[data-add-by-address-inline]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  assert.equal(asked, 1);
});

test('empty field: Add a tool shows Popular and the browse grid; Apps & accounts shows nothing until focus', async () => {
  const { api } = stubApi();
  const browse = await mount({ api, browseWhenEmpty: true, children: h('div', { 'data-grid': '' }, 'grid') });
  assert.ok(browse.querySelector('[data-group="popular"]'));
  assert.ok(browse.querySelector('[data-browse] [data-grid]'));
  assert.match(text(browse.querySelector('[data-browse]')), /All built-in tools/);
  for (const u of mounted) u();
  const page = await mount({ api });
  assert.equal(page.querySelector('[data-group]'), null);
  assert.equal(input(page).getAttribute('aria-expanded'), 'false');
  assert.equal(input(page).getAttribute('placeholder'), 'Find a tool, like gmail, stripe or a database');
});

test('a connected app offers Choose bots, and the live line counts suggestions', async () => {
  const chosen = [];
  const { api } = stubApi({
    list: async () => listOf(composio({ added: true, can_connect: true, connection_id: 'c1', apps_connected: [{ slug: 'gmail', name: 'Gmail' }] }), [{ id: 'c1', kind: 'mcp', name: 'Composio', used_by: [] }]),
  });
  const host = await mount({ api, onChooseBots: (t) => chosen.push(t) });
  await type(host, 'gmail', 450);
  const row = host.querySelector('[data-row-key="composio-c-gmail"]');
  assert.match(text(row), /Connected.*Choose bots/);
  assert.match(text(row), /No bot can use it yet\./);
  await key(input(host), 'Enter');
  assert.deepEqual(chosen, [{ kind: 'composio', slug: 'gmail', name: 'Gmail' }]);
  assert.match(text(host.querySelector('[data-find-live]')), /^\d+ suggestions?$/);
});
