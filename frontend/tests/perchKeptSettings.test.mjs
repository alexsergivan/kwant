// 24b (autonomy/app-settings-save-or-say): a Perch change the desktop app couldn't save doesn't pretend it stuck.
// Rust keeps the old settings, sets `__KWANT_ISLAND__` to them again and fires `kwant-island`; the card shows them.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { JSDOM, VirtualConsole } from 'jsdom';
import { loadBundled } from './bundle.mjs';

// jsdom can't navigate to kwant-app://, and says so on its console: that is the click reaching the app.
const quiet = new VirtualConsole();
const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://127.0.0.1:8799/settings', pretendToBeVisual: true, virtualConsole: quiet });
const g = globalThis;
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'MouseEvent', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle', 'localStorage', 'sessionStorage']) {
  if (!(k in dom.window)) continue;
  Object.defineProperty(g, k, { value: dom.window[k], configurable: true, writable: true });
}
g.window.matchMedia = (q) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} });
g.IS_REACT_ACT_ENVIRONMENT = false;

const m = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { flushSync } from 'react-dom';
  export { PerchSettingsCard } from './src/components/PlaceChip';
  export { ISLAND_EVENT } from './src/placeChip';
`);
const settle = (ms = 0) => new Promise((r) => setTimeout(r, ms));

/** What the app's island script does (apps/mac switcher::island_script). */
function island(mode, shortcut) {
  window.__KWANT_ISLAND__ = Object.freeze({ mode, shortcut, available: true });
  window.dispatchEvent(new Event('kwant-island'));
}

async function mount() {
  window.__KWANT_APP__ = 'linux';
  window.__KWANT_ISLAND__ = Object.freeze({ mode: 'busy', shortcut: 'k', available: true });
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = m.createRoot(host);
  m.flushSync(() => root.render(m.React.createElement(m.PerchSettingsCard)));
  await settle();
  return { host, done: () => { root.unmount(); host.remove(); } };
}
const checked = (host) => host.querySelector('[data-perch-mode][aria-checked="true"]')?.dataset.perchMode;

test('a change the app could not save goes back to the settings it kept', async () => {
  const { host, done } = await mount();
  assert.equal(checked(host), 'busy');
  m.flushSync(() => host.querySelector('[data-perch-mode="always"]').click());
  assert.equal(checked(host), 'always', 'the click shows at once');
  m.flushSync(() => island('busy', 'k')); // save failed: the old ones again
  assert.equal(checked(host), 'busy');
  assert.equal(host.querySelector('[data-perch-shortcut]').value, 'k');
  done();
});

test('a saved change stays, and the shortcut the app kept is the one shown', async () => {
  const { host, done } = await mount();
  m.flushSync(() => host.querySelector('[data-perch-mode="always"]').click());
  m.flushSync(() => island('always', 'k'));
  assert.equal(checked(host), 'always');
  m.flushSync(() => island('always', 'space'));
  assert.equal(host.querySelector('[data-perch-shortcut]').value, 'space');
  done();
});

test('the card listens for the event the app fires', () => {
  assert.equal(m.ISLAND_EVENT, 'kwant-island');
  const rust = readFileSync(new URL('../../apps/mac/src/switcher.rs', import.meta.url), 'utf8');
  assert.ok(rust.includes(`new Event('${m.ISLAND_EVENT}')`), 'switcher::island_script fires it');
});
