// 0.9.5 F8 (release-095/tool-failure-line/design-check.md): a connection only the person can fix is a
// Waiting-on-you row in the Inbox, "Gmail needs you to sign in again for Ben", with Reconnect.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'DOMParser', 'localStorage', 'sessionStorage', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}

const m = await loadBundled(`
  export { default as React } from 'react';
  export { renderToStaticMarkup } from 'react-dom/server.browser';
  export * as store from './src/inboxStore';
  export { INBOX_FIXTURE_EMPTY } from './src/api/inbox';
  export { InboxNew } from './src/components/InboxNew';
  export { itemLabel } from './src/away';
`);
const { React, store: s } = m;

// Shaped like src/server/inbox.rs connection_item.
const row = {
  kind: 'connection', id: 'msg1', kwant_id: 'k1', kwant_name: 'Ben', title: 'Gmail needs you to sign in again for Ben', summary: '',
  status: 'open', timestamp: '2026-11-12T18:00:00Z', link: { thread_id: null, channel_id: null, direct_kwant_id: 'k1' },
  connection: 'Gmail', cause: 'sign_in', fix: { label: 'Reconnect', opens: 'connections', connection_id: 'mcp:s1' },
  group: 'waiting_on_you', sort_key: [0, 0, 0, 'Ben', 'msg1'],
};

const render = (props) => {
  const data = JSON.parse(JSON.stringify(m.INBOX_FIXTURE_EMPTY));
  data.items = [row];
  data.counts.waiting_on_you = 1;
  s.inboxStore.reset();
  s.inboxStore.patch({ data });
  const html = m.renderToStaticMarkup(React.createElement(m.InboxNew, { noLoad: true, wide: false, onOpen: () => {}, ...props }));
  return new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
};

test('a connection row says who needs what, with Reconnect, in Waiting on you', () => {
  const d = render({ onOpenConnections: () => {} });
  const li = d.querySelector('[data-row="msg1"]');
  assert.ok(li, 'the row is listed');
  assert.equal(li.getAttribute('data-group'), 'waiting_on_you');
  assert.match(li.textContent, /Gmail needs you to sign in again for Ben/);
  assert.match(li.textContent, /Sign in again/);
  const btn = li.querySelector('[data-reconnect="msg1"]');
  assert.ok(btn);
  assert.equal(btn.textContent, 'Reconnect');
  assert.match(btn.className, /h-11/, '44 px on a phone');
  assert.deepEqual(m.itemLabel({ kind: 'connection', status: 'open' }), { label: 'Sign in again', tone: 'amber' });
  assert.deepEqual(m.itemLabel({ kind: 'connection', status: 'open', cause: 'sign_in' }), { label: 'Sign in again', tone: 'amber' });
});

test('a Composio app with no active account says Connect again, still with Reconnect', () => {
  const composio = {
    ...row, id: 'msg2', kwant_name: 'TechLead', title: 'Google Calendar needs to be connected again for TechLead',
    connection: 'Google Calendar', cause: 'reconnect', sort_key: [0, 0, 0, 'TechLead', 'msg2'],
  };
  const data = JSON.parse(JSON.stringify(m.INBOX_FIXTURE_EMPTY));
  data.items = [composio];
  data.counts.waiting_on_you = 1;
  s.inboxStore.reset();
  s.inboxStore.patch({ data });
  const html = m.renderToStaticMarkup(React.createElement(m.InboxNew, { noLoad: true, wide: false, onOpen: () => {}, onOpenConnections: () => {} }));
  const li = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html').querySelector('[data-row="msg2"]');
  assert.ok(li, 'the row is listed');
  assert.match(li.textContent, /Google Calendar needs to be connected again for TechLead/);
  assert.match(li.textContent, /Connect again/);
  assert.doesNotMatch(li.textContent, /Sign in again/);
  assert.equal(li.querySelector('[data-reconnect="msg2"]').textContent, 'Reconnect');
  assert.deepEqual(m.itemLabel({ kind: 'connection', status: 'open', cause: 'reconnect' }), { label: 'Connect again', tone: 'amber' });
});

test('without a way to open Connections there is no button', () => {
  assert.equal(render({}).querySelector('[data-reconnect]'), null);
});
