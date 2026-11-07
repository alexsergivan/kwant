// 0.7.4 GitHub Enterprise address (Lin): the one GitHub form asks for the Enterprise address as
// people paste it (the server finds the API), and says where an Enterprise token is made.
// Demo data only: example hosts, no real tokens, and nothing leaves this process.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/spc_demo', pretendToBeVisual: true });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'HTMLInputElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'FocusEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });

const posts = [];
const json = (b, status = 200) => new Response(JSON.stringify(b), { status, headers: { 'Content-Type': 'application/json' } });
g.fetch = g.window.fetch = async (url, init = {}) => {
  const path = String(url).split('?')[0];
  const method = init.method ?? 'GET';
  if (path === '/api/passwords/store') return json({ store: 'local', locked: false, local_count: 0, bitwarden: null, approvals_off: false, exposed: false, risk_accepted: false });
  if (path === '/api/zombie/sources' && method === 'POST') {
    const body = JSON.parse(init.body);
    posts.push(body);
    return json({ source: { id: 'src_demo', kind: 'github', label: body.label || 'GitHub', enabled: true }, test: null });
  }
  return json({});
};

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { GitHubConnectForm } from './src/components/LooseEndsView';
  export { githubSource } from './src/looseEnds';
`);
const { React, createRoot } = m;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const typeIn = (el, value) => {
  Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set.call(el, value);
  el.dispatchEvent(new window.Event('input', { bubbles: true }));
};

test('the address field asks for the Enterprise address; the token hint shows only once one is typed', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = createRoot(host);
  root.render(React.createElement(m.GitHubConnectForm, { onAdded: () => {}, onCancel: () => {} }));
  await settle(20);
  const label = [...document.querySelectorAll('label')].find((l) => /Enterprise/.test(l.textContent));
  assert.equal(label?.textContent.trim(), 'GitHub Enterprise address (leave empty for github.com)');
  const base = document.getElementById(label.htmlFor);
  assert.equal(base.placeholder, 'https://github.yourcompany.com');
  assert.equal(document.querySelector('[data-ghe-token-hint]'), null, 'github.com: no Enterprise hint');

  typeIn(base, 'https://acme.ghe.com');
  await settle();
  const hint = document.querySelector('[data-ghe-token-hint]');
  assert.equal(hint?.textContent, 'Make the token on your Enterprise server: Settings → Developer settings → Personal access tokens.');

  typeIn(base, '   ');
  await settle();
  assert.equal(document.querySelector('[data-ghe-token-hint]'), null, 'spaces only is empty');

  // What was typed goes to the server as it is: the server finds the API
  typeIn(base, 'https://github.example.com');
  document.querySelector('[data-github-token]').value = 'demo-not-a-token';
  document.querySelector('form[data-github-connect]').dispatchEvent(new window.Event('submit', { bubbles: true, cancelable: true }));
  await settle(20);
  assert.equal(posts.length, 1);
  assert.equal(posts[0].config.api_base, 'https://github.example.com');
  root.unmount();
});
