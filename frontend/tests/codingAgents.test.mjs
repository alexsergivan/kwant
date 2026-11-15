// Settings → Coding agents (0.7.4): connections for Claude Code, Codex and Cursor. The token is shown
// once, with setup snippets that match docs/agents-connect.md, and is never kept anywhere after.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

// The DOM exists before react-dom loads, so its event system is on.
const dom = new JSDOM('<!doctype html><div id="root"></div>', { url: 'https://kwant.example/settings', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'localStorage', 'sessionStorage']) {
  Object.defineProperty(globalThis, k, { value: k === 'window' ? dom.window : dom.window[k], configurable: true, writable: true });
}
globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  import React from 'react';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ConnectionRow, NewConnectionForm, TokenReveal, ActivityList, CodingAgents } from './src/components/CodingAgents';
  export * from './src/agentConnections';
  export { React, CodingAgents };
  export { createRoot } from 'react-dom/client';
  const noop = () => {};
  export const row = (props) => renderToStaticMarkup(React.createElement(ConnectionRow, {
    spaceName: (id) => ({ spc_default: 'Work', spc_home: 'Home' })[id], botName: (id) => ({ kw_1: 'Vega', kw_2: 'Lin' })[id],
    confirming: false, revoking: false, onAskRevoke: noop, onKeep: noop, onRevoke: noop, activity: null, onToggleActivity: noop, onRetryActivity: noop, ...props }));
  export const form = (props) => renderToStaticMarkup(React.createElement(NewConnectionForm, { spaces: [], busy: false, error: null, onCreate: noop, onCancel: noop, ...props }));
  export const reveal = (props) => renderToStaticMarkup(React.createElement(TokenReveal, { name: 'Claude Code', token: 'kag_x', origin: 'https://kwant.example', onDone: noop, ...props }));
  export const activity = (state, now) => renderToStaticMarkup(React.createElement(ActivityList, { state, now, onRetry: noop }));
`);

const text = (html) => html.replace(/<[^>]+>/g, ' ').replace(/&#x27;|&#39;|’/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const TOKEN = 'kag_' + 'A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8S9t0U1v'.slice(0, 43);
const conn = (o = {}) => ({ id: 'agc_1', name: 'Claude Code', space_ids: ['spc_default'], bots: 'all', created_at: '2026-11-07T00:10:00Z', last_used_at: null, ...o });

/** A fetch that answers each call from `answers` in turn ([status, body]) and records the calls. */
function fake(...answers) {
  const calls = [];
  const send = async (url, init = {}) => {
    calls.push({ url, init });
    const next = answers.shift() ?? [500, null];
    const [status, body] = typeof next === 'function' ? next(url, init) : next;
    return new Response(body === null ? null : JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  };
  return { calls, send };
}

// ---------------------------------------------------------------- API

test('list: live connections and the max; a missing max is 10, empty bots read as all', async () => {
  const f = fake([200, { connections: [conn(), conn({ id: 'agc_2', bots: [] }), { junk: 1 }], max: 10 }]);
  const l = await m.agentConnectionsApi(f.send).list();
  assert.equal(f.calls[0].url, '/api/agent-connections');
  assert.equal(l.max, 10);
  assert.equal(l.connections.length, 2);
  assert.equal(l.connections[1].bots, 'all');
  const g = fake([200, { connections: [] }]);
  assert.equal((await m.agentConnectionsApi(g.send).list()).max, 10);
});

test('create posts name, spaces and bots, and returns the token once', async () => {
  const f = fake([201, { connection: conn(), token: TOKEN }]);
  const out = await m.agentConnectionsApi(f.send).create({ name: 'Claude Code', space_ids: ['spc_default'], bots: 'all' });
  assert.equal(f.calls[0].init.method, 'POST');
  assert.deepEqual(JSON.parse(f.calls[0].init.body), { name: 'Claude Code', space_ids: ['spc_default'], bots: 'all' });
  assert.equal(out.token, TOKEN);
  assert.equal(out.connection.id, 'agc_1');
});

test('409 reads as the plain 10-connection line; 400 shows the server sentence as is', async () => {
  const api409 = m.agentConnectionsApi(fake([409, { error: 'You already have 10 coding agents connected. Revoke one first.' }]).send);
  await assert.rejects(api409.create({ name: 'x', space_ids: ['s'], bots: 'all' }), (e) => e.status === 409 && e.message === m.LIMIT_TEXT);
  const bare409 = m.agentConnectionsApi(fake([409, null]).send);
  await assert.rejects(bare409.create({ name: 'x', space_ids: ['s'], bots: 'all' }), (e) => e.message === m.LIMIT_TEXT);
  const api400 = m.agentConnectionsApi(fake([400, { error: 'every bot in bots must be in one of the connection\'s spaces' }]).send);
  await assert.rejects(api400.create({ name: 'x', space_ids: ['s'], bots: ['kw_9'] }), (e) => e.status === 400 && /every bot in bots/.test(e.message));
});

test('a create answer without a kag_ token is an error, not an empty token view', async () => {
  const api = m.agentConnectionsApi(fake([201, { connection: conn() }]).send);
  await assert.rejects(api.create({ name: 'x', space_ids: ['s'], bots: 'all' }), /Couldn’t connect it/);
});

test('revoke deletes by id; activity reads calls; a dropped network reads plainly', async () => {
  const f = fake([200, { id: 'agc_1', revoked: true }], [200, { calls: [{ connection_id: 'agc_1', connection_name: 'Claude Code', tool: 'ask', bot: 'Vega', outcome: 'ok', at: '2026-11-07T00:12:03Z' }] }]);
  const api = m.agentConnectionsApi(f.send);
  await api.revoke('agc_1');
  assert.equal(f.calls[0].url, '/api/agent-connections/agc_1');
  assert.equal(f.calls[0].init.method, 'DELETE');
  const calls = await api.activity('agc_1');
  assert.equal(f.calls[1].url, '/api/agent-connections/agc_1/activity');
  assert.equal(calls.length, 1);
  const off = m.agentConnectionsApi(async () => {
    throw new TypeError('Failed to fetch');
  });
  await assert.rejects(off.list(), /Couldn’t reach the server/);
  await assert.rejects(m.agentConnectionsApi(fake([404, { error: 'no such connection' }]).send).revoke('agc_x'), (e) => e.status === 404);
});

// ---------------------------------------------------------------- the form's rules

test('name rules match the server: trimmed 1–40, no line breaks, [ ] or < >', () => {
  assert.equal(m.nameError('  Claude Code  '), null);
  assert.match(m.nameError('   '), /Give it a name/);
  assert.match(m.nameError('x'.repeat(41)), /40 characters/);
  assert.equal(m.nameError('x'.repeat(40)), null);
  assert.match(m.nameError('a\nb'), /line breaks/);
  assert.match(m.nameError('a [b]'), /\[ \]/);
  assert.match(m.nameError('<b>'), /< >/);
  assert.deepEqual(m.createBody(' Cursor ', ['a', 'a', 'b'], true, []), { name: 'Cursor', space_ids: ['a', 'b'], bots: 'all' });
  assert.equal(m.createBody('Cursor', [], true, []), null);
  assert.equal(m.createBody('Cursor', ['a'], false, []), null);
  assert.deepEqual(m.createBody('Cursor', ['a'], false, ['kw_1', 'kw_1']).bots, ['kw_1']);
});

test('the new-connection form: name, spaces, all bots or some; Create waits for a name', () => {
  const spaces = [{ id: 'spc_default', name: 'Work', bots: [{ id: 'kw_1', name: 'Vega' }] }, { id: 'spc_home', name: 'Home', bots: [] }];
  const html = m.form({ spaces, initialSpace: 'spc_home' });
  const t = text(html);
  for (const s of ['Name', 'Spaces it can reach', 'Work', 'Home', 'Bots it can ask', 'All bots in these spaces', 'Only the bots I pick', 'Create', 'Cancel']) assert.ok(t.includes(s), s);
  assert.match(html, /<button type="submit" disabled=""/);
  // The current space starts ticked.
  assert.match(html, /checked=""[^>]*\/?>Home|Home/);
  const err = text(m.form({ spaces, error: 'You already have 10 coding agents connected. Revoke one first.' }));
  assert.ok(err.includes('You already have 10 coding agents connected. Revoke one first.'));
});

// ---------------------------------------------------------------- snippets match the docs

test('setup snippets are the docs’ blocks, with this server and the new token filled in', () => {
  const doc = readFileSync(new URL('../../docs/agents-connect.md', import.meta.url), 'utf8');
  const block = (heading) => {
    const at = doc.indexOf(`### ${heading}`);
    assert.ok(at >= 0, heading);
    const fence = doc.slice(at).match(/```[a-z]*\n([\s\S]*?)\n```/);
    return fence[1];
  };
  const docs = { claude: block('Claude Code'), codex: block('Codex'), cursor: block('Cursor') };
  for (const t of ['claude', 'codex', 'cursor']) assert.equal(m.setupSnippets(t, m.DOC_SERVER, m.DOC_TOKEN)[0], docs[t], t);
  assert.ok(doc.includes('`' + m.setupSnippets('codex', m.DOC_SERVER, m.DOC_TOKEN)[1] + '`'), 'the Codex export line');
  const filled = m.setupSnippets('cursor', m.serverUrl('https://kwant.example/'), TOKEN)[0];
  assert.ok(filled.includes('"url": "https://kwant.example/api/mcp/agents"'));
  assert.ok(filled.includes('Bearer ${env:KWANT_AGENT_TOKEN}') && !filled.includes(TOKEN), 'the config file holds no token');
  assert.equal(m.setupSnippets('cursor', m.DOC_SERVER, TOKEN)[1], `export KWANT_AGENT_TOKEN=${TOKEN}`);
  assert.equal(m.serverUrl('javascript:alert(1)'), m.DOC_SERVER);
  assert.equal(m.serverUrl(null), m.DOC_SERVER);
});

test('the token view: the token, Copy, the warning line, three tools, and Done', () => {
  const html = m.reveal({ token: TOKEN });
  const t = text(html);
  assert.ok(t.includes('Claude Code is connected. Copy its token now.'));
  assert.ok(t.includes(TOKEN));
  assert.ok(t.includes('Shown once. Anyone with it can ask these bots for work, like a password.'));
  for (const s of ['Claude Code', 'Codex', 'Cursor', 'Done, I\'ve saved it']) assert.ok(t.includes(s), s);
  assert.ok(t.includes('claude mcp add --transport http kwant https://kwant.example/api/mcp/agents'));
  assert.ok(!/MCP|endpoint|token budget/.test(t.replace(/claude mcp add/g, '').replace(/mcpServers|mcp\.json|mcp_servers|api\/mcp/g, '')), 'no jargon in the words around the snippets');
});

// ---------------------------------------------------------------- rows and activity

test('a row: name, spaces and bots, the added date, and "Never" before first use', () => {
  const t = text(m.row({ c: conn(), now: Date.parse('2026-11-07T01:00:00Z') }));
  assert.ok(t.includes('Claude Code'));
  assert.ok(t.includes('Work · All bots'));
  assert.ok(t.includes('Added Nov 7, 2026'));
  assert.ok(t.includes('Last used Never'));
  assert.ok(t.includes('Revoke'));
  assert.ok(t.includes('Recent activity'));
  const some = text(m.row({ c: conn({ space_ids: ['spc_default', 'spc_home'], bots: ['kw_1', 'kw_2'], last_used_at: '2026-11-07T00:48:00Z' }), now: Date.parse('2026-11-07T01:00:00Z') }));
  assert.ok(some.includes('Work and Home · Vega and Lin'));
  assert.ok(some.includes('Last used 12 min ago'));
  assert.ok(!some.includes('agc_1'), 'ids stay out of what the person reads');
});

test('Revoke takes one confirm', () => {
  const t = text(m.row({ c: conn(), confirming: true }));
  assert.ok(t.includes('Revoke Claude Code? Its token stops working at once. Its threads stay.'));
  assert.ok(t.includes('Keep'));
});

test('recent activity reads as plain lines, refused ones marked', () => {
  const now = new Date('2026-11-07T01:00:00Z');
  const calls = [
    { tool: 'ask', bot: 'Vega', outcome: 'ok', at: '2026-11-07T00:12:03Z' },
    { tool: 'list_bots', bot: null, outcome: 'ok', at: '2026-11-07T00:11:00Z' },
    { tool: 'ask', bot: 'Lin', outcome: 'refused', at: '2026-11-07T00:10:00Z' },
  ];
  const t = text(m.activity({ kind: 'ready', calls }, now));
  assert.ok(t.includes('Asked Vega'));
  assert.ok(t.includes('Looked up the bots'));
  assert.ok(t.includes('Asked Lin, refused'));
  assert.ok(text(m.activity({ kind: 'ready', calls: [] }, now)).includes('Nothing yet.'));
});

test('a coding agent’s message is labelled as the agent, not as you', () => {
  assert.equal(m.agentSenderLabel(JSON.stringify({ from_agent: 'Claude Code', agent_connection_id: 'agc_1' })), 'Claude Code (coding agent)');
  assert.equal(m.agentSenderLabel(null), null);
  assert.equal(m.agentSenderLabel('{}'), null);
  assert.equal(m.agentSenderLabel('not json'), null);
});

// ---------------------------------------------------------------- the whole card in a DOM

test('create → token shown once → Done forgets it; nothing lands in storage, the URL or the console', async () => {
  const logged = [];
  const orig = { log: console.log, info: console.info, warn: console.warn, error: console.error, debug: console.debug };
  for (const k of Object.keys(orig)) console[k] = (...a) => logged.push(a.map(String).join(' '));
  try {
    let live = [];
    const send = async (url, init = {}) => {
      const reply = (status, body) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
      if (url === '/api/spaces') return reply(200, [{ id: 'spc_default', name: 'Work', archived_at: null }]);
      if (url.startsWith('/api/kwants')) return reply(200, [{ id: 'kw_1', name: 'Vega' }]);
      if (url === '/api/agent-connections' && init.method === 'POST') {
        const c = { ...conn({ id: 'agc_9' }), name: JSON.parse(init.body).name };
        live = [c, ...live];
        return reply(201, { connection: c, token: TOKEN });
      }
      if (url === '/api/agent-connections') return reply(200, { connections: live, max: 10 });
      return reply(404, { error: 'no such connection' });
    };
    const root = m.createRoot(dom.window.document.getElementById('root'));
    const flush = () => new Promise((r) => setTimeout(r, 20));
    const act = async (fn) => {
      await fn();
      await flush();
    };
    await act(async () => {
      root.render(m.React.createElement(m.CodingAgents, { send, origin: 'https://kwant.example' }));
      await flush();
    });
    const $ = (sel) => dom.window.document.querySelector(sel);
    await act(async () => {
      $('[data-agent-add]').click();
      await flush();
    });
    const input = $('[data-agent-new] input');
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, 'Claude Code');
      input.dispatchEvent(new dom.window.Event('input', { bubbles: true }));
      await flush();
    });
    await act(async () => {
      $('[data-agent-new]').dispatchEvent(new dom.window.Event('submit', { bubbles: true, cancelable: true }));
      await flush();
    });
    assert.equal($('[data-agent-token]')?.textContent, TOKEN, 'the token is shown');
    assert.equal(dom.window.document.activeElement?.id, 'agent-token-h', 'focus moves to the token view');
    assert.ok($('[data-agent-row="agc_9"]'), 'the new row is listed');
    assert.ok(!$('[data-agent-row="agc_9"]').textContent.includes('kag_'), 'not in the row');
    await act(async () => {
      $('[data-agent-token-done]').click();
      await flush();
    });
    const html = dom.window.document.body.innerHTML;
    assert.ok(!html.includes(TOKEN), 'gone from the page after Done');
    assert.ok(!html.includes('kag_'), 'no token fragment either');
    assert.ok(!dom.window.location.href.includes('kag_'));
    for (const s of [dom.window.localStorage, dom.window.sessionStorage]) {
      for (let i = 0; i < s.length; i++) assert.ok(!String(s.getItem(s.key(i))).includes('kag_'));
    }
    assert.ok(!logged.some((l) => l.includes('kag_')), 'never logged');
    await act(async () => root.unmount());
  } finally {
    Object.assign(console, orig);
  }
});
