// 26e Connect Home Assistant and the Home team (design: autonomy/connect-home-assistant/design.md).
// Example data only: the "token" below is a made-up placeholder string, never a real access token.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'CustomEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * from './src/homeAssistant';
  export { homeAction, homePhrase, readableStepTitle } from './src/toolLabel';
  export { ModernApprovalCard, approvalAction } from './src/components/ModernApprovalCard';
  export { HomeAssistantSheet } from './src/components/HomeAssistantSheet';
  export { needView } from './src/teams/teamTemplates';
  export { TEMPLATES_WIRE } from './src/teams/templateFixtures';
  export { newKey, isNewThing } from './src/whatsNew';
  export { MCP_PRESETS, searchPresets } from './src/mcp';
  export { matchTool } from './src/toolSearch';
  export { TeamStep } from './src/components/GetStarted';
  export { ReadyTeamsProvider } from './src/components/ReadyTeams';
`);
const { React, createRoot } = m;
const h = React.createElement;
const FAKE_TOKEN = 'example-placeholder-not-a-token';

const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const typeIn = (el, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
};
const mount = async (el) => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(el);
  await settle();
  return { host, unmount: () => (root.unmount(), host.remove()) };
};

// ---- Address and result rules

test('the address becomes the MCP URL: http by default, 8123 for plain http, https keeps its port', () => {
  assert.deepEqual(m.normaliseHaAddress('homeassistant.local:8123'), { url: 'http://homeassistant.local:8123/api/mcp', origin: 'http://homeassistant.local:8123', host: 'homeassistant.local:8123' });
  assert.equal(m.normaliseHaAddress('192.168.1.20')?.url, 'http://192.168.1.20:8123/api/mcp');
  assert.equal(m.normaliseHaAddress(' https://home.example.org/lovelace/0 ')?.url, 'https://home.example.org/api/mcp');
  assert.equal(m.normaliseHaAddress('http://localhost:8123/api/mcp')?.url, 'http://localhost:8123/api/mcp');
  for (const bad of ['', 'foo', 'ftp://x.local', 'http://user:pw@x.local', 'two words.local']) assert.equal(m.normaliseHaAddress(bad), null, bad);
});

test('a test result maps to one plain state, first match wins', () => {
  assert.deepEqual(m.mapHaTest({ ok: true, tools: [{ name: 'HassTurnOn' }, { name: 'HassTurnOff' }], server_info: { name: 'home-assistant' } }), { kind: 'ok', actions: 2 });
  assert.equal(m.mapHaTest({ ok: true, tools: [], server_info: { name: 'other-server' } }).kind, 'not_ha');
  assert.equal(m.mapHaTest({ ok: false, auth_required: true }).kind, 'bad_token');
  assert.equal(m.mapHaTest({ ok: false, error: 'HTTP 401 Unauthorized' }).kind, 'bad_token');
  assert.equal(m.mapHaTest({ ok: false, error: 'HTTP 404 Not Found' }).kind, 'mcp_off');
  assert.equal(m.mapHaTest({ ok: false, error: 'error sending request for url' }).kind, 'unreachable');
  assert.equal(m.mapHaTest({ ok: false, error: 'something odd' }).kind, 'other');
  assert.equal(m.haResultTitle({ kind: 'ok', actions: 1 }), 'Connected · 1 action');
  assert.equal(m.haResultTitle({ kind: 'unreachable' }, '192.168.1.20:8123'), 'kwant can’t reach 192.168.1.20:8123.');
});

test('one Home Assistant in Tools: found by name or /api/mcp URL, and new ones get a free name', () => {
  const list = [
    { id: 's1', name: 'github', url: 'https://api.githubcopilot.com/mcp/', transport: 'http' },
    { id: 's2', name: 'Home_Assistant', url: 'http://ha.local:8123/api/mcp', transport: 'http' },
  ];
  assert.equal(m.findHaServer(list)?.id, 's2');
  assert.equal(m.findHaServer(list.slice(0, 1)), null);
  assert.equal(m.freeHaName(['github']), 'home-assistant');
  assert.equal(m.freeHaName(['home-assistant', 'home-assistant-2']), 'home-assistant-3');
});

test('the Home team row: Connected · N actions only after a passing check; failed says the next step', () => {
  const s = [{ id: 'ha', name: 'home-assistant', url: 'http://ha.local:8123/api/mcp', transport: 'http', enabled: true }];
  assert.deepEqual(m.haNeedState(null, {}), { state: 'checking' });
  assert.equal(m.haNeedState([], {}).state, 'none');
  assert.equal(m.haNeedState(s, {}).state, 'none', 'saved but never tested is not done');
  assert.deepEqual(m.haNeedState(s, { ha: { ok: true, tools: ['a', 'b', 'c'] } }), { state: 'ok', actions: 3, detail: '3 actions' });
  const bad = m.haNeedState(s, { ha: { ok: false, tools: [], auth_required: true } });
  assert.equal(bad.state, 'failed');
  assert.match(bad.detail, /didn’t accept the access token\. Make a new one/);
  const need = { id: 'ha', opens: 'home_assistant', required: true, why: 'House needs it' };
  const ctx = { canRun: true, imap: { state: 'none' }, apps: [], repo: { state: 'none' }, needsDone: [] };
  assert.deepEqual(m.needView(need, { ...ctx, homeAssistant: { state: 'ok', detail: '3 actions', actions: 3 } }), { state: 'done', text: 'Connected · 3 actions' });
  assert.equal(m.needView(need, { ...ctx, homeAssistant: { state: 'failed', detail: 'x' } }).state, 'bad');
  assert.equal(m.needView(need, ctx).state, 'req');
});

test('the Home template: House on the home sprite, Lights out off by default', () => {
  const home = m.TEMPLATES_WIRE.find((t) => t.id === 'home');
  assert.ok(home, 'Home template in the fixtures');
  assert.equal(home.bots[0].avatar_sprite, 'home');
  assert.equal(home.bots[0].name, 'House');
  assert.equal(home.routine.enabled, false);
});

test('New tags: home-team and home-assistant are new in 0.9.22; older ids keep their version', () => {
  assert.equal(m.newKey('home-team'), 'kwant:new:0.9.22:home-team');
  assert.equal(m.newKey('home-assistant'), 'kwant:new:0.9.22:home-assistant');
  assert.equal(m.newKey('mention'), 'kwant:new:0.6.9:mention');
});

test('Home Assistant is found in Add a tool by its own words', () => {
  assert.equal(m.MCP_PRESETS[0].id, 'home-assistant');
  for (const q of ['home assistant', 'hass', 'lights']) assert.ok(m.searchPresets(q).some((p) => p.id === 'home-assistant'), q);
});

// ---- House-friendly names

test('HA tools read in house words, with the target first by name, then area, floor, domain', () => {
  const p = (tool, args, ask) => m.homePhrase(m.homeAction(tool, args, ask), 'base');
  assert.equal(p('intent__HassTurnOff', { name: 'living_room_lamp' }), 'turn off Living room lamp');
  assert.equal(p('HassTurnOn', { area: 'Kitchen', domain: ['light'] }), 'turn on Kitchen lights');
  assert.equal(p('HassTurnOff', { floor: 'Upstairs' }), 'turn off everything in Upstairs');
  assert.equal(p('HassTurnOff', { domain: ['media_player'] }), 'turn off all speakers and TVs');
  assert.equal(p('HassTurnOff', {}), 'turn something off');
  assert.equal(p('HassTurnOff', { name: 'Front door', domain: ['lock'] }), 'unlock Front door');
  assert.equal(p('HassTurnOn', { name: 'Garage', domain: 'cover' }), 'open Garage');
  assert.equal(p('GetLiveContext', {}), 'look at your home');
  assert.equal(p('HassClimateSetTemperature', { name: 'Hall', temperature: 21 }), 'set Hall to 21 °C');
  assert.equal(m.homeAction('create_pull_request', {}), null, 'other tools fall through');
});

test('safety: an always-ask on/off call with no domain never reads "turn off"', () => {
  const a = m.homeAction('HassTurnOff', { name: 'Front door' }, true);
  assert.equal(m.homePhrase(a, 'base'), 'unlock or open Front door');
});

test('step titles: past form plus Home Assistant; waiting asks in words', () => {
  assert.equal(m.readableStepTitle('Using intent__HassTurnOff (home-assistant)'), 'Turned something off · Home Assistant');
  assert.equal(m.readableStepTitle('Using GetLiveContext (home-assistant)'), 'Looked at your home');
  assert.equal(m.readableStepTitle('Using HassClimateSetTemperature (home-assistant)'), 'Set the temperature · Home Assistant');
  assert.equal(m.readableStepTitle('Waiting for your approval: Using intent__HassTurnOff (home-assistant)'), 'Waiting for your OK to turn something off');
  assert.equal(m.readableStepTitle('Using create_pull_request (github)'), 'Opening a pull request on GitHub');
});

const haTicket = (tool, args, extra = {}) => ({
  id: 'h1',
  tool_name: `mcp__home-assistant__${tool}`,
  details: { source: 'mcp', server: 'home-assistant', tool, arguments: args, kwant_name: 'House', ...extra },
});
const card = (a) => m.renderToStaticMarkup(h(m.ModernApprovalCard, { approval: a, onResolve: () => {} }));

test('approval card: the device is bold, Area and In Home Assistant are the facts, no lock note on a lamp', () => {
  const html = card(haTicket('HassTurnOff', { name: 'Living room lamp', area: 'Living room' }));
  assert.match(html, /wants to turn off <strong[^>]*>Living room lamp<\/strong>/);
  assert.match(html, />Area<\/dt><dd[^>]*>Living room</);
  assert.match(html, /In<\/dt><dd[^>]*>Home Assistant</);
  assert.match(html, /Always allow/);
  assert.doesNotMatch(html, /data-lock-note/);
  assert.doesNotMatch(html, /kw-sticker-holo/, 'never holo on an approval');
});

test('approval card: a 26f lock call shows the lock note and no Always allow', () => {
  const html = card(haTicket('HassTurnOff', { name: 'Front door', domain: ['lock'] }, { home_always_ask: true }));
  assert.match(html, /wants to unlock <strong[^>]*>Front door<\/strong>/);
  assert.match(html, /data-lock-note[^>]*>.*Locks, doors, garage doors, gates and alarms always ask\./s);
  assert.doesNotMatch(html, /Always allow<\/span>/);
  assert.doesNotMatch(html, /kw-sticker-holo/);
});

test('approval card: a started-by card (same always_ask field) for another tool gets no lock note', () => {
  const html = card({ id: 'x', tool_name: 'write_file', details: { path: '/tmp/a', always_ask: true, started_by: 'Watch' } });
  assert.doesNotMatch(html, /data-lock-note/);
});

test('approval card: a 26f lock call without a domain still says unlock or open, from home_always_ask', () => {
  const html = card(haTicket('HassTurnOff', { name: 'Front door' }, { home_always_ask: true }));
  assert.match(html, /wants to unlock( or open)? <strong[^>]*>Front door<\/strong>/);
  assert.match(html, /data-lock-note/);
});

test('approval card: always_ask with no home_always_ask shows no lock note, even on Home Assistant', () => {
  const started = card(haTicket('HassTurnOff', { name: 'Front door' }, { always_ask: true, started_by: 'Watch' }));
  assert.doesNotMatch(started, /data-lock-note/);
  assert.doesNotMatch(started, /wants to unlock/, 'the lock verb reads home_always_ask only');
  assert.match(started, /wants to turn off <strong[^>]*>Front door<\/strong>/);
  const plain = card(haTicket('HassTurnOn', { name: 'Garage' }, { always_ask: true }));
  assert.doesNotMatch(plain, /data-lock-note/);
  assert.doesNotMatch(plain, /wants to (lock|open)/);
});

test('approval card: home_always_ask on a non-Home Assistant card gets no lock note', () => {
  const html = card({ id: 'y', tool_name: 'write_file', details: { path: '/tmp/a', home_always_ask: true } });
  assert.doesNotMatch(html, /data-lock-note/);
  assert.doesNotMatch(html, /Always allow<\/span>/);
});

// ---- The sheet

/** A fake kwant server: records calls, answers the test with `testResult`. Never holds a real token. */
const fakeServer = (testResult, servers = []) => {
  const calls = [];
  const fetcher = async (url, init = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body ? JSON.parse(init.body) : null });
    const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
    if (url === '/api/mcp/servers' && (init.method ?? 'GET') === 'GET') return json(servers);
    if (url === '/api/mcp/servers' && init.method === 'POST') return json({ id: 'srv_ha', name: 'home-assistant' });
    if (/\/test$/.test(url)) return json(testResult);
    if (/^\/api\/kwants\/[^/]+\/mcp-servers$/.test(url) && (init.method ?? 'GET') === 'GET') return json({ server_ids: ['srv_old'] });
    return json({});
  };
  return { calls, fetcher };
};

const fillAndConnect = async (host) => {
  typeIn(host.querySelector('[data-ha-address]'), 'homeassistant.local');
  await settle();
  typeIn(host.querySelector('[data-ha-token]'), FAKE_TOKEN);
  await settle();
  host.querySelector('[data-ha-connect]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle(60);
};

test('sheet, empty: House peeks, "Nothing connected yet", Connect waits for both fields', async () => {
  const { fetcher } = fakeServer({ ok: true });
  const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'tools' }, onClose() {}, fetcher }));
  const empty = v.host.querySelector('[data-ha-empty]');
  assert.ok(empty, 'empty strip');
  assert.equal(empty.getAttribute('role'), 'note');
  assert.match(empty.textContent, /Nothing connected yet/);
  assert.ok(empty.querySelector('[data-house-peek]'));
  assert.equal(v.host.querySelector('[data-ha-connect]').disabled, true);
  typeIn(v.host.querySelector('[data-ha-address]'), 'homeassistant.local');
  await settle();
  assert.equal(v.host.querySelector('[data-ha-empty]'), null, 'the strip goes once the address has text');
  v.unmount();
});

test('sheet, team door: Connect builds the URL and Bearer header, earns Tested, and gives the server to the team', async () => {
  localStorage.clear();
  const { calls, fetcher } = fakeServer({ ok: true, tools: [{ name: 'HassTurnOn' }, { name: 'HassTurnOff' }], server_info: { name: 'home-assistant' } });
  const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'team', teamId: 't1' }, onClose() {}, fetcher, teamKwantIds: ['kw_house'] }));
  await fillAndConnect(v.host);
  const post = calls.find((c) => c.url === '/api/mcp/servers' && c.method === 'POST');
  assert.equal(post.body.url, 'http://homeassistant.local:8123/api/mcp');
  assert.equal(post.body.name, 'home-assistant');
  assert.equal(post.body.headers.Authorization, `Bearer ${FAKE_TOKEN}`);
  const put = calls.find((c) => c.url === '/api/kwants/kw_house/mcp-servers' && c.method === 'PUT');
  assert.deepEqual(put.body, { server_ids: ['srv_old', 'srv_ha'] }, 'keeps the bot’s other tools');
  const result = v.host.querySelector('[data-ha-result="ok"]');
  assert.match(result.textContent, /Connected · 2 actions/);
  assert.equal(v.host.querySelectorAll('.kw-sticker-holo').length, 1, 'one holo, on Connected');
  assert.ok(!v.host.innerHTML.includes(FAKE_TOKEN), 'the token is cleared from the page once saved');
  assert.match(v.host.querySelector('[role="img"]')?.getAttribute('aria-label') ?? '', /House, connected/);
  v.unmount();
});

test('sheet: errors are plain words with the next step, and never holo', async () => {
  const cases = [
    [{ ok: false, auth_required: true }, 'bad_token', /didn’t accept this access token/],
    [{ ok: false, error: 'HTTP 404 Not Found' }, 'mcp_off', /MCP Server is off/],
    [{ ok: false, error: 'error sending request: connection refused' }, 'unreachable', /can’t reach homeassistant\.local:8123/],
  ];
  for (const [res, kind, re] of cases) {
    const { calls, fetcher } = fakeServer(res);
    let closed = 0;
    const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'tools' }, onClose: () => closed++, fetcher }));
    await fillAndConnect(v.host);
    const box = v.host.querySelector(`[data-ha-result="${kind}"]`);
    assert.ok(box, kind);
    assert.equal(box.getAttribute('role'), 'alert', kind);
    assert.match(box.textContent, re, kind);
    assert.equal(v.host.querySelectorAll('.kw-sticker-holo').length, 0, `${kind}: no holo`);
    assert.match(v.host.querySelector('[data-ha-connect]').textContent, /Try again/);
    // Leaving after a failure removes the server this sheet made.
    v.host.querySelector('[aria-label="Close"]').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    await settle();
    assert.ok(calls.some((c) => c.url === '/api/mcp/servers/srv_ha' && c.method === 'DELETE'), `${kind}: cleaned up`);
    assert.equal(closed, 1);
    v.unmount();
  }
});

test('sheet, tools door: Connected with zero actions earns no Tested; the house glyph stays (no House avatar)', async () => {
  const { fetcher } = fakeServer({ ok: true, tools: [], server_info: { name: 'home-assistant' } });
  const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'tools' }, onClose() {}, fetcher, homeTeamExists: false, onStartHomeTeam() {} }));
  await fillAndConnect(v.host);
  assert.ok(v.host.querySelector('[data-ha-result="ok"]'));
  assert.equal(v.host.querySelectorAll('.kw-sticker-holo').length, 0);
  assert.equal(v.host.querySelector('[role="img"][aria-label^="House"]'), null);
  assert.ok(v.host.querySelector('[data-ha-start-team]'), 'Start a Home team is offered from Tools');
  v.unmount();
});

// ---- Round 4: QA D1 (token out of the page after every save), the "other" next step, D2 (welcome grid tag)

/** Like fakeServer, but keeps the saved Authorization the way kwant does: the mask keeps what is stored. */
const keepingServer = (testFor, { postStatus = 200 } = {}) => {
  const calls = [];
  let stored = null;
  const fetcher = async (url, init = {}) => {
    const method = init.method ?? 'GET';
    const body = init.body ? JSON.parse(init.body) : null;
    const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
    if (url === '/api/mcp/servers' && method === 'GET') return json(stored ? [{ id: 'srv_ha', name: 'home-assistant', url: stored.url, headers: { Authorization: '********' } }] : []);
    if (url === '/api/mcp/servers' && method === 'POST') {
      calls.push({ url, method, auth: body.headers.Authorization });
      if (postStatus !== 200) return json({ error: 'disk full' }, postStatus);
      stored = { url: body.url, auth: body.headers.Authorization };
      return json({ id: 'srv_ha', name: 'home-assistant' });
    }
    if (url === '/api/mcp/servers/srv_ha' && method === 'PUT') {
      const a = body.headers.Authorization;
      stored = { url: body.url, auth: a === '********' ? stored.auth : a };
      calls.push({ url, method, auth: a, effective: stored.auth });
      return json({});
    }
    if (/\/test$/.test(url)) {
      calls.push({ url, method, auth: stored.auth });
      return json(testFor(stored));
    }
    return json({});
  };
  return { calls, fetcher, setPost: (s) => (postStatus = s) };
};
const click = async (el, n = 60) => {
  el.dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
  await settle(n);
};

test('D1: after a 401 the token field is empty, says Saved, and Try again with it empty reuses the saved Bearer', async () => {
  localStorage.clear();
  const okTools = { ok: true, tools: [{ name: 'HassTurnOn' }], server_info: { name: 'home-assistant' } };
  let addressRight = false;
  const { calls, fetcher } = keepingServer((st) => (addressRight && st.url.includes('192.168.1.20') ? okTools : { ok: false, auth_required: true }));
  const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'tools' }, onClose() {}, fetcher }));
  await fillAndConnect(v.host);
  assert.ok(v.host.querySelector('[data-ha-result="bad_token"]'));
  const field = v.host.querySelector('[data-ha-token]');
  assert.equal(field.value, '', 'the field is cleared after the save');
  assert.equal(field.getAttribute('value') ?? '', '');
  assert.equal(field.type, 'password');
  assert.ok(!v.host.innerHTML.includes(FAKE_TOKEN), 'no token anywhere in the page');
  assert.match(v.host.textContent, /Saved\. Paste a new one to replace it\./);
  // Fix the address, leave the token empty, Try again.
  addressRight = true;
  typeIn(v.host.querySelector('[data-ha-address]'), 'http://192.168.1.20:8123');
  await settle();
  const again = v.host.querySelector('[data-ha-connect]');
  assert.equal(again.disabled, false, 'Try again works with the field empty');
  assert.match(again.textContent, /Try again/);
  await click(again);
  const put = calls.find((c) => c.method === 'PUT');
  assert.equal(put.auth, '********', 'the token is not re-sent from the page');
  assert.equal(put.effective, `Bearer ${FAKE_TOKEN}`, 'the saved Bearer is kept');
  assert.equal(calls.filter((c) => c.url === '/api/mcp/servers' && c.method === 'POST').length, 1, 'same server, no duplicate');
  assert.equal(calls.at(-1).auth, `Bearer ${FAKE_TOKEN}`, 'the test ran with the saved Bearer');
  assert.ok(v.host.querySelector('[data-ha-result="ok"]'));
  v.unmount();
});

test('D1: if saving fails the field still clears; the token is kept off the page for Try again', async () => {
  const { calls, fetcher, setPost } = keepingServer(() => ({ ok: true, tools: [{ name: 'HassTurnOn' }], server_info: { name: 'home-assistant' } }), { postStatus: 500 });
  const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'tools' }, onClose() {}, fetcher }));
  await fillAndConnect(v.host);
  assert.ok(v.host.querySelector('[data-ha-result="other"]'));
  assert.equal(v.host.querySelector('[data-ha-token]').value, '');
  assert.ok(!v.host.innerHTML.includes(FAKE_TOKEN));
  assert.match(v.host.textContent, /Kept for the next try\. Paste a new one to replace it\./);
  setPost(200);
  await click(v.host.querySelector('[data-ha-connect]'));
  assert.equal(calls.filter((c) => c.url === '/api/mcp/servers' && c.method === 'POST').at(-1).auth, `Bearer ${FAKE_TOKEN}`, 'the held token is sent once more');
  assert.ok(v.host.querySelector('[data-ha-result="ok"]'));
  assert.match(v.host.textContent, /Saved\. Paste a new one/);
  v.unmount();
});

test('"other": the plain title, one next step, then the server text as a quiet detail', async () => {
  const { fetcher } = fakeServer({ ok: false, error: 'invalid response: error decoding response body' });
  const v = await mount(h(m.HomeAssistantSheet, { req: { door: 'tools' }, onClose() {}, fetcher }));
  await fillAndConnect(v.host);
  const box = v.host.querySelector('[data-ha-result="other"]');
  assert.ok(box);
  assert.equal(box.getAttribute('role'), 'alert');
  assert.match(box.textContent, /Couldn’t connect to Home Assistant\./);
  assert.equal(box.querySelector('[data-ha-next]').textContent, 'Check that the address opens your Home Assistant in a browser, then try again.');
  assert.equal(box.querySelector('[data-ha-detail]').textContent, 'invalid response: error decoding response body');
  assert.ok(box.textContent.indexOf('Check that') < box.textContent.indexOf('invalid response'), 'next step before the detail');
  assert.equal(box.querySelectorAll('.kw-sticker-holo').length, 0);
  v.unmount();
});

test('D2: the welcome grid’s Home card has the home-team New tag; picking it clears the tag', async () => {
  localStorage.clear();
  const realFetch = g.fetch;
  g.fetch = g.window.fetch = async (url) =>
    new Response(JSON.stringify(String(url).startsWith('/api/team-templates?') ? { templates: m.TEMPLATES_WIRE } : {}), { status: 200, headers: { 'Content-Type': 'application/json' } });
  try {
    const ids = m.TEMPLATES_WIRE.map((t) => t.id);
    assert.ok(ids.includes('home'));
    const view = { title: 'Pick a team', intro: 'Pick one.', start: null, templateIds: ids };
    const v = await mount(
      h(m.ReadyTeamsProvider, { space: 'spc_demo', bots: [], teams: [], onChanged() {}, onOpenTeam() {} },
        h(m.TeamStep, { phone: false, spaceName: 'Main', mainSpace: true, botNames: [], onKeep() {}, onPicked() {}, onImported() {}, view })),
    );
    const home = v.host.querySelector('[data-template="home"]');
    assert.ok(home, 'Home card in the grid');
    assert.ok(home.querySelector('[data-new-tag="home-team"]'), 'New on the Home card');
    assert.equal(v.host.querySelectorAll('[data-new-tag]').length, 1, 'only on Home');
    await click(home);
    assert.equal(m.isNewThing('home-team'), false, 'seen once picked');
    assert.equal(v.host.querySelector('[data-template="home"] [data-new-tag]:not([data-leaving])'), null);
    v.unmount();
  } finally {
    g.fetch = g.window.fetch = realFetch;
  }
});
