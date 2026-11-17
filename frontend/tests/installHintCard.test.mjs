// 20h round 2 (Rigel's QA on #691): on an iPhone the floating InstallHint ("Install kwant bots on your
// phone") sat over the PhoneAppCard in Settings › All spaces, which already shows the same steps.
// The hint steps aside while the card is mounted and comes back when it goes; its dismissal is unchanged.
import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://kwant.example/settings', pretendToBeVisual: true });
Object.defineProperty(dom.window.navigator, 'userAgent', { value: IPHONE, configurable: true });
Object.defineProperty(dom.window.navigator, 'maxTouchPoints', { value: 5, configurable: true });
Object.defineProperty(dom.window, 'isSecureContext', { value: true, configurable: true });
dom.window.matchMedia = (q) => ({ matches: /max-width: 767px/.test(q), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'KeyboardEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage', 'matchMedia']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.IS_REACT_ACT_ENVIRONMENT = false;
g.fetch = g.window.fetch = async () => new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { InstallHint } from './src/components/InstallHint';
  export { PhoneAppCard } from './src/components/PhoneAppCard';
  export * from './src/pwa';
`);
const { React, createRoot } = m;
const h = React.createElement;
const settle = async (n = 10) => {
  for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
};
const hint = () => document.querySelector('[data-install-hint]');

test('installHintKind: nothing while the phone card is on screen', () => {
  const base = { standalone: false, dismissed: false, phone: true, secure: true, ios: true, canPrompt: false };
  assert.equal(m.installHintKind(base), 'ios');
  assert.equal(m.installHintKind({ ...base, phoneCardOnScreen: true }), null);
  assert.equal(m.installHintKind({ ...base, ios: false, canPrompt: true, phoneCardOnScreen: true }), null, 'Android too');
  assert.equal(m.installHintKind({ ...base, phoneCardOnScreen: false }), 'ios');
});

test('phoneCardShown counts cards: the second unmount clears it, and cleanup twice is harmless', () => {
  let calls = 0;
  const unsub = m.subscribePhoneCard(() => calls++);
  assert.equal(m.isPhoneCardOnScreen(), false);
  const a = m.phoneCardShown();
  const b = m.phoneCardShown();
  a();
  a();
  assert.equal(m.isPhoneCardOnScreen(), true, 'one card still up');
  b();
  assert.equal(m.isPhoneCardOnScreen(), false);
  assert.equal(calls, 4);
  unsub();
});

test('iPhone: the hint shows elsewhere, steps aside while PhoneAppCard is mounted, and comes back after', async () => {
  localStorage.clear();
  const hintHost = document.createElement('div');
  const cardHost = document.createElement('div');
  document.body.append(hintHost, cardHost);
  const hintRoot = createRoot(hintHost);
  hintRoot.render(h(m.InstallHint));
  await settle();
  assert.equal(hint()?.getAttribute('data-install-hint'), 'ios', 'Share steps on any other screen');

  const cardRoot = createRoot(cardHost);
  cardRoot.render(h(m.PhoneAppCard, { phone: true }));
  await settle();
  assert.match(cardHost.textContent, /Add to Home Screen/, 'the card rendered, with the same iPhone steps');
  assert.equal(hint(), null, 'no second copy of the steps over the card');

  cardRoot.unmount();
  await settle();
  assert.equal(hint()?.getAttribute('data-install-hint'), 'ios', 'back once the card is gone');

  // Dismissal is unchanged: the X still hides it for good.
  hint().querySelector('button[aria-label]')?.click();
  await settle();
  assert.equal(localStorage.getItem(m.INSTALL_HINT_DISMISSED_KEY), '1');
  assert.equal(hint(), null);
  hintRoot.unmount();
});
