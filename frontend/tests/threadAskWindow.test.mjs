// 22x (brief: autonomy/thread-ask-lands-when-old/brief.md): a Needs-attention row whose notice sits in a
// thread, older than the thread's latest page (Rigel's F1 on 22i: 150+ messages, the notice near the top),
// loads the window around it from the thread's own route, shows it, and rings it, like a channel or 1:1.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame']) {
  if (k in dom.window) Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
dom.window.HTMLElement.prototype.scrollIntoView = function () {};
dom.window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
if (!('ResizeObserver' in dom.window)) dom.window.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
g.ResizeObserver = dom.window.ResizeObserver;
if (!g.CSS) g.CSS = { escape: (s) => String(s).replace(/["\\]/g, '\\$&') };

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export * from './src/askHere';
  export { ChatView } from './src/components/ChatView';
`);

// Rigel's case: a thread with 160 messages; the latest page is the newest 100, the notice is the 4th.
const thread = Array.from({ length: 160 }, (_, i) => ({
  id: `m${i}`,
  thread_id: 'th-1',
  channel_id: null,
  direct_kwant_id: null,
  sender_role: i % 2 ? 'user' : 'kwant',
  sender_id: i % 2 ? 'me' : 'Quant',
  text: i === 3 ? 'Stopped: I need you to sign in to GitHub' : `t${i}`,
  created_at: new Date(Date.UTC(2026, 10, 16, 8, 0, i)).toISOString(),
  tokens_used: 0,
}));
const latestPage = thread.slice(60);
const notice = 'm3';
const target = { channel_id: null, kwant_id: 'k-quant', thread_id: 'th-1', focus: notice, focusKind: 'message' };

/** A server: the thread route with `around` (22x) or, for `old`, one that ignores it (before 22x). */
const server = ({ old = false } = {}) => {
  const calls = [];
  const get = async (url) => {
    calls.push(url);
    const u = new URL(url, 'http://localhost');
    const around = u.searchParams.get('around');
    if (u.pathname !== '/api/threads/th-1/messages') return { ok: false, json: async () => ({ error: 'thread not found' }) };
    if (old || !around) return { ok: true, json: async () => latestPage };
    const i = thread.findIndex((x) => x.id === around);
    if (i < 0) return { ok: false, json: async () => ({ error: "That message isn't in this chat." }) };
    return { ok: true, json: async () => thread.slice(Math.max(0, i - 50), i + 50) };
  };
  return { calls, get };
};

test('a thread notice older than the latest page: loaded from the thread route, around the notice', async () => {
  assert.ok(!latestPage.some((x) => x.id === notice), 'the notice is not on the latest page');
  const s = server();
  const w = await m.loadAskWindow(target, notice, s.get);
  assert.deepEqual(s.calls, ['/api/threads/th-1/messages?around=m3']);
  assert.equal(w.ctx, 'th-1', 'the window belongs to the thread, so it shows while the thread is open');
  assert.ok(w.messages.some((x) => x.id === notice));
  assert.deepEqual(w.messages.map((x) => x.id), thread.slice(0, 53).map((x) => x.id));
});

test('channels and 1:1s keep the chat route; ids are encoded', () => {
  assert.deepEqual(m.askWindowUrl({ channel_id: 'c 1', kwant_id: 'k', thread_id: null }, 'a/b'), {
    ctx: 'c 1',
    url: '/api/channels/c%201/messages?around=a%2Fb',
  });
  assert.deepEqual(m.askWindowUrl({ channel_id: null, kwant_id: 'k-1', thread_id: null }, 'x'), {
    ctx: 'k-1',
    url: '/api/channels/k-1/messages?around=x',
  });
  assert.equal(m.askWindowUrl(target, notice).url, '/api/threads/th-1/messages?around=m3');
});

test('nothing to show: a 404, a failed load, or an older server that sends the latest page without the notice', async () => {
  assert.equal(await m.loadAskWindow(target, 'gone', server().get), null);
  assert.equal(await m.loadAskWindow({ ...target, thread_id: 'th-gone' }, notice, server().get), null);
  assert.equal(await m.loadAskWindow(target, notice, async () => { throw new Error('offline'); }), null);
  assert.equal(await m.loadAskWindow(target, notice, server({ old: true }).get), null);
  assert.equal(await m.loadAskWindow(target, notice, async () => ({ ok: true, json: async () => ({ error: 'x' }) })), null);
});

test('the window, shown in the thread pane, rings the notice as an ask', async () => {
  const w = await m.loadAskWindow(target, notice, server().get);
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  const props = {
    title: 'Quant', subtitle: 'Thread', messages: w.messages, streamingText: '', streamingThinking: '', isStreaming: false,
    pendingApprovals: [], onSendMessage: () => {}, onResolveApproval: () => {}, kwants: [], teams: [],
    highlightMessageId: notice, highlightRing: true, onHighlightDone: () => {},
  };
  m.flushSync(() => root.render(m.React.createElement(m.ChatView, props)));
  let el = null;
  for (let i = 0; i < 20 && !el?.querySelector('[data-flash-kind="ask"]') && el?.getAttribute('data-flash-kind') !== 'ask'; i++) {
    await new Promise((r) => setTimeout(r, 10));
    el = host.querySelector(`[data-message-id="${notice}"]`);
  }
  assert.ok(el, 'the notice is rendered from the window');
  const flagged = el.getAttribute('data-flash-kind') === 'ask' ? el : el.querySelector('[data-flash-kind="ask"]');
  assert.ok(flagged, 'the notice carries the ask ring');
  root.unmount();
  host.remove();
});

test('findAsk no longer gives up on threads: it loads the window, shows it while that chat is open, and rings', () => {
  const src = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8');
  const body = src.slice(src.indexOf('const findAsk = async'), src.indexOf('const awayLanding ='));
  assert.ok(body.length > 0);
  assert.doesNotMatch(body, /t\.thread_id\)\s*return false/, 'threads are not skipped');
  assert.match(body, /if \(quiet\) return false;/, 'the strip\'s quiet check still never fetches');
  assert.match(body, /loadAskWindow<ChatMessage>\(t, focus\)/);
  assert.match(body, /openCtxRef\.current !== w\.ctx\) return false;/);
  assert.match(body, /setHistoryWindow\(w\);\s*return ring\(\);/);
});
