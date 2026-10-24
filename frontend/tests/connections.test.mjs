// Connections (docs/connections.md): every connection with status, last check, who uses it and
// where its login is stored. Composio is optional, off by default, and always says that it keeps
// sign-ins on Composio's servers.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {}, removeItem: () => {} };

const m = await loadBundled(`export * from './src/connections';`);
const ui = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ConnectionsView, ConnectionRow, ComposioCard } from './src/components/ConnectionsView';
  export const view = (props) => renderToStaticMarkup(React.createElement(ConnectionsView, props));
  export const row = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(ConnectionRow, props)));
  export const composio = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(ComposioCard, props)));
`);

const NOW = Date.parse('2026-10-22T12:00:00Z');
const WARNING = "Composio keeps your app sign-ins on Composio's servers, not in kwnt's vault, and your bots' tool calls through it pass through Composio.";
// Shaped like Connection in src/server/connections.rs.
const conn = (over = {}) => ({
  id: 'mcp:s1', kind: 'mcp', name: 'github', detail: 'api.githubcopilot.com', status: 'connected', status_detail: null,
  last_check: '2026-10-22T11:55:00Z', used_by: [{ id: 'k1', name: 'Ada' }, { id: 'k2', name: 'Lin' }], logins_stored: 'kwnt',
  manage: '/api/mcp/servers/s1', ...over,
});
const composioEntry = (over = {}) => ({
  id: 'composio', kind: 'mcp', name: 'Composio', description: 'Hundreds of apps through one MCP server you set up at composio.dev.',
  optional: true, added: false, connection_id: null, enabled: false, logins_stored: 'composio', warning: WARNING, add: '/api/connections/composio', ...over,
});
const list = (over = {}) => ({
  connections: [
    conn(),
    conn({ id: 'account:p1', kind: 'account', name: 'Claude', detail: 'claude · me@example.com', status: 'limited', status_detail: 'Out of quota until 14:00', last_check: null, logins_stored: 'kwnt', manage: '/api/profiles/p1' }),
    conn({ id: 'login:k1:c1', kind: 'browser_login', name: 'github.com', detail: 'me @ https://github.com · saved for Ada', logins_stored: 'vault', used_by: [{ id: 'k1', name: 'Ada' }] }),
    conn({ id: 'source:m1', kind: 'email', name: 'Work mail', detail: 'me @ imap.example.com', status: 'error', status_detail: 'Login refused', logins_stored: 'vault' }),
  ],
  counts: { connected: 2, limited: 1, error: 1 },
  vault: { status: 'ok', logins: 1 },
  catalog: [composioEntry()],
  ...over,
});
const noop = () => {};
const viewProps = (over = {}) => ({ data: list(), loading: false, error: null, now: NOW, onRefresh: noop, onAddComposio: async () => null, ...over });

test('connections group by kind in a fixed order, and unknown kinds still show', () => {
  const g = m.groupConnections([...list().connections, conn({ id: 'x:1', kind: 'calendar', name: 'Cal' })]);
  assert.deepEqual(g.map((x) => x.key), ['mcp', 'account', 'browser_login', 'sources', 'other']);
  assert.deepEqual(g.map((x) => x.items.length), [1, 1, 1, 1, 1]);
});

test('every status has words, not just a colour, and the header counts what needs the person', () => {
  for (const s of ['connected', 'needs_sign_in', 'limited', 'error', 'off', 'locked', 'unknown', 'new_thing']) assert.ok(m.statusBadge(s).label);
  assert.equal(m.statusBadge('weird').label, 'Not checked');
  assert.equal(m.countsLine(list().connections), '2 connected · 2 need you');
  assert.equal(m.countsLine([conn({ status: 'off' })]), '0 connected · 1 off');
  assert.equal(m.countsLine([]), 'Nothing connected yet');
});

test('used-by reads as a sentence and stays short', () => {
  const b = (n) => Array.from({ length: n }, (_, i) => ({ id: `k${i}`, name: `Bot${i}` }));
  assert.equal(m.usedByText(b(1), 'connected'), 'Bot0');
  assert.equal(m.usedByText(b(3), 'connected'), 'Bot0, Bot1 and Bot2');
  assert.equal(m.usedByText(b(6), 'connected'), 'Bot0, Bot1 and 4 more');
  assert.equal(m.usedByText([], 'off'), 'No bot while it’s off');
  assert.equal(m.usedByText([], 'connected'), 'No bot uses it');
});

test('only https composio.dev addresses are accepted, and the key is optional', () => {
  assert.equal(m.isComposioUrl('https://backend.composio.dev/v3/mcp/abc/mcp?user_id=1'), true);
  assert.equal(m.isComposioUrl('https://composio.dev/x'), true);
  assert.equal(m.isComposioUrl('http://backend.composio.dev/x'), false);
  assert.equal(m.isComposioUrl('https://composio.dev.evil.example/x'), false);
  assert.equal(m.isComposioUrl('https://evilcomposio.dev/x'), false);
  assert.ok('error' in m.composioBody('', '', false));
  assert.ok('error' in m.composioBody('https://example.com/mcp', 'k', false));
  assert.deepEqual(m.composioBody(' https://backend.composio.dev/m ', '', false), { url: 'https://backend.composio.dev/m', enabled: false });
  assert.deepEqual(m.composioBody('https://backend.composio.dev/m', ' key ', true), { url: 'https://backend.composio.dev/m', api_key: 'key', enabled: true });
  assert.match(m.composioErrorText(new m.ConnectionsError('x', 409)), /already added/);
});

test('a row shows name, detail, status words, why, used-by, last check and where the login lives', () => {
  const html = ui.row({ c: conn({ status: 'error', status_detail: 'Connection refused' }), now: NOW, onCheck: noop, links: { onOpenTools: noop } });
  for (const t of ['github', 'api.githubcopilot.com', 'Error', 'Connection refused', 'Ada and Lin', 'Checked 5 min ago', 'Keys and settings kept on this computer', 'Check now', 'Manage']) {
    assert.ok(html.includes(t), `row shows "${t}"`);
  }
  assert.ok(ui.row({ c: conn({ last_check: null }), now: NOW }).includes('Never checked'));
  assert.ok(ui.row({ c: conn({ id: 'account:p1', kind: 'account' }), now: NOW }).includes('Login kept on this computer'), 'an account keeps a login, a tool server keeps keys');
  assert.ok(!ui.row({ c: conn({ status: 'off' }), now: NOW, onCheck: noop }).includes('Check now'), 'no check for a server that is off');
  assert.ok(!ui.row({ c: conn({ id: 'account:p1', kind: 'account' }), now: NOW, onCheck: noop }).includes('Check now'), 'only MCP servers can be checked');
});

test('a Composio connection is marked as storing its login on Composio’s servers', () => {
  const html = ui.row({ c: conn({ name: 'composio', logins_stored: 'composio' }), now: NOW });
  assert.ok(html.includes('Login on Composio’s servers'));
  assert.ok(html.includes('text-amber-200'));
});

test('the page groups rows, counts them, and offers Composio as optional with its warning', () => {
  const html = ui.view(viewProps({ onOpenTools: noop, onOpenAccounts: noop, onOpenLooseEnds: noop }));
  for (const t of ['Connections', '2 connected · 2 need you', 'Tool servers (MCP)', 'Engine accounts', 'Saved website logins', 'Mail and GitHub (Loose ends)', 'Out of quota until 14:00', 'Open Loose ends', 'Optional connectors', 'Composio', 'Optional', 'Add Composio']) {
    assert.ok(html.includes(t), `page shows "${t}"`);
  }
  assert.ok(html.includes('Composio keeps your app sign-ins on Composio&#x27;s servers'), 'the warning is next to the add button');
});

test('the add form is off by default and repeats nothing secret', () => {
  const html = ui.composio({ entry: composioEntry(), initialOpen: true, onAdd: async () => null });
  assert.match(html, /type="url"/);
  assert.match(html, /type="password"/);
  assert.match(html, /<input type="checkbox"[^>]*>/);
  assert.doesNotMatch(html, /type="checkbox"[^>]*checked/, 'the switch starts off');
  assert.ok(html.includes('Off by default'));
  assert.ok(html.includes('Composio keeps your app sign-ins'));
  assert.ok(html.includes('MCP URL') && html.includes('API key'));
});

test('once added, Composio says whether it is on and points at Tools instead of a second add', () => {
  const off = ui.composio({ entry: composioEntry({ added: true, connection_id: 'mcp:c1' }), onAdd: async () => null, onOpenTools: noop });
  assert.ok(off.includes('Added · off') && off.includes('Turn on in Tools'));
  assert.ok(!off.includes('Add Composio'));
  assert.ok(off.includes('Composio keeps your app sign-ins'), 'the warning stays once added');
  const on = ui.composio({ entry: composioEntry({ added: true, enabled: true }), onAdd: async () => null, onOpenTools: noop });
  assert.ok(on.includes('Added · on') && on.includes('Manage in Tools'));
});

test('a locked vault hides saved logins and offers to unlock; loading, error and token states show', () => {
  const locked = ui.view(viewProps({ data: list({ connections: [conn()], vault: { status: 'locked' } }), onUnlockVault: noop }));
  assert.ok(locked.includes('Unlock Bitwarden to list saved logins.'));
  assert.ok(locked.includes('Unlock Bitwarden</button>'));
  assert.ok(locked.includes('Hidden while Bitwarden is locked.'));
  const unavailable = ui.view(viewProps({ data: list({ vault: { status: 'unavailable' } }) }));
  assert.ok(unavailable.includes('can’t be read right now'));
  assert.ok(ui.view(viewProps({ data: null, loading: true })).includes('Loading…'));
  const err = ui.view(viewProps({ data: null, error: 'Something went wrong (HTTP 500).' }));
  assert.ok(err.includes('Couldn’t load connections') && err.includes('Retry'));
  assert.ok(ui.view(viewProps({ data: null, needsToken: true })).includes('Confirm it’s you'));
});

test('empty groups say so instead of leaving a gap', () => {
  const html = ui.view(viewProps({ data: list({ connections: [], catalog: [] }) }));
  assert.ok(html.includes('Nothing connected yet'));
  for (const t of ['No tool servers yet.', 'No engine accounts yet.', 'No saved website logins.', 'No mail or GitHub sources.']) assert.ok(html.includes(t));
  assert.ok(!html.includes('Optional connectors'));
});

const apps = [
  { slug: 'gmail', name: 'Gmail', description: 'Read and send mail.', logo: null, tools_count: 23, no_auth: false },
  { slug: 'hackernews', name: 'Hacker News', description: '', logo: null, tools_count: 4, no_auth: true },
];

test('"Find an app" shows only once Composio is added, and the warning stays', () => {
  const notAdded = ui.composio({ entry: composioEntry({ can_search: true }), onAdd: async () => null });
  assert.ok(!notAdded.includes('data-composio-search'), 'no search before Composio is added');
  const noKey = ui.composio({ entry: composioEntry({ added: true, can_search: false }), onAdd: async () => null });
  assert.ok(noKey.includes('data-composio-search="needs-key"') && noKey.includes('x-api-key'));
  const ready = ui.composio({ entry: composioEntry({ added: true, can_search: true, can_connect: true }), onAdd: async () => null });
  assert.ok(ready.includes('Find an app') && ready.includes('role="search"'));
  for (const html of [noKey, ready]) assert.ok(html.includes('Composio keeps your app sign-ins'), 'the warning stays');
});

test('found apps offer Connect when the Composio URL names a user, else point to composio.dev', () => {
  const entry = composioEntry({ added: true, can_search: true, can_connect: true });
  const html = ui.composio({ entry, onAdd: async () => null, initialApps: apps });
  assert.ok(html.includes('data-composio-app="gmail"') && html.includes('23 tools'));
  assert.equal((html.match(/>Connect</g) ?? []).length, 1, 'only apps that need a sign-in get Connect');
  assert.ok(html.includes('No sign-in needed'));
  const noUser = ui.composio({ entry: { ...entry, can_connect: false }, onAdd: async () => null, initialApps: apps });
  assert.ok(!noUser.includes('>Connect<') && noUser.includes('connect apps on composio.dev'));
});

test('the search and connect calls go to kwnt, never straight to Composio', async () => {
  const prev = globalThis.fetch;
  const seen = [];
  globalThis.fetch = async (url, init = {}) => {
    seen.push([String(url), init.method]);
    return new Response(JSON.stringify(String(url).endsWith('/connect') ? { redirect_url: 'https://connect.composio.dev/x', warning: WARNING } : { apps, can_connect: true, warning: WARNING }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  };
  try {
    assert.equal((await m.connectionsApi.searchComposioApps('g mail&x')).apps.length, 2);
    assert.equal((await m.connectionsApi.connectComposioApp('gmail')).redirect_url, 'https://connect.composio.dev/x');
  } finally {
    globalThis.fetch = prev;
  }
  assert.deepEqual(seen, [
    ['/api/connections/composio/apps?q=g%20mail%26x', 'GET'],
    ['/api/connections/composio/apps/gmail/connect', 'POST'],
  ]);
});

test('the server catalog lists Gmail as "via Composio" and explains why, with the warning', async () => {
  const cat = await loadBundled(`
    import React from 'react';
    import { renderToStaticMarkup } from 'react-dom/server.browser';
    import { PresetCatalog } from './src/components/McpServersModal';
    export const render = (props) => renderToStaticMarkup(React.createElement(PresetCatalog, props));
  `);
  const base = { query: 'gmail', onQuery: noop, onPick: noop, onCustom: noop, onOpenConnections: noop };
  const list = cat.render(base);
  assert.ok(list.includes('Gmail') && list.includes('via Composio'));
  const open = cat.render({ ...base, initialComposioApp: 'gmail' });
  assert.ok(open.includes('data-composio-note="gmail"') && open.includes('Open Connections'));
  assert.ok(open.includes('not in this app’s vault'));
});
