// 33k long-chat-loads-earlier (autonomy/long-chat-loads-earlier/brief.md): a long chat no longer stops at its newest
// 100 messages. "Earlier messages" loads the page before the first one (before=<id>&limit=100) until the chat's true
// start, which reads "Start of your chat with <bot> · <date>". A refresh after a new reply keeps the pages already
// loaded. A chat under 100 messages shows the start line and no button. Real SpaceApp in jsdom with a faked server
// that pages like routes.rs messages_page. Made-up words only.
import assert from 'node:assert/strict';
import test, { afterEach } from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_default/b/k1', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
/** The app's live connection; the test sends it an event that reloads the open chat. */
const sources = [];
class FakeEvents {
  constructor() {
    sources.push(this);
  }
  addEventListener() {}
  removeEventListener() {}
  close() {}
}
g.EventSource = g.window.EventSource = FakeEvents;
// jsdom has no CSS.escape; ChatView uses it to find a message row (ids here are plain).
g.CSS ??= { escape: (v) => String(v) };

const bot = (id, name) => ({ id, name, title: 'Analyst', persona_prompt: '', avatar_sprite: 'mouse', driver_type: 'claude', preferred_model: '' });
const kwants = [bot('k1', 'Ada'), bot('k2', 'Lin')];
const at = (i) => new Date(Date.UTC(2026, 9, 3, 9, 0, 0) + i * 60_000).toISOString();
/** A direct chat with Ada of `n` messages, oldest first. */
const seed = (n) =>
  Array.from({ length: n }, (_, i) => ({
    id: `m${String(i).padStart(4, '0')}`,
    direct_kwant_id: 'k1',
    sender_role: i % 2 ? 'kwant' : 'user',
    sender_id: i % 2 ? 'k1' : 'user',
    text: i % 2 ? `Answer ${i} about the autumn numbers` : `Question ${i} about the autumn numbers`,
    created_at: at(i),
  }));
const chats = { k1: seed(250) };
const asked = [];
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
/** routes.rs messages_page: the latest `limit` (100), or with before=<id> the `limit` before it, oldest first. */
const page = (id, params) => {
  const all = chats[id] ?? [];
  const limit = Number(params.get('limit') ?? 100);
  const before = params.get('before');
  if (!before) return json(all.slice(-limit));
  const i = all.findIndex((m) => m.id === before);
  if (i < 0) return json({ error: 'not found' }, 404);
  return json(all.slice(Math.max(0, i - limit), i));
};
const routes = {
  '/api/kwants': kwants,
  '/api/channels': [{ id: 'ch1', name: 'general', kwant_ids: ['k1', 'k2'], created_at: '' }],
  '/api/teams': [],
  '/api/engines': [{ id: 'claude', kind: 'cli', enabled: true, status: 'ready', ready: true }],
  '/api/automation-discovery': { setting: 'off', daily_cap: 3, blocked_by: [], last_scan_at: null, suggestions: [], stats: {} },
};
g.fetch = g.window.fetch = async (url, init) => {
  const u = new URL(String(url), 'http://localhost');
  const msgs = u.pathname.match(/^\/api\/channels\/([^/]+)\/messages$/);
  if (msgs) {
    asked.push(u.pathname + u.search);
    return page(decodeURIComponent(msgs[1]), u.searchParams);
  }
  return json(u.pathname in routes ? routes[u.pathname] : init?.method === 'PUT' ? {} : []);
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { SpaceApp } from './src/App';
  export { mergeLatest, prependOlder } from './src/chatPages';
`);
const { React } = m;
const noop = () => {};
const settle = async (n = 40) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const space = { id: 'spc_default', name: 'Default', icon: '💼', color: '#f59e0b', sort: 0, is_default: true, archived_at: null,
  created_at: '', general_channel_id: 'ch1', counts: { bots: 2, ready_accounts: 1, unread: 0 } };

const mounted = new Set();
afterEach(() => {
  for (const unmount of mounted) unmount();
  document.body.replaceChildren();
});
const openAda = async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  root.render(React.createElement(m.SpaceApp, {
    space, spaces: [space], initialTarget: { botId: 'k1' }, onSwitchSpace: noop, onCreateSpace: noop, onOpenArchived: noop,
    onSpacesChanged: noop, onSpaceArchived: noop, onChatChange: noop,
  }));
  await settle();
  const unmount = () => {
    if (!mounted.delete(unmount)) return;
    root.unmount();
    host.remove();
  };
  mounted.add(unmount);
  return unmount;
};
const log = () => document.querySelector('[role="log"]');
const shownIds = () => [...log().querySelectorAll('[data-message-id]')].map((el) => el.getAttribute('data-message-id')).filter((id) => /^m\d{4}$/.test(id));
const earlier = () => [...log().querySelectorAll('button')].find((b) => b.textContent.trim() === 'Earlier messages');
const startLine = () => log().querySelector('[data-chat-start]');
const click = async (el) => {
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await settle();
};

test('33k: a 250-message chat loads its earlier pages until the start line shows', async () => {
  chats.k1 = seed(250);
  asked.length = 0;
  await openAda();
  assert.ok(log(), 'the chat with Ada is open');
  assert.equal(shownIds().length, 100, 'the newest 100 first');
  assert.equal(shownIds()[0], 'm0150');
  assert.equal(startLine(), null, 'not the start yet');

  const btn = earlier();
  assert.ok(btn, 'an "Earlier messages" button sits at the top of a long chat');
  assert.equal(btn.getAttribute('type'), 'button');
  await click(btn);
  assert.ok(asked.includes('/api/channels/k1/messages?before=m0150&limit=100'), `asked for the page before the first message (asked: ${asked.join(', ')})`);
  assert.equal(shownIds().length, 200, 'one more page');
  assert.equal(shownIds()[0], 'm0050');

  await click(earlier());
  assert.ok(asked.includes('/api/channels/k1/messages?before=m0050&limit=100'));
  const ids = shownIds();
  assert.equal(ids.length, 250, 'the whole chat');
  assert.equal(ids[0], 'm0000');
  assert.equal(new Set(ids).size, 250, 'no message twice');
  assert.deepEqual(ids, [...ids].sort(), 'oldest first');
  assert.equal(earlier(), undefined, 'no button at the true start');
  const line = startLine();
  assert.ok(line, 'the start line shows');
  assert.match(line.textContent, /^Start of your chat with Ada · /);
  assert.match(line.textContent, /Oct 3, 2026/, 'with the date of the first message');
});

test('33k: a refresh after a new reply keeps the earlier pages already loaded', async () => {
  chats.k1 = seed(250);
  await openAda();
  await click(earlier());
  await click(earlier());
  assert.equal(shownIds().length, 250);

  // A new reply lands on the server and the bot's decision settles, which reloads the open chat's latest page.
  chats.k1 = [...chats.k1, { id: 'm0250', direct_kwant_id: 'k1', sender_role: 'kwant', sender_id: 'k1', text: 'A new answer', created_at: at(250) }];
  asked.length = 0;
  sources.at(-1).onmessage({ data: JSON.stringify({ type: 'decide_updated', data: { kwant_id: 'k1', context_id: 'k1', state: 'done' } }) });
  await settle();
  assert.ok(asked.includes('/api/channels/k1/messages'), `the latest page was reloaded (asked: ${asked.join(', ')})`);
  const ids = shownIds();
  assert.equal(ids.at(-1), 'm0250', 'the new reply shows');
  assert.equal(ids.length, 251, 'and every earlier page stays');
  assert.equal(ids[0], 'm0000');
  assert.equal(new Set(ids).size, 251, 'no message twice');
  assert.ok(startLine(), 'still at the start');
});

test('33k: a chat under 100 messages shows the start line and no button', async () => {
  chats.k1 = seed(12);
  asked.length = 0;
  await openAda();
  assert.equal(shownIds().length, 12);
  assert.equal(earlier(), undefined, 'no "Earlier messages"');
  assert.match(startLine()?.textContent ?? '', /^Start of your chat with Ada · Oct 3, 2026$/);
  assert.ok(!asked.some((a) => a.includes('before=')), 'never asks for an older page');
});

test('33k: merging pages keeps each message once, and a latest page with a gap before it replaces the old pages', () => {
  const msgs = seed(400);
  const cached = msgs.slice(100, 300); // two pages loaded
  const latest = msgs.slice(205, 305); // five new messages since
  const merged = m.mergeLatest(cached, latest);
  assert.deepEqual(merged.map((x) => x.id), msgs.slice(100, 305).map((x) => x.id), 'older page kept, new ones added, no repeats');
  // More than a page arrived meanwhile: nothing joins the cache, so the latest page alone (earlier ones load again).
  const far = msgs.slice(300, 400);
  assert.equal(m.mergeLatest(cached, far), far);
  // A short page is the whole chat (deletions included).
  const short = msgs.slice(0, 40);
  assert.equal(m.mergeLatest(msgs.slice(0, 41), short), short);
  // An earlier page in front, with an overlap, keeps each id once.
  assert.deepEqual(m.prependOlder(msgs.slice(100, 200), msgs.slice(0, 101)).map((x) => x.id), msgs.slice(0, 200).map((x) => x.id));
});
