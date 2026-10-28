// Ada (2026-10-28): a bot's "connect this app" card (meta kind connection_suggestion, from #106) gets
// a one-tap Connect that opens the sign-in outside kwant, watches until the app is connected and then
// says so. Without a usable Composio connector it says why and links to Connections instead.
import assert from 'node:assert/strict';
import test from 'node:test';
import { loadBundled } from './bundle.mjs';

globalThis.localStorage ??= { getItem: () => null, setItem: () => {} };

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ConnectionSuggestionView } from './src/components/ConnectionSuggestionCard';
  export * from './src/connectionSuggestions';
  export { connectionsApi, ConnectionsError } from './src/connections';
  export const view = (p) =>
    renderToStaticMarkup(React.createElement(ConnectionSuggestionView, { messageId: 'm1', kwantName: 'Scout', onConnect: () => {}, onOpenConnections: () => {}, ...p }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/’/g, "'").replace(/&#x27;|&#39;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const buttons = (html) => [...html.matchAll(/<(?:button|a)[^>]*>([\s\S]*?)<\/(?:button|a)>/g)].map((x) => text(x[1]));
const raw = (over = {}) => ({
  kind: 'connection_suggestion',
  kwant_id: 'k1',
  provider: 'composio',
  app: 'google_search_console',
  name: 'Google Search Console',
  reason: 'It shows which pages Google indexed.',
  composio: 'ready',
  connect: '/api/connections/composio/apps/google_search_console/connect',
  ...over,
});
const meta = (over) => m.parseConnectionMeta(raw(over));

// ---------------------------------------------------------------- the message meta

test('the card meta is read from the message, and anything else is left to other cards', () => {
  assert.deepEqual(meta(), {
    kind: 'connection_suggestion',
    app: 'google_search_console',
    name: 'Google Search Console',
    reason: 'It shows which pages Google indexed.',
    composio: 'ready',
    kwant_id: 'k1',
  });
  assert.equal(m.parseConnectionMeta(JSON.stringify(raw())).app, 'google_search_console', 'stored meta can be a JSON string');
  assert.equal(m.parseConnectionMeta(raw({ kind: 'mcp_suggestion' })), null);
  assert.equal(m.parseConnectionMeta(raw({ app: '../connect' })), null, 'only a plain slug goes into the URL');
  assert.equal(m.parseConnectionMeta(raw({ app: 'Gmail' })), null);
  assert.equal(meta({ name: '' }).name, 'google_search_console', 'no name: the slug');
  assert.equal(meta({ composio: 'weird' }).composio, 'ready', 'an unknown value lets Connect try; the server answers 409 if it can not');
  assert.equal(meta({ kwant_id: undefined }).kwant_id, null);
});

// ---------------------------------------------------------------- each state, drawn

test('ready: one Connect button, the app, the bot and why, and that nothing connects yet', () => {
  const html = m.view({ meta: meta(), state: m.initialState(meta()) });
  assert.match(html, /data-connection-card="ready"/);
  assert.deepEqual(buttons(html), ['Connect']);
  const t = text(html);
  assert.match(t, /Scout suggests a connection/);
  assert.match(t, /Google Search Console/);
  assert.match(t, /Why: It shows which pages Google indexed\./);
  assert.match(t, /Nothing connects until you do\./);
  assert.match(t, /through Composio, which keeps the sign-in on its servers/);
  assert.doesNotMatch(t, /Connected/);
});

test('connecting: the button waits, then the card says where to finish and offers the sign-in again', () => {
  const opening = m.view({ meta: meta(), state: { phase: 'opening' } });
  assert.match(opening, /<button[^>]*disabled=""[^>]*data-connect/);
  assert.deepEqual(buttons(opening), ['Opening the sign-in…']);

  const tab = m.view({ meta: meta(), state: { phase: 'waiting' } });
  assert.match(text(tab), /Finish signing in to Google Search Console in the new tab\. This updates by itself when you're done\./);
  assert.deepEqual(buttons(tab), ['Open the sign-in again']);
  assert.match(tab, /aria-live="polite"/, 'the change to Connected is announced');

  const mac = m.view({ meta: meta(), state: { phase: 'waiting' }, mac: true });
  assert.match(text(mac), /in your browser\. This updates by itself/);

  const blocked = m.view({ meta: meta(), state: { phase: 'waiting', link: 'https://connect.composio.dev/link/abc' } });
  assert.match(text(blocked), /Your browser blocked the new tab/);
  assert.match(blocked, /<a href="https:\/\/connect\.composio\.dev\/link\/abc" target="_blank" rel="noopener noreferrer"/);
  assert.deepEqual(buttons(blocked), ['Open the sign-in (opens in a new tab)']);
});

test('connected: says so, and no button is left', () => {
  const html = m.view({ meta: meta(), state: { phase: 'connected' } });
  assert.match(html, /data-connection-card="connected"/);
  assert.match(text(html), /Connected\. Scout can use Google Search Console from its next turn\./);
  assert.deepEqual(buttons(html), []);
});

test('Composio missing, without a key or off: the reason and Open Connections instead of Connect', () => {
  for (const [composio, reason] of [
    ['not_added', /Composio, which isn't added yet\. Add it in Connections first/],
    ['needs_key', /has no API key yet\. Add your Composio API key in Connections first/],
    ['off', /switched off\. Switch it on in Connections first/],
  ]) {
    const st = m.initialState(meta({ composio }));
    assert.equal(st.phase, 'missing', composio);
    const html = m.view({ meta: meta({ composio }), state: st });
    assert.match(text(html), reason, composio);
    assert.deepEqual(buttons(html), ['Open Connections'], composio);
  }
  assert.equal(m.composioMissingText('ready'), null);
});

test('timed out, unchecked and error: never a false "Connected", always a way on', () => {
  const late = m.view({ meta: meta(), state: { phase: 'timed_out' } });
  assert.match(text(late), /hasn't finished yet/);
  assert.deepEqual(buttons(late), ['Open the sign-in again', 'Open Connections']);

  const older = m.view({ meta: meta(), state: { phase: 'unchecked' } });
  assert.match(text(older), /Once you have, its tools reach your bots from their next turn\./);
  assert.doesNotMatch(text(older), /Connected|updates by itself/);
  assert.doesNotMatch(older, /animate-spin/, 'nothing pretends to wait');

  const err = m.view({ meta: meta(), state: { phase: 'error', message: 'Couldn’t reach the server.' } });
  assert.match(err, /role="alert"/);
  assert.deepEqual(buttons(err), ['Try again']);
});

// ---------------------------------------------------------------- one press of Connect

const fakeWin = ({ blocked = false } = {}) => {
  const w = { opened: [], assigned: [], tabs: [] };
  w.open = (url, target) => {
    w.opened.push([url, target]);
    if (blocked) return null;
    const tab = { opener: 'kwant', closed: false, location: { href: url }, close() { this.closed = true; } };
    w.tabs.push(tab);
    return tab;
  };
  w.location = { assign: (u) => w.assigned.push(u) };
  return w;
};
const err = (status, msg = 'nope') => new m.ConnectionsError(msg, status);
const fakeApi = ({ connect = async () => ({ redirect_url: 'https://connect.composio.dev/link/abc' }), statuses = [] } = {}) => {
  const api = { connects: 0, checks: 0 };
  api.connect = async (slug) => {
    api.connects++;
    api.slug = slug;
    return connect();
  };
  api.status = async () => {
    const next = statuses[Math.min(api.checks++, statuses.length - 1)];
    if (next instanceof Error) throw next;
    return { connected: !!next };
  };
  return api;
};
const run = (opts) =>
  new Promise((resolve) => {
    const seen = [];
    const stop = m.startConnect('google_search_console', (s) => {
      seen.push(s);
      if (!['opening', 'waiting'].includes(s.phase)) resolve({ seen, stop });
    }, { everyMs: 1, forMs: 200, ...opts });
  });
const phases = (seen) => seen.map((s) => s.phase);

test('browser: the tab opens inside the click, goes to the sign-in, and the card turns Connected', async () => {
  const win = fakeWin();
  const api = fakeApi({ statuses: [false, err(502), false, true] });
  const { seen } = await run({ win, api, mac: false });
  assert.deepEqual(win.opened, [['', '_blank']], 'opened before the fetch, so popup blockers allow it');
  assert.equal(win.tabs[0].opener, null, 'the sign-in page cannot reach back into kwant');
  assert.equal(win.tabs[0].location.href, 'https://connect.composio.dev/link/abc');
  assert.equal(api.slug, 'google_search_console');
  assert.deepEqual(phases(seen), ['opening', 'waiting', 'connected'], 'a failed check just waits for the next');
  assert.equal(seen[1].link, undefined);
  assert.equal(api.checks, 4);
});

test('Mac app: no second window; the sign-in goes to the default browser and the card still turns Connected', async () => {
  const win = fakeWin();
  const { seen } = await run({ win, api: fakeApi({ statuses: [true] }), mac: true });
  assert.deepEqual(win.opened, []);
  assert.deepEqual(win.assigned, ['https://connect.composio.dev/link/abc']);
  assert.deepEqual(phases(seen), ['opening', 'waiting', 'connected']);
});

test('a blocked tab: the card offers the sign-in as a link and keeps watching', async () => {
  const win = fakeWin({ blocked: true });
  const { seen } = await run({ win, api: fakeApi({ statuses: [false, true] }), mac: false });
  assert.equal(seen[1].link, 'https://connect.composio.dev/link/abc');
  assert.deepEqual(win.assigned, [], 'kwant itself never leaves for Composio');
  assert.equal(phases(seen).at(-1), 'connected');
});

test('Connect refused: 409 means Composio is not usable (missing), anything else is an error; the empty tab closes', async () => {
  let win = fakeWin();
  let r = await run({ win, api: fakeApi({ connect: async () => { throw err(409, 'Add the Composio connector first.'); } }), mac: false });
  assert.deepEqual(r.seen.at(-1), { phase: 'missing', message: 'Add the Composio connector first.' });
  assert.equal(win.tabs[0].closed, true);

  win = fakeWin();
  r = await run({ win, api: fakeApi({ connect: async () => { throw err(502, 'Composio didn’t answer.'); } }), mac: false });
  assert.deepEqual(r.seen.at(-1), { phase: 'error', message: 'Composio didn’t answer.' });
  assert.equal(win.tabs[0].closed, true);
});

test('watching ends honestly: an older server gives "unchecked", a 409 gives "missing", 10 minutes give "timed out"', async () => {
  let r = await run({ win: fakeWin(), api: fakeApi({ statuses: [err(404)] }), mac: false });
  assert.equal(r.seen.at(-1).phase, 'unchecked');
  r = await run({ win: fakeWin(), api: fakeApi({ statuses: [err(409, 'Switch Composio on.')] }), mac: false });
  assert.deepEqual(r.seen.at(-1), { phase: 'missing', message: 'Switch Composio on.' });
  r = await run({ win: fakeWin(), api: fakeApi({ statuses: [false] }), mac: false, forMs: 10 });
  assert.equal(r.seen.at(-1).phase, 'timed_out');
});

test('stopping (card gone, or pressed again) ends the watching', async () => {
  const api = fakeApi({ statuses: [false] });
  const seen = [];
  const stop = m.startConnect('google_search_console', (s) => seen.push(s), { win: fakeWin(), api, mac: false, everyMs: 1, forMs: 10_000 });
  await new Promise((r) => setTimeout(r, 20));
  stop();
  const checks = api.checks;
  await new Promise((r) => setTimeout(r, 20));
  assert.ok(checks > 0);
  assert.ok(api.checks <= checks + 1, 'no more checks after stop');
  assert.ok(!seen.some((s) => s.phase === 'connected' || s.phase === 'timed_out'));
});

test('already connected when the card appears: shown as connected; unknown stays unknown', async () => {
  assert.equal(await m.connectedNow(fakeApi({ statuses: [true] }), 'gmail'), true);
  assert.equal(await m.connectedNow(fakeApi({ statuses: [false] }), 'gmail'), false);
  assert.equal(await m.connectedNow(fakeApi({ statuses: [err(404)] }), 'gmail'), null);
});

// ---------------------------------------------------------------- the status call

test('the status call reads {connected}, and a server without the route is a 404, not "connected"', async () => {
  const prev = globalThis.fetch;
  const answers = [
    [200, { connected: true }],
    [200, { connected: false }],
    [200, null], // the app page from an older server's fallback
    [404, null],
    [409, { error: 'add the Composio connector first' }],
  ];
  const urls = [];
  globalThis.fetch = async (url) => {
    urls.push(url);
    const [status, body] = answers.shift();
    return new Response(body === null ? '<!doctype html>' : JSON.stringify(body), { status });
  };
  try {
    assert.deepEqual(await m.connectionsApi.composioAppStatus('google_search_console'), { connected: true });
    assert.equal(urls[0], '/api/connections/composio/apps/google_search_console/status');
    assert.deepEqual(await m.connectionsApi.composioAppStatus('x'), { connected: false });
    await assert.rejects(m.connectionsApi.composioAppStatus('x'), (e) => e.status === 404);
    await assert.rejects(m.connectionsApi.composioAppStatus('x'), (e) => e.status === 404);
    await assert.rejects(m.connectionsApi.composioAppStatus('x'), (e) => e.status === 409 && /Add the Composio connector first/.test(e.message));
  } finally {
    globalThis.fetch = prev;
  }
});
