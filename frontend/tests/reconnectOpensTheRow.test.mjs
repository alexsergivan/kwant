// 34h reconnect-opens-the-row (brief: autonomy/reconnect-opens-the-row/brief.md; Mira's
// ux-pass-0942-connections/report.md section 2). A chat line's or the Inbox's "Sign in again" opens
// Settings › Apps & accounts › Advanced on that connection's row, lit for 1.2 s, and the row has an
// amber "Sign in again" (a URL tool server whose sign-in ran out) that runs the same sign-in as a
// bot's suggestion card (mcp.ts signInToServer).
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLButtonElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'MutationObserver', 'DOMParser', 'localStorage', 'sessionStorage', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;
let reduced = false;
const scrolls = [];
dom.window.matchMedia = (q) => ({ matches: q.includes('reduce') ? reduced : false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
globalThis.matchMedia = dom.window.matchMedia;
// The row is centred in its own scroller (the host below); nothing else scrolls.
dom.window.HTMLElement.prototype.scrollIntoView = function (o) {
  scrolls.push({ intoView: this.getAttribute('data-connection'), ...o });
};
dom.window.HTMLElement.prototype.scrollTo = function (o) {
  scrolls.push({ scroller: this.getAttribute('data-scroller'), behavior: o.behavior });
};
if (!globalThis.CSS) globalThis.CSS = { escape: (s) => String(s) };
globalThis.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} };

const ui = await loadBundled(`
  import React from 'react';
  import { flushSync } from 'react-dom';
  import { createRoot } from 'react-dom/client';
  import { renderToStaticMarkup } from 'react-dom/server.browser';
  import { ConnectionsView, ConnectionRow, canSignInAgain, ROW_FLASH_MS } from './src/components/ConnectionsView';
  import { ToolFailureLine } from './src/components/ToolFailureLine';
  import { parseToolFailureMeta } from './src/toolFailures';
  export * as mcp from './src/mcp';
  export { scrollInScroller } from './src/scrollInScroller';
  export { canSignInAgain, ROW_FLASH_MS, parseToolFailureMeta };
  export const row = (props) => renderToStaticMarkup(React.createElement('ul', null, React.createElement(ConnectionRow, props)));
  export const mount = (C, p) => {
    const host = document.createElement('div');
    host.setAttribute('data-scroller', 'settings');
    host.style.overflowY = 'auto';
    Object.defineProperty(host, 'scrollHeight', { value: 2000 });
    Object.defineProperty(host, 'clientHeight', { value: 600 });
    document.body.appendChild(host);
    const root = createRoot(host);
    const render = (q) => flushSync(() => root.render(React.createElement(C === 'view' ? ConnectionsView : ToolFailureLine, q)));
    render(p);
    return { host, render, unmount: () => { flushSync(() => root.unmount()); host.remove(); } };
  };
`);

const conn = (over = {}) => ({
  id: 'mcp:s1', kind: 'mcp', name: 'Tasklane', detail: 'https://tasklane.example/mcp', status: 'needs_sign_in', status_detail: 'Sign-in required',
  last_check: null, used_by: [{ id: 'k1', name: 'Ben' }], logins_stored: 'kwnt', manage: '/api/mcp/servers/s1', ...over,
});
const data = (connections) => ({ connections, counts: {}, vault: { status: 'ok', logins: 0 }, catalog: [] });
const noop = () => {};
// JSDOM has no layout: say a box overflows.
const tall = (el) => {
  Object.defineProperty(el, 'scrollHeight', { value: 2000, configurable: true });
  Object.defineProperty(el, 'clientHeight', { value: 600, configurable: true });
};
// Effects run after the commit: let them.
const tick = () => new Promise((r) => setTimeout(r, 0));
const doc = (html) => new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');

test('the row has an amber "Sign in again" first in its strip only for a URL tool server whose sign-in ran out', () => {
  const d = doc(ui.row({ c: conn(), onSignIn: noop, onCheck: noop, links: { onOpenTools: noop } }));
  const strip = d.querySelector('[data-sign-in-again]').parentElement;
  const btn = strip.firstElementChild;
  assert.equal(btn.getAttribute('data-sign-in-again'), 'mcp:s1', 'first in the strip');
  assert.equal(btn.textContent, 'Sign in again: Tasklane');
  assert.equal(btn.querySelector('.sr-only').textContent, ': Tasklane', 'the name a screen reader hears starts with the words on the button');
  assert.match(btn.className, /bg-amber-500/);
  assert.match(btn.className, /min-h-\[44px\] sm:min-h-\[32px\]/);
  assert.match(btn.className, /focus-visible:outline-sky-500/);
  assert.match(strip.textContent, /Check now/);
  assert.match(strip.textContent, /Manage tools/);

  for (const c of [
    conn({ status: 'connected' }),
    conn({ status: 'error' }),
    conn({ logins_stored: 'composio', name: 'Composio' }),
    conn({ id: 'account:p1', kind: 'account', name: 'Claude' }),
    conn({ id: 'login:k1:c1', kind: 'browser_login' }),
  ]) {
    assert.equal(ui.canSignInAgain(c), false, `${c.kind} ${c.status} ${c.logins_stored}`);
    assert.equal(doc(ui.row({ c, onSignIn: noop })).querySelector('[data-sign-in-again]'), null);
  }
  assert.equal(doc(ui.row({ c: conn() })).querySelector('[data-sign-in-again]'), null, 'no button without a sign-in to run');
  assert.match(ui.row({ c: conn(), onSignIn: noop, signingIn: true }), /Waiting for sign-in…/);
});

test('opening on a row scrolls to it, lights it for 1.2 s and puts the keyboard on "Sign in again"', async () => {
  scrolls.length = 0;
  const other = conn({ id: 'mcp:s2', name: 'Notes', status: 'connected' });
  const props = { data: data([other, conn()]), loading: false, error: null, onRefresh: noop, onAddComposio: async () => null, onSignIn: noop };
  const v = ui.mount('view', { ...props, focusConnection: { id: 'mcp:s1', seq: 1 } });
  await tick();
  const lit = () => [...v.host.querySelectorAll('[data-flash]')].map((x) => x.getAttribute('data-connection'));
  assert.deepEqual(lit(), ['mcp:s1']);
  assert.match(v.host.querySelector('[data-connection="mcp:s1"]').className, /bg-sky-500\/10/);
  assert.deepEqual(scrolls, [{ scroller: 'settings', behavior: 'smooth' }], 'only the nearest scroller moves');
  assert.equal(document.activeElement?.getAttribute('data-sign-in-again'), 'mcp:s1');
  // A reload of the list doesn't light it again; a new open does.
  v.render({ ...props, data: data([other, conn()]), focusConnection: { id: 'mcp:s1', seq: 1 } });
  assert.equal(scrolls.length, 1);
  await new Promise((r) => setTimeout(r, ui.ROW_FLASH_MS + 50));
  assert.deepEqual(lit(), [], 'the light goes after 1.2 s');
  reduced = true;
  v.render({ ...props, focusConnection: { id: 'mcp:s1', seq: 2 } });
  await tick();
  assert.deepEqual(scrolls[1], { scroller: 'settings', behavior: 'auto' }, 'reduced motion jumps');
  assert.match(v.host.querySelector('[data-connection="mcp:s1"]').className, /motion-reduce:transition-none/);
  reduced = false;
  v.unmount();
});

test('opening before the list has loaded waits for the row', async () => {
  scrolls.length = 0;
  const props = { loading: true, error: null, onRefresh: noop, onAddComposio: async () => null, focusConnection: { id: 'mcp:s1', seq: 7 } };
  const v = ui.mount('view', { ...props, data: null });
  assert.equal(scrolls.length, 0);
  await tick();
  v.render({ ...props, loading: false, data: data([conn()]) });
  await tick();
  assert.deepEqual(scrolls.map((s) => s.scroller), ['settings']);
  v.unmount();
});

test('the chat line\'s button hands its connection id to Connections', () => {
  const got = [];
  const meta = ui.parseToolFailureMeta({
    kind: 'tool_failure', kwant_id: 'k1', connection: 'Tasklane', cause: 'sign_in', line: 'Tasklane stopped working for Ben: it needs you to sign in again.',
    needs_you: true, fix: { label: 'Reconnect', opens: 'connections', connection_id: 'mcp:s1' }, resolved_at: null,
  });
  const v = ui.mount('line', { meta, onFix: (id) => got.push(id) });
  const btn = v.host.querySelector('button');
  assert.equal(btn.textContent, 'Sign in again');
  btn.click();
  assert.deepEqual(got, ['mcp:s1']);
  v.unmount();
});

test('signInToServer opens the sign-in window inside the click, then tests the server', async () => {
  const calls = [];
  const popup = { closed: false, location: { href: '' }, focus() {}, close() { this.closed = true; } };
  const realOpen = window.open;
  const realFetch = globalThis.fetch;
  window.open = (...a) => {
    calls.push(['open', a[1]]);
    return popup;
  };
  globalThis.fetch = async (url, init) => {
    calls.push([init?.method ?? 'GET', url]);
    if (url.endsWith('/oauth/start')) {
      setTimeout(() => (popup.closed = true), 10);
      return new Response(JSON.stringify({ authorize_url: 'https://auth.example/authorize?x=1' }), { status: 200 });
    }
    return new Response(JSON.stringify({ ok: true, tools: [], auth_required: false }), { status: 200 });
  };
  try {
    const p = ui.mcp.signInToServer('s1');
    assert.deepEqual(calls[0], ['open', 'kwnt-mcp-oauth'], 'the window opens before the first await');
    assert.equal(await p, undefined);
    assert.equal(popup.location.href, 'https://auth.example/authorize?x=1');
    assert.deepEqual(calls.slice(1), [['POST', '/api/mcp/servers/s1/oauth/start'], ['POST', '/api/mcp/servers/s1/test']]);

    // A dead end closes the window and hands back its code.
    popup.closed = false;
    globalThis.fetch = async () => new Response(JSON.stringify({ error: 'no', code: 'no_client_registration' }), { status: 400 });
    assert.deepEqual(await ui.mcp.signInToServer('s1'), { code: 'no_client_registration' });
    assert.equal(popup.closed, true);
  } finally {
    window.open = realOpen;
    globalThis.fetch = realFetch;
  }
});

test('a section or row scroll moves its own scroller, never the clipped Settings layer around it', () => {
  scrolls.length = 0;
  const layer = document.createElement('div');
  layer.style.overflowY = 'hidden';
  tall(layer);
  layer.setAttribute('data-scroller', 'layer');
  const list = document.createElement('div');
  list.style.overflowY = 'auto';
  list.setAttribute('data-scroller', 'list');
  tall(list);
  const strip = document.createElement('div');
  strip.style.overflowX = 'auto'; // computes overflow-y to auto, but doesn't overflow down
  strip.setAttribute('data-scroller', 'strip');
  const clip = document.createElement('div');
  clip.style.overflowY = 'hidden';
  clip.setAttribute('data-scroller', 'clip');
  const el = document.createElement('div');
  clip.appendChild(el);
  strip.appendChild(clip);
  list.appendChild(strip);
  layer.appendChild(list);
  document.body.appendChild(layer);
  ui.scrollInScroller(el, 'start');
  ui.scrollInScroller(el, 'center', 'smooth');
  assert.deepEqual(scrolls, [{ scroller: 'list', behavior: 'auto' }, { scroller: 'list', behavior: 'smooth' }]);
  layer.remove();
});
