// Settings › Apps & accounts (0.9.5, autonomy/apps-accounts/brief.md): app tiles first, a 3-step
// Connect sheet where picking bots turns the app on, AI accounts named by engine with who uses
// them (an unused account never "needs you"), Advanced closed, and old addresses landing here.
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
  export * from './src/appsAccounts';
  export { countsLine } from './src/connections';
  export { parseView, viewSegment, SPACE_SETTINGS_TABS } from './src/nav';
  export { AppsAccounts, ConnectSheet } from './src/components/AppsAccounts';
  export { ConnectionsView } from './src/components/ConnectionsView';
  export { EnginesPanel } from './src/components/EnginesPanel';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export { createRoot } from 'react-dom/client';
  export { createElement } from 'react';
`);
const h = m.createElement;

const WARNING = "Composio keeps your app sign-ins on Composio's servers.";
const composio = (over = {}) => ({
  id: 'composio', kind: 'mcp', name: 'Composio', description: 'Hundreds of apps.', optional: true, added: false, connection_id: null,
  enabled: false, logins_stored: 'composio', warning: WARNING, add: '/api/connections/composio', ...over,
});
const acct = (id, over = {}) => ({
  id: `account:${id}`, kind: 'account', name: 'This machine', detail: 'claude · me@example.com', status: 'connected', status_detail: null,
  last_check: null, used_by: [], logins_stored: 'kwnt', manage: `/api/profiles/${id}`, ...over,
});
const profile = (id, engine, over = {}) => ({
  id, engine, name: id, kind: 'system', enabled: true, is_default: false, priority: 0, home_dir: null, has_key: false, signed_in: true,
  account_label: null, limited_until: null, last_error: null, ...over,
});
const bots = [
  { id: 'k1', name: 'Ann', approvals_effective: 'ask' },
  { id: 'k2', name: 'Ben', approvals_effective: 'ask' },
  { id: 'k3', name: 'Cody', approvals_effective: 'off' },
];

// ── DOM helpers ──────────────────────────────────────────────────────────────────────────────
const mounted = new Set();
afterEach(() => {
  for (const u of mounted) u();
  document.body.replaceChildren();
});
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
};
const mount = async (C, props) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(h(C, props));
  await settle();
  const u = () => {
    if (!mounted.delete(u)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(u);
  // The sheet renders into <body>, so look there.
  return document.body;
};
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const button = (host, t) => [...host.querySelectorAll('button')].find((b) => text(b).startsWith(t));
const click = async (el) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle();
};
const type = async (el, v) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, v);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
  await settle();
};
const submit = async (host) => {
  host.querySelector('form').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle();
};

const fakeApi = (over = {}) => {
  const calls = [];
  return {
    calls,
    addComposio: async (b) => (calls.push(['add', b]), { connection_id: 'mcp:cmp1' }),
    connect: async (slug) => (calls.push(['connect', slug]), { redirect_url: 'https://composio.dev/auth/x' }),
    give: async (s, all, chosen) => void calls.push(['give', s, all, chosen]),
    botsWith: async () => ['k2'],
    ...over,
  };
};
const sheetProps = (over = {}) => ({
  app: m.APP_TILES[0], composio: composio(), kwants: bots, spaceName: 'Home', spaceMode: 'ask', onClose: () => {}, onDone: () => {}, phone: false, ...over,
});

// ── The sheet's three steps ──────────────────────────────────────────────────────────────────

test('the sheet says what bots can do and that it goes through Composio, then shows three steps with no bot ticked', () => {
  const html = m.renderToStaticMarkup(h(m.ConnectSheet, sheetProps()));
  for (const t of ['Connect Gmail', 'Your bots will be able to', 'Read and search your mail', 'Through Composio, a service with a free plan', 'Open Composio', 'Paste your Composio link', 'Which bots get it?', 'None until you tick one.']) {
    assert.ok(html.includes(t), `sheet shows "${t}"`);
  }
  assert.equal((html.match(/data-sheet-step="/g) ?? []).length, 3);
  assert.ok(!/type="checkbox"[^>]*checked/.test(html), 'no bot is preselected');
  assert.match(html, /role="dialog"/);
  assert.match(html, /aria-modal="true"/);
  assert.ok(html.includes('MCP</span> in the left menu'), 'step 2 says where the link is');
  assert.ok(!html.includes('Turn on in Tools'));
});

test('a bad link stops at step 2 with words, and no bot stops at step 3', async () => {
  const api = fakeApi();
  const host = await mount(m.ConnectSheet, sheetProps({ api }));
  await submit(host);
  assert.match(text(host.querySelector('[data-sheet-error="2"]')), /^Paste your Composio link first\. It’s under MCP in Composio\.$/);
  await type(host.querySelector('input[type="url"]'), 'https://example.com/mcp');
  await submit(host);
  assert.match(text(host.querySelector('[data-sheet-error="2"]')), /^That isn’t a Composio link\. Copy the one under MCP in Composio; it starts with https:\/\/ and is on composio\.dev\.$/);
  await type(host.querySelector('input[type="url"]'), 'https://backend.composio.dev/v3/mcp/abc');
  await submit(host);
  assert.match(text(host.querySelector('[data-sheet-error="3"]')), /Tick at least one bot/);
  assert.deepEqual(api.calls, [], 'nothing is sent until the form is whole');
});

test('fixing the link or ticking a bot clears that step’s error at once', async () => {
  const host = await mount(m.ConnectSheet, sheetProps({ api: fakeApi() }));
  await type(host.querySelector('input[type="url"]'), 'https://example.com/mcp');
  await submit(host);
  assert.ok(host.querySelector('[data-sheet-error="2"]'));
  await type(host.querySelector('input[type="url"]'), 'https://backend.composio.dev/v3/mcp/abc');
  assert.equal(host.querySelector('[data-sheet-error="2"]'), null, 'the old link error goes as soon as the link changes');
  await submit(host);
  assert.ok(host.querySelector('[data-sheet-error="3"]'));
  await click(host.querySelector('[data-sheet-bot] input'));
  assert.equal(host.querySelector('[data-sheet-error="3"]'), null, 'ticking a bot clears the bot error');
});

test('Composio refusing the link shows its reason at step 2', async () => {
  const { ConnectionsError } = await loadBundled(`export { ConnectionsError } from './src/connections';`);
  const api = fakeApi({ addComposio: async () => { throw new ConnectionsError('Composio didn’t answer.', 400); } });
  const host = await mount(m.ConnectSheet, sheetProps({ api, initial: { url: 'https://backend.composio.dev/v3/mcp/abc', picked: ['k1'] } }));
  await submit(host);
  assert.match(text(host.querySelector('[data-sheet-error="2"]')), /Composio didn’t answer/);
});

test('picking one bot adds Composio and gives it to just that bot, then says so', async () => {
  const api = fakeApi();
  let done = null;
  const host = await mount(m.ConnectSheet, sheetProps({ api, onDone: (r) => (done = r) }));
  await type(host.querySelector('input[type="url"]'), 'https://backend.composio.dev/v3/mcp/abc');
  await click(host.querySelector('[data-sheet-bot="k1"] input'));
  assert.match(text(host), /Ann gets it, turned on/);
  await submit(host);
  assert.deepEqual(api.calls[0], ['add', { url: 'https://backend.composio.dev/v3/mcp/abc', enabled: false }], 'added off: only the picks turn it on');
  assert.deepEqual(api.calls[1], ['give', 'cmp1', ['k1', 'k2', 'k3'], ['k1']]);
  assert.deepEqual({ app: done.app.slug, who: done.who }, { app: 'gmail', who: ['Ann'] }, 'the page words the result once it knows the sign-in');
});

test('with Composio set up the sheet starts at the app’s own sign-in and the bots that have it now', async () => {
  const api = fakeApi();
  const host = await mount(m.ConnectSheet, sheetProps({ api, composio: composio({ added: true, enabled: true, connection_id: 'mcp:cmp1', can_connect: true }) }));
  assert.ok(!host.querySelector('input[type="url"]'), 'no link to paste again');
  assert.equal(host.querySelectorAll('[data-sheet-step]').length, 2);
  assert.ok(host.querySelector('[data-sheet-bot="k2"] input').checked, 'Ben has it already');
  assert.ok(!host.querySelector('[data-sheet-bot="k1"] input').checked);
  g.window.open = () => null;
  await click(host.querySelector('[data-sheet-signin]'));
  assert.deepEqual(api.calls[0], ['connect', 'gmail']);
  await submit(host);
  assert.deepEqual(api.calls[1], ['give', 'cmp1', ['k1', 'k2', 'k3'], ['k2']]);
});

test('Composio down while starting the app sign-in says so at step 1', async () => {
  const api = fakeApi({ connect: async () => { throw new Error("Composio couldn't be reached; try again later."); } });
  const host = await mount(m.ConnectSheet, sheetProps({ api, composio: composio({ added: true, enabled: true, connection_id: 'mcp:cmp1', can_connect: true }) }));
  g.window.open = () => null;
  await click(host.querySelector('[data-sheet-signin]'));
  const t = text(host.querySelector('[data-sheet-error="1"]'));
  assert.match(t, /Composio couldn’t be reached, so the Gmail sign-in didn’t open\. Try again in a minute\./);
  assert.equal(t.match(/try again/gi).length, 1, 'the server\'s own "try again later" isn\'t repeated');
});

test('a refused Composio key says so and doesn’t blame an outage', async () => {
  const { ConnectionsError } = await loadBundled(`export { ConnectionsError } from './src/connections';`);
  const api = fakeApi({ connect: async () => { throw new ConnectionsError('Composio refused the API key saved with the Composio connector.', 409); } });
  const host = await mount(m.ConnectSheet, sheetProps({ api, composio: composio({ added: true, enabled: true, connection_id: 'mcp:cmp1', can_connect: true }) }));
  g.window.open = () => null;
  await click(host.querySelector('[data-sheet-signin]'));
  const t = text(host.querySelector('[data-sheet-error="1"]'));
  assert.match(t, /refused your API key.*Advanced › Manage tools/);
  assert.doesNotMatch(t, /may be down/);
});

test('focus moves into the sheet (a modal: Tab is trapped by useEscapeLayer) and Escape closes it', async () => {
  let closed = 0;
  const host = await mount(m.ConnectSheet, sheetProps({ onClose: () => closed++ }));
  const dialog = host.querySelector('[role="dialog"]');
  assert.ok(dialog.contains(document.activeElement), 'focus moves into the sheet');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await settle();
  assert.equal(closed, 1);
});

test('the approval line follows the bots’ real approvals and never promises "asks first" when off', () => {
  assert.equal(m.approvalLine('Send mail', [bots[0]], 'ask').tone, 'ok');
  assert.equal(m.approvalLine('Send mail', [bots[0]], 'ask').text, 'Send mail only after you say yes');
  assert.match(m.approvalLine('Send mail', [bots[2]], 'ask').text, /without asking/);
  const mixed = m.approvalLine('Send mail', [bots[0], bots[2]], 'ask');
  assert.match(mixed.text, /Cody won’t ask first/);
  assert.doesNotMatch(m.approvalLine('Send mail', [], 'off').text, /say yes/);
  // F3 (Ada): smart asks before anything outward, so it gets the same line as ask.
  assert.deepEqual(m.approvalLine('Send mail', [{ name: 'X', approvals_effective: 'smart' }], 'ask'), { text: 'Send mail only after you say yes', tone: 'ok' });
  assert.match(m.approvalLine('Send mail', [bots[2]], 'ask').text, /\(approvals are off\)$/);
  assert.equal(mixed.text, 'Send mail: Cody won’t ask first (approvals off); the others ask you');
  const html = m.renderToStaticMarkup(h(m.ConnectSheet, sheetProps({ initial: { picked: ['k3'] } })));
  assert.match(html, /data-sheet-approval="warn"/);
  assert.ok(!html.includes('only after you say yes'));
});

// ── A bot pick turns the tool on ─────────────────────────────────────────────────────────────

const fakeServer = ({ enabled = false, lists = { k1: ['s9'], k2: ['s9', 'cmp1'], k3: ['cmp1'] } } = {}) => {
  const calls = [];
  const server = { id: 'cmp1', name: 'Composio', transport: 'http', command: null, args: [], env: {}, url: 'https://backend.composio.dev/v3/mcp/x', headers: { 'x-api-key': '••••' }, enabled, oauth: null, disabled_for_kwants: [] };
  const f = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    calls.push([method, url, init.body ? JSON.parse(init.body) : undefined]);
    const kw = url.match(/^\/api\/kwants\/(\w+)\/mcp-servers$/);
    const ok = (b) => ({ ok: true, status: 200, json: async () => b });
    if (kw && method === 'GET') return ok({ server_ids: lists[kw[1]] });
    if (kw && method === 'PUT') return (lists[kw[1]] = JSON.parse(init.body).server_ids, ok({ server_ids: lists[kw[1]] }));
    if (url === '/api/mcp/servers/cmp1' && method === 'GET') return ok(server);
    if (url === '/api/mcp/servers/cmp1' && method === 'PUT') return (server.enabled = JSON.parse(init.body).enabled, ok(server));
    return { ok: false, status: 404, json: async () => ({ error: 'nope' }) };
  };
  return { f, calls, lists, server };
};

test('a bot pick gives the server to exactly the picked bots and turns it on, keeping their other tools', async () => {
  const s = fakeServer();
  await m.giveServerTo('cmp1', ['k1', 'k2', 'k3'], ['k1'], s.f);
  assert.deepEqual(s.lists, { k1: ['s9', 'cmp1'], k2: ['s9'], k3: [] });
  assert.equal(s.server.enabled, true);
  const put = s.calls.find(([meth, url]) => meth === 'PUT' && url === '/api/mcp/servers/cmp1');
  assert.equal(put[2].enabled, true, 'the same call Tools’ on/off switch makes');
  assert.equal(put[2].url, 'https://backend.composio.dev/v3/mcp/x');
});

test('a server that is already on is not saved again, and no pick only takes it away', async () => {
  const on = fakeServer({ enabled: true });
  await m.giveServerTo('cmp1', ['k1', 'k2', 'k3'], ['k2', 'k3'], on.f);
  assert.ok(!on.calls.some(([meth, url]) => meth === 'PUT' && url.startsWith('/api/mcp/servers/')));
  assert.ok(!on.calls.some(([meth]) => meth === 'PUT'), 'nobody’s list changed');
  const none = fakeServer();
  await m.giveServerTo('cmp1', ['k1', 'k2', 'k3'], [], none.f);
  assert.equal(none.server.enabled, false);
  assert.deepEqual(none.lists, { k1: ['s9'], k2: ['s9'], k3: [] });
});

test('giving fails with the server’s own words', async () => {
  const f = async () => ({ ok: false, status: 500, json: async () => ({ error: 'Database is locked' }) });
  await assert.rejects(m.giveServerTo('cmp1', ['k1'], ['k1'], f), /Database is locked/);
  await assert.rejects(m.giveServerTo('cmp1', ['k1'], ['k1'], async () => { throw new TypeError('fetch failed'); }), /Couldn’t reach kwant/);
});

// ── AI accounts and the needs-you count ──────────────────────────────────────────────────────

test('an unused account that isn’t signed in is a quiet Sign in, never "needs you"', () => {
  const used = acct('p1', { status: 'needs_sign_in', used_by: [{ id: 'k1', name: 'Ann' }] });
  const unused = acct('p2', { name: 'Work', detail: 'codex', status: 'needs_sign_in' });
  assert.equal(m.accountNeedsYou(used), true);
  assert.equal(m.accountNeedsYou(unused), false);
  assert.equal(m.countsLine([used, unused]), '0 connected · 1 needs you');
  assert.equal(m.countsLine([unused]), '0 connected');
  assert.equal(m.pageCounts([used, unused]), '0 AI accounts ready · 0 tool servers on · 1 needs you');
});

// ── The page ─────────────────────────────────────────────────────────────────────────────────

const data = (over = {}) => ({
  connections: [acct('sys', { used_by: [{ id: 'k1', name: 'Ann' }, { id: 'k2', name: 'Ben' }] }), acct('p2', { name: 'Work', detail: 'codex', status: 'needs_sign_in' })],
  counts: {}, vault: { status: 'ok' }, catalog: [composio()], ...over,
});
const pageProps = (over = {}) => ({
  spaceName: 'Home', spaceMode: 'ask', kwants: bots, data: data(), profiles: [profile('sys', 'claude'), profile('p2', 'codex', { kind: 'cli_home', signed_in: false, priority: 1 })],
  engines: [{ id: 'claude', name: 'Claude', kind: 'cli', status: 'ready', enabled: true, ready: true, models: [] }, { id: 'codex', name: 'Codex', kind: 'cli', status: 'ready', enabled: true, ready: true, models: [] }],
  manageAccounts: () => h('p', null, 'MANAGE'), advanced: () => h('p', null, 'ADVANCED'), phone: false, ...over,
});

test('the page puts app tiles first, then another app, AI accounts, and a closed Advanced', () => {
  const html = m.renderToStaticMarkup(h(m.AppsAccounts, pageProps()));
  const order = ['data-apps-tiles', 'data-apps-search', 'data-ai-accounts', 'data-apps-advanced'].map((k) => html.indexOf(k));
  assert.ok(order.every((i, n) => i > 0 && (n === 0 || i > order[n - 1])), `order ${order}`);
  assert.deepEqual([...html.matchAll(/data-app-tile="(\w+)"/g)].map((x) => x[1]), ['gmail', 'googlecalendar', 'slack', 'notion', 'shopify', 'github']);
  assert.ok(html.includes('#231d4b'), 'plain initials on the overlay colour');
  assert.ok(!html.includes('<img'), 'no logos');
  assert.ok(!html.includes('ADVANCED'), 'Advanced is closed');
  assert.ok(html.includes('data-quiet-sign-in'), 'the unused Codex account offers a quiet Sign in');
  assert.ok(!html.includes('data-ai-needs'), 'and nothing "needs you"');
  assert.ok(!html.includes('Turn on in Tools'));
  // Decision 2: the order groups are the list, so each account shows once, with who uses it.
  const ai = html.slice(html.indexOf('data-ai-accounts'), html.indexOf('data-apps-advanced'));
  assert.equal((ai.match(/data-account-title/g) ?? []).length, 2, 'each account once');
  assert.match(ai, /Used by Ann and Ben/);
  assert.match(ai, /Signed in on this computer/);
  assert.doesNotMatch(ai, /This machine|MANAGE/, 'sign-ins and keys stay collapsed');
  assert.match(ai, /Sign-ins and settings/);
  // F10: the intro names both halves.
  assert.match(html, /The apps your bots can use, and the AI they think with\./);
  // F7: each tile has its own initials colour and a two-line blurb.
  const tints = [...html.matchAll(/data-app-mark-tint="([^"]+)"/g)].map((x) => x[1]);
  assert.equal(new Set(tints).size, 6, `six tints: ${tints}`);
  assert.equal((html.match(/line-clamp-2/g) ?? []).length, 6);
  // F8: phone-sized targets on the page's buttons.
  assert.match(html, /min-h-\[44px\] sm:min-h-\[32px\]/);
});

test('a tile opens the Connect sheet and Escape closes it', async () => {
  const host = await mount(m.AppsAccounts, pageProps({ connectApi: fakeApi() }));
  await click(host.querySelector('[data-app-tile="slack"]'));
  assert.equal(text(document.querySelector('[role="dialog"] h2')), 'Connect Slack');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await settle();
  assert.equal(document.querySelector('[data-connect-sheet]'), null);
});

test('opening at Advanced shows it; a used account that can’t run counts as needing you', () => {
  const html = m.renderToStaticMarkup(
    h(m.AppsAccounts, pageProps({ section: 'advanced', data: data({ connections: [acct('sys', { status: 'needs_sign_in', used_by: [{ id: 'k1', name: 'Ann' }] })] }) })),
  );
  assert.ok(html.includes('ADVANCED'));
  assert.match(html, /data-ai-needs[^>]*>1 needs you/);
});

test('Advanced’s Composio card offers Choose bots instead of Turn on in Tools, and leaves AI accounts out', () => {
  const html = m.renderToStaticMarkup(
    h(m.ConnectionsView, {
      data: data({ catalog: [composio({ added: true, connection_id: 'mcp:cmp1' })] }), loading: false, error: null, onRefresh: () => {}, onAddComposio: async () => null,
      embedded: true, hideAccounts: true, onChooseComposioBots: () => {}, onOpenTools: () => {},
    }),
  );
  assert.ok(html.includes('Choose bots to turn it on'));
  assert.ok(!html.includes('Turn on in Tools'));
  assert.ok(!html.includes('Engine accounts'));
});

// ── Old addresses ────────────────────────────────────────────────────────────────────────────

test('old Settings addresses land on Apps & accounts and are rewritten', () => {
  assert.ok(m.SPACE_SETTINGS_TABS.includes('apps'));
  for (const old of ['accounts', 'connections', 'tools']) assert.ok(!m.SPACE_SETTINGS_TABS.includes(old));
  const cases = { 'settings/accounts': 'accounts', 'settings/connections': 'advanced', 'settings/tools': 'advanced', connections: 'advanced' };
  for (const [seg, section] of Object.entries(cases)) {
    const r = m.parseView(seg);
    assert.deepEqual(r, { view: { kind: 'settings', scope: 'space', tab: 'apps', section }, legacy: true }, seg);
    assert.equal(m.viewSegment(r.view), 'settings/apps');
  }
  assert.deepEqual(m.parseView('settings/apps'), { view: { kind: 'settings', scope: 'space', tab: 'apps' }, legacy: false });
});

// ── Mira's design check (design-check.md F1–F9) ──────────────────────────────────────────────

const readyComposio = () => composio({ added: true, enabled: true, connection_id: 'mcp:cmp1', can_connect: true });

test('F1: the done line is green only when Composio confirms the app sign-in; otherwise it says what’s left', async () => {
  for (const [signedIn, tone, want] of [
    [true, 'ok', 'Gmail is on for Ann.'],
    [false, 'info', 'Composio is on for Ann. Ann can use Gmail once you’ve signed in to Gmail on Composio.'],
  ]) {
    const host = await mount(m.AppsAccounts, pageProps({
      data: data({ catalog: [readyComposio()] }), connectApi: fakeApi(), appStatus: async () => ({ connected: signedIn }),
      initialSheet: { slug: 'gmail', initial: { picked: ['k1'] } },
    }));
    await submit(document.querySelector('[data-connect-sheet]'));
    const n = host.querySelector('[data-apps-notice]');
    assert.equal(n?.getAttribute('data-apps-notice'), tone);
    assert.equal(text(n), want);
    for (const u of [...mounted]) u();
  }
});

test('F1: a tile Composio can’t check says "Sign in or choose bots", not Connect or Connected', () => {
  const html = m.renderToStaticMarkup(h(m.AppsAccounts, pageProps({ data: data({ catalog: [composio({ added: true, enabled: true, connection_id: 'mcp:cmp1', can_connect: false })] }) })));
  assert.equal((html.match(/data-tile-state="unknown"/g) ?? []).length, 6);
  assert.match(html, /Sign in or choose bots/);
  const fresh = m.renderToStaticMarkup(h(m.AppsAccounts, pageProps()));
  assert.equal((fresh.match(/data-tile-state="connect"/g) ?? []).length, 6);
});

test('F2: with Composio set up the steps are 1 and 2, and Choose bots alone has no number', () => {
  const ready = m.renderToStaticMarkup(h(m.ConnectSheet, sheetProps({ composio: readyComposio() })));
  assert.deepEqual([...ready.matchAll(/data-step-label="(\d+)"/g)].map((x) => x[1]), ['1', '2']);
  const choose = m.renderToStaticMarkup(h(m.ConnectSheet, sheetProps({ app: null, composio: readyComposio() })));
  assert.match(choose, /Choose bots for Composio/);
  assert.match(choose, /Picked bots can use every app on your Composio/);
  assert.doesNotMatch(choose, /data-step-label=/);
  assert.doesNotMatch(choose, /Step \d/);
});

test('F4: bot chips show the bot’s face and step 3 says Composio is one connection', () => {
  const html = m.renderToStaticMarkup(h(m.ConnectSheet, sheetProps()));
  const chip = html.slice(html.indexOf('data-sheet-bot="k1"'), html.indexOf('data-sheet-bot="k2"'));
  assert.match(chip, /aria-hidden="true"/);
  assert.match(chip, /min-h-\[44px\]/);
  assert.match(html, /data-sheet-one-connection="true">Composio is one connection: the bots you tick can use every app on it, not only Gmail\./);
});

test('F5: the link error sits right under the field and is read with it; a good link ticks step 2', async () => {
  const host = await mount(m.ConnectSheet, sheetProps({ api: fakeApi() }));
  const input = host.querySelector('input[type="url"]');
  await submit(host);
  const err = host.querySelector('[data-sheet-error="2"]');
  assert.equal(err.getAttribute('role'), 'alert');
  assert.ok(input.getAttribute('aria-describedby').split(' ').includes(err.id));
  assert.equal(input.getAttribute('aria-invalid'), 'true');
  const after = [...input.closest('[data-sheet-step]').querySelectorAll('*')];
  assert.ok(after.indexOf(err) > after.indexOf(input), 'error after the input');
  await type(input, 'https://backend.composio.dev/v3/mcp/abc');
  assert.equal(host.querySelector('[data-sheet-step="2"]').getAttribute('data-done'), 'true');
});

test('Sign-ins and settings has no account list of its own: no "What runs your kwant bots" there', () => {
  const { EnginesPanel } = m;
  const engines = [{ id: 'claude', name: 'Claude Code CLI', kind: 'cli', status: 'ready', enabled: true, ready: true, models: [], checked_at: null, uses: { profile_id: 'sys', label: 'This machine' } }];
  const api = { engines, error: null, rechecking: false, recheck: async () => {}, setEnabled: async () => {}, reload: async () => {} };
  const accounts = { profiles: [profile('sys', 'claude')], error: null, reload: async () => {} };
  const full = m.renderToStaticMarkup(h(EnginesPanel, { api, accounts }));
  assert.match(full, /data-route-summary/, 'the panel still has it elsewhere');
  const here = m.renderToStaticMarkup(h(EnginesPanel, { api, accounts, hideRoutes: true }));
  assert.doesNotMatch(here, /data-route-summary|What runs your/);
  assert.match(here, /Claude Code CLI/, 'sign-ins stay');
});

test('F16 and F13: tile labels sit on the tile’s bottom edge; the Sign-ins summary is 44 px', () => {
  const html = m.renderToStaticMarkup(h(m.AppsAccounts, pageProps()));
  const states = [...html.matchAll(/<span class="([^"]*)" data-tile-state=/g)].map((x) => x[1]);
  assert.equal(states.length, 6);
  for (const c of states) assert.match(c, /^mt-auto /);
  assert.match(html, /<summary class="[^"]*min-h-\[44px\][^"]*"[^>]*>[\s\S]*?Sign-ins and settings/);
});

test('round 5 (qa item 3): the page names bots on auto only on "Used first"; backups name bots pinned there', () => {
  // Rigel's space: Ann and Ben on Auto, Cody pinned to Codex, Gus pinned to Grok, and Grok put first.
  // The server lists auto bots on every engine's main account; the page keeps them to the first row.
  const kwants = [
    { id: 'k1', name: 'Ann', approvals_effective: 'ask', preferred_model: 'auto' },
    { id: 'k2', name: 'Ben', approvals_effective: 'ask', preferred_model: '' },
    { id: 'k3', name: 'Cody', approvals_effective: 'ask', preferred_model: 'codex' },
    { id: 'k4', name: 'Gus', approvals_effective: 'ask', preferred_model: 'grok/grok-4' },
  ];
  const [ann, ben, cody, gus] = kwants.map(({ id, name }) => ({ id, name }));
  const html = m.renderToStaticMarkup(
    h(
      m.AppsAccounts,
      pageProps({
        kwants,
        profiles: [profile('grok', 'grok', { is_default: true, priority: 10 }), profile('claude', 'claude', { is_default: true, priority: 20 }), profile('codex', 'codex', { is_default: true, priority: 30 })],
        engines: ['grok', 'claude', 'codex'].map((id) => ({ id, name: id[0].toUpperCase() + id.slice(1), kind: 'cli', status: 'ready', enabled: true, ready: true, models: [] })),
        data: data({
          connections: [
            acct('grok', { detail: 'grok', used_by: [ann, ben, gus] }),
            acct('claude', { detail: 'claude', used_by: [ann, ben] }),
            acct('codex', { detail: 'codex', used_by: [ann, ben, cody] }),
          ],
        }),
      }),
    ),
  );
  const part = (name) => html.split(`data-group="${name}"`)[1]?.split('data-group=')[0] ?? '';
  assert.match(part('first'), /Used by Ann, Ben and Gus/);
  const backup = part('backup');
  assert.match(backup, /Cody uses it first/);
  assert.doesNotMatch(backup, /Ann|Ben/, 'was "Ann, Ben and Cody use it first" and "Ann and Ben use it first"');
});
