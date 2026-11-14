// Find a tool / Another app (composio-simple/design.md "Autocomplete"; one-app-search/design.md): suggestions from
// the first letter, one row per app with its source and how you sign in, the other ways under it, the ghost and the
// keys, Connect before Composio is set up opening the key field under the row, one status line, and no plumbing words.
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
  export { FindTool, buildGroups, directoryNeeds, presetPublisher, markCutDescription } from './src/components/FindTool';
  export { mergeRows } from './src/appSearch';
  export { ConnectionsError, connectionsApi } from './src/connections';
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
const rowNames = (host) => [...host.querySelectorAll('[data-find-list] [data-row="app"] [data-row-name]')].map(text);
const rowKeys = (host) => [...host.querySelectorAll('[data-find-list] [data-row="app"]')].map((r) => r.dataset.rowKey);
const activeRow = (host) => host.querySelector('[data-active]');
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};

test('one letter already suggests, best match first, with the typed letter bold', async () => {
  const { api, calls } = stubApi();
  const host = await mount({ api });
  await type(host, 'g');
  const names = rowNames(host);
  for (const n of ['Gmail', 'Google Calendar', 'Google Sheets', 'GitHub']) assert.ok(names.includes(n), `${n} is listed`);
  assert.equal(host.querySelectorAll('[data-find-list]').length, 1, 'one list, not one per source');
  assert.equal(host.querySelector('[data-group]'), null, 'no group headers when typed');
  const first = host.querySelector('[data-find-list] [data-row-name] b');
  assert.equal(text(first).toLowerCase(), 'g');
  await settle(200);
  assert.equal(calls.filter((c) => c[0] === 'directory').length, 0, 'the public list waits for two letters');
});

test('one row per app: the chip says where it comes from, one sentence says how you sign in', async () => {
  const { api } = stubApi({ directory: async () => ({ entries: DIR, unavailable: false }) });
  const host = await mount({ api });
  await type(host, 'gm', 200);
  // "Gmail tools" from the public list is Gmail too, so it folds under Composio's Gmail.
  assert.deepEqual(rowKeys(host), ['app-gmail']);
  const row = host.querySelector('[data-row-key="app-gmail"]');
  assert.equal(text(row.querySelector('[data-source-chip]')), 'Through Composio');
  assert.equal(text(row.querySelector('[data-sign-in]')), 'Sign in with your Gmail account · free Composio key first');
  assert.equal(row.getAttribute('aria-label'), 'Gmail, through Composio, sign in with your Gmail account · free Composio key first');
  const link = row.querySelector('[data-other-ways]');
  assert.equal(text(link), 'Also as a community server for Gmail');
  assert.equal(link.getAttribute('aria-expanded'), 'false');
  assert.match(text(host.querySelector('[data-find-notes]')), /^Through Composio: sign-ins stay on Composio’s servers\.$/);
  await click(link);
  assert.equal(link.getAttribute('aria-expanded'), 'true');
  assert.ok(document.getElementById(link.getAttribute('aria-controls')), 'aria-controls names the open block');
  const other = host.querySelector('[data-row="other"]');
  assert.equal(other.getAttribute('role'), 'option', 'each other way is an option of the same list');
  assert.match(text(other), /Community\s*· example\.io.*Your own Gmail API key.*Add Gmail/);
  assert.match(text(host.querySelector('[data-find-notes]')), /Community: listed by their makers, not checked by us\./);
  assert.doesNotMatch(text(host), PLUMBING);
});

test('typos and other names find the app', async () => {
  const { api } = stubApi();
  const host = await mount({ api });
  for (const [q, name] of [['gmial', 'Gmail'], ['slak', 'Slack'], ['notoin', 'Notion'], ['sheets', 'Google Sheets']]) {
    await type(host, q);
    assert.ok(rowNames(host).includes(name), `${q} → ${name}`);
  }
});

test('nothing from any source: the empty box, with the not-listed line once Composio is added', async () => {
  const { api } = stubApi();
  let host = await mount({ api });
  await type(host, 'qwzx', 200);
  assert.ok(host.querySelector('[data-find-nothing]'));
  assert.equal(host.querySelector('[data-not-listed]'), null, 'without Composio the line would promise nothing');
  for (const u of mounted) u();
  const added = stubApi({ list: async () => listOf(composio({ added: true, can_connect: true })) });
  host = await mount({ api: added.api });
  await type(host, 'qwzx', 200);
  assert.equal(text(host.querySelector('[data-not-listed]')), 'Not listed? Bots with Composio find its other apps themselves.');
  assert.equal(host.querySelector('[data-find-status]'), null, 'every source answered: no status line');
});

test('Connect before Composio is set up opens the key field under that row, focused', async () => {
  const { api, calls } = stubApi();
  const opened = [];
  window.open = (url) => (opened.push(url), { closed: false, location: {}, close() {} });
  const host = await mount({ api });
  await type(host, 'gm');
  const row = host.querySelector('[data-row-key="app-gmail"]');
  await click(row.querySelector('[data-row-action]'));
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
  assert.equal(activeRow(host)?.dataset.rowKey, 'app-gmail');
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
  assert.match(text(host.querySelector('[data-row-key="app-gmail"]')), /Waiting for you to finish signing in/);
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

test('arrows move one highlight across rows and open other ways, with no wrap; ↑ from the first row clears it', async () => {
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
  await key(el, 'Escape'); // drop the ghost, so → opens the other ways
  await key(el, 'ArrowRight');
  assert.equal(host.querySelector('[data-other-ways]').getAttribute('aria-expanded'), 'true', '→ opens the row’s other ways');
  await key(el, 'ArrowDown');
  assert.equal(activeRow(host).dataset.rowKey, 'app-gmail~d-io.example/gmail-tools', 'down reaches the other way');
  assert.equal(el.getAttribute('aria-activedescendant'), activeRow(host).id);
  await key(el, 'ArrowDown');
  assert.equal(activeRow(host).dataset.rowKey, 'app-gmail~d-io.example/gmail-tools', 'no wrap at the end');
  await key(el, 'ArrowLeft');
  assert.equal(host.querySelector('[data-row="other"]'), null, '← closes them');
  assert.equal(activeRow(host).dataset.rowKey, 'app-gmail', 'and the highlight goes back to the row');
  await key(el, 'ArrowUp');
  assert.equal(activeRow(host), null);
  assert.equal(el.getAttribute('aria-activedescendant'), null);
  for (const lb of host.querySelectorAll('[role="listbox"]')) assert.ok(lb.getAttribute('aria-label') || document.getElementById(lb.getAttribute('aria-labelledby')));
});

test('Enter on a built-in tool adds it; an other way’s button runs that way', async () => {
  const picked = [];
  const { api } = stubApi({ directory: async () => ({ entries: DIR, unavailable: false }) });
  const host = await mount({ api, onAddPreset: (p) => picked.push(['preset', p.id]), onAddDirectory: (e) => picked.push(['dir', e.registry_name]) });
  await type(host, 'githu');
  assert.equal(activeRow(host).dataset.lead, 'b-github');
  await key(input(host), 'Enter');
  await type(host, 'gm', 200);
  await click(host.querySelector('[data-other-ways]'));
  await click(host.querySelector('[data-row="other"] [data-row-action]'));
  assert.deepEqual(picked, [['preset', 'github'], ['dir', 'io.example/gmail-tools']]);
});

test('the highlight stays on its app when the public list lands', async () => {
  let answer;
  const late = new Promise((r) => (answer = r));
  const { api } = stubApi({
    apps: async () => ({ apps: [{ slug: 'elevenlabs', name: 'ElevenLabs', tools_count: 12 }], source: 'popular' }),
    directory: () => late,
  });
  const host = await mount({ api });
  await type(host, 'el', 200);
  assert.equal(activeRow(host).dataset.rowKey, 'app-elevenlabs');
  assert.equal(host.querySelector('[data-find-status]').dataset.findStatus, 'loading');
  assert.equal(text(host.querySelector('[data-find-status]')), 'Searching more apps…');
  answer({ entries: [{ registry_name: 'co.elastic/mcp', title: 'Elastic', description: '', publisher: { label: 'elastic.co', kind: 'domain' }, kind: 'remote', url: 'https://mcp.elastic.co', inputs: [] }], unavailable: false });
  await settle();
  assert.deepEqual(rowKeys(host).slice(0, 2), ['app-elastic', 'app-elevenlabs'], 'the official remote Elastic moves in above');
  assert.equal(activeRow(host).dataset.rowKey, 'app-elevenlabs', 'Enter still runs ElevenLabs');
  assert.equal(host.querySelector('[data-find-status]'), null);
});

test('the public list hides on 404 and says so when it can’t be reached', async () => {
  const missing = stubApi();
  let host = await mount({ api: missing.api });
  await type(host, 'gm', 200);
  assert.equal(host.querySelector('[data-find-status]'), null, 'a server without the public list says nothing about it');
  assert.ok(missing.calls.some((c) => c[0] === 'directory'));
  for (const u of mounted) u();

  const down = stubApi({ directory: async () => ({ entries: [], unavailable: true }) });
  host = await mount({ api: down.api });
  await type(host, 'gm', 200);
  const line = host.querySelector('[data-find-status="down-directory"]');
  assert.ok(line);
  assert.equal(line.getAttribute('role'), 'status');
  assert.equal(text(line), 'Couldn’t reach the public list of tool servers, so only built-in tools and Composio apps are shown. Try again');
  assert.deepEqual(rowKeys(host), ['app-gmail'], 'the other rows still show');
});

test('Composio unreachable, and both unreachable: one quiet line, Try again retries', async () => {
  let fail = true;
  const { api, calls } = stubApi({
    list: async () => listOf(composio({ added: true, can_connect: true })),
    apps: async () => {
      calls.push(['apps']);
      if (fail) throw new Error('timeout');
      return { apps: APPS, source: 'all' };
    },
  });
  let host = await mount({ api });
  await type(host, 'git', 200);
  assert.equal(text(host.querySelector('[data-find-status="down-composio"]')), 'Couldn’t reach Composio, so its apps aren’t in this list. Try again');
  assert.ok(rowKeys(host).includes('app-github'), 'built-in tools still show');
  fail = false;
  await click(host.querySelector('[data-try-again]'));
  assert.equal(host.querySelector('[data-find-status]'), null);
  for (const u of mounted) u();

  fail = true;
  const both = stubApi({ list: api.list, apps: api.apps, directory: async () => ({ entries: [], unavailable: true }) });
  host = await mount({ api: both.api });
  await type(host, 'git', 200);
  assert.equal(text(host.querySelector('[data-find-status="down-both"]')), 'Couldn’t reach Composio or the public list. Built-in tools still show. Try again');
  assert.equal(host.querySelector('[data-find-nothing]'), null);
});

test('a directory entry that is a built-in tool is listed once, as built in', () => {
  const notion = m.MCP_PRESETS.find((p) => p.id === 'notion');
  const rows = m.mergeRows('notion', {
    presets: m.MCP_PRESETS,
    composio: [],
    directory: [{ registry_name: 'com.notion/mcp', title: 'Notion', description: '', publisher: { label: 'notion.com' }, kind: 'remote', url: notion.form.url + '/', inputs: [] }],
    connections: [],
  });
  assert.deepEqual(rows.map((r) => [r.key, r.lead.id, r.others.length]), [['app-notion', 'b-notion', 0]]);
});

test('a server named like a built-in tool at another address is not that tool', () => {
  // QA (mcp-finder/qa.md): a directory "Stripe" at https://api.mcp.ai/p_stripe made the built-in Stripe read "Added".
  const connections = (detail) => [{ id: 'mcp:s1', kind: 'mcp', name: 'Stripe', detail, status: 'ok', used_by: [] }];
  const stripe = (detail) =>
    m.mergeRows('stripe', { presets: m.MCP_PRESETS, composio: [], directory: null, connections: connections(detail) }).flatMap((r) => [r.lead, ...r.others]).find((s) => s.id === 'b-stripe');
  assert.equal(stripe('api.mcp.ai').added, null, 'another host is not the built-in Stripe');
  assert.equal(stripe('npx').added, null, 'a local server named Stripe is not the hosted one');
  assert.equal(stripe('mcp.stripe.com').added?.id, 'mcp:s1', 'the same host is');
  assert.equal(stripe('MCP.Stripe.com ').added?.id, 'mcp:s1', 'host case and spaces do not matter');
  assert.equal(stripe(null).added?.id, 'mcp:s1', 'no address: the name alone decides, as before');
  const connected = (detail) => m.buildGroups({ composioApps: [], connections: connections(detail) }).find((g) => g.key === 'connected');
  assert.equal(connected('api.mcp.ai'), undefined, 'not listed under Connected as the built-in Stripe');
  assert.deepEqual(connected('mcp.stripe.com').rows.map((r) => r.preset?.id), ['stripe']);
});

test('more than eight apps: one "Show n more apps" option', async () => {
  const many = Array.from({ length: 11 }, (_, i) => ({ slug: `gapp${i}`, name: `Gapp ${i}` }));
  const { api } = stubApi({ apps: async () => ({ apps: many, source: 'popular' }) });
  const host = await mount({ api });
  await type(host, 'gapp');
  assert.equal(rowKeys(host).length, 8);
  const more = host.querySelector('[data-more="apps"]');
  assert.equal(more.getAttribute('role'), 'option');
  assert.equal(text(more), 'Show 3 more apps');
  await click(more);
  assert.equal(rowKeys(host).length, 11);
  assert.equal(host.querySelector('[data-more]'), null);
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
  const row = host.querySelector('[data-row-key="app-gmail"]');
  assert.match(text(row), /Through Composio.*Connected · No bot can use it yet\..*Choose bots/);
  await key(input(host), 'Enter');
  assert.deepEqual(chosen, [{ kind: 'composio', slug: 'gmail', name: 'Gmail' }]);
  assert.match(text(host.querySelector('[data-find-live]')), /^\d+ suggestions?$/);
});

// Mira's design check FIX 1-3 (composio-simple/design-check.md, "Fixes, ranked").
const CUT = 'The official Stripe server lets bots call the Stripe API (api.stripe.com), covers the entire d';
test('a maker description cut at 100 characters ends in an ellipsis; a whole one is left alone', () => {
  const cut = CUT.padEnd(100, 'x');
  assert.equal(m.markCutDescription(cut), `${cut}\u2026`);
  assert.equal(m.markCutDescription(`${cut}  `), `${cut}\u2026`);
  for (const whole of [`${cut.slice(0, 99)}.`, `${cut.slice(0, 99)})`, `${cut.slice(0, 99)}\u2026`, 'Short and cut mid-wor']) assert.equal(m.markCutDescription(whole), whole);
});

test('a public-list row shows the cut and keeps the full text in title', async () => {
  const description = CUT.padEnd(100, 'x');
  const entries = [{ registry_name: 'io.github.zeb/zebra', title: 'Zebra payments', description, publisher: { label: 'github.com/zeb', kind: 'github' }, kind: 'remote', url: 'https://zebra.example', inputs: [] }];
  const { api } = stubApi({
    apps: async () => ({ apps: [...APPS, { slug: 'zebratool', name: 'Zebra Tool' }], source: 'popular' }),
    directory: async () => ({ entries, unavailable: false }),
  });
  const host = await mount({ api });
  await type(host, 'zebra', 200);
  const dir = host.querySelector('[data-lead="d-io.github.zeb/zebra"]');
  const p = dir.querySelector('p[title]');
  assert.equal(p.getAttribute('title'), description);
  assert.equal(p.textContent, `${description}…`);
  assert.match(text(dir), /Community\s*· github\.com\/zeb.*Signs in when you add it, if it asks/);
  const bare = host.querySelector('[data-lead="c-zebratool"]');
  assert.ok(bare, 'the bare Composio row is listed');
  assert.equal(bare.querySelector('p[title]'), null, 'no empty description line');
});

test('installing from the directory sends the Name from the form', async () => {
  const sent = [];
  const prev = globalThis.fetch;
  globalThis.fetch = async (url, init) => (sent.push([String(url), init?.method, JSON.parse(init?.body ?? 'null')]), new Response('{"server":{"id":"s1"}}', { status: 200, headers: { 'content-type': 'application/json' } }));
  try {
    await m.connectionsApi.installFromDirectory('com.stripe/mcp', 'Stripe', { A: '1' }, {});
  } finally {
    globalThis.fetch = prev;
  }
  assert.deepEqual(sent, [['/api/mcp/registry/install', 'POST', { registry_name: 'com.stripe/mcp', name: 'Stripe', env: { A: '1' }, headers: {} }]]);
});
