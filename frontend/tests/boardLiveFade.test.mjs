// 0.7 A7 on the mission board: the "Live now" row fades its trailing 24 px on phones while it can
// scroll further right, and has no fade at the end or when everything fits (the Suggested helper).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { loadBundled } from './bundle.mjs';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'http://localhost/s/s1/board', pretendToBeVisual: true });
for (const k of ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node', 'Event', 'localStorage', 'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle']) {
  if (k in dom.window) Object.defineProperty(globalThis, k, { value: dom.window[k], configurable: true, writable: true });
}
globalThis.IS_REACT_ACT_ENVIRONMENT = false;

const ui = await loadBundled(`
  export { default as React } from 'react';
  export { createRoot } from 'react-dom/client';
  export { LiveStrip } from './src/components/MissionBoardView';
`);
const h = ui.React.createElement;
const settle = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };

const NOW = Date.parse('2026-11-05T10:00:00Z');
const bot = (id, name) => ({ kwantId: id, name, kind: 'chat', what: 'Writing the launch copy', since: new Date(NOW - 600_000).toISOString(), more: 0 });
const bots = [bot('k_a', 'Ada'), bot('k_b', 'Lin'), bot('k_c', 'Pixel'), bot('k_d', 'Rigel')];
const teams = [{ id: 'tm_demo', name: 'Demo team', lead_kwant_id: 'k_a', member_kwants: ['k_b', 'k_c', 'k_d'] }];

// jsdom has no layout: give the row a size and a scroll position.
const size = (el, { scrollWidth, clientWidth, scrollLeft = 0 }) => {
  Object.defineProperty(el, 'scrollWidth', { value: scrollWidth, configurable: true });
  Object.defineProperty(el, 'clientWidth', { value: clientWidth, configurable: true });
  el.scrollLeft = scrollLeft;
  Object.defineProperty(el, 'scrollLeft', { value: scrollLeft, configurable: true, writable: true });
  el.dispatchEvent(new window.Event('scroll'));
};

test('Live now: the row fades while it can scroll, not at the end, and not when it all fits', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  root.render(h(ui.LiveStrip, { bots, teams, workingCards: 0, now: NOW, onOpen: () => {} }));
  await settle();
  const row = host.querySelector('[data-live-row]');
  assert.ok(row, 'the Live now row renders');
  assert.ok(row.classList.contains('kw-scroll-row'), 'uses the shared scroll-cue class');
  assert.equal(row.getAttribute('aria-label'), 'Bots working now');
  assert.equal(row.querySelectorAll('[data-live-bot]').length, 4);

  size(row, { scrollWidth: 1000, clientWidth: 390, scrollLeft: 0 });
  assert.equal(row.dataset.atEnd, 'false', 'more to the right: fade');
  assert.equal(row.dataset.atStart, 'true', 'at the start: no left fade');
  size(row, { scrollWidth: 1000, clientWidth: 390, scrollLeft: 300 });
  assert.equal(row.dataset.atEnd, 'false', 'part way: still fades');
  assert.equal(row.dataset.atStart, 'false', '14c: part way, the left edge fades too');
  size(row, { scrollWidth: 1000, clientWidth: 390, scrollLeft: 610 });
  assert.equal(row.dataset.atEnd, 'true', 'scrolled to the end: no fade');
  assert.equal(row.dataset.atStart, 'false', 'at the end: only the left edge fades');
  size(row, { scrollWidth: 390, clientWidth: 390, scrollLeft: 0 });
  assert.equal(row.dataset.atEnd, 'true', 'everything fits: no fade');
  assert.equal(row.dataset.atStart, 'true', 'everything fits: no fade');
  root.unmount();
});

test('Live now: no row (and so no fade) when nobody is working', async () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const root = ui.createRoot(host);
  root.render(h(ui.LiveStrip, { bots: [], teams, workingCards: 0, now: NOW }));
  await settle();
  assert.equal(host.querySelector('[data-live-row]'), null);
  root.unmount();
});

test('the fade itself (14c): 32 px on each side that can still scroll, phones only', () => {
  const css = readFileSync(new URL('../src/styles/modern.css', import.meta.url), 'utf8');
  const phone = css.slice(css.indexOf('@media (max-width: 767px)'));
  assert.match(phone, /\.kw-scroll-row\[data-at-start='false'\]\s*\{\s*--kw-fade-l: 32px;/);
  assert.match(phone, /\.kw-scroll-row\[data-at-end='false'\]\s*\{\s*--kw-fade-r: 32px;/);
  assert.match(phone, /\.kw-scroll-row:is\(\[data-at-start='false'\], \[data-at-end='false'\]\)\s*\{[^}]*mask-image: linear-gradient\(to right, transparent, #000 var\(--kw-fade-l\), #000 calc\(100% - var\(--kw-fade-r\)\), transparent\)/);
  // Nothing masks a row that fits (both sides "true")
  assert.doesNotMatch(phone.slice(0, phone.indexOf(':is(')), /\.kw-scroll-row \{[^}]*mask-image/);
});
